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
    fn the_density_spells_itself_the_way_the_file_does() {
        assert_eq!(Density::Comfortable.as_str(), "comfortable");
        assert_eq!(Density::Compact.as_str(), "compact");
        assert_eq!(Density::from_value("compact"), Density::Compact);
        assert_eq!(Density::from_value("anything"), Density::Comfortable);
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
