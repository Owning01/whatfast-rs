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
//! stale and gets replaced. The lock also writes the name of the program,
//! because Windows gives the numbers of the processes back to other
//! programs: without the name, a lock left behind by a killed app could
//! block the app forever.

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
            && owner.pid != std::process::id()
            && is_alive(&owner)
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
        let _ = writeln!(file, "{} {}", std::process::id(), program_name());
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
struct Owner {
    /// The number of the process.
    pid: u32,
    /// The name of its program: an empty name is a lock of an older version,
    /// and then the number of the process is all there is.
    program: String,
}

/// The process that wrote the lock, if it is written.
fn read_owner(path: &Path) -> Option<Owner> {
    let text = fs::read_to_string(path).ok()?;
    let mut parts = text.split_whitespace();
    let pid = parts.next()?.parse().ok()?;
    let program = parts.next().unwrap_or_default().to_string();
    Some(Owner { pid, program })
}

/// The name of the program of this process.
fn program_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_default()
}

/// Whether the process of the lock is still running.
///
/// The number of a process can be given to another program when the first
/// one goes away, so the name of the program is checked too.
#[cfg(windows)]
fn is_alive(owner: &Owner) -> bool {
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, GetLastError, STILL_ACTIVE,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    // SAFETY: the handle that the call returns is checked and closed right
    // away, and the code of the exit is only read while the handle is open.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, owner.pid);
        if handle.is_null() {
            // A process that is there but that can not be opened is still
            // running: the system protects it, or it belongs to another
            // user. It only counts as gone when it does not exist.
            return GetLastError() == ERROR_ACCESS_DENIED;
        }
        let mut codigo = 0u32;
        let leido = GetExitCodeProcess(handle, &mut codigo) != 0;
        let nombre = program_of(handle);
        CloseHandle(handle);
        if !leido || codigo != STILL_ACTIVE as u32 {
            return false;
        }
        // A lock that does not name the program is taken as it is: a lock of
        // an older version of the app only has the number of the process.
        match (nombre, owner.program.is_empty()) {
            (Some(nombre), false) => nombre.eq_ignore_ascii_case(&owner.program),
            _ => true,
        }
    }
}

/// The name of the program of a process.
///
/// Returns `None` when the path of the process can not be read.
#[cfg(windows)]
fn program_of(handle: windows_sys::Win32::Foundation::HANDLE) -> Option<String> {
    use windows_sys::Win32::System::Threading::QueryFullProcessImageNameW;
    let mut buffer = [0u16; 260];
    let mut length = buffer.len() as u32;
    // SAFETY: the buffer is wide enough for the path that the call writes,
    // and the length is only used when the call says it wrote something.
    let leido =
        unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut length) != 0 };
    if !leido {
        return None;
    }
    let ruta = String::from_utf16_lossy(&buffer[..length as usize]);
    let nombre = ruta
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or_default()
        .to_string();
    Some(nombre)
}

/// Whether the process of the lock is still running.
#[cfg(unix)]
fn is_alive(owner: &Owner) -> bool {
    Path::new(&format!("/proc/{}", owner.pid)).exists()
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

    /// A lock whose number was given to another program does not block the
    /// app.
    ///
    /// Windows gives the numbers of the processes back to other programs: a
    /// lock left by an app that was killed can block the app forever if only
    /// the number is checked.
    #[cfg(windows)]
    #[test]
    fn a_lock_of_another_program_does_not_block_the_app() {
        // El proceso del test es otro programa: con su numero y el nombre de
        // Whatfast, el lock esta vencido.
        let otro = Owner {
            pid: std::process::id(),
            program: "whatfast.exe".to_string(),
        };
        assert!(
            !is_alive(&otro),
            "el proceso que tiene el numero no es Whatfast"
        );
        // Con el nombre del programa, el lock sigue vivo.
        let mismo = Owner {
            pid: std::process::id(),
            program: program_name(),
        };
        assert!(is_alive(&mismo), "el proceso es el mismo programa");
    }
}
