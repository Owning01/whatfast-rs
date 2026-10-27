//! The registration of the MCP server in each client.
//!
//! Three shapes, one entry point. The command-line clients edit their own
//! configuration; the file clients get an entry merged next to the ones they
//! already hold; VS Code takes the definition as one JSON argument.
//!
//! Every write takes a copy of the file before it changes. The merge never
//! rewrites the whole document: JSON is parsed and written again, TOML and
//! YAML get the one block replaced in place, so comments and keys of the user
//! survive.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::spec::{ClientSpec, Method};
use crate::{McpCommand, Report, paths};

/// The suffix of the copy taken before a change.
const BACKUP: &str = "whatfast-backup";

/// Registers the server of Whatfast in one client.
pub fn register(spec: &ClientSpec, cmd: &McpCommand) -> io::Result<Report> {
    // A file the user picked by hand wins over the one the ficha names: the
    // ficha is a guess about the system, and the guess is sometimes wrong.
    if let Some(path) = saved_path(spec.id) {
        return register_at(spec, cmd, &path);
    }
    let name = ClientSpec::SERVER;
    match spec.method {
        Method::Json { file, key } => {
            let path = require(file_path(file)?)?;
            let backup = backup(&path)?;
            let text = read_or_empty(&path)?;
            let merged = merge_json(&text, key, name, &spec.server_json(cmd))?;
            fs::write(&path, merged)?;
            Ok(Report {
                method: "json",
                target: path.display().to_string(),
                backup,
            })
        }
        Method::Toml { file, section } => {
            let path = require(file_path(file)?)?;
            let backup = backup(&path)?;
            let text = read_or_empty(&path)?;
            let body = toml_body(cmd);
            let merged = upsert_toml(&text, section, &body);
            fs::write(&path, merged)?;
            Ok(Report {
                method: "toml",
                target: path.display().to_string(),
                backup,
            })
        }
        Method::Yaml { file, key } => {
            let path = require(file_path(file)?)?;
            let backup = backup(&path)?;
            let text = read_or_empty(&path)?;
            let merged = upsert_yaml(&text, key, name, &yaml_body(cmd));
            fs::write(&path, merged)?;
            Ok(Report {
                method: "yaml",
                target: path.display().to_string(),
                backup,
            })
        }
        Method::OpenCode { file } => {
            let path = require(file_path(file)?)?;
            let backup = backup(&path)?;
            let text = read_or_empty(&path)?;
            let merged = merge_opencode(&text, name, &spec.server_argv(cmd))?;
            fs::write(&path, merged)?;
            Ok(Report {
                method: "jsonc",
                target: path.display().to_string(),
                backup,
            })
        }
        Method::Cli { bin, args } => {
            let mut full: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
            full.extend(spec.server_argv(cmd));
            run(bin, &full)?;
            Ok(Report {
                method: "cli",
                target: format!("{bin} {}", args.join(" ")),
                backup: None,
            })
        }
        Method::JsonArg { bin, flag } => {
            let definition = serde_json::json!({
                "name": name,
                "command": cmd.exe_string(),
                "args": cmd.args(),
            });
            run(bin, &[flag.to_string(), definition.to_string()])?;
            Ok(Report {
                method: "cli",
                target: format!("{bin} {flag}"),
                backup: None,
            })
        }
    }
}

/// Removes the server from one client.
///
/// Only the clients with a documented remove command and the ones that keep a
/// file can be undone here. The rest say so, and the user removes the entry
/// in the client.
pub fn unregister(spec: &ClientSpec, _cmd: &McpCommand) -> io::Result<()> {
    // The file the user picked is the one that holds the entry.
    if let Some(path) = saved_path(spec.id) {
        return unregister_at(spec, &path);
    }
    let name = ClientSpec::SERVER;
    match spec.method {
        Method::Json { file, key } => {
            let Some(path) = file_path(file)? else {
                return Ok(());
            };
            if !path.exists() {
                return Ok(());
            }
            let text = read_or_empty(&path)?;
            let merged = remove_json(&text, key, name)?;
            fs::write(&path, merged)?;
            Ok(())
        }
        Method::Toml { file, section } => {
            let Some(path) = file_path(file)? else {
                return Ok(());
            };
            if !path.exists() {
                return Ok(());
            }
            let text = read_or_empty(&path)?;
            fs::write(&path, remove_toml(&text, section))?;
            Ok(())
        }
        Method::Yaml { file, key } => {
            let Some(path) = file_path(file)? else {
                return Ok(());
            };
            if !path.exists() {
                return Ok(());
            }
            let text = read_or_empty(&path)?;
            fs::write(&path, remove_yaml(&text, key, name))?;
            Ok(())
        }
        Method::OpenCode { file } => {
            let Some(path) = file_path(file)? else {
                return Ok(());
            };
            if !path.exists() {
                return Ok(());
            }
            let text = read_or_empty(&path)?;
            fs::write(&path, remove_opencode(&text, name)?)?;
            Ok(())
        }
        Method::Cli { .. } | Method::JsonArg { .. } => Err(io::Error::other(format!(
            "{} no tiene un comando de baja: quitá el servidor «{name}» desde el cliente",
            spec.name
        ))),
    }
}

/// Registers the server in one file the user picked by hand.
///
/// The format follows the extension of the file, not the method of the ficha:
/// the user may have moved the configuration of a client somewhere else, and
/// what the file can hold is written on its name. A `.jsonc` is read as the
/// file of OpenCode, which is the only client of the catalog with that shape.
pub fn register_at(spec: &ClientSpec, cmd: &McpCommand, path: &Path) -> io::Result<Report> {
    let name = ClientSpec::SERVER;
    let extension = extension(path).to_ascii_lowercase();
    let backup = backup(path)?;
    let text = read_or_empty(path)?;
    let (method, merged) = if matches!(spec.method, Method::OpenCode { .. }) || extension == "jsonc"
    {
        (
            "jsonc",
            merge_opencode(&text, name, &spec.server_argv(cmd))?,
        )
    } else {
        match extension.as_str() {
            "toml" => {
                let section = match spec.method {
                    Method::Toml { section, .. } => section,
                    _ => "mcp_servers.whatfast",
                };
                ("toml", upsert_toml(&text, section, &toml_body(cmd)))
            }
            "yaml" | "yml" => {
                let key = match spec.method {
                    Method::Yaml { key, .. } => key,
                    _ => "mcpServers",
                };
                ("yaml", upsert_yaml(&text, key, name, &yaml_body(cmd)))
            }
            _ => {
                let key = match spec.method {
                    Method::Json { key, .. } => key,
                    _ => "mcpServers",
                };
                (
                    "json",
                    merge_json(&text, key, name, &spec.server_json(cmd))?,
                )
            }
        }
    };
    if let Some(parent) = path.parent()
        && !parent.exists()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, merged)?;
    Ok(Report {
        method,
        target: path.display().to_string(),
        backup,
    })
}

/// The file the user picked for a harness, when it picked one.
pub fn saved_path(agent: &str) -> Option<PathBuf> {
    let path = paths::agents_dir()?.join(format!("{agent}.config"));
    let text = fs::read_to_string(path).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

/// Remembers the file the user picked for a harness.
pub fn save_path(agent: &str, path: &Path) -> io::Result<PathBuf> {
    let folder = paths::agents_dir()
        .ok_or_else(|| io::Error::other("the data folder of Whatfast is unknown"))?;
    fs::create_dir_all(&folder)?;
    let file = folder.join(format!("{agent}.config"));
    fs::write(&file, format!("{}\n", path.display()))?;
    Ok(file)
}

/// Forgets the file the user picked: the ficha is used again.
pub fn clear_path(agent: &str) -> io::Result<bool> {
    let Some(folder) = paths::agents_dir() else {
        return Ok(false);
    };
    let file = folder.join(format!("{agent}.config"));
    match fs::remove_file(file) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Takes the server out of one file the user picked by hand.
pub fn unregister_at(spec: &ClientSpec, path: &Path) -> io::Result<()> {
    let name = ClientSpec::SERVER;
    if !path.exists() {
        return Ok(());
    }
    let text = read_or_empty(path)?;
    let cleaned = match spec.method {
        Method::OpenCode { .. } => remove_opencode(&text, name)?,
        Method::Toml { section, .. } => remove_toml(&text, section),
        Method::Yaml { key, .. } => remove_yaml(&text, key, name),
        _ => remove_json(&text, key_of(spec), name)?,
    };
    fs::write(path, cleaned)?;
    Ok(())
}

/// The key a client keeps its servers under, for the files that hold objects.
fn key_of(spec: &ClientSpec) -> &'static str {
    match spec.method {
        Method::Json { key, .. } => key,
        Method::Yaml { key, .. } => key,
        Method::OpenCode { .. } => "mcp.servers",
        Method::Toml { .. } | Method::Cli { .. } | Method::JsonArg { .. } => "mcpServers",
    }
}

/// The path of a location, or `None` when this system has none.
fn file_path(location: crate::Location) -> io::Result<Option<PathBuf>> {
    Ok(crate::detect::location_path(location))
}

/// The entry of the server in the JSONC file of OpenCode.
///
/// The entry has the shape OpenCode reads: a local server, with the whole
/// command as one array. The file admits comments and trailing commas, so it is
/// cleaned before it is parsed. The comments of the head of the document stay in
/// place; the ones between keys are not part of the model and do not survive the
/// rewrite, which is what the copy taken before the change is for.
fn merge_opencode(text: &str, name: &str, argv: &[String]) -> io::Result<String> {
    let mut root = match parse_jsonc(text)? {
        serde_json::Value::Object(map) => map,
        serde_json::Value::Null => serde_json::Map::new(),
        other => {
            return Err(io::Error::other(format!(
                "the file is not a JSON object: {other}"
            )));
        }
    };
    let mcp = root
        .entry("mcp".to_owned())
        .or_insert_with(|| serde_json::json!({}));
    if !mcp.is_object() {
        *mcp = serde_json::json!({});
    }
    let servers = mcp
        .as_object_mut()
        .ok_or_else(|| io::Error::other("mcp is not an object"))?
        .entry("servers".to_owned())
        .or_insert_with(|| serde_json::json!({}));
    if !servers.is_object() {
        *servers = serde_json::json!({});
    }
    let entry = serde_json::json!({
        "type": "local",
        "command": argv,
    });
    servers
        .as_object_mut()
        .ok_or_else(|| io::Error::other("mcp.servers is not an object"))?
        .insert(name.to_owned(), entry);
    write_jsonc(text, &root)
}

/// Takes the server out of the JSONC file of OpenCode.
fn remove_opencode(text: &str, name: &str) -> io::Result<String> {
    let mut root = match parse_jsonc(text)? {
        serde_json::Value::Object(map) => map,
        _ => return Ok(text.to_owned()),
    };
    let Some(servers) = root
        .get_mut("mcp")
        .and_then(serde_json::Value::as_object_mut)
        .and_then(|mcp| mcp.get_mut("servers"))
        .and_then(serde_json::Value::as_object_mut)
    else {
        return Ok(text.to_owned());
    };
    if servers.remove(name).is_none() {
        return Ok(text.to_owned());
    }
    write_jsonc(text, &root)
}

/// The document with its head comments and the object, written again.
fn write_jsonc(
    original: &str,
    root: &serde_json::Map<String, serde_json::Value>,
) -> io::Result<String> {
    let body = serde_json::to_string_pretty(&serde_json::Value::Object(root.clone()))
        .map_err(|error| io::Error::other(error.to_string()))?;
    Ok(format!("{}{body}\n", head_comments(original)))
}

/// The comment lines that open the document, which are kept as they are.
fn head_comments(text: &str) -> String {
    let mut head = String::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            head.push_str(line);
            head.push('\n');
        } else if trimmed.is_empty() && head.is_empty() {
            continue;
        } else {
            break;
        }
    }
    head
}

/// Reads a JSONC document: comments and trailing commas are dropped first.
fn parse_jsonc(text: &str) -> io::Result<serde_json::Value> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(serde_json::Value::Null);
    }
    let clean = strip_jsonc(trimmed);
    serde_json::from_str(&clean)
        .map_err(|error| io::Error::other(format!("the file is not valid JSONC: {error}")))
}

/// The same text without comments and without the commas before a closing
/// bracket, which JSON does not allow and JSONC does.
fn strip_jsonc(text: &str) -> String {
    let mut without_comments = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;
    while let Some(character) = chars.next() {
        if in_string {
            without_comments.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            '"' => {
                in_string = true;
                without_comments.push(character);
            }
            '/' => match chars.peek() {
                Some('/') => {
                    // The line comment ends with its line, which stays.
                    for next in chars.by_ref() {
                        if next == '\n' {
                            without_comments.push('\n');
                            break;
                        }
                    }
                }
                Some('*') => {
                    chars.next();
                    let mut previous = '\0';
                    for next in chars.by_ref() {
                        if previous == '*' && next == '/' {
                            break;
                        }
                        previous = next;
                    }
                }
                _ => without_comments.push(character),
            },
            _ => without_comments.push(character),
        }
    }
    drop_trailing_commas(&without_comments)
}

/// Removes the comma that a JSONC document may leave before `}` or `]`.
fn drop_trailing_commas(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut clean = String::with_capacity(text.len());
    let mut in_string = false;
    let mut escaped = false;
    let mut index = 0;
    while index < chars.len() {
        let character = chars[index];
        if in_string {
            clean.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        if character == '"' {
            in_string = true;
            clean.push(character);
            index += 1;
            continue;
        }
        if character == ',' {
            let mut look = index + 1;
            while look < chars.len() && chars[look].is_whitespace() {
                look += 1;
            }
            if look < chars.len() && (chars[look] == '}' || chars[look] == ']') {
                index += 1;
                continue;
            }
        }
        clean.push(character);
        index += 1;
    }
    clean
}

/// An existing file, or an error that names it.
fn require(path: Option<PathBuf>) -> io::Result<PathBuf> {
    let path = path.ok_or_else(|| io::Error::other("this system has no path for the client"))?;
    if let Some(parent) = path.parent()
        && !parent.exists()
    {
        return Err(io::Error::other(format!(
            "no existe la carpeta {}: ¿está abierto el cliente alguna vez?",
            parent.display()
        )));
    }
    Ok(path)
}

/// Copies the file once, before the first change.
fn backup(path: &Path) -> io::Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let target = path.with_extension(format!("{}.{BACKUP}", extension(path)));
    if target.exists() {
        return Ok(Some(target));
    }
    fs::copy(path, &target)?;
    Ok(Some(target))
}

/// The extension of a file, without the dot.
fn extension(path: &Path) -> &str {
    path.extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("json")
}

/// The text of a file, or an empty document.
fn read_or_empty(path: &Path) -> io::Result<String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error),
    }
}

/// Runs a program and turns a failure into the text it printed.
fn run(bin: &str, args: &[String]) -> io::Result<()> {
    let output = Command::new(bin)
        .args(args)
        .output()
        .map_err(|error| io::Error::other(format!("no se pudo ejecutar {bin}: {error}")))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(io::Error::other(if stderr.is_empty() {
        format!("{bin} terminó con {}", output.status)
    } else {
        format!("{bin}: {stderr}")
    }))
}

/// Puts one server in the object of servers of a JSON file.
pub fn merge_json(
    text: &str,
    key: &str,
    name: &str,
    value: &serde_json::Value,
) -> io::Result<String> {
    let mut root: serde_json::Value = if text.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(text)
            .map_err(|error| io::Error::other(format!("el archivo no es JSON válido: {error}")))?
    };
    let Some(object) = root.as_object_mut() else {
        return Err(io::Error::other("el archivo no tiene un objeto en la raíz"));
    };
    let servers = object
        .entry(key.to_string())
        .or_insert_with(|| serde_json::json!({}));
    let Some(servers) = servers.as_object_mut() else {
        return Err(io::Error::other(format!("«{key}» no es un objeto")));
    };
    servers.insert(name.to_string(), value.clone());
    let mut out = serde_json::to_string_pretty(&root)
        .map_err(|error| io::Error::other(format!("no se pudo escribir el JSON: {error}")))?;
    out.push('\n');
    Ok(out)
}

/// Takes one server out of the object of servers of a JSON file.
pub fn remove_json(text: &str, key: &str, name: &str) -> io::Result<String> {
    let mut root: serde_json::Value = if text.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(text)
            .map_err(|error| io::Error::other(format!("el archivo no es JSON válido: {error}")))?
    };
    if let Some(servers) = root.get_mut(key).and_then(|value| value.as_object_mut()) {
        servers.remove(name);
    }
    let mut out = serde_json::to_string_pretty(&root)
        .map_err(|error| io::Error::other(format!("no se pudo escribir el JSON: {error}")))?;
    out.push('\n');
    Ok(out)
}

/// The keys of the table, in the order TOML wants.
fn toml_body(cmd: &McpCommand) -> Vec<(String, String)> {
    let args: Vec<String> = cmd
        .args()
        .iter()
        .map(|arg| format!("\"{}\"", escape(arg)))
        .collect();
    vec![
        (
            "command".into(),
            format!("\"{}\"", escape(&cmd.exe_string())),
        ),
        ("args".into(), format!("[{}]", args.join(", "))),
    ]
}

/// Replaces or adds one `[section]` with its keys.
pub fn upsert_toml(text: &str, section: &str, body: &[(String, String)]) -> String {
    let header = format!("[{section}]");
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.iter().position(|line| line.trim() == header);
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + body.len() + 2);
    match start {
        Some(start) => {
            out.extend(lines[..=start].iter().map(|line| (*line).to_string()));
            // The body ends at the next header or at the end of the file.
            let mut end = start + 1;
            while end < lines.len() && !lines[end].trim_start().starts_with('[') {
                end += 1;
            }
            for (key, value) in body {
                out.push(format!("{key} = {value}"));
            }
            out.extend(lines[end..].iter().map(|line| (*line).to_string()));
        }
        None => {
            out.extend(lines.iter().map(|line| (*line).to_string()));
            if !out.last().is_some_and(|line| line.trim().is_empty()) && !out.is_empty() {
                out.push(String::new());
            }
            out.push(header);
            for (key, value) in body {
                out.push(format!("{key} = {value}"));
            }
        }
    }
    let mut text = out.join("\n");
    text.push('\n');
    text
}

/// Takes one `[section]` out.
pub fn remove_toml(text: &str, section: &str) -> String {
    let header = format!("[{section}]");
    let mut out: Vec<&str> = Vec::new();
    let mut skipping = false;
    for line in text.lines() {
        if line.trim() == header {
            skipping = true;
            continue;
        }
        if skipping && line.trim_start().starts_with('[') {
            skipping = false;
        }
        if !skipping {
            out.push(line);
        }
    }
    let mut text = out.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    text
}

/// The keys of the mapping, as YAML lines indented four spaces.
fn yaml_body(cmd: &McpCommand) -> Vec<String> {
    let mut lines = vec![format!("    command: \"{}\"", escape(&cmd.exe_string()))];
    lines.push("    args:".to_string());
    for arg in cmd.args() {
        lines.push(format!("      - \"{}\"", escape(&arg)));
    }
    lines
}

/// Replaces or adds the mapping of one server under `key`.
pub fn upsert_yaml(text: &str, key: &str, name: &str, body: &[String]) -> String {
    let mut lines: Vec<String> = text.lines().map(|line| line.to_string()).collect();
    let header = format!("{key}:");
    let start = lines.iter().position(|line| line.trim_end() == header);
    let block = |_lines: &[String]| -> Vec<String> {
        let mut out = vec![format!("  {name}:")];
        out.extend(body.iter().cloned());
        out
    };
    match start {
        Some(start) => {
            // The list of servers is the run of lines indented two spaces.
            let mut end = start + 1;
            while end < lines.len()
                && (lines[end].trim().is_empty()
                    || lines[end].starts_with("  ")
                    || lines[end].starts_with('\t'))
            {
                end += 1;
            }
            let inner: Vec<String> = lines[start + 1..end].to_vec();
            let existing = inner
                .iter()
                .position(|line| line.trim() == format!("{name}:"));
            let mut rebuilt: Vec<String> = Vec::new();
            match existing {
                Some(at) => {
                    rebuilt.extend(inner[..at].iter().cloned());
                    rebuilt.extend(block(&lines));
                    // Skip the old body of this server: deeper than two spaces.
                    let mut after = at + 1;
                    while after < inner.len()
                        && (inner[after].starts_with("    ") || inner[after].trim().is_empty())
                    {
                        after += 1;
                    }
                    rebuilt.extend(inner[after..].iter().cloned());
                }
                None => {
                    rebuilt.extend(inner);
                    rebuilt.extend(block(&lines));
                }
            }
            lines.splice(start + 1..end, rebuilt);
        }
        None => {
            if !lines.last().is_some_and(|line| line.trim().is_empty()) && !lines.is_empty() {
                lines.push(String::new());
            }
            lines.push(header);
            lines.extend(block(&lines));
        }
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// Takes the mapping of one server out of `key`.
pub fn remove_yaml(text: &str, key: &str, name: &str) -> String {
    let mut lines: Vec<&str> = Vec::new();
    let mut skip_depth: Option<usize> = None;
    for line in text.lines() {
        if let Some(depth) = skip_depth {
            let indent = line.len() - line.trim_start().len();
            if line.trim().is_empty() || indent > depth {
                continue;
            }
            skip_depth = None;
        }
        if line.trim_end() == format!("{key}:") && line == line.trim_start() {
            // The next line may be the server: look ahead is not possible in
            // one pass, so keep the header and let the mapping rule work.
            lines.push(line);
            continue;
        }
        if line.starts_with("  ") && line.trim() == format!("{name}:") {
            skip_depth = Some(2);
            continue;
        }
        lines.push(line);
    }
    let mut text = lines.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    text
}

/// Escapes a string for TOML and for YAML in double quotes.
fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_adds_the_server_and_keeps_the_rest() {
        let before = r#"{"theme":"dark","mcpServers":{"other":{"command":"x"}}}"#;
        let value = serde_json::json!({"command": "C:/w.exe", "args": ["mcp"]});
        let after = merge_json(before, "mcpServers", "whatfast", &value).expect("json");
        let parsed: serde_json::Value = serde_json::from_str(&after).expect("json");
        assert_eq!(parsed["theme"], "dark");
        assert_eq!(parsed["mcpServers"]["other"]["command"], "x");
        assert_eq!(parsed["mcpServers"]["whatfast"]["command"], "C:/w.exe");
    }

    #[test]
    fn json_replace_is_idempotent() {
        let value = serde_json::json!({"command": "C:/w.exe", "args": ["mcp"]});
        let once = merge_json("{}", "mcpServers", "whatfast", &value).expect("json");
        let twice = merge_json(&once, "mcpServers", "whatfast", &value).expect("json");
        assert_eq!(once, twice);
    }

    #[test]
    fn json_refuses_a_broken_file() {
        let error = merge_json("{ nope", "mcpServers", "whatfast", &serde_json::json!({}))
            .expect_err("must fail");
        assert!(error.to_string().contains("JSON"));
    }

    #[test]
    fn opcode_adds_its_server_under_mcp_servers() {
        let before = "// Configuración global.\n{\n  \"theme\": \"dark\",\n  \"mcp\": {\n    \"servers\": {\n      \"other\": { \"command\": [\"x\"], \"type\": \"local\" },\n    },\n  },\n}\n";
        let argv = vec![
            "C:/w.exe".to_string(),
            "mcp".to_string(),
            "--agent".to_string(),
            "opencode".to_string(),
        ];
        let after = merge_opencode(before, "whatfast", &argv).expect("jsonc");
        // The head comment stays where it was.
        assert!(after.starts_with("// Configuración global.\n"));
        let parsed: serde_json::Value =
            serde_json::from_str(&strip_jsonc(&after)).expect("parseable");
        assert_eq!(parsed["theme"], "dark");
        assert_eq!(parsed["mcp"]["servers"]["other"]["command"][0], "x");
        assert_eq!(parsed["mcp"]["servers"]["whatfast"]["type"], "local");
        assert_eq!(
            parsed["mcp"]["servers"]["whatfast"]["command"][0],
            "C:/w.exe"
        );
        assert_eq!(
            parsed["mcp"]["servers"]["whatfast"]["command"][3],
            "opencode"
        );
    }

    #[test]
    fn opcode_creates_the_file_when_it_is_empty() {
        let argv = vec!["C:/w.exe".to_string(), "mcp".to_string()];
        let after = merge_opencode("", "whatfast", &argv).expect("jsonc");
        let parsed: serde_json::Value = serde_json::from_str(&after).expect("parseable");
        assert_eq!(parsed["mcp"]["servers"]["whatfast"]["type"], "local");
    }

    #[test]
    fn opcode_replace_is_idempotent() {
        let argv = vec!["C:/w.exe".to_string(), "mcp".to_string()];
        let once = merge_opencode("", "whatfast", &argv).expect("jsonc");
        let twice = merge_opencode(&once, "whatfast", &argv).expect("jsonc");
        assert_eq!(once, twice);
    }

    #[test]
    fn opcode_removes_its_server_and_keeps_the_others() {
        let argv = vec!["C:/w.exe".to_string(), "mcp".to_string()];
        let with = merge_opencode(
            "{\"mcp\":{\"servers\":{\"other\":{\"command\":[\"x\"]}}}}",
            "whatfast",
            &argv,
        )
        .expect("jsonc");
        let after = remove_opencode(&with, "whatfast").expect("jsonc");
        let parsed: serde_json::Value = serde_json::from_str(&after).expect("parseable");
        assert!(parsed["mcp"]["servers"]["whatfast"].is_null());
        assert_eq!(parsed["mcp"]["servers"]["other"]["command"][0], "x");
    }

    #[test]
    fn the_comments_and_the_trailing_commas_of_jsonc_are_dropped() {
        let clean = strip_jsonc(
            "{\n  // uno\n  \"a\": \"// no es un comentario\", /* dos */\n  \"b\": [1, 2,],\n}\n",
        );
        let parsed: serde_json::Value = serde_json::from_str(&clean).expect("parseable");
        assert_eq!(parsed["a"], "// no es un comentario");
        assert_eq!(parsed["b"][1], 2);
    }

    #[test]
    fn a_picked_file_is_written_and_remembered() {
        let dir = std::env::temp_dir().join(format!(
            "whatfast-picked-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("dir");
        let file = dir.join("otro.json");
        std::fs::write(&file, "{\"theme\":\"dark\"}").expect("file");
        let spec = crate::by_id("cursor").expect("cursor");
        let cmd = McpCommand::new("C:/w.exe", "cursor");
        let report = register_at(spec, &cmd, &file).expect("register");
        assert_eq!(report.method, "json");
        let text = std::fs::read_to_string(&file).expect("read");
        let parsed: serde_json::Value = serde_json::from_str(&text).expect("json");
        assert_eq!(parsed["mcpServers"]["whatfast"]["command"], "C:/w.exe");
        assert_eq!(parsed["theme"], "dark");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn toml_adds_the_table_after_the_rest() {
        let before = "model = \"o3\"\n";
        let body = vec![
            ("command".to_string(), "\"C:/w.exe\"".to_string()),
            ("args".to_string(), "[\"mcp\"]".to_string()),
        ];
        let after = upsert_toml(before, "mcp_servers.whatfast", &body);
        assert!(after.contains("model = \"o3\""));
        assert!(after.contains("[mcp_servers.whatfast]"));
        assert!(after.contains("command = \"C:/w.exe\""));
        let twice = upsert_toml(&after, "mcp_servers.whatfast", &body);
        assert_eq!(after, twice);
    }

    #[test]
    fn toml_replaces_the_body_and_keeps_the_next_table() {
        let before = "[mcp_servers.whatfast]\ncommand = \"old\"\nextra = 1\n\n[mcp_servers.other]\ncommand = \"x\"\n";
        let body = vec![("command".to_string(), "\"new\"".to_string())];
        let after = upsert_toml(before, "mcp_servers.whatfast", &body);
        assert!(after.contains("command = \"new\""));
        assert!(!after.contains("extra = 1"));
        assert!(after.contains("[mcp_servers.other]"));
    }

    #[test]
    fn yaml_adds_the_mapping_under_the_key() {
        let before = "model: x\n";
        let body = vec!["    command: \"C:/w.exe\"".to_string()];
        let after = upsert_yaml(before, "mcp_servers", "whatfast", &body);
        assert!(after.contains("model: x"));
        assert!(after.contains("mcp_servers:"));
        assert!(after.contains("  whatfast:"));
        assert!(after.contains("    command: \"C:/w.exe\""));
        let twice = upsert_yaml(&after, "mcp_servers", "whatfast", &body);
        assert_eq!(after, twice);
    }

    #[test]
    fn yaml_keeps_the_other_servers() {
        let before = "mcp_servers:\n  other:\n    command: \"x\"\n";
        let body = vec!["    command: \"C:/w.exe\"".to_string()];
        let after = upsert_yaml(before, "mcp_servers", "whatfast", &body);
        assert!(after.contains("  other:"));
        assert!(after.contains("  whatfast:"));
    }

    #[test]
    fn the_windows_path_survives_the_escape() {
        assert_eq!(
            escape(r"C:\Program Files\w.exe"),
            r"C:\\Program Files\\w.exe"
        );
    }
}
