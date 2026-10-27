//! The guide that tells each harness what an agent of Whatfast may do.
//!
//! The text lives once, in [`RULES`], and travels into the rules file that the
//! harness already reads. The block is fenced by two HTML comments, so a later
//! run replaces only its own lines and leaves the rest of the file untouched.
//!
//! The catalog in [`crate::spec`] does not carry a rules target today: the
//! [`ClientSpec`] struct has no field for it. This module keeps the table
//! itself, keyed by `id`, and covers the four harnesses with a documented file
//! of user rules:
//!
//! - Claude Code reads `~/.claude/CLAUDE.md`.
//! - Codex reads `~/.codex/AGENTS.md`.
//! - OpenCode reads `~/.config/opencode/AGENTS.md`.
//! - Cursor reads `~/.cursor/rules/whatfast.mdc`.
//!
//! Claude Desktop and VS Code have no file of rules of their own, and Pi and
//! Hermes declare none in the catalog and document no global one, so for those
//! four nothing is written and [`install`] returns an empty [`Report`] instead
//! of guessing a path. When the spec grows a rules target, the table moves
//! there and this module reads it instead of keeping its own.
//!
//! The end of line of the existing file wins: a file in CRLF stays in CRLF, a
//! file in LF stays in LF, and a new file takes the one of the system. A file
//! that ends without a line break gets one, because the block always closes
//! its own line.

use std::fs;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::Report;
use crate::paths;
use crate::spec::ClientSpec;

/// The guide, in Spanish, as the agent reads it.
///
/// It is one block of Markdown because every covered harness reads Markdown:
/// Claude Code and Codex take it as memory, Cursor as a rule file.
pub const RULES: &str = "# Whatfast: guía del agente

Podés leer chats y proponer mensajes. No cambiás ajustes ni la sesión.
Hablás con Whatfast por el MCP, nunca con la sesión de WhatsApp.

- Escribí corto y en el idioma del chat.
- Citá el mensaje con `reply_to_message_id` si respondés a uno.
- No mandes links ni archivos sin pedir permiso.
- Un chat por envío. Nunca difusión.
- Si el chat no está en tus permisos, pedilo; no insistas.
- En un chat sin permiso de envío el mensaje queda como borrador: usá `draft_propose` y avisá.
- Aprobar un borrador no lo envía: el envío es del usuario.
- Cerrá con: `Borrador: { chat_id, reply_to_message_id?, text, reason, confidence 0-1 }`.
";

/// The first line of the block that belongs to Whatfast.
const START: &str = "<!-- whatfast:inicio -->";

/// The last line of the block that belongs to Whatfast.
const END: &str = "<!-- whatfast:fin -->";

/// The suffix of the copy taken before a change.
const BACKUP: &str = "whatfast-backup";

/// How the report names this change.
const METHOD: &str = "guide";

/// The rules file of one harness, as a template for [`paths::expand`].
struct Target {
    /// The id of the harness in [`crate::spec`].
    id: &'static str,
    /// The file, with the variables that the expansion resolves.
    file: &'static str,
}

/// Where each covered harness reads its rules.
const TARGETS: &[Target] = &[
    Target {
        id: "claude-code",
        file: "~/.claude/CLAUDE.md",
    },
    Target {
        id: "codex",
        file: "~/.codex/AGENTS.md",
    },
    Target {
        id: "opencode",
        file: "~/.config/opencode/AGENTS.md",
    },
    Target {
        id: "cursor",
        file: "~/.cursor/rules/whatfast.mdc",
    },
];

/// The rules file of one harness on this system, when it has one.
///
/// The path is not created or checked: it is the place where [`install`] and
/// [`remove`] work.
pub fn rules_file(spec: &ClientSpec) -> Option<PathBuf> {
    let target = TARGETS.iter().find(|target| target.id == spec.id)?;
    paths::expand(target.file)
}

/// Writes the guide into the rules file of one harness.
///
/// The file is created with the block when it does not exist, and the block is
/// replaced in place when it does. The folder of a new file is created too,
/// because a rules folder such as `~/.cursor/rules` may be missing while the
/// harness is installed. A file that already holds the current text is not
/// written, so the second call leaves the copy of the first one alone.
///
/// The executable is part of the signature so the connector calls this next to
/// [`crate::write::register`]; the text of the guide does not name the program.
///
/// A harness without a file of rules writes nothing and returns a report with
/// no target and no backup.
pub fn install(spec: &ClientSpec, _exe: &Path) -> io::Result<Report> {
    let Some(path) = rules_file(spec) else {
        return Ok(Report {
            method: METHOD,
            target: String::new(),
            backup: None,
        });
    };
    install_at(&path)
}

/// Takes the block out of the rules file of one harness.
///
/// It returns `true` when the block was there, and `false` when the file, the
/// block or the path for this system is missing. The line break that [`install`]
/// put before the block goes with it, so the file keeps the shape it had. A file
/// that the block created stays there, empty, because an empty rules file is
/// harmless and a file of the user is not deleted.
pub fn remove(spec: &ClientSpec) -> io::Result<bool> {
    let Some(path) = rules_file(spec) else {
        return Ok(false);
    };
    remove_at(&path)
}

/// True when the rules file of the harness holds the block.
///
/// It answers whether the block is there, not whether it is the current text:
/// an older block still counts, and [`install`] is the one that refreshes it.
/// A harness without a file of rules is always `false`. A file that cannot be
/// read counts as one without the block.
pub fn installed(spec: &ClientSpec) -> bool {
    let Some(path) = rules_file(spec) else {
        return false;
    };
    installed_at(&path)
}

/// Adds the block to `text`, or replaces the one that is already there.
///
/// It returns `None` when the text already holds the current block, so the
/// caller skips the write. An opening mark without its closing one is an
/// error: appending a second block would leave two of them in the file.
pub fn with_block(text: &str) -> io::Result<Option<String>> {
    let newline = newline_of(text);
    let block = render_block(newline);
    if let Some(range) = span(text)? {
        if text[range.clone()] == block {
            return Ok(None);
        }
        let mut out = String::with_capacity(text.len() + block.len());
        out.push_str(&text[..range.start]);
        out.push_str(&block);
        out.push_str(&text[range.end..]);
        return Ok(Some(out));
    }
    let mut out = String::with_capacity(text.len() + block.len() + 2);
    out.push_str(text);
    if !text.is_empty() {
        if !text.ends_with('\n') {
            out.push_str(newline);
        }
        if !text.ends_with("\n\n") && !text.ends_with("\r\n\r\n") {
            out.push_str(newline);
        }
    }
    out.push_str(&block);
    Ok(Some(out))
}

/// Takes the block out of `text`, or returns `None` when there is none.
///
/// The blank line before the block goes too, because [`with_block`] added it.
pub fn without_block(text: &str) -> io::Result<Option<String>> {
    let Some(range) = span(text)? else {
        return Ok(None);
    };
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..range.start]);
    if out.ends_with("\r\n\r\n") {
        let len = out.len() - 2;
        out.truncate(len);
    } else if out.ends_with("\n\n") {
        let len = out.len() - 1;
        out.truncate(len);
    }
    out.push_str(&text[range.end..]);
    Ok(Some(out))
}

/// The block with the line breaks of the target file.
fn render_block(newline: &str) -> String {
    let mut out = String::with_capacity(RULES.len() + 2 * newline.len() + 64);
    out.push_str(START);
    out.push_str(newline);
    for line in RULES.lines() {
        out.push_str(line);
        out.push_str(newline);
    }
    out.push_str(END);
    out.push_str(newline);
    out
}

/// The byte range of the block, including the end of its last line.
fn span(text: &str) -> io::Result<Option<Range<usize>>> {
    let Some(start) = text.find(START) else {
        return Ok(None);
    };
    let after = start + START.len();
    let Some(end) = text[after..].find(END) else {
        return Err(io::Error::other(format!(
            "el archivo tiene «{START}» sin «{END}»: arreglá la marca antes de instalar"
        )));
    };
    let mut end = after + end + END.len();
    if text[end..].starts_with("\r\n") {
        end += 2;
    } else if text[end..].starts_with('\n') {
        end += 1;
    }
    Ok(Some(start..end))
}

/// The line break of a file, or the one of the system when it has none yet.
fn newline_of(text: &str) -> &'static str {
    match text.find('\n') {
        Some(at) if at > 0 && text.as_bytes()[at - 1] == b'\r' => "\r\n",
        Some(_) => "\n",
        None => system_newline(),
    }
}

/// The line break that a new file takes on this system.
fn system_newline() -> &'static str {
    if cfg!(windows) { "\r\n" } else { "\n" }
}

/// Writes or refreshes the block at one path.
fn install_at(path: &Path) -> io::Result<Report> {
    let text = read_or_empty(path)?;
    let report = Report {
        method: METHOD,
        target: path.display().to_string(),
        backup: None,
    };
    let Some(next) = with_block(&text)? else {
        return Ok(report);
    };
    let backup = backup(path)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, next)?;
    Ok(Report { backup, ..report })
}

/// Takes the block out of one path.
fn remove_at(path: &Path) -> io::Result<bool> {
    let text = read_or_empty(path)?;
    let Some(next) = without_block(&text)? else {
        return Ok(false);
    };
    fs::write(path, next)?;
    Ok(true)
}

/// True when one path holds the block.
fn installed_at(path: &Path) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    matches!(span(&text), Ok(Some(_)))
}

/// The text of a file, or an empty document when it is not there.
fn read_or_empty(path: &Path) -> io::Result<String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error),
    }
}

/// Copies the file once, before the first change.
fn backup(path: &Path) -> io::Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("md");
    let target = path.with_extension(format!("{extension}.{BACKUP}"));
    if target.exists() {
        return Ok(Some(target));
    }
    fs::copy(path, &target)?;
    Ok(Some(target))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::by_id;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A fresh folder under the temporary one, with a name of its own.
    fn folder(name: &str) -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let count = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "whatfast-guide-{}-{name}-{count}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("temp folder");
        path
    }

    #[test]
    fn creates_the_file_with_the_block() {
        let dir = folder("create");
        let path = dir.join("CLAUDE.md");
        let report = install_at(&path).expect("install");
        assert_eq!(report.backup, None);
        assert_eq!(report.target, path.display().to_string());
        let text = fs::read_to_string(&path).expect("read");
        assert!(text.starts_with(START));
        assert!(text.ends_with(&format!("{END}{}", system_newline())));
        assert!(text.contains("Nunca difusión."));
        fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn keeps_every_line_of_the_old_file() {
        let dir = folder("keep");
        let path = dir.join("AGENTS.md");
        let before = "# Mis reglas\r\n\r\n- Una regla.\r\n- Otra regla.\r\n";
        fs::write(&path, before).expect("write");
        install_at(&path).expect("install");
        let text = fs::read_to_string(&path).expect("read");
        assert!(text.starts_with(before));
        assert!(text.ends_with(&format!("{END}\r\n")));
        assert!(text.contains("\r\n# Whatfast"));
        assert!(!text.replace("\r\n", "").contains('\n'));
        fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn the_second_install_does_not_write() {
        let dir = folder("twice");
        let path = dir.join("AGENTS.md");
        fs::write(&path, "hola\n").expect("write");
        let first = install_at(&path).expect("install");
        assert!(first.backup.is_some());
        let once = fs::read_to_string(&path).expect("read");
        let second = install_at(&path).expect("install");
        assert_eq!(second.backup, None);
        let twice = fs::read_to_string(&path).expect("read");
        assert_eq!(once, twice);
        assert!(with_block(&once).expect("block").is_none());
        fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn replaces_a_block_that_changed() {
        let dir = folder("replace");
        let path = dir.join("whatfast.mdc");
        let before = "---\ndescription: reglas\n---\n\n";
        fs::write(&path, before).expect("write");
        install_at(&path).expect("install");
        let old = fs::read_to_string(&path).expect("read").replace(
            "Escribí corto y en el idioma del chat.",
            "Escribí largo y en inglés.",
        );
        fs::write(&path, &old).expect("write");
        assert!(installed_at(&path));
        install_at(&path).expect("install");
        let text = fs::read_to_string(&path).expect("read");
        assert!(text.starts_with(before));
        assert!(text.contains("Escribí corto y en el idioma del chat."));
        assert!(!text.contains("Escribí largo"));
        assert_eq!(text.matches(START).count(), 1);
        assert_eq!(text.matches(END).count(), 1);
        fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn remove_leaves_the_file_as_it_was() {
        let dir = folder("remove");
        let path = dir.join("CLAUDE.md");
        let before = "# Notas\n\nAlgo mío.\n";
        fs::write(&path, before).expect("write");
        install_at(&path).expect("install");
        assert!(installed_at(&path));
        assert!(remove_at(&path).expect("remove"));
        assert_eq!(fs::read_to_string(&path).expect("read"), before);
        assert!(!remove_at(&path).expect("remove"));
        fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn installed_answers_for_the_three_cases() {
        let dir = folder("installed");
        let path = dir.join("AGENTS.md");
        assert!(!installed_at(&path));
        fs::write(&path, "# Reglas sin whatfast\n").expect("write");
        assert!(!installed_at(&path));
        install_at(&path).expect("install");
        assert!(installed_at(&path));
        fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn a_mark_without_its_pair_is_an_error() {
        let text = format!("{START}\n## algo\n");
        let error = with_block(&text).expect_err("must fail");
        assert!(error.to_string().contains(END));
        assert!(without_block(&text).is_err());
    }

    #[test]
    fn the_new_file_takes_the_line_break_of_the_system() {
        let text = with_block("").expect("block").expect("text");
        assert!(text.ends_with(&format!("{END}{}", system_newline())));
    }

    #[test]
    fn every_covered_harness_has_a_file() {
        for id in ["claude-code", "codex", "opencode", "cursor"] {
            let spec = by_id(id).expect("spec");
            let path = rules_file(spec).expect("path");
            assert!(path.is_absolute(), "{id} has no absolute path");
        }
    }

    #[test]
    fn a_harness_without_rules_writes_nothing() {
        for id in ["claude-desktop", "vscode", "pi", "hermes"] {
            let spec = by_id(id).expect("spec");
            assert!(rules_file(spec).is_none(), "{id} must have no target");
            assert!(!installed(spec));
            let report = install(spec, Path::new("C:/Whatfast/whatfast.exe")).expect("install");
            assert!(report.target.is_empty());
            assert!(report.backup.is_none());
            assert!(!remove(spec).expect("remove"));
        }
    }
}
