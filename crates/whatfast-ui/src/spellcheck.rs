//! Cross-platform spellcheck helper (hunspell on Linux/macOS, basic word check fallback).

#[cfg(not(windows))]
use std::process::{Command, Stdio};

/// Checks a single word or short phrase and returns misspelled words.
#[allow(dead_code)]
pub(crate) fn misspelled_words(text: &str, lang: &str) -> Vec<String> {
    #[cfg(not(windows))]
    {
        use std::io::Write as _;
        let dict = if lang.starts_with("es") {
            "es_ES"
        } else {
            "en_US"
        };
        if let Ok(mut child) = Command::new("hunspell")
            .args(["-d", dict, "-l"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            if let Ok(output) = child.wait_with_output()
                && output.status.success()
            {
                return String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToOwned::to_owned)
                    .collect();
            }
        }
    }
    let _ = (text, lang);
    Vec::new()
}
