//! The MCP server of Whatfast.
//!
//! An agent harness (Claude Code, Codex, Cursor, OpenCode, Pi, Hermes) starts
//! this process as its MCP server and speaks JSON-RPC over stdio. Every tool
//! call becomes one request on the local bridge; the backend answers it with
//! the same envelopes the desktop uses. The agent never touches the WhatsApp
//! session: it only sees chats and messages.
//!
//! The process is short-lived by design: the harness owns it and starts it
//! again for the next session. When no Whatfast backend is listening, this
//! server starts one (`whatfast-backend --serve`) and lets it go with an idle
//! timeout, so an agent works with the window closed.

pub mod client;
mod framing;
mod server;
mod time;
pub mod tools;

use std::path::PathBuf;

/// How long a backend this process started stays alive after the last call.
const IDLE_EXIT: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// What the command line of the server says.
#[derive(Clone, Debug)]
pub struct Options {
    /// The agent this connection belongs to: `claude-code`, `codex`. It names
    /// the grants and, later, the log of the agent.
    pub agent: String,
    /// The profile directory: the same one the desktop resolves.
    pub data_dir: PathBuf,
    /// The executable of the backend, when the server has to start one.
    pub backend: PathBuf,
}

/// Runs the server over the process streams and returns the exit code.
///
/// The arguments arrive without the program name: `mcp --agent <id>
/// [--data-dir <path>] [--backend <path>]`.
pub fn run<I>(args: I) -> i32
where
    I: IntoIterator<Item = String>,
{
    match parse(args) {
        Ok(Some(options)) => server::serve(&options),
        Ok(None) => 0,
        Err(message) => {
            eprintln!("whatfast mcp: {message}");
            2
        }
    }
}

/// Reads the command line. `Ok(None)` means it only asked for something that
/// was already printed, like the version.
pub fn parse<I>(args: I) -> Result<Option<Options>, String>
where
    I: IntoIterator<Item = String>,
{
    let mut agent = None;
    let mut data_dir = None;
    let mut backend = None;
    let mut args = args.into_iter();
    while let Some(argument) = args.next() {
        let (flag, inline) = match argument.split_once('=') {
            Some((flag, value)) => (flag.to_owned(), Some(value.to_owned())),
            None => (argument, None),
        };
        match flag.as_str() {
            "--agent" => agent = Some(value(&mut args, inline, "--agent")?),
            "--data-dir" => data_dir = Some(PathBuf::from(value(&mut args, inline, "--data-dir")?)),
            "--backend" => backend = Some(PathBuf::from(value(&mut args, inline, "--backend")?)),
            "--version" => {
                println!("whatfast mcp {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(None);
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let Some(agent) = agent else {
        return Err("--agent is required".into());
    };
    let data_dir = match data_dir {
        Some(path) => path,
        None => default_data_dir().ok_or_else(|| {
            "the profile directory could not be resolved: pass --data-dir".to_owned()
        })?,
    };
    let backend = match backend {
        Some(path) => path,
        None => default_backend(&agent).ok_or_else(|| {
            "the backend executable was not found next to the application: pass --backend"
                .to_owned()
        })?,
    };
    Ok(Some(Options {
        agent,
        data_dir,
        backend,
    }))
}

/// What a missing argument prints.
pub const USAGE: &str = "usage: whatfast mcp --agent <id> [--data-dir <path>] [--backend <path>]";

fn value(
    args: &mut impl Iterator<Item = String>,
    inline: Option<String>,
    flag: &str,
) -> Result<String, String> {
    match inline {
        Some(value) => Ok(value),
        None => args.next().ok_or_else(|| format!("{flag} needs a value")),
    }
}

/// The profile directory of the desktop, resolved the same way it does:
/// `WHATFAST_DATA_DIR` first, then the platform folder.
pub fn default_data_dir() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("WHATFAST_DATA_DIR").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(path));
    }
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA")
            .filter(|value| !value.is_empty())
            .map(|base| PathBuf::from(base).join("whatfast"))
    }
    #[cfg(target_os = "macos")]
    {
        home().map(|home| home.join("Library/Application Support/whatfast"))
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .filter(|value| !value.is_empty())
            .map(|base| PathBuf::from(base).join("whatfast"))
            .or_else(|| home().map(|home| home.join(".local/share/whatfast")))
    }
}

#[cfg(not(windows))]
fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// The backend this application ships: the same name the desktop looks for.
pub fn default_backend(_agent: &str) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("WHATFAST_BACKEND").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(path));
    }
    let current = std::env::current_exe().ok()?;
    let name = if cfg!(windows) {
        "whatfast-backend.exe"
    } else {
        "whatfast-backend"
    };
    let candidate = current
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join(name);
    candidate.exists().then_some(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_agent_is_required() {
        assert!(parse(["--data-dir".to_owned(), "/tmp/w".to_owned()]).is_err());
    }

    #[test]
    fn every_part_of_the_command_line_parses() {
        let options = parse([
            "--agent=codex".to_owned(),
            "--data-dir".to_owned(),
            r"C:\profile".to_owned(),
            "--backend".to_owned(),
            r"C:\backend.exe".to_owned(),
        ])
        .expect("command line")
        .expect("options");
        assert_eq!(options.agent, "codex");
        assert_eq!(options.data_dir, PathBuf::from(r"C:\profile"));
        assert_eq!(options.backend, PathBuf::from(r"C:\backend.exe"));
    }

    #[test]
    fn the_version_prints_and_stops() {
        assert!(parse(["--version".to_owned()]).expect("version").is_none());
    }
}
