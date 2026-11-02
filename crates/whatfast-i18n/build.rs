//! Finds the language files and builds the static dictionaries.
//!
//! Every `locales/<code>.json` file is one language. Adding a language is
//! adding the file: this script reads its `meta.code` and `meta.name`, parses
//! its `strings` and writes a perfect hash map for each one. The map lives in
//! the read-only segment of the binary: the window never parses JSON, never
//! allocates a dictionary and never hashes a key at runtime.
//!
//! The file has this shape:
//!
//! ```json
//! {
//!   "meta": { "code": "es", "name": "Español" },
//!   "strings": { "Type a message": "Escribe un mensaje" }
//! }
//! ```
//!
//! The keys are the English text, so a language that forgets a string falls
//! back to English instead of showing a blank.

use std::{env, fs, path::Path};

use phf_codegen::Map;

fn main() {
    let manifest = env::var("CARGO_MANIFEST_DIR").expect("the manifest directory");
    let locales = Path::new(&manifest).join("locales");
    // A new file in the directory has to re-run the script: watching the
    // directory covers the files that do not exist yet.
    println!("cargo:rerun-if-changed=locales");
    println!("cargo:rerun-if-changed=build.rs");

    let mut found = Vec::new();
    if let Ok(entries) = fs::read_dir(&locales) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            let value: serde_json::Value = serde_json::from_str(&text)
                .unwrap_or_else(|error| panic!("{} is not valid JSON: {error}", path.display()));
            let code = value
                .get("meta")
                .and_then(|meta| meta.get("code"))
                .and_then(|code| code.as_str())
                .unwrap_or_else(|| panic!("{} has no meta.code", path.display()))
                .to_string();
            let name = value
                .get("meta")
                .and_then(|meta| meta.get("name"))
                .and_then(|name| name.as_str())
                .unwrap_or_else(|| panic!("{} has no meta.name", path.display()))
                .to_string();
            let mut strings = Map::new();
            let entries = value
                .get("strings")
                .and_then(|strings| strings.as_object())
                .unwrap_or_else(|| panic!("{} has no strings", path.display()));
            for (key, text) in entries {
                let text = text
                    .as_str()
                    .unwrap_or_else(|| panic!("{}: {key} is not a string", path.display()));
                // `phf_codegen` writes the literal with the quoting and the
                // escapes of Rust: a key with quotes or a newline survives.
                strings.entry(key.as_str(), &format!("{text:?}"));
            }
            found.push((code, name, strings.build().to_string()));
        }
    }
    // The order of the language list is the order of the files: a stable
    // order keeps the settings panel from moving between builds.
    found.sort_by(|left, right| left.0.cmp(&right.0));

    let mut table = String::from("pub(crate) static BUNDLED: &[BundledLocale] = &[\n");
    for (code, name, strings) in found {
        table.push_str(&format!(
            "    BundledLocale {{ code: {code:?}, name: {name:?}, strings: {strings} }},\n"
        ));
    }
    table.push_str("];\n");

    let out_dir = env::var("OUT_DIR").expect("the output directory");
    fs::write(Path::new(&out_dir).join("locales.rs"), table).expect("the generated table");
}
