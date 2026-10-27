//! One spec per harness, and the table the interface reads.

use crate::McpCommand;
use crate::paths::Os;

/// Where a configuration file lives on each system.
///
/// A system without a path means the client does not exist there. The paths
/// accept `~`, `%APPDATA%`, `%USERPROFILE%`, `$HOME` and `$XDG_CONFIG_HOME`;
/// [`crate::paths::expand`] resolves them.
#[derive(Clone, Copy, Debug)]
pub struct Location {
    /// The path on Windows.
    pub windows: Option<&'static str>,
    /// The path on macOS.
    pub macos: Option<&'static str>,
    /// The path on Linux.
    pub linux: Option<&'static str>,
}

impl Location {
    /// The path for one system.
    pub fn for_os(&self, os: Os) -> Option<&'static str> {
        match os {
            Os::Windows => self.windows,
            Os::Macos => self.macos,
            Os::Linux => self.linux,
        }
    }
}

/// A file at the same path on every system, under the home directory.
const fn home(path: &'static str) -> Location {
    Location {
        windows: Some(path),
        macos: Some(path),
        linux: Some(path),
    }
}

/// How the harness is found on this machine.
#[derive(Clone, Copy, Debug)]
pub enum Detect {
    /// A program on `PATH`.
    Bin(&'static str),
    /// A configuration file or a folder.
    File(Location),
    /// Either of the two.
    BinOrFile(&'static str, Location),
}

/// How the MCP server of Whatfast is registered in the client.
#[derive(Clone, Copy, Debug)]
pub enum Method {
    /// The client has a command that edits its own configuration.
    ///
    /// `args` runs up to the server command: the registration appends `--`
    /// and the server command, except for VS Code, which takes the whole
    /// definition as JSON in one argument. That case is `Method::JsonArg`.
    Cli {
        /// The program to run.
        bin: &'static str,
        /// The arguments before the server command.
        args: &'static [&'static str],
    },
    /// The client takes the definition as one JSON argument.
    JsonArg {
        /// The program to run.
        bin: &'static str,
        /// The flag that carries the definition.
        flag: &'static str,
    },
    /// A JSON file with an object of servers.
    Json {
        /// The file to change.
        file: Location,
        /// The key that holds the servers.
        key: &'static str,
    },
    /// The JSONC file of OpenCode, which keeps its servers under `mcp.servers`
    /// with the command as an array and no arguments of its own.
    ///
    /// It is not `Json` with another key: the entry has a shape of its own and
    /// the file admits comments and trailing commas, so it is read before it is
    /// parsed instead of through a plain JSON parser.
    OpenCode {
        /// The file to change.
        file: Location,
    },
    /// A TOML file with one table per server.
    Toml {
        /// The file to change.
        file: Location,
        /// The table of this server, without brackets.
        section: &'static str,
    },
    /// A YAML file with one mapping per server.
    Yaml {
        /// The file to change.
        file: Location,
        /// The key that holds the servers.
        key: &'static str,
    },
}

/// The way the harness runs one prompt without a terminal.
///
/// The field is `None` for the clients that only live inside a window: they
/// read and write through MCP, but Whatfast cannot ask them for a draft.
#[derive(Clone, Copy, Debug)]
pub struct RunSpec {
    /// The program to run.
    pub bin: &'static str,
    /// The arguments that put it in print mode. The prompt goes to stdin.
    pub print_args: &'static [&'static str],
}

/// One agent harness, as the interface and the connector see it.
#[derive(Clone, Copy, Debug)]
pub struct ClientSpec {
    /// The stable id: it names the agent, its token and its grants.
    pub id: &'static str,
    /// The name the card shows.
    pub name: &'static str,
    /// How the harness is found.
    pub detect: Detect,
    /// How the server is registered.
    pub method: Method,
    /// How the harness answers a prompt, when it can.
    pub run: Option<RunSpec>,
    /// What the user must do after the change.
    pub restart: &'static str,
}

/// Every harness this build knows, in the order the screen lists them.
pub const CLIENTS: &[ClientSpec] = &[
    ClientSpec {
        id: "claude-code",
        name: "Claude Code",
        detect: Detect::Bin("claude"),
        method: Method::Cli {
            bin: "claude",
            args: &["mcp", "add", "--scope", "user", "whatfast", "--"],
        },
        run: Some(RunSpec {
            bin: "claude",
            print_args: &["-p"],
        }),
        restart: "Reiniciá la sesión de Claude Code para que tome el servidor.",
    },
    ClientSpec {
        id: "codex",
        name: "Codex",
        detect: Detect::BinOrFile("codex", home("~/.codex/config.toml")),
        method: Method::Toml {
            file: home("~/.codex/config.toml"),
            section: "mcp_servers.whatfast",
        },
        run: Some(RunSpec {
            bin: "codex",
            print_args: &["exec"],
        }),
        restart: "Reiniciá Codex para que tome el servidor.",
    },
    ClientSpec {
        id: "opencode",
        name: "OpenCode",
        detect: Detect::Bin("opencode"),
        method: Method::OpenCode {
            file: home("~/.config/opencode/opencode.jsonc"),
        },
        run: Some(RunSpec {
            bin: "opencode",
            print_args: &["run"],
        }),
        restart: "Reiniciá OpenCode para que tome el servidor.",
    },
    ClientSpec {
        id: "cursor",
        name: "Cursor",
        detect: Detect::File(home("~/.cursor/mcp.json")),
        method: Method::Json {
            file: home("~/.cursor/mcp.json"),
            key: "mcpServers",
        },
        run: None,
        restart: "Reiniciá Cursor para que tome el servidor.",
    },
    ClientSpec {
        id: "claude-desktop",
        name: "Claude Desktop",
        detect: Detect::File(Location {
            windows: Some("%APPDATA%/Claude/claude_desktop_config.json"),
            macos: Some("~/Library/Application Support/Claude/claude_desktop_config.json"),
            linux: Some("$XDG_CONFIG_HOME/Claude/claude_desktop_config.json"),
        }),
        method: Method::Json {
            file: Location {
                windows: Some("%APPDATA%/Claude/claude_desktop_config.json"),
                macos: Some("~/Library/Application Support/Claude/claude_desktop_config.json"),
                linux: Some("$XDG_CONFIG_HOME/Claude/claude_desktop_config.json"),
            },
            key: "mcpServers",
        },
        run: None,
        restart: "Cerrá y abrí Claude Desktop para que tome el servidor.",
    },
    ClientSpec {
        id: "vscode",
        name: "VS Code",
        detect: Detect::Bin("code"),
        method: Method::JsonArg {
            bin: "code",
            flag: "--add-mcp",
        },
        run: None,
        restart: "Reiniciá la ventana de VS Code para que tome el servidor.",
    },
    ClientSpec {
        id: "pi",
        name: "Pi",
        detect: Detect::Bin("pi"),
        method: Method::Cli {
            bin: "pi",
            args: &["mcp", "add", "whatfast", "--"],
        },
        run: Some(RunSpec {
            bin: "pi",
            print_args: &["--print"],
        }),
        restart: "Reiniciá Pi para que tome el servidor.",
    },
    ClientSpec {
        id: "hermes",
        name: "Hermes",
        detect: Detect::BinOrFile("hermes", home("~/.hermes/config.yaml")),
        method: Method::Yaml {
            file: home("~/.hermes/config.yaml"),
            key: "mcp_servers",
        },
        run: None,
        restart: "Reiniciá Hermes o corré /reload-mcp para que tome el servidor.",
    },
];

/// Every spec, in the order of the screen.
pub fn all() -> &'static [ClientSpec] {
    CLIENTS
}

/// The spec with this id.
pub fn by_id(id: &str) -> Option<&'static ClientSpec> {
    CLIENTS.iter().find(|spec| spec.id == id)
}

impl ClientSpec {
    /// The name of the server inside the client. It is the same everywhere,
    /// so the user sees one entry and `disconnect` finds it.
    pub const SERVER: &'static str = "whatfast";

    /// The argument vector of the server for this agent.
    pub fn server_argv(&self, cmd: &McpCommand) -> Vec<String> {
        let mut argv = vec![cmd.exe_string()];
        argv.extend(cmd.args());
        argv
    }

    /// The definition as the clients that take JSON show it.
    pub fn server_json(&self, cmd: &McpCommand) -> serde_json::Value {
        serde_json::json!({
            "command": cmd.exe_string(),
            "args": cmd.args(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_id_is_unique() {
        let mut ids: Vec<&str> = all().iter().map(|spec| spec.id).collect();
        ids.sort_unstable();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count);
    }

    #[test]
    fn every_client_has_a_way_in() {
        for spec in all() {
            match spec.method {
                Method::Cli { .. } | Method::JsonArg { .. } => {}
                Method::Json { file, .. }
                | Method::Toml { file, .. }
                | Method::Yaml { file, .. }
                | Method::OpenCode { file } => {
                    assert!(
                        file.for_os(Os::Windows).is_some()
                            && file.for_os(Os::Macos).is_some()
                            && file.for_os(Os::Linux).is_some(),
                        "{} has no path on some system",
                        spec.id
                    );
                }
            }
        }
    }

    #[test]
    fn the_server_argv_is_the_same_for_every_client() {
        let cmd = McpCommand::new("C:/Whatfast/whatfast.exe", "codex");
        let argv = all()[0].server_argv(&cmd);
        assert_eq!(
            argv,
            vec!["C:/Whatfast/whatfast.exe", "mcp", "--agent", "codex"]
        );
    }
}
