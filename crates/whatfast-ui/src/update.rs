//! The self-update of an installed copy of Whatfast.
//!
//! The release workflow publishes, besides the packages, one manifest per
//! platform and the two executables that make up the app. The manifest names
//! the files and their SHA-256; this module reads it, compares its version
//! with the one compiled into this process, downloads the pair, verifies the
//! hashes and swaps the files when the user restarts.
//!
//! The swap renames the running executable instead of overwriting it: Windows
//! keeps the image of a process locked, but it allows the name to move, and
//! the new file lands on the free name. A swap that fails halfway leaves the
//! old files beside the new ones (`.old-<stamp>`), and the next start removes
//! them.
//!
//! The manifest is the one of the project: an attacker who can replace the
//! release can replace the manifest too, because there is no code signing.
//! The SHA-256 rules out a truncated or corrupted download, not a hostile
//! GitHub account.

use std::fs;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

/// The version of this build, the one the manifest is compared with.
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Where the latest release of the project lives.
///
/// `releases/latest/download/<name>` always answers with the asset of the
/// newest release, so the check is one request and no GitHub API quota.
const DEFAULT_BASE: &str = "https://github.com/Owning01/whatfast-rs/releases/latest/download";

/// The manifest of each platform this binary knows how to update itself.
const WINDOWS_MANIFEST: &str = "whatfast-windows-x86_64.json";
const LINUX_MANIFEST: &str = "whatfast-linux-x86_64.json";

/// The file a finished download leaves next to the two executables, so a
/// restart of the app does not throw the download away.
const READY: &str = "ready.json";

const USER_AGENT: &str = concat!("Whatfast/", env!("CARGO_PKG_VERSION"));

/// The largest manifest the check accepts: it is a handful of fields.
const MANIFEST_LIMIT: u64 = 64 * 1024;

/// The largest file the download accepts. The two executables are around
/// 30 MiB together; the ceiling stops a hostile server from filling the disk.
const ASSET_LIMIT: u64 = 256 * 1024 * 1024;

/// The deadline of the check and of each downloaded file.
const CHECK_TIMEOUT: Duration = Duration::from_secs(20);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// One file of the release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Asset {
    /// The name of the asset in the release, never a path.
    pub name: String,
    /// The SHA-256 of the bytes, in lower case hex.
    pub sha256: String,
}

/// The update of one platform: the version and the two executables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Manifest {
    pub version: String,
    pub app: Asset,
    pub backend: Asset,
}

/// What the thread of the update tells the interface.
#[derive(Clone, Debug)]
pub(crate) enum Event {
    /// A newer version is published.
    Found(Manifest),
    /// The published version is this one, or an older one.
    Current,
    /// The check or the download failed, with the reason.
    Failed(String),
    /// Bytes received and the size of the file being downloaded.
    Progress { received: u64, total: u64 },
    /// The two files are downloaded and verified; the path holds them.
    Downloaded(PathBuf),
}

/// The address of the releases, overridable for a local test server.
pub(crate) fn base_url() -> String {
    std::env::var("WHATFAST_UPDATE_URL")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_BASE.to_string())
}

/// The manifest of this platform, or `None` when the release has none.
///
/// macOS packs an `.app` and the other architectures have no build: those
/// copies update by hand.
pub(crate) fn platform_manifest() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some(WINDOWS_MANIFEST),
        ("linux", "x86_64") => Some(LINUX_MANIFEST),
        _ => None,
    }
}

/// Whether the interface may replace its own files in this run.
///
/// The escape hatch is for a build run from a folder that is not an
/// installation, and for a machine where the update has to be done by hand.
pub(crate) fn enabled() -> bool {
    std::env::var("WHATFAST_NO_UPDATE")
        .map(|value| !matches!(value.trim(), "1" | "true" | "yes"))
        .unwrap_or(true)
}

/// Asks the release for the manifest and says whether it is newer.
///
/// A platform without a published manifest answers "nothing new": the app
/// works, it just has no package to update itself with.
pub(crate) fn check(base: &str, manifest_name: &str) -> Result<Option<Manifest>, String> {
    let url = format!("{}/{manifest_name}", base.trim_end_matches('/'));
    let text = get_text(&url, MANIFEST_LIMIT, CHECK_TIMEOUT)?;
    let manifest = parse_manifest(&text)?;
    if is_newer(&manifest.version, VERSION) {
        Ok(Some(manifest))
    } else {
        Ok(None)
    }
}

/// Downloads the two files of the manifest into `version` of the staging
/// folder, checks their SHA-256 and leaves a `ready.json` beside them.
///
/// The files land under the name of the asset, so a repeated download
/// replaces the same paths.
pub(crate) fn download(
    manifest: &Manifest,
    base: &str,
    staging_root: &Path,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<PathBuf, String> {
    let folder = staging_root.join(&manifest.version);
    // A previous attempt of the same version is thrown away: a half file is
    // worse than no file.
    let _ = fs::remove_dir_all(&folder);
    fs::create_dir_all(&folder)
        .map_err(|error| format!("creating {}: {error}", folder.display()))?;
    let base = base.trim_end_matches('/');
    for asset in [&manifest.app, &manifest.backend] {
        let destination = folder.join(&asset.name);
        download_asset(base, asset, &destination, progress)?;
    }
    let ready = serde_json::json!({
        "version": manifest.version,
        "app": { "asset": manifest.app.name, "sha256": manifest.app.sha256 },
        "backend": { "asset": manifest.backend.name, "sha256": manifest.backend.sha256 },
    });
    let text = serde_json::to_string_pretty(&ready)
        .map_err(|error| format!("the manifest of the download: {error}"))?;
    fs::write(folder.join(READY), text)
        .map_err(|error| format!("writing the marker of the download: {error}"))?;
    Ok(folder)
}

/// The folder where a download waits for the restart.
pub(crate) fn staging_root() -> PathBuf {
    whatfast_model::paths::data_dir().join("update")
}

/// The download of a newer version that waits for a restart.
///
/// The interface offers it again after closing and opening the app: the
/// files are already on disk and the check is not repeated.
pub(crate) fn ready() -> Option<(Manifest, PathBuf)> {
    ready_in(&staging_root())
}

fn ready_in(root: &Path) -> Option<(Manifest, PathBuf)> {
    let mut best: Option<(Manifest, PathBuf)> = None;
    for entry in fs::read_dir(root).ok()?.flatten() {
        let folder = entry.path();
        if !folder.is_dir() {
            continue;
        }
        let Ok(text) = fs::read_to_string(folder.join(READY)) else {
            continue;
        };
        let Ok(manifest) = parse_manifest(&text) else {
            continue;
        };
        if !is_newer(&manifest.version, VERSION) {
            continue;
        }
        if !folder.join(&manifest.app.name).is_file()
            || !folder.join(&manifest.backend.name).is_file()
        {
            continue;
        }
        let better = best
            .as_ref()
            .is_none_or(|(current, _)| is_newer(&manifest.version, &current.version));
        if better {
            best = Some((manifest, folder));
        }
    }
    best
}

/// Replaces the two executables next to `exe` with the ones of `staging`.
///
/// The files are copied first, so a folder without write permission fails
/// before anything moves. The old names stay beside the new ones.
pub(crate) fn install(manifest: &Manifest, staging: &Path, exe: &Path) -> Result<PathBuf, String> {
    let folder = exe
        .parent()
        .ok_or_else(|| "the executable has no folder".to_string())?;
    let app_name = exe
        .file_name()
        .ok_or_else(|| "the executable has no name".to_string())?
        .to_string_lossy()
        .into_owned();
    let backend_name = if cfg!(windows) {
        "whatfast-backend.exe"
    } else {
        "whatfast-backend"
    };
    let app_source = staging.join(&manifest.app.name);
    let backend_source = staging.join(&manifest.backend.name);
    for source in [&app_source, &backend_source] {
        if !source.is_file() {
            return Err(format!("the update is missing {}", source.display()));
        }
    }

    // The copies are prepared under a dot name: they are not an executable
    // the user or an agent could run by accident.
    let app_new = folder.join(format!(".{app_name}.new"));
    let backend_new = folder.join(format!(".{backend_name}.new"));
    let app_target = folder.join(&app_name);
    let backend_target = folder.join(backend_name);
    prepare_copy(&app_source, &app_new)?;
    if let Err(error) = prepare_copy(&backend_source, &backend_new) {
        let _ = fs::remove_file(&app_new);
        return Err(error);
    }

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    let app_old = unique_old(&app_target, stamp);
    let backend_old = unique_old(&backend_target, stamp);

    // The interface goes first: if the backend cannot be replaced, the
    // interface is put back and nothing changes.
    replace(&app_target, &app_new, &app_old)?;
    if let Err(error) = replace(&backend_target, &backend_new, &backend_old) {
        let _ = fs::rename(&app_old, &app_target);
        return Err(error);
    }
    Ok(app_target)
}

/// Removes the files a finished update left around and the downloads that
/// are not waiting for a restart.
///
/// A file still held by another process (an agent server, for instance) is
/// skipped: the next start tries again.
pub(crate) fn cleanup() {
    if let Ok(exe) = std::env::current_exe()
        && let Some(folder) = exe.parent()
        && let Ok(entries) = fs::read_dir(folder)
    {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("whatfast") && name.contains(".old-") {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    prune_staging();
}

/// Keeps only the folder of the download that waits for a restart.
fn prune_staging() {
    let root = staging_root();
    let keep = ready().map(|(_, folder)| folder);
    let Ok(entries) = fs::read_dir(&root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if Some(&path) == keep.as_ref() {
            continue;
        }
        if path.is_dir() {
            let _ = fs::remove_dir_all(&path);
        } else {
            let _ = fs::remove_file(&path);
        }
    }
}

/// Whether the published version is newer than the one of this build.
pub(crate) fn is_newer(published: &str, current: &str) -> bool {
    let Some(published) = version_parts(published) else {
        return false;
    };
    let Some(current) = version_parts(current) else {
        return false;
    };
    let length = published.len().max(current.len());
    for index in 0..length {
        let published = published.get(index).copied().unwrap_or(0);
        let current = current.get(index).copied().unwrap_or(0);
        if published != current {
            return published > current;
        }
    }
    false
}

/// The numbers of a dotted version; a suffix after a `-` is not a release.
fn version_parts(text: &str) -> Option<Vec<u64>> {
    let text = text.trim().trim_start_matches('v');
    if text.is_empty() || text.contains('-') {
        return None;
    }
    text.split('.').map(|part| part.parse::<u64>().ok()).collect()
}

/// Reads one manifest and checks the fields the download needs.
pub(crate) fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|error| format!("the manifest is not JSON: {error}"))?;
    let version = value
        .get("version")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| version_parts(value).is_some())
        .ok_or_else(|| "the manifest has no version".to_string())?;
    let app = parse_asset(value.get("app")).map_err(|error| format!("app: {error}"))?;
    let backend = parse_asset(value.get("backend")).map_err(|error| format!("backend: {error}"))?;
    Ok(Manifest {
        version: version.trim_start_matches('v').to_string(),
        app,
        backend,
    })
}

fn parse_asset(value: Option<&serde_json::Value>) -> Result<Asset, String> {
    let value = value.ok_or_else(|| "missing".to_string())?;
    let name = value
        .get("asset")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|name| safe_name(name))
        .ok_or_else(|| "missing or unsafe name".to_string())?;
    let sha256 = value
        .get("sha256")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .filter(|hash| hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()))
        .ok_or_else(|| "missing or invalid sha256".to_string())?;
    Ok(Asset {
        name: name.to_string(),
        sha256,
    })
}

/// A name of an asset, never a path.
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && !name.starts_with('.')
}

fn get_text(url: &str, limit: u64, timeout: Duration) -> Result<String, String> {
    let response = build_agent(timeout)
        .get(url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|error| format!("{url}: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("{url}: HTTP {}", response.status()));
    }
    let body = response
        .into_body()
        .into_with_config()
        .limit(limit)
        .read_to_string()
        .map_err(|error| format!("{url}: {error}"))?;
    Ok(body)
}

fn build_agent(timeout: Duration) -> ureq::Agent {
    let config = ureq::config::Config::builder()
        .timeout_global(Some(timeout))
        // A 404 or a 500 is answered here: the status names the failure
        // better than the error of the library.
        .http_status_as_error(false)
        .build();
    config.into()
}

fn download_asset(
    base: &str,
    asset: &Asset,
    destination: &Path,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<(), String> {
    let url = format!("{base}/{}", asset.name);
    let mut response = build_agent(DOWNLOAD_TIMEOUT)
        .get(&url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|error| format!("{url}: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("{url}: HTTP {}", response.status()));
    }
    let total = response.body().content_length().unwrap_or(0);
    if total > ASSET_LIMIT {
        return Err(format!("{url}: the file is too large"));
    }
    let partial = partial_path(destination);
    let mut file = fs::File::create(&partial)
        .map_err(|error| format!("creating {}: {error}", partial.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut received = 0u64;
    let mut reader = response.body_mut().as_reader();
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| format!("{url}: {error}"))?;
        if read == 0 {
            break;
        }
        received += read as u64;
        if received > ASSET_LIMIT {
            let _ = fs::remove_file(&partial);
            return Err(format!("{url}: the file is too large"));
        }
        hasher.update(&buffer[..read]);
        file.write_all(&buffer[..read])
            .map_err(|error| format!("writing {}: {error}", partial.display()))?;
        progress(received, total);
    }
    file.sync_all()
        .map_err(|error| format!("flushing {}: {error}", partial.display()))?;
    drop(file);
    let digest = hex(&hasher.finalize());
    if digest != asset.sha256 {
        let _ = fs::remove_file(&partial);
        return Err(format!(
            "{}: the SHA-256 is {digest}, not {}",
            asset.name, asset.sha256
        ));
    }
    fs::rename(&partial, destination)
        .map_err(|error| format!("renaming {}: {error}", partial.display()))?;
    Ok(())
}

fn partial_path(destination: &Path) -> PathBuf {
    let name = destination
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "download".to_string());
    destination.with_file_name(format!("{name}.part"))
}

fn prepare_copy(source: &Path, destination: &Path) -> Result<(), String> {
    fs::copy(source, destination).map_err(|error| format!("copying {}: {error}", source.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = fs::set_permissions(destination, fs::Permissions::from_mode(0o755));
    }
    Ok(())
}

/// Moves the target aside and puts the prepared file in its place. A target
/// that does not exist (an install without the backend, for instance) is
/// not an error.
fn replace(target: &Path, prepared: &Path, old: &Path) -> Result<(), String> {
    if target.exists() {
        fs::rename(target, old).map_err(|error| {
            format!(
                "moving {} aside: {error}. The folder may need a permission.",
                target.display()
            )
        })?;
    }
    if let Err(error) = fs::rename(prepared, target) {
        if old.exists() {
            let _ = fs::rename(old, target);
        }
        return Err(format!("installing {}: {error}", target.display()));
    }
    Ok(())
}

fn unique_old(target: &Path, stamp: u64) -> PathBuf {
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "whatfast".to_string());
    let folder = target.parent().unwrap_or_else(|| Path::new("."));
    let old = folder.join(format!("{name}.old-{stamp}"));
    let _ = fs::remove_file(&old);
    old
}

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str, hash: char) -> Asset {
        Asset {
            name: name.to_string(),
            sha256: format!("{hash}").repeat(64),
        }
    }

    #[test]
    fn the_versions_are_compared_number_by_number() {
        assert!(is_newer("0.2.30", "0.2.29"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("v0.3", "0.2.29"));
        assert!(!is_newer("0.2.29", "0.2.29"));
        assert!(!is_newer("0.2.28", "0.2.29"));
        // A suffix is not a released version: it never wins.
        assert!(!is_newer("0.2.30-beta", "0.2.29"));
        assert!(!is_newer("", "0.2.29"));
    }

    #[test]
    fn the_manifest_reads_its_two_assets() {
        let text = format!(
            r#"{{
                "version": "0.2.30",
                "app": {{ "asset": "whatfast-windows-x86_64.exe", "sha256": "{}" }},
                "backend": {{ "asset": "whatfast-windows-x86_64-backend.exe", "sha256": "{}" }}
            }}"#,
            "a".repeat(64),
            "b".repeat(64)
        );
        let parsed = parse_manifest(&text).expect("the manifest parses");
        assert_eq!(parsed.version, "0.2.30");
        assert_eq!(parsed.app.name, "whatfast-windows-x86_64.exe");
        assert_eq!(parsed.backend.sha256, "b".repeat(64));
    }

    #[test]
    fn a_manifest_without_the_fields_is_refused() {
        assert!(parse_manifest("{}").is_err());
        assert!(parse_manifest("not json").is_err());
        assert!(parse_manifest(r#"{"version":"0.2.30"}"#).is_err());
        assert!(
            parse_manifest(&format!(
                r#"{{"version":"0.2.30","app":{{"asset":"../a","sha256":"{}"}},"backend":{{"asset":"b","sha256":"{}"}}}}"#,
                "a".repeat(64),
                "b".repeat(64)
            ))
            .is_err()
        );
        // A short hash is a typo or a truncated manifest, never a match.
        assert!(
            parse_manifest(&format!(
                r#"{{"version":"0.2.30","app":{{"asset":"a.exe","sha256":"ab"}},"backend":{{"asset":"b.exe","sha256":"{}"}}}}"#,
                "b".repeat(64)
            ))
            .is_err()
        );
    }

    #[test]
    fn an_asset_name_is_never_a_path() {
        assert!(safe_name("whatfast-linux-x86_64"));
        assert!(safe_name("whatfast.exe"));
        assert!(!safe_name("../whatfast.exe"));
        assert!(!safe_name("folder/whatfast.exe"));
        assert!(!safe_name(""));
        assert!(!safe_name(".hidden"));
    }

    #[test]
    fn the_install_replaces_the_pair_and_keeps_the_old_names() {
        let directory = tempfile::tempdir().unwrap();
        let folder = directory.path();
        let app = folder.join("whatfast.exe");
        let backend = folder.join("whatfast-backend.exe");
        let staging = folder.join("staging");
        fs::create_dir_all(&staging).unwrap();
        fs::write(&app, b"old app").unwrap();
        fs::write(&backend, b"old backend").unwrap();
        fs::write(staging.join("new-app.exe"), b"new app").unwrap();
        fs::write(staging.join("new-backend.exe"), b"new backend").unwrap();
        let manifest = Manifest {
            version: "9.9.9".into(),
            app: asset("new-app.exe", 'a'),
            backend: asset("new-backend.exe", 'b'),
        };
        let installed = install(&manifest, &staging, &app).expect("the swap works");
        assert_eq!(installed, app);
        assert_eq!(fs::read(&app).unwrap(), b"new app");
        assert_eq!(fs::read(&backend).unwrap(), b"new backend");
        let olds: Vec<_> = fs::read_dir(folder)
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().contains(".old-"))
            .collect();
        assert_eq!(olds.len(), 2, "the two old files stay beside the new ones");
        for entry in olds {
            let _ = fs::remove_file(entry.path());
        }
    }

    #[test]
    fn a_missing_staged_file_changes_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let folder = directory.path();
        let app = folder.join("whatfast.exe");
        fs::write(&app, b"old app").unwrap();
        let staging = folder.join("staging");
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("new-app.exe"), b"new app").unwrap();
        let manifest = Manifest {
            version: "9.9.9".into(),
            app: asset("new-app.exe", 'a'),
            backend: asset("missing.exe", 'b'),
        };
        assert!(install(&manifest, &staging, &app).is_err());
        assert_eq!(fs::read(&app).unwrap(), b"old app");
    }

    #[test]
    fn a_ready_download_is_found_and_an_incomplete_one_is_not() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let complete = root.join("9.9.9");
        fs::create_dir_all(&complete).unwrap();
        fs::write(complete.join("app.exe"), b"app").unwrap();
        fs::write(complete.join("backend.exe"), b"backend").unwrap();
        fs::write(
            complete.join(READY),
            format!(
                r#"{{"version":"9.9.9","app":{{"asset":"app.exe","sha256":"{}"}},"backend":{{"asset":"backend.exe","sha256":"{}"}}}}"#,
                "a".repeat(64),
                "b".repeat(64)
            ),
        )
        .unwrap();
        let partial = root.join("8.8.8");
        fs::create_dir_all(&partial).unwrap();
        fs::write(partial.join("app.exe"), b"app").unwrap();
        let (manifest, folder) = ready_in(root).expect("the finished download is found");
        assert_eq!(manifest.version, "9.9.9");
        assert_eq!(folder, complete);
        // The marker of a version that is not newer is ignored.
        fs::write(complete.join(READY), r#"{"version":"0.0.1"}"#).unwrap();
        assert!(ready_in(root).is_none());
    }

    #[test]
    fn the_sha_of_the_download_is_the_one_of_the_manifest() {
        let bytes = b"the bytes of a release";
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        assert_eq!(
            hex(&hasher.finalize()),
            "7195564326d5fb8828ee76618a81aa18e08e399753b244bbb96a7f2a469f3708"
        );
    }
}
