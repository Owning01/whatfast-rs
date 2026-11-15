//! Geometry for the image editor: crop rectangle and rotation.
//!
//! Port of `tauri/src/lib/media-crop.ts`.
//!
//! Coordinate spaces
//! -----------------
//! - SOURCE space: the pixels of the original image. The origin is the top-left
//!   corner, `x` grows to the right and `y` grows downwards.
//! - ROTATED space: the pixels of the whole source image AFTER the requested
//!   rotation has been applied. Every crop rectangle that leaves this module
//!   through `EditPlan::crop` is expressed in this space, and the Rust side
//!   must crop the rotated image with it (rotate first, crop second).
//!
//! Rotation convention
//! --------------------
//! All rotations are CLOCKWISE, and the rotated canvas is the source image
//! rotated around its own centre. A source point `(x, y)` maps to:
//!
//!   -   0 degrees -> `(x, y)`                  canvas stays `width x height`
//!   -  90 degrees -> `(height - y, x)`         canvas becomes `height x width`
//!   - 180 degrees -> `(width - x, height - y)` canvas stays `width x height`
//!   - 270 degrees -> `(y, width - x)`          canvas becomes `height x width`
//!
//! The Rust side must use the same convention. A pixel `(x, y)` of the rotated
//! image therefore reads the source pixel `(y, width - 1 - x)` for 90 degrees.
//!
//! Rounding rule
//! -------------
//! Every pixel coordinate is rounded exactly once with `Math.round` (halves go
//! up, towards +infinity). The order is: interpret the drag in float pixels
//! (a negative width or height means the drag went up or left and moves the
//! origin), round to whole pixels, then clamp into the source bounds. The
//! result of every function that returns pixels is therefore an integer and
//! never `NaN` or `Infinity`.
//!
//! port: the original throws a `RangeError` for a broken source size. The
//! port returns `Result` with `InvalidSourceSize` instead. `thiserror` is not
//! an authorised dependency of this crate (RULEBOOK 1.3), so the error type
//! implements `Display` and `Error` by hand.

use std::fmt;

/// Size of a bitmap in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PixelSize {
    pub width: f64,
    pub height: f64,
}

/// An axis-aligned rectangle in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CropRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A clockwise rotation in degrees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum Rotation {
    Deg0 = 0,
    Deg90 = 90,
    Deg180 = 180,
    Deg270 = 270,
}

impl Rotation {
    /// The four clockwise steps, in order.
    pub const ANGLES: [Rotation; 4] = [
        Rotation::Deg0,
        Rotation::Deg90,
        Rotation::Deg180,
        Rotation::Deg270,
    ];

    /// The angle in degrees.
    pub fn degrees(self) -> u32 {
        self as u32
    }

    /// Maps a plain number onto the four quarter turns, without a type
    /// assertion.
    ///
    /// port: the original compares with `===`, so only the exact angles match
    /// and everything else falls back to 0.
    pub fn from_number(value: f64) -> Rotation {
        for angle in Rotation::ANGLES {
            if angle.degrees() as f64 == value {
                return angle;
            }
        }
        Rotation::Deg0
    }
}

/// The full description of one edit, ready to be handed to the Rust side.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EditPlan {
    pub rotation: Rotation,
    /// The crop rectangle, expressed in the ROTATED coordinate space.
    pub crop: CropRect,
    /// Size of the produced image, in pixels.
    pub output: PixelSize,
}

/// A source size that is not finite or is negative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidSourceSize {
    /// The label the original used in the message: `source`, `target` or
    /// `canvas`.
    pub label: &'static str,
    pub message: String,
}

impl fmt::Display for InvalidSourceSize {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl std::error::Error for InvalidSourceSize {}

const FALLBACK_RECT: CropRect = CropRect {
    x: 0.0,
    y: 0.0,
    width: 1.0,
    height: 1.0,
};

fn clamp(value: f64, min: f64, max: f64) -> f64 {
    if max < min {
        return min;
    }
    js_min(js_max(value, min), max)
}

/// port: `Math.max` returns `NaN` when any argument is `NaN`.
fn js_max(left: f64, right: f64) -> f64 {
    if left.is_nan() || right.is_nan() {
        return f64::NAN;
    }
    if left > right {
        return left;
    }
    if right > left {
        return right;
    }
    // port: `Math.max` prefers `+0` over `-0`.
    if left == 0.0 && right == 0.0 && left.is_sign_negative() {
        return right;
    }
    left
}

/// port: `Math.min` returns `NaN` when any argument is `NaN`.
fn js_min(left: f64, right: f64) -> f64 {
    if left.is_nan() || right.is_nan() {
        return f64::NAN;
    }
    if left < right {
        return left;
    }
    if right < left {
        return right;
    }
    // port: `Math.min` prefers `-0` over `+0`.
    if left == 0.0 && right == 0.0 && left.is_sign_positive() {
        return right;
    }
    left
}

/// Rounds to a whole pixel. Non-finite input becomes 0 and negative zero is
/// normalised to positive zero so the result is always a plain integer.
///
/// port: `Math.round` rounds halves towards +Infinity, `f64::round` rounds
/// them away from zero.
fn integral(value: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    let rounded = js_round(value);
    if rounded == 0.0 { 0.0 } else { rounded }
}

/// port: `Math.round`, halves go up towards +Infinity.
fn js_round(value: f64) -> f64 {
    if value.is_nan() {
        return f64::NAN;
    }
    (value + 0.5).floor()
}

/// A source size is a programming error when it is not finite or negative:
/// unlike a crop rectangle there is nothing sensible to clamp it to.
fn assert_source_size(size: PixelSize, label: &'static str) -> Result<(), InvalidSourceSize> {
    if !size.width.is_finite() || !size.height.is_finite() {
        return Err(InvalidSourceSize {
            label,
            message: format!(
                "{} size must be finite, received {}x{}",
                label,
                js_number_to_string(size.width),
                js_number_to_string(size.height)
            ),
        });
    }
    if size.width < 0.0 || size.height < 0.0 {
        return Err(InvalidSourceSize {
            label,
            message: format!(
                "{} size must not be negative, received {}x{}",
                label,
                js_number_to_string(size.width),
                js_number_to_string(size.height)
            ),
        });
    }
    Ok(())
}

/// port: `String(number)` of the original.
fn js_number_to_string(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 {
            "Infinity".to_string()
        } else {
            "-Infinity".to_string()
        };
    }
    format!("{value}")
}

fn rotate_point(point_x: f64, point_y: f64, source: PixelSize, rotation: Rotation) -> (f64, f64) {
    match rotation {
        Rotation::Deg90 => (source.height - point_y, point_x),
        Rotation::Deg180 => (source.width - point_x, source.height - point_y),
        Rotation::Deg270 => (point_y, source.width - point_x),
        Rotation::Deg0 => (point_x, point_y),
    }
}

/// Clamps a crop rectangle into the source image.
///
/// A negative width or height means the drag went left or up: the origin moves
/// with it. The rectangle is then rounded to whole pixels and clamped to the
/// source bounds with a minimum of 1x1. A rectangle that lies completely
/// outside the source collapses to a 1x1 rectangle at the source origin.
/// Non-finite components are treated as 0 instead of throwing.
pub fn normalize_crop_rect(
    rect: CropRect,
    source: PixelSize,
) -> Result<CropRect, InvalidSourceSize> {
    assert_source_size(source, "source")?;

    let source_width = integral(source.width);
    let source_height = integral(source.height);

    let mut x = if rect.x.is_finite() { rect.x } else { 0.0 };
    let mut y = if rect.y.is_finite() { rect.y } else { 0.0 };
    let mut width = if rect.width.is_finite() {
        rect.width
    } else {
        0.0
    };
    let mut height = if rect.height.is_finite() {
        rect.height
    } else {
        0.0
    };

    if width < 0.0 {
        x += width;
        width = -width;
    }
    if height < 0.0 {
        y += height;
        height = -height;
    }

    let outside = x >= source_width || x + width <= 0.0 || y >= source_height || y + height <= 0.0;
    if outside {
        return Ok(FALLBACK_RECT);
    }

    let origin_x = clamp(integral(x), 0.0, js_max(0.0, source_width - 1.0));
    let origin_y = clamp(integral(y), 0.0, js_max(0.0, source_height - 1.0));

    Ok(CropRect {
        x: origin_x,
        y: origin_y,
        width: clamp(integral(width), 1.0, js_max(1.0, source_width - origin_x)),
        height: clamp(integral(height), 1.0, js_max(1.0, source_height - origin_y)),
    })
}

/// The size of the image after the rotation is applied. 90 and 270 degrees
/// swap the width and the height.
pub fn rotated_size(size: PixelSize, rotation: Rotation) -> Result<PixelSize, InvalidSourceSize> {
    assert_source_size(size, "source")?;
    if rotation == Rotation::Deg90 || rotation == Rotation::Deg270 {
        return Ok(PixelSize {
            width: size.height,
            height: size.width,
        });
    }
    Ok(PixelSize {
        width: size.width,
        height: size.height,
    })
}

/// Converts a crop rectangle from the SOURCE space into the ROTATED space.
///
/// The rectangle is normalised first, then its four corners are rotated and the
/// bounding box of the rotated corners is returned. Because the bounding box
/// is taken from the corners, the result is correct for any rectangle and not
/// only for one anchored at the origin. The result always stays inside the
/// rotated canvas.
pub fn crop_in_rotated_space(
    rect: CropRect,
    source: PixelSize,
    rotation: Rotation,
) -> Result<CropRect, InvalidSourceSize> {
    assert_source_size(source, "source")?;
    let normalized = normalize_crop_rect(rect, source)?;

    let right = normalized.x + normalized.width;
    let bottom = normalized.y + normalized.height;
    let corners: [(f64, f64); 4] = [
        (normalized.x, normalized.y),
        (right, normalized.y),
        (normalized.x, bottom),
        (right, bottom),
    ];

    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for (corner_x, corner_y) in corners {
        let (rotated_x, rotated_y) = rotate_point(corner_x, corner_y, source, rotation);
        min_x = js_min(min_x, rotated_x);
        min_y = js_min(min_y, rotated_y);
        max_x = js_max(max_x, rotated_x);
        max_y = js_max(max_y, rotated_y);
    }

    Ok(CropRect {
        x: min_x,
        y: min_y,
        width: max_x - min_x,
        height: max_y - min_y,
    })
}

/// Builds the edit plan for one image: the requested crop is normalised in the
/// SOURCE space, moved into the ROTATED space and the output size is the size
/// of that rotated crop. The Rust side rotates the whole image first and
/// crops it afterwards with `plan.crop`.
pub fn plan_edit(
    source: PixelSize,
    requested_crop: CropRect,
    rotation: Rotation,
) -> Result<EditPlan, InvalidSourceSize> {
    assert_source_size(source, "source")?;
    let normalized = normalize_crop_rect(requested_crop, source)?;
    let crop = crop_in_rotated_space(normalized, source, rotation)?;
    Ok(EditPlan {
        rotation,
        crop,
        output: PixelSize {
            width: crop.width,
            height: crop.height,
        },
    })
}

/// Normalises any number of degrees into the nearest quarter turn. Ties go
/// clockwise (45 becomes 90, -45 becomes 0) and non-finite input becomes 0.
pub fn parse_rotation(degrees: f64) -> Rotation {
    if !degrees.is_finite() {
        return Rotation::Deg0;
    }
    // port: `Math.round` rounds the halves towards +Infinity, so -45 becomes
    // 0 and not -1.
    let steps = js_round(degrees / 90.0);
    Rotation::from_number(((steps * 90.0) % 360.0 + 360.0) % 360.0)
}
