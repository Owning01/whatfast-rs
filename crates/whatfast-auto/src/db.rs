//! `agents.db`: the shared database of the profile.
//!
//! Four tables, the ones the plan drew: `agents` (one row per connected
//! harness, with its grant and the hash of its token), `drafts` (what the agent
//! proposes and the user approves), `schedules` (the automations) and
//! `agent_calls` (one row per tool call, so the user can read what the agent
//! did).
//!
//! The token never reaches the table: `agents` keeps its SHA-256 and the
//! comparison is constant time. The file lives next to `chats.db` in the
//! profile, and the first release that opens it creates it.

use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use rusqlite::{Connection, OptionalExtension, Row, params};
use sha2::{Digest, Sha256};

use crate::grant::{ChatFilter, Grant, Scope, Window};

/// The schema. Every statement is `IF NOT EXISTS`: the file is created the
/// first time the sidecar opens it and it is never migrated by hand.
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS agents (
    id TEXT PRIMARY KEY,
    client TEXT NOT NULL,
    scope TEXT NOT NULL,
    chats TEXT NOT NULL,
    per_hour INTEGER NOT NULL,
    window TEXT,
    humanize INTEGER NOT NULL DEFAULT 1,
    token_hash TEXT NOT NULL,
    created_ms INTEGER NOT NULL,
    revoked_ms INTEGER
);
CREATE TABLE IF NOT EXISTS drafts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL,
    chat_id TEXT NOT NULL,
    reply_to TEXT,
    text TEXT NOT NULL,
    reason TEXT,
    confidence REAL,
    state TEXT NOT NULL,
    at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS drafts_state ON drafts (state, at_ms DESC);
CREATE TABLE IF NOT EXISTS schedules (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL,
    chat_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('once','cron')),
    at_ms INTEGER,
    cron TEXT,
    prompt TEXT NOT NULL,
    output TEXT NOT NULL CHECK (output IN ('draft','send','read')),
    window TEXT,
    next_at_ms INTEGER,
    paused INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS agent_calls (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    at_ms INTEGER NOT NULL,
    agent_id TEXT NOT NULL,
    tool TEXT NOT NULL,
    chat_id TEXT,
    ok INTEGER NOT NULL,
    ms INTEGER NOT NULL,
    bytes INTEGER NOT NULL,
    detail TEXT
);
CREATE INDEX IF NOT EXISTS agent_calls_at ON agent_calls (agent_id, at_ms DESC);
"#;

/// The number of calls kept: the log is for the user to read, not an archive.
const CALLS_KEPT: i64 = 5_000;

/// One connected agent, as the table keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentRow {
    /// The id of the harness: `claude-code`, `codex`.
    pub id: String,
    /// The client it was connected with, when it differs from the id.
    pub client: String,
    /// What it may do, where and when.
    pub grant: Grant,
    /// When it was connected.
    pub created_ms: i64,
    /// When the user disconnected it; a revoked agent cannot come back.
    pub revoked_ms: Option<i64>,
}

impl AgentRow {
    /// Whether the agent still answers.
    pub fn live(&self) -> bool {
        self.revoked_ms.is_none()
    }
}

/// Where a draft is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraftState {
    /// The agent wrote it and waits for the user.
    Proposed,
    /// The user said yes; the next turn sends it.
    Approved,
    /// The send is on its way.
    Sending,
    /// It left.
    Sent,
    /// The send failed; the text stays for a retry.
    Failed,
    /// The user threw it away.
    Discarded,
}

impl DraftState {
    /// The name the table stores.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Approved => "approved",
            Self::Sending => "sending",
            Self::Sent => "sent",
            Self::Failed => "failed",
            Self::Discarded => "discarded",
        }
    }

    /// The state of a stored name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "proposed" => Some(Self::Proposed),
            "approved" => Some(Self::Approved),
            "sending" => Some(Self::Sending),
            "sent" => Some(Self::Sent),
            "failed" => Some(Self::Failed),
            "discarded" => Some(Self::Discarded),
            _ => None,
        }
    }
}

/// One message the agent proposed for a chat.
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    /// The row, `0` before it is inserted.
    pub id: i64,
    /// The agent that wrote it.
    pub agent_id: String,
    /// The chat it goes to.
    pub chat_id: String,
    /// The message it answers, when it is a reply.
    pub reply_to: Option<String>,
    /// The text.
    pub text: String,
    /// Why the agent wrote it, in one line.
    pub reason: Option<String>,
    /// How sure it is, `0` to `1`.
    pub confidence: Option<f64>,
    /// Where it is in its life.
    pub state: DraftState,
    /// When it was written.
    pub at_ms: i64,
}

/// What a scheduled task does when it fires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    /// It leaves a draft.
    Draft,
    /// It sends.
    Send,
    /// It only reads, for a summary.
    Read,
}

impl Output {
    /// The name the table stores.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Send => "send",
            Self::Read => "read",
        }
    }

    /// The output of a stored name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "draft" => Some(Self::Draft),
            "send" => Some(Self::Send),
            "read" => Some(Self::Read),
            _ => None,
        }
    }
}

/// Whether a task runs every so often or once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// One instant.
    Once,
    /// A cron.
    Cron,
}

impl Kind {
    /// The name the table stores.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Once => "once",
            Self::Cron => "cron",
        }
    }

    /// The kind of a stored name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "once" => Some(Self::Once),
            "cron" => Some(Self::Cron),
            _ => None,
        }
    }
}

/// One automation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Schedule {
    /// The row, `0` before it is inserted.
    pub id: i64,
    /// The agent it runs.
    pub agent_id: String,
    /// The chat it works on.
    pub chat_id: String,
    /// Once or every so often.
    pub kind: Kind,
    /// The instant of a `once` task.
    pub at_ms: Option<i64>,
    /// The expression of a `cron` task.
    pub cron: Option<String>,
    /// What the agent is asked.
    pub prompt: String,
    /// What it does with the answer.
    pub output: Output,
    /// The hours it may fire in.
    pub window: Option<Window>,
    /// The next turn, `None` when it has none left.
    pub next_at_ms: Option<i64>,
    /// Whether the user paused it.
    pub paused: bool,
}

/// One tool call, for the log the user reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    /// The agent that called.
    pub agent_id: String,
    /// The tool: `messages_read`, `message_send`.
    pub tool: String,
    /// The chat it touched, when the tool names one.
    pub chat_id: Option<String>,
    /// Whether it answered.
    pub ok: bool,
    /// How long it took.
    pub ms: i64,
    /// How many bytes the answer had.
    pub bytes: i64,
    /// The error, in one line, when it failed.
    pub detail: Option<String>,
}

/// The database of the profile.
#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

impl Db {
    /// Opens the file, creating it when it does not exist.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        // The sidecar and the window share the file: WAL keeps a read from
        // waiting on a write.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(SCHEMA)?;
        // A profile created by an earlier build has the table without the
        // later columns; `ALTER TABLE` has no `IF NOT EXISTS`, and the error of
        // a column that is already there is the expected one.
        let _ = conn.execute(
            "ALTER TABLE agents ADD COLUMN humanize INTEGER NOT NULL DEFAULT 1",
            [],
        );
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// The grant of one agent, read without opening the database for writing.
    ///
    /// The agent server only needs to know how to speak; it does not create the
    /// file, does not migrate it and does not write to it. A missing file, a
    /// missing row or a column of another version answer `None`, and the
    /// caller keeps the prudent defaults.
    pub fn grant_of(path: &Path, agent: &str) -> Option<Grant> {
        let conn = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                | rusqlite::OpenFlags::SQLITE_OPEN_URI
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .ok()?;
        conn.busy_timeout(Duration::from_millis(500)).ok()?;
        conn.query_row(
            "SELECT scope,chats,per_hour,window,humanize
             FROM agents WHERE id=?1 AND revoked_ms IS NULL",
            params![agent],
            |row| {
                let scope: String = row.get(0)?;
                let chats: String = row.get(1)?;
                let window: Option<String> = row.get(3)?;
                Ok(Grant {
                    scope: Scope::parse(&scope).unwrap_or(Scope::Read),
                    chats: ChatFilter::from_json(&chats),
                    per_hour: row.get(2)?,
                    window: window.as_deref().and_then(Window::parse),
                    humanize: row.get(4).unwrap_or(true),
                })
            },
        )
        .optional()
        .ok()
        .flatten()
    }

    /// The same schema over a database in memory, for the tests.
    pub fn in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// One write, on its own transaction.
    fn write<T>(
        &self,
        work: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> rusqlite::Result<T> {
        let conn = self.conn.lock().expect("agents lock");
        let tx = conn.unchecked_transaction()?;
        let value = work(&tx)?;
        tx.commit()?;
        Ok(value)
    }

    /// One read.
    fn read<T>(
        &self,
        work: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> rusqlite::Result<T> {
        let conn = self.conn.lock().expect("agents lock");
        work(&conn)
    }

    /// Connects an agent: the grant, the client and the hash of its token.
    pub fn connect_agent(
        &self,
        id: &str,
        client: &str,
        grant: &Grant,
        token: &str,
        now_ms: i64,
    ) -> rusqlite::Result<()> {
        let chats = grant.chats.to_json();
        let window = grant.window.map(|window| window.to_string());
        let hash = token_hash(token);
        self.write(|conn| {
            conn.execute(
                "INSERT INTO agents(id,client,scope,chats,per_hour,window,humanize,token_hash,created_ms,revoked_ms)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,NULL)
                 ON CONFLICT(id) DO UPDATE SET
                    client=excluded.client,
                    scope=excluded.scope,
                    chats=excluded.chats,
                    per_hour=excluded.per_hour,
                    window=excluded.window,
                    humanize=excluded.humanize,
                    token_hash=excluded.token_hash,
                    revoked_ms=NULL",
                params![
                    id,
                    client,
                    grant.scope.as_str(),
                    chats,
                    grant.per_hour,
                    window,
                    grant.humanize,
                    hash,
                    now_ms
                ],
            )?;
            Ok(())
        })
    }

    /// Changes what an agent may do without touching its token.
    pub fn set_grant(&self, id: &str, grant: &Grant) -> rusqlite::Result<()> {
        let chats = grant.chats.to_json();
        let window = grant.window.map(|window| window.to_string());
        self.write(|conn| {
            conn.execute(
                "UPDATE agents SET scope=?2, chats=?3, per_hour=?4, window=?5, humanize=?6 WHERE id=?1",
                params![
                    id,
                    grant.scope.as_str(),
                    chats,
                    grant.per_hour,
                    window,
                    grant.humanize
                ],
            )?;
            Ok(())
        })
    }

    /// One agent, when it exists.
    pub fn agent(&self, id: &str) -> rusqlite::Result<Option<AgentRow>> {
        self.read(|conn| {
            conn.query_row(
                "SELECT id,client,scope,chats,per_hour,window,humanize,created_ms,revoked_ms
                 FROM agents WHERE id=?1",
                params![id],
                agent_row,
            )
            .optional()
        })
    }

    /// Every agent the profile knows, revoked ones included.
    pub fn agents(&self) -> rusqlite::Result<Vec<AgentRow>> {
        self.read(|conn| {
            let mut statement = conn.prepare(
                "SELECT id,client,scope,chats,per_hour,window,humanize,created_ms,revoked_ms
                 FROM agents ORDER BY created_ms",
            )?;
            let rows = statement.query_map([], agent_row)?;
            rows.collect()
        })
    }

    /// Disconnects an agent: its token stops working.
    pub fn revoke(&self, id: &str, at_ms: i64) -> rusqlite::Result<()> {
        self.write(|conn| {
            conn.execute(
                "UPDATE agents SET revoked_ms=?2 WHERE id=?1 AND revoked_ms IS NULL",
                params![id, at_ms],
            )?;
            Ok(())
        })
    }

    /// Whether the token is the one this agent was given.
    ///
    /// The comparison is over the two digests, byte by byte, so a wrong token
    /// cannot be guessed by timing.
    pub fn authorize(&self, id: &str, token: &str) -> rusqlite::Result<bool> {
        let stored: Option<String> = self.read(|conn| {
            conn.query_row(
                "SELECT token_hash FROM agents WHERE id=?1 AND revoked_ms IS NULL",
                params![id],
                |row| row.get(0),
            )
            .optional()
        })?;
        let Some(stored) = stored else {
            return Ok(false);
        };
        Ok(constant_time_eq(
            stored.as_bytes(),
            token_hash(token).as_bytes(),
        ))
    }

    /// Appends one tool call to the log and keeps the log short.
    pub fn record_call(&self, call: &Call, at_ms: i64) -> rusqlite::Result<()> {
        self.write(|conn| {
            conn.execute(
                "INSERT INTO agent_calls(at_ms,agent_id,tool,chat_id,ok,ms,bytes,detail)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    at_ms,
                    call.agent_id,
                    call.tool,
                    call.chat_id,
                    call.ok,
                    call.ms,
                    call.bytes,
                    call.detail
                ],
            )?;
            conn.execute(
                "DELETE FROM agent_calls WHERE id <= (SELECT MAX(id) FROM agent_calls) - ?1",
                params![CALLS_KEPT],
            )?;
            Ok(())
        })
    }

    /// The calls of one agent, the newest first.
    pub fn calls(&self, agent_id: &str, limit: u32) -> rusqlite::Result<Vec<Call>> {
        self.read(|conn| {
            let mut statement = conn.prepare(
                "SELECT agent_id,tool,chat_id,ok,ms,bytes,detail
                 FROM agent_calls WHERE agent_id=?1 ORDER BY id DESC LIMIT ?2",
            )?;
            let rows = statement.query_map(params![agent_id, limit], |row| {
                Ok(Call {
                    agent_id: row.get(0)?,
                    tool: row.get(1)?,
                    chat_id: row.get(2)?,
                    ok: row.get(3)?,
                    ms: row.get(4)?,
                    bytes: row.get(5)?,
                    detail: row.get(6)?,
                })
            })?;
            rows.collect()
        })
    }

    /// How many messages the agent sent since the instant, which is what the
    /// ceiling of messages per hour counts.
    pub fn sends_since(&self, agent_id: &str, since_ms: i64) -> rusqlite::Result<u32> {
        self.read(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM agent_calls
                 WHERE agent_id=?1 AND tool='message_send' AND ok=1 AND at_ms>=?2",
                params![agent_id, since_ms],
                |row| row.get(0),
            )
        })
    }

    /// Saves a draft and returns its row.
    pub fn add_draft(&self, draft: &Draft) -> rusqlite::Result<i64> {
        self.write(|conn| {
            conn.execute(
                "INSERT INTO drafts(agent_id,chat_id,reply_to,text,reason,confidence,state,at_ms)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    draft.agent_id,
                    draft.chat_id,
                    draft.reply_to,
                    draft.text,
                    draft.reason,
                    draft.confidence,
                    draft.state.as_str(),
                    draft.at_ms
                ],
            )?;
            Ok(conn.last_insert_rowid())
        })
    }

    /// One draft by its row.
    pub fn draft(&self, id: i64) -> rusqlite::Result<Option<Draft>> {
        self.read(|conn| {
            conn.query_row(
                "SELECT id,agent_id,chat_id,reply_to,text,reason,confidence,state,at_ms
                 FROM drafts WHERE id=?1",
                params![id],
                draft_row,
            )
            .optional()
        })
    }

    /// The drafts in one state, the newest first.
    pub fn drafts(&self, state: DraftState, limit: u32) -> rusqlite::Result<Vec<Draft>> {
        self.read(|conn| {
            let mut statement = conn.prepare(
                "SELECT id,agent_id,chat_id,reply_to,text,reason,confidence,state,at_ms
                 FROM drafts WHERE state=?1 ORDER BY id DESC LIMIT ?2",
            )?;
            let rows = statement.query_map(params![state.as_str(), limit], draft_row)?;
            rows.collect()
        })
    }

    /// Moves a draft to its next state.
    pub fn set_draft_state(&self, id: i64, state: DraftState) -> rusqlite::Result<bool> {
        self.write(|conn| {
            let changed = conn.execute(
                "UPDATE drafts SET state=?2 WHERE id=?1",
                params![id, state.as_str()],
            )?;
            Ok(changed > 0)
        })
    }

    /// Saves an automation and returns its row.
    pub fn add_schedule(&self, schedule: &Schedule) -> rusqlite::Result<i64> {
        self.write(|conn| {
            conn.execute(
                "INSERT INTO schedules(agent_id,chat_id,kind,at_ms,cron,prompt,output,window,next_at_ms,paused)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    schedule.agent_id,
                    schedule.chat_id,
                    schedule.kind.as_str(),
                    schedule.at_ms,
                    schedule.cron,
                    schedule.prompt,
                    schedule.output.as_str(),
                    schedule.window.map(|window| window.to_string()),
                    schedule.next_at_ms,
                    schedule.paused
                ],
            )?;
            Ok(conn.last_insert_rowid())
        })
    }

    /// Every automation, the next one to fire first.
    pub fn schedules(&self) -> rusqlite::Result<Vec<Schedule>> {
        self.read(|conn| {
            let mut statement = conn.prepare(
                "SELECT id,agent_id,chat_id,kind,at_ms,cron,prompt,output,window,next_at_ms,paused
                 FROM schedules ORDER BY COALESCE(next_at_ms, at_ms), id",
            )?;
            let rows = statement.query_map([], schedule_row)?;
            rows.collect()
        })
    }

    /// One automation, when it exists.
    pub fn schedule(&self, id: i64) -> rusqlite::Result<Option<Schedule>> {
        self.read(|conn| {
            let mut statement = conn.prepare(
                "SELECT id,agent_id,chat_id,kind,at_ms,cron,prompt,output,window,next_at_ms,paused
                 FROM schedules WHERE id=?1",
            )?;
            let mut rows = statement.query_map(params![id], schedule_row)?;
            rows.next().transpose()
        })
    }

    /// Rewrites an automation in place, keeping its row.
    pub fn update_schedule(&self, schedule: &Schedule) -> rusqlite::Result<bool> {
        self.write(|conn| {
            let changed = conn.execute(
                "UPDATE schedules
                 SET agent_id=?2,chat_id=?3,kind=?4,at_ms=?5,cron=?6,prompt=?7,output=?8,
                     window=?9,next_at_ms=?10,paused=?11
                 WHERE id=?1",
                params![
                    schedule.id,
                    schedule.agent_id,
                    schedule.chat_id,
                    schedule.kind.as_str(),
                    schedule.at_ms,
                    schedule.cron,
                    schedule.prompt,
                    schedule.output.as_str(),
                    schedule.window.map(|window| window.to_string()),
                    schedule.next_at_ms,
                    schedule.paused
                ],
            )?;
            Ok(changed > 0)
        })
    }

    /// The automations that came due, paused ones excluded.
    pub fn due(&self, now_ms: i64) -> rusqlite::Result<Vec<Schedule>> {
        self.read(|conn| {
            let mut statement = conn.prepare(
                "SELECT id,agent_id,chat_id,kind,at_ms,cron,prompt,output,window,next_at_ms,paused
                 FROM schedules
                 WHERE paused=0 AND next_at_ms IS NOT NULL AND next_at_ms<=?1
                 ORDER BY next_at_ms, id",
            )?;
            let rows = statement.query_map(params![now_ms], schedule_row)?;
            rows.collect()
        })
    }

    /// Writes the next turn of an automation.
    pub fn set_schedule_next(&self, id: i64, next_at_ms: Option<i64>) -> rusqlite::Result<()> {
        self.write(|conn| {
            conn.execute(
                "UPDATE schedules SET next_at_ms=?2 WHERE id=?1",
                params![id, next_at_ms],
            )?;
            Ok(())
        })
    }

    /// Pauses or resumes an automation.
    pub fn pause_schedule(&self, id: i64, paused: bool) -> rusqlite::Result<()> {
        self.write(|conn| {
            conn.execute(
                "UPDATE schedules SET paused=?2 WHERE id=?1",
                params![id, paused],
            )?;
            Ok(())
        })
    }

    /// Deletes an automation.
    pub fn remove_schedule(&self, id: i64) -> rusqlite::Result<()> {
        self.write(|conn| {
            conn.execute("DELETE FROM schedules WHERE id=?1", params![id])?;
            Ok(())
        })
    }
}

/// One row of `agents` as a grant.
fn agent_row(row: &Row<'_>) -> rusqlite::Result<AgentRow> {
    let scope: String = row.get(2)?;
    let chats: String = row.get(3)?;
    let window: Option<String> = row.get(5)?;
    let grant = Grant {
        scope: Scope::parse(&scope).unwrap_or(Scope::Read),
        chats: ChatFilter::from_json(&chats),
        per_hour: row.get(4)?,
        window: window.as_deref().and_then(Window::parse),
        humanize: row.get(6)?,
    };
    Ok(AgentRow {
        id: row.get(0)?,
        client: row.get(1)?,
        grant,
        created_ms: row.get(7)?,
        revoked_ms: row.get(8)?,
    })
}

/// One row of `drafts`.
fn draft_row(row: &Row<'_>) -> rusqlite::Result<Draft> {
    let state: String = row.get(7)?;
    Ok(Draft {
        id: row.get(0)?,
        agent_id: row.get(1)?,
        chat_id: row.get(2)?,
        reply_to: row.get(3)?,
        text: row.get(4)?,
        reason: row.get(5)?,
        confidence: row.get(6)?,
        state: DraftState::parse(&state).unwrap_or(DraftState::Proposed),
        at_ms: row.get(8)?,
    })
}

/// One row of `schedules`.
fn schedule_row(row: &Row<'_>) -> rusqlite::Result<Schedule> {
    let kind: String = row.get(3)?;
    let output: String = row.get(7)?;
    let window: Option<String> = row.get(8)?;
    Ok(Schedule {
        id: row.get(0)?,
        agent_id: row.get(1)?,
        chat_id: row.get(2)?,
        kind: Kind::parse(&kind).unwrap_or(Kind::Once),
        at_ms: row.get(4)?,
        cron: row.get(5)?,
        prompt: row.get(6)?,
        output: Output::parse(&output).unwrap_or(Output::Draft),
        window: window.as_deref().and_then(Window::parse),
        next_at_ms: row.get(9)?,
        paused: row.get(10)?,
    })
}

/// The SHA-256 of a token, as lowercase hex.
fn token_hash(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// Compares two byte strings without stopping at the first difference.
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for (a, b) in left.iter().zip(right) {
        difference |= a ^ b;
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant_all() -> Grant {
        Grant {
            scope: Scope::Send,
            chats: ChatFilter::all(),
            per_hour: 5,
            window: Window::parse("08:00-23:00"),
            humanize: true,
        }
    }

    fn draft(agent: &str, chat: &str, text: &str, at_ms: i64) -> Draft {
        Draft {
            id: 0,
            agent_id: agent.into(),
            chat_id: chat.into(),
            reply_to: None,
            text: text.into(),
            reason: Some("porque sí".into()),
            confidence: Some(0.8),
            state: DraftState::Proposed,
            at_ms,
        }
    }

    fn schedule(agent: &str, chat: &str, next_at_ms: i64) -> Schedule {
        Schedule {
            id: 0,
            agent_id: agent.into(),
            chat_id: chat.into(),
            kind: Kind::Once,
            at_ms: Some(next_at_ms),
            cron: None,
            prompt: "resumí el día".into(),
            output: Output::Draft,
            window: Window::parse("08:00-23:00"),
            next_at_ms: Some(next_at_ms),
            paused: false,
        }
    }

    #[test]
    fn a_connected_agent_round_trips() {
        let db = Db::in_memory().expect("db");
        db.connect_agent("codex", "codex", &grant_all(), "tok", 1_000)
            .expect("connect");
        let agent = db.agent("codex").expect("read").expect("row");
        assert_eq!(agent.id, "codex");
        assert_eq!(agent.grant, grant_all());
        assert!(agent.live());
        assert_eq!(agent.created_ms, 1_000);
    }

    #[test]
    fn the_token_is_stored_as_a_hash() {
        let db = Db::in_memory().expect("db");
        db.connect_agent("codex", "codex", &grant_all(), "secreto", 1)
            .expect("connect");
        let stored: String = db
            .read(|conn| conn.query_row("SELECT token_hash FROM agents", [], |row| row.get(0)))
            .expect("hash");
        assert_ne!(stored, "secreto");
        assert_eq!(stored.len(), 64);
        assert!(
            stored
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        );
    }

    #[test]
    fn the_token_of_an_agent_only_works_for_that_agent() {
        let db = Db::in_memory().expect("db");
        db.connect_agent("codex", "codex", &grant_all(), "uno", 1)
            .expect("connect");
        db.connect_agent("cursor", "cursor", &grant_all(), "dos", 2)
            .expect("connect");
        assert!(db.authorize("codex", "uno").expect("check"));
        assert!(!db.authorize("codex", "dos").expect("check"));
        assert!(!db.authorize("cursor", "uno").expect("check"));
        assert!(!db.authorize("nadie", "uno").expect("check"));
    }

    #[test]
    fn a_revoked_agent_stops_answering() {
        let db = Db::in_memory().expect("db");
        db.connect_agent("codex", "codex", &grant_all(), "tok", 1)
            .expect("connect");
        db.revoke("codex", 5).expect("revoke");
        assert!(!db.authorize("codex", "tok").expect("check"));
        let agent = db.agent("codex").expect("read").expect("row");
        assert!(!agent.live());
    }

    #[test]
    fn connecting_again_lifts_the_revocation() {
        let db = Db::in_memory().expect("db");
        db.connect_agent("codex", "codex", &grant_all(), "uno", 1)
            .expect("connect");
        db.revoke("codex", 5).expect("revoke");
        db.connect_agent("codex", "codex", &grant_all(), "dos", 9)
            .expect("connect");
        assert!(db.authorize("codex", "dos").expect("check"));
        assert!(!db.authorize("codex", "uno").expect("check"));
        assert!(db.agent("codex").expect("read").expect("row").live());
    }

    #[test]
    fn a_grant_change_keeps_the_token() {
        let db = Db::in_memory().expect("db");
        db.connect_agent("codex", "codex", &grant_all(), "tok", 1)
            .expect("connect");
        let narrower = Grant {
            scope: Scope::Read,
            chats: ChatFilter::only(vec!["a@s.whatsapp.net".into()]),
            per_hour: 1,
            window: None,
            humanize: false,
        };
        db.set_grant("codex", &narrower).expect("set");
        assert_eq!(
            db.agent("codex").expect("read").expect("row").grant,
            narrower
        );
        assert!(db.authorize("codex", "tok").expect("check"));
    }

    #[test]
    fn the_calls_are_logged_and_capped() {
        let db = Db::in_memory().expect("db");
        let call = Call {
            agent_id: "codex".into(),
            tool: "messages_read".into(),
            chat_id: Some("a@s.whatsapp.net".into()),
            ok: true,
            ms: 12,
            bytes: 340,
            detail: None,
        };
        db.record_call(&call, 1_000).expect("record");
        let calls = db.calls("codex", 10).expect("read");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], call);
        assert!(db.calls("cursor", 10).expect("read").is_empty());
    }

    #[test]
    fn only_a_successful_send_counts_for_the_ceiling() {
        let db = Db::in_memory().expect("db");
        let ok = Call {
            agent_id: "codex".into(),
            tool: "message_send".into(),
            chat_id: None,
            ok: true,
            ms: 5,
            bytes: 10,
            detail: None,
        };
        let failed = Call {
            ok: false,
            ..ok.clone()
        };
        let read = Call {
            tool: "messages_read".into(),
            ..ok.clone()
        };
        db.record_call(&ok, 1_000).expect("record");
        db.record_call(&ok, 2_000).expect("record");
        db.record_call(&failed, 2_500).expect("record");
        db.record_call(&read, 2_600).expect("record");
        assert_eq!(db.sends_since("codex", 0).expect("count"), 2);
        assert_eq!(db.sends_since("codex", 1_500).expect("count"), 1);
    }

    #[test]
    fn a_draft_round_trips_and_moves() {
        let db = Db::in_memory().expect("db");
        let id = db
            .add_draft(&draft("codex", "a@s.whatsapp.net", "hola", 1_000))
            .expect("add");
        let stored = db.draft(id).expect("read").expect("row");
        assert_eq!(stored.text, "hola");
        assert_eq!(stored.state, DraftState::Proposed);
        assert_eq!(db.drafts(DraftState::Proposed, 10).expect("list").len(), 1);
        assert!(db.set_draft_state(id, DraftState::Sent).expect("move"));
        assert_eq!(db.drafts(DraftState::Proposed, 10).expect("list").len(), 0);
        assert_eq!(
            db.draft(id).expect("read").expect("row").state,
            DraftState::Sent
        );
        assert!(!db.set_draft_state(9_999, DraftState::Sent).expect("move"));
    }

    #[test]
    fn a_schedule_come_due_is_listed_once() {
        let db = Db::in_memory().expect("db");
        let id = db
            .add_schedule(&schedule("codex", "a@s.whatsapp.net", 1_000))
            .expect("add");
        assert!(db.due(999).expect("due").is_empty());
        let due = db.due(1_000).expect("due");
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, id);
        assert_eq!(db.schedules().expect("list").len(), 1);
    }

    #[test]
    fn a_paused_or_finished_schedule_does_not_fire() {
        let db = Db::in_memory().expect("db");
        let id = db
            .add_schedule(&schedule("codex", "a@s.whatsapp.net", 1_000))
            .expect("add");
        db.pause_schedule(id, true).expect("pause");
        assert!(db.due(2_000).expect("due").is_empty());
        db.pause_schedule(id, false).expect("resume");
        assert_eq!(db.due(2_000).expect("due").len(), 1);
        db.set_schedule_next(id, None).expect("finish");
        assert!(db.due(2_000).expect("due").is_empty());
    }

    #[test]
    fn a_deleted_schedule_is_gone() {
        let db = Db::in_memory().expect("db");
        let id = db
            .add_schedule(&schedule("codex", "a@s.whatsapp.net", 1_000))
            .expect("add");
        db.remove_schedule(id).expect("remove");
        assert!(db.schedules().expect("list").is_empty());
    }

    #[test]
    fn the_two_hashes_compare_byte_by_byte() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    #[test]
    fn the_names_of_the_states_round_trip() {
        for state in [
            DraftState::Proposed,
            DraftState::Approved,
            DraftState::Sending,
            DraftState::Sent,
            DraftState::Failed,
            DraftState::Discarded,
        ] {
            assert_eq!(DraftState::parse(state.as_str()), Some(state));
        }
        assert_eq!(DraftState::parse("otro"), None);
        for output in [Output::Draft, Output::Send, Output::Read] {
            assert_eq!(Output::parse(output.as_str()), Some(output));
        }
        for kind in [Kind::Once, Kind::Cron] {
            assert_eq!(Kind::parse(kind.as_str()), Some(kind));
        }
    }
}
