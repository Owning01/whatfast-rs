//! Finds the language files and embeds them in the binary.
//!
//! Every `locales/<code>.json` file is one language. Adding a language is
//! adding the file: this script reads its `meta.code` and `meta.name`, and
//! generates the table that [`whatfast_i18n`] reads at startup. There is no
//! list of languages to edit by hand.
//!
//! The file has this shape:
//!
//! ```json
//! {
//!   "meta": { "code": "es", "name": "Español" },
//!   "strings": { "Type a message": "Escribí un mensaje" }
//! }
//! ```
//!
//! The keys are the English text, so a language that forgets a string falls
//! back to English instead of showing a blank.

use std::{env, fs, path::Path};

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
            let file_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("a file name")
                .to_string();
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            let value: serde_json::Value = serde_json::from_str(&text)
                .unwrap_or_else(|error| panic!("{} is not valid JSON: {error}", path.display()));
            let code = value
                .get("meta")
                .and_then(|meta| meta.get("code"))
                .and_then(|code| code.as_str())
                .unwrap_or_else(|| panic!("{} has no meta.code", path.display()));
            let name = value
                .get("meta")
                .and_then(|meta| meta.get("name"))
                .and_then(|name| name.as_str())
                .unwrap_or_else(|| panic!("{} has no meta.name", path.display()));
            found.push((code.to_string(), name.to_string(), file_name));
        }
    }
    // The order of the language list is the order of the files: a stable
    // order keeps the settings panel from moving between builds.
    found.sort_by(|left, right| left.0.cmp(&right.0));

    let mut table = String::from("pub(crate) static BUNDLED: &[BundledLocale] = &[\n");
    for (code, name, file_name) in found {
        table.push_str(&format!(
            "    BundledLocale {{ code: {code:?}, name: {name:?}, json: include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/locales/{file_name}\")) }},\n"
        ));
    }
    table.push_str("];\n");

    let out_dir = env::var("OUT_DIR").expect("the output directory");
    fs::write(Path::new(&out_dir).join("locales.rs"), table).expect("the generated table");
}
