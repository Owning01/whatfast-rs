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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
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

impl<'de> Deserialize<'de> for ChatBackground {
    /// An unknown value comes from a newer release: this one keeps the
    /// default of the field instead of discarding the whole file.
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(match value.as_str() {
            Some("color") => Self::Color,
            Some("none") => Self::None,
            _ => Self::Pattern,
        })
    }
}

/// How much air the interface leaves between rows and messages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Density {
    /// The default spacing: the one of the reference screenshots.
    #[default]
    Comfortable,
    /// Tighter spacing, for more messages on screen.
    Compact,
}

impl<'de> Deserialize<'de> for Density {
    /// An unknown value comes from a newer release: this one keeps the
    /// default of the field instead of discarding the whole file.
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(match value.as_str() {
            Some("compact") => Self::Compact,
            _ => Self::Comfortable,
        })
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
    /// Whether the app plays a sound when a message arrives or when it sends
    /// one.
    pub sounds_enabled: bool,
    /// The sound of a message that arrives: `builtin:<id>`, `file:<path>` or
    /// `none`. Anything else falls back to the built-in one.
    pub sound_received: String,
    /// The sound of a message that this window sends. Same encoding as
    /// `sound_received`.
    pub sound_sent: String,
    /// The sound of a toast that appears. Same encoding as `sound_received`;
    /// `none` keeps the toast silent.
    pub sound_notification: String,
    /// The sound of a message that arrives in one conversation, when the
    /// user asked for one. A chat without an entry plays `sound_received`.
    #[serde(default)]
    pub chat_sounds: Vec<ChatSound>,
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
    /// Whether the app asks the releases for a newer version when it starts.
    pub check_updates: bool,
    /// Suppresses desktop notification popups and sounds.
    #[serde(default)]
    pub do_not_disturb: bool,
    /// User-defined chat folders (name + chat IDs).
    #[serde(default)]
    pub chat_folders: Vec<ChatFolder>,
    /// Quick `/templates` snippets for the composer.
    #[serde(default)]
    pub quick_templates: Vec<QuickTemplate>,
    /// Renders rich Markdown/code blocks in message bubbles when enabled.
    #[serde(default = "default_true")]
    pub rich_documents: bool,
    /// Starts Whatfast automatically on OS login.
    #[serde(default)]
    pub autostart: bool,
    /// Automatically locks the app after N minutes of inactivity (0 = disabled).
    #[serde(default)]
    pub auto_lock_minutes: u32,
    /// Blurs chat content when the window loses focus (if App Lock is configured).
    #[serde(default)]
    pub privacy_blur_on_unfocus: bool,
    /// Maximum media size in MiB for automatic downloads (0 = unlimited).
    #[serde(default = "default_auto_download_max_mb")]
    pub auto_download_max_mb: u32,
    /// Skips automatic media downloads on metered connections.
    #[serde(default = "default_true")]
    pub skip_auto_download_metered: bool,
    /// Optional HTTP or SOCKS5 proxy URL for WhatsApp network connections.
    #[serde(default)]
    pub proxy_url: String,
    /// Senders whose statuses are muted from the active strip.
    #[serde(default)]
    pub muted_status_senders: Vec<String>,
    /// Accessibility: increases contrast for borders, muted text, and focus rings.
    #[serde(default)]
    pub high_contrast: bool,
    /// Accessibility: reduces UI animations and transitions.
    #[serde(default)]
    pub reduced_motion: bool,
}

fn default_true() -> bool {
    true
}

fn default_auto_download_max_mb() -> u32 {
    25
}

/// The sound that the user picked for one conversation.
///
/// The sound uses the encoding of the profile: `builtin:<id>`, `file:<path>`
/// or `none` for silence. A conversation without an entry follows the sound
/// of the profile.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatSound {
    /// The conversation the sound belongs to.
    #[serde(default)]
    pub chat_id: String,
    /// The sound, with the encoding of `sound_received`.
    #[serde(default)]
    pub sound: String,
}

/// A custom folder of conversations shown in the chat filter bar.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatFolder {
    /// Display name of the folder.
    pub name: String,
    /// Chat JIDs included in this folder.
    #[serde(default)]
    pub chat_ids: Vec<String>,
}

/// A reusable quick-reply template triggered with `/templates` in the composer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickTemplate {
    /// Short command or label (e.g. "saludo").
    pub name: String,
    /// Expanded message body.
    pub body: String,
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
            sounds_enabled: true,
            // The pop of a new message and a shorter ping for the one that
            // leaves: the same pair the phone plays.
            sound_received: "builtin:pop".to_string(),
            sound_sent: "builtin:ping".to_string(),
            // A softer bell for the popup, different from the one of the chat
            // message: the two sounds tell the user what just happened.
            sound_notification: "builtin:bell".to_string(),
            chat_sounds: Vec::new(),
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
            check_updates: true,
            do_not_disturb: false,
            chat_folders: Vec::new(),
            quick_templates: Vec::new(),
            rich_documents: true,
            autostart: false,
            auto_lock_minutes: 0,
            privacy_blur_on_unfocus: false,
            auto_download_max_mb: default_auto_download_max_mb(),
            skip_auto_download_metered: true,
            proxy_url: String::new(),
            muted_status_senders: Vec::new(),
            high_contrast: false,
            reduced_motion: false,
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

    /// The sound that one conversation has, when the user picked one.
    ///
    /// `None` means the conversation follows `sound_received`.
    pub fn chat_sound(&self, chat_id: &str) -> Option<&str> {
        self.chat_sounds
            .iter()
            .find(|entry| entry.chat_id == chat_id)
            .map(|entry| entry.sound.as_str())
    }

    /// Sets or clears the sound of one conversation.
    ///
    /// An empty value removes the entry, so the chat follows the profile
    /// again; a value replaces the previous one.
    pub fn set_chat_sound(&mut self, chat_id: &str, sound: Option<String>) {
        let sound = sound.map(|value| value.trim().to_string());
        self.chat_sounds.retain(|entry| entry.chat_id != chat_id);
        if let Some(sound) = sound
            && !sound.is_empty()
        {
            self.chat_sounds.push(ChatSound {
                chat_id: chat_id.to_string(),
                sound,
            });
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

/// Serializes the writers of the settings, so a reader that merges one field
/// does not lose the change that the interface wrote at the same time.
static WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Takes the lock of the writers, surviving a panic of another one.
fn write_lock() -> std::sync::MutexGuard<'static, ()> {
    WRITE_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
}

/// Stores the settings, replacing the file atomically.
pub fn save(settings: &Settings) -> std::io::Result<()> {
    let _guard = write_lock();
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
///
/// The read and the write share the lock of the other writers: without it,
/// the interface saving its whole file at the same time left one of the two
/// changes behind.
pub fn save_ui_scale(scale: f32) -> std::io::Result<()> {
    let _guard = write_lock();
    let mut settings = load();
    settings.ui_scale = scale;
    settings.normalize();
    save_to(&settings_path(), &settings)
}

