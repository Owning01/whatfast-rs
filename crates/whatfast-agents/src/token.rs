//! The token that names one agent, and the file that keeps it.
//!
//! The token travels in the configuration of the harness, never on the
//! command line: a process list is public on a shared machine. The server
//! reads it from `agents/<id>.token` and compares it with the one the harness
//! presented.

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use crate::paths;

/// A random token, as lowercase hex.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    /// 32 random bytes, from the system generator.
    pub fn generate() -> io::Result<Self> {
        let mut bytes = [0u8; 32];
        getrandom::getrandom(&mut bytes)
            .map_err(|error| io::Error::other(format!("no random source: {error}")))?;
        let mut text = String::with_capacity(64);
        const HEX: &[u8; 16] = b"0123456789abcdef";
        for byte in bytes {
            text.push(HEX[(byte >> 4) as usize] as char);
            text.push(HEX[(byte & 0x0f) as usize] as char);
        }
        Ok(Self(text))
    }

    /// The token as the configuration stores it.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Writes the token of one agent and returns the file.
///
/// The file is private to the user: `0600` on the systems that have modes.
pub fn write(agent: &str, token: &Token) -> io::Result<PathBuf> {
    let folder = paths::agents_dir()
        .ok_or_else(|| io::Error::other("the data folder of Whatfast is unknown"))?;
    fs::create_dir_all(&folder)?;
    let path = folder.join(format!("{agent}.token"));
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    file.write_all(token.as_str().as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(path)
}

/// Deletes the token of one agent. A missing file is not an error.
pub fn revoke(agent: &str) -> io::Result<()> {
    let Some(folder) = paths::agents_dir() else {
        return Ok(());
    };
    match fs::remove_file(folder.join(format!("{agent}.token"))) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One variable of the environment, put back when the test ends.
    struct RestoreVar {
        name: &'static str,
        original: Option<std::ffi::OsString>,
    }

    impl RestoreVar {
        fn set(name: &'static str, value: &std::path::Path) -> Self {
            let original = std::env::var_os(name);
            // SAFETY: the test holds `ENV_LOCK` for its whole life, and no
            // other test of this crate reads this variable.
            unsafe { std::env::set_var(name, value) };
            Self { name, original }
        }
    }

    impl Drop for RestoreVar {
        fn drop(&mut self) {
            match self.original.take() {
                // SAFETY: the same lock the setter took is still held here.
                Some(value) => unsafe { std::env::set_var(self.name, value) },
                None => unsafe { std::env::remove_var(self.name) },
            }
        }
    }

    #[test]
    fn two_tokens_differ() {
        let first = Token::generate().expect("token");
        let second = Token::generate().expect("token");
        assert_eq!(first.as_str().len(), 64);
        assert_ne!(first, second);
        assert!(first.as_str().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn a_written_token_is_readable_and_revoke_erases_it() {
        use std::sync::Mutex;
        static ENV_LOCK: Mutex<()> = Mutex::new(());
        let _guard = ENV_LOCK.lock().expect("the lock of the environment");

        // The data folder is what the environment names: the test points it
        // at a folder of its own, so it never touches the tokens of the user.
        let dir = std::env::temp_dir().join(format!(
            "whatfast-token-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("the folder of the test");
        let _localappdata = RestoreVar::set("LOCALAPPDATA", &dir);
        #[cfg(not(windows))]
        let _xdg_data_home = RestoreVar::set("XDG_DATA_HOME", &dir);
        #[cfg(not(windows))]
        let _home = RestoreVar::set("HOME", &dir);

        let token = Token::generate().expect("the token");
        let path = write("roundtrip", &token).expect("the write");
        let text = std::fs::read_to_string(&path).expect("the token is readable");
        assert_eq!(text.trim(), token.as_str());

        revoke("roundtrip").expect("the revoke");
        assert!(!path.exists(), "the revoke erased the token file");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
