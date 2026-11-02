//! The video half of a call: the camera leaves as H.264 and the peer's
//! access units come back as pictures.
//!
//! The library only carries pre-encoded Annex-B access units, so this module
//! asks `ffmpeg` for the codec: it reads the camera through DirectShow,
//! encodes with libx264 at 15 frames per second, and decodes the peer's
//! stream into BGRA frames for the atlas. Both directions use the same fixed
//! geometry, so the two ends never disagree about the size of a picture.
//!
//! On Windows the camera is a DirectShow device; the decoder works wherever
//! `ffmpeg` does.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

/// The width of every call frame.
pub(crate) const WIDTH: u32 = 640;

/// The height of every call frame.
pub(crate) const HEIGHT: u32 = 360;

/// The bytes of one decoded frame, BGRA.
pub(crate) const FRAME_BYTES: usize = (WIDTH as usize) * (HEIGHT as usize) * 4;

/// The frames per second of both directions: the engine stamps the RTP clock
/// with the same cadence.
pub(crate) const FPS: u32 = 15;

/// The camera: still starting, streaming, or unavailable.
const STARTING: u8 = 0;
const RUNNING: u8 = 1;
const FAILED: u8 = 2;
/// It was streaming and the stream ended.
const ENDED: u8 = 3;

/// The ffmpeg of the machine.
///
/// The settings come first (`WHATFAST_FFMPEG`), then the copy that ships next
/// to the executable, then the one on the `PATH`. `None` means video calls
/// cannot run, and the screen says so instead of failing mid-call.
pub(crate) fn ffmpeg() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("WHATFAST_FFMPEG").map(PathBuf::from)
        && path.is_file()
    {
        return Some(path);
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(folder) = exe.parent()
    {
        for name in ["ffmpeg.exe", "ffmpeg"] {
            let candidate = folder.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    let path = std::env::var_os("PATH")?;
    for folder in std::env::split_paths(&path) {
        for name in ["ffmpeg.exe", "ffmpeg"] {
            let candidate = folder.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// The video devices of the machine, in the order DirectShow lists them.
#[cfg(windows)]
pub(crate) fn devices(ffmpeg: &Path) -> Vec<String> {
    let mut command = Command::new(ffmpeg);
    command
        .args([
            "-hide_banner",
            "-list_devices",
            "true",
            "-f",
            "dshow",
            "-i",
            "dummy",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    no_window(&mut command);
    let Ok(output) = command.output() else {
        return Vec::new();
    };
    // The list goes to stderr, one line per device:
    //   [dshow @ 0000] "Integrated Camera" (video)
    let text = String::from_utf8_lossy(&output.stderr);
    text.lines()
        .filter_map(|line| {
            let (_, tail) = line.rsplit_once("] ")?;
            let tail = tail.trim();
            let name = tail.strip_suffix("(video)")?.trim();
            let name = name.strip_prefix('"')?.strip_suffix('"')?;
            (!name.is_empty()).then(|| name.to_owned())
        })
        .collect()
}

#[cfg(not(windows))]
pub(crate) fn devices(_ffmpeg: &Path) -> Vec<String> {
    Vec::new()
}

/// A child process that must never open a console window: the app is a
/// `windows_subsystem` binary and every ffmpeg would flash one.
fn no_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = command;
    }
}

/// Encola el valor y, si la cola está llena, tira el más viejo.
///
/// En un flujo de tiempo real la cola no puede crecer: un visor que se atrasa
/// medio segundo acumularía decenas de megabytes de BGRA y mostraría el
/// pasado. El productor conserva un clon del receptor solo para descartar la
/// cabeza de la cola.
fn push_latest<T>(
    tx: &async_channel::Sender<T>,
    drop: &async_channel::Receiver<T>,
    mut value: T,
) {
    loop {
        match tx.try_send(value) {
            Ok(()) => return,
            Err(async_channel::TrySendError::Full(returned)) => {
                let _ = drop.try_recv();
                value = returned;
            }
            Err(async_channel::TrySendError::Closed(_)) => return,
        }
    }
}

/// The camera pipe: the encoder's H.264 byte stream, chunk by chunk.
pub(crate) struct Camera {
    process: Arc<Mutex<Option<Child>>>,
    frames: async_channel::Receiver<Vec<u8>>,
    state: Arc<AtomicU8>,
    /// Set when the call drops the camera: the worker never reopens a device
    /// for a call that is over.
    stopped: Arc<std::sync::atomic::AtomicBool>,
}

impl Camera {
    /// Opens the first camera that produces video.
    ///
    /// The devices are listed and tried in a worker thread: asking DirectShow
    /// for them takes a moment, and the call cannot wait for it. A camera
    /// that another program holds makes ffmpeg exit, and the next one gets
    /// the turn. The desktop keeps sending the stream as it arrives, so a
    /// camera that takes a second to warm up does not delay the call.
    pub(crate) fn open() -> anyhow::Result<Self> {
        let ffmpeg = ffmpeg().ok_or_else(|| {
            anyhow::anyhow!("ffmpeg is not installed, so the camera cannot start")
        })?;
        let (frames_tx, frames_rx) = async_channel::bounded(8);
        let drop_frames = frames_rx.clone();
        let process = Arc::new(Mutex::new(None));
        let state = Arc::new(AtomicU8::new(STARTING));
        let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        {
            let process = Arc::clone(&process);
            let state = Arc::clone(&state);
            let stopped = Arc::clone(&stopped);
            std::thread::spawn(move || {
                use std::sync::atomic::Ordering as AtomicOrdering;
                let devices = devices(&ffmpeg);
                for device in devices.iter().take(4) {
                    if stopped.load(AtomicOrdering::SeqCst) {
                        return;
                    }
                    let mut child = match spawn_encoder(&ffmpeg, device) {
                        Ok(child) => child,
                        Err(error) => {
                            eprintln!("call video: {error}");
                            continue;
                        }
                    };
                    let mut stdout = match child.stdout.take() {
                        Some(stdout) => stdout,
                        None => continue,
                    };
                    *process.lock().expect("camera lock") = Some(child);
                    let pumped = pump(&mut stdout, &frames_tx, &drop_frames, &state);
                    // The process ends when the call does; taking it here
                    // frees the slot and lets the next device try.
                    if let Some(mut child) = process.lock().expect("camera lock").take() {
                        let _ = child.kill();
                        let _ = child.wait();
                    }
                    if pumped {
                        // It worked: the end means the call dropped it or the
                        // camera itself stopped.
                        state.store(ENDED, Ordering::SeqCst);
                        return;
                    }
                    if stopped.load(AtomicOrdering::SeqCst) {
                        return;
                    }
                }
                state.store(FAILED, Ordering::SeqCst);
            });
        }
        Ok(Self {
            process,
            frames: frames_rx,
            state,
            stopped,
        })
    }

    /// The next chunk of the encoder's stream, or none when nothing arrived.
    pub(crate) fn try_take(&self) -> Option<Vec<u8>> {
        self.frames.try_recv().ok()
    }

    /// True when no device could be opened.
    pub(crate) fn failed(&self) -> bool {
        self.state.load(Ordering::SeqCst) == FAILED
    }

    /// True when the camera was streaming and stopped.
    pub(crate) fn ended(&self) -> bool {
        self.state.load(Ordering::SeqCst) == ENDED
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(mut child) = self.process.lock().expect("camera lock").take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(windows)]
fn spawn_encoder(ffmpeg: &Path, device: &str) -> std::io::Result<Child> {
    // 640x360 with the letterbox baked in, so both ends agree on the
    // geometry of every picture. The AUD and the repeated headers are what
    // the splitter and the decoder of the peer need.
    let filter = format!(
        "scale={WIDTH}:{HEIGHT}:force_original_aspect_ratio=decrease,\
         pad={WIDTH}:{HEIGHT}:(ow-iw)/2:(oh-ih)/2,fps={FPS}"
    );
    let input = format!("video={device}");
    let mut command = Command::new(ffmpeg);
    command
        .args(["-hide_banner", "-loglevel", "error", "-nostdin"])
        .args(["-f", "dshow", "-rtbufsize", "64M", "-i", &input])
        .args(["-vf", filter.as_str()])
        .args([
            "-c:v", "libx264", "-preset", "ultrafast", "-tune", "zerolatency",
        ])
        .args(["-profile:v", "baseline", "-level", "3.1", "-pix_fmt", "yuv420p"])
        .args(["-g", "15", "-keyint_min", "15", "-sc_threshold", "0"])
        .args(["-b:v", "800k", "-maxrate", "800k", "-bufsize", "800k"])
        .args(["-x264-params", "aud=1:repeat-headers=1"])
        .args(["-f", "h264", "pipe:1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    no_window(&mut command);
    command.spawn()
}

#[cfg(not(windows))]
fn spawn_encoder(_ffmpeg: &Path, _device: &str) -> std::io::Result<Child> {
    Err(std::io::Error::other(
        "the camera of a call needs Windows and DirectShow",
    ))
}

/// Reads the encoder's stdout into the channel. Returns whether any byte
/// came out: a camera that never produced is the next device's turn.
fn pump<R: Read>(
    stdout: &mut R,
    frames: &async_channel::Sender<Vec<u8>>,
    drop_frames: &async_channel::Receiver<Vec<u8>>,
    state: &AtomicU8,
) -> bool {
    let mut buffer = vec![0u8; 8192];
    let mut produced = false;
    loop {
        match stdout.read(&mut buffer) {
            Ok(0) => return produced,
            Ok(read) => {
                produced = true;
                state.store(RUNNING, Ordering::SeqCst);
                push_latest(frames, drop_frames, buffer[..read].to_vec());
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => {
                eprintln!("call video: {error}");
                return produced;
            }
        }
    }
}

/// One decoded picture of the peer.
pub(crate) struct Decoded {
    pub(crate) bgra: Vec<u8>,
}

/// The decoder pipe: access units in, BGRA frames out.
pub(crate) struct Decoder {
    process: Arc<Mutex<Option<Child>>>,
    input: async_channel::Sender<Vec<u8>>,
    /// The consumer's clone, used to drop the oldest access unit when the
    /// writer falls behind.
    input_drop: async_channel::Receiver<Vec<u8>>,
    frames: async_channel::Receiver<Decoded>,
}

impl Decoder {
    /// Starts `ffmpeg` as a live H.264 decoder.
    ///
    /// The first access unit must be a keyframe with its parameter sets: the
    /// caller starts the decoder there, and restarts it the same way if the
    /// process dies.
    pub(crate) fn open() -> anyhow::Result<Self> {
        let ffmpeg = ffmpeg().ok_or_else(|| {
            anyhow::anyhow!("ffmpeg is not installed, so the video cannot be decoded")
        })?;
        let size = format!("{WIDTH}x{HEIGHT}");
        let mut command = Command::new(&ffmpeg);
        command
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-flags",
                "low_delay",
                "-probesize",
                "32",
                "-analyzeduration",
                "0",
            ])
            .args(["-f", "h264", "-i", "pipe:0"])
            .args(["-f", "rawvideo", "-pix_fmt", "bgra", "-s", &size])
            .arg("pipe:1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        no_window(&mut command);
        let mut child = command
            .spawn()
            .map_err(|error| anyhow::anyhow!("the decoder did not start: {error}"))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("the decoder has no input"))?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("the decoder has no output"))?;
        let process = Arc::new(Mutex::new(Some(child)));
        let (input_tx, input_rx) = async_channel::bounded::<Vec<u8>>(16);
        let input_drop = input_rx.clone();
        let (frames_tx, frames_rx) = async_channel::bounded::<Decoded>(2);
        let drop_frames = frames_rx.clone();
        std::thread::spawn(move || {
            // The writer blocks on the pipe, never on the interface.
            while let Ok(au) = input_rx.recv_blocking() {
                if stdin.write_all(&au).is_err() || stdin.flush().is_err() {
                    return;
                }
            }
        });
        std::thread::spawn(move || {
            loop {
                let mut frame = vec![0u8; FRAME_BYTES];
                match stdout.read_exact(&mut frame) {
                    Ok(()) => {
                        push_latest(&frames_tx, &drop_frames, Decoded { bgra: frame });
                    }
                    Err(_) => return, // the decoder ended
                }
            }
        });
        Ok(Self {
            process,
            input: input_tx,
            input_drop,
            frames: frames_rx,
        })
    }

    /// Hands one access unit to the decoder. A full queue drops the oldest
    /// unit: the decoder always works on the present stream.
    pub(crate) fn push(&self, au: Vec<u8>) {
        push_latest(&self.input, &self.input_drop, au);
    }

    /// The next decoded picture, or none when nothing is ready.
    pub(crate) fn try_take(&self) -> Option<Decoded> {
        self.frames.try_recv().ok()
    }

    /// True while the decoder process is alive.
    pub(crate) fn alive(&mut self) -> bool {
        let mut slot = self.process.lock().expect("decoder lock");
        match slot.as_mut() {
            Some(child) => matches!(child.try_wait(), Ok(None)),
            None => false,
        }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        if let Some(mut child) = self.process.lock().expect("decoder lock").take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// The visible state of the video of a call.
#[derive(Default)]
pub(crate) struct CallVideo {
    /// True when this call carries video: the overlay shows the panel.
    pub(crate) on: bool,
    /// True while the camera is sending: the bar offers to turn it off.
    pub(crate) camera_on: bool,
    /// The last picture of the peer.
    pub(crate) remote: Option<Arc<gpui::RenderImage>>,
    /// The last picture of the camera, shown small over the remote one.
    pub(crate) local: Option<Arc<gpui::RenderImage>>,
    /// Why the video is not there, when it is not.
    pub(crate) error: Option<gpui::SharedString>,
}

impl CallVideo {
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::{HEIGHT, WIDTH, ffmpeg};

    #[test]
    fn the_frame_geometry_matches_the_boxes_of_the_call() {
        // Both ends scale to this box, so the decoder can read a fixed number
        // of bytes per picture.
        assert_eq!((WIDTH as usize) * (HEIGHT as usize) * 4, super::FRAME_BYTES);
        assert_eq!(WIDTH * 9, HEIGHT * 16);
    }

    #[test]
    fn ffmpeg_is_found_by_name_or_not_at_all() {
        // The machine may not have ffmpeg: the function answers `None`
        // instead of panicking, and the call falls back to voice.
        if let Some(path) = ffmpeg() {
            assert!(path.is_file(), "{} is not a file", path.display());
        }
    }
}
