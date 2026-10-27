//! The JSON-RPC side of the server: one message per line, on stdin and
//! stdout.
//!
//! The MCP stdio transport is newline-delimited JSON. Nothing else may be
//! written to stdout: the logs go to stderr, where the harness shows them.

use std::io::{BufRead, Write};

use serde_json::{Value, json};

use crate::{Options, client::Client, tools};

/// The protocol revisions this server speaks, newest first.
const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

/// What the harness loads into the model as soon as it connects.
const INSTRUCTIONS: &str = "\
Whatfast connects you to the WhatsApp account of the user, through the desktop app.

Read before you write: chats_list gives you the chat_id of every conversation, messages_read gives you what was said.
messages_search looks only at the cache of this device. messages_wait blocks until a new message arrives: use it instead of polling.
message_send sends one text message to one chat, as the linked device. Keep it short, in the language of the chat, and never broadcast.
In a chat the user did not enable for sending, message_send leaves the text as a draft instead: draft_propose does the same on purpose,
drafts_list shows what is waiting and draft_decide approves or discards it. Approving is not sending: the user's turn sends it.
Ids are opaque strings: use the ones a tool returned, never invent one.
The user sees everything you send. Do not send links, files or money requests without asking first.";

/// Runs the server until the harness closes stdin.
pub fn serve(options: &Options) -> i32 {
    eprintln!(
        "whatfast mcp {} ready for {}",
        env!("CARGO_PKG_VERSION"),
        options.agent
    );
    let mut client = match Client::new(
        options.data_dir.clone(),
        options.backend.clone(),
        options.agent.clone(),
    ) {
        Ok(client) => client,
        Err(message) => {
            eprintln!("whatfast mcp: {message}");
            return 1;
        }
    };
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let message: Value = match serde_json::from_str(line) {
            Ok(message) => message,
            Err(error) => {
                let response = failure(Value::Null, -32700, &format!("parse error: {error}"));
                write_message(&mut out, &response);
                continue;
            }
        };
        let id = message.get("id").cloned().unwrap_or(Value::Null);
        let is_notification = message.get("id").is_none();
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            // An answer, not a request: nothing here asks anything.
            continue;
        };
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let outcome = match method {
            "initialize" => Some(Ok(initialize(&params))),
            "ping" => Some(Ok(json!({}))),
            "tools/list" => Some(Ok(json!({"tools": tools::definitions()}))),
            "tools/call" => Some(Ok(tools_call(&mut client, &params))),
            // The server offers tools only; an empty list is the honest
            // answer, never an error a harness would show the user.
            "resources/list" => Some(Ok(json!({"resources": []}))),
            "prompts/list" => Some(Ok(json!({"prompts": []}))),
            "notifications/initialized"
            | "notifications/cancelled"
            | "notifications/roots/list_changed" => None,
            other if other.starts_with("notifications/") => None,
            other => Some(Err((-32601, format!("unknown method {other}")))),
        };
        match outcome {
            Some(Ok(result)) if !is_notification => {
                write_message(&mut out, &success(id, result));
            }
            Some(Err((code, message))) if !is_notification => {
                write_message(&mut out, &failure(id, code, &message));
            }
            _ => {}
        }
    }
    // A backend this server started has nobody left to serve: closing it here
    // keeps a second window from finding the profile taken. A backend that was
    // already running belongs to the application and stays.
    client.shutdown();
    0
}

fn initialize(params: &Value) -> Value {
    let offered = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let version = if PROTOCOL_VERSIONS.contains(&offered) {
        offered
    } else {
        PROTOCOL_VERSIONS[0]
    };
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": {"name": "whatfast", "version": env!("CARGO_PKG_VERSION")},
        "instructions": INSTRUCTIONS,
    })
}

fn tools_call(client: &mut Client, params: &Value) -> Value {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return error_object(-32602, "tools/call needs a name");
    };
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    match tools::call(client, name, &arguments) {
        Ok(value) => json!({
            "content": [{"type": "text", "text": value.to_string()}],
            "isError": false,
        }),
        Err(message) => json!({
            "content": [{"type": "text", "text": message}],
            "isError": true,
        }),
    }
}

fn success(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn failure(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn error_object(code: i64, message: &str) -> Value {
    // `tools/call` answers with a result, so an argument mistake is reported
    // as a failed tool, not as a broken protocol.
    json!({
        "content": [{"type": "text", "text": message}],
        "isError": true,
        "_meta": {"code": code},
    })
}

fn write_message(out: &mut impl Write, message: &Value) {
    if writeln!(out, "{message}").is_err() {
        return;
    }
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_revision_is_the_offer() {
        let result = initialize(&json!({"protocolVersion": "2025-06-18"}));
        assert_eq!(result["protocolVersion"], "2025-06-18");
        assert_eq!(result["serverInfo"]["name"], "whatfast");
        assert!(result["instructions"].as_str().unwrap().contains("chat_id"));
    }

    #[test]
    fn an_unknown_revision_falls_back_to_a_known_one() {
        let result = initialize(&json!({"protocolVersion": "1999-01-01"}));
        assert_eq!(result["protocolVersion"], PROTOCOL_VERSIONS[0]);
    }

    #[test]
    fn a_broken_call_is_a_failed_tool_not_a_broken_protocol() {
        let value = error_object(-32602, "tools/call needs a name");
        assert_eq!(value["isError"], true);
        assert_eq!(value["_meta"]["code"], -32602);
    }
}
