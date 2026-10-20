//! The data model of Whatfast, without GPUI.
//!
//! The origin already had a native client (`desktop/` in `whatfast`): a GPUI
//! application whose data model, profile paths, settings file, sticker
//! preparation and emoji catalog all lived in the same crate as the toolkit.
//! This crate is that model separated: the same code, the same behaviour, the
//! same tests, and **zero** GPUI in the dependency graph. The user interface
//! crate (`whatfast-ui`) consumes it and paints it.
//!
//! The protobuf DTOs are not copied here: they come from `whatfast-protocol`,
//! the contract the bridge speaks.

pub mod emoji_picker;
pub mod model;
pub mod paths;
pub mod settings;
pub mod sticker;

/// The geometry the model needs, reimplemented without GPUI.
///
/// The origin's `model.rs` imported `gpui::{Pixels, Size, px, size}` for one
/// reason: to type the cached size of every message. GPUI's `Pixels` is a
/// `f32` newtype and its `Size<T>` is a plain width/height pair, so both are
/// rebuilt here with the same arithmetic:
///
/// - `Pixels` is `f32`. GPUI's newtype only wrapped the number, so the
///   estimates of the origin are bit-for-bit the same numbers.
/// - `Size<T>` keeps the generic shape of `gpui::Size`, so the user interface
///   can convert a cached size with `gpui::px(size.width)` and
///   `gpui::px(size.height)` without changing a single number.
pub mod geom {
    /// A length in device pixels.
    ///
    /// port: GPUI's `Pixels` is a newtype over `f32`; the alias keeps the same
    /// numeric behaviour without the wrapper.
    pub type Pixels = f32;

    /// A width/height pair, mirroring `gpui::Size`.
    #[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
    pub struct Size<T = Pixels> {
        /// The horizontal component.
        pub width: T,
        /// The vertical component.
        pub height: T,
    }

    impl<T> Size<T> {
        /// Builds a size out of its two components.
        pub const fn new(width: T, height: T) -> Self {
            Self { width, height }
        }
    }

    /// Mirrors `gpui::px`: turns a raw number into a length.
    pub const fn px(value: f32) -> Pixels {
        value
    }

    /// Mirrors `gpui::size`: builds a size out of its two components.
    pub const fn size<T>(width: T, height: T) -> Size<T> {
        Size { width, height }
    }
}
