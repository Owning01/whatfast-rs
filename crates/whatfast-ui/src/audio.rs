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
    /// Change the speed, keeping the second the bubble shows.
    Speed { speed: f32, from_seconds: f32 },
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

    /// Changes the speed of the sound that is already playing.
    ///
    /// The second the bubble shows travels with the command: the stretched
    /// sound is a new buffer, and it has to start where the old one was.
    pub(crate) fn speed(&self, value: f32, from_seconds: f32) {
        let _ = self.commands.send_blocking(Command::Speed {
            speed: value,
            from_seconds,
        });
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
    // The last decoded file and the copy played at one speed: changing the
    // speed, or going back to the same note, does not decode it again.
    let mut library = Library::default();

    while let Ok(command) = commands.recv_blocking() {
        match command {
            Command::Play {
                path,
                from_seconds,
                speed,
            } => {
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
                match library.start(&path, mixer, from_seconds, speed) {
                    Ok((next, decoded)) => {
                        player = Some(next);
                        let _ = events.send_blocking(AudioEvent::Ready {
                            duration: decoded.duration_seconds(),
                            peaks: decoded.peaks().as_ref().clone(),
                        });
                    }
                    Err(error) => {
                        let _ = events.send_blocking(AudioEvent::Failed(error));
                        continue;
                    }
                }
            }
            Command::Pause => {
                if let Some(player) = player.as_ref() {
                    player.pause();
                }
            }
            Command::Speed {
                speed,
                from_seconds,
            } => {
                // The sound of the note is rebuilt from the stretched copy:
                // a resampled player would raise the pitch of the voice.
                let Some(mixer) = device.as_ref().map(|sink| sink.mixer()) else {
                    continue;
                };
                let path = library.path().map(Path::to_path_buf);
                if let Some(path) = path {
                    match library.start(&path, mixer, from_seconds, speed) {
                        Ok((next, _)) => player = Some(next),
                        Err(error) => {
                            let _ = events.send_blocking(AudioEvent::Failed(error));
                        }
                    }
                }
            }
            Command::Stop => {
                player = None;
            }
        }
    }
}

/// What the thread keeps of the note that is playing.
///
/// The stretched copy is the expensive one: a note of a minute holds a few
/// million samples, so only the last speed is kept.
#[derive(Default)]
struct Library {
    decoded: Option<(PathBuf, Arc<DecodedAudio>)>,
    played: Option<(f32, Arc<Vec<f32>>)>,
}

impl Library {
    fn path(&self) -> Option<&Path> {
        self.decoded.as_ref().map(|(path, _)| path.as_path())
    }

    /// The file of the note, decoded once.
    fn decoded(&mut self, path: &Path) -> Result<Arc<DecodedAudio>, String> {
        if let Some((cached_path, decoded)) = self.decoded.as_ref()
            && cached_path == path
        {
            return Ok(decoded.clone());
        }
        match decode(path) {
            Ok(decoded) => {
                let decoded = Arc::new(decoded);
                self.decoded = Some((path.to_path_buf(), decoded.clone()));
                self.played = None;
                Ok(decoded)
            }
            Err(error) => Err(format!("The note could not be decoded: {error:#}")),
        }
    }

    /// The samples of the note at one speed, stretched once per speed.
    fn samples_at(&mut self, speed: f32) -> Arc<Vec<f32>> {
        if let Some((cached_speed, samples)) = self.played.as_ref()
            && (*cached_speed - speed).abs() < 0.01
        {
            return samples.clone();
        }
        let Some((_, decoded)) = self.decoded.as_ref() else {
            return Arc::new(Vec::new());
        };
        let samples = Arc::new(stretch(
            &decoded.samples,
            decoded.channels.max(1) as usize,
            decoded.sample_rate,
            speed,
        ));
        self.played = Some((speed, samples.clone()));
        samples
    }

    /// A player for the note, ready to play from one second on.
    fn start(
        &mut self,
        path: &Path,
        mixer: &rodio::mixer::Mixer,
        from_seconds: f32,
        speed: f32,
    ) -> Result<(rodio::Player, Arc<DecodedAudio>), String> {
        let decoded = self.decoded(path)?;
        let speed = clamp_speed(speed);
        let samples = self.samples_at(speed);
        let next = rodio::Player::connect_new(mixer);
        // The samples already carry the speed: the player never resamples,
        // because that is what used to turn the voice into a chipmunk.
        if let Some(source) = SamplesFrom::new(
            samples,
            decoded.sample_rate,
            decoded.channels,
            from_seconds,
            speed,
        ) {
            next.append(source);
        }
        Ok((next, decoded))
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

/// The samples of one note played at another speed, with the pitch kept.
///
/// port note: the origin played the note with the `<audio>` tag and its
/// `playbackRate`, which the browser stretches in time without touching the
/// pitch. `rodio` only resamples, and a resampled voice at 1.5x sounds like
/// a chipmunk, so the note is stretched here: overlapping windows of the
/// sound are laid down at the new rhythm, and every window is aligned with
/// the sound that the ear expects to hear next (WSOLA).
pub(crate) fn stretch(samples: &[f32], channels: usize, sample_rate: u32, speed: f32) -> Vec<f32> {
    let channels = channels.max(1);
    let frames = samples.len() / channels;
    let speed = clamp_speed(speed);
    if frames == 0 || (speed - 1.0).abs() < 0.01 {
        return samples.to_vec();
    }
    // A window of 40 ms, half of it shared with the next one: a periodic
    // Hann window at that step sums to one and never pumps the volume.
    let rate = sample_rate.max(1) as usize;
    let window = ((rate as f32 * 0.040) as usize).max(2);
    let hop = window / 2;
    // The window may move this far to find the sound that continues the
    // last one: 6 ms is enough for a voice and keeps the search cheap.
    let search = ((rate as f32 * 0.006) as usize).max(1);
    let hann: Vec<f32> = (0..window)
        .map(|index| 0.5 - 0.5 * (std::f32::consts::TAU * index as f32 / window as f32).cos())
        .collect();
    let out_frames = (frames as f32 / speed).ceil() as usize + window;
    let mut out = vec![0f32; out_frames * channels];
    let mut written = 0usize;
    // The position of the source runs in frames, not in samples, so the
    // speed does not have to be divided by the channels.
    let mut ideal = 0f64;
    let mut previous: Option<usize> = None;
    loop {
        let start = ideal.round() as usize;
        if start + window + search >= frames || written + window > out_frames {
            break;
        }
        // The first window starts where the sound is; the rest start near
        // the new second, moved to where they continue the last one.
        let start = match previous {
            Some(previous) => best_start(samples, channels, previous + hop, start, search, hop),
            None => start,
        };
        for (index, gain) in hann.iter().enumerate() {
            let target = (written + index) * channels;
            let source = (start + index) * channels;
            for channel in 0..channels {
                out[target + channel] += samples[source + channel] * gain;
            }
        }
        previous = Some(start);
        written += hop;
        ideal += hop as f64 * f64::from(speed);
    }
    out.truncate((written + window).min(out_frames) * channels);
    out
}

/// The start of a window, near `ideal`, whose sound continues the one that
/// follows `continuation` the best.
///
/// The score is the cross-correlation of the two windows, sampled every four
/// frames: the alignment does not need every sample, and the search runs on
/// the thread of the sound.
fn best_start(
    samples: &[f32],
    channels: usize,
    continuation: usize,
    ideal: usize,
    search: usize,
    span: usize,
) -> usize {
    let lowest = ideal.saturating_sub(search);
    let mut best = ideal;
    let mut best_score = f32::NEG_INFINITY;
    for candidate in lowest..=ideal + search {
        let mut score = 0f32;
        let mut index = 0;
        while index < span {
            score += samples[(candidate + index) * channels]
                * samples[(continuation + index) * channels];
            index += 4;
        }
        if score > best_score {
            best_score = score;
            best = candidate;
        }
    }
    best
}

/// The samples of one file that the player reads until the end.
///
/// The data is already stretched to the speed of the chip, so the player
/// runs at its natural rate: a resampled player would raise the pitch.
struct SamplesFrom {
    data: Arc<Vec<f32>>,
    position: usize,
    channels: NonZero<u16>,
    sample_rate: NonZero<u32>,
}

impl SamplesFrom {
    fn new(
        data: Arc<Vec<f32>>,
        sample_rate: u32,
        channels: u16,
        from_seconds: f32,
        speed: f32,
    ) -> Option<Self> {
        let channels = NonZero::new(channels)?;
        let sample_rate = NonZero::new(sample_rate)?;
        // The buffer already carries the speed: the second of the bubble is
        // the second of the stretched sound.
        let frame =
            (from_seconds.max(0.0) * sample_rate.get() as f32 * speed).round() as usize;
        let position = (frame * usize::from(channels.get())).min(data.len());
        Some(Self {
            data,
            position,
            channels,
            sample_rate,
        })
    }
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
                // The sound is rebuilt where the bubble is: the note may
                // have changed speed while it was paused, and a resumed
                // player would keep the old one.
                let at = self.voice.position();
                let speed = self.voice.speed;
                self.voice_audio.play(Path::new(path), at, speed);
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
        let playing = self.voice.key.is_some() && !self.voice.paused;
        let at = self.voice.position();
        self.voice.rebase(next);
        if playing {
            // The stretched sound starts where the bubble shows: the note is
            // rebuilt at the new speed, never resampled.
            self.voice_audio.speed(next, at);
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
    use super::{MIN_BAR, SPEEDS, WAVE_BARS, clamp_speed, peaks, stretch};

    /// The frequency of a tone, from the zero crossings of its middle part:
    /// the edges of the stretch are windows, not the tone.
    fn tone_frequency(samples: &[f32], rate: u32) -> f32 {
        let edge = (rate as usize / 10).min(samples.len() / 4);
        let body = &samples[edge..samples.len() - edge];
        let crossings = body
            .windows(2)
            .filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0))
            .count();
        let seconds = body.len() as f32 / rate as f32;
        crossings as f32 / (2.0 * seconds)
    }

    fn tone(frequency: f32, rate: u32, seconds: f32) -> Vec<f32> {
        let frames = (rate as f32 * seconds) as usize;
        (0..frames)
            .map(|index| {
                (std::f32::consts::TAU * frequency * index as f32 / rate as f32).sin()
            })
            .collect()
    }

    #[test]
    fn a_stretched_note_keeps_the_pitch_of_the_voice() {
        // The bug: the player resampled the note, and at 1.5x and 2x the
        // voice sounded like a chipmunk.
        let rate = 48_000;
        let source = tone(440.0, rate, 1.0);
        for speed in [1.5f32, 2.0] {
            let fast = stretch(&source, 1, rate, speed);
            let frequency = tone_frequency(&fast, rate);
            assert!(
                (frequency - 440.0).abs() < 15.0,
                "at {speed}x the tone has to stay at 440 Hz, and it is {frequency} Hz"
            );
        }
    }

    #[test]
    fn a_stretched_note_lasts_the_time_the_speed_says() {
        let rate = 48_000;
        let source = tone(300.0, rate, 2.0);
        let half = stretch(&source, 1, rate, 2.0);
        assert!(
            (half.len() as f32 - source.len() as f32 / 2.0).abs() < rate as f32 * 0.1,
            "2x leaves half of the frames, and it left {} of {}",
            half.len(),
            source.len()
        );
        let slower = stretch(&source, 1, rate, 1.5);
        assert!(
            (slower.len() as f32 - source.len() as f32 / 1.5).abs() < rate as f32 * 0.1,
            "1.5x leaves two thirds of the frames"
        );
    }

    #[test]
    fn the_speed_of_one_leaves_the_note_as_it_is() {
        let source = tone(440.0, 48_000, 0.5);
        assert_eq!(stretch(&source, 1, 48_000, 1.0), source);
    }

    #[test]
    fn the_stretch_keeps_both_channels() {
        let rate = 48_000;
        let left = tone(440.0, rate, 0.5);
        let right = tone(660.0, rate, 0.5);
        let mut stereo = Vec::with_capacity(left.len() * 2);
        for (a, b) in left.iter().zip(right.iter()) {
            stereo.push(*a);
            stereo.push(*b);
        }
        let fast = stretch(&stereo, 2, rate, 2.0);
        assert_eq!(fast.len() % 2, 0);
        let left: Vec<f32> = fast.iter().step_by(2).copied().collect();
        let right: Vec<f32> = fast.iter().skip(1).step_by(2).copied().collect();
        assert!((tone_frequency(&left, rate) - 440.0).abs() < 15.0);
        assert!((tone_frequency(&right, rate) - 660.0).abs() < 20.0);
    }

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
