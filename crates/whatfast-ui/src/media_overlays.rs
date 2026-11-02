//! Dibuja las capas flotantes del editor sobre la foto final.
//!
//! La vista previa del editor pinta los textos, los emojis y las formas como
//! `div` flotantes sobre la imagen. El archivo que se manda no pasaba por esas
//! capas: el destinatario recibía la foto sin las anotaciones. Este módulo
//! rasteriza las tres capas en el `RgbaImage` antes del recorte.
//!
//! Las coordenadas de las capas están en píxeles de la foto rotada. Los
//! tamaños (letra, emoji, borde) están en píxeles de pantalla, porque así los
//! dibuja la vista previa; `screen_per_pixel` los lleva a píxeles de la foto.

use fontdue::{Font, FontSettings};
use image::RgbaImage;

use crate::media_editor::{EmojiOverlay, ShapeKind, ShapeOverlay, TextOverlay};

/// El tamaño de la letra del texto flotante, en píxeles de pantalla: el mismo
/// `text_lg` que usa la vista previa.
const TEXT_SIZE: f32 = 18.0;
/// El aire horizontal de la caja del texto (`px_2p5`).
const TEXT_PAD_X: f32 = 10.0;
/// El aire vertical de la caja del texto (`py_1`).
const TEXT_PAD_Y: f32 = 4.0;
/// El borde de una forma (`border_3`).
const SHAPE_BORDER: f32 = 3.0;

/// Dibuja las tres capas en el orden en que las apila la vista previa:
/// formas, textos y emojis.
pub(crate) fn draw(
    image: &mut RgbaImage,
    shapes: &[ShapeOverlay],
    texts: &[TextOverlay],
    emojis: &[EmojiOverlay],
    screen_per_pixel: f64,
) {
    let scale = if screen_per_pixel.is_finite() && screen_per_pixel > 0.0 {
        screen_per_pixel
    } else {
        1.0
    };
    for shape in shapes {
        draw_shape(image, shape, scale);
    }
    if !texts.is_empty() {
        let font = ui_font();
        if let Some(font) = font.as_ref() {
            for text in texts {
                draw_text(image, text, font, scale);
            }
        }
    }
    if !emojis.is_empty()
        && let Ok(face) = ttf_parser::Face::parse(crate::emoji_font::bytes(), 0)
    {
        for emoji in emojis {
            draw_emoji(image, emoji, &face, scale);
        }
    }
}

/// La fuente del texto de la interfaz, si el sistema tiene una.
fn ui_font() -> Option<Font> {
    for path in text_font_candidates() {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        if let Ok(font) = Font::from_bytes(bytes, FontSettings::default()) {
            return Some(font);
        }
    }
    None
}

/// Las rutas de las fuentes que pueden dibujar el texto del usuario.
fn text_font_candidates() -> &'static [&'static str] {
    #[cfg(windows)]
    {
        &[
            r"C:\Windows\Fonts\segoeuib.ttf",
            r"C:\Windows\Fonts\segoeui.ttf",
            r"C:\Windows\Fonts\arialbd.ttf",
            r"C:\Windows\Fonts\arial.ttf",
        ]
    }
    #[cfg(target_os = "macos")]
    {
        &[
            "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
            "/System/Library/Fonts/Helvetica.ttc",
        ]
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        &[
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf",
        ]
    }
}

fn draw_text(image: &mut RgbaImage, overlay: &TextOverlay, font: &Font, scale: f64) {
    let size = (TEXT_SIZE as f64 / scale).max(1.0) as f32;
    let pad_x = TEXT_PAD_X as f64 / scale;
    let pad_y = TEXT_PAD_Y as f64 / scale;
    let left = overlay.x + pad_x;
    let top = overlay.y + pad_y;

    let lines: Vec<&str> = overlay.text.split('\n').collect();
    let line_height = font
        .horizontal_line_metrics(size)
        .map(|metrics| metrics.new_line_size)
        .unwrap_or(size * 1.25);
    let width = lines
        .iter()
        .map(|line| {
            line.chars()
                .map(|character| font.metrics(character, size).advance_width)
                .sum::<f32>()
        })
        .fold(0.0_f32, f32::max) as f64;

    if overlay.has_bg {
        fill_rect(
            image,
            left - pad_x,
            top - pad_y,
            width + pad_x * 2.0,
            line_height as f64 * lines.len() as f64 + pad_y * 2.0,
            [0, 0, 0, 0xaa],
        );
    }

    let ascent = font
        .horizontal_line_metrics(size)
        .map(|metrics| metrics.ascent)
        .unwrap_or(size * 0.8);
    for (index, line) in lines.iter().enumerate() {
        let baseline = top + ascent as f64 + index as f64 * line_height as f64;
        let mut pen_x = left;
        for character in line.chars() {
            let advance = font.metrics(character, size).advance_width;
            let (metrics, bitmap) = font.rasterize(character, size);
            if metrics.width > 0 && metrics.height > 0 {
                let origin_x = pen_x + metrics.xmin as f64;
                let origin_y = baseline - metrics.height as f64 - metrics.ymin as f64;
                draw_coverage(image, origin_x, origin_y, &metrics, &bitmap, overlay.color);
            }
            pen_x += advance as f64;
        }
    }
}

fn draw_coverage(
    image: &mut RgbaImage,
    origin_x: f64,
    origin_y: f64,
    metrics: &fontdue::Metrics,
    bitmap: &[u8],
    color: [u8; 4],
) {
    for row in 0..metrics.height {
        for column in 0..metrics.width {
            let coverage = bitmap[row * metrics.width + column];
            if coverage == 0 {
                continue;
            }
            let x = origin_x.round() as i64 + column as i64;
            let y = origin_y.round() as i64 + row as i64;
            let alpha = coverage as f32 / 255.0 * (color[3] as f32 / 255.0);
            blend(image, x, y, color, alpha);
        }
    }
}

fn draw_shape(image: &mut RgbaImage, shape: &ShapeOverlay, scale: f64) {
    let border = (SHAPE_BORDER as f64 / scale).max(1.0);
    match shape.kind {
        ShapeKind::Rectangle => {
            stroke_rect(image, shape.x, shape.y, shape.width, shape.height, border, shape.color);
        }
        ShapeKind::Circle => {
            stroke_ellipse(
                image,
                shape.x,
                shape.y,
                shape.width,
                shape.height,
                border,
                shape.color,
            );
        }
        ShapeKind::Arrow => {
            // La vista previa dibuja un glifo en la esquina. El archivo dibuja
            // la flecha sobre la caja entera: es la forma que el usuario
            // espera cuando arrastra el trazo de una punta a la otra.
            let (x0, y0) = (shape.x, shape.y);
            let (x1, y1) = (shape.x + shape.width, shape.y + shape.height);
            draw_line(image, x0, y0, x1, y1, border, shape.color);
            let angle = (y1 - y0).atan2(x1 - x0);
            let head = (shape.width.hypot(shape.height) * 0.18).max(border * 4.0);
            for offset in [2.5_f64, -2.5] {
                let end = angle + offset;
                draw_line(
                    image,
                    x1,
                    y1,
                    x1 - head * end.cos(),
                    y1 - head * end.sin(),
                    border,
                    shape.color,
                );
            }
        }
    }
}

fn stroke_rect(
    image: &mut RgbaImage,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    thickness: f64,
    color: [u8; 4],
) {
    fill_rect(image, x, y, width, thickness, color);
    fill_rect(image, x, y + height - thickness, width, thickness, color);
    fill_rect(image, x, y, thickness, height, color);
    fill_rect(image, x + width - thickness, y, thickness, height, color);
}

fn stroke_ellipse(
    image: &mut RgbaImage,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    thickness: f64,
    color: [u8; 4],
) {
    let radius_x = width / 2.0;
    let radius_y = height / 2.0;
    if radius_x <= 0.0 || radius_y <= 0.0 {
        return;
    }
    let center_x = x + radius_x;
    let center_y = y + radius_y;
    let half = thickness / 2.0;
    let min_x = (x - half).floor().max(0.0) as u32;
    let max_x = (x + width + half).ceil().min(image.width() as f64) as u32;
    let min_y = (y - half).floor().max(0.0) as u32;
    let max_y = (y + height + half).ceil().min(image.height() as f64) as u32;
    for pixel_y in min_y..max_y {
        for pixel_x in min_x..max_x {
            let dx = (pixel_x as f64 + 0.5 - center_x) / radius_x;
            let dy = (pixel_y as f64 + 0.5 - center_y) / radius_y;
            let distance = dx.hypot(dy);
            let ring = (distance - 1.0).abs() * radius_x.min(radius_y);
            if ring <= half {
                blend(image, pixel_x as i64, pixel_y as i64, color, color[3] as f32 / 255.0);
            }
        }
    }
}

fn fill_rect(image: &mut RgbaImage, x: f64, y: f64, width: f64, height: f64, color: [u8; 4]) {
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    let min_x = x.floor().max(0.0) as u32;
    let max_x = (x + width).ceil().min(image.width() as f64) as u32;
    let min_y = y.floor().max(0.0) as u32;
    let max_y = (y + height).ceil().min(image.height() as f64) as u32;
    let alpha = color[3] as f32 / 255.0;
    for pixel_y in min_y..max_y {
        for pixel_x in min_x..max_x {
            blend(image, pixel_x as i64, pixel_y as i64, color, alpha);
        }
    }
}

fn draw_line(
    image: &mut RgbaImage,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    thickness: f64,
    color: [u8; 4],
) {
    let half = thickness / 2.0;
    let min_x = x0.min(x1).floor().max(0.0) as u32;
    let max_x = x0.max(x1).ceil().min(image.width() as f64) as u32;
    let min_y = y0.min(y1).floor().max(0.0) as u32;
    let max_y = y0.max(y1).ceil().min(image.height() as f64) as u32;
    let alpha = color[3] as f32 / 255.0;
    for pixel_y in min_y..max_y {
        for pixel_x in min_x..max_x {
            let point_x = pixel_x as f64 + 0.5;
            let point_y = pixel_y as f64 + 0.5;
            if distance_to_segment(point_x, point_y, x0, y0, x1, y1) <= half {
                blend(image, pixel_x as i64, pixel_y as i64, color, alpha);
            }
        }
    }
}

fn distance_to_segment(px: f64, py: f64, x0: f64, y0: f64, x1: f64, y1: f64) -> f64 {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let length = dx * dx + dy * dy;
    if length <= f64::EPSILON {
        return (px - x0).hypot(py - y0);
    }
    let t = (((px - x0) * dx + (py - y0) * dy) / length).clamp(0.0, 1.0);
    (px - (x0 + t * dx)).hypot(py - (y0 + t * dy))
}

fn emojis_in(text: &str) -> Vec<&'static str> {
    if let Some(emoji) = emojis::get(text) {
        return vec![emoji.as_str()];
    }
    let mut found = Vec::new();
    for character in text.chars() {
        let mut buffer = [0_u8; 4];
        if let Some(emoji) = emojis::get(character.encode_utf8(&mut buffer)) {
            found.push(emoji.as_str());
        }
    }
    found
}

fn draw_emoji(
    image: &mut RgbaImage,
    overlay: &EmojiOverlay,
    face: &ttf_parser::Face<'_>,
    scale: f64,
) {
    let size = (overlay.size as f64 / scale).max(1.0);
    let mut left = overlay.x - size / 2.0;
    let top = overlay.y - size / 2.0;
    for emoji in emojis_in(&overlay.emoji) {
        let Some(character) = emoji
            .chars()
            .find(|character| *character != '\u{fe0f}')
        else {
            continue;
        };
        let Some(glyph) = face.glyph_index(character) else {
            left += size;
            continue;
        };
        let Some(raster) = face.glyph_raster_image(glyph, size as u16) else {
            left += size;
            continue;
        };
        if raster.format != ttf_parser::RasterImageFormat::PNG {
            left += size;
            continue;
        }
        let Ok(decoded) = image::load_from_memory(raster.data) else {
            left += size;
            continue;
        };
        let decoded = decoded.to_rgba8();
        let resized = image::imageops::resize(
            &decoded,
            size.round() as u32,
            size.round() as u32,
            image::imageops::FilterType::Triangle,
        );
        draw_rgba(image, &resized, left, top);
        left += size;
    }
}

fn draw_rgba(image: &mut RgbaImage, source: &RgbaImage, x: f64, y: f64) {
    let left = x.round() as i64;
    let top = y.round() as i64;
    for (source_x, source_y, pixel) in source.enumerate_pixels() {
        let alpha = pixel[3] as f32 / 255.0;
        if alpha == 0.0 {
            continue;
        }
        blend(
            image,
            left + source_x as i64,
            top + source_y as i64,
            [pixel[0], pixel[1], pixel[2], 255],
            alpha,
        );
    }
}

fn blend(image: &mut RgbaImage, x: i64, y: i64, color: [u8; 4], alpha: f32) {
    if alpha <= 0.0 || x < 0 || y < 0 || x >= image.width() as i64 || y >= image.height() as i64 {
        return;
    }
    let alpha = alpha.clamp(0.0, 1.0);
    let pixel = image.get_pixel_mut(x as u32, y as u32);
    for channel in 0..3 {
        pixel[channel] = (color[channel] as f32 * alpha + pixel[channel] as f32 * (1.0 - alpha))
            .clamp(0.0, 255.0) as u8;
    }
    pixel[3] = 255;
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn white(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_pixel(width, height, Rgba([255, 255, 255, 255]))
    }

    #[test]
    fn a_rectangle_leaves_a_border_in_the_picture() {
        let mut image = white(120, 120);
        let shape = ShapeOverlay {
            kind: ShapeKind::Rectangle,
            x: 20.0,
            y: 20.0,
            width: 80.0,
            height: 60.0,
            color: [255, 0, 0, 255],
        };
        draw(&mut image, &[shape], &[], &[], 1.0);
        assert_eq!(image.get_pixel(20, 20)[0], 255);
        assert_eq!(image.get_pixel(20, 20)[1], 0);
        assert_eq!(image.get_pixel(50, 50)[0], 255);
        assert_eq!(image.get_pixel(50, 50)[1], 255);
    }

    #[test]
    fn a_text_overlay_changes_pixels() {
        let mut image = white(200, 80);
        let text = TextOverlay {
            text: "HOLA".to_string(),
            x: 10.0,
            y: 10.0,
            color: [0, 0, 0, 255],
            has_bg: false,
        };
        draw(&mut image, &[], &[text], &[], 1.0);
        let dark = image
            .pixels()
            .filter(|pixel| pixel[0] < 200 && pixel[1] < 200 && pixel[2] < 200)
            .count();
        assert!(dark > 0, "the text left no pixel in the picture");
    }

    #[test]
    fn a_zero_scale_does_not_divide_by_zero() {
        let mut image = white(40, 40);
        let shape = ShapeOverlay {
            kind: ShapeKind::Rectangle,
            x: 5.0,
            y: 5.0,
            width: 20.0,
            height: 20.0,
            color: [0, 0, 0, 255],
        };
        draw(&mut image, &[shape], &[], &[], 0.0);
        assert_eq!(image.get_pixel(5, 5)[0], 0);
    }
}
