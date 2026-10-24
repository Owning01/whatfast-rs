//! The desktop's half of a call's audio: the microphone in, the speaker
//! out, both at the engine's clock (16 kHz mono i16).
//!
//! The sidecar owns the engine and the relay; this module only moves
//! frames. The capture opens the platform's default input (the same cpal
//! path the voice notes use) and resamples whatever the device runs at down
//! to 16 kHz with linear interpolation; the playback hands frames to rodio
//! at 16 kHz and lets the platform mixer do the rest.
//!
//! There is deliberately no echo cancellation in this wave: the engine has
//! its own concealment, and the practical mitigation is the same one
//! WhatsApp Web's first calls shipped with — use a headset. The AEC port
//! (`sonora`) is its own follow-up.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use cpal::traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _};
use cpal::{SampleFormat, StreamConfig};

/// The engine's audio clock.
pub const RATE: u32 = 16_000;

/// The live capture: drops into `frames` as engine-clock i16 chunks.
pub struct Capture {
    _stream: cpal::Stream,
    frames: async_channel::Receiver<Vec<i16>>,
}

impl Capture {
    /// Opens the default microphone. `Err` when there is none or it refuses.
    pub fn open() -> anyhow::Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| anyhow::anyhow!("there is no microphone in this device"))?;
        let supported = device
            .default_input_config()
            .map_err(|error| anyhow::anyhow!("the microphone could not be opened: {error}"))?;
        let device_rate = supported.sample_rate();
        let channels = usize::from(supported.channels());
        if channels == 0 || device_rate == 0 {
            anyhow::bail!("the microphone has no channels");
        }
        let (frames_tx, frames_rx) = async_channel::unbounded::<Vec<i16>>();
        let state = Arc::new(CaptureState {
            rate: device_rate,
            channels,
            carry: std::sync::Mutex::new(Vec::new()),
            position: std::sync::atomic::AtomicU64::new(0),
        });
        let config: StreamConfig = supported.into();
        let format = supported.sample_format();
        let stream = match format {
            SampleFormat::F32 => {
                capture_build::<f32>(&device, config, Arc::clone(&state), frames_tx)
            }
            SampleFormat::I16 => {
                capture_build::<i16>(&device, config, Arc::clone(&state), frames_tx)
            }
            SampleFormat::U16 => {
                capture_build::<u16>(&device, config, Arc::clone(&state), frames_tx)
            }
            other => {
                anyhow::bail!("the microphone samples in a format that is not supported: {other:?}")
            }
        }
        .map_err(|error| anyhow::anyhow!("the microphone could not be opened: {error}"))?;
        stream
            .play()
            .map_err(|error| anyhow::anyhow!("the microphone could not start: {error}"))?;
        Ok(Self {
            _stream: stream,
            frames: frames_rx,
        })
    }

    /// The next engine-clock frames, or none when the microphone closed.
    pub fn try_take(&self) -> Option<Vec<i16>> {
        self.frames.try_recv().ok()
    }

    /// Silences the outgoing frames without closing the microphone.
    pub fn set_muted(&self, muted: bool) {
        MUTE.store(muted, Ordering::SeqCst);
    }
}

static MUTE: AtomicBool = AtomicBool::new(false);

/// The resampler's shared state: device geometry and the leftover samples
/// between callbacks.
struct CaptureState {
    rate: u32,
    channels: usize,
    carry: std::sync::Mutex<Vec<f32>>,
    position: std::sync::atomic::AtomicU64,
}

impl CaptureState {
    /// Converts one device callback into engine-clock mono frames of
    /// exactly 960 samples (60 ms), the size the sidecar's pipes expect.
    fn accept(&self, samples: &[i16], out: &async_channel::Sender<Vec<i16>>) {
        let mono: Vec<f32> = samples
            .chunks(self.channels.max(1))
            .map(|frame| {
                let sum: f32 = frame.iter().map(|sample| f32::from(*sample)).sum();
                sum / self.channels.max(1) as f32
            })
            .collect();
        let mut carry = self.carry.lock().expect("carry lock");
        carry.extend_from_slice(&mono);
        let ratio = f64::from(self.rate) / f64::from(RATE);
        let mut position = self.position.load(Ordering::Relaxed) as f64;
        let mut engine: Vec<i16> = Vec::with_capacity(carry.len());
        while ((position as usize) + 1) < carry.len() {
            let index = position as usize;
            let fraction = (position - index as f64) as f32;
            let sample = carry[index] * (1.0 - fraction) + carry[index + 1] * fraction;
            let muted_sample: i16 = if MUTE.load(Ordering::SeqCst) {
                0
            } else {
                (sample.clamp(-1.0, 1.0) * 32_767.0) as i16
            };
            engine.push(muted_sample);
            position += ratio;
        }
        self.position.store(position as u64, Ordering::Relaxed);
        carry.drain(..position as usize);
        if !engine.is_empty() {
            let _ = out.try_send(engine);
        }
    }
}

fn capture_build<T>(
    device: &cpal::Device,
    config: StreamConfig,
    state: Arc<CaptureState>,
    frames: async_channel::Sender<Vec<i16>>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: cpal::SizedSample + IntoSample<i16>,
{
    device.build_input_stream(
        config,
        move |samples: &[T], _: &cpal::InputCallbackInfo| {
            let converted: Vec<i16> = samples.iter().map(|sample| sample.into_sample()).collect();
            state.accept(&converted, &frames);
        },
        move |error| eprintln!("microphone error: {error}"),
        None,
    )
}

/// The conversion the callback needs, spelled once. cpal's own `FromSample`
/// covers the pairs the default input can produce.
trait IntoSample<T> {
    fn into_sample(self) -> T;
}

impl IntoSample<i16> for i16 {
    fn into_sample(self) -> i16 {
        self
    }
}

impl IntoSample<i16> for f32 {
    fn into_sample(self) -> i16 {
        (self.clamp(-1.0, 1.0) * 32_767.0) as i16
    }
}

impl IntoSample<i16> for u16 {
    fn into_sample(self) -> i16 {
        (i32::from(self) - 32_768) as i16
    }
}

/// The playout half: a rodio sink that accepts engine-clock frames as they
/// arrive and mixes them into the platform output.
pub struct Playback {
    sink: rodio::Player,
    #[expect(dead_code, reason = "keeps the output stream alive")]
    mixer: rodio::MixerDeviceSink,
}

impl Playback {
    /// Opens the default output.
    pub fn open() -> anyhow::Result<Self> {
        let mixer = rodio::DeviceSinkBuilder::open_default_sink()
            .map_err(|error| anyhow::anyhow!("the speaker could not be opened: {error}"))?;
        let sink = rodio::Player::connect_new(mixer.mixer());
        Ok(Self { sink, mixer })
    }

    /// Plays one 16 kHz mono frame. The sink grows only by what arrives:
    /// the caller drains at the engine's pace, and a stalled speaker is the
    /// sidecar's dropping policy, not a queue here.
    pub fn play(&self, frame: Vec<i16>) {
        let floats: Vec<f32> = frame
            .iter()
            .map(|sample| f32::from(*sample) / 32_768.0)
            .collect();
        let source = rodio::buffer::SamplesBuffer::new(
            std::num::NonZeroU16::new(1).expect("one channel"),
            RATE.try_into().expect("16 kHz fits a NonZeroU32"),
            floats,
        );
        // (channels, rate, samples) — checked against rodio 0.23 below if it still complains
        self.sink.append(source);
    }

    /// Empties whatever is pending: the right move when a call ends.
    pub fn flush(&self) {
        self.sink.clear();
    }
}
