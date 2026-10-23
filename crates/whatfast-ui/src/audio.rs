//! Reproduction of the voice notes inside the chat.
//!
//! port: the web client plays the note with the `<audio>` tag of the
//! browser, right in the bubble. The native client has no browser, so the
//! file is decoded here and the sound goes to the device of the system
//! through `rodio`.
//!
//! The device lives in its own thread: the handle of the card is not `Send`
//! and the window must never wait for a decoder to finish.

use std::collections::HashMap;
use std::io::Cursor;
use std::num::NonZero;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use async_channel::{Receiver, Sender};
use gpui::{Context, Window};
use smol::Timer;

/// The speeds that the chip of the bubble cycles through.
pub(crate) const SPEEDS: [f32; 3] = [1.0, 1.5, 2.0];

/// The bars that the wave of the bubble paints.
pub(crate) const WAVE_BARS: usize = 32;

/// The shortest bar, so a silent stretch still shows the wave.
const MIN_BAR: f32 = 0.12;

/// The sample rate of every Opus stream, in hertz.
const OPUS_RATE: u32 = 48_000;

/// One audio file already decoded to samples.
pub(crate) struct DecodedAudio {
    /// The samples of every channel, interleaved.
    samples: Arc<Vec<f32>>,
    sample_rate: u32,
    channels: u16,
    /// The height of every bar of the wave, from zero to one.
    peaks: Arc<Vec<f32>>,
}

impl DecodedAudio {
    /// The length of the file, in seconds.
    pub(crate) fn duration_seconds(&self) -> f32 {
        let channels = self.channels.max(1) as usize;
        let frames = self.samples.len() / channels;
        frames as f32 / self.sample_rate.max(1) as f32
    }

    /// The heights of the wave of the bubble.
    pub(crate) fn peaks(&self) -> Arc<Vec<f32>> {
        self.peaks.clone()
    }

    /// The source that plays the file from one second on.
    fn source_from(&self, from_seconds: f32) -> Option<SamplesFrom> {
        let channels = NonZero::new(self.channels)?;
        let sample_rate = NonZero::new(self.sample_rate)?;
        let frame = (from_seconds.max(0.0) * self.sample_rate as f32).round() as usize;
        let first = (frame * self.channels.max(1) as usize).min(self.samples.len());
        Some(SamplesFrom {
            data: self.samples.clone(),
            position: first,
            channels,
            sample_rate,
        })
    }
}

/// One command for the thread of the sound.
enum Command {
    /// Decode the file (the last one is kept) and play it from one second on.
    Play {
        path: PathBuf,
        from_seconds: f32,
        speed: f32,
    },
    Pause,
    Resume,
    Speed(f32),
    Stop,
}

/// What the thread of the sound says back to the window.
pub(crate) enum AudioEvent {
    /// The file is decoded and the sound is on its way.
    Ready { duration: f32, peaks: Vec<f32> },
    /// The device, or the file, could not be used.
    Failed(String),
}

/// The voice note that the window follows: the position runs on a clock, so
/// the wave moves even between ticks.
pub(crate) struct VoiceState {
    /// The chat and the message of the note that is playing or paused.
    pub(crate) key: Option<(String, String)>,
    /// The second where the clock of the bubble started.
    base: f32,
    clock: Instant,
    /// Whether the sound is stopped at [`Self::base`].
    pub(crate) paused: bool,
    /// The speed that the chip offers.
    pub(crate) speed: f32,
    /// The length of the file, once it is decoded.
    pub(crate) duration: f32,
    /// Whether the file is still being decoded: the clock waits.
    pub(crate) waiting: bool,
    /// The wave of every note that was decoded in this session.
    pub(crate) waves: HashMap<(String, String), Arc<Vec<f32>>>,
}

impl Default for VoiceState {
    fn default() -> Self {
        Self {
            key: None,
            base: 0.0,
            clock: Instant::now(),
            paused: false,
            speed: SPEEDS[0],
            duration: 0.0,
            waiting: false,
            waves: HashMap::new(),
        }
    }
}

impl VoiceState {
    /// The second of the note that the bubble paints.
    pub(crate) fn position(&self) -> f32 {
        if self.paused || self.waiting {
            self.base
        } else {
            self.base + self.clock.elapsed().as_secs_f32() * self.speed
        }
    }

    /// Whether the note of this message is the one of the bubble.
    pub(crate) fn is_on(&self, key: &(String, String)) -> bool {
        self.key.as_ref() == Some(key)
    }

    /// Starts one note from the given second.
    pub(crate) fn start(&mut self, key: (String, String), from: f32, speed: f32) {
        self.key = Some(key);
        self.base = from.max(0.0);
        self.clock = Instant::now();
        self.paused = false;
        self.speed = speed;
        self.duration = 0.0;
        self.waiting = true;
    }

    /// Freezes the clock where the sound stops.
    pub(crate) fn pause(&mut self) {
        self.base = self.position();
        self.paused = true;
    }

    /// Goes on from the frozen second.
    pub(crate) fn resume(&mut self) {
        self.clock = Instant::now();
        self.paused = false;
    }

    /// Changes the speed without losing the second where the sound is.
    pub(crate) fn rebase(&mut self, speed: f32) {
        self.base = self.position();
        self.clock = Instant::now();
        self.speed = speed;
    }

    /// Drops the note: the bubble goes back to its first second.
    pub(crate) fn clear(&mut self) {
        self.key = None;
        self.base = 0.0;
        self.clock = Instant::now();
        self.paused = false;
        self.duration = 0.0;
        self.waiting = false;
    }
}

/// The handle of the thread of the sound.
pub(crate) struct VoiceAudio {
    commands: Sender<Command>,
    events: Receiver<AudioEvent>,
}

impl VoiceAudio {
    /// Starts the thread of the sound. The device opens with the first note.
    pub(crate) fn new() -> Self {
        let (commands, command_receiver) = async_channel::unbounded();
        let (event_sender, events) = async_channel::unbounded();
        // A thread that cannot start only leaves the notes silent; the
        // window keeps working, so the failure is not fatal.
        let _ = std::thread::Builder::new()
            .name("whatfast-audio".into())
            .spawn(move || run_audio(command_receiver, event_sender));
        Self { commands, events }
    }

    /// Plays one file from the given second.
    pub(crate) fn play(&self, path: &Path, from_seconds: f32, speed: f32) {
        let _ = self.commands.send_blocking(Command::Play {
            path: path.to_path_buf(),
            from_seconds,
            speed,
        });
    }

    /// Stops the sound without losing the position of the bubble.
    pub(crate) fn pause(&self) {
        let _ = self.commands.send_blocking(Command::Pause);
    }

    /// Goes on with the sound where it stopped.
    pub(crate) fn resume(&self) {
        let _ = self.commands.send_blocking(Command::Resume);
    }

    /// Changes the speed of the sound that is already playing.
    pub(crate) fn speed(&self, value: f32) {
        let _ = self.commands.send_blocking(Command::Speed(value));
    }

    /// Drops the sound of the note.
    pub(crate) fn stop(&self) {
        let _ = self.commands.send_blocking(Command::Stop);
    }

    /// Takes the next message of the thread, if there is one.
    pub(crate) fn try_recv(&self) -> Option<AudioEvent> {
        self.events.try_recv().ok()
    }
}

/// The body of the thread of the sound.
///
/// The device and the player live here: they are created with the first note
/// and stay alive until the thread ends.
fn run_audio(commands: Receiver<Command>, events: Sender<AudioEvent>) {
    let mut device: Option<rodio::MixerDeviceSink> = None;
    let mut player: Option<rodio::Player> = None;
    // The last decoded file: changing the speed, or going back to the same
    // note, does not decode it again.
    let mut cached: Option<(PathBuf, Arc<DecodedAudio>)> = None;

    while let Ok(command) = commands.recv_blocking() {
        match command {
            Command::Play {
                path,
                from_seconds,
                speed,
            } => {
                let decoded = match cached.as_ref() {
                    Some((cached_path, decoded)) if cached_path == &path => decoded.clone(),
                    _ => match decode(&path) {
                        Ok(decoded) => {
                            let decoded = Arc::new(decoded);
                            cached = Some((path.clone(), decoded.clone()));
                            decoded
                        }
                        Err(error) => {
                            let _ = events.send_blocking(AudioEvent::Failed(format!(
                                "The note could not be decoded: {error:#}"
                            )));
                            continue;
                        }
                    },
                };
                if device.is_none() {
                    match rodio::DeviceSinkBuilder::open_default_sink() {
                        Ok(sink) => device = Some(sink),
                        Err(error) => {
                            let _ = events.send_blocking(AudioEvent::Failed(format!(
                                "There is no audio device: {error}"
                            )));
                            continue;
                        }
                    }
                }
                let Some(mixer) = device.as_ref().map(|sink| sink.mixer()) else {
                    continue;
                };
                // A new player for every start: dropping the last one stops
                // whatever it was playing.
                let next = rodio::Player::connect_new(mixer);
                next.set_speed(clamp_speed(speed));
                if let Some(source) = decoded.source_from(from_seconds) {
                    next.append(source);
                }
                player = Some(next);
                let _ = events.send_blocking(AudioEvent::Ready {
                    duration: decoded.duration_seconds(),
                    peaks: decoded.peaks().as_ref().clone(),
                });
            }
            Command::Pause => {
                if let Some(player) = player.as_ref() {
                    player.pause();
                }
            }
            Command::Resume => {
                if let Some(player) = player.as_ref() {
                    player.play();
                }
            }
            Command::Speed(speed) => {
                if let Some(player) = player.as_ref() {
                    player.set_speed(clamp_speed(speed));
                }
            }
            Command::Stop => {
                player = None;
            }
        }
    }
}

/// The speed stays in the range that the chip offers.
fn clamp_speed(speed: f32) -> f32 {
    if speed.is_finite() {
        speed.clamp(SPEEDS[0], SPEEDS[SPEEDS.len() - 1])
    } else {
        SPEEDS[0]
    }
}

/// The samples of one file that the player reads until the end.
struct SamplesFrom {
    data: Arc<Vec<f32>>,
    position: usize,
    channels: NonZero<u16>,
    sample_rate: NonZero<u32>,
}

impl Iterator for SamplesFrom {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let sample = self.data.get(self.position).copied();
        self.position += 1;
        sample
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.data.len().saturating_sub(self.position);
        (remaining, Some(remaining))
    }
}

impl rodio::Source for SamplesFrom {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> rodio::ChannelCount {
        self.channels
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        let remaining = self.data.len().saturating_sub(self.position);
        let channels = self.channels.get() as usize;
        let rate = self.sample_rate.get() as usize;
        Some(Duration::from_secs_f32(
            remaining as f32 / (rate.max(1) * channels.max(1)) as f32,
        ))
    }
}

/// Decodes one file to samples, whatever the container is.
///
/// The notes of WhatsApp are Ogg Opus, which `symphonia` does not decode, so
/// that container has its own path. The rest of the audio goes through the
/// decoder of `rodio`.
pub(crate) fn decode(path: &Path) -> anyhow::Result<DecodedAudio> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("the file {} could not be read", path.display()))?;
    if bytes.starts_with(b"OggS") {
        decode_ogg_opus(&bytes)
    } else {
        decode_with_rodio(Cursor::new(bytes))
    }
}

/// The path of the Ogg Opus: the header of the codec gives the channels and
/// the samples that the decoder has to throw away at the start.
fn decode_ogg_opus(bytes: &[u8]) -> anyhow::Result<DecodedAudio> {
    let mut reader = ogg::PacketReader::new(Cursor::new(bytes.to_vec()));
    let head = reader
        .read_packet()
        .context("the packet of the header could not be read")?
        .context("the file has no header")?;
    let data = head.data;
    if data.len() < 19 || &data[..8] != b"OpusHead" {
        bail!("the Ogg stream does not carry Opus");
    }
    let channels = usize::from(data[9]);
    if channels != 1 && channels != 2 {
        bail!("Opus with {channels} channels is not supported");
    }
    let pre_skip = usize::from(u16::from_le_bytes([data[10], data[11]]));
    // The second packet carries the tags of the stream: the sound is after
    // it.
    let _tags = reader.read_packet().context("the tags are missing")?;
    let mut decoder = opus_decoder::OpusDecoder::new(OPUS_RATE, channels)
        .context("the decoder of Opus could not start")?;
    let mut frame = vec![0f32; opus_decoder::OpusDecoder::MAX_FRAME_SIZE_48K * channels];
    let mut samples: Vec<f32> = Vec::new();
    loop {
        match reader.read_packet() {
            Ok(Some(packet)) => {
                let decoded = decoder
                    .decode_float(&packet.data, &mut frame, false)
                    .context("the packet of the note could not be decoded")?;
                samples.extend_from_slice(&frame[..decoded * channels]);
            }
            Ok(None) => break,
            Err(error) => {
                // A page in the middle of the stream that the reader cannot
                // parse ends the sound there, with what was decoded.
                let _ = error;
                break;
            }
        }
    }
    // The start of the stream is padding of the encoder.
    let skip = pre_skip * channels;
    if skip > 0 && skip < samples.len() {
        samples.drain(..skip);
    }
    Ok(finish(samples, OPUS_RATE, channels as u16))
}

/// Every other container goes through the decoder of `rodio`.
fn decode_with_rodio<R>(reader: R) -> anyhow::Result<DecodedAudio>
where
    R: std::io::Read + std::io::Seek + Send + Sync + 'static,
{
    let decoder = rodio::Decoder::new(reader).context("the format of the file is not supported")?;
    use rodio::Source as _;
    let channels = decoder.channels().get();
    let sample_rate = decoder.sample_rate().get();
    let samples: Vec<f32> = decoder.collect();
    if samples.is_empty() {
        bail!("the file has no sound");
    }
    Ok(finish(samples, sample_rate, channels))
}

/// Builds the decoded file with the wave of the bubble.
fn finish(samples: Vec<f32>, sample_rate: u32, channels: u16) -> DecodedAudio {
    let peaks = peaks(&samples, channels.max(1) as usize, WAVE_BARS);
    DecodedAudio {
        samples: Arc::new(samples),
        sample_rate,
        channels,
        peaks: Arc::new(peaks),
    }
}

/// The height of every bar of the wave: the loudest sample of its stretch,
/// divided by the loudest one of the file.
fn peaks(samples: &[f32], channels: usize, bars: usize) -> Vec<f32> {
    if bars == 0 || samples.is_empty() || channels == 0 {
        return Vec::new();
    }
    let frames = samples.len() / channels;
    if frames == 0 {
        return vec![MIN_BAR; bars];
    }
    let per_bar = frames.div_ceil(bars);
    let mut heights = Vec::with_capacity(bars);
    let mut loudest = 0f32;
    for bar in 0..bars {
        let start = bar * per_bar;
        if start >= frames {
            break;
        }
        let end = ((bar + 1) * per_bar).min(frames);
        let mut height = 0f32;
        for frame in start..end {
            let sample = samples[frame * channels];
            height = height.max(sample.abs());
        }
        loudest = loudest.max(height);
        heights.push(height);
    }
    if loudest <= f32::EPSILON {
        return vec![MIN_BAR; bars];
    }
    heights
        .into_iter()
        .map(|height| (height / loudest).clamp(MIN_BAR, 1.0))
        .collect()
}

/// The window side of the voice notes: the click of the bubble and the tick
/// that moves the wave.
impl crate::app::Whatfast {
    /// Starts, pauses or goes on with the voice note of one message.
    pub(crate) fn toggle_voice_note(
        &mut self,
        chat_id: &str,
        message_id: &str,
        path: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = (chat_id.to_string(), message_id.to_string());
        if self.voice.is_on(&key) {
            if self.voice.paused {
                self.voice.resume();
                self.voice_audio.resume();
            } else {
                self.voice.pause();
                self.voice_audio.pause();
            }
            cx.notify();
            return;
        }
        // The note of another message stops where it is.
        self.voice_audio.stop();
        let speed = self.voice.speed;
        self.voice.start(key, 0.0, speed);
        self.voice_audio.play(Path::new(path), 0.0, speed);
        self.start_voice_tick(window, cx);
        cx.notify();
    }

    /// Cycles the chip of the speed: 1x, 1.5x and 2x, as the reference.
    pub(crate) fn cycle_voice_speed(&mut self, cx: &mut Context<Self>) {
        let current = self.voice.speed;
        let next = SPEEDS
            .iter()
            .copied()
            .find(|speed| *speed > current + 0.01)
            .unwrap_or(SPEEDS[0]);
        self.voice.rebase(next);
        if self.voice.key.is_some() && !self.voice.paused {
            self.voice_audio.speed(next);
        }
        cx.notify();
    }

    /// Reads what the thread of the sound says and moves the wave of the
    /// note that is playing.
    pub(crate) fn tick_voice(&mut self, cx: &mut Context<Self>) {
        while let Some(event) = self.voice_audio.try_recv() {
            match event {
                AudioEvent::Ready { duration, peaks } => {
                    if let Some(key) = self.voice.key.clone() {
                        self.voice.waves.insert(key, Arc::new(peaks));
                    }
                    self.voice.duration = duration;
                    self.voice.waiting = false;
                    self.voice.clock = Instant::now();
                }
                AudioEvent::Failed(message) => {
                    self.store.toast_error = Some(message);
                    self.voice.clear();
                    self.voice_audio.stop();
                }
            }
        }
        if self.voice.key.is_some() && !self.voice.paused && !self.voice.waiting {
            let duration = self.voice.duration;
            if duration > 0.0 && self.voice.position() >= duration {
                self.voice.clear();
                self.voice_audio.stop();
            }
        }
        cx.notify();
    }

    /// Starts the tick that moves the wave of the note, once per window.
    fn start_voice_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.voice_tick {
            return;
        }
        self.voice_tick = true;
        let view = cx.entity().downgrade();
        cx.spawn_in(window, async move |_, window| {
            loop {
                Timer::after(Duration::from_millis(80)).await;
                let playing = window.update(|_, cx| {
                    view.update(cx, |this, cx| {
                        this.tick_voice(cx);
                        let playing = this.voice.key.is_some();
                        if !playing {
                            this.voice_tick = false;
                        }
                        playing
                    })
                });
                match playing {
                    Ok(Ok(true)) => {}
                    _ => break,
                }
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::{MIN_BAR, SPEEDS, WAVE_BARS, clamp_speed, peaks};

    #[test]
    fn the_wave_has_one_bar_for_every_stretch() {
        let samples = vec![0.5f32; 4_000];
        let heights = peaks(&samples, 1, WAVE_BARS);
        assert_eq!(heights.len(), WAVE_BARS);
        assert!(heights.iter().all(|height| *height == 1.0));
    }

    #[test]
    fn the_wave_grows_with_the_sound() {
        // The first half is quiet and the second one is loud.
        let mut samples = vec![0.1f32; 2_000];
        samples.extend(std::iter::repeat_n(0.8f32, 2_000));
        let heights = peaks(&samples, 1, WAVE_BARS);
        let quiet = heights[..WAVE_BARS / 2].iter().sum::<f32>();
        let loud = heights[WAVE_BARS / 2..].iter().sum::<f32>();
        assert!(loud > quiet);
    }

    #[test]
    fn a_silent_file_still_paints_its_wave() {
        let heights = peaks(&vec![0f32; 1_000], 1, WAVE_BARS);
        assert_eq!(heights.len(), WAVE_BARS);
        assert!(heights.iter().all(|height| *height == MIN_BAR));
    }

    #[test]
    fn the_speed_stays_in_the_range_of_the_chip() {
        assert_eq!(clamp_speed(SPEEDS[0] - 1.0), SPEEDS[0]);
        assert_eq!(clamp_speed(10.0), SPEEDS[SPEEDS.len() - 1]);
        assert_eq!(clamp_speed(f32::NAN), SPEEDS[0]);
        assert_eq!(clamp_speed(1.5), 1.5);
    }
}
