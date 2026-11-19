//! The translations of the interface of Whatfast.
//!
//! One JSON file per language lives in `locales/`. The build script embeds
//! every file and this module keeps the active one: to add a language, add
//! `locales/<code>.json` with its `meta` and its `strings` and rebuild. No
//! code changes, no central list.
//!
//! The key of a string is the English text, so an untranslated string falls
//! back to English instead of showing a blank or a key. The English file is
//! therefore a dictionary of synonyms at most: the source text already is
//! the fallback.
//!
//! ```ignore
//! whatfast_i18n::set_language("es");
//! assert_eq!(whatfast_i18n::t("Type a message"), "Escribí un mensaje");
//! ```

use std::sync::LazyLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// One language, as the build script found it.
///
/// The strings are a perfect hash map built at compile time: the table lives
/// in the read-only segment of the binary. No JSON is parsed at startup, no
/// string is leaked into the heap, and a lookup is O(1) without hashing the
/// key at runtime.
pub(crate) struct BundledLocale {
    /// The code of the language: `en`, `es`, `zh`.
    pub(crate) code: &'static str,
    /// The name of the language in its own language: `English`, `Español`,
    /// `中文`. It is what the settings panel shows.
    pub(crate) name: &'static str,
    /// The strings of the file, in the read-only segment.
    pub(crate) strings: phf::Map<&'static str, &'static str>,
}

include!(concat!(env!("OUT_DIR"), "/locales.rs"));

/// A language the application can speak.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Language {
    /// The code: `en`, `es`, `zh`.
    pub code: &'static str,
    /// The name in its own language.
    pub name: &'static str,
}

/// The index of the language in use.
static ACTIVE: AtomicUsize = AtomicUsize::new(0);

/// The language of a profile that never chose one.
pub const DEFAULT_LANGUAGE: &str = "en";

/// The index of English: the fallback of every missing string. It is found
/// once, not on every lookup that misses.
static ENGLISH_INDEX: LazyLock<usize> = LazyLock::new(|| {
    BUNDLED
        .iter()
        .position(|language| language.code == DEFAULT_LANGUAGE)
        .unwrap_or(0)
});

/// The languages the application can speak, English first.
pub fn languages() -> Vec<Language> {
    let mut languages: Vec<Language> = BUNDLED
        .iter()
        .map(|language| Language {
            code: language.code,
            name: language.name,
        })
        .collect();
    languages.sort_by_key(|language| (language.code != DEFAULT_LANGUAGE, language.code));
    languages
}

/// Chooses the language of the interface.
///
/// Returns `false` when the code is not one of the embedded languages; the
/// active language does not change in that case.
pub fn set_language(code: &str) -> bool {
    let Some(index) = BUNDLED
        .iter()
        .position(|language| language.code.eq_ignore_ascii_case(code))
    else {
        return false;
    };
    ACTIVE.store(index, Ordering::Relaxed);
    true
}

/// The code of the active language.
pub fn current_language() -> &'static str {
    BUNDLED[ACTIVE.load(Ordering::Relaxed).min(BUNDLED.len() - 1)].code
}

/// The text of a key in the active language.
///
/// Falls back to English and then to the key itself, so a missing translation
/// never hides a label.
pub fn t(key: &'static str) -> &'static str {
    let active = &BUNDLED[ACTIVE.load(Ordering::Relaxed).min(BUNDLED.len() - 1)];
    if let Some(text) = active.strings.get(key) {
        return text;
    }
    if let Some(text) = BUNDLED[*ENGLISH_INDEX].strings.get(key) {
        return text;
    }
    key
}

/// The text of a key with its placeholders replaced.
///
/// The key carries the placeholders as `{name}`: `"{count} members"`. A
/// translation keeps the same placeholder, so the order of the words can
/// change between languages.
pub fn t_args(key: &'static str, args: &[(&str, &str)]) -> String {
    let mut text = t(key).to_string();
    for (name, value) in args {
        text = text.replace(&format!("{{{name}}}"), value);
    }
    text
}

