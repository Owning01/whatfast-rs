//! Inline message translation with background HTTP lookup and local memory cache.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

fn cache() -> &'static Mutex<HashMap<String, String>> {
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cache_key(message_id: &str, target_lang: &str) -> String {
    let lang = if target_lang.is_empty() {
        "en"
    } else {
        target_lang
    };
    format!("{lang}:{message_id}")
}

/// Returns a cached translation if already fetched for `target_lang`.
pub(crate) fn cached(message_id: &str, target_lang: &str) -> Option<String> {
    let key = cache_key(message_id, target_lang);
    cache().lock().ok().and_then(|map| map.get(&key).cloned())
}

/// Translates `text` into `target_lang` (e.g. `"es"`, `"en"`) synchronously on a worker thread.
pub(crate) fn translate_sync(message_id: &str, text: &str, target_lang: &str) -> Result<String, String> {
    let lang = if target_lang.is_empty() { "en" } else { target_lang };
    if let Some(hit) = cached(message_id, lang) {
        return Ok(hit);
    }
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("Empty message".to_string());
    }
    let langpair = format!("autodetect|{lang}");
    let encoded_q = crate::helpers::percent_encode(trimmed);
    let url = format!(
        "https://api.mymemory.translated.net/get?q={encoded_q}&langpair={langpair}"
    );
    let resp = ureq::get(&url)
        .call()
        .map_err(|e| format!("Translation request failed: {e}"))?;
    let bytes = resp
        .into_body()
        .read_to_vec()
        .map_err(|e| format!("Translation read failed: {e}"))?;
    let val: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("Invalid JSON: {e}"))?;
    let translated = val
        .get("responseData")
        .and_then(|d| d.get("translatedText"))
        .and_then(|t| t.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "No translation returned".to_string())?
        .to_string();
    if let Ok(mut map) = cache().lock() {
        if map.len() >= 512 {
            map.clear();
        }
        map.insert(cache_key(message_id, lang), translated.clone());
    }
    Ok(translated)
}
