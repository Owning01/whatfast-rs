//! El video de un mensaje se reproduce dentro de la burbuja.
//!
//! El cliente nativo no tiene webview, asi que no hay un `<video>` que valga:
//! los cuadros los decodifica Media Foundation (viene con Windows, ningun
//! decodificador externo) en un hilo propio, y la interfaz los pinta como una
//! imagen mas. El sonido del archivo va por `rodio`, el mismo motor de las
//! notas de voz.
//!
//! El hilo del video no comparte nada con la interfaz: recibe comandos
//! (`Play`, `Pause`, `Resume`, `Stop`) y devuelve cuadros ya decodificados por
//! un par de canales. Un cuadro que llega tarde se tira: la prioridad es que
//! el reloj del video siga el del archivo y no que la interfaz se atrase.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use async_channel::{Receiver, Sender};
use gpui::{RenderImage, SharedString};

/// El ancho maximo que se decodifica. Un video de 4K baja a este ancho y se
/// estira al pintarse: el tamaño nativo cuesta memoria y copias por cuadro
/// sin que se note en una burbuja de chat.
const MAX_WIDTH: u32 = 720;

/// El ancho de la caja de la burbuja, antes de la escala de la interfaz.
const BUBBLE_MAX_WIDTH: f32 = 420.;

/// El alto maximo de la caja de la burbuja, antes de la escala.
const BUBBLE_MAX_HEIGHT: f32 = 320.;

/// The width of the box of a video in the viewer of the statuses. It matches
/// the pictures of the same viewer.
const STATUS_MAX_WIDTH: f32 = 800.;

/// The height of the box of a video in the viewer of the statuses.
const STATUS_MAX_HEIGHT: f32 = 640.;

/// La proporcion de la caja cuando el mensaje no trae el tamaño del video.
const DEFAULT_ASPECT: f32 = 16. / 9.;

/// Cada cuantos milisegundos la interfaz mira lo que el hilo tiene para decir.
pub(crate) const TICK_MS: u64 = 16;

/// La clave de un mensaje: el chat y el mensaje.
pub(crate) type VideoKey = (String, String);

/// Un cuadro ya decodificado, en el orden de bytes que pinta el atlas de la
/// interfaz: BGRA.
pub(crate) struct VideoFrame {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) bgra: Vec<u8>,
}

/// Lo que el hilo del video devuelve a la interfaz.
pub(crate) enum VideoEvent {
    Frame {
        key: VideoKey,
        frame: VideoFrame,
        position: f32,
    },
    Finished {
        key: VideoKey,
    },
    Failed {
        key: VideoKey,
        message: SharedString,
    },
}

enum Command {
    Play {
        key: VideoKey,
        path: PathBuf,
    },
    Pause,
    Resume,
    Stop,
    /// El volumen que tiene que sonar, de 0 a 1. Llega con el video en
    /// marcha: el reproductor lo aplica sin cortar el sonido.
    Volume(f32),
}

/// Si el adjunto es un video que se puede reproducir en la burbuja.
///
/// El que no esta en el disco tambien cuenta: la burbuja muestra el
/// reproductor y el primer boton de reproducir lo descarga.
pub(crate) fn is_video(attachment: &whatfast_protocol::AttachmentContent) -> bool {
    !attachment.voice_note && attachment.mime_type.starts_with("video/")
}

/// La caja del video en la burbuja: la proporcion del archivo, topada por el
/// ancho y el alto de la referencia.
pub(crate) fn video_box_size(width: u32, height: u32, scale: f32) -> (f32, f32) {
    fit_box(width, height, scale, BUBBLE_MAX_WIDTH, BUBBLE_MAX_HEIGHT)
}

/// The box of the video of a status: the viewer fills the window, so the box
/// grows past the width of a bubble.
pub(crate) fn status_box_size(width: u32, height: u32, scale: f32) -> (f32, f32) {
    fit_box(width, height, scale, STATUS_MAX_WIDTH, STATUS_MAX_HEIGHT)
}

/// The box of the video in the viewer that fills the window.
///
/// The limits arrive in pixels because the window already carries its own
/// scale: the box is as big as the window lets it, keeping the shape of the
/// file.
pub(crate) fn viewer_box_size(
    width: u32,
    height: u32,
    max_width: f32,
    max_height: f32,
) -> (f32, f32) {
    let max_width = max_width.max(1.);
    let max_height = max_height.max(1.);
    let (source_width, source_height) = if width == 0 || height == 0 {
        (DEFAULT_ASPECT, 1.)
    } else {
        (width as f32, height as f32)
    };
    let scale_to_fit = (max_width / source_width).min(max_height / source_height);
    (
        (source_width * scale_to_fit).max(1.),
        (source_height * scale_to_fit).max(1.),
    )
}

fn fit_box(width: u32, height: u32, scale: f32, max_width: f32, max_height: f32) -> (f32, f32) {
    let max_width = (max_width * scale).max(1.);
    let max_height = (max_height * scale).max(1.);
    let (source_width, source_height) = if width == 0 || height == 0 {
        (DEFAULT_ASPECT, 1.)
    } else {
        (width as f32, height as f32)
    };
    let scale_to_fit = (max_width / source_width).min(max_height / source_height);
    (
        (source_width * scale_to_fit).max(1.),
        (source_height * scale_to_fit).max(1.),
    )
}

/// El cuadro crudo convertido a la imagen que pinta la interfaz.
///
/// El atlas de gpui sube los bytes tal cual, y en Windows espera BGRA: el
/// decodificador ya entrega ese orden.
pub(crate) fn render_image(frame: VideoFrame) -> Option<Arc<RenderImage>> {
    if frame.width == 0 || frame.height == 0 {
        return None;
    }
    let expected = frame.width as usize * frame.height as usize * 4;
    let mut bgra = frame.bgra;
    if bgra.len() < expected {
        return None;
    }
    bgra.truncate(expected);
    let buffer = image::ImageBuffer::from_raw(frame.width, frame.height, bgra)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

/// El estado visible del reproductor de la burbuja.
pub(crate) struct VideoPlayback {
    pub(crate) key: Option<VideoKey>,
    /// El archivo que se esta reproduciendo.
    pub(crate) path: String,
    /// Hay una sesion abierta: el video esta en el hilo.
    pub(crate) active: bool,
    /// El reloj del video corre.
    pub(crate) playing: bool,
    /// El archivo se esta bajando del sidecar.
    pub(crate) waiting: bool,
    pub(crate) duration: f32,
    pub(crate) position: f32,
    pub(crate) frame: Option<Arc<RenderImage>>,
    pub(crate) error: Option<SharedString>,
    /// El volumen elegido, de 0 a 1.
    pub(crate) volume: f32,
    /// El sonido esta apagado sin perder el volumen elegido.
    pub(crate) muted: bool,
}

impl Default for VideoPlayback {
    fn default() -> Self {
        Self {
            key: None,
            path: String::new(),
            active: false,
            playing: false,
            waiting: false,
            duration: 0.0,
            position: 0.0,
            frame: None,
            error: None,
            // El video arranca con todo el sonido: el que lo quiera bajo lo
            // baja, y el volumen elegido se mantiene para el proximo.
            volume: 1.0,
            muted: false,
        }
    }
}

impl VideoPlayback {
    /// Si el reproductor esta pintando este mensaje.
    pub(crate) fn is_for(&self, key: &VideoKey) -> bool {
        self.key.as_ref() == Some(key)
    }

    /// Si el video esta corriendo de verdad, para pintar el boton de pausa.
    pub(crate) fn is_running(&self) -> bool {
        self.active && self.playing
    }

    /// El volumen que oye el reproductor: en cero cuando esta en silencio.
    pub(crate) fn effective_volume(&self) -> f32 {
        if self.muted { 0.0 } else { self.volume }
    }
}

/// El handle del hilo del video.
pub(crate) struct VideoEngine {
    commands: Sender<Command>,
    events: Receiver<VideoEvent>,
    /// El volumen de la sesion que se abra despues, en los bits de un `f32`:
    /// el hilo lo lee al empezar y la sesion nueva arranca con el sonido que
    /// el usuario dejo, sin esperar a que llegue un comando.
    volume: Arc<AtomicU32>,
}

impl VideoEngine {
    /// Arranca el hilo. Un hilo que no arranca deja los videos sin imagen y
    /// el resto de la ventana funciona igual, asi que el fallo no es fatal.
    pub(crate) fn new() -> Self {
        let (commands, command_receiver) = async_channel::unbounded();
        // Una cola corta: si la interfaz no llega a pintar, los cuadros
        // viejos se descartan en vez de acumularse.
        let (event_sender, events) = async_channel::bounded(4);
        let volume = Arc::new(AtomicU32::new(1f32.to_bits()));
        let level = volume.clone();
        let _ = std::thread::Builder::new()
            .name("whatfast-video".into())
            .spawn(move || run_video(command_receiver, event_sender, level));
        Self {
            commands,
            events,
            volume,
        }
    }

    /// Arranca el archivo desde el principio.
    pub(crate) fn play(&self, key: VideoKey, path: &Path) {
        let _ = self.commands.send_blocking(Command::Play {
            key,
            path: path.to_path_buf(),
        });
    }

    pub(crate) fn pause(&self) {
        let _ = self.commands.send_blocking(Command::Pause);
    }

    pub(crate) fn resume(&self) {
        let _ = self.commands.send_blocking(Command::Resume);
    }

    pub(crate) fn stop(&self) {
        let _ = self.commands.send_blocking(Command::Stop);
    }

    /// Fija el volumen de lo que este sonando y el de la proxima sesion.
    pub(crate) fn set_volume(&self, volume: f32) {
        let volume = volume.clamp(0.0, 1.0);
        self.volume.store(volume.to_bits(), Ordering::Relaxed);
        let _ = self.commands.send_blocking(Command::Volume(volume));
    }

    pub(crate) fn try_recv(&self) -> Option<VideoEvent> {
        self.events.try_recv().ok()
    }
}

/// El cuerpo del hilo: una sesion por vez.
fn run_video(commands: Receiver<Command>, events: Sender<VideoEvent>, volume: Arc<AtomicU32>) {
    loop {
        // Espera una sesion nueva; la pausa y el alto de una sesion que ya
        // termino no tienen a quien avisar.
        let (key, path) = loop {
            match commands.recv_blocking() {
                Ok(Command::Play { key, path }) => break (key, path),
                Ok(_) => continue,
                Err(_) => return,
            }
        };
        match play_session(&key, &path, &commands, &events, &volume) {
            Ok(SessionEnd::Stop) => {}
            Ok(SessionEnd::Restart { key, path }) => {
                if let Err(message) = play_session(&key, &path, &commands, &events, &volume) {
                    let _ = events.send_blocking(VideoEvent::Failed {
                        key,
                        message: message.into(),
                    });
                }
            }
            Err(message) => {
                let _ = events.send_blocking(VideoEvent::Failed {
                    key,
                    message: message.into(),
                });
            }
        }
    }
}

enum SessionEnd {
    Stop,
    Restart { key: VideoKey, path: PathBuf },
}

/// Reproduce un archivo hasta que termina o hasta que llega otro video.
fn play_session(
    key: &VideoKey,
    path: &Path,
    commands: &Receiver<Command>,
    events: &Sender<VideoEvent>,
    volume: &AtomicU32,
) -> Result<SessionEnd, String> {
    let mut source = decoder::open(path)?;
    // El sonido es opcional: un video sin pista de audio, o una maquina sin
    // placa de sonido, se mira igual.
    let mut level = f32::from_bits(volume.load(Ordering::Relaxed));
    let mut audio = AudioTrack::start(path, level);
    let mut paused = false;
    let mut clock: Option<(i64, Instant)> = None;
    let mut paused_total = Duration::ZERO;
    let mut paused_at: Option<Instant> = None;

    loop {
        // El sonido que ya bajo del archivo espera en la cola mientras la
        // interfaz mira el cuadro.
        audio.pump();
        // Los comandos que llegaron mientras se decodificaba el cuadro.
        while let Ok(command) = commands.try_recv() {
            match command {
                Command::Stop => return Ok(SessionEnd::Stop),
                Command::Play { key, path } => {
                    return Ok(SessionEnd::Restart { key, path });
                }
                Command::Pause => {
                    if !paused {
                        paused = true;
                        paused_at = Some(Instant::now());
                        audio.pause();
                    }
                }
                Command::Resume => {
                    if paused {
                        paused = false;
                        if let Some(at) = paused_at.take() {
                            paused_total += at.elapsed();
                        }
                        audio.resume();
                    }
                }
                Command::Volume(value) => {
                    // El volumen cambia tambien en pausa: al volver a
                    // reproducir ya suena como el usuario lo dejo.
                    level = value.clamp(0.0, 1.0);
                    audio.set_volume(level);
                }
            }
        }
        if paused {
            // En pausa el sonido tampoco corre; la cola se llena y espera.
            audio.pump();
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        let Some((frame, timestamp)) = source.next_frame()? else {
            let _ = events.send_blocking(VideoEvent::Finished { key: key.clone() });
            return Ok(SessionEnd::Stop);
        };
        let (first, started) = *clock.get_or_insert((timestamp, Instant::now()));
        let due = Duration::from_nanos((timestamp - first).max(0) as u64 * 100);
        let elapsed = started.elapsed().saturating_sub(paused_total);
        if due > elapsed {
            // El cuadro todavia no es su turno: se espera lo que falta, en
            // tramos cortos para atender una pausa sin demora.
            let mut left = due - elapsed;
            while left > Duration::ZERO {
                let step = left.min(Duration::from_millis(10));
                std::thread::sleep(step);
                audio.pump();
                left = left.saturating_sub(step);
            }
        }
        let position = (timestamp - first) as f32 / 10_000_000.0;
        let _ = events.try_send(VideoEvent::Frame {
            key: key.clone(),
            frame,
            position,
        });
    }
}

/// Cuantos bloques de sonido esperan al reproductor. Cada bloque trae unos
/// 85 ms, asi que la cola cubre varios segundos de video sin crecer.
const AUDIO_QUEUE: usize = 64;

/// El sonido del video que `rodio` va sacando de la cola.
///
/// El decodificador es el de Media Foundation, el mismo del cuadro: `rodio`
/// solo no puede con estos archivos. Su `Decoder::new` toma la primera pista
/// con codec del MP4, que es el H264 del video, y al no tener decodificador
/// de video devuelve `UnrecognizedFormat`; el AAC de la segunda pista nunca
/// suena. Symphonia tampoco abre estos MP4: rechaza el descriptor `esds` que
/// escribe WhatsApp. Por eso el sonido se decodifica en el hilo del video y
/// aca solo se entrega a la salida del sistema.
#[cfg(windows)]
struct VideoAudio {
    chunks: async_channel::Receiver<Vec<f32>>,
    channels: rodio::ChannelCount,
    sample_rate: rodio::SampleRate,
    current: Vec<f32>,
    index: usize,
}

#[cfg(windows)]
impl Iterator for VideoAudio {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        loop {
            if self.index < self.current.len() {
                let sample = self.current[self.index];
                self.index += 1;
                return Some(sample);
            }
            // La cola se llena mientras el video decodifica el cuadro: aca
            // casi siempre hay un bloque listo.
            let chunk = self.chunks.recv_blocking().ok()?;
            self.current = chunk;
            self.index = 0;
        }
    }
}

#[cfg(windows)]
impl rodio::Source for VideoAudio {
    fn current_span_len(&self) -> Option<usize> {
        // El sonido termina cuando la cola se cierra: entre bloque y bloque
        // no hay un corte que valga.
        None
    }

    fn channels(&self) -> rodio::ChannelCount {
        self.channels
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

/// La pista de sonido del archivo, si la hay.
struct AudioTrack {
    device: Option<rodio::MixerDeviceSink>,
    player: Option<rodio::Player>,
    #[cfg(windows)]
    reader: Option<windows_mf::AudioReader>,
    #[cfg(windows)]
    sender: Option<async_channel::Sender<Vec<f32>>>,
    #[cfg(windows)]
    pending: Option<Vec<f32>>,
    /// El archivo no tiene mas sonido, o no tenia pista de audio.
    #[cfg(windows)]
    finished: bool,
}

impl AudioTrack {
    fn start(path: &Path, volume: f32) -> Self {
        #[cfg(windows)]
        {
            let mut track = Self {
                device: None,
                player: None,
                reader: None,
                sender: None,
                pending: None,
                finished: true,
            };
            // Sin pista de audio, o sin placa de sonido, el video se mira
            // igual: el sonido es opcional.
            let Ok(reader) = windows_mf::AudioReader::open(path) else {
                return track;
            };
            let (sender, receiver) = async_channel::bounded(AUDIO_QUEUE);
            let (Some(channels), Some(sample_rate)) = (
                rodio::ChannelCount::new(reader.channels()),
                rodio::SampleRate::new(reader.sample_rate()),
            ) else {
                return track;
            };
            let Ok(device) = rodio::DeviceSinkBuilder::open_default_sink() else {
                return track;
            };
            let player = rodio::Player::connect_new(device.mixer());
            // El volumen de la sesion se fija antes de que suene el primer
            // bloque: el video arranca con el sonido que el usuario dejo.
            player.set_volume(volume.clamp(0.0, 1.0));
            player.append(VideoAudio {
                chunks: receiver,
                channels,
                sample_rate,
                current: Vec::new(),
                index: 0,
            });
            track.device = Some(device);
            track.player = Some(player);
            track.reader = Some(reader);
            track.sender = Some(sender);
            track.finished = false;
            // El primer cuadro tarda lo suyo: el sonido arranca con la cola
            // ya llena.
            track.pump();
            track
        }
        #[cfg(not(windows))]
        {
            let _ = path;
            Self {
                device: None,
                player: None,
            }
        }
    }

    /// Mueve el sonido del archivo a la cola del reproductor.
    ///
    /// Se llama desde el bucle del video, que ya espera entre cuadro y
    /// cuadro: el lector de Media Foundation no toca el hilo del audio.
    fn pump(&mut self) {
        #[cfg(windows)]
        {
            // Un tope por vuelta: el bucle del video no se queda leyendo el
            // archivo entero si la cola tiene lugar.
            for _ in 0..4 {
                if self.finished {
                    break;
                }
                if self.pending.is_none() {
                    let Some(reader) = self.reader.as_mut() else {
                        self.finished = true;
                        break;
                    };
                    match reader.next_chunk() {
                        Ok(Some(chunk)) => self.pending = Some(chunk),
                        Ok(None) | Err(_) => {
                            self.finished = true;
                            break;
                        }
                    }
                }
                let Some(chunk) = self.pending.take() else {
                    break;
                };
                let Some(sender) = self.sender.as_ref() else {
                    self.finished = true;
                    break;
                };
                match sender.try_send(chunk) {
                    Ok(()) => {}
                    Err(async_channel::TrySendError::Full(chunk)) => {
                        self.pending = Some(chunk);
                        break;
                    }
                    Err(async_channel::TrySendError::Closed(_)) => {
                        self.finished = true;
                        break;
                    }
                }
            }
            // Sin sonido que mandar, la cola se cierra y el reproductor
            // termina: el video sigue su reloj.
            if self.finished {
                self.sender = None;
            }
        }
    }

    fn pause(&self) {
        if let Some(player) = self.player.as_ref() {
            player.pause();
        }
    }

    fn resume(&self) {
        if let Some(player) = self.player.as_ref() {
            player.play();
        }
    }

    /// Cambia el volumen de lo que ya esta sonando.
    fn set_volume(&self, volume: f32) {
        if let Some(player) = self.player.as_ref() {
            player.set_volume(volume.clamp(0.0, 1.0));
        }
    }
}

impl Drop for AudioTrack {
    fn drop(&mut self) {
        // El reproductor se suelta antes que el dispositivo: al reves, la
        // salida se cierra con el sonido todavia conectado.
        self.player = None;
        self.device = None;
    }
}

/// Un decodificador de cuadros.
struct DecoderSource {
    #[cfg(windows)]
    inner: windows_mf::Source,
}

impl DecoderSource {
    fn next_frame(&mut self) -> Result<Option<(VideoFrame, i64)>, String> {
        #[cfg(windows)]
        {
            self.inner.next_frame()
        }
        #[cfg(not(windows))]
        {
            Err("The video player is only available on Windows".into())
        }
    }
}

mod decoder {
    use std::path::Path;

    use super::DecoderSource;

    pub(super) fn open(path: &Path) -> Result<DecoderSource, String> {
        #[cfg(windows)]
        {
            let inner = super::windows_mf::Source::open(path)?;
            Ok(DecoderSource { inner })
        }
        #[cfg(not(windows))]
        {
            let _ = path;
            Err("The video player is only available on Windows".into())
        }
    }
}

/// Media Foundation: el decodificador de video que trae Windows.
#[cfg(windows)]
mod windows_mf {
    use std::path::Path;
    use std::ptr;

    use windows::Win32::Media::MediaFoundation::{
        IMFAttributes, IMFMediaType, IMFSourceReader, MF_MT_AUDIO_BITS_PER_SAMPLE,
        MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_DEFAULT_STRIDE,
        MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_SOURCE_READER_ALL_STREAMS,
        MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, MF_SOURCE_READER_FIRST_AUDIO_STREAM,
        MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED,
        MF_SOURCE_READERF_ENDOFSTREAM, MF_VERSION, MFAudioFormat_Float, MFAudioFormat_PCM,
        MFCreateAttributes, MFCreateMediaType, MFCreateSourceReaderFromURL, MFMediaType_Audio,
        MFMediaType_Video, MFSTARTUP_FULL, MFStartup, MFVideoFormat_RGB32,
    };
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
    use windows::core::{GUID, HSTRING};

    use super::{MAX_WIDTH, VideoFrame};

    /// El stream de video del archivo: el indice negativo que pide la API.
    const VIDEO_STREAM: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

    /// El stream de audio, con el mismo truco del indice negativo.
    const AUDIO_STREAM: u32 = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;

    /// Los frames de sonido que viajan en cada bloque: unos 85 ms a 48 kHz.
    const AUDIO_CHUNK_FRAMES: usize = 4096;

    /// El lector de la pista de sonido.
    pub(super) struct AudioReader {
        reader: IMFSourceReader,
        sample_rate: u32,
        channels: u16,
        bits: u32,
        float: bool,
    }

    impl AudioReader {
        pub(super) fn open(path: &Path) -> Result<Self, String> {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                MFStartup(MF_VERSION, MFSTARTUP_FULL)
                    .map_err(|error| format!("Media Foundation did not start: {error}"))?;
            }
            let reader = unsafe { open_audio_source(path)? };
            unsafe {
                let _ = reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false);
                let _ = reader.SetStreamSelection(AUDIO_STREAM, true);
            }
            // El decodificador entrega PCM: se pide ese tipo y se lee el
            // formato que quedo puesto.
            let integer = unsafe { audio_output_type(MFAudioFormat_PCM) }?;
            if unsafe { reader.SetCurrentMediaType(AUDIO_STREAM, None, &integer) }.is_err() {
                let float = unsafe { audio_output_type(MFAudioFormat_Float) }?;
                unsafe {
                    reader
                        .SetCurrentMediaType(AUDIO_STREAM, None, &float)
                        .map_err(|error| {
                            format!("The sound of the video could not be decoded: {error}")
                        })?;
                }
            }
            let current = unsafe { reader.GetCurrentMediaType(AUDIO_STREAM) }
                .map_err(|error| format!("The sound of the video has no format: {error}"))?;
            let channels =
                unsafe { current.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS) }.unwrap_or(2) as u16;
            let sample_rate =
                unsafe { current.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND) }.unwrap_or(48_000);
            let bits = unsafe { current.GetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE) }.unwrap_or(16);
            let float = unsafe { current.GetGUID(&MF_MT_SUBTYPE) }
                .is_ok_and(|subtype| subtype == MFAudioFormat_Float);
            Ok(Self {
                reader,
                sample_rate,
                channels: channels.max(1),
                bits,
                float,
            })
        }

        pub(super) fn sample_rate(&self) -> u32 {
            self.sample_rate
        }

        pub(super) fn channels(&self) -> u16 {
            self.channels
        }

        /// Entrega el bloque de sonido que sigue, ya en f32 intercalado.
        pub(super) fn next_chunk(&mut self) -> Result<Option<Vec<f32>>, String> {
            let target = AUDIO_CHUNK_FRAMES * self.channels as usize;
            let mut samples = Vec::with_capacity(target);
            // Un decodificador que devuelve muestras vacias no puede dejar
            // este bucle girando para siempre.
            for _ in 0..64 {
                if samples.len() >= target {
                    break;
                }
                let mut flags = 0u32;
                let mut timestamp = 0i64;
                let mut sample: Option<windows::Win32::Media::MediaFoundation::IMFSample> = None;
                unsafe {
                    self.reader
                        .ReadSample(
                            AUDIO_STREAM,
                            0,
                            None,
                            Some(&mut flags),
                            Some(&mut timestamp),
                            Some(&mut sample),
                        )
                        .map_err(|error| {
                            format!("The sound of the video could not be read: {error}")
                        })?;
                }
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                    break;
                }
                let Some(sample) = sample else {
                    continue;
                };
                let bytes = unsafe { sample_bytes(&sample)? };
                self.push_samples(&bytes, &mut samples);
            }
            Ok((!samples.is_empty()).then_some(samples))
        }

        fn push_samples(&self, bytes: &[u8], out: &mut Vec<f32>) {
            match (self.float, self.bits) {
                (true, 32) => out.extend(
                    bytes
                        .chunks_exact(4)
                        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])),
                ),
                (false, 16) => out.extend(
                    bytes
                        .chunks_exact(2)
                        .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]) as f32 / 32_768.0),
                ),
                (false, 32) => out.extend(bytes.chunks_exact(4).map(|chunk| {
                    i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as f32
                        / 2_147_483_648.0
                })),
                (false, 8) => out.extend(bytes.iter().map(|byte| (*byte as f32 - 128.0) / 128.0)),
                _ => {}
            }
        }
    }

    /// El tipo de salida PCM que se le pide al decodificador.
    unsafe fn audio_output_type(subtype: GUID) -> Result<IMFMediaType, String> {
        let media_type = unsafe {
            MFCreateMediaType().map_err(|error| format!("The sound type failed: {error}"))?
        };
        unsafe {
            media_type
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)
                .and_then(|()| media_type.SetGUID(&MF_MT_SUBTYPE, &subtype))
                .map_err(|error| format!("The sound type failed: {error}"))?;
        }
        Ok(media_type)
    }

    /// Abre el lector del archivo sin los atributos de video.
    unsafe fn open_audio_source(path: &Path) -> Result<IMFSourceReader, String> {
        let url = file_url(path);
        unsafe { MFCreateSourceReaderFromURL(&url, None) }
            .map_err(|error| format!("The video could not be opened: {error}"))
    }

    pub(super) struct Source {
        reader: IMFSourceReader,
        width: u32,
        height: u32,
        stride: i32,
    }

    impl Source {
        pub(super) fn open(path: &Path) -> Result<Self, String> {
            // El hilo de Media Foundation es de apartamento multiple: el
            // reader y sus muestras no salen de aca.
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                MFStartup(MF_VERSION, MFSTARTUP_FULL)
                    .map_err(|error| format!("Media Foundation did not start: {error}"))?;
            }
            let reader = unsafe { open_reader(path)? };
            // Solo el video: el sonido lo lleva `rodio`, y decodificarlo dos
            // veces cuesta.
            unsafe {
                let _ = reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false);
                let _ = reader.SetStreamSelection(VIDEO_STREAM, true);
            }
            let native = unsafe { native_size(&reader)? };
            let (width, height) = fit(native, MAX_WIDTH);
            let (width, height) = unsafe { set_output_size(&reader, width, height)? };
            let stride = unsafe { default_stride(&reader, width) };
            Ok(Self {
                reader,
                width,
                height,
                stride,
            })
        }

        pub(super) fn next_frame(&mut self) -> Result<Option<(VideoFrame, i64)>, String> {
            let mut flags = 0u32;
            let mut timestamp = 0i64;
            let mut sample: Option<windows::Win32::Media::MediaFoundation::IMFSample> = None;
            unsafe {
                self.reader
                    .ReadSample(
                        VIDEO_STREAM,
                        0,
                        None,
                        Some(&mut flags),
                        Some(&mut timestamp),
                        Some(&mut sample),
                    )
                    .map_err(|error| format!("The video could not be read: {error}"))?;
            }
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                return Ok(None);
            }
            if flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32 != 0
                && let Some((width, height)) = unsafe { current_size(&self.reader) }
            {
                // El archivo cambio de resolucion a mitad de camino.
                self.width = width;
                self.height = height;
                self.stride = unsafe { default_stride(&self.reader, width) };
            }
            let Some(sample) = sample else {
                // Un hueco del archivo: el reloj sigue con el cuadro que viene.
                return Ok(Some((
                    VideoFrame {
                        width: self.width,
                        height: self.height,
                        bgra: Vec::new(),
                    },
                    timestamp,
                )));
            };
            let bytes = unsafe { sample_bytes(&sample)? };
            let bgra = self.pack(&bytes);
            let (bgra, width, height) = shrink(bgra, self.width, self.height);
            Ok(Some((
                VideoFrame {
                    width,
                    height,
                    bgra,
                },
                timestamp,
            )))
        }

        /// Deja el cuadro como lo pinta el atlas: BGRA, filas de arriba a
        /// abajo y el alfa en opaco.
        fn pack(&self, bytes: &[u8]) -> Vec<u8> {
            let row_bytes = self.width as usize * 4;
            let height = self.height as usize;
            let stride = self.stride;
            let mut out = vec![0u8; row_bytes * height];
            if stride.unsigned_abs() as usize >= row_bytes {
                // La fila puede traer relleno, o venir de abajo hacia arriba
                // (stride negativo).
                let step = stride.unsigned_abs() as usize;
                for row in 0..height {
                    let source_row = if stride >= 0 { row } else { height - 1 - row };
                    let start = source_row * step;
                    let Some(line) = bytes.get(start..start + row_bytes) else {
                        break;
                    };
                    out[row * row_bytes..(row + 1) * row_bytes].copy_from_slice(line);
                }
            } else if bytes.len() >= row_bytes * height {
                out.copy_from_slice(&bytes[..row_bytes * height]);
            }
            // El X de BGRX no siempre viene en 255: sin alfa la imagen se
            // pintaria transparente.
            for pixel in out.as_chunks_mut::<4>().0 {
                pixel[3] = 255;
            }
            out
        }
    }

    /// La ruta de Windows como url de archivo: es la forma que el resolvedor
    /// de medios entiende, con espacios y acentos.
    fn file_url(path: &Path) -> HSTRING {
        HSTRING::from(format!(
            "file:///{}",
            path.to_string_lossy().replace('\\', "/")
        ))
    }

    unsafe fn open_reader(path: &Path) -> Result<IMFSourceReader, String> {
        let url = file_url(path);
        let mut last_error = String::new();
        // Con los procesadores de video el cuadro sale ya escalado; si el
        // archivo o el decodificador no los aceptan, se abre igual sin
        // ellos, que es lo que importa.
        for with_processing in [true, false] {
            let attributes = if with_processing {
                unsafe { video_attributes() }?
            } else {
                None
            };
            match unsafe { MFCreateSourceReaderFromURL(&url, attributes.as_ref()) } {
                Ok(reader) => return Ok(reader),
                Err(error) => last_error = format!("{error}"),
            }
        }
        Err(format!("The video could not be opened: {last_error}"))
    }

    /// Los atributos del lector que piden el escalado del cuadro.
    unsafe fn video_attributes() -> Result<Option<IMFAttributes>, String> {
        let mut attributes: Option<IMFAttributes> = None;
        unsafe {
            MFCreateAttributes(&mut attributes, 2)
                .map_err(|error| format!("The video reader did not start: {error}"))?;
        }
        if let Some(attributes) = attributes.as_ref() {
            unsafe {
                let _ = attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1);
            }
        }
        Ok(attributes)
    }

    /// El tamaño nativo del primer stream de video.
    /// El tamano de un cuadro, como lo guarda `MF_MT_FRAME_SIZE`.
    ///
    /// Media Foundation lo empaqueta con el ancho en los 32 bits altos y el
    /// alto en los bajos (`MFSetAttributeSize`). Leerlo al reves decodifica
    /// un video vertical como si fuera horizontal y las filas se corren.
    pub(super) fn unpack_size(packed: u64) -> (u32, u32) {
        ((packed >> 32) as u32, packed as u32)
    }

    /// El camino de ida: el ancho en los 32 bits altos.
    pub(super) fn pack_size(width: u32, height: u32) -> u64 {
        u64::from(width) << 32 | u64::from(height)
    }

    unsafe fn native_size(reader: &IMFSourceReader) -> Result<(u32, u32), String> {
        let media_type = unsafe {
            reader
                .GetNativeMediaType(VIDEO_STREAM, 0)
                .map_err(|error| format!("The video has no picture: {error}"))?
        };
        let packed = unsafe { media_type.GetUINT64(&MF_MT_FRAME_SIZE) }
            .map_err(|error| format!("The size of the video is unknown: {error}"))?;
        Ok(unpack_size(packed))
    }

    /// El ancho y el alto que se decodifican: el nativo o el que entra en el
    /// tope, conservando la proporcion.
    fn fit((width, height): (u32, u32), max_width: u32) -> (u32, u32) {
        if width == 0 || height == 0 {
            return (max_width, max_width * 9 / 16);
        }
        if width <= max_width {
            return (width, height);
        }
        let height = ((height as u64 * max_width as u64) / width as u64).max(2) as u32;
        (max_width, height & !1)
    }

    unsafe fn set_output_size(
        reader: &IMFSourceReader,
        width: u32,
        height: u32,
    ) -> Result<(u32, u32), String> {
        // Primero se pide el tamaño reducido: el procesador de video de
        // Media Foundation escala el cuadro al decodificarlo.
        if let Ok(media_type) = unsafe { output_type(width, height) }
            && unsafe {
                reader
                    .SetCurrentMediaType(VIDEO_STREAM, None, &media_type)
                    .is_ok()
            }
        {
            // El decodificador puede haber elegido otro tamaño: manda el que
            // quedo puesto, porque con el se arman las filas.
            return Ok(unsafe { current_size(reader) }.unwrap_or((width, height)));
        }
        // Un decodificador que no escala: se pide el cuadro como viene.
        let media_type = unsafe { output_type(0, 0) }?;
        unsafe {
            reader
                .SetCurrentMediaType(VIDEO_STREAM, None, &media_type)
                .map_err(|error| format!("The video could not be decoded: {error}"))?;
        }
        Ok(unsafe { current_size(reader) }.unwrap_or((width, height)))
    }

    /// El tamaño del cuadro que el lector entrega de verdad.
    unsafe fn current_size(reader: &IMFSourceReader) -> Option<(u32, u32)> {
        let media_type = unsafe { reader.GetCurrentMediaType(VIDEO_STREAM) }.ok()?;
        let packed = unsafe { media_type.GetUINT64(&MF_MT_FRAME_SIZE) }.ok()?;
        let (width, height) = unpack_size(packed);
        (width > 0 && height > 0).then_some((width, height))
    }

    unsafe fn output_type(width: u32, height: u32) -> Result<IMFMediaType, String> {
        let media_type = unsafe {
            MFCreateMediaType().map_err(|error| format!("The video type failed: {error}"))?
        };
        unsafe {
            media_type
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                .and_then(|()| media_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32))
                .and_then(|()| {
                    if width > 0 && height > 0 {
                        media_type.SetUINT64(&MF_MT_FRAME_SIZE, pack_size(width, height))
                    } else {
                        Ok(())
                    }
                })
                .map_err(|error| format!("The video type failed: {error}"))?;
        }
        Ok(media_type)
    }

    unsafe fn default_stride(reader: &IMFSourceReader, width: u32) -> i32 {
        let fallback = width as i32 * 4;
        let Ok(media_type) = (unsafe { reader.GetCurrentMediaType(VIDEO_STREAM) }) else {
            return fallback;
        };
        match unsafe { media_type.GetUINT32(&MF_MT_DEFAULT_STRIDE) } {
            Ok(stride) => stride as i32,
            Err(_) => fallback,
        }
    }

    /// Baja el cuadro al ancho maximo cuando el archivo es mas grande.
    ///
    /// Media Foundation escala solo con el procesador de video avanzado, que
    /// en esta version de Windows no deja crear el lector: la reduccion se
    /// hace aca, sobre el cuadro ya decodificado. El buffer se declara como
    /// RGBA porque el tipo solo manda el tamaño del pixel; los canales
    /// siguen en el orden BGRA que pinta el atlas.
    fn shrink(bgra: Vec<u8>, width: u32, height: u32) -> (Vec<u8>, u32, u32) {
        if width <= MAX_WIDTH || width == 0 || height == 0 {
            return (bgra, width, height);
        }
        let target_height =
            ((u64::from(height) * u64::from(MAX_WIDTH)) / u64::from(width)).max(2) as u32;
        let Some(buffer) =
            image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(width, height, bgra)
        else {
            return (Vec::new(), width, height);
        };
        // `thumbnail` muestrea en vez de interpolar: baja un cuadro 4K en
        // una sexta parte del tiempo de `resize`, y en una burbuja no se
        // nota la diferencia.
        let small = image::imageops::thumbnail(&buffer, MAX_WIDTH, target_height);
        // `thumbnail` conserva la proporcion y puede devolver un pixel menos:
        // el tamano que se informa es el del buffer, o las filas se corren.
        let (width, height) = small.dimensions();
        (small.into_raw(), width, height)
    }

    unsafe fn sample_bytes(
        sample: &windows::Win32::Media::MediaFoundation::IMFSample,
    ) -> Result<Vec<u8>, String> {
        let buffer = unsafe {
            sample
                .ConvertToContiguousBuffer()
                .map_err(|error| format!("The video frame failed: {error}"))?
        };
        let mut data = ptr::null_mut();
        let mut length = 0u32;
        unsafe {
            buffer
                .Lock(&mut data, None, Some(&mut length))
                .map_err(|error| format!("The video frame failed: {error}"))?;
        }
        let bytes = unsafe { std::slice::from_raw_parts(data, length as usize) }.to_vec();
        let _ = unsafe { buffer.Unlock() };
        Ok(bytes)
    }
}

/// Trims a video file between `start_seconds` and optional `duration_seconds` using ffmpeg.
pub(crate) fn trim_video(
    source: &Path,
    destination: &Path,
    start_seconds: f64,
    duration_seconds: Option<f64>,
) -> Result<(), String> {
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.arg("-y")
        .arg("-ss")
        .arg(format!("{start_seconds:.3}"))
        .arg("-i")
        .arg(source);
    if let Some(dur) = duration_seconds {
        cmd.arg("-t").arg(format!("{dur:.3}"));
    }
    cmd.arg("-c").arg("copy").arg(destination);
    let output = cmd
        .output()
        .map_err(|e| format!("Failed to execute ffmpeg: {e}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffmpeg trimming failed: {err}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{VideoPlayback, is_video, status_box_size, video_box_size};
    use whatfast_protocol::AttachmentContent;

    fn attachment(mime: &str) -> AttachmentContent {
        AttachmentContent {
            mime_type: mime.into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_video_is_the_one_that_plays_inside_the_bubble() {
        assert!(is_video(&attachment("video/mp4")));
        assert!(is_video(&attachment("video/quicktime")));
        assert!(!is_video(&attachment("audio/ogg")));
        assert!(!is_video(&attachment("image/png")));
        assert!(!is_video(&attachment("application/pdf")));
        let mut note = attachment("video/mp4");
        note.voice_note = true;
        assert!(!is_video(&note));
    }

    #[test]
    fn the_viewer_gives_a_video_more_room_than_a_bubble() {
        // The same landscape file: the bubble stops at 420 and the viewer at
        // 800, so the status is not a postage stamp on a full window.
        let (bubble_width, _) = video_box_size(1920, 1080, 1.0);
        let (viewer_width, viewer_height) = status_box_size(1920, 1080, 1.0);
        assert!((bubble_width - 420.).abs() < 0.5, "ancho {bubble_width}");
        assert!((viewer_width - 800.).abs() < 0.5, "ancho {viewer_width}");
        assert!((viewer_height - 450.).abs() < 0.5, "alto {viewer_height}");
        // A portrait video keeps its shape and stays inside the box.
        let (width, height) = status_box_size(576, 1024, 1.0);
        assert!(height <= 640. && width <= 800., "{width}x{height}");
        assert!((height / width - 1024. / 576.).abs() < 0.01);
    }

    #[test]
    fn the_box_of_the_video_keeps_the_shape_of_the_file() {
        let (width, height) = video_box_size(1080, 1920, 1.0);
        assert!((width - 180.).abs() < 0.5, "ancho {width}");
        assert!(height <= 320.);
        let (width, height) = video_box_size(1920, 1080, 1.0);
        assert!((width - 420.).abs() < 0.5, "ancho {width}");
        assert!((height - 236.25).abs() < 0.5, "alto {height}");
    }

    #[test]
    fn the_sound_of_the_video_follows_the_mute_button() {
        // El volumen elegido se mantiene: el silencio no lo borra.
        let mut player = VideoPlayback::default();
        assert!((player.effective_volume() - 1.0).abs() < f32::EPSILON);
        player.volume = 0.4;
        assert!((player.effective_volume() - 0.4).abs() < f32::EPSILON);
        player.muted = true;
        assert!(player.effective_volume().abs() < f32::EPSILON);
        player.muted = false;
        assert!((player.effective_volume() - 0.4).abs() < f32::EPSILON);
    }

    #[test]
    fn the_viewer_uses_the_room_of_the_window() {
        // Un video horizontal ocupa el ancho que la ventana le da, sin
        // pasarse del alto.
        let (width, height) = super::viewer_box_size(1920, 1080, 1200., 700.);
        assert!((width - 1200.).abs() < 0.5, "ancho {width}");
        assert!((height - 675.).abs() < 0.5, "alto {height}");
        // Uno vertical se topa con el alto y conserva la forma.
        let (width, height) = super::viewer_box_size(576, 1024, 1200., 700.);
        assert!(height <= 700. && width <= 1200., "{width}x{height}");
        assert!((height / width - 1024. / 576.).abs() < 0.01);
    }

    #[test]
    fn a_video_without_a_size_falls_back_to_the_sixteen_by_nine() {
        let (width, height) = video_box_size(0, 0, 1.0);
        assert!((width - 420.).abs() < 0.5, "ancho {width}");
        assert!((height - 236.25).abs() < 0.5, "alto {height}");
    }

    #[test]
    fn the_box_scales_with_the_interface() {
        let (small, _) = video_box_size(1920, 1080, 1.0);
        let (large, _) = video_box_size(1920, 1080, 2.0);
        assert!((large - small * 2.).abs() < 0.5);
    }

    /// Media Foundation guarda el ancho en los 32 bits altos: leerlo al
    /// reves decodifica un video vertical como horizontal y corre las filas.
    #[cfg(windows)]
    #[test]
    fn the_size_of_a_frame_keeps_the_width_in_the_high_bits() {
        let packed = u64::from(576u32) << 32 | u64::from(1024u32);
        assert_eq!(super::windows_mf::unpack_size(packed), (576, 1024));
        assert_eq!(super::windows_mf::pack_size(576, 1024), packed);
        assert_eq!(super::windows_mf::unpack_size(packed).1, 1024);
    }

    #[test]
    fn a_player_without_a_video_is_not_running() {
        let playback = VideoPlayback::default();
        assert!(!playback.is_running());
        assert!(!playback.is_for(&("chat".into(), "message".into())));
    }

    /// El audio de un video de verdad sale por el mismo decodificador que usa
    /// el reproductor.
    ///
    /// El AAC dentro del MP4 va por Media Foundation, igual que la pista que
    /// arranca `AudioTrack`. Un archivo sin sonido, o un decodificador que no
    /// entiende el contenedor, deja el video mudo sin ningun error visible.
    #[cfg(windows)]
    #[test]
    #[ignore = "needs WHATFAST_VIDEO_SAMPLE with a real video file"]
    fn a_real_video_gives_samples_of_its_sound() {
        let Ok(path) = std::env::var("WHATFAST_VIDEO_SAMPLE") else {
            return;
        };
        let mut reader = super::windows_mf::AudioReader::open(std::path::Path::new(&path))
            .expect("the sound of the video opens");
        let sample_rate = reader.sample_rate();
        let channels = reader.channels();
        let mut samples = 0usize;
        let mut audible = 0usize;
        while let Some(chunk) = reader.next_chunk().expect("the sound of the video decodes") {
            samples += chunk.len();
            audible += chunk.iter().filter(|sample| sample.abs() > 0.001).count();
        }
        assert!(samples > 0, "the video gave no samples of sound");
        assert!(audible > 0, "the sound of the video is silent");
        eprintln!("audio: {sample_rate} Hz, {channels} canales, {audible} muestras con sonido");
    }

    /// Decodifica un video de verdad, cuando la maquina tiene uno a mano.
    ///
    /// Los videos de prueba no se guardan en el repo: el test se salta salvo
    /// que `WHATFAST_VIDEO_SAMPLE` apunte a un archivo.
    #[test]
    #[ignore = "needs WHATFAST_VIDEO_SAMPLE with a real video file"]
    fn a_real_video_gives_frames_of_the_size_of_its_box() {
        let Ok(path) = std::env::var("WHATFAST_VIDEO_SAMPLE") else {
            return;
        };
        let mut source =
            super::decoder::open(std::path::Path::new(&path)).expect("the video opens");
        let mut frames = 0;
        while let Some((frame, _)) = source.next_frame().expect("the frame decodes") {
            if frame.bgra.is_empty() {
                continue;
            }
            assert_eq!(
                frame.bgra.len(),
                frame.width as usize * frame.height as usize * 4,
                "el cuadro no mide lo que dice"
            );
            assert!(frame.width > 0 && frame.height > 0);
            assert!(frame.width <= super::MAX_WIDTH, "el ancho no se topo");
            // El alfa queda opaco: el atlas pintaria nada con alfa cero.
            assert!(
                frame
                    .bgra
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .all(|pixel| pixel[3] == 255)
            );
            frames += 1;
            if frames == 10 {
                break;
            }
        }
        assert!(frames > 0, "el video no dio ningun cuadro");
    }
}
