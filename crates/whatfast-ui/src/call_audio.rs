//! The desktop's half of a call's audio: the microphone in, the speaker
//! out, both at the engine's clock (16 kHz mono i16).
//!
//! The sidecar owns the engine and the relay; this module only moves
//! frames. The capture opens the platform's default input (the same cpal
//! path the voice notes use), normalizes every sample format to `-1..1`,
//! removes the DC offset and the frequencies above the engine's Nyquist,
//! and resamples down to 16 kHz. The playback hands frames to rodio at 16 kHz
//! and lets the platform mixer do the rest.
//!
//! There is deliberately no echo cancellation in this wave: the engine has
//! its own concealment, and the practical mitigation is the same one
//! WhatsApp Web's first calls shipped with — use a headset.

use std::sync::atomic::{AtomicBool, Ordering};

use cpal::traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _};
use cpal::{SampleFormat, StreamConfig};

/// The engine's audio clock.
pub const RATE: u32 = 16_000;

/// The engine reads frames of exactly 60 ms: 960 samples at 16 kHz.
const FRAME_SAMPLES: usize = 960;

/// Seconds of microphone audio the queue holds: eight frames, 480 ms.
const QUEUE_FRAMES: usize = 8;

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
        let (frames_tx, frames_rx) =
            async_channel::bounded::<[i16; FRAME_SAMPLES]>(QUEUE_FRAMES);
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
            SampleFormat::F64 => capture_build::<f64>(
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
            SampleFormat::I32 => capture_build::<i32>(
                &device,
                config,
                device_rate,
                channels,
                frames_tx,
                oldest,
            ),
            SampleFormat::U32 => capture_build::<u32>(
                &device,
                config,
                device_rate,
                channels,
                frames_tx,
                oldest,
            ),
            SampleFormat::I8 => capture_build::<i8>(
                &device,
                config,
                device_rate,
                channels,
                frames_tx,
                oldest,
            ),
            SampleFormat::U8 => capture_build::<u8>(
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

    /// The frames of the capture. When the capture drops, the channel closes
    /// and a receive that waits on it ends at once, without a timer.
    pub fn receiver(&self) -> async_channel::Receiver<[i16; FRAME_SAMPLES]> {
        self.frames.clone()
    }

    /// Silences the outgoing frames without closing the microphone.
    pub fn set_muted(&self, muted: bool) {
        MUTE.store(muted, Ordering::SeqCst);
    }
}

static MUTE: AtomicBool = AtomicBool::new(false);

/// A Direct Form II Transposed biquad. The coefficients and the state are
/// f64 so a device rate of 192 kHz does not lose precision; the sample that
/// goes in and the one that comes out are the normalized f32 of the wave.
#[derive(Clone, Copy)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Biquad {
    fn highpass(frequency: f64, rate: f64, q: f64) -> Self {
        let omega = std::f64::consts::TAU * frequency / rate;
        let cosine = omega.cos();
        let alpha = omega.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self::new(
            ((1.0 + cosine) / 2.0) / a0,
            -(1.0 + cosine) / a0,
            ((1.0 + cosine) / 2.0) / a0,
            (-2.0 * cosine) / a0,
            (1.0 - alpha) / a0,
        )
    }

    fn lowpass(frequency: f64, rate: f64, q: f64) -> Self {
        let omega = std::f64::consts::TAU * frequency / rate;
        let cosine = omega.cos();
        let alpha = omega.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self::new(
            ((1.0 - cosine) / 2.0) / a0,
            (1.0 - cosine) / a0,
            ((1.0 - cosine) / 2.0) / a0,
            (-2.0 * cosine) / a0,
            (1.0 - alpha) / a0,
        )
    }

    fn new(b0: f64, b1: f64, b2: f64, a1: f64, a2: f64) -> Self {
        Self {
            b0,
            b1,
            b2,
            a1,
            a2,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    /// One sample. No allocation, no branch: the state lives in the struct.
    fn process(&mut self, sample: f64) -> f64 {
        let out = self.b0 * sample + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = sample;
        self.y2 = self.y1;
        self.y1 = out;
        out
    }
}

/// The filters of one capture, chosen from the rate of the device.
struct Filters {
    /// Removes the offset and the rumble below the voice.
    highpass: Option<Biquad>,
    /// Two Butterworth sections: a fourth-order roll-off before decimating.
    lowpass_1: Option<Biquad>,
    lowpass_2: Option<Biquad>,
}

impl Filters {
    fn new(rate: u32) -> Self {
        let rate_f = f64::from(rate);
        let highpass = (rate > 200)
            .then(|| Biquad::highpass(80.0, rate_f, std::f64::consts::FRAC_1_SQRT_2));
        // The corner is below Nyquist of the engine and below 42 % of the
        // device rate, whichever is tighter. A tone that would fold over the
        // voice is gone before the linear interpolation drops samples.
        let corner = 7_000.0f64.min(0.42 * rate_f);
        let lowpass = (rate > RATE).then(|| {
            (
                Biquad::lowpass(corner, rate_f, 0.541_196_1),
                Biquad::lowpass(corner, rate_f, 1.306_563),
            )
        });
        Self {
            highpass,
            lowpass_1: lowpass.as_ref().map(|(first, _)| *first),
            lowpass_2: lowpass.map(|(_, second)| second),
        }
    }

    fn apply(&mut self, sample: f32) -> f32 {
        let mut value = f64::from(sample);
        if let Some(filter) = &mut self.highpass {
            value = filter.process(value);
        }
        if let Some(filter) = &mut self.lowpass_1 {
            value = filter.process(value);
        }
        if let Some(filter) = &mut self.lowpass_2 {
            value = filter.process(value);
        }
        // A resonant device rate can ring for a moment. Keep the excursion
        // bounded before it reaches the fixed-point conversion.
        value.clamp(-4.0, 4.0) as f32
    }
}

/// The resampler's state: the device geometry and the buffers it reuses.
///
/// The callback owns this value, so it never locks a mutex and never allocates
/// once the buffers reached their size. The frame that leaves is a fixed array:
/// the queue stores it inline and the heap stays out of the audio thread.
struct Resampler {
    rate: u32,
    channels: usize,
    filters: Filters,
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
    /// It keeps its fraction: a 44.1 kHz device steps by 2.75625.
    position: f64,
}

impl Resampler {
    fn new(rate: u32, channels: usize) -> Self {
        Self {
            rate,
            channels,
            filters: Filters::new(rate),
            buffers: CaptureBuffers::default(),
        }
    }

    /// Converts one device callback into engine-clock mono frames of
    /// exactly 960 samples (60 ms), the size the sidecar's pipes expect.
    fn accept(
        &mut self,
        samples: &[f32],
        out: &async_channel::Sender<[i16; FRAME_SAMPLES]>,
        oldest: &async_channel::Receiver<[i16; FRAME_SAMPLES]>,
    ) {
        let channels = self.channels.max(1);
        let rate = self.rate;
        let muted = MUTE.load(Ordering::Relaxed);
        let filters = &mut self.filters;
        let CaptureBuffers {
            carry,
            pending,
            mono,
            engine,
            position,
        } = &mut self.buffers;
        // Every sample is already normalized by ToNormalizedSample. Mix the
        // channels, filter in f32, and only turn into i16 at the very end.
        mono.clear();
        for chunk in samples.chunks(channels) {
            let mut sum = 0.0f64;
            for sample in chunk {
                sum += f64::from(*sample);
            }
            mono.push(filters.apply((sum / channels as f64) as f32));
        }
        carry.extend_from_slice(mono);
        let ratio = f64::from(rate) / f64::from(RATE);
        engine.clear();
        while ((*position as usize) + 1) < carry.len() {
            let index = *position as usize;
            let fraction = (*position - index as f64) as f32;
            let sample = carry[index] * (1.0 - fraction) + carry[index + 1] * fraction;
            let muted_sample = if muted {
                0
            } else {
                (sample.clamp(-1.0, 1.0) * 32_767.0).round() as i16
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
    T: cpal::SizedSample + ToNormalizedSample,
{
    // The scratch buffer and the resampler live in the callback: no lock and,
    // after the first callbacks, no allocation on the audio thread.
    let mut converted: Vec<f32> = Vec::new();
    let mut resampler = Resampler::new(rate, channels);
    device.build_input_stream(
        config,
        move |samples: &[T], _: &cpal::InputCallbackInfo| {
            converted.clear();
            converted.extend(samples.iter().map(|sample| sample.to_normalized()));
            resampler.accept(&converted, &frames, &oldest);
        },
        move |error| eprintln!("microphone error: {error}"),
        None,
    )
}

/// The conversion of every sample format of the device to the `-1..1` range.
/// The same table the voice notes use (`recording.rs`), so a call and a note
/// record the same wave from the same microphone.
trait ToNormalizedSample {
    fn to_normalized(self) -> f32;
}

impl ToNormalizedSample for f32 {
    fn to_normalized(self) -> f32 {
        self.clamp(-1.0, 1.0)
    }
}

impl ToNormalizedSample for f64 {
    fn to_normalized(self) -> f32 {
        (self as f32).clamp(-1.0, 1.0)
    }
}

impl ToNormalizedSample for i16 {
    fn to_normalized(self) -> f32 {
        self as f32 / 32_768.0
    }
}

impl ToNormalizedSample for u16 {
    fn to_normalized(self) -> f32 {
        (self as f32 - 32_768.0) / 32_768.0
    }
}

impl ToNormalizedSample for i32 {
    fn to_normalized(self) -> f32 {
        self as f32 / 2_147_483_648.0
    }
}

impl ToNormalizedSample for u32 {
    fn to_normalized(self) -> f32 {
        (self as f64 - 2_147_483_648.0) as f32 / 2_147_483_648.0
    }
}

impl ToNormalizedSample for i8 {
    fn to_normalized(self) -> f32 {
        self as f32 / 128.0
    }
}

impl ToNormalizedSample for u8 {
    fn to_normalized(self) -> f32 {
        (self as f32 - 128.0) / 128.0
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
        self.sink.append(source);
    }

    /// Empties whatever is pending: the right move when a call ends.
    pub fn flush(&self) {
        self.sink.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::{FRAME_SAMPLES, MUTE, Resampler, ToNormalizedSample};
    use std::sync::atomic::Ordering;
    use std::sync::Mutex;

    /// The mute is a process-wide atomic. Serialize the tests that set it so
    /// one test cannot silence another one halfway through.
    static MUTE_LOCK: Mutex<()> = Mutex::new(());

    /// Collects all the engine frames of a run of device-rate samples.
    fn render(rate: u32, samples: &[f32]) -> Vec<i16> {
        let (tx, rx) = async_channel::bounded(256);
        let oldest = rx.clone();
        let mut resampler = Resampler::new(rate, 1);
        for chunk in samples.chunks(1024) {
            resampler.accept(chunk, &tx, &oldest);
        }
        let mut out = Vec::new();
        while let Ok(frame) = rx.try_recv() {
            out.extend_from_slice(&frame);
        }
        out
    }

    #[test]
    fn the_capture_hands_the_engine_frames_of_sixty_milliseconds() {
        let _guard = MUTE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        // The engine rejects anything that is not 960 samples. A device that
        // calls back with 1024 samples at 48 kHz produces shorter chunks, so
        // the capture has to wait and join them.
        MUTE.store(false, Ordering::SeqCst);
        let (tx, rx) = async_channel::bounded(64);
        let oldest = rx.clone();
        let mut resampler = Resampler::new(48_000, 1);
        let block = vec![0_f32; 1024];
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

    /// Every format of the microphone starts in the same normalized range.
    #[test]
    fn every_sample_format_normalizes_to_the_same_range() {
        assert_eq!(0_i16.to_normalized(), 0.0);
        assert!((i16::MAX.to_normalized() - 1.0).abs() < 0.001);
        assert!((i16::MIN.to_normalized() + 1.0).abs() < 0.001);
        assert!(u16::MAX.to_normalized() > 0.99);
        assert!((0_u8.to_normalized() + 1.0).abs() < 0.001);
        assert!((255_u8.to_normalized() - 0.992_187_5).abs() < 0.001);
        assert!((i8::MIN.to_normalized() + 1.0).abs() < 0.001);
        assert!((i32::MAX.to_normalized() - 1.0).abs() < 0.001);
        assert!((u32::MAX.to_normalized() - 1.0).abs() < 0.001);
    }

    /// The old code saturated every nonzero sample to the i16 limit. A quiet
    /// 440 Hz tone must come out at about a quarter of full scale, not at a
    /// square wave of maximum volume.
    #[test]
    fn a_quiet_sine_keeps_its_amplitude_and_its_shape() {
        let _guard = MUTE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        MUTE.store(false, Ordering::SeqCst);
        let rate = 48_000_f64;
        let samples: Vec<f32> = (0..(0.3 * rate) as usize)
            .map(|index| {
                (0.25 * (std::f64::consts::TAU * 440.0 * index as f64 / rate).sin()) as f32
            })
            .collect();
        let rendered = render(48_000, &samples);
        // The high-pass needs about 50 ms to settle: skip the first 800
        // engine samples so the transient does not skew the amplitude.
        let settled = &rendered[800..];
        let peak = settled.iter().map(|sample| sample.abs()).max().unwrap() as f32;
        assert!(
            (7_000.0..=9_500.0).contains(&peak),
            "peak {peak} should be near 8192, not a saturated 32767"
        );
        let rms = (settled
            .iter()
            .map(|sample| f64::from(*sample) * f64::from(*sample))
            .sum::<f64>()
            / settled.len() as f64)
            .sqrt();
        let expected = 0.25 / 2.0_f64.sqrt() * 32_767.0;
        assert!(
            (rms - expected).abs() / expected < 0.15,
            "rms {rms} differs from {expected}"
        );
        // Correlate against a pure tone, allowing for the phase of the
        // high-pass: a shape that survived scores near one at some offset.
        let best = (0..64)
            .map(|shift| {
                let reference: Vec<f64> = (shift..shift + 1_600)
                    .map(|index| {
                        (std::f64::consts::TAU * 440.0 * index as f64 / 16_000.0).sin()
                    })
                    .collect();
                let wave: Vec<f64> = settled[..1_600]
                    .iter()
                    .map(|sample| f64::from(*sample))
                    .collect();
                let dot: f64 = wave.iter().zip(&reference).map(|(a, b)| a * b).sum();
                let energy: f64 = wave.iter().map(|a| a * a).sum::<f64>()
                    * reference.iter().map(|b| b * b).sum::<f64>();
                dot / energy.sqrt()
            })
            .fold(0.0_f64, f64::max);
        assert!(best > 0.95, "correlation {best} with a pure 440 Hz tone");
    }

    /// A 14 kHz tone at 48 kHz would fold down to 2 kHz if the capture just
    /// dropped two of every three samples. The low-pass has to remove it.
    #[test]
    fn a_tone_above_nyquist_is_filtered_before_decimating() {
        let _guard = MUTE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        MUTE.store(false, Ordering::SeqCst);
        let rate = 48_000_f64;
        let samples: Vec<f32> = (0..(0.3 * rate) as usize)
            .map(|index| {
                (0.8 * (std::f64::consts::TAU * 14_000.0 * index as f64 / rate).sin()) as f32
            })
            .collect();
        let rendered = render(48_000, &samples);
        let settled = &rendered[800..];
        let peak = settled.iter().map(|sample| sample.abs()).max().unwrap() as f32;
        assert!(
            peak < 3_000.0,
            "peak {peak} should be well under 3000 after anti-aliasing"
        );
    }

    /// Muting is exact: every sample of the outgoing frame is zero.
    #[test]
    fn mute_silences_every_sample() {
        let _guard = MUTE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        MUTE.store(true, Ordering::SeqCst);
        let samples = vec![0.5_f32; 48_000];
        let rendered = render(48_000, &samples);
        assert!(!rendered.is_empty());
        assert!(rendered.iter().all(|sample| *sample == 0));
        MUTE.store(false, Ordering::SeqCst);
    }
}
