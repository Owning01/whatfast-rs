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

/// The token of one agent, when it exists.
pub fn read(agent: &str) -> io::Result<Option<Token>> {
    let Some(folder) = paths::agents_dir() else {
        return Ok(None);
    };
    let path = folder.join(format!("{agent}.token"));
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(Token(text.trim().to_string()))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
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

    #[test]
    fn two_tokens_differ() {
        let first = Token::generate().expect("token");
        let second = Token::generate().expect("token");
        assert_eq!(first.as_str().len(), 64);
        assert_ne!(first, second);
        assert!(first.as_str().chars().all(|c| c.is_ascii_hexdigit()));
    }
}
