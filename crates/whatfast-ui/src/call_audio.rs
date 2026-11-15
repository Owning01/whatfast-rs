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

use std::sync::atomic::{AtomicBool, Ordering};

use cpal::traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _};
use cpal::{SampleFormat, StreamConfig};

/// The engine's audio clock.
pub const RATE: u32 = 16_000;

/// The engine reads frames of exactly 60 ms: 960 samples at 16 kHz.
const FRAME_SAMPLES: usize = 960;

/// The live capture: drops into `frames` as engine-clock i16 chunks.
pub struct Capture {
    _stream: cpal::Stream,
    frames: async_channel::Receiver<[i16; FRAME_SAMPLES]>,
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
        let (frames_tx, frames_rx) = async_channel::bounded::<[i16; FRAME_SAMPLES]>(4);
        let oldest = frames_rx.clone();
        let config: StreamConfig = supported.into();
        let format = supported.sample_format();
        let stream = match format {
            SampleFormat::F32 => capture_build::<f32>(
                &device,
                config,
                device_rate,
                channels,
                frames_tx,
                oldest,
            ),
            SampleFormat::I16 => capture_build::<i16>(
                &device,
                config,
                device_rate,
                channels,
                frames_tx,
                oldest,
            ),
            SampleFormat::U16 => capture_build::<u16>(
                &device,
                config,
                device_rate,
                channels,
                frames_tx,
                oldest,
            ),
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

    /// Drains the next engine-clock frame into a caller-reused buffer without allocating.
    pub fn try_take_into(&self, out: &mut Vec<i16>) -> bool {
        let Ok(frame) = self.frames.try_recv() else {
            return false;
        };
        out.clear();
        out.extend_from_slice(frame.as_slice());
        true
    }

    /// Silences the outgoing frames without closing the microphone.
    pub fn set_muted(&self, muted: bool) {
        MUTE.store(muted, Ordering::SeqCst);
    }
}

/// Pure-Rust NLMS Acoustic Echo Canceller + simple RMS noise gate for call audio.
pub struct EchoCanceller {
    taps: usize,
    weights: Vec<f32>,
    far_history: Vec<f32>,
    cursor: usize,
    step_size: f32,
    gate_threshold: f32,
}

impl EchoCanceller {
    pub fn new(taps: usize) -> Self {
        Self {
            taps: taps.max(32),
            weights: vec![0.0; taps.max(32)],
            far_history: vec![0.0; taps.max(32)],
            cursor: 0,
            step_size: 0.15,
            gate_threshold: 0.008,
        }
    }

    /// Cancels echo from `near` using the `far` reference frame in-place.
    pub fn process(&mut self, near: &mut [i16], far: &[i16]) {
        let len = near.len().min(far.len());
        let mut energy = 0.0f32;
        for i in 0..len {
            let x = f32::from(far[i]) / 32768.0;
            let d = f32::from(near[i]) / 32768.0;
            self.far_history[self.cursor] = x;
            let mut y = 0.0f32;
            let mut norm = 1e-6f32;
            for k in 0..self.taps {
                let idx = (self.cursor + self.taps - k) % self.taps;
                let h = self.far_history[idx];
                y += self.weights[k] * h;
                norm += h * h;
            }
            let err = d - y;
            let mu = self.step_size / norm;
            for k in 0..self.taps {
                let idx = (self.cursor + self.taps - k) % self.taps;
                self.weights[k] += mu * err * self.far_history[idx];
            }
            self.cursor = (self.cursor + 1) % self.taps;
            energy += err * err;
            near[i] = (err.clamp(-1.0, 1.0) * 32767.0) as i16;
        }
        if len > 0 && (energy / len as f32).sqrt() < self.gate_threshold {
            for s in &mut near[..len] {
                *s = (*s as f32 * 0.25) as i16;
            }
        }
    }
}

static MUTE: AtomicBool = AtomicBool::new(false);

/// The resampler's state: the device geometry and the buffers it reuses.
///
/// The callback owns this value, so it never locks a mutex and never allocates
/// once the buffers reached their size. The frame that leaves is a fixed array:
/// the queue stores it inline and the heap stays out of the audio thread.
struct Resampler {
    rate: u32,
    channels: usize,
    buffers: CaptureBuffers,
}

/// The buffers of the capture, reused on every callback.
#[derive(Default)]
struct CaptureBuffers {
    /// Device-rate mono samples left over between callbacks.
    carry: Vec<f32>,
    /// Engine-clock samples that wait for a whole frame of 960.
    pending: Vec<i16>,
    /// The mono samples of the callback being converted.
    mono: Vec<f32>,
    /// The engine-clock samples of the callback being resampled.
    engine: Vec<i16>,
    /// The position of the next sample, relative to the start of `carry`.
    /// It keeps its fraction: a 48 kHz device steps by three, so the
    /// fraction is not always zero.
    position: f64,
}

impl Resampler {
    fn new(rate: u32, channels: usize) -> Self {
        Self {
            rate,
            channels,
            buffers: CaptureBuffers::default(),
        }
    }

    /// Converts one device callback into engine-clock mono frames of
    /// exactly 960 samples (60 ms), the size the sidecar's pipes expect.
    fn accept(
        &mut self,
        samples: &[i16],
        out: &async_channel::Sender<[i16; FRAME_SAMPLES]>,
        oldest: &async_channel::Receiver<[i16; FRAME_SAMPLES]>,
    ) {
        let channels = self.channels.max(1);
        let rate = self.rate;
        let muted = MUTE.load(Ordering::Relaxed);
        let CaptureBuffers {
            carry,
            pending,
            mono,
            engine,
            position,
        } = &mut self.buffers;
        mono.clear();
        mono.extend(samples.chunks(channels).map(|chunk| {
            let sum: f32 = chunk.iter().map(|sample| f32::from(*sample)).sum();
            sum / channels as f32
        }));
        carry.extend_from_slice(mono);
        let ratio = f64::from(rate) / f64::from(RATE);
        // The position is relative to the first sample of the carry, and it
        // keeps its fraction.
        engine.clear();
        while ((*position as usize) + 1) < carry.len() {
            let index = *position as usize;
            let fraction = (*position - index as f64) as f32;
            let sample = carry[index] * (1.0 - fraction) + carry[index + 1] * fraction;
            let muted_sample: i16 = if muted {
                0
            } else {
                (sample.clamp(-1.0, 1.0) * 32_767.0) as i16
            };
            engine.push(muted_sample);
            *position += ratio;
        }
        // The step can pass the end of the carry: the samples before the next
        // position are consumed, and the position becomes relative to the new
        // first sample. Draining the raw position would cut more than the
        // carry holds and panic on the next block.
        let consumed = (*position as usize).min(carry.len());
        carry.drain(..consumed);
        *position -= consumed as f64;
        if engine.is_empty() {
            return;
        }
        // The engine only accepts frames of exactly 960 samples: the samples
        // of a short callback wait here for the next one.
        pending.extend_from_slice(engine);
        while pending.len() >= FRAME_SAMPLES {
            let mut frame = [0_i16; FRAME_SAMPLES];
            frame.copy_from_slice(&pending[..FRAME_SAMPLES]);
            pending.drain(..FRAME_SAMPLES);
            match out.try_send(frame) {
                Ok(()) => {}
                Err(async_channel::TrySendError::Full(frame)) => {
                    // The reader is behind: the oldest frame leaves so the
                    // newest one is what plays next.
                    let _ = oldest.try_recv();
                    let _ = out.try_send(frame);
                }
                Err(async_channel::TrySendError::Closed(_)) => return,
            }
        }
    }
}

fn capture_build<T>(
    device: &cpal::Device,
    config: StreamConfig,
    rate: u32,
    channels: usize,
    frames: async_channel::Sender<[i16; FRAME_SAMPLES]>,
    oldest: async_channel::Receiver<[i16; FRAME_SAMPLES]>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: cpal::SizedSample + IntoSample<i16>,
{
    // The scratch buffer and the resampler live in the callback: no lock and,
    // after the first callbacks, no allocation on the audio thread.
    let mut converted: Vec<i16> = Vec::new();
    let mut resampler = Resampler::new(rate, channels);
    device.build_input_stream(
        config,
        move |samples: &[T], _: &cpal::InputCallbackInfo| {
            converted.clear();
            converted.extend(samples.iter().map(|sample| sample.into_sample()));
            resampler.accept(&converted, &frames, &oldest);
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

#[cfg(test)]
mod tests {
    use super::{FRAME_SAMPLES, MUTE, Resampler};
    use std::sync::atomic::Ordering;

    #[test]
    fn the_capture_hands_the_engine_frames_of_sixty_milliseconds() {
        // The engine rejects anything that is not 960 samples. A device that
        // calls back with 1024 samples at 48 kHz produces shorter chunks, so
        // the capture has to wait and join them.
        MUTE.store(false, Ordering::SeqCst);
        let (tx, rx) = async_channel::bounded(64);
        let oldest = rx.clone();
        let mut resampler = Resampler::new(48_000, 1);
        let block = vec![0i16; 1024];
        for _ in 0..10 {
            resampler.accept(&block, &tx, &oldest);
        }
        let mut frames = Vec::new();
        while let Ok(frame) = rx.try_recv() {
            frames.push(frame);
        }
        assert!(!frames.is_empty());
        for frame in &frames {
            assert_eq!(frame.len(), FRAME_SAMPLES);
        }
        // 10 blocks of 1024 at 48 kHz are 3413 samples at 16 kHz: three whole
        // frames, and the rest stays for the next callback.
        assert_eq!(frames.len(), 3);
    }
}
