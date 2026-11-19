//! The sounds of the messages: the one of a message that arrives and the one
//! of a message that leaves.
//!
//! The profile picks a sound per event. A sound is a built-in of this module
//! (`builtin:<id>`), a file of the user (`file:<path>`) or silence (`none`).
//! The built-in ones are synthesized here, so the app does not carry audio
//! files and every platform plays the same thing.
//!
//! The speaker is opened by a thread of its own, once, and every sound is a
//! request in a queue: the interface never waits for the audio device, the same
//! way it never waits for the notification center.

use std::num::{NonZeroU16, NonZeroU32};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use rodio::Source as _;

/// The rate of the built-in sounds. The device resamples when it needs to.
const RATE: u32 = 48_000;

/// The longest piece of a file that a message plays: a song picked by mistake
/// must not take over the speaker.
const MAX_FILE_SECONDS: u64 = 4;

/// The fade at the start of a built-in sound, in seconds: without it the wave
/// begins away from zero and the speaker clicks.
const ATTACK_SECONDS: f32 = 0.003;

/// Which message a sound belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SoundEvent {
    /// A message that arrives from somebody else.
    Received,
    /// A message that this window sends.
    Sent,
    /// A toast of the system that just appeared.
    Notification,
}

impl SoundEvent {
    /// The name of the event in the settings file.
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Received => "received",
            Self::Sent => "sent",
            Self::Notification => "notification",
        }
    }
}

/// The sounds that ship with the app, synthesized so no audio file is needed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Builtin {
    /// A short blip that falls: the pop of a message.
    Pop,
    /// Two partials of a bell.
    Ding,
    /// A single high note.
    Ping,
    /// Three notes that go up.
    Chime,
    /// Three warm notes of a marimba that come down.
    Marimba,
    /// A bubble that rises twice.
    Bubble,
    /// A low bell with two overtones.
    Bell,
    /// A short wooden knock.
    Wood,
    /// Two soft pulses with a gap between them.
    Pulse,
    /// Three bright notes that sparkle.
    Sparkle,
}

impl Builtin {
    /// All the built-in sounds, in the order of the settings.
    pub(crate) const ALL: [Self; 10] = [
        Self::Pop,
        Self::Ding,
        Self::Ping,
        Self::Chime,
        Self::Marimba,
        Self::Bubble,
        Self::Bell,
        Self::Wood,
        Self::Pulse,
        Self::Sparkle,
    ];

    /// The name of the sound in the settings file.
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Pop => "pop",
            Self::Ding => "ding",
            Self::Ping => "ping",
            Self::Chime => "chime",
            Self::Marimba => "marimba",
            Self::Bubble => "bubble",
            Self::Bell => "bell",
            Self::Wood => "wood",
            Self::Pulse => "pulse",
            Self::Sparkle => "sparkle",
        }
    }

    /// The sound of a stored name.
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|sound| sound.id() == id)
    }

    /// The samples of the sound: mono, [`RATE`] and with the fade at the end.
    fn samples(self) -> Vec<f32> {
        match self {
            Self::Pop => tone(880.0, 520.0, 0.075, 2.2, 0.30),
            Self::Ding => {
                let mut bell = tone(1318.5, 1318.5, 0.32, 2.6, 0.20);
                mix(&mut bell, &tone(1975.5, 1975.5, 0.32, 3.4, 0.10));
                bell
            }
            Self::Ping => tone(1568.0, 1568.0, 0.09, 1.6, 0.24),
            Self::Chime => {
                let mut notes = tone(523.25, 523.25, 0.14, 2.0, 0.18);
                notes.extend(tone(659.25, 659.25, 0.14, 2.0, 0.18));
                notes.extend(tone(783.99, 783.99, 0.20, 2.0, 0.18));
                notes
            }
            Self::Marimba => {
                let mut notes = tone(523.25, 523.25, 0.09, 3.0, 0.17);
                notes.extend(tone(440.0, 440.0, 0.09, 3.0, 0.17));
                notes.extend(tone(349.23, 349.23, 0.12, 3.0, 0.17));
                notes
            }
            Self::Bubble => {
                let mut bubbles = tone(300.0, 900.0, 0.12, 2.4, 0.22);
                bubbles.extend(tone(500.0, 1200.0, 0.08, 2.6, 0.18));
                bubbles
            }
            Self::Bell => {
                let mut bell = tone(440.0, 440.0, 0.40, 2.8, 0.18);
                mix(&mut bell, &tone(880.0, 880.0, 0.40, 3.2, 0.08));
                mix(&mut bell, &tone(1320.0, 1320.0, 0.40, 3.6, 0.04));
                bell
            }
            Self::Wood => {
                let mut knock = tone(180.0, 90.0, 0.06, 4.0, 0.34);
                mix(&mut knock, &tone(360.0, 180.0, 0.06, 4.5, 0.10));
                knock
            }
            Self::Pulse => {
                let mut pulses = tone(880.0, 880.0, 0.05, 2.0, 0.22);
                pulses.extend(vec![0.0; (0.05 * RATE as f32) as usize]);
                pulses.extend(tone(880.0, 880.0, 0.05, 2.0, 0.22));
                pulses
            }
            Self::Sparkle => {
                let mut notes = tone(1046.5, 1046.5, 0.05, 2.0, 0.16);
                notes.extend(tone(1318.5, 1318.5, 0.05, 2.0, 0.16));
                notes.extend(tone(1568.0, 1568.0, 0.07, 2.0, 0.16));
                notes
            }
        }
    }
}

/// What the profile chose for one event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Choice {
    /// The message is silent.
    None,
    /// One of the sounds of the app.
    Builtin(Builtin),
    /// A file of the user.
    File(PathBuf),
}

impl Choice {
    /// The choice of a stored value.
    ///
    /// An empty, unknown or broken value gives `None`: the caller decides
    /// whether that means the built-in of the event or silence.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        match text {
            "" => None,
            "none" => Some(Self::None),
            other => {
                if let Some(id) = other.strip_prefix("builtin:") {
                    Builtin::from_id(id).map(Self::Builtin)
                } else if let Some(path) = other.strip_prefix("file:") {
                    (!path.trim().is_empty()).then(|| Self::File(PathBuf::from(path)))
                } else {
                    None
                }
            }
        }
    }

    /// The choice of the settings, with the built-in of the event when the
    /// value is empty or one this build does not know.
    pub(crate) fn from_setting(text: &str, fallback: Builtin) -> Self {
        Self::parse(text).unwrap_or(Self::Builtin(fallback))
    }

    /// The value the settings file keeps.
    pub(crate) fn encode(&self) -> String {
        match self {
            Self::None => "none".to_string(),
            Self::Builtin(builtin) => format!("builtin:{}", builtin.id()),
            Self::File(path) => format!("file:{}", path.display()),
        }
    }

    /// The name of the file, when the choice is one.
    pub(crate) fn file_name(&self) -> Option<String> {
        let Self::File(path) = self else {
            return None;
        };
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .or_else(|| Some(path.display().to_string()))
    }
}

/// The facts that decide whether a message that arrives plays its sound.
///
/// The silence of a muted chat, of "Do Not Disturb" and of a history that is
/// still loading are the same rules of the toasts; the difference is that a
/// sound plays even when the conversation is in front, because the user asked
/// for the sound of the message, not for the popup.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Incoming {
    /// The profile plays sounds.
    pub(crate) enabled: bool,
    /// The first sync finished: the history is not news.
    pub(crate) ready: bool,
    /// The message is one of the user.
    pub(crate) from_me: bool,
    /// The message was revoked.
    pub(crate) revoked: bool,
    /// The message was not in the store yet.
    pub(crate) is_new: bool,
    /// How long ago the message was written, in milliseconds.
    pub(crate) age_ms: i64,
    /// The chat is muted.
    pub(crate) muted: bool,
    /// "Do Not Disturb" is on.
    pub(crate) do_not_disturb: bool,
}

impl Incoming {
    /// Whether the message plays its sound, given the oldest message that is
    /// still news.
    pub(crate) fn plays(&self, max_age_ms: i64) -> bool {
        self.enabled
            && self.ready
            && !self.from_me
            && !self.revoked
            && self.is_new
            && !self.muted
            && !self.do_not_disturb
            && self.age_ms <= max_age_ms
    }
}

/// One sound waiting for the thread of the speaker.
enum Request {
    Builtin(Builtin),
    File(PathBuf),
}

/// The queue of the sounds.
pub(crate) struct Sounds {
    sender: Option<Sender<Request>>,
}

impl Sounds {
    /// Starts the thread that owns the speaker.
    pub(crate) fn new() -> Self {
        let (sender, receiver) = mpsc::channel::<Request>();
        let started = std::thread::Builder::new()
            .name("whatfast-sounds".to_string())
            .spawn(move || run(receiver));
        Self {
            // A thread that cannot start leaves the app without sounds, which
            // is better than a panic when a message arrives.
            sender: started.ok().map(|_| sender),
        }
    }

    /// Queues one sound.
    pub(crate) fn play(&self, choice: &Choice) {
        let Some(sender) = &self.sender else {
            return;
        };
        let request = match choice {
            Choice::None => return,
            Choice::Builtin(builtin) => Request::Builtin(*builtin),
            Choice::File(path) => Request::File(path.clone()),
        };
        let _ = sender.send(request);
    }
}

impl Default for Sounds {
    fn default() -> Self {
        Self::new()
    }
}

/// The body of the thread of the speaker.
///
/// The device lives here and opens with the first sound: a profile that never
/// plays one never takes the speaker.
fn run(receiver: mpsc::Receiver<Request>) {
    let mut device: Option<rodio::MixerDeviceSink> = None;
    // The sounds that are alive. Dropping the player stops its sound, so
    // playing one clears the previous one: a burst of messages cuts the old
    // blip instead of piling up noise.
    let mut playing: Vec<rodio::Player> = Vec::new();
    while let Ok(request) = receiver.recv() {
        if device.is_none() {
            match rodio::DeviceSinkBuilder::open_default_sink() {
                Ok(sink) => device = Some(sink),
                Err(error) => {
                    eprintln!("sounds: there is no audio device: {error}");
                    continue;
                }
            }
        }
        let Some(mixer) = device.as_ref().map(|sink| sink.mixer()) else {
            continue;
        };
        match request {
            Request::Builtin(builtin) => {
                playing.clear();
                let next = rodio::Player::connect_new(mixer);
                next.append(rodio::buffer::SamplesBuffer::new(
                    NonZeroU16::new(1).expect("one channel"),
                    NonZeroU32::new(RATE).expect("48 kHz"),
                    builtin.samples(),
                ));
                playing.push(next);
            }
            // A file that cannot be opened leaves the previous sound alone: a
            // broken pick is not a reason to cut what is playing.
            Request::File(path) => match open_sound(&path) {
                Ok(source) => {
                    playing.clear();
                    let next = rodio::Player::connect_new(mixer);
                    next.append(source.take_duration(Duration::from_secs(MAX_FILE_SECONDS)));
                    playing.push(next);
                }
                Err(error) => eprintln!("sounds: {error}"),
            },
        }
    }
}

/// Opens a sound file of the user. The decoder keeps the rate and the
/// channels of the file: the mixer of the device adapts them.
fn open_sound(path: &Path) -> Result<rodio::Decoder<std::io::BufReader<std::fs::File>>, String> {
    let file = std::fs::File::open(path)
        .map_err(|error| format!("the sound {} could not be opened: {error}", path.display()))?;
    rodio::Decoder::new(std::io::BufReader::new(file))
        .map_err(|error| format!("the sound {} could not be decoded: {error}", path.display()))
}

/// One tone that glides from `from_hz` to `to_hz` with a fade to zero.
fn tone(from_hz: f32, to_hz: f32, seconds: f32, decay: f32, gain: f32) -> Vec<f32> {
    let count = (seconds * RATE as f32) as usize;
    // The attack: a sound that starts in the middle of a wave clicks.
    let attack = (ATTACK_SECONDS * RATE as f32).max(1.0);
    let mut phase = 0.0_f32;
    (0..count)
        .map(|index| {
            let progress = index as f32 / count.max(1) as f32;
            let frequency = from_hz + (to_hz - from_hz) * progress;
            let sample = phase.sin();
            phase += std::f32::consts::TAU * frequency / RATE as f32;
            // The envelope ends at zero, so a cut never clicks.
            let envelope = (1.0 - progress).powf(decay) * (index as f32 / attack).min(1.0);
            sample * envelope * gain
        })
        .collect()
}

/// Adds `other` to `target`, sample by sample.
fn mix(target: &mut [f32], other: &[f32]) {
    for (sample, extra) in target.iter_mut().zip(other) {
        *sample += extra;
    }
}

