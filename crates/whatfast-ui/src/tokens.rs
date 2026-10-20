//! The design tokens of the interface: the scale, the lengths, the colors and
//! the shadows.
//!
//! port: the oracle draws the interface with CSS (`tauri/src/styles.css`) and
//! writes the theme of the app onto the document root as custom properties
//! (`tauri/src/lib/theme.ts`). The model of the theme — the thirty tokens of
//! every theme — already lives in `whatfast_core::theme`; this module is the
//! bridge from those strings to the values GPUI paints with.
//!
//! Two sets of radii live in the stylesheet and they are NOT the same set:
//! - the modern style (the `:root` of the stylesheet) uses 6px, 8px and 12px,
//! - the skeuomorphic style (`html[data-theme-style="skeuomorphic"]`) replaces
//!   them with the tighter 5px, 7px and 9px.
//!
//! `--radius-full` (999px) is a literal in both blocks: the scale never
//! touches it.
//!
//! The stylesheet declares NO theme custom property: the thirty colors of the
//! theme are written at runtime by `applyTheme` of `lib/theme.ts`. The
//! default values of the theme therefore live in `whatfast_core::theme`, and
//! the test `the_default_theme_matches_the_values_of_the_oracle` pins them
//! against the literals of `lib/theme.ts`.

// The module arrives before its callers: the shell, the composer and the
// settings of the batches B-U2..B-U6 read the tokens from here.
#![allow(dead_code)]

use gpui::{BoxShadow, Pixels, Rgba, Window, px};
use gpui_component::theme::ThemeColor;
use whatfast_core::theme::{Theme, ThemeAppearance, ThemeToken, ThemeVisualStyle};

/// The smallest interface scale.
pub(crate) const MIN_UI_SCALE: f32 = 1.0;

/// The biggest interface scale.
pub(crate) const MAX_UI_SCALE: f32 = 1.5;

/// The step of the interface scale.
pub(crate) const UI_SCALE_STEP: f32 = 0.1;

/// The interface scale of a profile that never changed it.
pub(crate) const DEFAULT_UI_SCALE: f32 = 1.0;

/// The size of the interface font, before the scale.
pub(crate) const BASE_UI_FONT_SIZE: f32 = 16.0;

/// The size of the monospace font, before the scale.
pub(crate) const BASE_MONO_FONT_SIZE: f32 = 13.0;

/// The green of WhatsApp.
///
/// It is a color of the brand: it does not change with the theme.
pub(crate) const DARK_GREEN: u32 = 0x075e54;

/// The color of the panel of the brand.
///
/// It is a color of the brand: it does not change with the theme.
pub(crate) const PANEL: u32 = 0xf7faf9;

/// The height of the title bar, before the scale.
const TITLEBAR_HEIGHT: f32 = 38.0;

/// The height of the header of a panel, before the scale.
const HEADER_HEIGHT: f32 = 56.0;

/// The height of the composer, before the scale.
const COMPOSER_HEIGHT: f32 = 60.0;

/// The width of the navigation rail, before the scale.
///
/// port: the first column of the grid of the title bar and of the shell.
const NAV_RAIL_WIDTH: f32 = 56.0;

/// The width of the chat list, before the scale.
///
/// port: the second column of the grid of the title bar and of the shell.
const CHAT_LIST_WIDTH: f32 = 300.0;

/// The smallest width of the chat list, before the scale.
///
/// port: the second column of the grid is `minmax(260px, 300px)`.
const CHAT_LIST_MIN_WIDTH: f32 = 260.0;

/// The green of WhatsApp, as GPUI paints it.
pub(crate) fn dark_green() -> Rgba {
    gpui::rgb(DARK_GREEN)
}

/// The color of the panel of the brand, as GPUI paints it.
pub(crate) fn panel() -> Rgba {
    gpui::rgb(PANEL)
}

/// Normalizes an interface scale.
///
/// port: `normalized_ui_scale` of the origin: the scale is clamped between
/// 1.0 and 1.5 and rounded to a tenth. The origin filtered the non-finite
/// values in `load_ui_scale`, before the normalization; here the filter is
/// inside the normalization, so the result is never a NaN.
pub(crate) fn normalized_ui_scale(scale: f32) -> f32 {
    if scale.is_nan() {
        return DEFAULT_UI_SCALE;
    }
    ((scale.clamp(MIN_UI_SCALE, MAX_UI_SCALE) * 10.0).round()) / 10.0
}

/// The sizes of the fonts, after the scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FontSizes {
    /// The size of the interface font.
    pub(crate) font_size: Pixels,
    /// The size of the monospace font.
    pub(crate) mono_font_size: Pixels,
}

/// The sizes of the fonts for an interface scale.
///
/// port: `apply_theme_scale` of the origin multiplies the base sizes by the
/// scale: 16px for the interface font and 13px for the monospace one.
pub(crate) fn font_sizes(scale: f32) -> FontSizes {
    FontSizes {
        font_size: px(BASE_UI_FONT_SIZE * scale),
        mono_font_size: px(BASE_MONO_FONT_SIZE * scale),
    }
}

/// The radii of a visual style, in unscaled pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RadiusSpec {
    /// The radius of the small elements.
    pub(crate) sm: f32,
    /// The radius of the medium elements.
    pub(crate) md: f32,
    /// The radius of the large elements.
    pub(crate) lg: f32,
    /// The radius of the pills.
    pub(crate) full: f32,
}

/// The radii of the modern style.
///
/// port: the `:root` of the stylesheet.
pub(crate) const MODERN_RADII: RadiusSpec = RadiusSpec {
    sm: 6.0,
    md: 8.0,
    lg: 12.0,
    full: 999.0,
};

/// The radii of the skeuomorphic style.
///
/// port: `html[data-theme-style="skeuomorphic"]` replaces the three radii of
/// the `:root` with tighter ones. The radius of the pills does not change.
pub(crate) const SKEUOMORPHIC_RADII: RadiusSpec = RadiusSpec {
    sm: 5.0,
    md: 7.0,
    lg: 9.0,
    full: 999.0,
};

impl RadiusSpec {
    /// The radii of a visual style.
    pub(crate) fn for_style(style: ThemeVisualStyle) -> Self {
        match style {
            ThemeVisualStyle::Modern => MODERN_RADII,
            ThemeVisualStyle::Skeuomorphic => SKEUOMORPHIC_RADII,
        }
    }
}

/// The radii of the interface, in pixels, after the scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Radii {
    /// The radius of the small elements.
    pub(crate) sm: Pixels,
    /// The radius of the medium elements.
    pub(crate) md: Pixels,
    /// The radius of the large elements.
    pub(crate) lg: Pixels,
    /// The radius of the pills.
    pub(crate) full: Pixels,
}

impl Radii {
    /// The radii of a visual style for an interface scale.
    pub(crate) fn new(spec: RadiusSpec, scale: f32) -> Self {
        Self {
            sm: px(spec.sm * scale),
            md: px(spec.md * scale),
            lg: px(spec.lg * scale),
            // port: `--radius-full: 999px` is a literal in the stylesheet: the
            // scale does not touch it.
            full: px(spec.full),
        }
    }
}

/// The lengths of the interface, in pixels, after the scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Metrics {
    /// The scale the lengths were multiplied by.
    pub(crate) scale: f32,
    /// The radii of the interface.
    pub(crate) radii: Radii,
    /// The height of the title bar.
    pub(crate) titlebar_height: Pixels,
    /// The height of the header of a panel.
    pub(crate) header_height: Pixels,
    /// The height of the composer.
    pub(crate) composer_height: Pixels,
    /// The width of the navigation rail.
    pub(crate) nav_rail_width: Pixels,
    /// The width of the chat list.
    pub(crate) chat_list_width: Pixels,
    /// The smallest width of the chat list.
    pub(crate) chat_list_min_width: Pixels,
}

impl Metrics {
    /// The lengths of the interface for a scale and a visual style.
    ///
    /// The scale is normalized first, so the lengths are always the ones the
    /// interface would draw with the stored scale.
    pub(crate) fn new(scale: f32, style: ThemeVisualStyle) -> Self {
        let scale = normalized_ui_scale(scale);
        Self {
            scale,
            radii: Radii::new(RadiusSpec::for_style(style), scale),
            titlebar_height: px(TITLEBAR_HEIGHT * scale),
            header_height: px(HEADER_HEIGHT * scale),
            composer_height: px(COMPOSER_HEIGHT * scale),
            nav_rail_width: px(NAV_RAIL_WIDTH * scale),
            chat_list_width: px(CHAT_LIST_WIDTH * scale),
            chat_list_min_width: px(CHAT_LIST_MIN_WIDTH * scale),
        }
    }
}

/// A shadow of a theme, in the values of the stylesheet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ThemeShadow {
    /// The horizontal offset of the shadow.
    pub(crate) offset_x: f32,
    /// The vertical offset of the shadow.
    pub(crate) offset_y: f32,
    /// The blur radius of the shadow.
    pub(crate) blur_radius: f32,
    /// The spread radius of the shadow.
    pub(crate) spread_radius: f32,
    /// The color of the shadow.
    pub(crate) color: Rgba,
    /// Whether the shadow is drawn inside the bounds of the element.
    pub(crate) inset: bool,
}

impl ThemeShadow {
    /// The shadow, as GPUI paints it.
    pub(crate) fn to_box_shadow(self) -> BoxShadow {
        let mut shadow = BoxShadow::new(px(self.offset_x), px(self.offset_y), self.color.into());
        shadow = shadow.blur_radius(px(self.blur_radius));
        shadow = shadow.spread_radius(px(self.spread_radius));
        if self.inset {
            shadow = shadow.inset();
        }
        shadow
    }
}

/// Converts a CSS color into an RGB color.
///
/// port: the tokens of a theme are the strings of the CSS custom properties.
/// The oracle let the browser parse them, so this function has to accept
/// every notation the built-in themes use: `#rrggbb` and `rgba(r, g, b, a)`.
/// The short hex forms and the percentages are accepted too, because an
/// imported theme can carry them.
pub(crate) fn parse_color(value: &str) -> Option<Rgba> {
    let value = value.trim();
    match value.strip_prefix('#') {
        Some(hex) => parse_hex_color(hex),
        None => parse_function_color(value),
    }
}

/// Converts the hexadecimal form of a CSS color into an RGB color.
fn parse_hex_color(hex: &str) -> Option<Rgba> {
    let digits: Option<Vec<u8>> = hex
        .chars()
        .map(|character| character.to_digit(16).map(|digit| digit as u8))
        .collect();
    let digits = digits?;
    match digits.len() {
        3 => Some(Rgba {
            r: f32::from(digits[0] * 17) / 255.0,
            g: f32::from(digits[1] * 17) / 255.0,
            b: f32::from(digits[2] * 17) / 255.0,
            a: 1.0,
        }),
        4 => Some(Rgba {
            r: f32::from(digits[0] * 17) / 255.0,
            g: f32::from(digits[1] * 17) / 255.0,
            b: f32::from(digits[2] * 17) / 255.0,
            a: f32::from(digits[3] * 17) / 255.0,
        }),
        6 => Some(Rgba {
            r: f32::from(digits[0] * 16 + digits[1]) / 255.0,
            g: f32::from(digits[2] * 16 + digits[3]) / 255.0,
            b: f32::from(digits[4] * 16 + digits[5]) / 255.0,
            a: 1.0,
        }),
        8 => Some(Rgba {
            r: f32::from(digits[0] * 16 + digits[1]) / 255.0,
            g: f32::from(digits[2] * 16 + digits[3]) / 255.0,
            b: f32::from(digits[4] * 16 + digits[5]) / 255.0,
            a: f32::from(digits[6] * 16 + digits[7]) / 255.0,
        }),
        _ => None,
    }
}

/// Converts the functional form of a CSS color into an RGB color.
fn parse_function_color(value: &str) -> Option<Rgba> {
    let lowered = value.to_ascii_lowercase();
    let body = lowered
        .strip_prefix("rgba(")
        .or_else(|| lowered.strip_prefix("rgb("))?;
    let body = body.strip_suffix(')')?;
    let (channels, alpha) = if body.contains(',') {
        let parts: Vec<&str> = body.split(',').map(str::trim).collect();
        if parts.len() < 3 || parts.len() > 4 {
            return None;
        }
        (parts[..3].to_vec(), parts.get(3).copied())
    } else {
        match body.split_once('/') {
            Some((left, right)) => (
                left.split_whitespace().collect::<Vec<&str>>(),
                Some(right.trim()),
            ),
            None => (body.split_whitespace().collect::<Vec<&str>>(), None),
        }
    };
    if channels.len() != 3 {
        return None;
    }
    Some(Rgba {
        r: parse_channel(channels[0])?,
        g: parse_channel(channels[1])?,
        b: parse_channel(channels[2])?,
        a: match alpha {
            Some(alpha) => parse_alpha(alpha)?,
            None => 1.0,
        },
    })
}

/// Converts one channel of a CSS color, in bytes or in percent.
fn parse_channel(value: &str) -> Option<f32> {
    if let Some(percent) = value.strip_suffix('%') {
        let number: f32 = percent.trim().parse().ok()?;
        if !number.is_finite() {
            return None;
        }
        Some((number / 100.0).clamp(0.0, 1.0))
    } else {
        let number: f32 = value.parse().ok()?;
        if !number.is_finite() {
            return None;
        }
        Some((number / 255.0).clamp(0.0, 1.0))
    }
}

/// Converts the alpha of a CSS color, in units or in percent.
fn parse_alpha(value: &str) -> Option<f32> {
    if let Some(percent) = value.strip_suffix('%') {
        let number: f32 = percent.trim().parse().ok()?;
        if !number.is_finite() {
            return None;
        }
        Some((number / 100.0).clamp(0.0, 1.0))
    } else {
        let number: f32 = value.parse().ok()?;
        if !number.is_finite() {
            return None;
        }
        Some(number.clamp(0.0, 1.0))
    }
}

/// Converts a length of the stylesheet into pixels.
fn parse_length(value: &str) -> Option<f32> {
    let number = value.strip_suffix("px").unwrap_or(value);
    let number: f32 = number.trim().parse().ok()?;
    if !number.is_finite() {
        return None;
    }
    Some(number)
}

/// Converts the `box-shadow` of a theme into a shadow of the interface.
///
/// port: the shadow tokens of the themes are the `box-shadow` values of the
/// stylesheet: an offset, an optional blur, an optional spread and a color.
pub(crate) fn parse_shadow(value: &str) -> Option<ThemeShadow> {
    let mut rest = value.trim();
    let mut inset = false;
    if let Some(after) = rest.strip_prefix("inset") {
        inset = true;
        rest = after.trim_start();
    }
    let mut lengths: Vec<f32> = Vec::new();
    while lengths.len() < 4 {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let Some(length) = parse_length(&rest[..end]) else {
            break;
        };
        lengths.push(length);
        rest = rest[end..].trim_start();
    }
    if lengths.len() < 2 {
        return None;
    }
    let color = parse_color(rest)?;
    Some(ThemeShadow {
        offset_x: lengths[0],
        offset_y: lengths[1],
        blur_radius: lengths.get(2).copied().unwrap_or(0.0),
        spread_radius: lengths.get(3).copied().unwrap_or(0.0),
        color,
        inset,
    })
}

/// The colors and the shadows of a theme, already converted for GPUI.
///
/// The thirty fields are the thirty tokens of the theme, in the order of
/// `whatfast_core::theme::THEME_TOKENS`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ThemeColors {
    /// The background of the application.
    pub(crate) bg_app: Rgba,
    /// The background of the panels.
    pub(crate) bg_panel: Rgba,
    /// The background of the surfaces above the panels.
    pub(crate) bg_elevated: Rgba,
    /// The background of a hovered element.
    pub(crate) bg_hover: Rgba,
    /// The background of an active element.
    pub(crate) bg_active: Rgba,
    /// The background of the inputs.
    pub(crate) bg_input: Rgba,
    /// The background of the overlays.
    pub(crate) bg_overlay: Rgba,
    /// The color of the borders.
    pub(crate) border: Rgba,
    /// The color of the strong borders.
    pub(crate) border_strong: Rgba,
    /// The color of the focus ring.
    pub(crate) border_focus: Rgba,
    /// The color of the text.
    pub(crate) fg: Rgba,
    /// The color of the secondary text.
    pub(crate) fg_muted: Rgba,
    /// The color of the tertiary text.
    pub(crate) fg_subtle: Rgba,
    /// The color of the text over an inverted background.
    pub(crate) fg_inverted: Rgba,
    /// The color of the accent.
    pub(crate) accent: Rgba,
    /// The color of the hovered accent.
    pub(crate) accent_hover: Rgba,
    /// The color of the text over the accent.
    pub(crate) accent_fg: Rgba,
    /// The soft color of the accent.
    pub(crate) accent_soft: Rgba,
    /// The background of the incoming bubbles.
    pub(crate) bubble_in_bg: Rgba,
    /// The color of the text of the incoming bubbles.
    pub(crate) bubble_in_fg: Rgba,
    /// The background of the outgoing bubbles.
    pub(crate) bubble_out_bg: Rgba,
    /// The color of the text of the outgoing bubbles.
    pub(crate) bubble_out_fg: Rgba,
    /// The color of the bar of a quoted message.
    pub(crate) quote_bar: Rgba,
    /// The color of the success.
    pub(crate) success: Rgba,
    /// The color of the warning.
    pub(crate) warning: Rgba,
    /// The color of the danger.
    pub(crate) danger: Rgba,
    /// The color of the information.
    pub(crate) info: Rgba,
    /// The smallest shadow.
    pub(crate) shadow_sm: ThemeShadow,
    /// The medium shadow.
    pub(crate) shadow_md: ThemeShadow,
    /// The biggest shadow.
    pub(crate) shadow_lg: ThemeShadow,
}

impl ThemeColors {
    /// The colors of a theme.
    ///
    /// A token that is not a color the converter understands falls back to the
    /// value of the same token of the default theme: the oracle left the
    /// invalid value to the browser, which dropped the declaration.
    pub(crate) fn resolve(theme: &Theme) -> Self {
        let fallback = whatfast_core::theme::default_theme();
        Self {
            bg_app: resolve_color(theme, &fallback, ThemeToken::BgApp),
            bg_panel: resolve_color(theme, &fallback, ThemeToken::BgPanel),
            bg_elevated: resolve_color(theme, &fallback, ThemeToken::BgElevated),
            bg_hover: resolve_color(theme, &fallback, ThemeToken::BgHover),
            bg_active: resolve_color(theme, &fallback, ThemeToken::BgActive),
            bg_input: resolve_color(theme, &fallback, ThemeToken::BgInput),
            bg_overlay: resolve_color(theme, &fallback, ThemeToken::BgOverlay),
            border: resolve_color(theme, &fallback, ThemeToken::Border),
            border_strong: resolve_color(theme, &fallback, ThemeToken::BorderStrong),
            border_focus: resolve_color(theme, &fallback, ThemeToken::BorderFocus),
            fg: resolve_color(theme, &fallback, ThemeToken::Fg),
            fg_muted: resolve_color(theme, &fallback, ThemeToken::FgMuted),
            fg_subtle: resolve_color(theme, &fallback, ThemeToken::FgSubtle),
            fg_inverted: resolve_color(theme, &fallback, ThemeToken::FgInverted),
            accent: resolve_color(theme, &fallback, ThemeToken::Accent),
            accent_hover: resolve_color(theme, &fallback, ThemeToken::AccentHover),
            accent_fg: resolve_color(theme, &fallback, ThemeToken::AccentFg),
            accent_soft: resolve_color(theme, &fallback, ThemeToken::AccentSoft),
            bubble_in_bg: resolve_color(theme, &fallback, ThemeToken::BubbleInBg),
            bubble_in_fg: resolve_color(theme, &fallback, ThemeToken::BubbleInFg),
            bubble_out_bg: resolve_color(theme, &fallback, ThemeToken::BubbleOutBg),
            bubble_out_fg: resolve_color(theme, &fallback, ThemeToken::BubbleOutFg),
            quote_bar: resolve_color(theme, &fallback, ThemeToken::QuoteBar),
            success: resolve_color(theme, &fallback, ThemeToken::Success),
            warning: resolve_color(theme, &fallback, ThemeToken::Warning),
            danger: resolve_color(theme, &fallback, ThemeToken::Danger),
            info: resolve_color(theme, &fallback, ThemeToken::Info),
            shadow_sm: resolve_shadow(theme, &fallback, ThemeToken::ShadowSm),
            shadow_md: resolve_shadow(theme, &fallback, ThemeToken::ShadowMd),
            shadow_lg: resolve_shadow(theme, &fallback, ThemeToken::ShadowLg),
        }
    }

    /// Writes the colors of the theme over the colors of `gpui_component`.
    ///
    /// The fields that Whatfast has no token for keep the value that
    /// `gpui_component` gave them.
    pub(crate) fn apply_to(&self, colors: &mut ThemeColor) {
        colors.background = self.bg_app.into();
        colors.foreground = self.fg.into();
        colors.muted_foreground = self.fg_muted.into();
        colors.muted = self.bg_hover.into();
        colors.accent = self.accent.into();
        colors.accent_foreground = self.accent_fg.into();
        colors.border = self.border.into();
        colors.ring = self.border_focus.into();
        colors.input = self.bg_input.into();
        colors.caret = self.accent.into();
        colors.drag_border = self.accent.into();
        colors.drop_target = self.accent_soft.into();
        colors.selection = self.accent_soft.into();
        colors.progress_bar = self.accent.into();
        colors.link = self.info.into();
        colors.link_hover = self.info.into();
        colors.link_active = self.info.into();
        colors.overlay = self.bg_overlay.into();
        colors.popover = self.bg_elevated.into();
        colors.popover_foreground = self.fg.into();
        colors.group_box = self.bg_elevated.into();
        colors.group_box_foreground = self.fg.into();
        colors.accordion = self.bg_elevated.into();
        colors.accordion_hover = self.bg_hover.into();
        colors.description_list_label = self.bg_elevated.into();
        colors.description_list_label_foreground = self.fg.into();
        colors.skeleton = self.bg_elevated.into();
        colors.tiles = self.bg_elevated.into();

        colors.primary = self.accent.into();
        colors.primary_foreground = self.accent_fg.into();
        colors.primary_hover = self.accent_hover.into();
        colors.primary_active = self.accent_hover.into();
        colors.secondary = self.bg_elevated.into();
        colors.secondary_foreground = self.fg.into();
        colors.secondary_hover = self.bg_hover.into();
        colors.secondary_active = self.bg_active.into();

        colors.button = self.bg_elevated.into();
        colors.button_foreground = self.fg.into();
        colors.button_hover = self.bg_hover.into();
        colors.button_active = self.bg_active.into();
        colors.button_primary = self.accent.into();
        colors.button_primary_foreground = self.accent_fg.into();
        colors.button_primary_hover = self.accent_hover.into();
        colors.button_primary_active = self.accent_hover.into();
        colors.button_secondary = self.bg_elevated.into();
        colors.button_secondary_foreground = self.fg.into();
        colors.button_secondary_hover = self.bg_hover.into();
        colors.button_secondary_active = self.bg_active.into();
        colors.button_danger = self.danger.into();
        colors.button_danger_foreground = self.fg.into();
        colors.button_danger_hover = self.danger.into();
        colors.button_danger_active = self.danger.into();
        colors.button_success = self.success.into();
        colors.button_success_foreground = self.fg.into();
        colors.button_success_hover = self.success.into();
        colors.button_success_active = self.success.into();
        colors.button_warning = self.warning.into();
        colors.button_warning_foreground = self.fg.into();
        colors.button_warning_hover = self.warning.into();
        colors.button_warning_active = self.warning.into();
        colors.button_info = self.info.into();
        colors.button_info_foreground = self.fg.into();
        colors.button_info_hover = self.info.into();
        colors.button_info_active = self.info.into();

        colors.success = self.success.into();
        colors.success_foreground = self.fg.into();
        colors.success_hover = self.success.into();
        colors.success_active = self.success.into();
        colors.warning = self.warning.into();
        colors.warning_foreground = self.fg.into();
        colors.warning_hover = self.warning.into();
        colors.warning_active = self.warning.into();
        colors.danger = self.danger.into();
        colors.danger_foreground = self.fg.into();
        colors.danger_hover = self.danger.into();
        colors.danger_active = self.danger.into();
        colors.info = self.info.into();
        colors.info_foreground = self.fg.into();
        colors.info_hover = self.info.into();
        colors.info_active = self.info.into();

        colors.sidebar = self.bg_panel.into();
        colors.sidebar_foreground = self.fg.into();
        colors.sidebar_border = self.border.into();
        colors.sidebar_accent = self.accent.into();
        colors.sidebar_accent_foreground = self.accent_fg.into();
        colors.sidebar_primary = self.accent.into();
        colors.sidebar_primary_foreground = self.accent_fg.into();
        colors.title_bar = self.bg_panel.into();
        colors.title_bar_border = self.border.into();
        colors.status_bar = self.bg_panel.into();
        colors.status_bar_border = self.border.into();
        colors.window_border = self.border_strong.into();

        colors.list = self.bg_panel.into();
        colors.list_even = self.bg_panel.into();
        colors.list_head = self.bg_elevated.into();
        colors.list_hover = self.bg_hover.into();
        colors.list_active = self.bg_active.into();
        colors.list_active_border = self.border_strong.into();

        colors.tab = self.bg_panel.into();
        colors.tab_active = self.bg_elevated.into();
        colors.tab_foreground = self.fg.into();
        colors.tab_active_foreground = self.fg.into();
        colors.tab_bar = self.bg_panel.into();
        colors.tab_bar_segmented = self.bg_elevated.into();

        colors.table = self.bg_panel.into();
        colors.table_even = self.bg_panel.into();
        colors.table_head = self.bg_elevated.into();
        colors.table_head_foreground = self.fg.into();
        colors.table_hover = self.bg_hover.into();
        colors.table_active = self.bg_active.into();
        colors.table_active_border = self.border_strong.into();
        colors.table_row_border = self.border.into();

        colors.switch = self.bg_active.into();
        colors.switch_thumb = self.fg.into();
        colors.slider_bar = self.bg_elevated.into();
        colors.slider_thumb = self.accent.into();

        colors.scrollbar = self.bg_panel.into();
        colors.scrollbar_thumb = self.fg_subtle.into();
        colors.scrollbar_thumb_hover = self.fg_muted.into();
    }
}

/// The color of a token, with the default theme as the fallback.
fn resolve_color(theme: &Theme, fallback: &Theme, token: ThemeToken) -> Rgba {
    parse_color(theme.tokens.get(token))
        .or_else(|| parse_color(fallback.tokens.get(token)))
        .unwrap_or(Rgba {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        })
}

/// The shadow of a token, with the default theme as the fallback.
fn resolve_shadow(theme: &Theme, fallback: &Theme, token: ThemeToken) -> ThemeShadow {
    parse_shadow(theme.tokens.get(token))
        .or_else(|| parse_shadow(fallback.tokens.get(token)))
        .unwrap_or(ThemeShadow {
            offset_x: 0.0,
            offset_y: 0.0,
            blur_radius: 0.0,
            spread_radius: 0.0,
            color: Rgba {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            },
            inset: false,
        })
}

/// Applies a theme of Whatfast over the theme of `gpui_component`.
///
/// The application needs `gpui_component::init` to have been called before:
/// the theme of `gpui_component` is a global of the application.
///
/// The scale is the interface scale: it sets the sizes of the fonts and the
/// radii of the interface.
///
/// The rem size of the window follows the font size, so [`apply_theme_scale`]
/// has to be called with the window after this.
pub(crate) fn apply_theme(theme: &Theme, scale: f32, cx: &mut gpui::App) {
    let mode = match theme.appearance {
        ThemeAppearance::Dark => gpui_component::ThemeMode::Dark,
        ThemeAppearance::Light => gpui_component::ThemeMode::Light,
    };
    // port: `Theme::change` rewrites every color of the theme from the
    // configuration of the mode, so the mode comes first and the tokens of
    // Whatfast land on top of it.
    gpui_component::Theme::change(mode, None, cx);

    let colors = ThemeColors::resolve(theme);
    let metrics = Metrics::new(scale, theme.visual_style);
    let fonts = font_sizes(metrics.scale);

    let global = gpui_component::Theme::global_mut(cx);
    colors.apply_to(&mut global.colors);
    // port: the tokens of `gpui_component` keep, for every color, the solid
    // color and the background that is painted. They follow the colors.
    let values = global.colors;
    global.tokens = gpui_component::theme::ThemeTokens::from(values);
    // port: the stylesheet uses the small radius for the general elements and
    // the big one for the dialogs and the panels.
    global.radius = metrics.radii.sm;
    global.radius_lg = metrics.radii.lg;
    global.font_size = fonts.font_size;
    global.mono_font_size = fonts.mono_font_size;
}

/// Applies an interface scale to the fonts of the application.
///
/// port: `apply_theme_scale` of the origin: the base font size (16px) and the
/// monospace one (13px) are multiplied by the scale, and the rem size of the
/// window follows the font size.
///
/// The radii of the interface scale too, so a change of the scale has to be
/// followed by [`apply_theme`].
pub(crate) fn apply_theme_scale(scale: f32, window: &mut Window, cx: &mut gpui::App) {
    let fonts = font_sizes(normalized_ui_scale(scale));
    let theme = gpui_component::Theme::global_mut(cx);
    theme.font_size = fonts.font_size;
    theme.mono_font_size = fonts.mono_font_size;
    window.set_rem_size(theme.font_size);
    window.refresh();
}

#[cfg(test)]
mod tests {
    use gpui::px;
    use whatfast_core::theme::{Theme, ThemeToken, ThemeVisualStyle};

    use super::{
        BASE_MONO_FONT_SIZE, BASE_UI_FONT_SIZE, DARK_GREEN, DEFAULT_UI_SCALE, MAX_UI_SCALE,
        MIN_UI_SCALE, MODERN_RADII, Metrics, PANEL, SKEUOMORPHIC_RADII, ThemeColors, ThemeShadow,
        UI_SCALE_STEP, dark_green, font_sizes, normalized_ui_scale, panel, parse_color,
        parse_shadow,
    };

    /// The color of a hexadecimal literal, as the converter has to build it.
    fn color(hex: u32) -> gpui::Rgba {
        gpui::rgb(hex)
    }

    /// The color of a `rgba()` literal of the stylesheet.
    fn rgba(red: u32, green: u32, blue: u32, alpha: f32) -> gpui::Rgba {
        gpui::Rgba {
            r: red as f32 / 255.0,
            g: green as f32 / 255.0,
            b: blue as f32 / 255.0,
            a: alpha,
        }
    }

    /// The default theme of the application.
    fn default_theme() -> Theme {
        whatfast_core::theme::default_theme()
    }

    #[test]
    fn the_scale_is_clamped_between_one_and_one_and_a_half() {
        assert_eq!(normalized_ui_scale(0.5), MIN_UI_SCALE);
        assert_eq!(normalized_ui_scale(0.0), MIN_UI_SCALE);
        assert_eq!(normalized_ui_scale(-3.0), MIN_UI_SCALE);
        assert_eq!(normalized_ui_scale(2.0), MAX_UI_SCALE);
        assert_eq!(normalized_ui_scale(99.0), MAX_UI_SCALE);
        assert_eq!(normalized_ui_scale(1.0), 1.0);
        assert_eq!(normalized_ui_scale(1.5), 1.5);
    }

    #[test]
    fn the_scale_is_rounded_to_a_tenth() {
        assert_eq!(normalized_ui_scale(1.04), 1.0);
        assert_eq!(normalized_ui_scale(1.06), 1.1);
        assert_eq!(normalized_ui_scale(1.24), 1.2);
        assert_eq!(normalized_ui_scale(1.44), 1.4);
        assert_eq!(normalized_ui_scale(1.46), 1.5);
    }

    #[test]
    fn a_scale_that_is_not_a_number_falls_back_to_the_default() {
        assert_eq!(normalized_ui_scale(f32::NAN), DEFAULT_UI_SCALE);
        assert_eq!(normalized_ui_scale(f32::INFINITY), MAX_UI_SCALE);
        assert_eq!(normalized_ui_scale(f32::NEG_INFINITY), MIN_UI_SCALE);
    }

    #[test]
    fn the_step_of_the_scale_is_a_tenth() {
        assert_eq!(UI_SCALE_STEP, 0.1);
        assert_eq!(MIN_UI_SCALE, 1.0);
        assert_eq!(MAX_UI_SCALE, 1.5);
    }

    #[test]
    fn the_sizes_of_the_fonts_follow_the_scale() {
        let plain = font_sizes(1.0);
        assert_eq!(plain.font_size, px(16.0));
        assert_eq!(plain.mono_font_size, px(13.0));

        let big = font_sizes(1.5);
        assert_eq!(big.font_size, px(24.0));
        assert_eq!(big.mono_font_size, px(19.5));

        assert_eq!(BASE_UI_FONT_SIZE, 16.0);
        assert_eq!(BASE_MONO_FONT_SIZE, 13.0);
    }

    #[test]
    fn the_radii_of_the_modern_style_come_from_the_root_of_the_stylesheet() {
        let metrics = Metrics::new(1.0, ThemeVisualStyle::Modern);
        assert_eq!(metrics.radii.sm, px(6.0));
        assert_eq!(metrics.radii.md, px(8.0));
        assert_eq!(metrics.radii.lg, px(12.0));
        assert_eq!(metrics.radii.full, px(999.0));
        assert_eq!(MODERN_RADII.sm, 6.0);
        assert_eq!(MODERN_RADII.md, 8.0);
        assert_eq!(MODERN_RADII.lg, 12.0);
    }

    #[test]
    fn the_radii_of_the_skeuomorphic_style_are_the_tighter_ones() {
        let metrics = Metrics::new(1.0, ThemeVisualStyle::Skeuomorphic);
        assert_eq!(metrics.radii.sm, px(5.0));
        assert_eq!(metrics.radii.md, px(7.0));
        assert_eq!(metrics.radii.lg, px(9.0));
        // port: `--radius-full` is a literal in the stylesheet: the scale does
        // not touch it.
        assert_eq!(metrics.radii.full, px(999.0));
        assert_eq!(SKEUOMORPHIC_RADII.sm, 5.0);
        assert_eq!(SKEUOMORPHIC_RADII.md, 7.0);
        assert_eq!(SKEUOMORPHIC_RADII.lg, 9.0);
    }

    #[test]
    fn the_measures_of_the_interface_follow_the_scale() {
        let plain = Metrics::new(1.0, ThemeVisualStyle::Modern);
        assert_eq!(plain.scale, 1.0);
        assert_eq!(plain.titlebar_height, px(38.0));
        assert_eq!(plain.header_height, px(56.0));
        assert_eq!(plain.composer_height, px(60.0));
        assert_eq!(plain.nav_rail_width, px(56.0));
        assert_eq!(plain.chat_list_width, px(300.0));
        assert_eq!(plain.chat_list_min_width, px(260.0));

        let big = Metrics::new(1.5, ThemeVisualStyle::Modern);
        assert_eq!(big.scale, 1.5);
        assert_eq!(big.titlebar_height, px(57.0));
        assert_eq!(big.header_height, px(84.0));
        assert_eq!(big.composer_height, px(90.0));
        assert_eq!(big.nav_rail_width, px(84.0));
        assert_eq!(big.chat_list_width, px(450.0));
        assert_eq!(big.chat_list_min_width, px(390.0));
        assert_eq!(big.radii.sm, px(9.0));
        assert_eq!(big.radii.md, px(12.0));
        assert_eq!(big.radii.lg, px(18.0));
        assert_eq!(big.radii.full, px(999.0));
    }

    #[test]
    fn the_measures_normalize_the_scale_first() {
        let metrics = Metrics::new(2.4, ThemeVisualStyle::Modern);
        assert_eq!(metrics.scale, MAX_UI_SCALE);
        assert_eq!(metrics.titlebar_height, px(57.0));
    }

    #[test]
    fn the_colors_of_the_brand_do_not_change_with_the_theme() {
        assert_eq!(DARK_GREEN, 0x075e54);
        assert_eq!(PANEL, 0xf7faf9);
        assert_eq!(dark_green(), color(0x075e54));
        assert_eq!(panel(), color(0xf7faf9));
    }

    #[test]
    fn a_hexadecimal_color_is_converted() {
        assert_eq!(parse_color("#000000"), Some(color(0x000000)));
        assert_eq!(parse_color("#ffffff"), Some(color(0xffffff)));
        assert_eq!(parse_color("#0072f5"), Some(color(0x0072f5)));
        assert_eq!(parse_color("#ededed"), Some(color(0xededed)));
        assert_eq!(parse_color("  #0a0a0a  "), Some(color(0x0a0a0a)));
        assert_eq!(parse_color("#fff"), Some(color(0xffffff)));
        assert_eq!(
            parse_color("#00000080"),
            Some(color(0x000000).alpha(128.0 / 255.0))
        );
    }

    #[test]
    fn a_functional_color_is_converted() {
        assert_eq!(
            parse_color("rgba(0, 0, 0, 0.72)"),
            Some(rgba(0, 0, 0, 0.72))
        );
        assert_eq!(
            parse_color("rgba(0, 114, 245, 0.16)"),
            Some(rgba(0, 114, 245, 0.16))
        );
        assert_eq!(
            parse_color("rgba(45, 54, 68, 0.58)"),
            Some(rgba(45, 54, 68, 0.58))
        );
        assert_eq!(parse_color("rgb(255, 255, 255)"), Some(color(0xffffff)));
        assert_eq!(parse_color("RGB(0, 0, 0)"), Some(color(0x000000)));
        assert_eq!(parse_color("rgb(0 0 0 / 50%)"), Some(rgba(0, 0, 0, 0.5)));
    }

    #[test]
    fn a_color_that_cannot_be_converted_is_refused() {
        assert_eq!(parse_color(""), None);
        assert_eq!(parse_color("nonsense"), None);
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("#gggggg"), None);
        assert_eq!(parse_color("rgb(1, 2)"), None);
        assert_eq!(parse_color("rgba(1, 2, 3, 4, 5)"), None);
    }

    #[test]
    fn a_shadow_is_converted() {
        let shadow = parse_shadow("0 1px 2px rgba(0, 0, 0, 0.5)");
        assert_eq!(
            shadow,
            Some(ThemeShadow {
                offset_x: 0.0,
                offset_y: 1.0,
                blur_radius: 2.0,
                spread_radius: 0.0,
                color: rgba(0, 0, 0, 0.5),
                inset: false,
            })
        );
    }

    #[test]
    fn a_shadow_without_a_blur_is_converted() {
        let shadow = parse_shadow("0 4px #000000");
        assert_eq!(
            shadow,
            Some(ThemeShadow {
                offset_x: 0.0,
                offset_y: 4.0,
                blur_radius: 0.0,
                spread_radius: 0.0,
                color: color(0x000000),
                inset: false,
            })
        );
    }

    #[test]
    fn a_shadow_that_cannot_be_converted_is_refused() {
        assert_eq!(parse_shadow(""), None);
        assert_eq!(parse_shadow("0"), None);
        assert_eq!(parse_shadow("0 1px"), None);
    }

    #[test]
    fn the_default_theme_matches_the_values_of_the_oracle() {
        // port: the oracle of these values is `tauri/src/lib/theme.ts`
        // (`vercelDark`). The stylesheet has no theme custom property: the
        // theme is written on the document root at runtime.
        let theme = default_theme();
        assert_eq!(theme.id, "vercel-dark");
        assert_eq!(theme.appearance.as_str(), "dark");
        assert_eq!(theme.visual_style.as_str(), "modern");

        let expected = [
            (ThemeToken::BgApp, "#000000"),
            (ThemeToken::BgPanel, "#0a0a0a"),
            (ThemeToken::BgElevated, "#111111"),
            (ThemeToken::BgHover, "#1a1a1a"),
            (ThemeToken::BgActive, "#242424"),
            (ThemeToken::BgInput, "#0a0a0a"),
            (ThemeToken::BgOverlay, "rgba(0, 0, 0, 0.72)"),
            (ThemeToken::Border, "#2e2e2e"),
            (ThemeToken::BorderStrong, "#454545"),
            (ThemeToken::BorderFocus, "#0072f5"),
            (ThemeToken::Fg, "#ededed"),
            (ThemeToken::FgMuted, "#a1a1a1"),
            (ThemeToken::FgSubtle, "#707070"),
            (ThemeToken::FgInverted, "#0a0a0a"),
            (ThemeToken::Accent, "#0072f5"),
            (ThemeToken::AccentHover, "#3291ff"),
            (ThemeToken::AccentFg, "#ffffff"),
            (ThemeToken::AccentSoft, "rgba(0, 114, 245, 0.16)"),
            (ThemeToken::BubbleInBg, "#161616"),
            (ThemeToken::BubbleInFg, "#ededed"),
            (ThemeToken::BubbleOutBg, "#0059c1"),
            (ThemeToken::BubbleOutFg, "#ffffff"),
            (ThemeToken::QuoteBar, "#0072f5"),
            (ThemeToken::Success, "#45d483"),
            (ThemeToken::Warning, "#f7b955"),
            (ThemeToken::Danger, "#ff6166"),
            (ThemeToken::Info, "#52a8ff"),
            (ThemeToken::ShadowSm, "0 1px 2px rgba(0, 0, 0, 0.5)"),
            (ThemeToken::ShadowMd, "0 4px 16px rgba(0, 0, 0, 0.55)"),
            (ThemeToken::ShadowLg, "0 16px 48px rgba(0, 0, 0, 0.65)"),
        ];
        assert_eq!(expected.len(), whatfast_core::theme::THEME_TOKENS.len());
        for (token, value) in expected {
            assert_eq!(theme.tokens.get(token), value, "token {:?}", token);
        }
    }

    #[test]
    fn every_token_of_the_default_theme_becomes_a_color() {
        let colors = ThemeColors::resolve(&default_theme());
        assert_eq!(colors.bg_app, color(0x000000));
        assert_eq!(colors.bg_panel, color(0x0a0a0a));
        assert_eq!(colors.bg_elevated, color(0x111111));
        assert_eq!(colors.bg_hover, color(0x1a1a1a));
        assert_eq!(colors.bg_active, color(0x242424));
        assert_eq!(colors.bg_input, color(0x0a0a0a));
        assert_eq!(colors.bg_overlay, rgba(0, 0, 0, 0.72));
        assert_eq!(colors.border, color(0x2e2e2e));
        assert_eq!(colors.border_strong, color(0x454545));
        assert_eq!(colors.border_focus, color(0x0072f5));
        assert_eq!(colors.fg, color(0xededed));
        assert_eq!(colors.fg_muted, color(0xa1a1a1));
        assert_eq!(colors.fg_subtle, color(0x707070));
        assert_eq!(colors.fg_inverted, color(0x0a0a0a));
        assert_eq!(colors.accent, color(0x0072f5));
        assert_eq!(colors.accent_hover, color(0x3291ff));
        assert_eq!(colors.accent_fg, color(0xffffff));
        assert_eq!(colors.accent_soft, rgba(0, 114, 245, 0.16));
        assert_eq!(colors.bubble_in_bg, color(0x161616));
        assert_eq!(colors.bubble_in_fg, color(0xededed));
        assert_eq!(colors.bubble_out_bg, color(0x0059c1));
        assert_eq!(colors.bubble_out_fg, color(0xffffff));
        assert_eq!(colors.quote_bar, color(0x0072f5));
        assert_eq!(colors.success, color(0x45d483));
        assert_eq!(colors.warning, color(0xf7b955));
        assert_eq!(colors.danger, color(0xff6166));
        assert_eq!(colors.info, color(0x52a8ff));
    }

    #[test]
    fn every_shadow_of_the_default_theme_becomes_a_shadow() {
        let colors = ThemeColors::resolve(&default_theme());
        assert_eq!(
            colors.shadow_sm,
            ThemeShadow {
                offset_x: 0.0,
                offset_y: 1.0,
                blur_radius: 2.0,
                spread_radius: 0.0,
                color: rgba(0, 0, 0, 0.5),
                inset: false,
            }
        );
        assert_eq!(
            colors.shadow_md,
            ThemeShadow {
                offset_x: 0.0,
                offset_y: 4.0,
                blur_radius: 16.0,
                spread_radius: 0.0,
                color: rgba(0, 0, 0, 0.55),
                inset: false,
            }
        );
        assert_eq!(
            colors.shadow_lg,
            ThemeShadow {
                offset_x: 0.0,
                offset_y: 16.0,
                blur_radius: 48.0,
                spread_radius: 0.0,
                color: rgba(0, 0, 0, 0.65),
                inset: false,
            }
        );
    }

    #[test]
    fn a_token_that_is_not_a_color_falls_back_to_the_default_theme() {
        let mut theme = default_theme();
        theme.tokens.set(ThemeToken::BgApp, "#not-a-color");
        let colors = ThemeColors::resolve(&theme);
        assert_eq!(colors.bg_app, color(0x000000));
    }

    #[test]
    fn the_light_theme_converts_its_translucent_tokens() {
        let theme = whatfast_core::theme::builtin_themes()
            .iter()
            .find(|theme| theme.id == "vercel-light")
            .unwrap();
        let colors = ThemeColors::resolve(theme);
        assert_eq!(colors.bg_app, color(0xffffff));
        assert_eq!(colors.bg_panel, color(0xfafafa));
        assert_eq!(colors.bg_overlay, rgba(255, 255, 255, 0.75));
        assert_eq!(colors.accent_soft, rgba(0, 114, 245, 0.1));
        assert_eq!(colors.fg, color(0x171717));
    }
}
