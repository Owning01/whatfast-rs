//! The emoji of WhatsApp.
//!
//! WhatsApp paints its emoji with the set of the phone: the Google one on
//! Android, the Apple one on the web and on the iPhone. The window drew them
//! with the font of the system — Segoe UI Emoji on Windows —, which is another
//! drawing: the same smile is not the one of the chat.
//!
//! Two fonts take part:
//!
//! - The one of the user, when there is a file: `<data>/fonts/emoji.ttf`, or
//!   the file that `WHATFAST_EMOJI_FONT` points at. The Apple set goes here,
//!   because its art cannot travel in the repository.
//! - The Google set (Noto Color Emoji), inside the executable, for the
//!   profiles without a file of their own.
//!
//! Both are bitmap fonts (CBDT/CBLC), the format that DirectWrite and FreeType
//! read: the same file works on Windows and on Linux. On Windows the file is
//! also copied to the folder of the fonts of the user, because the fallback of
//! the platform resolves the families in the collection of the system, not in
//! the one of the fonts loaded in memory. On Linux the fonts of the process are
//! the whole database of the text system, so loading them is enough.
//!
//! The root of the window names the family in its fallbacks, so a glyph that
//! the font of the interface does not have (an emoji) is painted with this one.

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::OnceLock;

/// The family of the Google set, the one that travels inside the executable.
const NOTO_FAMILY: &str = "Noto Color Emoji";

/// The name of the file of the Google set in the folder of the fonts.
const NOTO_FILE: &str = "NotoColorEmoji.ttf";

/// The font of the Google set, inside the executable: the bitmap build that
/// Android uses.
const FONT: &[u8] = include_bytes!("../assets/fonts/NotoColorEmoji.ttf");

/// The family of the font that is in use.
static FAMILY: OnceLock<String> = OnceLock::new();

/// The family that the fallbacks of the window have to name.
pub(crate) fn family() -> &'static str {
    FAMILY.get().map(String::as_str).unwrap_or(NOTO_FAMILY)
}

/// The font of the emoji that the process uses.
struct EmojiFont {
    bytes: Cow<'static, [u8]>,
    family: String,
    file: String,
}

/// Puts the font of the emoji in the text system of the process.
///
/// A profile that cannot take the file still has the font in the custom
/// collection of the text system: the picker and the reactions keep their
/// drawing, and the messages fall back to the font of the system.
pub(crate) fn install(cx: &gpui::App) {
    let font = load();
    let _ = FAMILY.set(font.family.clone());
    #[cfg(windows)]
    let _ = install_for_the_user(&font);
    let _ = cx.text_system().add_fonts(vec![font.bytes]);
    #[cfg(windows)]
    wait_for_the_system_collection(cx);
}

/// The font of the user if there is one, and the Google set otherwise.
fn load() -> EmojiFont {
    if let Some(font) = the_font_of_the_user() {
        return font;
    }
    EmojiFont {
        bytes: Cow::Borrowed(FONT),
        family: NOTO_FAMILY.to_string(),
        file: NOTO_FILE.to_string(),
    }
}

/// Reads the file that the user put for the emoji, when it is a font.
fn the_font_of_the_user() -> Option<EmojiFont> {
    let path = std::env::var_os("WHATFAST_EMOJI_FONT")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .or_else(|| {
            let path = whatfast_model::paths::data_dir().join("fonts").join("emoji.ttf");
            path.is_file().then_some(path)
        })?;
    let bytes = std::fs::read(&path).ok()?;
    let family = family_name(&bytes)?;
    Some(EmojiFont {
        bytes: Cow::Owned(bytes),
        family,
        file: path.file_name()?.to_string_lossy().into_owned(),
    })
}

/// The family name of a font, read from its `name` table.
///
/// The first record of the Windows platform wins; the other platforms are the
/// fallback, because a file can come with the two of them.
fn family_name(bytes: &[u8]) -> Option<String> {
    let tables = u16::from_be_bytes([*bytes.get(4)?, *bytes.get(5)?]) as usize;
    for table in 0..tables {
        let record = 12 + table * 16;
        if bytes.get(record..record + 4)? != b"name" {
            continue;
        }
        let start = u32::from_be_bytes(bytes.get(record + 8..record + 12)?.try_into().ok()?) as usize;
        let count = u16::from_be_bytes(bytes.get(start + 2..start + 4)?.try_into().ok()?) as usize;
        let storage =
            start + u16::from_be_bytes(bytes.get(start + 4..start + 6)?.try_into().ok()?) as usize;
        let mut other = None;
        for name in 0..count {
            let entry = start + 6 + name * 12;
            let platform = u16::from_be_bytes(bytes.get(entry..entry + 2)?.try_into().ok()?);
            let id = u16::from_be_bytes(bytes.get(entry + 6..entry + 8)?.try_into().ok()?);
            if id != 1 {
                continue;
            }
            let length = u16::from_be_bytes(bytes.get(entry + 8..entry + 10)?.try_into().ok()?) as usize;
            let offset = u16::from_be_bytes(bytes.get(entry + 10..entry + 12)?.try_into().ok()?) as usize;
            let text = bytes.get(storage + offset..storage + offset + length)?;
            let name = if platform == 3 || platform == 0 {
                let units: Vec<u16> = text
                    .chunks_exact(2)
                    .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                    .collect();
                String::from_utf16(&units).ok()?
            } else {
                text.iter().map(|byte| *byte as char).collect()
            };
            if name.is_empty() {
                continue;
            }
            if platform == 3 || platform == 0 {
                return Some(name);
            }
            other = other.or(Some(name));
        }
        if other.is_some() {
            return other;
        }
    }
    None
}

/// Copies the font to the folder of the fonts of the user and registers it.
#[cfg(windows)]
fn install_for_the_user(font: &EmojiFont) -> std::io::Result<PathBuf> {
    let path = font_path(&font.file).ok_or_else(|| std::io::Error::other("no folder of fonts"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let size = std::fs::metadata(&path)
        .map(|meta| meta.len())
        .unwrap_or_default();
    if size != font.bytes.len() as u64 {
        std::fs::write(&path, &font.bytes)?;
    }
    register(&path, &font.family)?;
    Ok(path)
}

/// The path of the font in the profile of the user.
#[cfg(windows)]
fn font_path(file: &str) -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|base| {
        PathBuf::from(base)
            .join("Microsoft")
            .join("Windows")
            .join("Fonts")
            .join(file)
    })
}

/// Registers the file in the fonts of the user and tells the system.
#[cfg(windows)]
fn register(path: &std::path::Path, family: &str) -> std::io::Result<()> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegCreateKeyExW, RegSetValueExW,
    };

    const SUBKEY: &str = "Software\\Microsoft\\Windows NT\\CurrentVersion\\Fonts";

    let subkey = wide(SUBKEY);
    let value = wide(&format!("{family} (TrueType)"));
    let data = wide(&path.to_string_lossy());
    let mut key: windows_sys::Win32::System::Registry::HKEY = std::ptr::null_mut();
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            std::ptr::null(),
            0,
            KEY_SET_VALUE,
            std::ptr::null(),
            &mut key,
            std::ptr::null_mut(),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(status as i32));
    }
    let bytes = unsafe {
        std::slice::from_raw_parts(
            data.as_ptr().cast::<u8>(),
            data.len() * std::mem::size_of::<u16>(),
        )
    };
    let status = unsafe {
        RegSetValueExW(
            key,
            value.as_ptr(),
            0,
            REG_SZ,
            bytes.as_ptr(),
            bytes.len() as u32,
        )
    };
    unsafe {
        RegCloseKey(key);
    }
    if status != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(status as i32));
    }
    broadcast_font_change();
    Ok(())
}

/// A text of Windows: UTF-16 and the zero that ends it.
#[cfg(windows)]
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Asks the shell to look at the fonts again, so the new file is in the
/// collection of every program.
#[cfg(windows)]
fn broadcast_font_change() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_FONTCHANGE,
    };
    let mut result = 0usize;
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_FONTCHANGE,
            0,
            0,
            SMTO_ABORTIFHUNG,
            1000,
            &mut result,
        );
    }
}

/// Waits until the collection of the system knows the family.
///
/// The fallback of a font is built with the collection of the system, and the
/// one that the process opened before the copy does not have the file. A
/// family that does not exist asks the platform for the collection again, and
/// the loop gives the service of the fonts the moment it needs.
#[cfg(windows)]
fn wait_for_the_system_collection(cx: &gpui::App) {
    for attempt in 0..10 {
        let _ = cx.text_system().resolve_font(&gpui::Font {
            family: "Whatfast emoji refresh".into(),
            ..Default::default()
        });
        if the_system_has_the_font(cx) {
            return;
        }
        if attempt < 9 {
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
    }
}

/// Whether the collection of the system already lists the family.
///
/// The list of the text system has the names of the system and then the ones
/// of the fonts loaded in memory: the family appears twice when the system
/// knows the file.
#[cfg(windows)]
fn the_system_has_the_font(cx: &gpui::App) -> bool {
    cx.text_system()
        .all_font_names()
        .iter()
        .filter(|name| name.as_str() == family())
        .count()
        >= 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_font_is_a_true_type_file() {
        // The first four bytes of an sfnt: the version of TrueType.
        assert_eq!(&FONT[..4], &[0, 1, 0, 0]);
        assert!(FONT.len() > 1_000_000);
    }

    #[test]
    fn the_family_is_the_one_of_the_bundled_font() {
        assert_eq!(family_name(FONT).as_deref(), Some(NOTO_FAMILY));
    }

    #[test]
    fn a_file_that_is_not_a_font_has_no_family() {
        assert_eq!(family_name(b"not a font"), None);
    }
}
