//! Whether a harness exists on this machine, and where.

use std::path::{Path, PathBuf};

use crate::paths::{self, Os};
use crate::spec::{ClientSpec, Detect, Location, Method};

/// What the detection found.
#[derive(Clone, Debug, Default)]
pub struct Found {
    /// The program on `PATH`, when the client ships one.
    pub bin: Option<PathBuf>,
    /// The configuration file or folder, when the client has one.
    pub file: Option<PathBuf>,
}

impl Found {
    /// True when at least one of the two appeared.
    pub fn is_some(&self) -> bool {
        self.bin.is_some() || self.file.is_some()
    }
}

/// Looks for one client on this machine.
pub fn find(spec: &ClientSpec) -> Found {
    match spec.detect {
        Detect::Bin(name) => Found {
            bin: which(name),
            file: None,
        },
        Detect::File(location) => Found {
            bin: None,
            file: existing(location),
        },
        Detect::BinOrFile(name, location) => Found {
            bin: which(name),
            file: existing(location),
        },
    }
}

/// True when the client is installed.
pub fn installed(spec: &ClientSpec) -> bool {
    find(spec).is_some()
}

/// The configuration file of a client that keeps one on disk.
pub fn config_path(spec: &ClientSpec) -> Option<PathBuf> {
    let location = match spec.method {
        Method::Json { file, .. }
        | Method::Toml { file, .. }
        | Method::Yaml { file, .. }
        | Method::OpenCode { file } => file,
        Method::Cli { .. } | Method::JsonArg { .. } => return None,
    };
    existing(location)
}

/// The path of a location on this system, expanded.
pub fn location_path(location: Location) -> Option<PathBuf> {
    let raw = location.for_os(paths::os())?;
    paths::expand(raw)
}

/// The path when it exists.
fn existing(location: Location) -> Option<PathBuf> {
    let path = location_path(location)?;
    path.exists().then_some(path)
}

/// Finds a program on `PATH`, with the extensions of Windows.
pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for folder in std::env::split_paths(&path) {
        if folder.as_os_str().is_empty() {
            continue;
        }
        let direct = folder.join(name);
        if is_executable(&direct) {
            return Some(direct);
        }
        if paths::os() == Os::Windows {
            for ext in windows_extensions() {
                let candidate = folder.join(format!("{name}{ext}"));
                if is_executable(&candidate) {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// The extensions Windows tries, `PATHEXT` first.
fn windows_extensions() -> Vec<String> {
    let mut list = Vec::new();
    if let Ok(value) = std::env::var("PATHEXT") {
        for ext in value.split(';') {
            let ext = ext.trim();
            if !ext.is_empty() {
                list.push(ext.to_ascii_lowercase());
            }
        }
    }
    if list.is_empty() {
        list = vec![".exe".into(), ".cmd".into(), ".bat".into()];
    }
    list
}

/// A file this process can run.
fn is_executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        return meta.permissions().mode() & 0o111 != 0;
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::all;

    #[test]
    fn detects_what_the_path_holds() {
        // `cargo` runs this test, so it must be on PATH.
        assert!(which("cargo").is_some());
        assert!(which("whatfast-not-a-program-anywhere").is_none());
    }

    #[test]
    fn a_client_without_a_program_still_reports_a_file() {
        for spec in all() {
            let found = find(spec);
            assert_eq!(found.is_some(), installed(spec));
        }
    }
}
