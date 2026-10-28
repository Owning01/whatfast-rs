//! The settings of the profile: one JSON file next to the data of the app.
//!
//! port: the origin kept its preferences in the local storage of the webview
//! (`tauri/src/state/preferences.ts`, `whatfast-preferences`). The native app
//! has no local storage, so the same fields live in one JSON file, written
//! atomically. The old file of the single scale (`settings`, one float) is
//! migrated the first time the new one is read.
//!
//! Every field has a default and the unknown fields are ignored, so a file
//! written by a newer version does not break an older one.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The smallest interface scale.
pub const MIN_UI_SCALE: f32 = 1.0;

/// The biggest interface scale.
pub const MAX_UI_SCALE: f32 = 1.5;

/// The scale of a profile that never changed it.
pub const DEFAULT_UI_SCALE: f32 = 1.0;

/// The smallest radius of a bubble, in pixels.
pub const MIN_BUBBLE_RADIUS: f32 = 0.0;

/// The biggest radius of a bubble, in pixels.
pub const MAX_BUBBLE_RADIUS: f32 = 20.0;

/// The narrowest bubble, as a part of the width of the thread.
pub const MIN_BUBBLE_WIDTH: f32 = 0.45;

/// The widest bubble, as a part of the width of the thread.
pub const MAX_BUBBLE_WIDTH: f32 = 0.85;

/// The smallest text of the messages, as a part of the one of the
/// interface.
pub const MIN_MESSAGE_FONT_SCALE: f32 = 0.85;

/// The biggest text of the messages, as a part of the one of the interface.
pub const MAX_MESSAGE_FONT_SCALE: f32 = 1.3;

/// Where the background of a conversation comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatBackground {
    /// The pattern of the reference: the default of the application.
    #[default]
    Pattern,
    /// A plain color of the profile.
    Color,
    /// Nothing: the background of the theme shows through.
    None,
}

impl ChatBackground {
    /// The background as the settings file spells it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pattern => "pattern",
            Self::Color => "color",
            Self::None => "none",
        }
    }

    /// Reads a background, falling back to the pattern.
    pub fn from_value(value: &str) -> Self {
        match value {
            "color" => Self::Color,
            "none" => Self::None,
            _ => Self::Pattern,
        }
    }
}

/// How much air the interface leaves between rows and messages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Density {
    /// The default spacing: the one of the reference screenshots.
    #[default]
    Comfortable,
    /// Tighter spacing, for more messages on screen.
    Compact,
}

impl Density {
    /// The density as the settings file spells it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Comfortable => "comfortable",
            Self::Compact => "compact",
        }
    }

    /// Reads a density, falling back to the comfortable one.
    pub fn from_value(value: &str) -> Self {
        match value {
            "compact" => Self::Compact,
            _ => Self::Comfortable,
        }
    }
}

/// The preferences of the profile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The scale of the interface, before the fonts and the lengths.
    pub ui_scale: f32,
    /// The code of the language of the interface: `en`, `es`, `zh`.
    pub language: String,
    /// The id of the theme: a builtin one or the id of a custom one.
    ///
    /// An empty string means "the theme of the app": `whatsapp-dark` or
    /// `whatsapp-light`, depending on the appearance.
    pub theme_id: String,
    /// The air between rows and messages.
    pub density: Density,
    /// Whether a plain Enter sends the message, instead of a newline.
    /// With this off, Ctrl+Enter sends.
    pub enter_to_send: bool,
    /// Whether a message that arrives in the background shows a toast of
    /// the system.
    pub notifications_enabled: bool,
    /// Whether the toast includes the text of the message.
    pub notification_previews: bool,
    /// The directory of "Save to folder"; empty asks every time.
    pub download_dir: String,
    /// Whether the chat list stays beside the conversation even on narrow
    /// windows.
    pub split_view: bool,
    /// Whether the wallpaper and the smooth scrolling are off, to save
    /// battery.
    pub battery_saver: bool,
    /// The themes the user made, as the JSON of `whatfast_core::theme`.
    pub custom_themes: Vec<serde_json::Value>,
    /// The radius of the bubbles, in pixels; `-1` keeps the one of the
    /// theme.
    pub bubble_radius: f32,
    /// The widest bubble, as a part of the width of the thread; `-1` keeps
    /// the width of the reference.
    pub bubble_width: f32,
    /// The text of the messages, as a part of the text of the interface.
    pub message_font_scale: f32,
    /// Where the background of the conversation comes from.
    pub chat_background: ChatBackground,
    /// The color of the background when it is a plain one, as `#rrggbb`;
    /// empty asks the theme.
    pub chat_background_color: String,
    /// The image of the background, relative to the profile; empty when
    /// there is none.
    pub chat_wallpaper: String,
    /// How solid the image of the background is, from 0.15 to 1.
    pub chat_wallpaper_opacity: f32,
    /// The salt and the hash of the PIN of the app lock, in PBKDF2-SHA256.
    /// Both empty means the lock is off; the PIN itself never touches the
    /// disk.
    pub app_lock_salt: String,
    pub app_lock_hash: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ui_scale: DEFAULT_UI_SCALE,
            // The app starts in English, the same default as the web client.
            language: "en".to_string(),
            theme_id: String::new(),
            density: Density::Comfortable,
            enter_to_send: true,
            notifications_enabled: true,
            notification_previews: true,
            download_dir: String::new(),
            split_view: false,
            battery_saver: false,
            custom_themes: Vec::new(),
            bubble_radius: -1.0,
            bubble_width: -1.0,
            message_font_scale: 1.0,
            chat_background: ChatBackground::Pattern,
            chat_background_color: String::new(),
            chat_wallpaper: String::new(),
            chat_wallpaper_opacity: 1.0,
            app_lock_salt: String::new(),
            app_lock_hash: String::new(),
        }
    }
}

impl Settings {
    /// Clamps the values that the interface cannot paint.
    pub fn normalize(&mut self) {
        if !self.ui_scale.is_finite() {
            self.ui_scale = DEFAULT_UI_SCALE;
        }
        self.ui_scale = self.ui_scale.clamp(MIN_UI_SCALE, MAX_UI_SCALE);
        // The density comes from a closed set: the serialized name is the
        // only thing that can be wrong in a hand-edited file.
        self.language = self.language.trim().to_lowercase();
        if self.language.is_empty() {
            self.language = "en".to_string();
        }
        // A radius of `-1` is the theme's own: anything else is clamped to
        // what the bubble can paint.
        if !self.bubble_radius.is_finite() {
            self.bubble_radius = -1.0;
        }
        if self.bubble_radius >= 0.0 {
            self.bubble_radius = self.bubble_radius.clamp(MIN_BUBBLE_RADIUS, MAX_BUBBLE_RADIUS);
        }
        if !self.bubble_width.is_finite() {
            self.bubble_width = -1.0;
        }
        if self.bubble_width >= 0.0 {
            self.bubble_width = self.bubble_width.clamp(MIN_BUBBLE_WIDTH, MAX_BUBBLE_WIDTH);
        }
        if !self.message_font_scale.is_finite() {
            self.message_font_scale = 1.0;
        }
        self.message_font_scale = self
            .message_font_scale
            .clamp(MIN_MESSAGE_FONT_SCALE, MAX_MESSAGE_FONT_SCALE);
        self.chat_background_color = self.chat_background_color.trim().to_lowercase();
        self.chat_wallpaper = self.chat_wallpaper.trim().to_string();
        if !self.chat_wallpaper_opacity.is_finite() {
            self.chat_wallpaper_opacity = 1.0;
        }
        self.chat_wallpaper_opacity = self.chat_wallpaper_opacity.clamp(0.15, 1.0);
    }

    /// The radius of the bubbles, or `None` for the one of the theme.
    pub fn bubble_radius(&self) -> Option<f32> {
        (self.bubble_radius >= 0.0).then_some(self.bubble_radius)
    }

    /// The width of the bubbles, or `None` for the one of the reference.
    pub fn bubble_width(&self) -> Option<f32> {
        (self.bubble_width >= 0.0).then_some(self.bubble_width)
    }
}

/// The directory of the settings of the profile.
fn settings_dir() -> PathBuf {
    crate::paths::config_dir()
}

/// The file of the settings, the JSON one.
pub fn settings_path() -> PathBuf {
    settings_dir().join("settings.json")
}

/// The file of the old release: one float with the scale.
fn legacy_settings_path() -> PathBuf {
    settings_dir().join("settings")
}

/// Reads the settings of the profile.
///
/// A missing or broken file gives the defaults, and the old file of the
/// scale is carried forward once.
pub fn load() -> Settings {
    load_from(&settings_path(), &legacy_settings_path())
}

/// Reads the settings of one directory, as the tests need.
pub fn load_from(path: &Path, legacy: &Path) -> Settings {
    if let Ok(text) = fs::read_to_string(path)
        && let Ok(mut settings) = serde_json::from_str::<Settings>(&text)
    {
        settings.normalize();
        return settings;
    }
    let mut settings = Settings::default();
    if let Ok(text) = fs::read_to_string(legacy)
        && let Ok(scale) = text.trim().parse::<f32>()
        && scale.is_finite()
    {
        settings.ui_scale = scale;
        settings.normalize();
        // The old file stays: a downgrade to the previous release still reads
        // it. The new file is written on the next save.
    }
    settings
}

/// Stores the settings, replacing the file atomically.
pub fn save(settings: &Settings) -> std::io::Result<()> {
    save_to(&settings_path(), settings)
}

/// Stores the settings in one directory, as the tests need.
pub fn save_to(path: &Path, settings: &Settings) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "settings path has no parent",
        )
    })?;
    fs::create_dir_all(parent)?;
    let existing_permissions = fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions());
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    if let Some(permissions) = existing_permissions {
        temporary.as_file().set_permissions(permissions)?;
    }
    let mut json = serde_json::to_string_pretty(settings)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    json.push('\n');
    temporary.write_all(json.as_bytes())?;
    temporary.as_file_mut().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// Reads the stored UI scale, if the profile has one.
///
/// The entry point that the window uses before it builds the app: the rest
/// of the settings need a window to be applied.
pub fn load_ui_scale() -> Option<f32> {
    Some(load().ui_scale)
}

/// Stores the UI scale, keeping the other settings.
pub fn save_ui_scale(scale: f32) -> std::io::Result<()> {
    let mut settings = load();
    settings.ui_scale = scale;
    settings.normalize();
    save(&settings)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{Density, Settings, load_from, save_to};

    #[test]
    fn the_settings_round_trip_through_atomic_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let legacy = directory.path().join("settings");
        let settings = Settings {
            ui_scale: 1.2,
            language: "es".into(),
            theme_id: "whatsapp-light".into(),
            density: Density::Compact,
            enter_to_send: false,
            notifications_enabled: false,
            notification_previews: false,
            download_dir: "C:\\descargas".into(),
            split_view: true,
            battery_saver: true,
            custom_themes: vec![serde_json::json!({ "id": "mine" })],
            bubble_radius: 14.0,
            bubble_width: 0.6,
            message_font_scale: 1.1,
            chat_background: super::ChatBackground::Color,
            chat_background_color: "#101010".into(),
            chat_wallpaper: "wallpapers/uno.png".into(),
            chat_wallpaper_opacity: 0.6,
        };
        save_to(&path, &settings).unwrap();
        assert_eq!(load_from(&path, &legacy), settings);
    }

    #[test]
    fn a_missing_file_gives_the_defaults() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let legacy = directory.path().join("settings");
        let settings = load_from(&path, &legacy);
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn the_old_scale_file_is_carried_forward() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let legacy = directory.path().join("settings");
        fs::write(&legacy, "1.3\n").unwrap();
        let settings = load_from(&path, &legacy);
        assert_eq!(settings.ui_scale, 1.3);
        assert_eq!(settings.language, "en");
        // The old file is not deleted: a downgrade still reads it.
        assert!(legacy.is_file());
    }

    #[test]
    fn a_broken_file_falls_back_to_the_defaults() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let legacy = directory.path().join("settings");
        fs::write(&path, "{ this is not json").unwrap();
        assert_eq!(load_from(&path, &legacy), Settings::default());
    }

    #[test]
    fn a_newer_file_with_unknown_fields_still_loads() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let legacy = directory.path().join("settings");
        fs::write(
            &path,
            r#"{ "ui_scale": 1.1, "language": "zh", "future_setting": 42 }"#,
        )
        .unwrap();
        let settings = load_from(&path, &legacy);
        assert_eq!(settings.ui_scale, 1.1);
        assert_eq!(settings.language, "zh");
    }

    #[test]
    fn the_values_are_clamped() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let legacy = directory.path().join("settings");
        fs::write(&path, r#"{ "ui_scale": 99.0, "language": "  ES " }"#).unwrap();
        let settings = load_from(&path, &legacy);
        assert_eq!(settings.ui_scale, super::MAX_UI_SCALE);
        assert_eq!(settings.language, "es");
    }

    #[test]
    fn the_bubbles_of_a_file_without_them_keep_the_ones_of_the_theme() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let legacy = directory.path().join("settings");
        fs::write(&path, r#"{ "ui_scale": 1.1 }"#).unwrap();
        let settings = load_from(&path, &legacy);
        assert_eq!(settings.bubble_radius(), None);
        assert_eq!(settings.bubble_width(), None);
        assert_eq!(settings.message_font_scale, 1.0);
        assert_eq!(settings.chat_background, super::ChatBackground::Pattern);
    }

    #[test]
    fn the_bubbles_are_clamped_to_what_the_window_paints() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let legacy = directory.path().join("settings");
        fs::write(
            &path,
            r##"{ "bubble_radius": 90.0, "bubble_width": 0.1, "message_font_scale": 9.0, "chat_background": "color", "chat_background_color": "#AABBCC" }"##,
        )
        .unwrap();
        let settings = load_from(&path, &legacy);
        assert_eq!(settings.bubble_radius(), Some(super::MAX_BUBBLE_RADIUS));
        assert_eq!(settings.bubble_width(), Some(super::MIN_BUBBLE_WIDTH));
        assert_eq!(settings.message_font_scale, super::MAX_MESSAGE_FONT_SCALE);
        assert_eq!(settings.chat_background, super::ChatBackground::Color);
        assert_eq!(settings.chat_background_color, "#aabbcc");
    }

    #[test]
    fn the_density_spells_itself_the_way_the_file_does() {
        assert_eq!(Density::Comfortable.as_str(), "comfortable");
        assert_eq!(Density::Compact.as_str(), "compact");
        assert_eq!(Density::from_value("compact"), Density::Compact);
        assert_eq!(Density::from_value("anything"), Density::Comfortable);
        assert_eq!(super::ChatBackground::from_value("color"), super::ChatBackground::Color);
        assert_eq!(super::ChatBackground::from_value("nada"), super::ChatBackground::Pattern);
        assert_eq!(super::ChatBackground::None.as_str(), "none");
    }

    #[test]
    fn a_float_file_is_not_read_as_the_new_format() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let legacy = directory.path().join("settings");
        fs::write(&path, "1.2\n").unwrap();
        assert_eq!(load_from(&path, &legacy), Settings::default());
    }
}
