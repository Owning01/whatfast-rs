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
use std::sync::atomic::{AtomicU32, Ordering};
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

/// Transcribes a local voice-note file using `whisper` or `whisper-cli` if installed.
#[allow(dead_code)]
pub(crate) fn transcribe_voice_note(path: &Path) -> Result<String, String> {
    if !path.is_file() {
        return Err("Audio file not downloaded yet".to_string());
    }
    let out_dir = whatfast_model::paths::data_dir().join("cache");
    let _ = std::fs::create_dir_all(&out_dir);
    for bin in ["whisper", "whisper-cli", "whisper.cpp"] {
        if let Ok(output) = std::process::Command::new(bin)
            .arg(path)
            .args(["--output_format", "txt", "--output_dir"])
            .arg(&out_dir)
            .output()
            && output.status.success()
        {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("audio");
            let txt_path = out_dir.join(format!("{stem}.txt"));
            if let Ok(text) = std::fs::read_to_string(&txt_path) {
                let _ = std::fs::remove_file(&txt_path);
                let trimmed = text.trim().to_string();
                if !trimmed.is_empty() {
                    return Ok(trimmed);
                }
            }
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !stdout.is_empty() {
                return Ok(stdout);
            }
        }
    }
    Err("Install `whisper` CLI in PATH for local voice-note transcription".to_string())
}

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
    ///
    /// The session is the id of the request: the window ignores the answer
    /// of a request that another click already replaced.
    Play {
        path: PathBuf,
        from_seconds: f32,
        speed: f32,
        session: u64,
    },
    Pause,
    /// Change the speed of the note that is playing or paused.
    ///
    /// The sound is not rebuilt: the source of the player reads the new
    /// value on its next window, so the voice never jumps back.
    Speed { speed: f32 },
    Stop,
}

/// What the thread of the sound says back to the window.
pub(crate) enum AudioEvent {
    /// The file is decoded and the sound starts at this second.
    Started {
        session: u64,
        position: f32,
        duration: f32,
        peaks: Vec<f32>,
    },
    /// The device, or the file, could not be used.
    Failed { session: u64, message: String },
}

/// The voice note that the window follows: the position runs on a clock, so
/// the wave moves even between ticks.
pub(crate) struct VoiceState {
    /// The chat and the message of the note that is playing or paused.
    pub(crate) key: Option<(String, String)>,
    /// El id del pedido de reproduccion: una respuesta con otro id es vieja.
    pub(crate) session: u64,
    /// The file of the note that is playing or paused: seeking and the
    /// arrow keys start the sound again from it.
    pub(crate) path: String,
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
            session: 0,
            path: String::new(),
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
    pub(crate) fn start(&mut self, key: (String, String), path: &str, from: f32, speed: f32) {
        self.session = self.session.wrapping_add(1);
        self.key = Some(key);
        self.path = path.to_string();
        self.base = from.max(0.0);
        self.clock = Instant::now();
        self.paused = false;
        self.speed = speed;
        self.duration = 0.0;
        self.waiting = true;
    }

    /// Moves the clock of the note to another second.
    ///
    /// The sound is not touched here: the window restarts the player when
    /// the note is playing, and the play button takes the new second when
    /// the note was paused.
    pub(crate) fn seek(&mut self, seconds: f32) {
        self.session = self.session.wrapping_add(1);
        let limit = if self.duration > 0.0 {
            self.duration
        } else {
            f32::MAX
        };
        self.base = seconds.clamp(0.0, limit);
        self.clock = Instant::now();
    }

    /// Freezes the clock where the sound stops.
    pub(crate) fn pause(&mut self) {
        self.base = self.position();
        self.paused = true;
    }

    /// Goes on from the frozen second.
    pub(crate) fn resume(&mut self) {
        self.session = self.session.wrapping_add(1);
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
        self.session = self.session.wrapping_add(1);
        self.key = None;
        self.path.clear();
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
    pub(crate) fn play(&self, path: &Path, from_seconds: f32, speed: f32, session: u64) {
        let _ = self.commands.send_blocking(Command::Play {
            path: path.to_path_buf(),
            from_seconds,
            speed,
            session,
        });
    }

    /// Stops the sound without losing the position of the bubble.
    pub(crate) fn pause(&self) {
        let _ = self.commands.send_blocking(Command::Pause);
    }

    /// Changes the speed of the sound that is already playing.
    ///
    /// The source of the player reads the new value on its next window: the
    /// note keeps sounding, without a gap and without going back.
    pub(crate) fn speed(&self, value: f32) {
        let _ = self.commands.send_blocking(Command::Speed { speed: value });
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

    // The speed of the source that the player is reading: changing the chip
    // writes here, so the sound never stops to rebuild itself.
    let mut speed: Option<Arc<AtomicU32>> = None;

    while let Ok(command) = commands.recv_blocking() {
        match command {
            Command::Play {
                path,
                from_seconds,
                speed: value,
                session,
            } => {
                if device.is_none() {
                    match rodio::DeviceSinkBuilder::open_default_sink() {
                        Ok(sink) => device = Some(sink),
                        Err(error) => {
                            let _ = events.send_blocking(AudioEvent::Failed {
                                session,
                                message: format!("There is no audio device: {error}"),
                            });
                            continue;
                        }
                    }
                }
                let Some(mixer) = device.as_ref().map(|sink| sink.mixer()) else {
                    continue;
                };
                // The device never plays two notes at once: the old player
                // goes away before the new one takes its place.
                player = None;
                speed = None;
                match library.start(&path, mixer, from_seconds, value) {
                    Ok((next, control, decoded)) => {
                        player = Some(next);
                        speed = Some(control);
                        let _ = events.send_blocking(AudioEvent::Started {
                            session,
                            position: from_seconds.max(0.0),
                            duration: decoded.duration_seconds(),
                            peaks: decoded.peaks().as_ref().clone(),
                        });
                    }
                    Err(message) => {
                        let _ = events.send_blocking(AudioEvent::Failed { session, message });
                    }
                }
            }
            Command::Pause => {
                if let Some(player) = player.as_ref() {
                    player.pause();
                }
            }
            Command::Speed { speed: value } => {
                if let Some(speed) = speed.as_ref() {
                    speed.store(clamp_speed(value).to_bits(), Ordering::Relaxed);
                }
            }
            Command::Stop => {
                player = None;
                speed = None;
            }
        }
    }
}

/// What the thread keeps of the note that is playing: the decoded file, so
/// going back to the same note does not read and decode it again.
#[derive(Default)]
struct Library {
    decoded: Option<(PathBuf, Arc<DecodedAudio>)>,
}

impl Library {
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
                Ok(decoded)
            }
            Err(error) => Err(format!("The note could not be decoded: {error:#}")),
        }
    }

    /// A player for the note, ready to play from one second on.
    fn start(
        &mut self,
        path: &Path,
        mixer: &rodio::mixer::Mixer,
        from_seconds: f32,
        speed: f32,
    ) -> Result<(rodio::Player, Arc<AtomicU32>, Arc<DecodedAudio>), String> {
        let decoded = self.decoded(path)?;
        let control = Arc::new(AtomicU32::new(clamp_speed(speed).to_bits()));
        let next = rodio::Player::connect_new(mixer);
        // The source stretches the file while it reads it: the player never
        // resamples, because that is what used to turn the voice into a
        // chipmunk, and the chip can change the rhythm without stopping the
        // sound.
        next.append(WsolaSource::new(
            decoded.samples.clone(),
            decoded.sample_rate,
            decoded.channels,
            from_seconds,
            control.clone(),
        ));
        Ok((next, control, decoded))
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

/// The samples of one note that the player reads, stretched in time while
/// the note plays.
///
/// port note: the origin played the note with the `<audio>` tag and its
/// `playbackRate`, which the browser stretches in time without touching the
/// pitch. `rodio` only resamples, and a resampled voice at 1.5x sounds like
/// a chipmunk, so the note is stretched here: overlapping windows of the
/// sound are laid down at the new rhythm, and every window is aligned with
/// the sound that the ear expects to hear next (WSOLA).
///
/// The stretch happens while the file is read, not in one big pass: the
/// speed lives in an atomic, so the chip changes the rhythm on the next
/// window without stopping the sound, without a gap and without going back.
struct WsolaSource {
    samples: Arc<Vec<f32>>,
    channels: NonZero<u16>,
    sample_rate: NonZero<u32>,
    /// The length of one window, in frames.
    window: usize,
    /// The frames that every window adds to the output.
    hop: usize,
    /// How far a window may move to follow the last one.
    search: usize,
    hann: Vec<f32>,
    /// The window being mixed: the next [`Self::window`] frames of output.
    accumulator: Vec<f32>,
    /// The frames that are ready to go to the device.
    ready: Vec<f32>,
    /// The next sample of [`Self::ready`] to hand out.
    ready_index: usize,
    /// The frame of the file where the next window should start.
    ideal: f64,
    /// The frame where the last window started.
    previous: Option<usize>,
    /// The speed of the chip, shared with the thread of the sound.
    speed: Arc<AtomicU32>,
    /// Whether no more windows fit in the file.
    finished: bool,
    /// Whether the last tail of the file was already handed out.
    tail_taken: bool,
}

impl WsolaSource {
    fn new(
        samples: Arc<Vec<f32>>,
        sample_rate: u32,
        channels: u16,
        from_seconds: f32,
        speed: Arc<AtomicU32>,
    ) -> Self {
        let rate = sample_rate.max(1) as usize;
        // A window of 40 ms, half of it shared with the next one: a periodic
        // Hann window at that step sums to one and never pumps the volume.
        let window = ((rate as f32 * 0.040) as usize).max(2);
        let hop = window / 2;
        // The window may move this far to find the sound that continues the
        // last one: 6 ms is enough for a voice and keeps the search cheap.
        let search = ((rate as f32 * 0.006) as usize).max(1);
        let hann: Vec<f32> = (0..window)
            .map(|index| 0.5 - 0.5 * (std::f32::consts::TAU * index as f32 / window as f32).cos())
            .collect();
        let channels = NonZero::new(channels.max(1)).expect("one channel at least");
        let sample_rate = NonZero::new(sample_rate.max(1)).expect("one hertz at least");
        let frames = samples.len() / usize::from(channels.get());
        let frame = ((from_seconds.max(0.0) * rate as f32).round() as usize).min(frames);
        Self {
            samples,
            channels,
            sample_rate,
            window,
            hop,
            search,
            hann,
            accumulator: vec![0.0; window * usize::from(channels.get())],
            ready: vec![0.0; hop * usize::from(channels.get())],
            ready_index: hop * usize::from(channels.get()),
            ideal: frame as f64,
            previous: None,
            speed,
            finished: false,
            tail_taken: false,
        }
    }

    /// The frame where the next window starts, if the file has room for it.
    fn next_start(&self) -> Option<usize> {
        let channels = usize::from(self.channels.get());
        let frames = self.samples.len() / channels;
        if frames < self.window {
            return None;
        }
        let ideal = self.ideal.max(0.0).round() as usize;
        let highest = ideal
            .saturating_add(self.search)
            .min(frames - self.window);
        if ideal > highest {
            return None;
        }
        let lowest = ideal.saturating_sub(self.search).min(highest);
        let speed = clamp_speed(f32::from_bits(self.speed.load(Ordering::Relaxed)));
        Some(match self.previous {
            // The voice is not stretched: the window that follows the last
            // one is the same sound, one hop later.
            Some(previous) if (speed - 1.0).abs() < 0.01 => {
                (previous + self.hop).clamp(lowest, highest)
            }
            Some(previous) => best_start(
                &self.samples,
                channels,
                previous + self.hop,
                ideal,
                self.hop,
                lowest,
                highest,
            ),
            None => ideal.clamp(lowest, highest),
        })
    }

    /// Mixes the next window and leaves one hop of sound ready.
    fn produce(&mut self) {
        let channels = usize::from(self.channels.get());
        let start = if self.finished { None } else { self.next_start() };
        match start {
            Some(start) => {
                for index in 0..self.window {
                    let target = index * channels;
                    let source = (start + index) * channels;
                    let gain = self.hann[index];
                    for channel in 0..channels {
                        self.accumulator[target + channel] +=
                            self.samples[source + channel] * gain;
                    }
                }
                // The head of the window is what the device hears now.
                let hop = self.hop * channels;
                self.ready.copy_from_slice(&self.accumulator[..hop]);
                // The tail of the window is the sound that the next one has
                // to continue: it stays at the front of the accumulator.
                self.accumulator
                    .copy_within(hop..self.window * channels, 0);
                self.accumulator[(self.window - self.hop) * channels..].fill(0.0);
                self.ready_index = 0;
                self.previous = Some(start);
                let speed = clamp_speed(f32::from_bits(self.speed.load(Ordering::Relaxed)));
                self.ideal += self.hop as f64 * f64::from(speed);
            }
            None => {
                self.finished = true;
                let hop = self.hop * channels;
                if self.tail_taken {
                    self.ready_index = self.ready.len();
                } else {
                    // The tail of the last window is the last sound of the
                    // file: hand it out before the note ends.
                    self.ready.copy_from_slice(&self.accumulator[..hop]);
                    self.accumulator[..hop].fill(0.0);
                    self.ready_index = 0;
                    self.tail_taken = true;
                }
            }
        }
    }
}

impl Iterator for WsolaSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.ready_index >= self.ready.len() {
            if self.finished && self.tail_taken {
                return None;
            }
            self.produce();
            if self.ready_index >= self.ready.len() {
                return None;
            }
        }
        let sample = self.ready[self.ready_index];
        self.ready_index += 1;
        Some(sample)
    }
}

impl rodio::Source for WsolaSource {
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
        // The length depends on the speed of the chip, which may change
        // while the note plays: the source ends when the file ends.
        None
    }
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
    span: usize,
    lowest: usize,
    highest: usize,
) -> usize {
    let mut best = ideal.clamp(lowest, highest);
    let mut best_score = f32::NEG_INFINITY;
    let mut best_distance = usize::MAX;
    for candidate in lowest..=highest {
        let mut score = 0f32;
        let mut index = 0;
        while index < span {
            score += samples[(candidate + index) * channels]
                * samples[(continuation + index) * channels];
            index += 4;
        }
        let distance = candidate.abs_diff(ideal);
        if score > best_score || (score == best_score && distance < best_distance) {
            best_score = score;
            best = candidate;
            best_distance = distance;
        }
    }
    best
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
                // The sound is rebuilt where the bubble is: the note may
                // have changed speed while it was paused, and a resumed
                // player would keep the old one.
                self.voice.resume();
                let at = self.voice.position();
                let speed = self.voice.speed;
                let session = self.voice.session;
                self.voice_audio.play(Path::new(path), at, speed, session);
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
        self.voice.start(key, path, 0.0, speed);
        let session = self.voice.session;
        self.voice_audio.play(Path::new(path), 0.0, speed, session);
        self.start_voice_tick(window, cx);
        cx.notify();
    }

    /// Moves one note to another second: the wave, the click on its bars and
    /// the arrows of the keyboard land here.
    ///
    /// A note that was not playing starts from that second; a paused note
    /// only moves its needle and waits for the play button.
    pub(crate) fn seek_voice_note(
        &mut self,
        chat_id: &str,
        message_id: &str,
        path: &str,
        seconds: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = (chat_id.to_string(), message_id.to_string());
        if !self.voice.is_on(&key) {
            self.voice_audio.stop();
            let speed = self.voice.speed;
            self.voice.start(key, path, seconds, speed);
            let session = self.voice.session;
            self.voice_audio.play(Path::new(path), seconds, speed, session);
            self.start_voice_tick(window, cx);
            cx.notify();
            return;
        }
        self.voice.seek(seconds);
        // Paused, the needle moves without sound: the play button restarts
        // the file from the second the bubble shows.
        if !self.voice.paused {
            let speed = self.voice.speed;
            let session = self.voice.session;
            self.voice_audio
                .play(Path::new(path), self.voice.position(), speed, session);
        }
        cx.notify();
    }

    /// Moves the note that is playing or paused by the given seconds.
    ///
    /// The arrows of the keyboard walk the note with this: five seconds
    /// back or forward, without leaving the file.
    pub(crate) fn seek_active_voice_note(&mut self, delta: f32, cx: &mut Context<Self>) {
        if self.voice.key.is_none() {
            return;
        }
        let target = self.voice.position() + delta;
        self.voice.seek(target);
        if !self.voice.paused && !self.voice.path.is_empty() {
            let speed = self.voice.speed;
            let path = self.voice.path.clone();
            let session = self.voice.session;
            self.voice_audio
                .play(Path::new(&path), self.voice.position(), speed, session);
        }
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
        // The source of the player reads the new speed on its next window:
        // the note keeps sounding, without a gap and without going back. A
        // paused note takes it when the play button goes on.
        if self.voice.key.is_some() {
            self.voice_audio.speed(next);
        }
        cx.notify();
    }

    /// Reads what the thread of the sound says and moves the wave of the
    /// note that is playing.
    pub(crate) fn tick_voice(&mut self, cx: &mut Context<Self>) {
        while let Some(event) = self.voice_audio.try_recv() {
            match event {
                AudioEvent::Started {
                    session,
                    position,
                    duration,
                    peaks,
                } => {
                    // The answer of a note that another click replaced says
                    // nothing about the note of the bubble.
                    if session != self.voice.session {
                        continue;
                    }
                    if let Some(key) = self.voice.key.clone() {
                        self.voice.waves.insert(key, Arc::new(peaks));
                    }
                    self.voice.duration = duration;
                    // The sound starts now, not when the click happened.
                    self.voice.base = position;
                    self.voice.clock = Instant::now();
                    self.voice.waiting = false;
                }
                AudioEvent::Failed { session, message } => {
                    if session != self.voice.session {
                        continue;
                    }
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
        // While the note is paused nothing moves on the bubble.
        if !self.voice.paused {
            cx.notify();
        }
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
                    _ => {
                        // The window closed or the view could not be read: the
                        // flag must not stay on, or the next note never starts
                        // its own tick.
                        let _ = window.update(|_, cx| {
                            let _ = view.update(cx, |this, _| this.voice_tick = false);
                        });
                        break;
                    }
                }
            }
        })
        .detach();
    }
}

