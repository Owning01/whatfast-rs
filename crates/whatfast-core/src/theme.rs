//! Port of `tauri/src/lib/theme.ts` (SolidJS frontend, frozen oracle).
//!
//! The theme model, the normalization of an untrusted theme file, the clone,
//! the export and the catalog of the builtin themes are pure, so they live
//! here. The effect of the original — `applyTheme` writes the tokens as CSS
//! custom properties on `document.documentElement` — belongs to the UI crate:
//! this module decides *what* to write ([`theme_root_declaration`]) and the
//! UI writes it.
//!
//! Fidelity notes (the odd bits are deliberate, they are the oracle's):
//! - `isSafeTokenValue` is a hand-written matcher: the crate takes no new
//!   dependency, so the three regular expressions of the original are
//!   implemented by hand (the forbidden characters, the forbidden keywords
//!   and the `var (` check).
//! - `hash` is the FNV-1a of the original over UTF-16 code units and the
//!   result is rendered in base 36, like `Number.prototype.toString(36)`.
//! - The lengths and the slices of the original count UTF-16 code units (the
//!   strings of JS are UTF-16), so the port counts them that way.

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

/// The design tokens every theme carries.
///
/// port: the original exports them as the tuple `THEME_TOKENS` of strings.
/// The port keeps the same order and the same names, as the enum the rest of
/// the module indexes by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ThemeToken {
    BgApp,
    BgPanel,
    BgElevated,
    BgHover,
    BgActive,
    BgInput,
    BgOverlay,
    Border,
    BorderStrong,
    BorderFocus,
    Fg,
    FgMuted,
    FgSubtle,
    FgInverted,
    Accent,
    AccentHover,
    AccentFg,
    AccentSoft,
    BubbleInBg,
    BubbleInFg,
    BubbleOutBg,
    BubbleOutFg,
    QuoteBar,
    Success,
    Warning,
    Danger,
    Info,
    ShadowSm,
    ShadowMd,
    ShadowLg,
}

/// The tokens of a theme, in the order the original declares them.
pub const THEME_TOKENS: [ThemeToken; 30] = [
    ThemeToken::BgApp,
    ThemeToken::BgPanel,
    ThemeToken::BgElevated,
    ThemeToken::BgHover,
    ThemeToken::BgActive,
    ThemeToken::BgInput,
    ThemeToken::BgOverlay,
    ThemeToken::Border,
    ThemeToken::BorderStrong,
    ThemeToken::BorderFocus,
    ThemeToken::Fg,
    ThemeToken::FgMuted,
    ThemeToken::FgSubtle,
    ThemeToken::FgInverted,
    ThemeToken::Accent,
    ThemeToken::AccentHover,
    ThemeToken::AccentFg,
    ThemeToken::AccentSoft,
    ThemeToken::BubbleInBg,
    ThemeToken::BubbleInFg,
    ThemeToken::BubbleOutBg,
    ThemeToken::BubbleOutFg,
    ThemeToken::QuoteBar,
    ThemeToken::Success,
    ThemeToken::Warning,
    ThemeToken::Danger,
    ThemeToken::Info,
    ThemeToken::ShadowSm,
    ThemeToken::ShadowMd,
    ThemeToken::ShadowLg,
];

impl ThemeToken {
    /// The CSS custom property name of the token, without the `--` prefix.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BgApp => "bg-app",
            Self::BgPanel => "bg-panel",
            Self::BgElevated => "bg-elevated",
            Self::BgHover => "bg-hover",
            Self::BgActive => "bg-active",
            Self::BgInput => "bg-input",
            Self::BgOverlay => "bg-overlay",
            Self::Border => "border",
            Self::BorderStrong => "border-strong",
            Self::BorderFocus => "border-focus",
            Self::Fg => "fg",
            Self::FgMuted => "fg-muted",
            Self::FgSubtle => "fg-subtle",
            Self::FgInverted => "fg-inverted",
            Self::Accent => "accent",
            Self::AccentHover => "accent-hover",
            Self::AccentFg => "accent-fg",
            Self::AccentSoft => "accent-soft",
            Self::BubbleInBg => "bubble-in-bg",
            Self::BubbleInFg => "bubble-in-fg",
            Self::BubbleOutBg => "bubble-out-bg",
            Self::BubbleOutFg => "bubble-out-fg",
            Self::QuoteBar => "quote-bar",
            Self::Success => "success",
            Self::Warning => "warning",
            Self::Danger => "danger",
            Self::Info => "info",
            Self::ShadowSm => "shadow-sm",
            Self::ShadowMd => "shadow-md",
            Self::ShadowLg => "shadow-lg",
        }
    }
}

/// The colors and the shadows of a theme.
///
/// port: the original is `Record<ThemeToken, string>`: the thirty keys are
/// always there, so the port uses a struct instead of a map. The serde names
/// are the CSS custom property names, minus the `--`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ThemeTokens {
    pub bg_app: String,
    pub bg_panel: String,
    pub bg_elevated: String,
    pub bg_hover: String,
    pub bg_active: String,
    pub bg_input: String,
    pub bg_overlay: String,
    pub border: String,
    pub border_strong: String,
    pub border_focus: String,
    pub fg: String,
    pub fg_muted: String,
    pub fg_subtle: String,
    pub fg_inverted: String,
    pub accent: String,
    pub accent_hover: String,
    pub accent_fg: String,
    pub accent_soft: String,
    pub bubble_in_bg: String,
    pub bubble_in_fg: String,
    pub bubble_out_bg: String,
    pub bubble_out_fg: String,
    pub quote_bar: String,
    pub success: String,
    pub warning: String,
    pub danger: String,
    pub info: String,
    pub shadow_sm: String,
    pub shadow_md: String,
    pub shadow_lg: String,
}

/// The appearance of a theme: it drives `color-scheme` and the native
/// scrollbars.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeAppearance {
    #[default]
    Dark,
    Light,
}

impl ThemeAppearance {
    /// The value the original writes on the document root.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }
}

/// The structural treatment of a theme.
///
/// port: it is the deliberately small escape hatch of the original: a theme
/// opts into the shared skeuomorphic treatment, but an imported JSON file
/// cannot inject arbitrary CSS.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeVisualStyle {
    #[default]
    Modern,
    Skeuomorphic,
}

impl ThemeVisualStyle {
    /// The value the original writes on the document root.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Modern => "modern",
            Self::Skeuomorphic => "skeuomorphic",
        }
    }
}

/// A theme of the app.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Theme {
    pub id: String,
    pub name: String,
    pub appearance: ThemeAppearance,
    pub visual_style: ThemeVisualStyle,
    /// Built-in themes cannot be edited or deleted, only duplicated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtin: Option<bool>,
    pub tokens: ThemeTokens,
}

impl ThemeTokens {
    /// The value of one token.
    pub fn get(&self, token: ThemeToken) -> &str {
        match token {
            ThemeToken::BgApp => &self.bg_app,
            ThemeToken::BgPanel => &self.bg_panel,
            ThemeToken::BgElevated => &self.bg_elevated,
            ThemeToken::BgHover => &self.bg_hover,
            ThemeToken::BgActive => &self.bg_active,
            ThemeToken::BgInput => &self.bg_input,
            ThemeToken::BgOverlay => &self.bg_overlay,
            ThemeToken::Border => &self.border,
            ThemeToken::BorderStrong => &self.border_strong,
            ThemeToken::BorderFocus => &self.border_focus,
            ThemeToken::Fg => &self.fg,
            ThemeToken::FgMuted => &self.fg_muted,
            ThemeToken::FgSubtle => &self.fg_subtle,
            ThemeToken::FgInverted => &self.fg_inverted,
            ThemeToken::Accent => &self.accent,
            ThemeToken::AccentHover => &self.accent_hover,
            ThemeToken::AccentFg => &self.accent_fg,
            ThemeToken::AccentSoft => &self.accent_soft,
            ThemeToken::BubbleInBg => &self.bubble_in_bg,
            ThemeToken::BubbleInFg => &self.bubble_in_fg,
            ThemeToken::BubbleOutBg => &self.bubble_out_bg,
            ThemeToken::BubbleOutFg => &self.bubble_out_fg,
            ThemeToken::QuoteBar => &self.quote_bar,
            ThemeToken::Success => &self.success,
            ThemeToken::Warning => &self.warning,
            ThemeToken::Danger => &self.danger,
            ThemeToken::Info => &self.info,
            ThemeToken::ShadowSm => &self.shadow_sm,
            ThemeToken::ShadowMd => &self.shadow_md,
            ThemeToken::ShadowLg => &self.shadow_lg,
        }
    }

    /// Overwrite one token.
    pub fn set(&mut self, token: ThemeToken, value: &str) {
        let value = value.to_string();
        match token {
            ThemeToken::BgApp => self.bg_app = value,
            ThemeToken::BgPanel => self.bg_panel = value,
            ThemeToken::BgElevated => self.bg_elevated = value,
            ThemeToken::BgHover => self.bg_hover = value,
            ThemeToken::BgActive => self.bg_active = value,
            ThemeToken::BgInput => self.bg_input = value,
            ThemeToken::BgOverlay => self.bg_overlay = value,
            ThemeToken::Border => self.border = value,
            ThemeToken::BorderStrong => self.border_strong = value,
            ThemeToken::BorderFocus => self.border_focus = value,
            ThemeToken::Fg => self.fg = value,
            ThemeToken::FgMuted => self.fg_muted = value,
            ThemeToken::FgSubtle => self.fg_subtle = value,
            ThemeToken::FgInverted => self.fg_inverted = value,
            ThemeToken::Accent => self.accent = value,
            ThemeToken::AccentHover => self.accent_hover = value,
            ThemeToken::AccentFg => self.accent_fg = value,
            ThemeToken::AccentSoft => self.accent_soft = value,
            ThemeToken::BubbleInBg => self.bubble_in_bg = value,
            ThemeToken::BubbleInFg => self.bubble_in_fg = value,
            ThemeToken::BubbleOutBg => self.bubble_out_bg = value,
            ThemeToken::BubbleOutFg => self.bubble_out_fg = value,
            ThemeToken::QuoteBar => self.quote_bar = value,
            ThemeToken::Success => self.success = value,
            ThemeToken::Warning => self.warning = value,
            ThemeToken::Danger => self.danger = value,
            ThemeToken::Info => self.info = value,
            ThemeToken::ShadowSm => self.shadow_sm = value,
            ThemeToken::ShadowMd => self.shadow_md = value,
            ThemeToken::ShadowLg => self.shadow_lg = value,
        }
    }
}

/// The themes the app ships with.
static BUILTIN_THEMES: LazyLock<Vec<Theme>> = LazyLock::new(|| {
    vec![
        Theme {
            id: "throwback-graphite".to_string(),
            name: "Throwback Graphite".to_string(),
            appearance: ThemeAppearance::Dark,
            visual_style: ThemeVisualStyle::Skeuomorphic,
            builtin: Some(true),
            tokens: throwback_graphite(),
        },
        Theme {
            id: "classic-messages".to_string(),
            name: "Classic Messages".to_string(),
            appearance: ThemeAppearance::Light,
            visual_style: ThemeVisualStyle::Skeuomorphic,
            builtin: Some(true),
            tokens: classic_messages(),
        },
        Theme {
            id: "vercel-dark".to_string(),
            name: "Vercel Dark".to_string(),
            appearance: ThemeAppearance::Dark,
            visual_style: ThemeVisualStyle::Modern,
            builtin: Some(true),
            tokens: vercel_dark(),
        },
        Theme {
            id: "vercel-light".to_string(),
            name: "Vercel Light".to_string(),
            appearance: ThemeAppearance::Light,
            visual_style: ThemeVisualStyle::Modern,
            builtin: Some(true),
            tokens: vercel_light(),
        },
        Theme {
            id: "vercel-mono".to_string(),
            name: "Vercel Mono".to_string(),
            appearance: ThemeAppearance::Dark,
            visual_style: ThemeVisualStyle::Modern,
            builtin: Some(true),
            tokens: with_overrides(
                &vercel_dark(),
                &[
                    (ThemeToken::Accent, "#ededed"),
                    (ThemeToken::AccentHover, "#ffffff"),
                    (ThemeToken::AccentFg, "#0a0a0a"),
                    (ThemeToken::AccentSoft, "rgba(237, 237, 237, 0.12)"),
                    (ThemeToken::BorderFocus, "#ededed"),
                    (ThemeToken::BubbleOutBg, "#ededed"),
                    (ThemeToken::BubbleOutFg, "#0a0a0a"),
                    (ThemeToken::QuoteBar, "#ededed"),
                ],
            ),
        },
        Theme {
            id: "midnight".to_string(),
            name: "Midnight".to_string(),
            appearance: ThemeAppearance::Dark,
            visual_style: ThemeVisualStyle::Modern,
            builtin: Some(true),
            tokens: with_overrides(
                &vercel_dark(),
                &[
                    (ThemeToken::BgApp, "#0b0f17"),
                    (ThemeToken::BgPanel, "#101623"),
                    (ThemeToken::BgElevated, "#151d2c"),
                    (ThemeToken::BgHover, "#1b2436"),
                    (ThemeToken::BgActive, "#232f45"),
                    (ThemeToken::BgInput, "#101623"),
                    (ThemeToken::Border, "#1f293b"),
                    (ThemeToken::BorderStrong, "#31405b"),
                    (ThemeToken::Fg, "#e6ebf4"),
                    (ThemeToken::FgMuted, "#9aa7bd"),
                    (ThemeToken::FgSubtle, "#6d7c94"),
                    (ThemeToken::Accent, "#7c8cff"),
                    (ThemeToken::AccentHover, "#96a3ff"),
                    (ThemeToken::AccentSoft, "rgba(124, 140, 255, 0.16)"),
                    (ThemeToken::BorderFocus, "#7c8cff"),
                    (ThemeToken::BubbleInBg, "#182236"),
                    (ThemeToken::BubbleOutBg, "#3b46a8"),
                    (ThemeToken::QuoteBar, "#7c8cff"),
                ],
            ),
        },
        Theme {
            id: "emerald".to_string(),
            name: "Emerald".to_string(),
            appearance: ThemeAppearance::Dark,
            visual_style: ThemeVisualStyle::Modern,
            builtin: Some(true),
            tokens: with_overrides(
                &vercel_dark(),
                &[
                    (ThemeToken::Accent, "#00b884"),
                    (ThemeToken::AccentHover, "#22d6a0"),
                    (ThemeToken::AccentSoft, "rgba(0, 184, 132, 0.16)"),
                    (ThemeToken::BorderFocus, "#00b884"),
                    (ThemeToken::BubbleOutBg, "#00684f"),
                    (ThemeToken::QuoteBar, "#00b884"),
                ],
            ),
        },
        // Los dos temas de WhatsApp: la paleta se midio sobre las capturas
        // de la referencia (`docs/referencia/01-whatsapp-chats.png`).
        Theme {
            id: "whatsapp-dark".to_string(),
            name: "WhatsApp Dark".to_string(),
            appearance: ThemeAppearance::Dark,
            visual_style: ThemeVisualStyle::Modern,
            builtin: Some(true),
            tokens: whatsapp_dark(),
        },
        Theme {
            id: "whatsapp-light".to_string(),
            name: "WhatsApp Light".to_string(),
            appearance: ThemeAppearance::Light,
            visual_style: ThemeVisualStyle::Modern,
            builtin: Some(true),
            tokens: whatsapp_light(),
        },
    ]
});

/// The themes the app ships with, in the order of the original.
pub fn builtin_themes() -> &'static [Theme] {
    &BUILTIN_THEMES
}

/// The theme the app starts with.
pub const DEFAULT_THEME_ID: &str = "vercel-dark";

/// The theme the app starts with.
///
/// port: `BUILTIN_THEMES.find((theme) => theme.id === DEFAULT_THEME_ID) ??
/// BUILTIN_THEMES[0]`. The builtin catalog is a constant, so the search always
/// finds "vercel-dark"; the fallback is the first theme of the catalog.
pub fn default_theme() -> Theme {
    let themes = builtin_themes();
    let found = themes
        .iter()
        .find(|theme| theme.id == DEFAULT_THEME_ID)
        .or_else(|| themes.first());
    match found {
        Some(theme) => theme.clone(),
        // port: the catalog is never empty, so this branch is unreachable.
        None => Theme {
            id: DEFAULT_THEME_ID.to_string(),
            name: "Vercel Dark".to_string(),
            appearance: ThemeAppearance::Dark,
            visual_style: ThemeVisualStyle::Modern,
            builtin: Some(true),
            tokens: vercel_dark(),
        },
    }
}

/// What a theme writes on the document root.
///
/// port: `applyTheme` sets the thirty custom properties, the `data-appearance`
/// and the `data-theme-style` attributes and the `color-scheme` style of
/// `document.documentElement`. The DOM is an effect of the UI crate, so this
/// module only decides the values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeRootDeclaration {
    /// The CSS custom properties, in [`THEME_TOKENS`] order.
    pub properties: Vec<(String, String)>,
    /// The value of the `data-appearance` attribute of the root.
    pub appearance: &'static str,
    /// The value of the `data-theme-style` attribute of the root.
    pub theme_style: &'static str,
    /// The value of the `color-scheme` style of the root.
    pub color_scheme: &'static str,
}

/// The values `applyTheme` writes on the document root.
pub fn theme_root_declaration(theme: &Theme) -> ThemeRootDeclaration {
    let properties = THEME_TOKENS
        .iter()
        .map(|token| {
            (
                format!("--{}", token.as_str()),
                theme.tokens.get(*token).to_string(),
            )
        })
        .collect();
    ThemeRootDeclaration {
        properties,
        appearance: theme.appearance.as_str(),
        theme_style: theme.visual_style.as_str(),
        color_scheme: theme.appearance.as_str(),
    }
}

/// Accept a user-authored theme without trusting it.
///
/// Unknown keys are dropped, missing keys fall back to the base theme and
/// every value must look like a plain CSS token, so a theme file cannot
/// inject `url(...)` or close a rule.
pub fn normalize_theme(value: &serde_json::Value) -> Option<Theme> {
    normalize_theme_with_base(value, &default_theme())
}

/// [`normalize_theme`] against an explicit base theme.
///
/// port: the original takes the base as its second parameter, defaulting to
/// [`default_theme`].
pub fn normalize_theme_with_base(value: &serde_json::Value, base: &Theme) -> Option<Theme> {
    // port: `typeof value !== "object" || value === null` → `undefined`.
    let candidate = value.as_object()?;

    // port: the original accepts an array here (`typeof [] === "object"`), but
    // an array never carries a token, so every token falls back to the base.
    let raw_tokens = candidate
        .get("tokens")
        .and_then(serde_json::Value::as_object);

    let mut tokens = base.tokens.clone();
    for token in THEME_TOKENS {
        let raw = raw_tokens
            .and_then(|raw_tokens| raw_tokens.get(token.as_str()))
            .and_then(serde_json::Value::as_str);
        let value = match raw {
            Some(raw) if is_safe_token_value(raw) => js_trim(raw).to_string(),
            _ => base.tokens.get(token).to_string(),
        };
        tokens.set(token, &value);
    }

    let id = match candidate.get("id").and_then(serde_json::Value::as_str) {
        Some(id) if !js_trim(id).is_empty() => js_trim(id).to_string(),
        _ => format!("custom-{}", hash(&tokens_json(&tokens))),
    };
    let name = match candidate.get("name").and_then(serde_json::Value::as_str) {
        Some(name) if !js_trim(name).is_empty() => js_slice_utf16(js_trim(name), 60),
        _ => "Custom theme".to_string(),
    };

    Some(Theme {
        id: js_slice_utf16(&id, 80),
        name,
        appearance: if candidate
            .get("appearance")
            .and_then(serde_json::Value::as_str)
            == Some("light")
        {
            ThemeAppearance::Light
        } else {
            ThemeAppearance::Dark
        },
        visual_style: if candidate
            .get("visualStyle")
            .and_then(serde_json::Value::as_str)
            == Some("skeuomorphic")
        {
            ThemeVisualStyle::Skeuomorphic
        } else {
            ThemeVisualStyle::Modern
        },
        builtin: None,
        tokens,
    })
}

/// Copy a theme under a new name.
///
/// port: the copy is a custom theme: it has no `builtin` flag and a fresh id
/// derived from the name and the tokens.
pub fn clone_theme(theme: &Theme, name: &str) -> Theme {
    let tokens = theme.tokens.clone();
    let source = format!("{name}{}{}", tokens_json(&tokens), js_utf16_len(name));
    Theme {
        id: format!("custom-{}", hash(&source)),
        name: js_slice_utf16(name, 60),
        appearance: theme.appearance,
        visual_style: theme.visual_style,
        builtin: None,
        tokens,
    }
}

/// The JSON a theme file holds.
pub fn export_theme(theme: &Theme) -> String {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct ThemeExport<'a> {
        id: &'a str,
        name: &'a str,
        appearance: ThemeAppearance,
        visual_style: ThemeVisualStyle,
        tokens: &'a ThemeTokens,
    }

    let export = ThemeExport {
        id: &theme.id,
        name: &theme.name,
        appearance: theme.appearance,
        visual_style: theme.visual_style,
        tokens: &theme.tokens,
    };
    // port: `JSON.stringify(theme, null, 2)` never throws for a theme.
    serde_json::to_string_pretty(&export).unwrap_or_default()
}

/// The colors, lengths and shadow lists a theme file may carry.
///
/// port: the original rejects `(`, `;`, `{`, `}` and the quotes, and then
/// refuses `url`, `expression`, `image`, `@import` and `var (`, so a
/// downloaded theme cannot smuggle `url()` requests or extra declarations
/// through `setProperty`. The three regular expressions are implemented by
/// hand: the crate takes no new dependency.
fn is_safe_token_value(value: &str) -> bool {
    let trimmed = js_trim(value);
    if trimmed.is_empty() || js_utf16_len(trimmed) > 200 {
        return false;
    }
    if trimmed
        .chars()
        .any(|character| matches!(character, ';' | '{' | '}' | '"' | '\'' | '\\' | '<' | '>'))
    {
        return false;
    }
    if has_forbidden_keyword(trimmed) {
        return false;
    }
    trimmed.chars().all(is_safe_token_character)
}

/// port: `/url|expression|image|@import|var\s*\(/i`.
fn has_forbidden_keyword(value: &str) -> bool {
    let lowered = value.to_lowercase();
    if lowered.contains("url")
        || lowered.contains("expression")
        || lowered.contains("image")
        || lowered.contains("@import")
    {
        return true;
    }
    for (index, _) in lowered.match_indices("var") {
        let mut cursor = index + "var".len();
        let bytes = lowered.as_bytes();
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if lowered[cursor..].starts_with('(') {
            return true;
        }
    }
    false
}

/// port: `/^[#a-z0-9\s.,%()-]+$/i`.
fn is_safe_token_character(character: char) -> bool {
    matches!(character, '#' | '.' | ',' | '%' | '(' | ')' | '-')
        || character.is_ascii_alphanumeric()
        || js_is_whitespace(character)
}

/// port: the FNV-1a of the original, over the UTF-16 code units of the
/// string, rendered in base 36.
fn hash(value: &str) -> String {
    // port: the original keeps the accumulator in the 32 bit integer of the
    // bitwise operators, so the seed wraps around.
    let mut result: i32 = 2_166_136_261u32 as i32;
    for unit in value.encode_utf16() {
        result ^= i32::from(unit);
        result = result.wrapping_mul(16_777_619);
    }
    to_base36(result as u32)
}

/// port: `Number.prototype.toString(36)`.
fn to_base36(mut value: u32) -> String {
    const DIGITS: [char; 36] = [
        '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h',
        'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z',
    ];
    if value == 0 {
        return "0".to_string();
    }
    let mut digits = Vec::new();
    while value > 0 {
        digits.push(DIGITS[(value % 36) as usize]);
        value /= 36;
    }
    digits.reverse();
    digits.into_iter().collect()
}

/// port: `JSON.stringify(theme.tokens)`: the compact JSON of the tokens, in
/// the order of the declaration.
fn tokens_json(tokens: &ThemeTokens) -> String {
    serde_json::to_string(tokens).unwrap_or_default()
}

/// port: the strings of JS are UTF-16, so `String.prototype.length` counts
/// UTF-16 code units.
fn js_utf16_len(value: &str) -> usize {
    value.encode_utf16().count()
}

/// port: `String.prototype.slice(0, end)` counted in UTF-16 code units. The
/// port never cuts a surrogate pair in half: the character that would be
/// split is left out.
fn js_slice_utf16(value: &str, end: usize) -> String {
    let mut sliced = String::new();
    let mut units = 0usize;
    for character in value.chars() {
        let width = character.len_utf16();
        if units + width > end {
            break;
        }
        sliced.push(character);
        units += width;
    }
    sliced
}

/// port: the whitespace of JavaScript: the `White_Space` property plus the
/// byte order mark, which Rust's `char::is_whitespace` does not count.
fn js_is_whitespace(character: char) -> bool {
    matches!(
        character,
        '\u{9}'..='\u{d}'
            | '\u{20}'
            | '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
            | '\u{feff}'
    )
}

/// port: `String.prototype.trim`.
fn js_trim(value: &str) -> &str {
    value.trim_matches(js_is_whitespace)
}

/// Vercel's monochrome look: the outgoing bubble is an inverted primary.
fn vercel_dark() -> ThemeTokens {
    ThemeTokens {
        bg_app: "#000000".to_string(),
        bg_panel: "#0a0a0a".to_string(),
        bg_elevated: "#111111".to_string(),
        bg_hover: "#1a1a1a".to_string(),
        bg_active: "#242424".to_string(),
        bg_input: "#0a0a0a".to_string(),
        bg_overlay: "rgba(0, 0, 0, 0.72)".to_string(),
        border: "#2e2e2e".to_string(),
        border_strong: "#454545".to_string(),
        border_focus: "#0072f5".to_string(),
        fg: "#ededed".to_string(),
        fg_muted: "#a1a1a1".to_string(),
        fg_subtle: "#707070".to_string(),
        fg_inverted: "#0a0a0a".to_string(),
        accent: "#0072f5".to_string(),
        accent_hover: "#3291ff".to_string(),
        accent_fg: "#ffffff".to_string(),
        accent_soft: "rgba(0, 114, 245, 0.16)".to_string(),
        bubble_in_bg: "#161616".to_string(),
        bubble_in_fg: "#ededed".to_string(),
        bubble_out_bg: "#0059c1".to_string(),
        bubble_out_fg: "#ffffff".to_string(),
        quote_bar: "#0072f5".to_string(),
        success: "#45d483".to_string(),
        warning: "#f7b955".to_string(),
        danger: "#ff6166".to_string(),
        info: "#52a8ff".to_string(),
        shadow_sm: "0 1px 2px rgba(0, 0, 0, 0.5)".to_string(),
        shadow_md: "0 4px 16px rgba(0, 0, 0, 0.55)".to_string(),
        shadow_lg: "0 16px 48px rgba(0, 0, 0, 0.65)".to_string(),
    }
}

/// The light counterpart of [`vercel_dark`].
fn vercel_light() -> ThemeTokens {
    ThemeTokens {
        bg_app: "#ffffff".to_string(),
        bg_panel: "#fafafa".to_string(),
        bg_elevated: "#ffffff".to_string(),
        bg_hover: "#f2f2f2".to_string(),
        bg_active: "#ebebeb".to_string(),
        bg_input: "#ffffff".to_string(),
        bg_overlay: "rgba(255, 255, 255, 0.75)".to_string(),
        border: "#ebebeb".to_string(),
        border_strong: "#d4d4d4".to_string(),
        border_focus: "#0072f5".to_string(),
        fg: "#171717".to_string(),
        fg_muted: "#666666".to_string(),
        fg_subtle: "#8f8f8f".to_string(),
        fg_inverted: "#ffffff".to_string(),
        accent: "#0072f5".to_string(),
        accent_hover: "#0761d1".to_string(),
        accent_fg: "#ffffff".to_string(),
        accent_soft: "rgba(0, 114, 245, 0.1)".to_string(),
        bubble_in_bg: "#f2f2f2".to_string(),
        bubble_in_fg: "#171717".to_string(),
        bubble_out_bg: "#0072f5".to_string(),
        bubble_out_fg: "#ffffff".to_string(),
        quote_bar: "#0072f5".to_string(),
        success: "#0f7b3f".to_string(),
        warning: "#ab5700".to_string(),
        danger: "#c9282d".to_string(),
        info: "#0068d6".to_string(),
        shadow_sm: "0 1px 2px rgba(0, 0, 0, 0.06)".to_string(),
        shadow_md: "0 4px 16px rgba(0, 0, 0, 0.08)".to_string(),
        shadow_lg: "0 16px 48px rgba(0, 0, 0, 0.12)".to_string(),
    }
}

/// The skeuomorphic dark theme of the original.
fn throwback_graphite() -> ThemeTokens {
    ThemeTokens {
        bg_app: "#292929".to_string(),
        bg_panel: "#3b3b3b".to_string(),
        bg_elevated: "#474747".to_string(),
        bg_hover: "#555555".to_string(),
        bg_active: "#626262".to_string(),
        bg_input: "#151515".to_string(),
        bg_overlay: "rgba(8, 8, 8, 0.78)".to_string(),
        border: "#202020".to_string(),
        border_strong: "#777777".to_string(),
        border_focus: "#a2d422".to_string(),
        fg: "#f2f2f2".to_string(),
        fg_muted: "#c7c7c7".to_string(),
        fg_subtle: "#969696".to_string(),
        fg_inverted: "#151515".to_string(),
        accent: "#a2d422".to_string(),
        accent_hover: "#b6e53c".to_string(),
        accent_fg: "#172000".to_string(),
        accent_soft: "rgba(162, 212, 34, 0.20)".to_string(),
        bubble_in_bg: "#505050".to_string(),
        bubble_in_fg: "#f4f4f4".to_string(),
        bubble_out_bg: "#7da914".to_string(),
        bubble_out_fg: "#101600".to_string(),
        quote_bar: "#a2d422".to_string(),
        success: "#a2d422".to_string(),
        warning: "#efb43c".to_string(),
        danger: "#e85d54".to_string(),
        info: "#82b9e6".to_string(),
        shadow_sm: "0 1px 2px rgba(0, 0, 0, 0.62)".to_string(),
        shadow_md: "0 5px 16px rgba(0, 0, 0, 0.64)".to_string(),
        shadow_lg: "0 18px 50px rgba(0, 0, 0, 0.72)".to_string(),
    }
}

/// The skeuomorphic light theme of the original.
fn classic_messages() -> ThemeTokens {
    ThemeTokens {
        bg_app: "#ccd3df".to_string(),
        bg_panel: "#e8edf4".to_string(),
        bg_elevated: "#f6f7f9".to_string(),
        bg_hover: "#d8e0ec".to_string(),
        bg_active: "#b8c9df".to_string(),
        bg_input: "#ffffff".to_string(),
        bg_overlay: "rgba(45, 54, 68, 0.58)".to_string(),
        border: "#9aa6b7".to_string(),
        border_strong: "#687789".to_string(),
        border_focus: "#315e96".to_string(),
        fg: "#17202b".to_string(),
        fg_muted: "#4d5867".to_string(),
        fg_subtle: "#707b8a".to_string(),
        fg_inverted: "#ffffff".to_string(),
        accent: "#2d67a3".to_string(),
        accent_hover: "#3e7ebf".to_string(),
        accent_fg: "#ffffff".to_string(),
        accent_soft: "rgba(55, 105, 164, 0.20)".to_string(),
        bubble_in_bg: "#f4f4f1".to_string(),
        bubble_in_fg: "#202020".to_string(),
        bubble_out_bg: "#7fb1e6".to_string(),
        bubble_out_fg: "#10243b".to_string(),
        quote_bar: "#356da8".to_string(),
        success: "#4f8a22".to_string(),
        warning: "#a56600".to_string(),
        danger: "#b73131".to_string(),
        info: "#2d67a3".to_string(),
        shadow_sm: "0 1px 2px rgba(35, 48, 65, 0.28)".to_string(),
        shadow_md: "0 5px 16px rgba(35, 48, 65, 0.30)".to_string(),
        shadow_lg: "0 18px 48px rgba(35, 48, 65, 0.38)".to_string(),
    }
}

/// The dark palette of WhatsApp.
///
/// The values come from the reference screenshots of WhatsApp Desktop
/// (`docs/referencia/01-whatsapp-chats.png`): the column of chats is
/// `#161717`, the column of icons is `#161717` a shade lighter, the search
/// field is `#2e2f2f` and the green of the new chat button is `#21c063`.
fn whatsapp_dark() -> ThemeTokens {
    ThemeTokens {
        bg_app: "#161717".to_string(),
        bg_panel: "#1d1f1f".to_string(),
        bg_elevated: "#2e2f2f".to_string(),
        bg_hover: "#232425".to_string(),
        bg_active: "#2a2b2c".to_string(),
        bg_input: "#2e2f2f".to_string(),
        bg_overlay: "rgba(0, 0, 0, 0.72)".to_string(),
        border: "#2e2f2f".to_string(),
        border_strong: "#3d3e3f".to_string(),
        border_focus: "#21c063".to_string(),
        fg: "#fafafa".to_string(),
        fg_muted: "#a2a2a2".to_string(),
        fg_subtle: "#7a7a7a".to_string(),
        fg_inverted: "#0b141a".to_string(),
        accent: "#21c063".to_string(),
        accent_hover: "#2ed072".to_string(),
        accent_fg: "#0b141a".to_string(),
        accent_soft: "rgba(33, 192, 99, 0.22)".to_string(),
        bubble_in_bg: "#242626".to_string(),
        bubble_in_fg: "#fafafa".to_string(),
        bubble_out_bg: "#144d37".to_string(),
        bubble_out_fg: "#e9edef".to_string(),
        quote_bar: "#21c063".to_string(),
        success: "#21c063".to_string(),
        warning: "#f7b955".to_string(),
        danger: "#ff6166".to_string(),
        info: "#53bdeb".to_string(),
        shadow_sm: "0 1px 2px rgba(0, 0, 0, 0.5)".to_string(),
        shadow_md: "0 4px 16px rgba(0, 0, 0, 0.55)".to_string(),
        shadow_lg: "0 16px 48px rgba(0, 0, 0, 0.65)".to_string(),
    }
}

/// The light palette of WhatsApp.
fn whatsapp_light() -> ThemeTokens {
    ThemeTokens {
        bg_app: "#ffffff".to_string(),
        bg_panel: "#f0f2f5".to_string(),
        bg_elevated: "#ffffff".to_string(),
        bg_hover: "#f0f2f5".to_string(),
        bg_active: "#e9edef".to_string(),
        bg_input: "#f0f2f5".to_string(),
        bg_overlay: "rgba(11, 20, 26, 0.4)".to_string(),
        border: "#e9edef".to_string(),
        border_strong: "#d1d7db".to_string(),
        border_focus: "#00a884".to_string(),
        fg: "#111b21".to_string(),
        fg_muted: "#667781".to_string(),
        fg_subtle: "#8696a0".to_string(),
        fg_inverted: "#ffffff".to_string(),
        accent: "#00a884".to_string(),
        accent_hover: "#06cf9c".to_string(),
        accent_fg: "#ffffff".to_string(),
        accent_soft: "rgba(0, 168, 132, 0.16)".to_string(),
        bubble_in_bg: "#ffffff".to_string(),
        bubble_in_fg: "#111b21".to_string(),
        bubble_out_bg: "#d9fdd3".to_string(),
        bubble_out_fg: "#111b21".to_string(),
        quote_bar: "#00a884".to_string(),
        success: "#00a884".to_string(),
        warning: "#b8860b".to_string(),
        danger: "#d32f2f".to_string(),
        info: "#027eb5".to_string(),
        shadow_sm: "0 1px 2px rgba(11, 20, 26, 0.13)".to_string(),
        shadow_md: "0 2px 10px rgba(11, 20, 26, 0.13)".to_string(),
        shadow_lg: "0 12px 40px rgba(11, 20, 26, 0.16)".to_string(),
    }
}

/// port: the object spread `{ ...base, ...overrides }` of the original.
fn with_overrides(base: &ThemeTokens, overrides: &[(ThemeToken, &str)]) -> ThemeTokens {
    let mut tokens = base.clone();
    for (token, value) in overrides {
        tokens.set(*token, value);
    }
    tokens
}
