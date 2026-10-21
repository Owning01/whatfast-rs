//! The lock that keeps a single instance of the app.
//!
//! Two instances would mean two sidecars with the same device session: the
//! server would see two connections for the same phone and the session could
//! be invalidated. The web client prevents it with a plugin; here it is a
//! lock file with the process that holds it.
//!
//! The lock is taken before the sidecar starts, and it is released when the
//! app goes away. A lock left by a crash does not block the next start: the
//! process that wrote it is looked for, and if it is not alive the lock is
//! stale and gets replaced.

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// The name of the lock file inside the profile of the user.
const LOCK_FILE: &str = "whatfast.lock";

/// The lock of the single instance.
///
/// The lock is released when the value goes out of scope.
#[derive(Debug)]
pub(crate) struct InstanceLock {
    path: PathBuf,
}

impl InstanceLock {
    /// Takes the lock of the app.
    ///
    /// Returns `None` when another instance is already running.
    pub(crate) fn acquire(profile: &Path) -> Option<Self> {
        let path = profile.join(LOCK_FILE);
        if let Some(owner) = read_owner(&path)
            && owner != std::process::id()
            && is_alive(owner)
        {
            return None;
        }
        // A lock left by a process that is not running is stale: it is
        // replaced.
        if path.exists() {
            let _ = fs::remove_file(&path);
        }
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        // The lock is taken with a file that has to be new: the creation is
        // atomic, so two instances can not take it at the same time.
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .ok()?;
        let _ = writeln!(file, "{}", std::process::id());
        let _ = file.sync_all();
        Some(Self { path })
    }

    /// The path of the lock file.
    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// The process that wrote the lock, if it is written.
fn read_owner(path: &Path) -> Option<u32> {
    let text = fs::read_to_string(path).ok()?;
    text.trim().parse().ok()
}

/// Whether a process is still running.
fn is_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        // Without a dependency: the list of the tasks of the system answers
        // if the process is there. It only runs when the lock exists, that
        // is, when another instance could be running.
        std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .map(|output| {
                let text = String::from_utf8_lossy(&output.stdout);
                text.contains(&format!("{pid}")) && !text.contains("INFO: No tasks")
            })
            .unwrap_or(false)
    }
    #[cfg(unix)]
    {
        Path::new(&format!("/proc/{pid}")).exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lock is taken once and released when it goes out of scope.
    #[test]
    fn the_lock_is_taken_and_released() {
        let directory = tempfile::tempdir().unwrap();
        let profile = directory.path().join("profile");
        {
            let lock = InstanceLock::acquire(&profile).expect("the lock is free");
            assert!(lock.path().exists());
        }
        assert!(!profile.join(LOCK_FILE).exists(), "the lock is released");
    }

    /// Another instance that is running keeps the app from starting.
    #[test]
    fn a_lock_of_a_running_process_stops_the_second_instance() {
        let directory = tempfile::tempdir().unwrap();
        let profile = directory.path().join("profile");
        fs::create_dir_all(&profile).unwrap();
        // The process 4 is the system on Windows and the init on Linux: both
        // are always running.
        let owner = if cfg!(windows) { 4 } else { 1 };
        fs::write(profile.join(LOCK_FILE), format!("{owner}\n")).unwrap();
        assert!(
            InstanceLock::acquire(&profile).is_none(),
            "the app is already running"
        );
    }

    /// A lock left by a process that is not running does not block the app.
    #[test]
    fn a_stale_lock_does_not_block_the_app() {
        let directory = tempfile::tempdir().unwrap();
        let profile = directory.path().join("profile");
        fs::create_dir_all(&profile).unwrap();
        fs::write(profile.join(LOCK_FILE), "4294967294\n").unwrap();
        assert!(
            InstanceLock::acquire(&profile).is_some(),
            "a lock of a dead process is stale"
        );
    }
}
