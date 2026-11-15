//! The real configuration of OpenCode, when this machine has one.
//!
//! The file of the user is not a fixture: it lives in the home folder and it is
//! only read here. A machine without OpenCode gets a file with the same shape,
//! so the test always checks the merge: the head comment survives, the entry is
//! added and removed, and the rest of the file stays.

use std::path::PathBuf;

/// The file OpenCode reads on this system.
fn config_file() -> PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from);
    home.unwrap_or_default()
        .join(".config")
        .join("opencode")
        .join("opencode.jsonc")
}

/// The shape of the file on a machine without OpenCode: a head comment, one
/// key of the user and one server that is not ours.
const FALLBACK: &str = "// Configuración global.\n{\n  \"theme\": \"dark\",\n  \"mcp\": {\n    \"servers\": {\n      \"other\": { \"command\": [\"x\"], \"type\": \"local\" },\n    },\n  },\n}\n";

/// The comment lines that open the document, which the merge must keep.
fn head_comments(text: &str) -> String {
    let mut head = String::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            head.push_str(line);
            head.push('\n');
        } else if trimmed.is_empty() && head.is_empty() {
            continue;
        } else {
            break;
        }
    }
    head
}

#[test]
fn the_merge_survives_the_real_file() {
    let real = std::fs::read_to_string(config_file()).ok();
    let text = real.unwrap_or_else(|| FALLBACK.to_string());
    let head = head_comments(&text);
    assert!(!head.is_empty(), "the file must open with a comment");
    let spec = whatfast_agents::by_id("opencode").expect("opencode");
    let command = whatfast_agents::McpCommand::new("C:/Whatfast/whatfast.exe", "opencode");
    let dir = std::env::temp_dir().join(format!(
        "whatfast-jsonc-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir).expect("dir");
    let copy = dir.join("opencode.jsonc");
    std::fs::write(&copy, &text).expect("copy");

    let report = whatfast_agents::write::register_at(spec, &command, &copy).expect("register");
    assert_eq!(report.method, "jsonc");
    let written = std::fs::read_to_string(&copy).expect("read");
    // The head comment stays where it was: the merge never starts over.
    assert!(written.starts_with(&head), "{written}");
    assert!(written.contains("\"whatfast\""), "{written}");
    assert!(written.contains("--agent"), "{written}");

    // Removing the entry leaves the file without it and with the rest intact.
    let cleaned = whatfast_agents::write::unregister_at(spec, &copy);
    assert!(cleaned.is_ok(), "{cleaned:?}");
    let after = std::fs::read_to_string(&copy).expect("read");
    assert!(!after.contains("\"whatfast\""), "{after}");
    assert!(after.starts_with(&head), "{after}");
    let _ = std::fs::remove_dir_all(&dir);
}
