//! The cache of decoded images.
//!
//! Two things were wrong with the cache GPUI uses by default. It keeps every
//! image it ever decoded, with every frame of every animated sticker, and it
//! decodes each frame at the size of the file. An animated sticker is a WebP
//! with dozens or hundreds of 512×512 frames: decoded, each frame takes a
//! megabyte, and a conversation with a few of them leaves gigabytes in the
//! process.
//!
//! This cache decodes the frames at the size the interface paints them, keeps
//! the images used most recently under a byte budget, and never has more than
//! a couple of decodes in flight, so the peak of a list of stickers stays
//! bounded.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io::Cursor,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

use futures::FutureExt as _;
use gpui::{
    App, AppContext as _, Asset as _, AssetLogger, Entity, ImageAssetLoader, ImageCache,
    ImageCacheError, ImageCacheItem, RenderImage, Resource, Window, hash,
};
use image::{
    AnimationDecoder, Delay, Frame, ImageFormat, Rgba, RgbaImage,
    codecs::{gif::GifDecoder, webp::WebPDecoder},
    imageops::FilterType,
};

/// The decoded bytes the cache may keep before it evicts the oldest image.
const MAX_BYTES: usize = 384 * 1024 * 1024;

/// The images the cache may keep. A sticker of 244 frames already takes tens
/// of megabytes; this bound keeps a long conversation from holding thousands.
/// La biblioteca de stickers junta hasta tres packs de 300 celdas: con 64
/// entradas el cache echaba las primeras para cargar las ultimas y el
/// siguiente cuadro volvia a pedirlas, en un parpadeo sin fin.
const MAX_ENTRIES: usize = 1024;

/// How many images may be decoding at the same time. Each one holds every
/// frame in memory before the cache can count it.
const MAX_IN_FLIGHT: usize = 4;

/// El lado mayor de un sticker animado en pantalla. El archivo trae 512×512 y
/// la burbuja lo pinta a la mitad: decodificar el doble es memoria tirada.
const ANIMATED_STICKER_EDGE: u32 = 256;

/// El lado mayor de las fotos y de los stickers quietos.
const PHOTO_EDGE: u32 = 2048;

/// El lado mayor del cuadro quieto de la rejilla de stickers. La celda mide
/// 76 puntos: 256 alcanza para una pantalla de doble densidad.
const STILL_EDGE: u32 = 256;

/// El prefijo de los recursos que la rejilla pide como cuadro quieto.
///
/// GPUI anima toda imagen con mas de un cuadro y no tiene una opcion para
/// apagarlo. El pedido viaja en el nombre del recurso, y este cache lo
/// reconoce: el mismo archivo entra con otra clave y se decodifica una sola
/// vez, sin cientos de stickers animandose a la vez en una rejilla.
pub(crate) const STILL_PREFIX: &str = "__whatfast_still__";

/// El nombre con el que la rejilla de stickers pide una imagen quieta.
pub(crate) fn still_image_source(path: &Path) -> String {
    format!("{STILL_PREFIX}{}", path.display())
}

/// El archivo que lleva un pedido de imagen quieta, si lo es.
fn still_path(name: &str) -> Option<PathBuf> {
    name.strip_prefix(STILL_PREFIX).map(PathBuf::from)
}

const BATTERY_SAVER_MAX_BYTES: usize = 96 * 1024 * 1024;
const BATTERY_SAVER_MAX_ENTRIES: usize = 256;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ImageCacheStats {
    pub entries: usize,
    pub max_entries: usize,
    pub bytes: usize,
    pub max_bytes: usize,
    pub in_flight: usize,
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub battery_saver: bool,
}

static STAT_ENTRIES: AtomicUsize = AtomicUsize::new(0);
static STAT_MAX_ENTRIES: AtomicUsize = AtomicUsize::new(1024);
static STAT_BYTES: AtomicUsize = AtomicUsize::new(0);
static STAT_MAX_BYTES: AtomicUsize = AtomicUsize::new(384 * 1024 * 1024);
static STAT_IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static STAT_HITS: AtomicU64 = AtomicU64::new(0);
static STAT_MISSES: AtomicU64 = AtomicU64::new(0);
static STAT_EVICTIONS: AtomicU64 = AtomicU64::new(0);
static STAT_BATTERY_SAVER: AtomicBool = AtomicBool::new(false);

pub(crate) fn stats_snapshot() -> ImageCacheStats {
    ImageCacheStats {
        entries: STAT_ENTRIES.load(Ordering::Relaxed),
        max_entries: STAT_MAX_ENTRIES.load(Ordering::Relaxed),
        bytes: STAT_BYTES.load(Ordering::Relaxed),
        max_bytes: STAT_MAX_BYTES.load(Ordering::Relaxed),
        in_flight: STAT_IN_FLIGHT.load(Ordering::Relaxed),
        hits: STAT_HITS.load(Ordering::Relaxed),
        misses: STAT_MISSES.load(Ordering::Relaxed),
        evictions: STAT_EVICTIONS.load(Ordering::Relaxed),
        battery_saver: STAT_BATTERY_SAVER.load(Ordering::Relaxed),
    }
}

struct Entry {
    item: ImageCacheItem,
    /// The decoded bytes this image takes, once it finished loading.
    bytes: usize,
    /// Monotonic access tick for O(log n) LRU eviction.
    last_used: u64,
}

pub(crate) struct BoundedImageCache {
    entries: HashMap<u64, Entry>,
    /// Monotonic LRU index: `last_used -> hash`.
    lru: BTreeMap<u64, u64>,
    /// Keys whose future has not resolved yet, keeping `in_flight()` O(1) and `settle()` O(in_flight).
    loading: HashSet<u64>,
    clock: u64,
    bytes: usize,
    max_bytes: usize,
    max_entries: usize,
    battery_saver: bool,
}

impl BoundedImageCache {
    pub(crate) fn new(cx: &mut App) -> Entity<Self> {
        let cache = cx.new(|_| Self {
            entries: HashMap::new(),
            lru: BTreeMap::new(),
            loading: HashSet::new(),
            clock: 0,
            bytes: 0,
            max_bytes: MAX_BYTES,
            max_entries: MAX_ENTRIES,
            battery_saver: false,
        });
        // The textures live in the windows: they have to be dropped with the
        // cache or the GPU keeps them until the process ends.
        cx.observe_release(&cache, |cache, cx| {
            for (_, mut entry) in std::mem::take(&mut cache.entries) {
                if let Some(Ok(image)) = entry.item.get() {
                    cx.drop_image(image, None);
                }
            }
            cache.lru.clear();
            cache.loading.clear();
            cache.bytes = 0;
            cache.sync_stats();
        })
        .detach();
        cache
    }

    pub(crate) fn set_battery_saver(&mut self, enabled: bool) {
        self.battery_saver = enabled;
        if enabled {
            self.max_bytes = BATTERY_SAVER_MAX_BYTES;
            self.max_entries = BATTERY_SAVER_MAX_ENTRIES;
        } else {
            self.max_bytes = MAX_BYTES;
            self.max_entries = MAX_ENTRIES;
        }
        self.sync_stats();
    }

    fn sync_stats(&self) {
        STAT_ENTRIES.store(self.entries.len(), Ordering::Relaxed);
        STAT_MAX_ENTRIES.store(self.max_entries, Ordering::Relaxed);
        STAT_BYTES.store(self.bytes, Ordering::Relaxed);
        STAT_MAX_BYTES.store(self.max_bytes, Ordering::Relaxed);
        STAT_IN_FLIGHT.store(self.loading.len(), Ordering::Relaxed);
        STAT_BATTERY_SAVER.store(self.battery_saver, Ordering::Relaxed);
    }

    fn touch(&mut self, hash: u64) {
        self.clock = self.clock.wrapping_add(1);
        let tick = self.clock;
        if let Some(entry) = self.entries.get_mut(&hash) {
            if entry.last_used != 0 {
                self.lru.remove(&entry.last_used);
            }
            entry.last_used = tick;
            self.lru.insert(tick, hash);
        }
    }

    fn in_flight(&self) -> usize {
        self.loading.len()
    }

    /// Moves the decodes that finished to their final size, even when the
    /// image was not drawn again. Only inspects keys still in `loading`.
    fn settle(&mut self) {
        if self.loading.is_empty() {
            return;
        }
        let mut finished = Vec::new();
        let mut decoded = 0usize;
        for &key in &self.loading {
            let Some(entry) = self.entries.get_mut(&key) else {
                finished.push(key);
                continue;
            };
            if let Some(result) = entry.item.get() {
                if let Ok(image) = result
                    && entry.bytes == 0
                {
                    entry.bytes = decoded_bytes(&image);
                    decoded = decoded.saturating_add(entry.bytes);
                }
                finished.push(key);
            }
        }
        for key in finished {
            self.loading.remove(&key);
        }
        self.bytes = self.bytes.saturating_add(decoded);
    }

    /// Drops the least recently used images until the budget is met again.
    fn evict(&mut self, keep: u64, _window: &mut Window, cx: &mut App) {
        let mut dropped: Vec<Arc<RenderImage>> = Vec::new();
        let mut kept_tick: Option<u64> = None;
        while self.bytes > self.max_bytes || self.entries.len() > self.max_entries {
            let Some((tick, hash)) = self.lru.pop_first() else {
                break;
            };
            if hash == keep {
                kept_tick = Some(tick);
                continue;
            }
            self.loading.remove(&hash);
            let Some(mut entry) = self.entries.remove(&hash) else {
                continue;
            };
            self.bytes = self.bytes.saturating_sub(entry.bytes);
            STAT_EVICTIONS.fetch_add(1, Ordering::Relaxed);
            if let Some(Ok(image)) = entry.item.get() {
                dropped.push(image);
            }
        }
        if let Some(tick) = kept_tick {
            self.lru.insert(tick, keep);
        }
        if dropped.is_empty() {
            return;
        }
        cx.defer(move |cx| {
            for image in dropped {
                cx.drop_image(image, None);
            }
        });
    }
}

impl ImageCache for BoundedImageCache {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        let key = hash(resource);
        self.settle();
        let mut failed_decode = false;
        let loaded = if let Some(entry) = self.entries.get_mut(&key) {
            STAT_HITS.fetch_add(1, Ordering::Relaxed);
            let result = entry.item.get();
            // A broken or missing file must not be cached forever: the next
            // draw retries, after the file has maybe been downloaded.
            failed_decode = matches!(result, Some(Err(_)));
            result
        } else {
            STAT_MISSES.fetch_add(1, Ordering::Relaxed);
            // A decode of every visible sticker at once is what made the
            // process reach gigabytes: the skipped ones are retried on the
            // next draw, when one of these two finished.
            if self.in_flight() >= MAX_IN_FLIGHT {
                self.sync_stats();
                return None;
            }
            let task = match resource {
                Resource::Embedded(name) if name.starts_with(STILL_PREFIX) => {
                    let path = still_path(name).expect("the prefix was just checked");
                    cx.background_executor()
                        .spawn(async move { decode_still_path(&path) })
                        .shared()
                }
                Resource::Path(path) if is_not_svg(path) => {
                    let path = path.clone();
                    cx.background_executor()
                        .spawn(async move { decode_path(&path) })
                        .shared()
                }
                _ => {
                    let future = AssetLogger::<ImageAssetLoader>::load(resource.clone(), cx);
                    cx.background_executor().spawn(future).shared()
                }
            };
            self.entries.insert(
                key,
                Entry {
                    item: ImageCacheItem::Loading(task.clone()),
                    bytes: 0,
                    last_used: 0,
                },
            );
            self.loading.insert(key);
            // A load that finished after the draw has to ask for the next
            // frame, or the image stays blank until something else repaints.
            let view = window.current_view();
            window
                .spawn(cx, async move |cx| {
                    _ = task.await;
                    cx.on_next_frame(move |_, cx| cx.notify(view));
                })
                .detach();
            None
        };
        if failed_decode {
            self.entries.remove(&key);
            self.lru.retain(|_, hash| *hash != key);
            self.loading.remove(&key);
        }
        self.touch(key);
        self.evict(key, window, cx);
        self.sync_stats();
        loaded
    }
}

/// SVG is vector: it goes through the loader of GPUI, which knows how to ask
/// the renderer for it.
fn is_not_svg(path: &Path) -> bool {
    !path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"))
}

fn decode_path(path: &Path) -> Result<Arc<RenderImage>, ImageCacheError> {
    let bytes = std::fs::read(path).map_err(|error| ImageCacheError::Io(Arc::new(error)))?;
    let format = image::guess_format(&bytes).map_err(image_error)?;
    let frames = match format {
        ImageFormat::Gif => decode_gif(&bytes)?,
        ImageFormat::WebP => decode_webp(&bytes)?,
        _ => vec![still_frame(&bytes, format, PHOTO_EDGE)?],
    };
    if frames.is_empty() {
        return Err(other("the image could not be decoded: no frames"));
    }
    Ok(Arc::new(RenderImage::new(frames)))
}

fn decode_gif(bytes: &[u8]) -> Result<Vec<Frame>, ImageCacheError> {
    let decoder = GifDecoder::new(Cursor::new(bytes)).map_err(image_error)?;
    animated_frames(decoder)
}

fn decode_webp(bytes: &[u8]) -> Result<Vec<Frame>, ImageCacheError> {
    let mut decoder =
        WebPDecoder::new(Cursor::new(bytes)).map_err(image_error)?;
    if !decoder.has_animation() {
        return Ok(vec![still_frame(
            bytes,
            ImageFormat::WebP,
            ANIMATED_STICKER_EDGE * 2,
        )?]);
    }
    let _ = decoder.set_background_color(Rgba([0, 0, 0, 0]));
    animated_frames(decoder)
}

fn animated_frames<'a>(
    decoder: impl AnimationDecoder<'a>,
) -> Result<Vec<Frame>, ImageCacheError> {
    let mut frames = Vec::new();
    for frame in decoder.into_frames() {
        let frame = frame.map_err(image_error)?;
        let delay = frame.delay();
        let buffer = downscale(frame.into_buffer(), ANIMATED_STICKER_EDGE);
        frames.push(Frame::from_parts(bgra(buffer), 0, 0, delay));
    }
    Ok(frames)
}

/// Decodes one file as a single frame, the shape the sticker grid asks for.
fn decode_still_path(path: &Path) -> Result<Arc<RenderImage>, ImageCacheError> {
    let bytes = std::fs::read(path).map_err(|error| ImageCacheError::Io(Arc::new(error)))?;
    let format = image::guess_format(&bytes).map_err(image_error)?;
    let frame = match format {
        ImageFormat::Gif => first_gif_frame(&bytes)?,
        ImageFormat::WebP => first_webp_frame(&bytes)?,
        _ => still_frame(&bytes, format, STILL_EDGE)?,
    };
    Ok(Arc::new(RenderImage::new(vec![frame])))
}

fn first_gif_frame(bytes: &[u8]) -> Result<Frame, ImageCacheError> {
    let decoder = GifDecoder::new(Cursor::new(bytes)).map_err(image_error)?;
    first_animated_frame(decoder)
}

fn first_webp_frame(bytes: &[u8]) -> Result<Frame, ImageCacheError> {
    let mut decoder = WebPDecoder::new(Cursor::new(bytes)).map_err(image_error)?;
    if !decoder.has_animation() {
        return still_frame(bytes, ImageFormat::WebP, STILL_EDGE);
    }
    let _ = decoder.set_background_color(Rgba([0, 0, 0, 0]));
    first_animated_frame(decoder)
}

/// Decodes only the first frame of an animated file.
fn first_animated_frame<'a>(
    decoder: impl AnimationDecoder<'a>,
) -> Result<Frame, ImageCacheError> {
    let frame = decoder
        .into_frames()
        .next()
        .ok_or_else(|| other("the image could not be decoded: no frames"))?
        .map_err(image_error)?;
    let delay = frame.delay();
    let buffer = downscale(frame.into_buffer(), STILL_EDGE);
    Ok(Frame::from_parts(bgra(buffer), 0, 0, delay))
}

fn still_frame(bytes: &[u8], format: ImageFormat, edge: u32) -> Result<Frame, ImageCacheError> {
    let image = image::load_from_memory_with_format(bytes, format)
        .map_err(image_error)?
        .into_rgba8();
    Ok(Frame::from_parts(
        bgra(downscale(image, edge)),
        0,
        0,
        Delay::from_numer_denom_ms(0, 1),
    ))
}

/// Cuts the image down to `edge` on its longest side, keeping the ratio.
fn downscale(image: RgbaImage, edge: u32) -> RgbaImage {
    let (width, height) = image.dimensions();
    let longest = width.max(height);
    if longest <= edge {
        return image;
    }
    let scale = f64::from(edge) / f64::from(longest);
    let width = ((f64::from(width) * scale).round() as u32).max(1);
    let height = ((f64::from(height) * scale).round() as u32).max(1);
    image::imageops::resize(&image, width, height, FilterType::Triangle)
}

/// GPUI paints BGRA; the decoders give RGBA.
fn bgra(mut image: RgbaImage) -> RgbaImage {
    for pixel in image.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    image
}

fn image_error(error: image::ImageError) -> ImageCacheError {
    ImageCacheError::Image(Arc::new(error))
}

fn other(message: &str) -> ImageCacheError {
    ImageCacheError::Other(Arc::new(anyhow::anyhow!(message.to_owned())))
}

/// The bytes one decoded image takes: every frame is a whole RGBA bitmap.
fn decoded_bytes(image: &RenderImage) -> usize {
    let frames = image.frame_count().max(1);
    let size = image.size(0);
    let width = usize::try_from(size.width.0).unwrap_or(0);
    let height = usize::try_from(size.height.0).unwrap_or(0);
    frames
        .saturating_mul(width)
        .saturating_mul(height)
        .saturating_mul(4)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba};

    type Buffer = ImageBuffer<Rgba<u8>, Vec<u8>>;

    fn image(width: u32, height: u32, frames: usize) -> RenderImage {
        let frame = || {
            let buffer: Buffer =
                ImageBuffer::from_raw(width, height, vec![0; (width * height * 4) as usize])
                    .expect("the buffer matches the size");
            Frame::new(buffer)
        };
        RenderImage::new((0..frames).map(|_| frame()).collect::<Vec<_>>())
    }

    #[test]
    fn one_frame_counts_its_whole_rgba_bitmap() {
        assert_eq!(decoded_bytes(&image(512, 512, 1)), 512 * 512 * 4);
        // Every frame of an animated sticker counts.
        assert_eq!(decoded_bytes(&image(512, 512, 244)), 244 * 512 * 512 * 4);
        // An image without frames takes nothing.
        assert_eq!(decoded_bytes(&RenderImage::new(Vec::new())), 0);
    }

    #[test]
    fn a_still_request_keeps_the_file_and_its_own_key() {
        let path = Path::new("C:/media/fav-1.webp");
        let name = still_image_source(path);
        assert!(name.starts_with(STILL_PREFIX));
        assert_eq!(still_path(&name), Some(PathBuf::from(path)));
        // Un recurso que no es nuestro no se toca.
        assert_eq!(still_path("fav-1.webp"), None);
    }

    #[test]
    fn a_still_request_decodes_only_the_first_frame() {
        use image::codecs::gif::GifEncoder;

        let frame = || {
            let buffer: Buffer = ImageBuffer::from_pixel(4, 4, Rgba([10, 20, 30, 255]));
            Frame::new(buffer)
        };
        let mut bytes = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut bytes);
            encoder
                .encode_frames([frame(), frame()])
                .expect("the gif encodes");
        }
        let dir = std::env::temp_dir().join(format!(
            "whatfast-still-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("the clock is after the epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("the folder is created");
        let path = dir.join("two-frames.gif");
        std::fs::write(&path, &bytes).expect("the gif is written");

        let image = decode_still_path(&path).expect("the gif decodes");
        assert_eq!(image.frame_count(), 1);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn a_big_frame_is_cut_to_the_display_edge_keeping_the_ratio() {
        let source: Buffer = ImageBuffer::from_pixel(1024, 512, Rgba([1, 2, 3, 255]));
        let scaled = downscale(source, 256);
        assert_eq!(scaled.dimensions(), (256, 128));
        // A small frame keeps its size.
        let source: Buffer = ImageBuffer::from_pixel(128, 128, Rgba([1, 2, 3, 255]));
        let scaled = downscale(source, 256);
        assert_eq!(scaled.dimensions(), (128, 128));
    }

    #[test]
    fn the_buffer_of_gpui_is_bgra() {
        let source: Buffer = ImageBuffer::from_pixel(1, 1, Rgba([10, 20, 30, 255]));
        let converted = bgra(source);
        assert_eq!(converted.get_pixel(0, 0).0, [30, 20, 10, 255]);
    }

    #[test]
    fn battery_saver_reduces_budget_constants() {
        const {
            assert!(BATTERY_SAVER_MAX_BYTES < MAX_BYTES);
            assert!(BATTERY_SAVER_MAX_ENTRIES < MAX_ENTRIES);
        }
    }
}
