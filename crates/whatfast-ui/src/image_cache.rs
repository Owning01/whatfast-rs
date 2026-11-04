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

use std::{collections::HashMap, io::Cursor, path::Path, sync::Arc};

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
const MAX_ENTRIES: usize = 64;

/// How many images may be decoding at the same time. Each one holds every
/// frame in memory before the cache can count it.
const MAX_IN_FLIGHT: usize = 2;

/// El lado mayor de un sticker animado en pantalla. El archivo trae 512×512 y
/// la burbuja lo pinta a la mitad: decodificar el doble es memoria tirada.
const ANIMATED_STICKER_EDGE: u32 = 256;

/// El lado mayor de las fotos y de los stickers quietos.
const PHOTO_EDGE: u32 = 2048;

struct Entry {
    item: ImageCacheItem,
    /// The decoded bytes this image takes, once it finished loading.
    bytes: usize,
}

pub(crate) struct BoundedImageCache {
    entries: HashMap<u64, Entry>,
    /// The hashes, most recently used first.
    usage: Vec<u64>,
    bytes: usize,
}

impl BoundedImageCache {
    pub(crate) fn new(cx: &mut App) -> Entity<Self> {
        let cache = cx.new(|_| Self {
            entries: HashMap::new(),
            usage: Vec::new(),
            bytes: 0,
        });
        // The textures live in the windows: they have to be dropped with the
        // cache or the GPU keeps them until the process ends.
        cx.observe_release(&cache, |cache, cx| {
            for (_, mut entry) in std::mem::take(&mut cache.entries) {
                if let Some(Ok(image)) = entry.item.get() {
                    cx.drop_image(image, None);
                }
            }
            cache.usage.clear();
            cache.bytes = 0;
        })
        .detach();
        cache
    }

    fn touch(&mut self, hash: u64) {
        if let Some(position) = self.usage.iter().position(|item| *item == hash) {
            self.usage.remove(position);
        }
        self.usage.insert(0, hash);
    }

    fn in_flight(&self) -> usize {
        self.entries
            .values()
            .filter(|entry| matches!(entry.item, ImageCacheItem::Loading(_)))
            .count()
    }

    /// Moves the decodes that finished to their final size, even when the
    /// image was not drawn again. Without this an image that scrolled away
    /// stays counted as in flight forever, and two of those would stop every
    /// later load.
    fn settle(&mut self) {
        let mut decoded = 0usize;
        for entry in self.entries.values_mut() {
            if entry.bytes == 0
                && matches!(entry.item, ImageCacheItem::Loading(_))
                && let Some(Ok(image)) = entry.item.get()
            {
                entry.bytes = decoded_bytes(&image);
                decoded = decoded.saturating_add(entry.bytes);
            }
        }
        self.bytes = self.bytes.saturating_add(decoded);
    }

    /// Drops the least recently used images until the budget is met again.
    ///
    /// `keep` is the image of this draw: it is never dropped, even when it is
    /// alone over the budget, because the frame about to paint needs it.
    ///
    /// The textures leave on a deferred callback: `load` runs inside the
    /// paint, and taking a texture out of the sprite atlas in the middle of
    /// it is not safe.
    fn evict(&mut self, keep: u64, _window: &mut Window, cx: &mut App) {
        let mut dropped: Vec<Arc<RenderImage>> = Vec::new();
        while self.bytes > MAX_BYTES || self.entries.len() > MAX_ENTRIES {
            let Some(position) = self.usage.iter().rposition(|hash| *hash != keep) else {
                break;
            };
            let hash = self.usage.remove(position);
            let Some(mut entry) = self.entries.remove(&hash) else {
                continue;
            };
            self.bytes = self.bytes.saturating_sub(entry.bytes);
            if let Some(Ok(image)) = entry.item.get() {
                dropped.push(image);
            }
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
        let loaded = if let Some(entry) = self.entries.get_mut(&key) {
            entry.item.get()
        } else {
            // A decode of every visible sticker at once is what made the
            // process reach gigabytes: the skipped ones are retried on the
            // next draw, when one of these two finished.
            if self.in_flight() >= MAX_IN_FLIGHT {
                return None;
            }
            let task = match resource {
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
                },
            );
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
        self.touch(key);
        self.evict(key, window, cx);
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
        _ => vec![still_frame(&bytes, format)?],
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
        return Ok(vec![still_frame(bytes, ImageFormat::WebP)?]);
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

fn still_frame(bytes: &[u8], format: ImageFormat) -> Result<Frame, ImageCacheError> {
    let image = image::load_from_memory_with_format(bytes, format)
        .map_err(image_error)?
        .into_rgba8();
    // Un sticker quieto tambien es 512×512; una foto se queda con su detalle.
    let edge = if format == ImageFormat::WebP {
        ANIMATED_STICKER_EDGE * 2
    } else {
        PHOTO_EDGE
    };
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
    }

    #[test]
    fn every_frame_of_an_animated_sticker_counts() {
        assert_eq!(decoded_bytes(&image(512, 512, 244)), 244 * 512 * 512 * 4);
    }

    #[test]
    fn an_image_without_frames_takes_nothing() {
        assert_eq!(decoded_bytes(&RenderImage::new(Vec::new())), 0);
    }

    #[test]
    fn a_big_frame_is_cut_to_the_display_edge_keeping_the_ratio() {
        let source: Buffer = ImageBuffer::from_pixel(1024, 512, Rgba([1, 2, 3, 255]));
        let scaled = downscale(source, 256);
        assert_eq!(scaled.dimensions(), (256, 128));
    }

    #[test]
    fn a_small_frame_keeps_its_size() {
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
}
