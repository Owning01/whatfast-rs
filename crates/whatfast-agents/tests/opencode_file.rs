//! The real configuration of OpenCode, when this machine has one.
//!
//! The file of the user is not a fixture: it lives in the home folder and it is
//! only read here. The test passes on a machine without OpenCode and checks the
//! real shape —`mcp.servers`, comments, trailing commas— on the one that has it,
//! which is the only way to know that the merge survives a file nobody wrote for
//! this program.

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

#[test]
fn the_merge_survives_the_real_file() {
    let path = config_file();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
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
    // The keys of the user are still there: the merge never starts over.
    assert!(written.contains("disabled_providers"), "{written}");
    assert!(written.contains("\"whatfast\""), "{written}");
    assert!(written.contains("--agent"), "{written}");

    // Removing the entry leaves the file without it and with the rest intact.
    let cleaned = whatfast_agents::write::unregister_at(spec, &copy);
    assert!(cleaned.is_ok(), "{cleaned:?}");
    let after = std::fs::read_to_string(&copy).expect("read");
    assert!(!after.contains("\"whatfast\""), "{after}");
    assert!(after.contains("disabled_providers"), "{after}");
    let _ = std::fs::remove_dir_all(&dir);
}
