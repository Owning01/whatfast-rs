//! The local endpoint the bridge listens on for clients that are not its
//! parent.
//!
//! The desktop owns the backend through stdin and stdout. An agent harness is
//! a different process, started by another program: it cannot inherit those
//! pipes, so it reaches the same backend through a local endpoint derived from
//! the profile directory. Windows gives every profile a named pipe; the
//! systems with Unix sockets give it a file inside the profile tree, as
//! private as the rest of it.
//!
//! The endpoint is derived, never configured: the backend and every client
//! that resolves the same data directory meet on the same name, and two
//! profiles never share one.

use std::path::Path;

/// The name of the local endpoint of one profile.
///
/// A Windows named pipe is a single path component, so the profile directory
/// is folded into a fingerprint. On the other systems the endpoint is a socket
/// file inside the profile directory.
pub fn endpoint(data_dir: &Path) -> String {
    #[cfg(windows)]
    {
        format!(r"\\.\pipe\whatfast-{:016x}", fingerprint(data_dir))
    }
    #[cfg(not(windows))]
    {
        data_dir.join("agent.sock").to_string_lossy().into_owned()
    }
}

/// FNV-1a over the profile path, frames lowered the way Windows compares them.
///
/// The hash is not a security boundary: it exists so two data directories
/// never collide on one pipe name without carrying the whole path.
fn fingerprint(data_dir: &Path) -> u64 {
    let text = data_dir.to_string_lossy().to_lowercase();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

