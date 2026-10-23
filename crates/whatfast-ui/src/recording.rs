//! The capture of the voice notes that the composer sends.
//!
//! port: the reference records the note in the browser (`MediaRecorder`) and
//! uploads the blob. The native client has no browser, so the microphone is
//! read with `cpal` and the samples are coded to the Ogg Opus file that the
//! backend demands (`SendAttachmentRequest.voice_note`): Opus is the only
//! format that WhatsApp accepts as a voice message.
//!
//! The device lives in its own thread, the same way the player of
//! `crate::audio` does: the handle of the stream is not `Send` on every
//! platform, and the window must never wait for the microphone.

use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context as _, bail};
use async_channel::{Receiver, Sender};
use cpal::traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _};
use cpal::{SampleFormat, StreamConfig};
use opus_pure::{Application, MAX_PACKET_BYTES, OggOpusWriter, OpusEncoder, OpusHead};

/// The rate of every Opus stream, in hertz.
pub(crate) const OPUS_RATE: u32 = 48_000;

/// One frame of the encoder: twenty milliseconds at 48 kHz.
const FRAME: usize = 960;

/// The bitrate of a voice note, in bits per second.
///
/// port: WhatsApp codes the notes around this rate; it is enough for the
/// voice and it keeps the file of a minute under a quarter of a megabyte.
const BITRATE: i32 = 32_000;

/// The shortest note that is worth sending.
///
/// A click of the microphone must not send half a second of silence: the
/// reference reacts the same way to a recording that never started.
pub(crate) const MIN_DURATION_SECONDS: f32 = 0.4;

/// How long one note may be. The reference stops at fifteen minutes; the
/// client keeps five, because the samples wait in memory until the send.
pub(crate) const MAX_DURATION_SECONDS: f32 = 300.0;

/// One command for the thread of the microphone.
enum Command {
    Start,
    /// Stops the capture and codes the note.
    Stop,
    /// Drops the capture without coding anything.
    Cancel,
}

/// What the thread of the microphone says back to the window.
pub(crate) enum RecorderEvent {
    /// The device is open and the samples are coming.
    Started,
    /// The note is coded and ready to go to the backend.
    Finished { path: PathBuf, duration: f32 },
    /// The device, or the encoder, could not be used.
    Failed(String),
    /// The user dropped the note: there is no file to send.
    Cancelled,
}

/// The handle of the thread of the microphone.
pub(crate) struct VoiceRecorder {
    commands: Sender<Command>,
    events: Receiver<RecorderEvent>,
}

impl VoiceRecorder {
    /// Starts the thread of the microphone. The device opens with the first
    /// note, so the app starts without a permission prompt.
    pub(crate) fn new() -> Self {
        let (commands, command_receiver) = async_channel::unbounded();
        let (event_sender, events) = async_channel::unbounded();
        // A thread that cannot start only leaves the notes silent: the
        // window keeps working, so the failure is not fatal.
        let _ = std::thread::Builder::new()
            .name("whatfast-recorder".into())
            .spawn(move || run_recorder(command_receiver, event_sender));
        Self { commands, events }
    }

    /// Opens the microphone and starts keeping the samples.
    pub(crate) fn start(&self) {
        let _ = self.commands.send_blocking(Command::Start);
    }

    /// Closes the microphone and codes the note.
    pub(crate) fn stop(&self) {
        let _ = self.commands.send_blocking(Command::Stop);
    }

    /// Closes the microphone and throws the samples away.
    pub(crate) fn cancel(&self) {
        let _ = self.commands.send_blocking(Command::Cancel);
    }

    /// Takes the next message of the thread, if there is one.
    pub(crate) fn try_recv(&self) -> Option<RecorderEvent> {
        self.events.try_recv().ok()
    }
}

/// The samples of the note that is being recorded.
struct Session {
    /// Dropping the stream closes the microphone.
    _stream: cpal::Stream,
    samples: Arc<Mutex<Vec<f32>>>,
    sample_rate: u32,
    channels: usize,
}

impl Session {
    /// Opens the default microphone of the system.
    fn open() -> anyhow::Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .context("there is no microphone in this device")?;
        let supported = device
            .default_input_config()
            .context("the microphone could not be opened")?;
        let sample_rate = supported.sample_rate();
        let channels = usize::from(supported.channels());
        if channels == 0 || sample_rate == 0 {
            bail!("the microphone has no channels");
        }
        let format = supported.sample_format();
        let config: StreamConfig = supported.into();
        let samples = Arc::new(Mutex::new(Vec::new()));
        let stream = match format {
            SampleFormat::F32 => build::<f32>(&device, config, samples.clone()),
            SampleFormat::F64 => build::<f64>(&device, config, samples.clone()),
            SampleFormat::I16 => build::<i16>(&device, config, samples.clone()),
            SampleFormat::U16 => build::<u16>(&device, config, samples.clone()),
            SampleFormat::I32 => build::<i32>(&device, config, samples.clone()),
            SampleFormat::U32 => build::<u32>(&device, config, samples.clone()),
            SampleFormat::I8 => build::<i8>(&device, config, samples.clone()),
            SampleFormat::U8 => build::<u8>(&device, config, samples.clone()),
            other => bail!("the microphone samples in a format that is not supported: {other:?}"),
        }
        .context("the microphone could not be opened")?;
        stream.play().context("the microphone could not start")?;
        Ok(Self {
            _stream: stream,
            samples,
            sample_rate,
            channels,
        })
    }

    /// The samples that the microphone kept, and their length.
    fn take(&self) -> (Vec<f32>, Option<Duration>) {
        let samples = self
            .samples
            .lock()
            .map(|mut samples| std::mem::take(&mut *samples))
            .unwrap_or_default();
        let frames = samples.len() / self.channels.max(1);
        let duration = (self.sample_rate > 0)
            .then(|| Duration::from_secs_f32(frames as f32 / self.sample_rate as f32));
        (samples, duration)
    }
}

/// The callback of the microphone, for the sample type of the device.
fn build<T>(
    device: &cpal::Device,
    config: StreamConfig,
    samples: Arc<Mutex<Vec<f32>>>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: cpal::SizedSample + ToAmplitude,
{
    let error_samples = samples.clone();
    device.build_input_stream(
        config,
        move |data: &[T], _| {
            let Ok(mut target) = samples.lock() else {
                return;
            };
            target.extend(data.iter().map(|sample| sample.to_amplitude()));
        },
        move |_error| {
            // A device that disappears stops the samples: the window learns
            // it when the note comes out empty.
            let _ = &error_samples;
        },
        None,
    )
}

/// The conversion of the samples of the device to the `-1..1` range of Opus.
trait ToAmplitude {
    fn to_amplitude(self) -> f32;
}

impl ToAmplitude for f32 {
    fn to_amplitude(self) -> f32 {
        self.clamp(-1.0, 1.0)
    }
}

impl ToAmplitude for f64 {
    fn to_amplitude(self) -> f32 {
        (self as f32).clamp(-1.0, 1.0)
    }
}

impl ToAmplitude for i16 {
    fn to_amplitude(self) -> f32 {
        self as f32 / 32_768.0
    }
}

impl ToAmplitude for u16 {
    fn to_amplitude(self) -> f32 {
        (self as f32 - 32_768.0) / 32_768.0
    }
}

impl ToAmplitude for i32 {
    fn to_amplitude(self) -> f32 {
        self as f32 / 2_147_483_648.0
    }
}

impl ToAmplitude for u32 {
    fn to_amplitude(self) -> f32 {
        (self as f64 - 2_147_483_648.0) as f32 / 2_147_483_648.0
    }
}

impl ToAmplitude for i8 {
    fn to_amplitude(self) -> f32 {
        self as f32 / 128.0
    }
}

impl ToAmplitude for u8 {
    fn to_amplitude(self) -> f32 {
        (self as f32 - 128.0) / 128.0
    }
}

/// The body of the thread of the microphone.
fn run_recorder(commands: Receiver<Command>, events: Sender<RecorderEvent>) {
    let mut session: Option<Session> = None;
    while let Ok(command) = commands.recv_blocking() {
        match command {
            Command::Start => {
                if session.is_some() {
                    continue;
                }
                match Session::open() {
                    Ok(open) => {
                        session = Some(open);
                        let _ = events.send_blocking(RecorderEvent::Started);
                    }
                    Err(error) => {
                        let _ = events.send_blocking(RecorderEvent::Failed(format!(
                            "The microphone could not be used: {error:#}"
                        )));
                    }
                }
            }
            Command::Stop => {
                let Some(open) = session.take() else {
                    continue;
                };
                let (samples, _) = open.take();
                if samples.is_empty() {
                    let _ = events.send_blocking(RecorderEvent::Failed(
                        "The microphone did not record anything".into(),
                    ));
                    continue;
                }
                let path = note_path();
                match encode_voice_note(&samples, open.sample_rate, open.channels, &path) {
                    Ok(duration) => {
                        let _ = events.send_blocking(RecorderEvent::Finished { path, duration });
                    }
                    Err(error) => {
                        let _ = events.send_blocking(RecorderEvent::Failed(format!(
                            "The voice note could not be saved: {error:#}"
                        )));
                    }
                }
            }
            Command::Cancel => {
                session = None;
                let _ = events.send_blocking(RecorderEvent::Cancelled);
            }
        }
    }
}

/// Where the note waits for the send.
fn note_path() -> PathBuf {
    whatfast_model::paths::data_dir()
        .join("voice-notes")
        .join(format!("nota-{}.ogg", uuid::Uuid::new_v4()))
}

/// Turns the samples of the microphone into the Ogg Opus file of one note.
///
/// The length is measured on the audio that went in, not on the packets:
/// the encoder pads the last frame with silence.
pub(crate) fn encode_voice_note(
    samples: &[f32],
    sample_rate: u32,
    channels: usize,
    path: &Path,
) -> anyhow::Result<f32> {
    let mono = at_opus_rate(samples, sample_rate, channels);
    let total = mono.len();
    if total == 0 {
        bail!("the recording has no samples");
    }
    let duration = total as f32 / OPUS_RATE as f32;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("the folder {} could not be created", parent.display()))?;
    }
    let file = std::fs::File::create(path)
        .with_context(|| format!("the file {} could not be created", path.display()))?;
    let mut encoder = OpusEncoder::new(OPUS_RATE as i32, 1, Application::Voip)
        .context("the encoder of Opus could not start")?;
    encoder.bitrate_bps = BITRATE;
    let head = OpusHead::for_encoder(&encoder, OPUS_RATE);
    // The encoder runs `pre_skip` samples behind its input: the last ones
    // would be lost without this extra frame of silence. RFC 7845 §4.2 and
    // §4.4 put the correction in the header and in the last granule.
    let lookahead = head.pre_skip as usize;
    let frames = (total + lookahead).div_ceil(FRAME);
    let final_granule = u64::from(head.pre_skip) + total as u64;
    let mut writer =
        OggOpusWriter::new(BufWriter::new(file), head).context("the Ogg file could not start")?;
    let mut packet = vec![0u8; MAX_PACKET_BYTES];
    let mut block = vec![0f32; FRAME];
    for index in 0..frames {
        let start = (index * FRAME).min(total);
        let end = (start + FRAME).min(total);
        block[..end - start].copy_from_slice(&mono[start..end]);
        block[end - start..].fill(0.0);
        let written = encoder
            .encode(&block, FRAME, &mut packet)
            .context("the note could not be coded")?;
        let packet = &packet[..written];
        if index + 1 == frames {
            let duration = final_granule - writer.granule().max(0) as u64;
            writer
                .write_packet_with_duration(packet, duration as u32)
                .context("the last packet of the note could not be written")?;
        } else {
            writer
                .write_packet(packet)
                .context("a packet of the note could not be written")?;
        }
    }
    writer.finish().context("the note could not be closed")?;
    Ok(duration)
}

/// Mixes the channels down to one and puts the samples at 48 kHz.
fn at_opus_rate(samples: &[f32], sample_rate: u32, channels: usize) -> Vec<f32> {
    let channels = channels.max(1);
    let mut mono = Vec::with_capacity(samples.len() / channels);
    for frame in samples.chunks_exact(channels) {
        let sum: f32 = frame.iter().sum();
        mono.push(sum / channels as f32);
    }
    // The remainder of a frame that the callback cut in half is dropped:
    // the encoder would mix it with the next packet anyway.
    if sample_rate == OPUS_RATE || mono.is_empty() {
        return mono;
    }
    // Linear interpolation: the note is voice, so the small loss of the
    // high frequencies is not heard and the code stays dependency free.
    let ratio = sample_rate as f64 / OPUS_RATE as f64;
    let output_len = ((mono.len() as f64) / ratio).floor() as usize;
    let mut output = Vec::with_capacity(output_len);
    for index in 0..output_len {
        let position = index as f64 * ratio;
        let first = position.floor() as usize;
        let second = (first + 1).min(mono.len() - 1);
        let fraction = (position - first as f64) as f32;
        let value = mono[first] * (1.0 - fraction) + mono[second] * fraction;
        output.push(value);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{FRAME, OPUS_RATE, at_opus_rate, encode_voice_note};

    fn note(duration: f32) -> Vec<f32> {
        (0..(OPUS_RATE as f32 * duration) as usize)
            .map(|index| {
                let t = index as f32 / OPUS_RATE as f32;
                (t * 440.0 * std::f32::consts::TAU).sin() * 0.5
            })
            .collect()
    }

    #[test]
    fn the_channels_are_mixed_down_to_one() {
        // Two channels: the left is loud and the right is silent.
        let samples = [0.8f32, 0.0, 0.8, 0.0];
        assert_eq!(at_opus_rate(&samples, OPUS_RATE, 2), vec![0.4, 0.4]);
    }

    #[test]
    fn another_rate_is_resampled_to_the_rate_of_opus() {
        let input = vec![0.5f32; 24_000];
        let output = at_opus_rate(&input, 24_000, 1);
        assert_eq!(output.len(), 48_000);
        assert!(output.iter().all(|sample| (*sample - 0.5).abs() < 0.01));
    }

    #[test]
    fn a_mono_note_at_the_rate_of_opus_is_kept_as_it_is() {
        let input = note(0.1);
        assert_eq!(at_opus_rate(&input, OPUS_RATE, 1), input);
    }

    #[test]
    fn the_note_is_an_ogg_opus_file_of_the_length_that_went_in() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nota.ogg");
        let samples = note(0.5);
        let duration = encode_voice_note(&samples, OPUS_RATE, 1, &path).unwrap();
        assert!((duration - 0.5).abs() < 0.01);
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.starts_with(b"OggS"));
        // The decoder of the player reads it back: the file is a note that
        // the app itself can play.
        let decoded = crate::audio::decode(&path).unwrap();
        assert!((decoded.duration_seconds() - 0.5).abs() < 0.05);
        assert_eq!(decoded.peaks().len(), crate::audio::WAVE_BARS);
        // The encoder pads the last frame: the file is at most one frame
        // longer than the audio.
        assert!(bytes.len() > FRAME / 8);
    }
}
