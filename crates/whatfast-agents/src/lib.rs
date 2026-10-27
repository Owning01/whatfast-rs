//! The catalog of agent harnesses that Whatfast can connect.
//!
//! One [`ClientSpec`] per harness says four things: how the harness is
//! detected on this machine, how the MCP server is registered in it, how the
//! harness runs a prompt without a terminal, and what to tell the user after
//! the change.
//!
//! The registration is not the same everywhere. Claude Code, OpenCode, Pi and
//! VS Code take a command line. Codex takes a TOML table, Hermes a YAML
//! mapping, Cursor and Claude Desktop a JSON object. Cursor also accepts a
//! deep link. The catalog keeps those differences in the spec, so the
//! interface never learns them.
//!
//! ```ignore
//! let spec = whatfast_agents::by_id("codex").unwrap();
//! if whatfast_agents::detect::installed(spec) {
//!     let cmd = whatfast_agents::McpCommand::new("C:\\Program Files\\Whatfast\\whatfast.exe", "codex");
//!     whatfast_agents::write::register(spec, &cmd)?;
//! }
//! ```

pub mod detect;
pub mod guide;
pub mod paths;
pub mod spec;
pub mod token;
pub mod write;

pub use spec::{ClientSpec, Detect, Location, Method, RunSpec, all, by_id};

/// The command a harness runs to start the MCP server of Whatfast.
///
/// Every client registers the same three parts: the installed executable, the
/// `mcp` subcommand and the agent id that names the grants and the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpCommand {
    /// The path of `whatfast.exe` on this machine.
    pub exe: std::path::PathBuf,
    /// The agent this connection belongs to: `codex`, `cursor`.
    pub agent: String,
}

impl McpCommand {
    /// Builds the command for one agent.
    pub fn new(exe: impl Into<std::path::PathBuf>, agent: impl Into<String>) -> Self {
        Self {
            exe: exe.into(),
            agent: agent.into(),
        }
    }

    /// The argument list of the server, without the executable.
    pub fn args(&self) -> Vec<String> {
        vec!["mcp".to_string(), "--agent".to_string(), self.agent.clone()]
    }

    /// The executable as the string a config file stores.
    pub fn exe_string(&self) -> String {
        self.exe.to_string_lossy().to_string()
    }
}

/// One thing a registration did, for the card in the interface.
#[derive(Clone, Debug)]
pub struct Report {
    /// How the client was registered: `cli`, `json`, `toml`, `yaml`, `link`.
    pub method: &'static str,
    /// The file or the command that changed.
    pub target: String,
    /// The copy taken before the change, when the write touched a file.
    pub backup: Option<std::path::PathBuf>,
}
