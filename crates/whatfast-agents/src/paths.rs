//! The folders of Whatfast on each system, and the expansion of the paths
//! that the specs write with variables.

use std::path::PathBuf;

/// The system this build runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    /// Microsoft Windows.
    Windows,
    /// Apple macOS.
    Macos,
    /// Linux and the systems that follow its layout.
    Linux,
}

/// The system of this build.
pub fn os() -> Os {
    if cfg!(windows) {
        Os::Windows
    } else if cfg!(target_os = "macos") {
        Os::Macos
    } else {
        Os::Linux
    }
}

/// The folder of the user data: the device store, the media and the agents.
///
/// `%LOCALAPPDATA%\Whatfast` on Windows, `~/Library/Application Support/Whatfast`
/// on macOS, `$XDG_DATA_HOME/whatfast` on Linux.
pub fn data_dir() -> Option<PathBuf> {
    match os() {
        Os::Windows => expand("%LOCALAPPDATA%/Whatfast"),
        Os::Macos => expand("~/Library/Application Support/Whatfast"),
        Os::Linux => {
            expand("$XDG_DATA_HOME/whatfast").or_else(|| expand("~/.local/share/whatfast"))
        }
    }
}

/// The folder of the agents: one token file per agent.
pub fn agents_dir() -> Option<PathBuf> {
    Some(data_dir()?.join("agents"))
}

/// Resolves `~`, `%VAR%`, `$VAR` and `${VAR}` in a path.
///
/// It returns `None` when a variable has no value, so a spec with a path that
/// does not exist on this system is skipped instead of pointing at a literal.
pub fn expand(template: &str) -> Option<PathBuf> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    if let Some(stripped) = rest.strip_prefix("~/") {
        out.push_str(&home()?);
        out.push('/');
        rest = stripped;
    } else if rest == "~" {
        return Some(PathBuf::from(home()?));
    }
    let bytes: Vec<char> = rest.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == '%' {
            let end = bytes[i + 1..].iter().position(|ch| *ch == '%')?;
            let name: String = bytes[i + 1..i + 1 + end].iter().collect();
            out.push_str(&variable(&name)?);
            i += end + 2;
            continue;
        }
        if c == '$' {
            if bytes.get(i + 1) == Some(&'{') {
                let end = bytes[i + 2..].iter().position(|ch| *ch == '}')?;
                let name: String = bytes[i + 2..i + 2 + end].iter().collect();
                out.push_str(&variable(&name)?);
                i += end + 3;
                continue;
            }
            let mut end = i + 1;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == '_') {
                end += 1;
            }
            if end > i + 1 {
                let name: String = bytes[i + 1..end].iter().collect();
                out.push_str(&variable(&name)?);
                i = end;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    Some(PathBuf::from(out))
}

/// The home folder of the user.
pub fn home() -> Option<String> {
    std::env::var("HOME")
        .ok()
        .or_else(|| std::env::var("USERPROFILE").ok())
        .filter(|value| !value.is_empty())
}

/// One environment variable, with the two names Windows and the rest use.
fn variable(name: &str) -> Option<String> {
    let value = std::env::var(name)
        .or_else(|_| std::env::var(name.to_ascii_uppercase()))
        .ok()?;
    if value.is_empty() { None } else { Some(value) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_the_home() {
        let home = home().expect("home");
        let path = expand("~/.codex/config.toml").expect("path");
        assert_eq!(path, PathBuf::from(format!("{home}/.codex/config.toml")));
    }

    #[test]
    fn expands_a_windows_variable() {
        let path = expand("%USERPROFILE%/.cursor/mcp.json").expect("path");
        assert!(path.to_string_lossy().ends_with("/.cursor/mcp.json"));
        assert!(!path.to_string_lossy().contains('%'));
    }

    #[test]
    fn a_missing_variable_is_none() {
        assert!(expand("%WHATFAST_NOT_SET_ANYWHERE%/x").is_none());
    }
}
