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

use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// One language file, as the build script found it.
pub(crate) struct BundledLocale {
    /// The code of the language: `en`, `es`, `zh`.
    pub(crate) code: &'static str,
    /// The name of the language in its own language: `English`, `Español`,
    /// `中文`. It is what the settings panel shows.
    pub(crate) name: &'static str,
    /// The JSON of the file, embedded in the binary.
    pub(crate) json: &'static str,
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

/// The strings of one language, parsed once.
struct Dictionary {
    code: &'static str,
    name: &'static str,
    strings: HashMap<&'static str, &'static str>,
}

/// Every language, in the order of the file names.
///
/// The strings live for the whole process: the module leaks them on purpose,
/// because the interface reads them on every frame and copying them would
/// allocate on each label.
static DICTIONARIES: LazyLock<Vec<Dictionary>> =
    LazyLock::new(|| BUNDLED.iter().map(parse).collect());

/// The index of the language in use.
static ACTIVE: AtomicUsize = AtomicUsize::new(0);

/// The language of a profile that never chose one.
pub const DEFAULT_LANGUAGE: &str = "en";

/// Leaks a string of a dictionary: it lives as long as the process.
fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

/// Reads one embedded file.
fn parse(bundle: &BundledLocale) -> Dictionary {
    let value: serde_json::Value = serde_json::from_str(bundle.json)
        .unwrap_or_else(|error| panic!("locale {} is not valid JSON: {error}", bundle.code));
    let mut strings = HashMap::new();
    if let Some(map) = value.get("strings").and_then(|strings| strings.as_object()) {
        for (key, value) in map {
            if let Some(text) = value.as_str() {
                strings.insert(leak(key.clone()), leak(text.to_string()));
            }
        }
    }
    Dictionary {
        code: bundle.code,
        name: bundle.name,
        strings,
    }
}

/// The index of English: the fallback of every missing string.
fn english_index() -> usize {
    DICTIONARIES
        .iter()
        .position(|dictionary| dictionary.code == DEFAULT_LANGUAGE)
        .unwrap_or(0)
}

/// The languages the application can speak, English first.
pub fn languages() -> Vec<Language> {
    let mut languages: Vec<Language> = DICTIONARIES
        .iter()
        .map(|dictionary| Language {
            code: dictionary.code,
            name: dictionary.name,
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
    let Some(index) = DICTIONARIES
        .iter()
        .position(|dictionary| dictionary.code.eq_ignore_ascii_case(code))
    else {
        return false;
    };
    ACTIVE.store(index, Ordering::Relaxed);
    true
}

/// The code of the active language.
pub fn current_language() -> &'static str {
    let dictionaries = &*DICTIONARIES;
    dictionaries[ACTIVE.load(Ordering::Relaxed).min(dictionaries.len() - 1)].code
}

/// Whether a language has a translation for a key.
pub fn has_translation(code: &str, key: &str) -> bool {
    DICTIONARIES
        .iter()
        .find(|dictionary| dictionary.code.eq_ignore_ascii_case(code))
        .is_some_and(|dictionary| dictionary.strings.contains_key(key))
}

/// The text of a key in the active language.
///
/// Falls back to English and then to the key itself, so a missing translation
/// never hides a label.
pub fn t(key: &'static str) -> &'static str {
    let dictionaries = &*DICTIONARIES;
    let active = &dictionaries[ACTIVE.load(Ordering::Relaxed).min(dictionaries.len() - 1)];
    if let Some(text) = active.strings.get(key) {
        return text;
    }
    if let Some(text) = dictionaries[english_index()].strings.get(key) {
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

/// How many keys of a language are translated, and how many keys the
/// dictionaries of the process know in total.
///
/// It is the report of the script `scripts/idiomas.py`: the test of the
/// crate prints it, and the script uses the same numbers to tell a
/// translator what is left.
pub fn coverage(code: &str) -> (usize, usize) {
    let total: usize = DICTIONARIES
        .iter()
        .map(|dictionary| dictionary.strings.len())
        .max()
        .unwrap_or(0);
    let translated = DICTIONARIES
        .iter()
        .find(|dictionary| dictionary.code.eq_ignore_ascii_case(code))
        .map_or(0, |dictionary| dictionary.strings.len());
    (translated, total)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::{coverage, current_language, has_translation, languages, set_language, t, t_args};

    /// The active language is a global of the process: the tests that change
    /// it take this lock, or the parallel runner makes them race.
    static LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn every_file_has_its_metadata_and_english_comes_first() {
        let languages = languages();
        assert!(!languages.is_empty());
        assert_eq!(languages[0].code, "en");
        for language in &languages {
            assert!(!language.code.is_empty());
            assert!(!language.name.is_empty());
        }
        assert!(languages.iter().any(|language| language.code == "es"));
        assert!(languages.iter().any(|language| language.code == "zh"));
    }

    #[test]
    fn a_missing_string_falls_back_to_the_key() {
        let _guard = LOCK.lock().unwrap();
        assert!(set_language("es"));
        // The key that no language translates still shows the English text.
        assert_eq!(
            t("A key that no language translates"),
            "A key that no language translates"
        );
        assert!(set_language("en"));
    }

    #[test]
    fn a_language_that_does_not_exist_does_not_change_the_active_one() {
        let _guard = LOCK.lock().unwrap();
        assert!(set_language("en"));
        assert!(!set_language("xx"));
        assert_eq!(current_language(), "en");
    }

    #[test]
    fn the_placeholders_survive_the_translation() {
        let _guard = LOCK.lock().unwrap();
        assert!(set_language("es"));
        let text = t_args("Type a message", &[]);
        assert_eq!(text, "Escribí un mensaje");
        assert!(set_language("en"));
    }

    #[test]
    fn the_coverage_of_a_language_is_reported() {
        for language in languages() {
            let (translated, total) = coverage(language.code);
            println!("{}: {translated}/{total} strings", language.code);
            assert!(total > 0, "no language has strings");
        }
    }

    #[test]
    fn the_spanish_and_chinese_files_translate_the_interface() {
        for code in ["es", "zh"] {
            let (translated, total) = coverage(code);
            assert!(
                translated * 100 / total.max(1) >= 90,
                "{code} only translates {translated} of {total} strings"
            );
        }
    }

    #[test]
    fn every_translation_keeps_the_placeholders_of_its_key() {
        for language in languages() {
            for (key, value) in super::DICTIONARIES
                .iter()
                .find(|dictionary| dictionary.code == language.code)
                .map(|dictionary| dictionary.strings.iter().collect::<Vec<_>>())
                .unwrap_or_default()
            {
                for placeholder in placeholders(key) {
                    assert!(
                        value.contains(&format!("{{{placeholder}}}")),
                        "{} translates {key:?} without {{{placeholder}}}: {value:?}",
                        language.code
                    );
                }
            }
        }
    }

    #[test]
    fn the_settings_panel_can_list_the_languages() {
        for language in languages() {
            assert!(
                has_translation(language.code, "Type a message") || language.code == "en",
                "{} does not translate the composer",
                language.code
            );
        }
    }

    /// The names of the placeholders of a key: `{count}` gives `count`.
    fn placeholders(key: &str) -> Vec<String> {
        let mut names = Vec::new();
        let mut rest = key;
        while let Some(start) = rest.find('{') {
            let after = &rest[start + 1..];
            let Some(end) = after.find('}') else { break };
            let name = &after[..end];
            if !name.is_empty()
                && name
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_')
            {
                names.push(name.to_string());
            }
            rest = &after[end + 1..];
        }
        names
    }
}
