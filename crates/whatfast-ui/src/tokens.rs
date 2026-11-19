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

use gpui::{Pixels, Rgba, Window, px};
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

/// Applies high-contrast accessibility overrides to the active theme colors.
pub(crate) fn apply_accessibility_colors(
    high_contrast: bool,
    dark: bool,
    cx: &mut gpui::App,
) {
    if !high_contrast {
        return;
    }
    let global = gpui_component::Theme::global_mut(cx);
    let fg = global.colors.foreground;
    global.colors.muted_foreground = fg.opacity(0.92);
    global.colors.border = fg.opacity(if dark { 0.45 } else { 0.55 });
    global.colors.ring = if dark {
        gpui::rgb(0x38bdf8).into()
    } else {
        gpui::rgb(0x0052cc).into()
    };
    let values = global.colors;
    global.tokens = gpui_component::theme::ThemeTokens::from(values);
}

/// The theme the app paints with.
///
/// The app wears the palette of WhatsApp: the colors come from the
/// reference screenshots of WhatsApp Desktop
/// (`docs/referencia/01-whatsapp-chats.png`). The catalog of the core keeps
/// the themes of the web client, so this is only the choice of the app.
pub(crate) fn app_theme(appearance: ThemeAppearance) -> Theme {
    let id = match appearance {
        ThemeAppearance::Dark => "whatsapp-dark",
        ThemeAppearance::Light => "whatsapp-light",
    };
    whatfast_core::theme::builtin_themes()
        .iter()
        .find(|theme| theme.id == id)
        .cloned()
        .unwrap_or_else(whatfast_core::theme::default_theme)
}

/// The theme of an id: a builtin one, a custom one, or the dark palette.
///
/// port: `availableThemes()` of the origin: the catalog of the core plus the
/// themes of the profile. An id that is not there (a file of a newer release,
/// a theme that was deleted) falls back to the palette of the app instead of
/// leaving the window without colors.
pub(crate) fn theme_by_id(id: &str, custom: &[Theme]) -> Theme {
    if !id.is_empty() {
        if let Some(theme) = custom.iter().find(|theme| theme.id == id) {
            return theme.clone();
        }
        if let Some(theme) = whatfast_core::theme::builtin_themes()
            .iter()
            .find(|theme| theme.id == id)
        {
            return theme.clone();
        }
    }
    app_theme(ThemeAppearance::Dark)
}

/// The color of one token of a theme, for the previews of the settings.
pub(crate) fn token_color(theme: &Theme, token: ThemeToken) -> Rgba {
    parse_color(theme.tokens.get(token)).unwrap_or_else(|| gpui::rgb(0x808080))
}

/// The colors of the bubbles of the messages.
///
/// The theme of `gpui_component` has no colors for the bubbles, so the ones
/// of Whatfast are used: the received messages go on the gray of the
/// reference and the sent ones on its green.
///
/// The theme is the one in use and not the one of the appearance: a theme
/// the user made has its own bubbles, and asking by appearance painted the
/// palette of the app over it.
pub(crate) fn bubble_colors(theme: &Theme) -> (Rgba, Rgba) {
    let colors = ThemeColors::resolve(theme);
    (colors.bubble_in_bg, colors.bubble_out_bg)
}

/// The colors of the text over the bubbles: the received and the sent ones.
///
/// A card inside a bubble does not use the palette of the window: its text
/// has to be readable over the color of its own bubble.
pub(crate) fn bubble_text_colors(theme: &Theme) -> (Rgba, Rgba) {
    let colors = ThemeColors::resolve(theme);
    (colors.bubble_in_fg, colors.bubble_out_fg)
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

