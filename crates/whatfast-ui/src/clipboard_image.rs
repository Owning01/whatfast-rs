//! Copy a picture to the clipboard in the format that Windows reads.
//!
//! GPUI hands a picture to the clipboard in its own registered format
//! ("PNG" or "JFIF"). Paint, Word and the chat clients do not look for that
//! format: they ask for a DIB and paste nothing, so the copy looks broken.
//! This module decodes the file and publishes a CF_DIB, the uncompressed
//! bitmap that every program of the system understands.

use std::path::Path;

/// Copies the picture of `path` to the clipboard as a DIB.
pub(crate) fn copy_image_file(path: &Path) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    copy_image_bytes(&bytes)
}

/// Copies a picture that is already in memory to the clipboard.
pub(crate) fn copy_image_bytes(bytes: &[u8]) -> Result<(), String> {
    let picture = image::load_from_memory(bytes)
        .map_err(|error| error.to_string())?
        .to_rgba8();
    #[cfg(windows)]
    {
        let dib = dib_from_rgba(&picture)?;
        write_dib(&dib)
    }
    #[cfg(not(windows))]
    {
        let png = png_from_rgba(&picture)?;
        write_png_clipboard(&png)
    }
}

/// Encodes an RGBA image into PNG bytes for X11/Wayland clipboards.
#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn png_from_rgba(picture: &image::RgbaImage) -> Result<Vec<u8>, String> {
    use image::ImageEncoder as _;

    let (width, height) = picture.dimensions();
    if width == 0 || height == 0 {
        return Err("the picture has no pixels".to_string());
    }
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(picture.as_raw(), width, height, image::ExtendedColorType::Rgba8)
        .map_err(|error| error.to_string())?;
    Ok(png)
}

/// The bytes of the DIB of a picture: the header of a 32-bit bitmap and the
/// pixels in BGRA, the bottom row first.
pub(crate) fn dib_from_rgba(picture: &image::RgbaImage) -> Result<Vec<u8>, String> {
    let (width, height) = picture.dimensions();
    if width == 0 || height == 0 {
        return Err("the picture has no pixels".to_string());
    }
    let size = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .and_then(|bytes| u32::try_from(bytes).ok())
        .ok_or_else(|| "the picture is too big".to_string())?;

    // BITMAPINFOHEADER. The height goes positive: the rows are stored from
    // the bottom, which is what every reader of CF_DIB expects.
    let mut dib = Vec::with_capacity(40 + size as usize);
    dib.extend_from_slice(&40u32.to_le_bytes());
    dib.extend_from_slice(&(width as i32).to_le_bytes());
    dib.extend_from_slice(&(height as i32).to_le_bytes());
    dib.extend_from_slice(&1u16.to_le_bytes());
    dib.extend_from_slice(&32u16.to_le_bytes());
    dib.extend_from_slice(&0u32.to_le_bytes());
    dib.extend_from_slice(&size.to_le_bytes());
    dib.extend_from_slice(&2835i32.to_le_bytes());
    dib.extend_from_slice(&2835i32.to_le_bytes());
    dib.extend_from_slice(&0u32.to_le_bytes());
    dib.extend_from_slice(&0u32.to_le_bytes());

    let row_bytes = (width as usize) * 4;
    for row in picture.as_raw().chunks_exact(row_bytes).rev() {
        for &[r, g, b, _] in row.as_chunks::<4>().0 {
            // BI_RGB with 32 bits keeps the channels in BGRA. The alpha goes
            // opaque: most programs ignore it and a zero would paint black.
            dib.extend_from_slice(&[b, g, r, 0xFF]);
        }
    }
    Ok(dib)
}

/// Copies the picture of an image message: the full file when the device has
/// it, or the thumbnail it already downloaded.
pub(crate) fn copy_image_content(image: &whatfast_protocol::ImageContent) -> Result<(), String> {
    if !image.local_path.is_empty() {
        copy_image_file(Path::new(&image.local_path))
    } else if !image.thumbnail_path.is_empty() {
        copy_image_file(Path::new(&image.thumbnail_path))
    } else if !image.jpeg_thumbnail.is_empty() {
        copy_image_bytes(&image.jpeg_thumbnail)
    } else {
        Err("the picture is not on this device".to_string())
    }
}

#[cfg(windows)]
fn write_dib(dib: &[u8]) -> Result<(), String> {
    use std::ffi::c_void;

    /// The clipboard format of a device-independent bitmap.
    const CF_DIB: u32 = 8;
    /// The handle moves: the clipboard takes ownership of it.
    const GMEM_MOVEABLE: u32 = 0x0002;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn OpenClipboard(owner: *mut c_void) -> i32;
        fn EmptyClipboard() -> i32;
        fn SetClipboardData(format: u32, memory: *mut c_void) -> *mut c_void;
        fn CloseClipboard() -> i32;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalAlloc(flags: u32, bytes: usize) -> *mut c_void;
        fn GlobalLock(memory: *mut c_void) -> *mut c_void;
        fn GlobalUnlock(memory: *mut c_void) -> i32;
        fn GlobalFree(memory: *mut c_void) -> *mut c_void;
    }

    // Another program can hold the clipboard for a moment: retry briefly
    // instead of losing the copy.
    let mut opened = false;
    for _ in 0..20 {
        if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
            opened = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if !opened {
        return Err("another program is using the clipboard".to_string());
    }

    let result = unsafe {
        if EmptyClipboard() == 0 {
            Err("the clipboard could not be emptied".to_string())
        } else {
            let memory = GlobalAlloc(GMEM_MOVEABLE, dib.len());
            if memory.is_null() {
                Err("there is no memory for the picture".to_string())
            } else {
                let pointer = GlobalLock(memory);
                if pointer.is_null() {
                    GlobalFree(memory);
                    Err("the picture could not be locked".to_string())
                } else {
                    std::ptr::copy_nonoverlapping(dib.as_ptr(), pointer as *mut u8, dib.len());
                    GlobalUnlock(memory);
                    if SetClipboardData(CF_DIB, memory).is_null() {
                        GlobalFree(memory);
                        Err("the system refused the picture".to_string())
                    } else {
                        // The system owns the memory now.
                        Ok(())
                    }
                }
            }
        }
    };
    unsafe { CloseClipboard() };
    result
}

#[cfg(not(windows))]
fn write_png_clipboard(png: &[u8]) -> Result<(), String> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let mut candidates: Vec<(&str, &[&str])> = Vec::new();
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        candidates.push(("wl-copy", &["--type", "image/png"]));
    }
    if std::env::var_os("DISPLAY").is_some() {
        candidates.push(("xclip", &["-selection", "clipboard", "-t", "image/png"]));
    }
    if candidates.is_empty() {
        candidates.push(("wl-copy", &["--type", "image/png"]));
        candidates.push(("xclip", &["-selection", "clipboard", "-t", "image/png"]));
    }

    for (program, args) in candidates {
        let Ok(mut child) = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue;
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(png);
        }
        if child.wait().is_ok_and(|status| status.success()) {
            return Ok(());
        }
    }
    Err("Install wl-copy (Wayland) or xclip (X11) to copy images to the clipboard".to_string())
}

#[cfg(test)]
mod tests {
    use super::{dib_from_rgba, png_from_rgba};

    /// La cabecera es la de un bitmap de 32 bits con las filas de abajo
    /// hacia arriba.
    #[test]
    fn the_dib_starts_with_the_header_of_a_32_bit_bitmap() {
        let picture = image::RgbaImage::from_pixel(3, 2, image::Rgba([10, 20, 30, 255]));
        let dib = dib_from_rgba(&picture).expect("the picture has pixels");

        assert_eq!(&dib[0..4], &40u32.to_le_bytes());
        assert_eq!(&dib[4..8], &3i32.to_le_bytes());
        assert_eq!(&dib[8..12], &2i32.to_le_bytes());
        assert_eq!(&dib[12..14], &1u16.to_le_bytes());
        assert_eq!(&dib[14..16], &32u16.to_le_bytes());
        assert_eq!(&dib[16..20], &0u32.to_le_bytes());
        assert_eq!(&dib[20..24], &(3u32 * 2 * 4).to_le_bytes());
        assert_eq!(dib.len(), 40 + 3 * 2 * 4);
    }

    /// La primera fila del DIB es la ultima de la imagen, y cada pixel va en
    /// el orden BGRA.
    #[test]
    fn the_dib_stores_the_bottom_row_first_in_bgra() {
        let mut picture = image::RgbaImage::new(1, 2);
        picture.put_pixel(0, 0, image::Rgba([1, 2, 3, 255]));
        picture.put_pixel(0, 1, image::Rgba([4, 5, 6, 255]));
        let dib = dib_from_rgba(&picture).expect("the picture has pixels");

        assert_eq!(&dib[40..44], &[6, 5, 4, 0xFF]);
        assert_eq!(&dib[44..48], &[3, 2, 1, 0xFF]);
    }

    #[test]
    fn png_encoding_produces_valid_png_signature() {
        let picture = image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255]));
        let png = png_from_rgba(&picture).expect("png bytes");
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]));
    }
}
