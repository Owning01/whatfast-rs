//! Chat export (`TXT`, `JSON`, `HTML`) and portable settings/theme backup.

use std::fs;
use std::path::PathBuf;

use whatfast_protocol::{Message, message::Content};

/// Target directory for user-triggered exports (Cross-platform: Downloads or home).
pub(crate) fn exports_dir() -> PathBuf {
    let base = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let downloads = base.join("Downloads");
    if downloads.is_dir() {
        downloads
    } else {
        base
    }
}

/// Sanitizes a chat title into a safe filename stem.
pub(crate) fn safe_filename_stem(raw: &str) -> String {
    let clean: String = raw
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = clean.trim().replace(' ', "_");
    if trimmed.is_empty() {
        "chat".to_string()
    } else {
        trimmed.chars().take(48).collect()
    }
}

fn msg_body(msg: &Message) -> String {
    match &msg.content {
        Some(Content::Text(t)) => t.text.clone(),
        Some(Content::Image(img)) => {
            if img.caption.is_empty() {
                "[Photo]".to_string()
            } else {
                format!("[Photo] {}", img.caption)
            }
        }
        Some(Content::Attachment(att)) => {
            let name = if att.file_name.is_empty() {
                att.kind.as_str()
            } else {
                att.file_name.as_str()
            };
            if att.caption.is_empty() {
                format!("<{name}>")
            } else {
                format!("<{name}> {}", att.caption)
            }
        }
        Some(Content::Poll(p)) => format!("[Poll] {}", p.question),
        _ => String::new(),
    }
}

/// Formats messages as plain readable `.txt`.
pub(crate) fn format_chat_txt(chat_title: &str, messages: &[Message]) -> String {
    let mut out = String::new();
    out.push_str(&format!("# Whatfast Export: {chat_title}\n\n"));
    for msg in messages {
        let sender = if msg.from_me {
            "Me"
        } else if !msg.sender_name.is_empty() {
            msg.sender_name.as_str()
        } else {
            msg.sender_id.as_str()
        };
        let ts = crate::helpers::format_epoch_date_numeric(msg.timestamp_ms);
        let body = msg_body(msg);
        if !body.is_empty() {
            out.push_str(&format!("[{ts}] {sender}: {body}\n"));
        }
    }
    out
}

/// Formats messages as standalone `.html`.
pub(crate) fn format_chat_html(chat_title: &str, messages: &[Message]) -> String {
    let esc = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    let mut out = String::from(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Whatfast Export</title>\
        <style>body{font-family:system-ui,sans-serif;max-width:760px;margin:2rem auto;background:#111b21;color:#e9edef}\
        .msg{padding:8px 12px;border-radius:8px;margin:6px 0;background:#202c33}\
        .me{background:#005c4b;margin-left:20%}.meta{font-size:12px;color:#8696a0;margin-bottom:4px}</style></head><body>",
    );
    out.push_str(&format!("<h1>{}</h1>", esc(chat_title)));
    for msg in messages {
        let sender = if msg.from_me {
            "Me"
        } else if !msg.sender_name.is_empty() {
            msg.sender_name.as_str()
        } else {
            msg.sender_id.as_str()
        };
        let cls = if msg.from_me { "msg me" } else { "msg" };
        let ts = crate::helpers::format_epoch_date_numeric(msg.timestamp_ms);
        let body = esc(&msg_body(msg));
        if !body.is_empty() {
            out.push_str(&format!(
                "<div class=\"{cls}\"><div class=\"meta\">{} · {ts}</div><div>{body}</div></div>",
                esc(sender)
            ));
        }
    }
    out.push_str("</body></html>");
    out
}

/// Exports a chat's loaded messages to `txt`, `json`, or `html`.
pub(crate) fn export_chat_file(
    chat_title: &str,
    messages: &[Message],
    format: &str,
) -> Result<PathBuf, String> {
    let stem = safe_filename_stem(chat_title);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let ext = match format {
        "json" => "json",
        "html" => "html",
        _ => "txt",
    };
    let path = exports_dir().join(format!("whatfast-{stem}-{ts}.{ext}"));
    let content = match ext {
        "json" => {
            let items: Vec<serde_json::Value> = messages
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "id": m.id,
                        "chat_id": m.chat_id,
                        "from_me": m.from_me,
                        "sender_name": m.sender_name,
                        "timestamp_ms": m.timestamp_ms,
                        "body": msg_body(m),
                    })
                })
                .collect();
            serde_json::to_string_pretty(&items).map_err(|e| e.to_string())?
        }
        "html" => format_chat_html(chat_title, messages),
        _ => format_chat_txt(chat_title, messages),
    };
    fs::write(&path, content).map_err(|e| e.to_string())?;
    Ok(path)
}
