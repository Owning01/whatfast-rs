//! The sidecar's own database: chats, messages, and every companion table
//! the bridge contract needs.
//!
//! The schema is the one the Go sidecar shipped (same tables, same indexes),
//! so the reconciliation and repair logic ported wave by wave reads the same
//! shapes the Go wrote. Two kinds of columns are additions of this port, not
//! of the oracle: the `media_*` attachment group (the Go reused the `image_*`
//! columns for every attachment) and `messages.payload` (the original wire
//! proto, so forwards and resends replay the exact bytes). Timestamps are
//! Unix milliseconds everywhere. One connection, one writer: the bridge
//! serves one desktop, and SQLite in WAL mode serializes the rest.

use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use rusqlite::{Connection, OptionalExtension, Row, params};

/// A row of `chats`, the conversation as the store knows it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChatRow {
    pub jid: String,
    pub preferred_jid: String,
    pub name: String,
    pub last_message_id: String,
    pub last_message_text: String,
    pub last_message_at: i64,
    pub last_message_from_me: bool,
    pub unread_count: i64,
    pub muted_until: i64,
    pub archived: bool,
    pub pinned: bool,
    pub blocked: bool,
    /// The `chat_addresses` alias the conversation arrived under, resolved
    /// at read time.
    pub address_jid: String,
    // Presentation, joined from the contacts table (direct chats).
    pub contact_name: String,
    pub push_name: String,
    pub business_name: String,
    pub phone_number: String,
    /// Whether a group-name backfill already ran for this chat.
    pub named: bool,
}

/// A row of `messages` plus the reactions attached to it.
#[derive(Debug, Clone, Default)]
pub struct MessageRow {
    pub id: String,
    pub chat_jid: String,
    pub transport_jid: String,
    pub sender_jid: String,
    pub text: String,
    pub timestamp: i64,
    pub from_me: bool,
    pub status: i64,
    pub kind: String,
    pub reply_to_id: String,
    pub reply_to_chat_id: String,
    pub edited_at: i64,
    pub revoked: bool,
    pub forwarding_score: i64,
    pub unread: bool,
    /// The reactions the read helpers join in; empty on a fresh write.
    pub reactions: Vec<ReactionRow>,
    /// The wire proto this row came from (or went out as), kept so forwards
    /// and media resends can replay the exact original.
    pub payload: Vec<u8>,
    // The image/sticker columns.
    pub image_mime: String,
    pub image_caption: String,
    pub image_thumbnail: Vec<u8>,
    pub image_local_path: String,
    pub image_direct_path: String,
    pub image_media_key: Vec<u8>,
    pub image_file_sha256: Vec<u8>,
    pub image_file_enc_sha256: Vec<u8>,
    pub image_width: i64,
    pub image_height: i64,
    pub image_size: i64,
    pub image_animated: bool,
    // The attachment columns (video, audio, document).
    pub media_mime: String,
    pub media_file_name: String,
    pub media_local_path: String,
    pub media_direct_path: String,
    pub media_media_key: Vec<u8>,
    pub media_file_sha256: Vec<u8>,
    pub media_file_enc_sha256: Vec<u8>,
    pub media_width: i64,
    pub media_height: i64,
    pub media_size: i64,
    pub media_duration: i64,
    pub media_voice: bool,
    pub media_animated: bool,
    pub media_caption: String,
    // Contacts, location, link preview.
    pub contacts_json: String,
    pub location_lat: f64,
    pub location_lng: f64,
    pub location_name: String,
    pub location_address: String,
    pub location_url: String,
    pub location_live: bool,
    pub link_preview_url: String,
    pub link_preview_title: String,
    pub link_preview_description: String,
    pub link_preview_thumbnail: Vec<u8>,
    pub link_preview_width: i64,
    pub link_preview_height: i64,
}

/// One reaction attached to a message.
#[derive(Debug, Clone, Default)]
pub struct ReactionRow {
    pub chat_jid: String,
    pub message_id: String,
    pub sender_jid: String,
    pub emoji: String,
    pub timestamp: i64,
    pub from_me: bool,
}

/// The store: schema owner and the only writer of the profile's chats.db.
#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

/// The schema, ported verbatim from the Go sidecar's migration. A fresh
/// database is created at the final shape; the Go's incremental ALTERs only
/// exist for databases it already created.
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS schema_version(version INTEGER NOT NULL);
INSERT INTO schema_version(version) SELECT 2 WHERE NOT EXISTS(SELECT 1 FROM schema_version);
CREATE TABLE IF NOT EXISTS chats(
  jid TEXT PRIMARY KEY,
  preferred_jid TEXT NOT NULL,
  name TEXT NOT NULL DEFAULT '',
  last_message_id TEXT NOT NULL DEFAULT '',
  last_message_text TEXT NOT NULL DEFAULT '',
  last_message_at INTEGER NOT NULL DEFAULT 0,
  unread_count INTEGER NOT NULL DEFAULT 0,
  muted_until INTEGER NOT NULL DEFAULT 0,
  archived INTEGER NOT NULL DEFAULT 0,
  pinned INTEGER NOT NULL DEFAULT 0,
  blocked INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS chats_last_message_idx ON chats(last_message_at DESC, jid DESC);
CREATE TABLE IF NOT EXISTS chat_addresses(
  jid TEXT PRIMARY KEY,
  chat_jid TEXT NOT NULL,
  last_seen_at INTEGER NOT NULL DEFAULT 0,
  FOREIGN KEY(chat_jid) REFERENCES chats(jid) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS chat_addresses_chat_idx ON chat_addresses(chat_jid);
CREATE TABLE IF NOT EXISTS chat_redirects(
  old_chat_jid TEXT PRIMARY KEY,
  new_chat_jid TEXT NOT NULL,
  FOREIGN KEY(new_chat_jid) REFERENCES chats(jid) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS messages(
  id TEXT NOT NULL,
  chat_jid TEXT NOT NULL,
  transport_jid TEXT NOT NULL,
  sender_jid TEXT NOT NULL DEFAULT '',
  text TEXT NOT NULL DEFAULT '',
  timestamp INTEGER NOT NULL,
  from_me INTEGER NOT NULL DEFAULT 0,
  status INTEGER NOT NULL DEFAULT 0,
  kind TEXT NOT NULL DEFAULT 'text',
  reply_to_id TEXT NOT NULL DEFAULT '',
  reply_to_chat_id TEXT NOT NULL DEFAULT '',
  edited_at INTEGER NOT NULL DEFAULT 0,
  revoked INTEGER NOT NULL DEFAULT 0,
  forwarding_score INTEGER NOT NULL DEFAULT 0,
  unread INTEGER NOT NULL DEFAULT 0,
  image_mime TEXT NOT NULL DEFAULT '',
  image_caption TEXT NOT NULL DEFAULT '',
  image_thumbnail BLOB NOT NULL DEFAULT X'',
  image_local_path TEXT NOT NULL DEFAULT '',
  image_direct_path TEXT NOT NULL DEFAULT '',
  image_media_key BLOB NOT NULL DEFAULT X'',
  image_file_sha256 BLOB NOT NULL DEFAULT X'',
  image_file_enc_sha256 BLOB NOT NULL DEFAULT X'',
  image_width INTEGER NOT NULL DEFAULT 0,
  image_height INTEGER NOT NULL DEFAULT 0,
  image_size INTEGER NOT NULL DEFAULT 0,
  image_animated INTEGER NOT NULL DEFAULT 0,
  media_mime TEXT NOT NULL DEFAULT '',
  media_file_name TEXT NOT NULL DEFAULT '',
  media_local_path TEXT NOT NULL DEFAULT '',
  media_direct_path TEXT NOT NULL DEFAULT '',
  media_media_key BLOB NOT NULL DEFAULT X'',
  media_file_sha256 BLOB NOT NULL DEFAULT X'',
  media_file_enc_sha256 BLOB NOT NULL DEFAULT X'',
  media_width INTEGER NOT NULL DEFAULT 0,
  media_height INTEGER NOT NULL DEFAULT 0,
  media_size INTEGER NOT NULL DEFAULT 0,
  media_duration INTEGER NOT NULL DEFAULT 0,
  media_voice INTEGER NOT NULL DEFAULT 0,
  media_animated INTEGER NOT NULL DEFAULT 0,
  media_caption TEXT NOT NULL DEFAULT '',
  contacts_json TEXT NOT NULL DEFAULT '',
  location_lat REAL NOT NULL DEFAULT 0,
  location_lng REAL NOT NULL DEFAULT 0,
  location_name TEXT NOT NULL DEFAULT '',
  location_address TEXT NOT NULL DEFAULT '',
  location_url TEXT NOT NULL DEFAULT '',
  location_live INTEGER NOT NULL DEFAULT 0,
  link_preview_url TEXT NOT NULL DEFAULT '',
  link_preview_title TEXT NOT NULL DEFAULT '',
  link_preview_description TEXT NOT NULL DEFAULT '',
  link_preview_thumbnail BLOB NOT NULL DEFAULT X'',
  link_preview_width INTEGER NOT NULL DEFAULT 0,
  link_preview_height INTEGER NOT NULL DEFAULT 0,
  payload BLOB NOT NULL DEFAULT X'',
  PRIMARY KEY(chat_jid, id),
  FOREIGN KEY(chat_jid) REFERENCES chats(jid) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS messages_page_idx ON messages(chat_jid, timestamp DESC, id DESC);
CREATE INDEX IF NOT EXISTS messages_unread_cursor_idx ON messages(chat_jid,unread,timestamp,id);
CREATE TABLE IF NOT EXISTS newsletters(
  jid TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  subscribed_at INTEGER NOT NULL,
  muted INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS messages_sticker_idx ON messages(timestamp DESC, id DESC) WHERE kind='sticker';
CREATE TABLE IF NOT EXISTS reactions(
  chat_jid TEXT NOT NULL,
  message_id TEXT NOT NULL,
  sender_jid TEXT NOT NULL,
  emoji TEXT NOT NULL,
  timestamp INTEGER NOT NULL,
  from_me INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY(chat_jid,message_id,sender_jid)
);
CREATE INDEX IF NOT EXISTS reactions_message_idx ON reactions(chat_jid,message_id,timestamp,sender_jid);
CREATE TABLE IF NOT EXISTS polls(
  chat_jid TEXT NOT NULL, message_id TEXT NOT NULL, question TEXT NOT NULL,
  selectable_count INTEGER NOT NULL DEFAULT 1, total_voters INTEGER NOT NULL DEFAULT 0, options_json TEXT NOT NULL,
  snapshot_at INTEGER NOT NULL DEFAULT 0,
  creator_jid TEXT NOT NULL DEFAULT '',
  message_secret BLOB NOT NULL DEFAULT x'',
  PRIMARY KEY(chat_jid,message_id),
  FOREIGN KEY(chat_jid,message_id) REFERENCES messages(chat_jid,id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS poll_votes(
  chat_jid TEXT NOT NULL, poll_message_id TEXT NOT NULL, voter_jid TEXT NOT NULL,
  selected_json TEXT NOT NULL, timestamp INTEGER NOT NULL, from_me INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY(chat_jid,poll_message_id,voter_jid),
  FOREIGN KEY(chat_jid,poll_message_id) REFERENCES polls(chat_jid,message_id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS pinned_messages(
  chat_jid TEXT NOT NULL, message_id TEXT NOT NULL, pinned_at INTEGER NOT NULL, pinned_by TEXT NOT NULL DEFAULT '', pinned INTEGER NOT NULL DEFAULT 1,
  PRIMARY KEY(chat_jid,message_id), FOREIGN KEY(chat_jid) REFERENCES chats(jid) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS contacts(
  jid TEXT PRIMARY KEY,
  contact_name TEXT NOT NULL DEFAULT '',
  push_name TEXT NOT NULL DEFAULT '',
  business_name TEXT NOT NULL DEFAULT '',
  phone_number TEXT NOT NULL DEFAULT '',
  updated_at INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS sync_state(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS storage_health(
  singleton INTEGER PRIMARY KEY CHECK(singleton=1),
  last_message_persisted_at INTEGER NOT NULL DEFAULT 0,
  last_reconciliation_at INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS inbound_messages(
  journal_id INTEGER PRIMARY KEY AUTOINCREMENT,
  chat_jid TEXT NOT NULL,
  message_id TEXT NOT NULL,
  payload BLOB NOT NULL,
  increment_unread INTEGER NOT NULL DEFAULT 0,
  attempts INTEGER NOT NULL DEFAULT 0,
  next_attempt_at INTEGER NOT NULL DEFAULT 0,
  last_error TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS inbound_messages_retry_idx ON inbound_messages(next_attempt_at,journal_id);
CREATE TABLE IF NOT EXISTS inbound_events(
  journal_id INTEGER PRIMARY KEY AUTOINCREMENT,
  event_kind TEXT NOT NULL,
  chat_jid TEXT NOT NULL,
  entity_id TEXT NOT NULL DEFAULT '',
  payload BLOB NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0,
  next_attempt_at INTEGER NOT NULL DEFAULT 0,
  last_error TEXT NOT NULL DEFAULT '',
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS inbound_events_retry_idx ON inbound_events(next_attempt_at,journal_id);
CREATE TABLE IF NOT EXISTS history_coverage(
  chat_jid TEXT PRIMARY KEY,
  state TEXT NOT NULL DEFAULT 'unknown',
  revision INTEGER NOT NULL DEFAULT 0,
  on_demand_added INTEGER NOT NULL DEFAULT 0,
  last_requested_count INTEGER NOT NULL DEFAULT 0,
  last_received_count INTEGER NOT NULL DEFAULT 0,
  last_added_count INTEGER NOT NULL DEFAULT 0,
  last_requested_at INTEGER NOT NULL DEFAULT 0,
  retry_after INTEGER NOT NULL DEFAULT 0,
  detail TEXT NOT NULL DEFAULT '',
  FOREIGN KEY(chat_jid) REFERENCES chats(jid) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS outgoing_requests(
  client_request_id TEXT PRIMARY KEY,
  chat_jid TEXT NOT NULL,
  text TEXT NOT NULL,
  message_id TEXT NOT NULL UNIQUE,
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS outgoing_reactions(
  client_reaction_id TEXT PRIMARY KEY,
  chat_jid TEXT NOT NULL,
  message_id TEXT NOT NULL,
  emoji TEXT NOT NULL,
  completed_at INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS reaction_repair_jobs(
  chat_jid TEXT PRIMARY KEY,
  transport_jid TEXT NOT NULL,
  anchor_message_id TEXT NOT NULL,
  anchor_timestamp INTEGER NOT NULL,
  anchor_from_me INTEGER NOT NULL DEFAULT 0,
  requested_at INTEGER NOT NULL DEFAULT 0,
  attempts INTEGER NOT NULL DEFAULT 0,
  completed_at INTEGER NOT NULL DEFAULT 0,
  legacy_reaction_at INTEGER NOT NULL DEFAULT 0,
  legacy_reaction_id TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS legacy_reaction_replays(
  chat_jid TEXT NOT NULL,
  transport_jid TEXT NOT NULL,
  event_message_id TEXT NOT NULL,
  sender_jid TEXT NOT NULL,
  timestamp INTEGER NOT NULL,
  from_me INTEGER NOT NULL DEFAULT 0,
  status INTEGER NOT NULL DEFAULT 0,
  request_id TEXT NOT NULL DEFAULT '',
  requested_at INTEGER NOT NULL DEFAULT 0,
  attempts INTEGER NOT NULL DEFAULT 0,
  completed_at INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY(chat_jid,event_message_id)
);
CREATE INDEX IF NOT EXISTS legacy_reaction_replays_request_idx ON legacy_reaction_replays(request_id);
CREATE TABLE IF NOT EXISTS sticker_favorites(
  id TEXT PRIMARY KEY,
  mime_type TEXT NOT NULL DEFAULT '',
  direct_path TEXT NOT NULL DEFAULT '',
  media_key BLOB NOT NULL DEFAULT X'',
  file_enc_sha256 BLOB NOT NULL DEFAULT X'',
  width INTEGER NOT NULL DEFAULT 0,
  height INTEGER NOT NULL DEFAULT 0,
  animated INTEGER NOT NULL DEFAULT 0,
  file_size INTEGER NOT NULL DEFAULT 0,
  local_path TEXT NOT NULL DEFAULT '',
  updated_at INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS sticker_favorites_updated_idx ON sticker_favorites(updated_at DESC, id DESC);
"#;

impl Store {
    /// Opens (creating if needed) the profile's chats.db.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        // The Go sidecar waited five seconds on a busy database instead of
        // failing a write; the repair passes can overlap a long history batch.
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(SCHEMA)?;
        // The poll key material arrived after the table: profiles created by
        // older builds lack the columns, and SQLite has no ADD COLUMN IF NOT
        // EXISTS, so a duplicate-column error just means the column is there.
        for ddl in [
            "ALTER TABLE polls ADD COLUMN creator_jid TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE polls ADD COLUMN message_secret BLOB NOT NULL DEFAULT x''",
        ] {
            if let Err(error) = conn.execute_batch(ddl)
                && !error.to_string().contains("duplicate column name")
            {
                return Err(error);
            }
        }
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Runs one write closure atomically; the connection is locked for its
    /// duration, so a write never interleaves with a read.
    pub fn write<T>(
        &self,
        work: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> rusqlite::Result<T> {
        let conn = self.conn.lock().expect("store lock");
        let tx = conn.unchecked_transaction()?;
        let value = work(&tx)?;
        tx.commit()?;
        Ok(value)
    }

    /// Runs one read closure. Reads take the same lock as writes: the
    /// volumes are small and the bridge is the only client.
    pub fn read<T>(
        &self,
        work: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> rusqlite::Result<T> {
        let conn = self.conn.lock().expect("store lock");
        work(&conn)
    }

    // ------------------------------------------------------------------
    // Chats
    // ------------------------------------------------------------------

    /// Inserts the conversation when missing. The Go contract: a chat row
    /// never loses its display state to an upsert of only the identity.
    ///
    /// Unlike `EnsureConversation`, the preferred address of an existing chat
    /// is left alone: this port keys the chat by its address, so switching it
    /// to the hidden-identity alias would unhook the contacts join. The
    /// reconciliation pass owns the fold.
    pub fn ensure_chat(&self, jid: &str, preferred_jid: &str) -> rusqlite::Result<()> {
        self.write(|conn| ensure_chat_in(conn, jid, preferred_jid))
    }

    /// One chat row with its derived `last_message_from_me` and the address
    /// alias, or `None` when the jid is unknown.
    ///
    /// The lookup is exact on purpose: this port's chat id *is* its address,
    /// the reconciliation deletes the retired row, and callers that hold an
    /// alias normalize it with `resolve_chat_id` first. (The Go `Chat`
    /// resolved inside the read because its ids were opaque.)
    pub fn chat(&self, jid: &str) -> rusqlite::Result<Option<ChatRow>> {
        self.read(|conn| {
            conn.query_row(
                &format!("{CHAT_SELECT} WHERE c.jid=?1"),
                params![jid],
                chat_from_row,
            )
            .optional()
        })
    }

    /// One page of chats, newest first, with the total and the exclusive
    /// cursor of the next page ("at:jid" of the last row, empty when done).
    ///
    /// Status updates and channel subscriptions never surface as chat rows:
    /// they have their own surfaces in the client, exactly like the Go
    /// `Chats` query.
    pub fn chats_page(
        &self,
        cursor_at: i64,
        cursor_jid: &str,
        limit: u32,
    ) -> rusqlite::Result<(Vec<ChatRow>, u64, String)> {
        let limit = clamp_limit(limit);
        self.read(|conn| {
            // The Go answered the page from the filtered query but the total
            // from ChatCount, which counts every stored conversation.
            let total: i64 = conn.query_row("SELECT COUNT(*) FROM chats", [], |row| row.get(0))?;
            // One extra row tells whether a next page exists; the Go asked
            // for limit+1 and only then built a cursor.
            let mut statement = conn.prepare(&format!(
                "{CHAT_SELECT} WHERE c.preferred_jid NOT IN ('status@broadcast')
                    AND c.preferred_jid NOT LIKE '%@newsletter'
                    AND ((?1 = 0 AND ?2 = '')
                      OR (c.last_message_at < ?1)
                      OR (c.last_message_at = ?1 AND c.jid < ?2))
                 ORDER BY c.last_message_at DESC, c.jid DESC LIMIT ?3"
            ))?;
            let mut rows = statement
                .query_map(params![cursor_at, cursor_jid, limit + 1], chat_from_row)?
                .collect::<Result<Vec<_>, _>>()?;
            let has_more = rows.len() as u32 > limit;
            if has_more {
                rows.truncate(limit as usize);
            }
            let next_cursor = if has_more {
                rows.last()
                    .map(|chat| format!("{}:{}", chat.last_message_at, chat.jid))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            Ok((rows, total as u64, next_cursor))
        })
    }

    /// Sets the chat display flags the desktop owns (mute/archive/pin/block
    /// live in this profile: the linked-device transport does not expose
    /// app-state for them). A flag set before the first message creates the
    /// conversation, like the Go `SetChatPinned`/`SetChatBlocked`.
    pub fn set_chat_flag(&self, jid: &str, column: ChatFlag, value: bool) -> rusqlite::Result<()> {
        let jid = self.resolve_chat_id(jid)?;
        self.write(|conn| {
            conn.execute(
                "INSERT INTO chats(jid,preferred_jid) VALUES(?1,?1)
                 ON CONFLICT(jid) DO NOTHING",
                params![jid],
            )?;
            conn.execute(
                "INSERT INTO chat_addresses(jid,chat_jid,last_seen_at) VALUES(?1,?1,?2)
                 ON CONFLICT(jid) DO NOTHING",
                params![jid, now_ms()],
            )?;
            conn.execute(
                &format!("UPDATE chats SET {}=?1 WHERE jid=?2", column.as_str()),
                params![value as i64, jid],
            )?;
            Ok(())
        })
    }

    /// Records a mute-until instant (zero unmutes), creating the conversation
    /// when the mute arrives before its first message.
    pub fn set_chat_mute(&self, jid: &str, mute_until: i64) -> rusqlite::Result<()> {
        let jid = self.resolve_chat_id(jid)?;
        self.write(|conn| {
            conn.execute(
                "INSERT INTO chats(jid,preferred_jid) VALUES(?1,?1)
                 ON CONFLICT(jid) DO NOTHING",
                params![jid],
            )?;
            conn.execute(
                "INSERT INTO chat_addresses(jid,chat_jid,last_seen_at) VALUES(?1,?1,?2)
                 ON CONFLICT(jid) DO NOTHING",
                params![jid, now_ms()],
            )?;
            conn.execute(
                "UPDATE chats SET muted_until=?1 WHERE jid=?2",
                params![mute_until, jid],
            )?;
            Ok(())
        })
    }

    // ------------------------------------------------------------------
    // Messages
    // ------------------------------------------------------------------

    /// Applies one message through the reducer the Go sidecar used for live
    /// and history rows: a re-delivery fills blanks and wins mutations
    /// without erasing newer state, the chat pointer only moves forward, and
    /// the unread counter grows once per new incoming row.
    ///
    /// Returns true when the row is new (a replay is not).
    pub fn insert_message(&self, message: &MessageRow) -> rusqlite::Result<bool> {
        if message.kind == "reaction" {
            // Legacy reaction pseudo-messages are events, not content; the Go
            // reducer never stored them as message rows.
            self.ensure_chat(&message.chat_jid, &message.chat_jid)?;
            return Ok(false);
        }
        // The Go reducer stamped an event that carried no time when it
        // reduced it; a zero timestamp must not land in the timeline.
        let timestamp = if message.timestamp == 0 {
            now_ms()
        } else {
            message.timestamp
        };
        self.write(|conn| {
            if let Some(old) = conn
                .query_row(
                    &format!("{MESSAGE_SELECT} WHERE m.chat_jid=?1 AND m.id=?2"),
                    params![message.chat_jid, message.id],
                    message_from_row,
                )
                .optional()?
            {
                let merged = merge_message(&old, message, timestamp);
                update_message_row(conn, &merged)?;
                return finish_message_write(conn, &merged, merged.timestamp, false);
            }
            // The insert runs through a closure so a foreign-key miss can
            // adopt the chat and retry once instead of losing the write.
            let insert = |conn: &Connection| {
                conn.execute(
                "INSERT OR IGNORE INTO messages(
                   id,chat_jid,transport_jid,sender_jid,text,timestamp,from_me,status,kind,
                   reply_to_id,reply_to_chat_id,edited_at,revoked,forwarding_score,unread,
                   image_mime,image_caption,image_thumbnail,image_local_path,image_direct_path,
                   image_media_key,image_file_sha256,image_file_enc_sha256,image_width,image_height,image_size,image_animated,
                   media_mime,media_file_name,media_local_path,media_direct_path,media_media_key,
                   media_file_sha256,media_file_enc_sha256,media_width,media_height,media_size,
                   media_duration,media_voice,media_animated,media_caption,
                   contacts_json,location_lat,location_lng,location_name,location_address,location_url,location_live,
                   link_preview_url,link_preview_title,link_preview_description,link_preview_thumbnail,link_preview_width,link_preview_height,payload
                 ) VALUES(
                   ?1,?2,?3,?4,?5,?6,?7,?8,?9,
                   ?10,?11,?12,?13,?14,?15,
                   ?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,
                   ?28,?29,?30,?31,?32,?33,?34,?35,?36,?37,?38,?39,?40,?41,
                   ?42,?43,?44,?45,?46,?47,?48,
                   ?49,?50,?51,?52,?53,?54,?55
                 )",
                params![
                    message.id, message.chat_jid, message.transport_jid, message.sender_jid,
                    message.text, timestamp, message.from_me as i64, message.status,
                    message.kind, message.reply_to_id, message.reply_to_chat_id, message.edited_at,
                    message.revoked as i64, message.forwarding_score, message.unread as i64,
                    message.image_mime, message.image_caption, message.image_thumbnail,
                    message.image_local_path, message.image_direct_path, message.image_media_key,
                    message.image_file_sha256, message.image_file_enc_sha256, message.image_width,
                    message.image_height, message.image_size, message.image_animated as i64,
                    message.media_mime, message.media_file_name, message.media_local_path,
                    message.media_direct_path, message.media_media_key, message.media_file_sha256,
                    message.media_file_enc_sha256, message.media_width, message.media_height,
                    message.media_size, message.media_duration, message.media_voice as i64,
                    message.media_animated as i64, message.media_caption, message.contacts_json,
                    message.location_lat, message.location_lng, message.location_name,
                    message.location_address, message.location_url, message.location_live as i64,
                    message.link_preview_url, message.link_preview_title,
                    message.link_preview_description, message.link_preview_thumbnail,
                    message.link_preview_width, message.link_preview_height,
                    message.payload
                ],
            )};
            let changed = match insert(conn) {
                Ok(changed) => changed,
                // `INSERT OR IGNORE` does not ignore a foreign-key miss: the
                // error is hard. Adopt the chat and retry once instead of
                // losing the write; anything else propagates.
                Err(error) if is_missing_chat(&error) => {
                    ensure_chat_in(conn, &message.chat_jid, &message.chat_jid)?;
                    insert(conn)?
                }
                Err(error) => return Err(error),
            };
            if changed == 0 {
                return Ok(false);
            }
            finish_message_write(conn, message, timestamp, true)
        })
    }

    /// The unread counters: `messages.unread` marks the rows, `chats.unread_count`
    /// caches the total the list shows. Port note: the Go `MarkRead` only
    /// zeroed the counter and left the rows marked, which its own integrity
    /// check counts as an inconsistency; this port clears both so derived
    /// state stays consistent (the Go reached the same end through
    /// `MarkReadIDs`/`MarkReadThrough`).
    pub fn mark_chat_read(&self, chat_jid: &str) -> rusqlite::Result<u32> {
        self.write(|conn| {
            let changed = conn.execute(
                "UPDATE messages SET unread=0 WHERE chat_jid=?1 AND unread=1",
                params![chat_jid],
            )?;
            conn.execute(
                "UPDATE chats SET unread_count=0 WHERE jid=?1",
                params![chat_jid],
            )?;
            Ok(changed as u32)
        })
    }

    /// One bounded page, oldest-to-newest, ending strictly before the
    /// (timestamp,id) cursor. (0,"") means the newest page.
    pub fn messages_before(
        &self,
        chat_jid: &str,
        before_ts: i64,
        before_id: &str,
        limit: u32,
    ) -> rusqlite::Result<Vec<MessageRow>> {
        let limit = clamp_limit(limit);
        self.read(|conn| {
            let mut statement = conn.prepare(&format!(
                "{MESSAGE_SELECT}
                 WHERE m.chat_jid=?1
                   AND m.kind<>'reaction'
                   AND (?2 = 0 OR m.timestamp < ?2 OR (m.timestamp = ?2 AND m.id < ?3))
                 ORDER BY m.timestamp DESC, m.id DESC LIMIT ?4"
            ))?;
            let mut rows = statement
                .query_map(
                    params![chat_jid, before_ts, before_id, limit],
                    message_from_row,
                )?
                .collect::<Result<Vec<_>, _>>()?;
            attach_reactions(conn, &mut rows)?;
            rows.reverse();
            Ok(rows)
        })
    }

    /// Ascending page strictly after the cursor. Both cursor halves are
    /// required, like the Go `MessagesAfter`: without them the call is a bug,
    /// not the newest page.
    pub fn messages_after(
        &self,
        chat_jid: &str,
        after_ts: i64,
        after_id: &str,
        limit: u32,
    ) -> rusqlite::Result<(Vec<MessageRow>, bool)> {
        if after_ts <= 0 || after_id.is_empty() {
            return Err(invalid_parameter(
                "after timestamp and message ID are required",
            ));
        }
        let limit = clamp_limit(limit);
        self.read(|conn| {
            let mut statement = conn.prepare(&format!(
                "{MESSAGE_SELECT}
                 WHERE m.chat_jid=?1
                   AND m.kind<>'reaction'
                   AND (m.timestamp > ?2 OR (m.timestamp = ?2 AND m.id > ?3))
                 ORDER BY m.timestamp ASC, m.id ASC LIMIT ?4"
            ))?;
            let mut rows = statement
                .query_map(
                    params![chat_jid, after_ts, after_id, limit + 1],
                    message_from_row,
                )?
                .collect::<Result<Vec<_>, _>>()?;
            attach_reactions(conn, &mut rows)?;
            let has_more = rows.len() as u32 > limit;
            if has_more {
                rows.truncate(limit as usize);
            }
            Ok((rows, has_more))
        })
    }

    /// The window the Go `MessagesAround` built: `side_limit` messages
    /// strictly older, the anchor, and `side_limit` strictly newer, in
    /// oldest-to-newest order. The side limit defaults to 25 and never
    /// exceeds 50, like the oracle.
    pub fn messages_around(
        &self,
        chat_jid: &str,
        message_id: &str,
        side_limit: u32,
    ) -> rusqlite::Result<Option<(Vec<MessageRow>, bool, bool)>> {
        let side_limit = if side_limit == 0 || side_limit > 50 {
            25
        } else {
            side_limit
        };
        let Some(anchor) = self.message(chat_jid, message_id)? else {
            return Ok(None);
        };
        let chat_jid = anchor.chat_jid.clone();
        let anchor_ts = anchor.timestamp;
        self.read(|conn| {
            let mut older = conn.prepare(&format!(
                "{MESSAGE_SELECT}
                 WHERE m.chat_jid=?1 AND m.kind<>'reaction'
                   AND (m.timestamp < ?2 OR (m.timestamp = ?2 AND m.id < ?3))
                 ORDER BY m.timestamp DESC, m.id DESC LIMIT ?4"
            ))?;
            let mut older_rows = older
                .query_map(
                    params![&chat_jid, anchor_ts, &anchor.id, side_limit + 1],
                    message_from_row,
                )?
                .collect::<Result<Vec<_>, _>>()?;
            let has_older = older_rows.len() as u32 > side_limit;
            older_rows.truncate(side_limit as usize);
            let mut newer = conn.prepare(&format!(
                "{MESSAGE_SELECT}
                 WHERE m.chat_jid=?1 AND m.kind<>'reaction'
                   AND (m.timestamp > ?2 OR (m.timestamp = ?2 AND m.id > ?3))
                 ORDER BY m.timestamp ASC, m.id ASC LIMIT ?4"
            ))?;
            let mut newer_rows = newer
                .query_map(
                    params![&chat_jid, anchor_ts, &anchor.id, side_limit + 1],
                    message_from_row,
                )?
                .collect::<Result<Vec<_>, _>>()?;
            let has_newer = newer_rows.len() as u32 > side_limit;
            newer_rows.truncate(side_limit as usize);
            let mut all = Vec::with_capacity(older_rows.len() + 1 + newer_rows.len());
            older_rows.reverse();
            all.extend(older_rows);
            all.push(anchor);
            all.extend(newer_rows);
            attach_reactions(conn, &mut all)?;
            Ok(Some((all, has_older, has_newer)))
        })
    }

    /// One stored message with its reactions, or none. The chat lookup is
    /// exact, like `chat`: callers normalize aliases first.
    pub fn message(
        &self,
        chat_jid: &str,
        message_id: &str,
    ) -> rusqlite::Result<Option<MessageRow>> {
        let message = self.read(|conn| {
            conn.query_row(
                &format!("{MESSAGE_SELECT} WHERE m.chat_jid=?1 AND m.id=?2 AND m.kind<>'reaction'"),
                params![chat_jid, message_id],
                message_from_row,
            )
            .optional()
        })?;
        let Some(mut message) = message else {
            return Ok(None);
        };
        self.read(|conn| attach_reactions(conn, std::slice::from_mut(&mut message)))?;
        Ok(Some(message))
    }

    /// Records an edit the way the Go reducer did: the stored timestamp stays
    /// put, a revoked row never accepts new text, and the chat preview follows
    /// when the edited row is the one the pointer names.
    pub fn mark_message_edited(
        &self,
        chat_jid: &str,
        message_id: &str,
        text: &str,
    ) -> rusqlite::Result<()> {
        let chat_jid = self.resolve_chat_id(chat_jid)?;
        self.write(|conn| {
            let changed = conn.execute(
                "UPDATE messages SET text=?1, edited_at=?2
                 WHERE chat_jid=?3 AND id=?4 AND revoked=0",
                params![text, now_ms(), chat_jid, message_id],
            )?;
            if changed > 0 {
                conn.execute(
                    "UPDATE chats SET last_message_text=COALESCE((SELECT CASE
                        WHEN text<>'' THEN text
                        WHEN image_caption<>'' THEN image_caption
                        WHEN media_caption<>'' THEN media_caption
                        ELSE '' END
                      FROM messages WHERE chat_jid=?1 AND id=?2),'')
                     WHERE jid=?1 AND last_message_id=?2",
                    params![chat_jid, message_id],
                )?;
            }
            Ok(())
        })
    }

    pub fn mark_message_revoked(&self, chat_jid: &str, message_id: &str) -> rusqlite::Result<()> {
        let chat_jid = self.resolve_chat_id(chat_jid)?;
        self.write(|conn| {
            conn.execute(
                "UPDATE messages SET revoked=1, kind='revoked' WHERE chat_jid=?1 AND id=?2",
                params![chat_jid, message_id],
            )?;
            Ok(())
        })
    }

    /// Points one message's attachment (a video, an audio or a document) at
    /// the cache file the download wrote. Only attachment rows accept it.
    pub fn set_media_local_path(
        &self,
        chat_jid: &str,
        message_id: &str,
        path: &str,
    ) -> rusqlite::Result<()> {
        self.set_kind_local_path(
            chat_jid,
            message_id,
            path,
            "media_local_path",
            "('video','audio','document')",
        )
    }

    /// Points one message's image or sticker at the cache file the download
    /// wrote: the full-resolution entry the image viewer opens.
    pub fn set_image_local_path(
        &self,
        chat_jid: &str,
        message_id: &str,
        path: &str,
    ) -> rusqlite::Result<()> {
        self.set_kind_local_path(
            chat_jid,
            message_id,
            path,
            "image_local_path",
            "('image','sticker')",
        )
    }

    fn set_kind_local_path(
        &self,
        chat_jid: &str,
        message_id: &str,
        path: &str,
        column: &str,
        kinds: &str,
    ) -> rusqlite::Result<()> {
        let chat_jid = self.resolve_chat_id(chat_jid)?;
        self.write(|conn| {
            let changed = conn.execute(
                &format!(
                    "UPDATE messages SET {column}=?1
                     WHERE chat_jid=?2 AND id=?3 AND kind IN {kinds}"
                ),
                params![path, chat_jid, message_id],
            )?;
            if changed == 0 {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            Ok(())
        })
    }

    /// Advances one message's delivery status with the Go transition table:
    /// FAILED overrides only a pending or unknown row, a failed row is
    /// recovered by the next real receipt, and nothing else ever goes
    /// backwards. Answers whether the stored status actually changed.
    pub fn set_message_status(
        &self,
        chat_jid: &str,
        message_id: &str,
        status: i64,
    ) -> rusqlite::Result<bool> {
        let chat_jid = self.resolve_chat_id(chat_jid)?;
        self.write(|conn| {
            let current: Option<i64> = conn
                .query_row(
                    "SELECT status FROM messages WHERE chat_jid=?1 AND id=?2",
                    params![chat_jid, message_id],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(current) = current else {
                return Ok(false);
            };
            let next = if status == 5 {
                if current <= 1 { 5 } else { current }
            } else if current == 5 {
                status
            } else {
                current.max(status)
            };
            if next == current {
                return Ok(false);
            }
            conn.execute(
                "UPDATE messages SET status=?1 WHERE chat_jid=?2 AND id=?3",
                params![next, chat_jid, message_id],
            )?;
            Ok(true)
        })
    }

    // ------------------------------------------------------------------
    // Reactions
    // ------------------------------------------------------------------

    /// Applies one reaction the way the Go reducer did. A sender keeps one
    /// row per message, an empty emoji leaves a tombstone that a stale replay
    /// cannot overwrite, and the self aliases (PN and LID) collapse into the
    /// newest one.
    pub fn set_reaction(&self, reaction: &ReactionRow) -> rusqlite::Result<()> {
        let chat_jid = self.resolve_chat_id(&reaction.chat_jid)?;
        let timestamp = if reaction.timestamp == 0 {
            now_ms()
        } else {
            reaction.timestamp
        };
        self.write(|conn| {
            if reaction.from_me {
                let newest: Option<i64> = conn.query_row(
                    "SELECT max(timestamp) FROM reactions
                     WHERE chat_jid=?1 AND message_id=?2 AND from_me=1",
                    params![chat_jid, reaction.message_id],
                    |row| row.get(0),
                )?;
                if newest.is_some_and(|newest| timestamp < newest) {
                    return Ok(());
                }
                // PN and LID are aliases of the same current user: replace
                // every self row atomically so only one survives.
                conn.execute(
                    "DELETE FROM reactions WHERE chat_jid=?1 AND message_id=?2 AND from_me=1",
                    params![chat_jid, reaction.message_id],
                )?;
                conn.execute(
                    "INSERT INTO reactions(chat_jid,message_id,sender_jid,emoji,timestamp,from_me)
                     VALUES(?1,?2,?3,?4,?5,1)",
                    params![
                        chat_jid,
                        reaction.message_id,
                        reaction.sender_jid,
                        reaction.emoji,
                        timestamp
                    ],
                )?;
                return Ok(());
            }
            conn.execute(
                "INSERT INTO reactions(chat_jid,message_id,sender_jid,emoji,timestamp,from_me)
                 VALUES(?1,?2,?3,?4,?5,0)
                 ON CONFLICT(chat_jid,message_id,sender_jid)
                 DO UPDATE SET emoji=excluded.emoji,timestamp=excluded.timestamp,from_me=excluded.from_me
                 WHERE excluded.timestamp>=reactions.timestamp",
                params![
                    chat_jid,
                    reaction.message_id,
                    reaction.sender_jid,
                    reaction.emoji,
                    timestamp
                ],
            )?;
            Ok(())
        })
    }

    // ------------------------------------------------------------------
    // Search
    // ------------------------------------------------------------------

    /// The group conversations whose name matches, ranked the Go `SearchLocal`
    /// way: fuzzy score plus the group bonus, then recency, then jid.
    ///
    /// Every group is considered, not one page of chats: a group with an old
    /// last message is still searchable in the oracle.
    pub fn search_groups(&self, query: &str, limit: u32) -> rusqlite::Result<Vec<ChatRow>> {
        let matcher = SearchMatcher::new(query);
        let chats = self.read(|conn| {
            let mut statement = conn.prepare(&format!(
                "{CHAT_SELECT} WHERE c.preferred_jid LIKE '%@g.us'
                 ORDER BY c.last_message_at DESC, c.jid DESC"
            ))?;
            let rows = statement
                .query_map([], chat_from_row)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })?;
        let mut ranked: Vec<(i64, ChatRow)> = chats
            .into_iter()
            .filter_map(|chat| matcher.score(&chat.name).map(|score| (score + 250, chat)))
            .collect();
        ranked.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then(b.1.last_message_at.cmp(&a.1.last_message_at))
                .then(a.1.jid.cmp(&b.1.jid))
        });
        ranked.truncate(limit as usize);
        Ok(ranked.into_iter().map(|(_, chat)| chat).collect())
    }

    /// The messages whose indexed fields match, best score first.
    ///
    /// The Go filtered candidates through an FTS5 trigram index and rescored
    /// them with its fuzzy matcher; this port keeps the matcher and turns the
    /// trigrams into OR'd LIKE probes, so typos, partial words, captions,
    /// file names and locations still match without the extra virtual table.
    pub fn search_messages(
        &self,
        query: &str,
        limit: u32,
    ) -> rusqlite::Result<Vec<(ChatRow, MessageRow)>> {
        let matcher = SearchMatcher::new(query);
        let trigrams = search_trigrams(&matcher.query);
        if trigrams.is_empty() {
            return Ok(Vec::new());
        }
        let limit = if limit == 0 || limit > 20 { 20 } else { limit };
        const FIELDS: [&str; 6] = [
            "LOWER(text)",
            "LOWER(image_caption)",
            "LOWER(media_file_name)",
            "LOWER(contacts_json)",
            "LOWER(location_name)",
            "LOWER(location_address)",
        ];
        let mut probes = Vec::with_capacity(trigrams.len() * FIELDS.len());
        let mut conditions = Vec::with_capacity(trigrams.len());
        for trigram in &trigrams {
            let mut column_probes = Vec::with_capacity(FIELDS.len());
            for field in FIELDS {
                column_probes.push(format!("{field} LIKE ? ESCAPE '\\'"));
                probes.push(format!("%{}%", escape_like(trigram)));
            }
            conditions.push(format!("({})", column_probes.join(" OR ")));
        }
        let sql = format!(
            "SELECT chat_jid,id,timestamp,text,image_caption,media_file_name,
                    contacts_json,location_name,location_address
             FROM messages
             WHERE kind<>'reaction' AND revoked=0 AND ({})
             ORDER BY timestamp DESC, id DESC LIMIT 256",
            conditions.join(" OR ")
        );
        let candidates = self.read(|conn| {
            let mut statement = conn.prepare(&sql)?;
            let rows = statement
                .query_map(rusqlite::params_from_iter(probes.iter()), |row| {
                    Ok(SearchCandidate {
                        chat_jid: row.get(0)?,
                        message_id: row.get(1)?,
                        timestamp: row.get(2)?,
                        text: row.get(3)?,
                        image_caption: row.get(4)?,
                        media_file_name: row.get(5)?,
                        contacts_json: row.get(6)?,
                        location_name: row.get(7)?,
                        location_address: row.get(8)?,
                        score: 0,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })?;
        let mut hits: Vec<SearchCandidate> = candidates
            .into_iter()
            .filter_map(|mut candidate| {
                candidate.score = score_search_candidate(&matcher, &candidate)?;
                Some(candidate)
            })
            .collect();
        hits.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then(b.timestamp.cmp(&a.timestamp))
                .then(a.chat_jid.cmp(&b.chat_jid))
                .then(a.message_id.cmp(&b.message_id))
        });
        hits.truncate(limit as usize);
        let mut out = Vec::with_capacity(hits.len());
        for hit in hits {
            let chat = self.chat(&hit.chat_jid)?;
            let message = self.message(&hit.chat_jid, &hit.message_id)?;
            if let (Some(chat), Some(message)) = (chat, message) {
                out.push((chat, message));
            }
        }
        Ok(out)
    }

    /// Learns one contact's names, never erasing what it already knows.
    /// Answers whether any name actually changed.
    pub fn learn_contact(
        &self,
        jid: &str,
        contact_name: &str,
        push_name: &str,
        business_name: &str,
    ) -> rusqlite::Result<bool> {
        self.write(|conn| {
            let before: Option<(String, String, String)> = conn
                .query_row(
                    "SELECT contact_name,push_name,business_name FROM contacts WHERE jid=?1",
                    [jid],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            conn.execute(
                "INSERT INTO contacts(jid,contact_name,push_name,business_name,phone_number,updated_at)
                 VALUES(?1,?2,?3,?4,'',?5)
                 ON CONFLICT(jid) DO UPDATE SET
                   contact_name=CASE WHEN excluded.contact_name<>'' THEN excluded.contact_name ELSE contacts.contact_name END,
                   push_name=CASE WHEN excluded.push_name<>'' THEN excluded.push_name ELSE contacts.push_name END,
                   business_name=CASE WHEN excluded.business_name<>'' THEN excluded.business_name ELSE contacts.business_name END,
                   updated_at=excluded.updated_at",
                rusqlite::params![jid, contact_name, push_name, business_name, now_ms()],
            )?;
            let after: Option<(String, String, String)> = conn
                .query_row(
                    "SELECT contact_name,push_name,business_name FROM contacts WHERE jid=?1",
                    [jid],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            Ok(before != after)
        })
    }

    /// One contact's presentation row, or none.
    pub fn contact(&self, jid: &str) -> rusqlite::Result<Option<(String, String, String, String)>> {
        self.read(|conn| {
            conn.query_row(
                "SELECT contact_name,push_name,business_name,phone_number
                 FROM contacts WHERE jid=?1",
                params![jid],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
        })
    }

    /// Names a conversation (the group-subject backfill). An empty name never
    /// erases a known one, and the conversation is created when the subject
    /// arrives first, like the Go `UpsertChatName`.
    pub fn set_chat_name(&self, jid: &str, name: &str) -> rusqlite::Result<()> {
        let jid = self.resolve_chat_id(jid)?;
        if name.is_empty() {
            return Ok(());
        }
        self.write(|conn| {
            conn.execute(
                "INSERT INTO chats(jid,preferred_jid) VALUES(?1,?1)
                 ON CONFLICT(jid) DO NOTHING",
                params![jid],
            )?;
            conn.execute(
                "INSERT INTO chat_addresses(jid,chat_jid,last_seen_at) VALUES(?1,?1,?2)
                 ON CONFLICT(jid) DO NOTHING",
                params![jid, now_ms()],
            )?;
            conn.execute("UPDATE chats SET name=?1 WHERE jid=?2", params![name, jid])?;
            Ok(())
        })
    }

    /// The canonical chat of one jid, in the Go `resolveChatTx` order: a chat
    /// row with that id, then a registered address, then a redirect, then the
    /// value itself.
    pub fn resolve_chat_id(&self, jid: &str) -> rusqlite::Result<String> {
        self.read(|conn| {
            if let Some(id) = conn
                .query_row("SELECT jid FROM chats WHERE jid=?1", [jid], |row| {
                    row.get::<_, String>(0)
                })
                .optional()?
            {
                return Ok(id);
            }
            if let Some(id) = conn
                .query_row(
                    "SELECT chat_jid FROM chat_addresses WHERE jid=?1",
                    [jid],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
            {
                return Ok(id);
            }
            conn.query_row(
                "SELECT new_chat_jid FROM chat_redirects WHERE old_chat_jid=?1",
                [jid],
                |row| row.get(0),
            )
            .optional()
            .map(|redirected| redirected.unwrap_or_else(|| jid.to_owned()))
        })
    }

    /// Registers one address form under its chat, keeping the previous
    /// mapping when the two disagree.
    ///
    /// Returns the pair of the old and the new canonical chat when the
    /// registration moved an existing conversation: the caller merges them.
    pub fn adopt_address(
        &self,
        address: &str,
        chat_jid: &str,
    ) -> rusqlite::Result<Option<(String, String)>> {
        self.write(|conn| {
            let previous: Option<String> = conn
                .query_row(
                    "SELECT chat_jid FROM chat_addresses WHERE jid=?1",
                    [address],
                    |row| row.get(0),
                )
                .optional()?;
            conn.execute(
                "INSERT INTO chat_addresses(jid,chat_jid,last_seen_at) VALUES(?1,?2,?3)
                 ON CONFLICT(jid) DO UPDATE SET chat_jid=excluded.chat_jid,last_seen_at=excluded.last_seen_at",
                params![address, chat_jid, now_ms()],
            )?;
            Ok(match previous {
                Some(old) if old != chat_jid => Some((old, chat_jid.to_owned())),
                _ => None,
            })
        })
    }

    /// Folds the loser chat into the winner: messages, reactions, polls,
    /// votes, pins, sends and display flags move over, both addresses point
    /// at the winner, and the loser's jid redirects to it forever.
    ///
    /// The caller creates the winner chat first (`ensure_chat`), so there is
    /// always a row to fold into.
    ///
    /// port: the trimmed version of `MergeChats` of the Go store — the
    /// journal, legacy-reaction and history-coverage tables it also carried
    /// are not ported. Colliding rows are merged column by column instead of
    /// dropped, so a download path, a newer edit or a newer vote survives the
    /// fold.
    pub fn merge_chats(&self, winner: &str, loser: &str) -> rusqlite::Result<()> {
        if winner == loser {
            return Ok(());
        }
        self.write(|conn| {
            conn.execute(
                "INSERT INTO messages(
                   id,chat_jid,transport_jid,sender_jid,text,timestamp,from_me,status,kind,
                   reply_to_id,reply_to_chat_id,edited_at,revoked,forwarding_score,unread,
                   image_mime,image_caption,image_thumbnail,image_local_path,image_direct_path,
                   image_media_key,image_file_sha256,image_file_enc_sha256,image_width,image_height,
                   image_size,image_animated,media_mime,media_file_name,media_local_path,
                   media_direct_path,media_media_key,media_file_sha256,media_file_enc_sha256,
                   media_width,media_height,media_size,media_duration,media_voice,media_animated,
                   media_caption,contacts_json,location_lat,location_lng,location_name,
                   location_address,location_url,location_live,link_preview_url,link_preview_title,
                   link_preview_description,link_preview_thumbnail,link_preview_width,
                   link_preview_height,payload)
                 SELECT id,?1,transport_jid,sender_jid,text,timestamp,from_me,status,kind,
                   reply_to_id,reply_to_chat_id,edited_at,revoked,forwarding_score,unread,
                   image_mime,image_caption,image_thumbnail,image_local_path,image_direct_path,
                   image_media_key,image_file_sha256,image_file_enc_sha256,image_width,image_height,
                   image_size,image_animated,media_mime,media_file_name,media_local_path,
                   media_direct_path,media_media_key,media_file_sha256,media_file_enc_sha256,
                   media_width,media_height,media_size,media_duration,media_voice,media_animated,
                   media_caption,contacts_json,location_lat,location_lng,location_name,
                   location_address,location_url,location_live,link_preview_url,link_preview_title,
                   link_preview_description,link_preview_thumbnail,link_preview_width,
                   link_preview_height,payload
                 FROM messages WHERE chat_jid=?2
                 ON CONFLICT(chat_jid,id) DO UPDATE SET
                   transport_jid=CASE WHEN excluded.transport_jid LIKE '%@lid' OR messages.transport_jid=''
                                      THEN excluded.transport_jid ELSE messages.transport_jid END,
                   sender_jid=excluded.sender_jid,
                   timestamp=max(messages.timestamp,excluded.timestamp),
                   from_me=excluded.from_me,
                   status=CASE WHEN excluded.status=5 AND messages.status<=1 THEN 5
                               WHEN excluded.status=5 THEN messages.status
                               WHEN messages.status=5 THEN excluded.status
                               ELSE max(messages.status,excluded.status) END,
                   kind=excluded.kind,
                   reply_to_id=excluded.reply_to_id,
                   reply_to_chat_id=excluded.reply_to_chat_id,
                   text=CASE WHEN excluded.edited_at>=messages.edited_at THEN excluded.text ELSE messages.text END,
                   edited_at=max(messages.edited_at,excluded.edited_at),
                   revoked=(messages.revoked OR excluded.revoked),
                   forwarding_score=max(messages.forwarding_score,excluded.forwarding_score),
                   unread=(messages.unread OR excluded.unread),
                   image_mime=CASE WHEN excluded.image_mime<>'' THEN excluded.image_mime ELSE messages.image_mime END,
                   image_caption=CASE WHEN excluded.image_caption<>'' THEN excluded.image_caption ELSE messages.image_caption END,
                   image_thumbnail=CASE WHEN length(excluded.image_thumbnail)>0 THEN excluded.image_thumbnail ELSE messages.image_thumbnail END,
                   image_local_path=CASE WHEN excluded.image_local_path<>'' THEN excluded.image_local_path ELSE messages.image_local_path END,
                   image_direct_path=CASE WHEN excluded.image_direct_path<>'' THEN excluded.image_direct_path ELSE messages.image_direct_path END,
                   image_media_key=CASE WHEN length(excluded.image_media_key)>0 THEN excluded.image_media_key ELSE messages.image_media_key END,
                   image_file_sha256=CASE WHEN length(excluded.image_file_sha256)>0 THEN excluded.image_file_sha256 ELSE messages.image_file_sha256 END,
                   image_file_enc_sha256=CASE WHEN length(excluded.image_file_enc_sha256)>0 THEN excluded.image_file_enc_sha256 ELSE messages.image_file_enc_sha256 END,
                   image_width=max(messages.image_width,excluded.image_width),
                   image_height=max(messages.image_height,excluded.image_height),
                   image_size=max(messages.image_size,excluded.image_size),
                   image_animated=(messages.image_animated OR excluded.image_animated),
                   media_mime=CASE WHEN excluded.media_mime<>'' THEN excluded.media_mime ELSE messages.media_mime END,
                   media_file_name=CASE WHEN excluded.media_file_name<>'' THEN excluded.media_file_name ELSE messages.media_file_name END,
                   media_local_path=CASE WHEN excluded.media_local_path<>'' THEN excluded.media_local_path ELSE messages.media_local_path END,
                   media_direct_path=CASE WHEN excluded.media_direct_path<>'' THEN excluded.media_direct_path ELSE messages.media_direct_path END,
                   media_media_key=CASE WHEN length(excluded.media_media_key)>0 THEN excluded.media_media_key ELSE messages.media_media_key END,
                   media_file_sha256=CASE WHEN length(excluded.media_file_sha256)>0 THEN excluded.media_file_sha256 ELSE messages.media_file_sha256 END,
                   media_file_enc_sha256=CASE WHEN length(excluded.media_file_enc_sha256)>0 THEN excluded.media_file_enc_sha256 ELSE messages.media_file_enc_sha256 END,
                   media_width=max(messages.media_width,excluded.media_width),
                   media_height=max(messages.media_height,excluded.media_height),
                   media_size=max(messages.media_size,excluded.media_size),
                   media_duration=max(messages.media_duration,excluded.media_duration),
                   media_voice=(messages.media_voice OR excluded.media_voice),
                   media_animated=(messages.media_animated OR excluded.media_animated),
                   media_caption=CASE WHEN excluded.media_caption<>'' THEN excluded.media_caption ELSE messages.media_caption END,
                   contacts_json=CASE WHEN excluded.contacts_json<>'' THEN excluded.contacts_json ELSE messages.contacts_json END,
                   location_lat=CASE WHEN excluded.kind='location' THEN excluded.location_lat ELSE messages.location_lat END,
                   location_lng=CASE WHEN excluded.kind='location' THEN excluded.location_lng ELSE messages.location_lng END,
                   location_name=CASE WHEN excluded.location_name<>'' THEN excluded.location_name ELSE messages.location_name END,
                   location_address=CASE WHEN excluded.location_address<>'' THEN excluded.location_address ELSE messages.location_address END,
                   location_url=CASE WHEN excluded.location_url<>'' THEN excluded.location_url ELSE messages.location_url END,
                   location_live=(messages.location_live OR excluded.location_live),
                   link_preview_url=CASE WHEN excluded.link_preview_url<>'' THEN excluded.link_preview_url ELSE messages.link_preview_url END,
                   link_preview_title=CASE WHEN excluded.link_preview_title<>'' THEN excluded.link_preview_title ELSE messages.link_preview_title END,
                   link_preview_description=CASE WHEN excluded.link_preview_description<>'' THEN excluded.link_preview_description ELSE messages.link_preview_description END,
                   link_preview_thumbnail=CASE WHEN length(excluded.link_preview_thumbnail)>0 THEN excluded.link_preview_thumbnail ELSE messages.link_preview_thumbnail END,
                   link_preview_width=max(messages.link_preview_width,excluded.link_preview_width),
                   link_preview_height=max(messages.link_preview_height,excluded.link_preview_height),
                   payload=CASE WHEN length(excluded.payload)>0 THEN excluded.payload ELSE messages.payload END",
                params![winner, loser],
            )?;
            conn.execute(
                "INSERT INTO polls(chat_jid,message_id,question,selectable_count,total_voters,options_json,snapshot_at)
                 SELECT ?1,message_id,question,selectable_count,total_voters,options_json,snapshot_at
                 FROM polls WHERE chat_jid=?2
                 ON CONFLICT(chat_jid,message_id) DO UPDATE SET
                   question=excluded.question,selectable_count=excluded.selectable_count,
                   total_voters=excluded.total_voters,options_json=excluded.options_json,
                   snapshot_at=excluded.snapshot_at
                 WHERE excluded.snapshot_at>=polls.snapshot_at",
                params![winner, loser],
            )?;
            conn.execute(
                "INSERT INTO poll_votes(chat_jid,poll_message_id,voter_jid,selected_json,timestamp,from_me)
                 SELECT ?1,v.poll_message_id,v.voter_jid,v.selected_json,v.timestamp,v.from_me
                 FROM poll_votes v
                 WHERE v.chat_jid=?2
                   AND v.timestamp>COALESCE((SELECT p.snapshot_at FROM polls p
                                             WHERE p.chat_jid=?1 AND p.message_id=v.poll_message_id),0)
                 ON CONFLICT(chat_jid,poll_message_id,voter_jid) DO UPDATE SET
                   selected_json=excluded.selected_json,timestamp=excluded.timestamp,from_me=excluded.from_me
                 WHERE excluded.timestamp>=poll_votes.timestamp",
                params![winner, loser],
            )?;
            conn.execute(
                "INSERT INTO pinned_messages(chat_jid,message_id,pinned_at,pinned_by,pinned)
                 SELECT ?1,message_id,pinned_at,pinned_by,pinned
                 FROM pinned_messages WHERE chat_jid=?2
                 ON CONFLICT(chat_jid,message_id) DO UPDATE SET
                   pinned_at=excluded.pinned_at,pinned_by=excluded.pinned_by,pinned=excluded.pinned
                 WHERE excluded.pinned_at>=pinned_messages.pinned_at",
                params![winner, loser],
            )?;
            conn.execute("DELETE FROM messages WHERE chat_jid=?1", params![loser])?;
            conn.execute(
                "UPDATE messages SET reply_to_chat_id=?1 WHERE reply_to_chat_id=?2",
                params![winner, loser],
            )?;
            conn.execute(
                "INSERT INTO reactions(chat_jid,message_id,sender_jid,emoji,timestamp,from_me)
                 SELECT ?1,message_id,sender_jid,emoji,timestamp,from_me
                 FROM reactions WHERE chat_jid=?2
                 ON CONFLICT(chat_jid,message_id,sender_jid) DO UPDATE SET
                   emoji=excluded.emoji,timestamp=excluded.timestamp,from_me=excluded.from_me
                 WHERE excluded.timestamp>=reactions.timestamp",
                params![winner, loser],
            )?;
            conn.execute("DELETE FROM reactions WHERE chat_jid=?1", params![loser])?;
            // PN and LID aliases of this device collapse to the newest self
            // reaction after the fold, like the Go merge.
            conn.execute(
                "DELETE FROM reactions WHERE chat_jid=?1 AND from_me=1 AND EXISTS (
                   SELECT 1 FROM reactions newer
                   WHERE newer.chat_jid=reactions.chat_jid AND newer.message_id=reactions.message_id
                     AND newer.from_me=1
                     AND (newer.timestamp>reactions.timestamp
                       OR (newer.timestamp=reactions.timestamp AND newer.rowid>reactions.rowid))
                 )",
                params![winner],
            )?;
            for table in ["outgoing_requests", "outgoing_reactions"] {
                conn.execute(
                    &format!("UPDATE {table} SET chat_jid=?1 WHERE chat_jid=?2"),
                    params![winner, loser],
                )?;
            }
            conn.execute(
                "UPDATE chat_addresses SET chat_jid=?1 WHERE chat_jid=?2",
                params![winner, loser],
            )?;
            conn.execute(
                "INSERT OR REPLACE INTO chat_redirects(old_chat_jid,new_chat_jid) VALUES(?1,?2)",
                params![loser, winner],
            )?;
            conn.execute(
                "UPDATE chat_redirects SET new_chat_jid=?1 WHERE new_chat_jid=?2",
                params![winner, loser],
            )?;
            // Display flags: the loser's title only fills an unnamed winner,
            // mute and block take the maximum, and the archive state follows
            // the newer conversation (the Go merge rule).
            conn.execute(
                "UPDATE chats SET
                   name=CASE WHEN name='' THEN (SELECT name FROM chats WHERE jid=?2) ELSE name END,
                   muted_until=max(muted_until,(SELECT muted_until FROM chats WHERE jid=?2)),
                   archived=CASE WHEN (SELECT last_message_at FROM chats WHERE jid=?2)>last_message_at
                                 THEN (SELECT archived FROM chats WHERE jid=?2) ELSE archived END,
                   blocked=max(blocked,(SELECT blocked FROM chats WHERE jid=?2))
                 WHERE jid=?1",
                params![winner, loser],
            )?;
            conn.execute("DELETE FROM chats WHERE jid=?1", params![loser])?;
            Ok(())
        })?;
        self.refresh_chat_pointer(winner)
    }

    /// Recomputes the derived state of one chat from its rows, the way the Go
    /// `RepairDerivedState` did: the pointer, its preview, the unread counter
    /// and the last sender. Reaction pseudo-messages never count.
    pub fn refresh_chat_pointer(&self, jid: &str) -> rusqlite::Result<()> {
        self.write(|conn| {
            conn.execute(
                "UPDATE chats SET
                   last_message_id=COALESCE((SELECT id FROM messages WHERE chat_jid=?1 AND kind<>'reaction' ORDER BY timestamp DESC,id DESC LIMIT 1),''),
                   last_message_text=COALESCE((SELECT CASE
                        WHEN text<>'' THEN text
                        WHEN image_caption<>'' THEN image_caption
                        WHEN media_caption<>'' THEN media_caption
                        ELSE '' END
                      FROM messages WHERE chat_jid=?1 AND kind<>'reaction' ORDER BY timestamp DESC,id DESC LIMIT 1),''),
                   last_message_at=COALESCE((SELECT timestamp FROM messages WHERE chat_jid=?1 AND kind<>'reaction' ORDER BY timestamp DESC,id DESC LIMIT 1),0),
                   unread_count=(SELECT count(*) FROM messages WHERE chat_jid=?1 AND kind<>'reaction' AND unread=1)
                 WHERE jid=?1",
                params![jid],
            )?;
            Ok(())
        })
    }

    /// Every chat jid that carries one of the given addresses, so a
    /// reconciliation pass can find the twins the ingest created.
    pub fn chats_of_addresses(&self, addresses: &[String]) -> rusqlite::Result<Vec<String>> {
        if addresses.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = addresses.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        self.read(|conn| {
            let mut statement = conn.prepare(&format!(
                "SELECT DISTINCT chat_jid FROM chat_addresses WHERE jid IN ({placeholders})"
            ))?;
            let rows = statement
                .query_map(rusqlite::params_from_iter(addresses.iter()), |row| {
                    row.get::<_, String>(0)
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// Every chat row that lives under a hidden-identity jid: the inputs of
    /// the reconciliation pass.
    pub fn lid_chats(&self) -> rusqlite::Result<Vec<String>> {
        self.read(|conn| {
            let mut statement = conn.prepare("SELECT jid FROM chats WHERE jid LIKE '%@lid'")?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// Records the poll content of a message the desktop can vote on. A
    /// re-delivered poll definition refreshes the row unless an aggregate
    /// snapshot already owns it, like the Go reducer.
    pub fn upsert_poll(
        &self,
        chat_jid: &str,
        message_id: &str,
        question: &str,
        options: &[String],
        selectable_count: u32,
    ) -> rusqlite::Result<()> {
        let chat_jid = self.resolve_chat_id(chat_jid)?;
        let mut options_json = String::from("[");
        for (index, option) in options.iter().enumerate() {
            if index > 0 {
                options_json.push(',');
            }
            options_json.push('"');
            for character in option.chars() {
                match character {
                    '"' => options_json.push_str("\\\""),
                    '\\' => options_json.push_str("\\\\"),
                    other => options_json.push(other),
                }
            }
            options_json.push('"');
        }
        options_json.push(']');
        self.write(|conn| {
            conn.execute(
                "INSERT INTO polls(chat_jid,message_id,question,selectable_count,options_json)
                 VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(chat_jid,message_id) DO UPDATE SET
                   question=excluded.question,selectable_count=excluded.selectable_count,
                   options_json=excluded.options_json
                 WHERE polls.snapshot_at=0",
                rusqlite::params![
                    chat_jid,
                    message_id,
                    question,
                    selectable_count,
                    options_json
                ],
            )?;
            Ok(())
        })
    }

    /// The most recent live sticker row matching a sticker id (the plaintext
    /// file hash, hex): the (chat, message) pair that can resend it.
    pub fn message_by_sticker_id(
        &self,
        sticker_id: &str,
    ) -> rusqlite::Result<Option<(String, String)>> {
        self.read(|conn| {
            conn.query_row(
                "SELECT chat_jid,id FROM messages
                 WHERE kind='sticker' AND revoked=0 AND LOWER(HEX(image_file_sha256))=LOWER(?1)
                 ORDER BY timestamp DESC, id DESC LIMIT 1",
                params![sticker_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
        })
    }

    /// The subscribed newsletters, newest subscription first, like the Go
    /// `Newsletters`.
    pub fn list_newsletters(&self) -> rusqlite::Result<Vec<whatfast_protocol::Newsletter>> {
        self.read(|conn| {
            let mut statement = conn.prepare(
                "SELECT jid,name,subscribed_at,muted FROM newsletters ORDER BY subscribed_at DESC",
            )?;
            let rows = statement
                .query_map([], |row| {
                    Ok(whatfast_protocol::Newsletter {
                        jid: row.get(0)?,
                        name: row.get(1)?,
                        subscribed_at_ms: row.get::<_, i64>(2)?,
                        muted: row.get::<_, i64>(3)? != 0,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// Records one newsletter subscription (or a rename). A re-subscribe
    /// refreshes the subscription instant, exactly like `UpsertNewsletter`.
    pub fn upsert_newsletter(&self, jid: &str, name: &str, muted: bool) -> rusqlite::Result<()> {
        self.write(|conn| {
            conn.execute(
                "INSERT INTO newsletters(jid,name,subscribed_at,muted) VALUES(?1,?2,?3,?4)
                 ON CONFLICT(jid) DO UPDATE SET name=excluded.name,subscribed_at=excluded.subscribed_at",
                rusqlite::params![jid, name, now_ms(), muted as i64],
            )?;
            Ok(())
        })
    }

    /// Removes one newsletter subscription.
    pub fn remove_newsletter(&self, jid: &str) -> rusqlite::Result<()> {
        self.write(|conn| {
            conn.execute("DELETE FROM newsletters WHERE jid=?1", params![jid])?;
            Ok(())
        })
    }

    /// The jids of group chats whose name is still empty, oldest activity
    /// first, capped: the backfill walks them slowly.
    pub fn unnamed_groups(&self, limit: u32) -> rusqlite::Result<Vec<String>> {
        self.read(|conn| {
            let mut statement = conn.prepare(
                "SELECT jid FROM chats
                 WHERE preferred_jid LIKE '%@g.us' AND name=''
                 ORDER BY last_message_at DESC LIMIT ?1",
            )?;
            let rows = statement
                .query_map(params![limit], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    // ------------------------------------------------------------------
    // Key-value state
    // ------------------------------------------------------------------

    pub fn meta_get(&self, key: &str) -> rusqlite::Result<Option<String>> {
        self.read(|conn| {
            conn.query_row(
                "SELECT value FROM sync_state WHERE key=?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
        })
    }

    pub fn meta_set(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        self.write(|conn| {
            conn.execute(
                "INSERT INTO sync_state(key,value) VALUES(?1,?2)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![key, value],
            )?;
            Ok(())
        })
    }

    // ------------------------------------------------------------------
    // Server wave: pages, statuses, pins, stickers, reactions, forwards
    // ------------------------------------------------------------------

    /// One bounded page, oldest-to-newest, ending strictly before the
    /// (timestamp,id) cursor, plus whether an older page exists.
    ///
    /// The Go `Messages` asked for `limit+1` rows and set `NextCursor` only
    /// when the extra row arrived; `has_more` is that same fact, so an
    /// exactly-full page is not announced as a page that does not exist.
    pub fn messages_before_page(
        &self,
        chat_jid: &str,
        before_ts: i64,
        before_id: &str,
        limit: u32,
    ) -> rusqlite::Result<(Vec<MessageRow>, bool)> {
        let limit = clamp_limit(limit);
        self.read(|conn| {
            let mut statement = conn.prepare(&format!(
                "{MESSAGE_SELECT}
                 WHERE m.chat_jid=?1
                   AND m.kind<>'reaction'
                   AND (?2 = 0 OR m.timestamp < ?2 OR (m.timestamp = ?2 AND m.id < ?3))
                 ORDER BY m.timestamp DESC, m.id DESC LIMIT ?4"
            ))?;
            let mut rows = statement
                .query_map(
                    params![chat_jid, before_ts, before_id, limit + 1],
                    message_from_row,
                )?
                .collect::<Result<Vec<_>, _>>()?;
            attach_reactions(conn, &mut rows)?;
            let has_more = rows.len() as u32 > limit;
            if has_more {
                rows.truncate(limit as usize);
            }
            rows.reverse();
            Ok((rows, has_more))
        })
    }

    /// The `status@broadcast` rows newer than `since`, newest first, capped at
    /// 500 and without reaction pseudo-rows, exactly like the Go
    /// `StatusMessagesSince`. WhatsApp expires a status after 24 hours; the
    /// caller passes the cutoff.
    pub fn status_messages_since(&self, since: i64) -> rusqlite::Result<Vec<MessageRow>> {
        self.read(|conn| {
            let mut statement = conn.prepare(&format!(
                "{MESSAGE_SELECT}
                 WHERE m.transport_jid='status@broadcast'
                   AND m.timestamp>=?1
                   AND m.kind<>'reaction'
                 ORDER BY m.timestamp DESC, m.id DESC LIMIT 500"
            ))?;
            let rows = statement
                .query_map(params![since], message_from_row)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// The poll snapshots of one page, built the way the Go `Poll` read them:
    /// option names from `options_json` plus one tally per stored vote. Pages
    /// without polls answer an empty list.
    pub fn poll_snapshots(
        &self,
        chat_jid: &str,
        message_ids: &[String],
    ) -> rusqlite::Result<Vec<PollSnapshot>> {
        if message_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.read(|conn| {
            let mut definition = conn.prepare(
                "SELECT question,selectable_count,total_voters,options_json
                 FROM polls WHERE chat_jid=?1 AND message_id=?2",
            )?;
            let mut votes = conn.prepare(
                "SELECT voter_jid,selected_json,from_me FROM poll_votes
                 WHERE chat_jid=?1 AND poll_message_id=?2",
            )?;
            let mut snapshots = Vec::new();
            for message_id in message_ids {
                let row = definition
                    .query_row(params![chat_jid, message_id], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, String>(3)?,
                        ))
                    })
                    .optional()?;
                let Some((question, selectable, total_voters, options_json)) = row else {
                    continue;
                };
                let mut snapshot = PollSnapshot {
                    message_id: message_id.clone(),
                    question,
                    selectable_count: selectable.max(0) as u32,
                    total_voters: total_voters.max(0) as u32,
                    options: decode_poll_option_names(&options_json)
                        .into_iter()
                        .map(|name| PollOptionRow {
                            name,
                            votes: 0,
                            selected_by_me: false,
                            voters: Vec::new(),
                        })
                        .collect(),
                };
                let stored = votes
                    .query_map(params![chat_jid, message_id], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)? != 0,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                for (voter_jid, selected_json, from_me) in stored {
                    let mut selected = decode_json_strings(&selected_json);
                    if !selected.is_empty() {
                        snapshot.total_voters += 1;
                    }
                    for name in selected.drain(..) {
                        if let Some(option) = snapshot
                            .options
                            .iter_mut()
                            .find(|option| option.name == name)
                        {
                            option.votes += 1;
                            option.voters.push(PollVoterRow {
                                jid: voter_jid.clone(),
                                from_me,
                            });
                            if from_me {
                                option.selected_by_me = true;
                            }
                        }
                    }
                }
                snapshots.push(snapshot);
            }
            Ok(snapshots)
        })
    }

    /// Records one pin or unpin, the Go `SetMessagePinned` upsert: a stale
    /// write never replaces a newer one.
    pub fn set_message_pinned(
        &self,
        chat_jid: &str,
        message_id: &str,
        pinned_by: &str,
        at: i64,
        pinned: bool,
    ) -> rusqlite::Result<()> {
        let chat_jid = self.resolve_chat_id(chat_jid)?;
        let at = if at == 0 { now_ms() } else { at };
        self.write(|conn| {
            conn.execute(
                "INSERT INTO pinned_messages(chat_jid,message_id,pinned_at,pinned_by,pinned)
                 VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(chat_jid,message_id) DO UPDATE SET
                   pinned_at=excluded.pinned_at,pinned_by=excluded.pinned_by,pinned=excluded.pinned
                 WHERE excluded.pinned_at>=pinned_messages.pinned_at",
                params![chat_jid, message_id, at, pinned_by, pinned as i64],
            )?;
            Ok(())
        })
    }

    /// The live pins of one chat, newest first, the order the Go
    /// `PinnedMessages` answered.
    pub fn pinned_messages(&self, chat_jid: &str) -> rusqlite::Result<Vec<PinnedRow>> {
        let chat_jid = self.resolve_chat_id(chat_jid)?;
        self.read(|conn| {
            let mut statement = conn.prepare(
                "SELECT message_id,pinned_at,pinned_by FROM pinned_messages
                 WHERE chat_jid=?1 AND pinned=1 ORDER BY pinned_at DESC, message_id",
            )?;
            let rows = statement
                .query_map(params![chat_jid], |row| {
                    Ok(PinnedRow {
                        message_id: row.get(0)?,
                        pinned_at: row.get(1)?,
                        pinned_by: row.get(2)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// The sticker messages that can supply bytes, newest first, bounded by
    /// `limit` (2000 by default, 5000 at most), the Go
    /// `RecentStickerMessages` scan. Only rows with a known plaintext hash
    /// are candidates.
    pub fn recent_sticker_messages(
        &self,
        limit: u32,
    ) -> rusqlite::Result<Vec<StickerCandidateRow>> {
        let limit = if limit == 0 || limit > 5000 {
            2000
        } else {
            limit
        };
        self.read(|conn| {
            let mut statement = conn.prepare(
                "SELECT chat_jid,id,from_me,timestamp,image_mime,image_file_sha256,
                        image_width,image_height,image_animated
                 FROM messages
                 WHERE kind='sticker' AND revoked=0 AND image_file_sha256<>X''
                 ORDER BY timestamp DESC,id DESC LIMIT ?1",
            )?;
            let rows = statement
                .query_map(params![limit], |row| {
                    let digest: Vec<u8> = row.get(5)?;
                    Ok(StickerCandidateRow {
                        id: hex_lower(&digest),
                        chat_jid: row.get(0)?,
                        message_id: row.get(1)?,
                        from_me: row.get::<_, i64>(2)? != 0,
                        timestamp: row.get(3)?,
                        mime: row.get(4)?,
                        width: row.get(6)?,
                        height: row.get(7)?,
                        animated: row.get::<_, i64>(8)? != 0,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// The stickers WhatsApp synced as favourited on a linked device, newest
    /// first, bounded like the Go `FavoriteStickers` (300 by default).
    pub fn favorite_stickers(&self, limit: u32) -> rusqlite::Result<Vec<FavoriteStickerRow>> {
        let limit = if limit == 0 || limit > 1000 {
            300
        } else {
            limit
        };
        self.read(|conn| {
            let mut statement = conn.prepare(
                "SELECT id,mime_type,direct_path,media_key,file_enc_sha256,width,height,
                        animated,file_size,local_path,updated_at
                 FROM sticker_favorites ORDER BY updated_at DESC,id DESC LIMIT ?1",
            )?;
            let rows = statement
                .query_map(params![limit], favorite_sticker_from_row)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// One favourite sticker by its id (hex of the encrypted file hash).
    pub fn favorite_sticker(&self, id: &str) -> rusqlite::Result<Option<FavoriteStickerRow>> {
        self.read(|conn| {
            conn.query_row(
                "SELECT id,mime_type,direct_path,media_key,file_enc_sha256,width,height,
                        animated,file_size,local_path,updated_at
                 FROM sticker_favorites WHERE id=?1",
                params![id],
                favorite_sticker_from_row,
            )
            .optional()
        })
    }

    /// Records or refreshes one favourite synced from another device. The
    /// local cache path survives the refresh: the bytes did not change.
    pub fn upsert_favorite_sticker(&self, favorite: &FavoriteStickerRow) -> rusqlite::Result<()> {
        self.write(|conn| {
            conn.execute(
                "INSERT INTO sticker_favorites(id,mime_type,direct_path,media_key,file_enc_sha256,
                   width,height,animated,file_size,local_path,updated_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
                 ON CONFLICT(id) DO UPDATE SET
                   mime_type=excluded.mime_type,direct_path=excluded.direct_path,
                   media_key=excluded.media_key,file_enc_sha256=excluded.file_enc_sha256,
                   width=excluded.width,height=excluded.height,animated=excluded.animated,
                   file_size=excluded.file_size,updated_at=excluded.updated_at",
                params![
                    favorite.id,
                    favorite.mime,
                    favorite.direct_path,
                    favorite.media_key,
                    favorite.file_enc_sha256,
                    favorite.width,
                    favorite.height,
                    favorite.animated as i64,
                    favorite.file_size,
                    favorite.local_path,
                    favorite.updated_at,
                ],
            )?;
            Ok(())
        })
    }

    /// Un-favourites one sticker; true when a row actually went away, so the
    /// caller only announces a change that happened.
    pub fn remove_favorite_sticker(&self, id: &str) -> rusqlite::Result<bool> {
        self.write(|conn| {
            let changed = conn.execute("DELETE FROM sticker_favorites WHERE id=?1", params![id])?;
            Ok(changed > 0)
        })
    }

    /// Points one favourite at its downloaded cache file.
    pub fn set_favorite_sticker_local_path(
        &self,
        id: &str,
        local_path: &str,
    ) -> rusqlite::Result<()> {
        self.write(|conn| {
            conn.execute(
                "UPDATE sticker_favorites SET local_path=?1 WHERE id=?2",
                params![local_path, id],
            )?;
            Ok(())
        })
    }

    /// The unread rows a read receipt still owes through one message, plus
    /// which refusal the Go `UnreadThrough` answered: the message is missing,
    /// it is outgoing, or the rows themselves.
    pub fn unread_through(
        &self,
        chat_jid: &str,
        message_id: &str,
    ) -> rusqlite::Result<UnreadThrough> {
        let chat_jid = self.resolve_chat_id(chat_jid)?;
        self.read(|conn| {
            let anchor = conn
                .query_row(
                    "SELECT timestamp,from_me FROM messages WHERE chat_jid=?1 AND id=?2",
                    params![chat_jid, message_id],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)? != 0)),
                )
                .optional()?;
            let Some((timestamp, from_me)) = anchor else {
                return Ok(UnreadThrough::Missing);
            };
            if from_me {
                return Ok(UnreadThrough::Outgoing);
            }
            let mut statement = conn.prepare(
                "SELECT id,transport_jid,sender_jid,timestamp FROM messages
                 WHERE chat_jid=?1 AND unread=1
                   AND (timestamp<?2 OR (timestamp=?2 AND id<=?3))
                 ORDER BY timestamp,id",
            )?;
            let rows = statement
                .query_map(params![chat_jid, timestamp, message_id], |row| {
                    Ok(UnreadRow {
                        id: row.get(0)?,
                        transport_jid: row.get(1)?,
                        sender_jid: row.get(2)?,
                        timestamp: row.get(3)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(UnreadThrough::Rows(rows))
        })
    }

    /// Marks the given rows read and refreshes the cached counter from the
    /// rows themselves, the Go `MarkReadIDs`. The counter recomputes even
    /// when no row was dirty: a counter inflated without flagged rows heals
    /// the next time the chat opens instead of sticking forever.
    pub fn mark_read_ids(&self, chat_jid: &str, ids: &[String]) -> rusqlite::Result<()> {
        let chat_jid = self.resolve_chat_id(chat_jid)?;
        self.write(|conn| {
            for id in ids {
                conn.execute(
                    "UPDATE messages SET unread=0 WHERE chat_jid=?1 AND id=?2",
                    params![chat_jid, id],
                )?;
            }
            conn.execute(
                "UPDATE chats SET unread_count=(SELECT COUNT(*) FROM messages
                   WHERE chat_jid=?1 AND kind<>'reaction' AND unread=1) WHERE jid=?1",
                params![chat_jid],
            )?;
            Ok(())
        })
    }

    /// Reserves one outgoing reaction by its client id, the Go
    /// `ReserveOutgoingReaction` contract: a completed reservation answers the
    /// instant it was sent, so a retry never reaches the network again. The
    /// same id with a different payload is a refusal.
    pub fn reserve_outgoing_reaction(
        &self,
        client_reaction_id: &str,
        chat_jid: &str,
        message_id: &str,
        emoji: &str,
    ) -> rusqlite::Result<ReactionReservation> {
        let chat_jid = self.resolve_chat_id(chat_jid)?;
        self.read(|conn| {
            let existing = conn
                .query_row(
                    "SELECT chat_jid,message_id,emoji,completed_at FROM outgoing_reactions
                     WHERE client_reaction_id=?1",
                    params![client_reaction_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, i64>(3)?,
                        ))
                    },
                )
                .optional()?;
            match existing {
                Some((chat, message, previous, _))
                    if chat != chat_jid || message != message_id || previous != emoji =>
                {
                    Err(invalid_parameter(
                        "client_reaction_id already used with different payload",
                    ))
                }
                Some((_, _, _, completed_at)) if completed_at > 0 => {
                    Ok(ReactionReservation::Completed(completed_at))
                }
                _ => Ok(ReactionReservation::Send),
            }
        })
        .and_then(|reservation| {
            if matches!(reservation, ReactionReservation::Send) {
                self.write(|conn| {
                    conn.execute(
                        "INSERT INTO outgoing_reactions(client_reaction_id,chat_jid,message_id,emoji,created_at)
                         VALUES(?1,?2,?3,?4,?5) ON CONFLICT(client_reaction_id) DO NOTHING",
                        params![client_reaction_id, chat_jid, message_id, emoji, now_ms()],
                    )?;
                    Ok(())
                })?;
            }
            Ok(reservation)
        })
    }

    /// Stamps one reaction reservation as completed at `timestamp`.
    pub fn complete_outgoing_reaction(
        &self,
        client_reaction_id: &str,
        chat_jid: &str,
        message_id: &str,
        emoji: &str,
        timestamp: i64,
    ) -> rusqlite::Result<()> {
        let chat_jid = self.resolve_chat_id(chat_jid)?;
        self.write(|conn| {
            conn.execute(
                "UPDATE outgoing_reactions SET completed_at=?1
                 WHERE client_reaction_id=?2 AND chat_jid=?3 AND message_id=?4 AND emoji=?5",
                params![timestamp, client_reaction_id, chat_jid, message_id, emoji],
            )?;
            Ok(())
        })
    }

    /// The stored answer of one outgoing request id: the chat its fingerprint
    /// names and the message it produced, so a retry replays the row instead
    /// of sending again.
    pub fn outgoing_request(
        &self,
        client_request_id: &str,
    ) -> rusqlite::Result<Option<(String, String, String)>> {
        self.read(|conn| {
            conn.query_row(
                "SELECT chat_jid,text,message_id FROM outgoing_requests
                 WHERE client_request_id=?1",
                params![client_request_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
        })
    }

    /// Remembers the message one outgoing request produced. A replayed id
    /// keeps its first answer.
    pub fn remember_outgoing_request(
        &self,
        client_request_id: &str,
        chat_jid: &str,
        fingerprint: &str,
        message_id: &str,
    ) -> rusqlite::Result<()> {
        self.write(|conn| {
            conn.execute(
                "INSERT INTO outgoing_requests(client_request_id,chat_jid,text,message_id,created_at)
                 VALUES(?1,?2,?3,?4,?5) ON CONFLICT(client_request_id) DO NOTHING",
                params![client_request_id, chat_jid, fingerprint, message_id, now_ms()],
            )?;
            Ok(())
        })
    }

    // ------------------------------------------------------------------
    // History coverage and integrity
    // ------------------------------------------------------------------

    /// The coverage of one chat's history, the Go `HistoryCoverage`: the live
    /// counts off `messages` plus the request state off `history_coverage`.
    /// A chat with no local message has no anchor; a short batch that the
    /// phone once called the end stops being one when more arrived.
    pub fn history_coverage(&self, chat_id: &str) -> rusqlite::Result<HistoryCoverageData> {
        let chat_id = self.resolve_chat_id(chat_id)?;
        self.read(|conn| {
            let (local_message_count, oldest): (i64, Option<i64>) = conn.query_row(
                "SELECT count(*),min(timestamp) FROM messages
                 WHERE chat_jid=?1 AND kind<>'reaction'",
                params![chat_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            let mut data = HistoryCoverageData {
                chat_id: chat_id.clone(),
                local_message_count: local_message_count.max(0) as u64,
                oldest_message_timestamp: oldest.unwrap_or(0),
                ..Default::default()
            };
            let stored: Option<HistoryCoverageData> = conn
                .query_row(
                    "SELECT state,revision,on_demand_added,last_requested_count,
                            last_received_count,last_added_count,last_requested_at,
                            retry_after,detail
                     FROM history_coverage WHERE chat_jid=?1",
                    params![chat_id],
                    |row| {
                        Ok(HistoryCoverageData {
                            chat_id: chat_id.clone(),
                            state: row.get(0)?,
                            revision: row.get::<_, i64>(1)?.max(0) as u64,
                            local_message_count: data.local_message_count,
                            oldest_message_timestamp: data.oldest_message_timestamp,
                            on_demand_added: row.get::<_, i64>(2)?.max(0) as u64,
                            last_requested_count: row.get::<_, i64>(3)?.max(0) as u32,
                            last_received_count: row.get::<_, i64>(4)?.max(0) as u32,
                            last_added_count: row.get::<_, i64>(5)?.max(0) as u32,
                            last_requested_at: row.get(6)?,
                            retry_after: row.get(7)?,
                            detail: row.get(8)?,
                        })
                    },
                )
                .optional()?;
            if let Some(stored) = stored {
                data.state = stored.state;
                data.revision = stored.revision;
                data.on_demand_added = stored.on_demand_added;
                data.last_requested_count = stored.last_requested_count;
                data.last_received_count = stored.last_received_count;
                data.last_added_count = stored.last_added_count;
                data.last_requested_at = stored.last_requested_at;
                data.retry_after = stored.retry_after;
                data.detail = stored.detail;
            } else {
                data.state = "unknown".into();
                data.detail = "Older WhatsApp history has not been checked".into();
            }
            if data.local_message_count == 0 {
                data.state = "no_anchor".into();
                data.detail =
                    "No local message is available to anchor an older-history request".into();
            } else if data.state == "exhausted" && data.last_received_count > 0 {
                data.state = "more_available".into();
                data.detail = "More older messages can be requested".into();
            }
            Ok(data)
        })
    }

    /// Saves one coverage row, bumping its revision like the Go
    /// `SaveHistoryCoverage` did, and answers the stored shape back.
    pub fn save_history_coverage(
        &self,
        mut data: HistoryCoverageData,
    ) -> rusqlite::Result<HistoryCoverageData> {
        let chat_id = self.resolve_chat_id(&data.chat_id)?;
        data.chat_id = chat_id.clone();
        data.revision += 1;
        self.write(|conn| {
            conn.execute(
                "INSERT INTO history_coverage(chat_jid,state,revision,on_demand_added,
                        last_requested_count,last_received_count,last_added_count,
                        last_requested_at,retry_after,detail)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
                 ON CONFLICT(chat_jid) DO UPDATE SET
                   state=excluded.state,revision=excluded.revision,
                   on_demand_added=excluded.on_demand_added,
                   last_requested_count=excluded.last_requested_count,
                   last_received_count=excluded.last_received_count,
                   last_added_count=excluded.last_added_count,
                   last_requested_at=excluded.last_requested_at,
                   retry_after=excluded.retry_after,detail=excluded.detail",
                params![
                    data.chat_id,
                    data.state,
                    data.revision as i64,
                    data.on_demand_added as i64,
                    data.last_requested_count as i64,
                    data.last_received_count as i64,
                    data.last_added_count as i64,
                    data.last_requested_at,
                    data.retry_after,
                    data.detail,
                ],
            )?;
            Ok(())
        })?;
        self.history_coverage(&chat_id)
    }

    /// The five history counters, the Go `HistoryOverview` query verbatim.
    pub fn history_overview(&self) -> rusqlite::Result<HistoryOverviewData> {
        self.read(|conn| {
            conn.query_row(
                "SELECT
                   (SELECT count(*) FROM messages WHERE kind<>'reaction'),
                   (SELECT count(DISTINCT chat_jid) FROM messages WHERE kind<>'reaction'),
                   (SELECT count(*) FROM history_coverage WHERE state='exhausted'),
                   (SELECT count(*) FROM chats WHERE jid NOT IN (SELECT chat_jid FROM history_coverage)),
                   (SELECT count(*) FROM chats WHERE jid NOT IN (SELECT DISTINCT chat_jid FROM messages WHERE kind<>'reaction'))",
                [],
                |row| {
                    Ok(HistoryOverviewData {
                        local_message_count: row.get::<_, i64>(0)?.max(0) as u64,
                        chats_with_messages: row.get::<_, i64>(1)?.max(0) as u64,
                        chats_checked: row.get::<_, i64>(2)?.max(0) as u64,
                        chats_unknown: row.get::<_, i64>(3)?.max(0) as u64,
                        chats_without_anchor: row.get::<_, i64>(4)?.max(0) as u64,
                    })
                },
            )
        })
    }

    /// The oldest stored message of one chat, the anchor an older-history
    /// request resumes from.
    pub fn oldest_message_row(&self, chat_id: &str) -> rusqlite::Result<Option<MessageRow>> {
        let chat_id = self.resolve_chat_id(chat_id)?;
        let id: Option<String> = self.read(|conn| {
            conn.query_row(
                "SELECT id FROM messages WHERE chat_jid=?1 AND kind<>'reaction'
                 ORDER BY timestamp,id LIMIT 1",
                params![chat_id],
                |row| row.get(0),
            )
            .optional()
        })?;
        match id {
            Some(id) => self.message(&chat_id, &id),
            None => Ok(None),
        }
    }

    /// The local health, the Go `IntegrityStatus`: the SQLite check, the
    /// inbound journal and the derived chat summaries.
    pub fn integrity_status(&self) -> rusqlite::Result<IntegrityStatusData> {
        self.read(|conn| {
            let quick: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
            let mut status = IntegrityStatusData {
                database_ok: quick == "ok",
                detail: quick,
                ..Default::default()
            };
            let (pending, failed, oldest): (i64, i64, Option<i64>) = conn.query_row(
                "SELECT count(*),coalesce(sum(failed),0),min(created_at) FROM (
                   SELECT attempts>0 AS failed,created_at FROM inbound_messages
                   UNION ALL
                   SELECT attempts>0,created_at FROM inbound_events)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
            status.pending_inbound = pending.max(0) as u64;
            status.failed_inbound = failed.max(0) as u64;
            status.oldest_pending_at = oldest.unwrap_or(0);
            let stored: Option<(i64, i64)> = conn
                .query_row(
                    "SELECT last_message_persisted_at,last_reconciliation_at
                     FROM storage_health WHERE singleton=1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            if let Some((persisted, reconciled)) = stored {
                status.last_message_persisted_at = persisted;
                status.last_reconciliation_at = reconciled;
            }
            status.inconsistent_chats = conn
                .query_row(
                    "SELECT count(*) FROM chats c WHERE
                       c.last_message_id<>COALESCE((SELECT id FROM messages WHERE chat_jid=c.jid AND kind<>'reaction' ORDER BY timestamp DESC,id DESC LIMIT 1),'')
                       OR c.last_message_text<>COALESCE((SELECT text FROM messages WHERE chat_jid=c.jid AND kind<>'reaction' ORDER BY timestamp DESC,id DESC LIMIT 1),'')
                       OR c.last_message_at<>COALESCE((SELECT timestamp FROM messages WHERE chat_jid=c.jid AND kind<>'reaction' ORDER BY timestamp DESC,id DESC LIMIT 1),0)
                       OR c.unread_count<>(SELECT count(*) FROM messages WHERE chat_jid=c.jid AND kind<>'reaction' AND unread=1)",
                    [],
                    |row| row.get::<_, i64>(0),
                )?
                .max(0) as u64;
            if status.database_ok && status.inconsistent_chats == 0 {
                status.detail = "Local database and derived chat state are consistent".into();
            } else if status.database_ok {
                status.detail =
                    format!("{} chat summaries need repair", status.inconsistent_chats);
            }
            Ok(status)
        })
    }

    /// Rebuilds the derived chat summaries from the message rows and stamps
    /// the repair, the Go `RepairDerivedState` minus the FTS rebuild: this
    /// port searches with trigram LIKE, it keeps no `message_search` table.
    pub fn repair_local_cache(&self) -> rusqlite::Result<IntegrityStatusData> {
        self.write(|conn| {
            conn.execute(
                "UPDATE chats SET
                   last_message_id=COALESCE((SELECT id FROM messages WHERE chat_jid=chats.jid AND kind<>'reaction' ORDER BY timestamp DESC,id DESC LIMIT 1),''),
                   last_message_text=COALESCE((SELECT text FROM messages WHERE chat_jid=chats.jid AND kind<>'reaction' ORDER BY timestamp DESC,id DESC LIMIT 1),''),
                   last_message_at=COALESCE((SELECT timestamp FROM messages WHERE chat_jid=chats.jid AND kind<>'reaction' ORDER BY timestamp DESC,id DESC LIMIT 1),0),
                   unread_count=(SELECT count(*) FROM messages WHERE chat_jid=chats.jid AND kind<>'reaction' AND unread=1)",
                [],
            )?;
            conn.execute(
                "INSERT INTO storage_health(singleton,last_reconciliation_at) VALUES(1,?1)
                 ON CONFLICT(singleton) DO UPDATE SET last_reconciliation_at=excluded.last_reconciliation_at",
                params![now_ms()],
            )?;
            Ok(())
        })?;
        self.integrity_status()
    }

    // ------------------------------------------------------------------
    // Poll key material and poll votes
    // ------------------------------------------------------------------

    /// Stores who created a poll and the secret its votes encrypt with, the
    /// two fields a later `VotePoll` needs and the definition row never had.
    pub fn set_poll_meta(
        &self,
        chat_jid: &str,
        message_id: &str,
        creator_jid: &str,
        secret: &[u8],
    ) -> rusqlite::Result<()> {
        let chat_jid = self.resolve_chat_id(chat_jid)?;
        self.write(|conn| {
            conn.execute(
                "UPDATE polls SET creator_jid=?3,message_secret=?4
                 WHERE chat_jid=?1 AND message_id=?2",
                params![chat_jid, message_id, creator_jid, secret],
            )?;
            Ok(())
        })
    }

    /// The key material of one stored poll, if the definition knows it.
    pub fn poll_meta(
        &self,
        chat_jid: &str,
        message_id: &str,
    ) -> rusqlite::Result<Option<PollMeta>> {
        let chat_id = self.resolve_chat_id(chat_jid)?;
        self.read(|conn| {
            conn.query_row(
                "SELECT creator_jid,message_secret FROM polls
                 WHERE chat_jid=?1 AND message_id=?2",
                params![chat_id, message_id],
                |row| {
                    Ok(PollMeta {
                        creator_jid: row.get(0)?,
                        message_secret: row.get(1)?,
                    })
                },
            )
            .optional()
        })
    }

    /// Records one vote on a stored poll, newest write wins per voter like
    /// the merge of the Go's `ApplyPollVote`.
    pub fn apply_poll_vote(
        &self,
        chat_jid: &str,
        poll_message_id: &str,
        voter_jid: &str,
        selected: &[String],
        timestamp: i64,
        from_me: bool,
    ) -> rusqlite::Result<()> {
        let chat_id = self.resolve_chat_id(chat_jid)?;
        let mut selected_json = String::from("[");
        for (index, option) in selected.iter().enumerate() {
            if index > 0 {
                selected_json.push(',');
            }
            selected_json.push('"');
            for character in option.chars() {
                match character {
                    '"' => selected_json.push_str("\\\""),
                    '\\' => selected_json.push_str("\\\\"),
                    other => selected_json.push(other),
                }
            }
            selected_json.push('"');
        }
        selected_json.push(']');
        self.write(|conn| {
            conn.execute(
                "INSERT INTO poll_votes(chat_jid,poll_message_id,voter_jid,selected_json,timestamp,from_me)
                 VALUES(?1,?2,?3,?4,?5,?6)
                 ON CONFLICT(chat_jid,poll_message_id,voter_jid) DO UPDATE SET
                   selected_json=excluded.selected_json,timestamp=excluded.timestamp,
                   from_me=excluded.from_me
                 WHERE excluded.timestamp>=poll_votes.timestamp",
                params![
                    chat_id,
                    poll_message_id,
                    voter_jid,
                    selected_json,
                    timestamp,
                    from_me as i64
                ],
            )?;
            Ok(())
        })
    }

    // ------------------------------------------------------------------
    // Legacy reaction replays
    // ------------------------------------------------------------------

    /// Records one placeholder the repair wave may resend, the rows the Go
    /// journal drain queued for `ReserveLegacyReactionReplays`.
    #[expect(clippy::too_many_arguments, reason = "one argument per replay column")]
    pub fn record_legacy_replay(
        &self,
        chat_jid: &str,
        transport_jid: &str,
        event_message_id: &str,
        sender_jid: &str,
        timestamp: i64,
        from_me: bool,
        status: i64,
    ) -> rusqlite::Result<()> {
        let chat_id = self.resolve_chat_id(chat_jid)?;
        self.write(|conn| {
            conn.execute(
                "INSERT INTO legacy_reaction_replays(chat_jid,transport_jid,event_message_id,
                        sender_jid,timestamp,from_me,status,requested_at,attempts,completed_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,0,0,0)
                 ON CONFLICT(chat_jid,event_message_id) DO NOTHING",
                params![
                    chat_id,
                    transport_jid,
                    event_message_id,
                    sender_jid,
                    timestamp,
                    from_me as i64,
                    status,
                ],
            )?;
            Ok(())
        })
    }

    /// The pending placeholder resends of one chat, oldest first, the rows
    /// the Go `ReserveLegacyReactionReplays` picked for one repair call.
    pub fn pending_legacy_replays(
        &self,
        chat_jid: &str,
        limit: u32,
    ) -> rusqlite::Result<Vec<LegacyReplayRow>> {
        let chat_id = self.resolve_chat_id(chat_jid)?;
        self.read(|conn| {
            let mut statement = conn.prepare(
                "SELECT chat_jid,transport_jid,event_message_id,sender_jid,timestamp,
                        from_me,status,request_id,requested_at,attempts
                 FROM legacy_reaction_replays
                 WHERE chat_jid=?1 AND completed_at=0
                 ORDER BY timestamp LIMIT ?2",
            )?;
            let rows = statement
                .query_map(params![chat_id, limit as i64], |row| {
                    Ok(LegacyReplayRow {
                        chat_jid: row.get(0)?,
                        transport_jid: row.get(1)?,
                        event_message_id: row.get(2)?,
                        sender_jid: row.get(3)?,
                        timestamp: row.get(4)?,
                        from_me: row.get::<_, i64>(5)? != 0,
                        status: row.get(6)?,
                        request_id: row.get(7)?,
                        requested_at: row.get(8)?,
                        attempts: row.get::<_, i64>(9)?.max(0) as u32,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// Marks one replay as requested under this repair call, bumping its
    /// attempt count like the Go reservation did.
    pub fn mark_legacy_replay_requested(
        &self,
        chat_jid: &str,
        event_message_id: &str,
        request_id: &str,
    ) -> rusqlite::Result<()> {
        let chat_id = self.resolve_chat_id(chat_jid)?;
        let now = now_ms();
        self.write(|conn| {
            conn.execute(
                "UPDATE legacy_reaction_replays
                 SET request_id=?3,requested_at=?4,attempts=attempts+1
                 WHERE chat_jid=?1 AND event_message_id=?2",
                params![chat_id, event_message_id, request_id, now],
            )?;
            Ok(())
        })
    }

    /// Marks one replay as done after its placeholder round-trip.
    pub fn complete_legacy_replay(
        &self,
        chat_jid: &str,
        event_message_id: &str,
    ) -> rusqlite::Result<()> {
        let chat_id = self.resolve_chat_id(chat_jid)?;
        self.write(|conn| {
            conn.execute(
                "UPDATE legacy_reaction_replays SET completed_at=?3
                 WHERE chat_jid=?1 AND event_message_id=?2",
                params![chat_id, event_message_id, now_ms()],
            )?;
            Ok(())
        })
    }
}

/// One pinned message row, without the message it points at.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PinnedRow {
    pub message_id: String,
    pub pinned_at: i64,
    pub pinned_by: String,
}

/// One sticker message that can supply its bytes on demand.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StickerCandidateRow {
    /// Hex of the plaintext file hash: the Go id space for history stickers.
    pub id: String,
    pub chat_jid: String,
    pub message_id: String,
    pub mime: String,
    pub width: i64,
    pub height: i64,
    pub animated: bool,
    pub from_me: bool,
    pub timestamp: i64,
}

/// One sticker WhatsApp synced as favourited on another device.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FavoriteStickerRow {
    /// Hex of the encrypted file hash: the id space WhatsApp syncs.
    pub id: String,
    pub mime: String,
    pub direct_path: String,
    pub media_key: Vec<u8>,
    pub file_enc_sha256: Vec<u8>,
    pub width: i64,
    pub height: i64,
    pub animated: bool,
    pub file_size: i64,
    pub local_path: String,
    pub updated_at: i64,
}

/// One stored poll, with every option's tally, the Go `Poll` shape.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PollSnapshot {
    pub message_id: String,
    pub question: String,
    pub selectable_count: u32,
    pub total_voters: u32,
    pub options: Vec<PollOptionRow>,
}

/// One option of a stored poll.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PollOptionRow {
    pub name: String,
    pub votes: u32,
    pub selected_by_me: bool,
    pub voters: Vec<PollVoterRow>,
}

/// One stored vote on one option.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PollVoterRow {
    pub jid: String,
    pub from_me: bool,
}

/// One unread row a read receipt still owes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UnreadRow {
    pub id: String,
    pub transport_jid: String,
    pub sender_jid: String,
    pub timestamp: i64,
}

/// What a through-message read found.
#[derive(Debug, Clone, PartialEq)]
pub enum UnreadThrough {
    /// The named message is not stored.
    Missing,
    /// The named message is outgoing; the Go refused it by name.
    Outgoing,
    /// The unread rows at or before it, oldest first.
    Rows(Vec<UnreadRow>),
}

/// What one outgoing reaction reservation means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReactionReservation {
    /// Never sent, or reserved and interrupted: send it.
    Send,
    /// Already sent at this instant: replay, do not send again.
    Completed(i64),
}

fn favorite_sticker_from_row(row: &Row) -> rusqlite::Result<FavoriteStickerRow> {
    Ok(FavoriteStickerRow {
        id: row.get(0)?,
        mime: row.get(1)?,
        direct_path: row.get(2)?,
        media_key: row.get(3)?,
        file_enc_sha256: row.get(4)?,
        width: row.get(5)?,
        height: row.get(6)?,
        animated: row.get::<_, i64>(7)? != 0,
        file_size: row.get(8)?,
        local_path: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

/// The lowercase hex of a byte string, the id format the sticker tables use.
pub(crate) fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
        out.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
    }
    out
}

/// The option names of a stored poll definition. This port writes a JSON
/// array of strings; the Go schema stored `domain.PollOption` objects, whose
/// `name` field is what a migrated row carries.
fn decode_poll_option_names(raw: &str) -> Vec<String> {
    if raw.contains("\"name\"") {
        let mut names = Vec::new();
        let mut rest = raw;
        while let Some(index) = rest.find("\"name\"") {
            rest = &rest[index + 6..];
            let Some(colon) = rest.find(':') else { break };
            rest = &rest[colon + 1..];
            let Some(quote) = rest.find('"') else { break };
            let (value, next) = read_json_string(&rest[quote + 1..]);
            names.push(value);
            rest = next;
        }
        return names;
    }
    decode_json_strings(raw)
}

/// Every string element of one JSON array of strings.
fn decode_json_strings(raw: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut rest = raw;
    while let Some(index) = rest.find('"') {
        let (value, next) = read_json_string(&rest[index + 1..]);
        values.push(value);
        rest = next;
    }
    values
}

/// One JSON string body and whatever follows its closing quote. Only the two
/// escapes the writers emit are decoded; anything else is kept verbatim.
fn read_json_string(raw: &str) -> (String, &str) {
    let mut value = String::new();
    let mut escaped = false;
    for (index, character) in raw.char_indices() {
        if escaped {
            if character != '"' && character != '\\' {
                value.push('\\');
            }
            value.push(character);
            escaped = false;
            continue;
        }
        match character {
            '\\' => escaped = true,
            '"' => return (value, &raw[index + 1..]),
            other => value.push(other),
        }
    }
    (value, "")
}

// ----------------------------------------------------------------------
// Row plumbing
// ----------------------------------------------------------------------

const CHAT_SELECT: &str = "
SELECT c.jid, c.preferred_jid, c.name, c.last_message_id, c.last_message_text,
       c.last_message_at,
       COALESCE((SELECT m.from_me FROM messages m WHERE m.chat_jid=c.jid
                 ORDER BY m.timestamp DESC, m.id DESC LIMIT 1), 0) AS last_from_me,
       c.unread_count, c.muted_until, c.archived, c.pinned, c.blocked,
       COALESCE((SELECT a.jid FROM chat_addresses a WHERE a.chat_jid=c.jid
                 ORDER BY a.last_seen_at DESC LIMIT 1), c.preferred_jid) AS address_jid,
       COALESCE(k.contact_name,''), COALESCE(k.push_name,''),
       COALESCE(k.business_name,''), COALESCE(k.phone_number,'')
FROM chats c
LEFT JOIN contacts k ON k.jid = c.preferred_jid";

fn chat_from_row(row: &Row) -> rusqlite::Result<ChatRow> {
    Ok(ChatRow {
        jid: row.get(0)?,
        preferred_jid: row.get(1)?,
        name: row.get(2)?,
        last_message_id: row.get(3)?,
        last_message_text: row.get(4)?,
        last_message_at: row.get(5)?,
        last_message_from_me: row.get::<_, i64>(6)? != 0,
        unread_count: row.get(7)?,
        muted_until: row.get(8)?,
        archived: row.get::<_, i64>(9)? != 0,
        pinned: row.get::<_, i64>(10)? != 0,
        blocked: row.get::<_, i64>(11)? != 0,
        address_jid: row.get(12)?,
        contact_name: row.get(13)?,
        push_name: row.get(14)?,
        business_name: row.get(15)?,
        phone_number: row.get(16)?,
        named: false,
    })
}

const MESSAGE_SELECT: &str = "SELECT m.* FROM messages m";

/// Inserts the conversation when missing, shared by `Store::ensure_chat`
/// and the message insert: one conversation per address, the display state
/// of an existing row untouched.
fn ensure_chat_in(conn: &Connection, jid: &str, preferred_jid: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO chats(jid,preferred_jid) VALUES(?1,?2)
         ON CONFLICT(jid) DO NOTHING",
        params![jid, preferred_jid],
    )?;
    conn.execute(
        "INSERT INTO chat_addresses(jid,chat_jid,last_seen_at) VALUES(?1,?2,?3)
         ON CONFLICT(jid) DO UPDATE SET chat_jid=excluded.chat_jid,last_seen_at=excluded.last_seen_at",
        params![preferred_jid, jid, now_ms()],
    )?;
    Ok(())
}

/// Whether a store error is a foreign-key miss on the chat of a message:
/// `INSERT OR IGNORE` ignores id conflicts but fails hard without the chat.
fn is_missing_chat(error: &rusqlite::Error) -> bool {
    error.to_string().contains("FOREIGN KEY")
}

fn message_from_row(row: &Row) -> rusqlite::Result<MessageRow> {
    Ok(MessageRow {
        id: row.get("id")?,
        chat_jid: row.get("chat_jid")?,
        transport_jid: row.get("transport_jid")?,
        sender_jid: row.get("sender_jid")?,
        text: row.get("text")?,
        timestamp: row.get("timestamp")?,
        from_me: row.get::<_, i64>("from_me")? != 0,
        status: row.get("status")?,
        kind: row.get("kind")?,
        reply_to_id: row.get("reply_to_id")?,
        reply_to_chat_id: row.get("reply_to_chat_id")?,
        edited_at: row.get("edited_at")?,
        revoked: row.get::<_, i64>("revoked")? != 0,
        forwarding_score: row.get("forwarding_score")?,
        unread: row.get::<_, i64>("unread")? != 0,
        reactions: Vec::new(),
        payload: row.get("payload").unwrap_or_default(),
        image_mime: row.get("image_mime")?,
        image_caption: row.get("image_caption")?,
        image_thumbnail: row.get("image_thumbnail")?,
        image_local_path: row.get("image_local_path")?,
        image_direct_path: row.get("image_direct_path")?,
        image_media_key: row.get("image_media_key")?,
        image_file_sha256: row.get("image_file_sha256")?,
        image_file_enc_sha256: row.get("image_file_enc_sha256")?,
        image_width: row.get("image_width")?,
        image_height: row.get("image_height")?,
        image_size: row.get("image_size")?,
        image_animated: row.get::<_, i64>("image_animated")? != 0,
        media_mime: row.get("media_mime")?,
        media_file_name: row.get("media_file_name")?,
        media_local_path: row.get("media_local_path")?,
        media_direct_path: row.get("media_direct_path")?,
        media_media_key: row.get("media_media_key")?,
        media_file_sha256: row.get("media_file_sha256")?,
        media_file_enc_sha256: row.get("media_file_enc_sha256")?,
        media_width: row.get("media_width")?,
        media_height: row.get("media_height")?,
        media_size: row.get("media_size")?,
        media_duration: row.get("media_duration")?,
        media_voice: row.get::<_, i64>("media_voice")? != 0,
        media_animated: row.get::<_, i64>("media_animated")? != 0,
        media_caption: row.get("media_caption")?,
        contacts_json: row.get("contacts_json")?,
        location_lat: row.get("location_lat")?,
        location_lng: row.get("location_lng")?,
        location_name: row.get("location_name")?,
        location_address: row.get("location_address")?,
        location_url: row.get("location_url")?,
        location_live: row.get::<_, i64>("location_live")? != 0,
        link_preview_url: row.get("link_preview_url")?,
        link_preview_title: row.get("link_preview_title")?,
        link_preview_description: row.get("link_preview_description")?,
        link_preview_thumbnail: row.get("link_preview_thumbnail")?,
        link_preview_width: row.get("link_preview_width")?,
        link_preview_height: row.get("link_preview_height")?,
    })
}

/// The preview text the chat list shows for one row: the body when it has
/// one, otherwise the image or attachment caption. The Go stored the plain
/// body; the caption fallback is this port's rendering rule.
fn preview_text(message: &MessageRow) -> String {
    if !message.text.is_empty() {
        message.text.clone()
    } else if !message.image_caption.is_empty() {
        message.image_caption.clone()
    } else {
        message.media_caption.clone()
    }
}

/// Advances the owning chat after a message write: the pointer and its
/// preview only move forward, a new incoming unread row grows the counter
/// once, and the storage timestamp records the write. This is the Go
/// reducer's chat update, statement for statement.
fn finish_message_write(
    conn: &Connection,
    stored: &MessageRow,
    timestamp: i64,
    inserted: bool,
) -> rusqlite::Result<bool> {
    let unread = i64::from(inserted && stored.unread && !stored.from_me);
    let preview = preview_text(stored);
    conn.execute(
        "UPDATE chats SET
           last_message_id=CASE WHEN last_message_at<?2
                                  OR (last_message_at=?2 AND last_message_id<?3)
                                THEN ?3 ELSE last_message_id END,
           last_message_text=CASE WHEN last_message_id=?3
                                    OR last_message_at<?2
                                    OR (last_message_at=?2 AND last_message_id<?3)
                                  THEN ?4 ELSE last_message_text END,
           last_message_at=max(last_message_at,?2),
           unread_count=unread_count+?5
         WHERE jid=?1",
        params![stored.chat_jid, timestamp, stored.id, preview, unread],
    )?;
    conn.execute(
        "INSERT INTO storage_health(singleton,last_message_persisted_at) VALUES(1,?1)
         ON CONFLICT(singleton) DO UPDATE SET
           last_message_persisted_at=max(storage_health.last_message_persisted_at,excluded.last_message_persisted_at)",
        params![now_ms()],
    )?;
    Ok(inserted)
}

/// Overwrites one stored row with its merged state.
fn update_message_row(conn: &Connection, message: &MessageRow) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE messages SET
           transport_jid=?3, sender_jid=?4, text=?5, timestamp=?6, from_me=?7, status=?8,
           kind=?9, reply_to_id=?10, reply_to_chat_id=?11, edited_at=?12, revoked=?13,
           forwarding_score=?14, unread=?15,
           image_mime=?16, image_caption=?17, image_thumbnail=?18, image_local_path=?19,
           image_direct_path=?20, image_media_key=?21, image_file_sha256=?22,
           image_file_enc_sha256=?23, image_width=?24, image_height=?25, image_size=?26,
           image_animated=?27, media_mime=?28, media_file_name=?29, media_local_path=?30,
           media_direct_path=?31, media_media_key=?32, media_file_sha256=?33,
           media_file_enc_sha256=?34, media_width=?35, media_height=?36, media_size=?37,
           media_duration=?38, media_voice=?39, media_animated=?40, media_caption=?41,
           contacts_json=?42, location_lat=?43, location_lng=?44, location_name=?45,
           location_address=?46, location_url=?47, location_live=?48,
           link_preview_url=?49, link_preview_title=?50, link_preview_description=?51,
           link_preview_thumbnail=?52, link_preview_width=?53, link_preview_height=?54,
           payload=?55
         WHERE chat_jid=?1 AND id=?2",
        params![
            message.chat_jid,
            message.id,
            message.transport_jid,
            message.sender_jid,
            message.text,
            message.timestamp,
            message.from_me as i64,
            message.status,
            message.kind,
            message.reply_to_id,
            message.reply_to_chat_id,
            message.edited_at,
            message.revoked as i64,
            message.forwarding_score,
            message.unread as i64,
            message.image_mime,
            message.image_caption,
            message.image_thumbnail,
            message.image_local_path,
            message.image_direct_path,
            message.image_media_key,
            message.image_file_sha256,
            message.image_file_enc_sha256,
            message.image_width,
            message.image_height,
            message.image_size,
            message.image_animated as i64,
            message.media_mime,
            message.media_file_name,
            message.media_local_path,
            message.media_direct_path,
            message.media_media_key,
            message.media_file_sha256,
            message.media_file_enc_sha256,
            message.media_width,
            message.media_height,
            message.media_size,
            message.media_duration,
            message.media_voice as i64,
            message.media_animated as i64,
            message.media_caption,
            message.contacts_json,
            message.location_lat,
            message.location_lng,
            message.location_name,
            message.location_address,
            message.location_url,
            message.location_live as i64,
            message.link_preview_url,
            message.link_preview_title,
            message.link_preview_description,
            message.link_preview_thumbnail,
            message.link_preview_width,
            message.link_preview_height,
            message.payload
        ],
    )?;
    Ok(())
}

/// Folds a re-delivered row into the stored one with the Go reducer's rules:
/// a mutation wins, stale history only fills blanks, and nothing already
/// known (a timestamp, a download path, a sender) is erased.
fn merge_message(old: &MessageRow, new: &MessageRow, timestamp: i64) -> MessageRow {
    let newer_edit = new.edited_at > old.edited_at;
    let same_edit = new.edited_at == old.edited_at;
    let text = if old.revoked && !new.revoked {
        old.text.clone()
    } else if newer_edit || (same_edit && !new.text.is_empty()) {
        new.text.clone()
    } else {
        old.text.clone()
    };
    let kind = if old.revoked {
        old.kind.clone()
    } else if new.revoked || newer_edit || (same_edit && !new.kind.is_empty()) {
        new.kind.clone()
    } else {
        old.kind.clone()
    };
    let status = if new.status == 5 && old.status <= 1 {
        5
    } else if new.status == 5 {
        old.status
    } else if old.status == 5 {
        new.status
    } else {
        old.status.max(new.status)
    };
    let image_caption = if newer_edit {
        new.image_caption.clone()
    } else {
        fill_text(&new.image_caption, &old.image_caption)
    };
    let reply_to_id = if newer_edit || old.reply_to_id.is_empty() {
        new.reply_to_id.clone()
    } else {
        old.reply_to_id.clone()
    };
    let reply_to_chat_id = if newer_edit || old.reply_to_chat_id.is_empty() {
        new.reply_to_chat_id.clone()
    } else {
        old.reply_to_chat_id.clone()
    };
    let (location_lat, location_lng) = if new.location_lat != 0.0 || new.location_lng != 0.0 {
        (new.location_lat, new.location_lng)
    } else {
        (old.location_lat, old.location_lng)
    };
    MessageRow {
        id: old.id.clone(),
        chat_jid: old.chat_jid.clone(),
        transport_jid: if new.transport_jid.ends_with("@lid") || old.transport_jid.is_empty() {
            new.transport_jid.clone()
        } else {
            old.transport_jid.clone()
        },
        sender_jid: if old.sender_jid.is_empty() {
            new.sender_jid.clone()
        } else {
            old.sender_jid.clone()
        },
        text,
        timestamp: if old.timestamp == 0 {
            timestamp
        } else {
            old.timestamp
        },
        from_me: old.from_me,
        status,
        kind,
        reply_to_id,
        reply_to_chat_id,
        edited_at: old.edited_at.max(new.edited_at),
        revoked: old.revoked || new.revoked,
        forwarding_score: old.forwarding_score.max(new.forwarding_score),
        unread: old.unread,
        reactions: Vec::new(),
        payload: fill_blob(&new.payload, &old.payload),
        image_mime: fill_text(&new.image_mime, &old.image_mime),
        image_caption,
        image_thumbnail: fill_blob(&new.image_thumbnail, &old.image_thumbnail),
        image_local_path: fill_text(&new.image_local_path, &old.image_local_path),
        image_direct_path: fill_text(&new.image_direct_path, &old.image_direct_path),
        image_media_key: fill_blob(&new.image_media_key, &old.image_media_key),
        image_file_sha256: fill_blob(&new.image_file_sha256, &old.image_file_sha256),
        image_file_enc_sha256: fill_blob(&new.image_file_enc_sha256, &old.image_file_enc_sha256),
        image_width: fill_number(new.image_width, old.image_width),
        image_height: fill_number(new.image_height, old.image_height),
        image_size: fill_number(new.image_size, old.image_size),
        image_animated: old.image_animated || new.image_animated,
        media_mime: fill_text(&new.media_mime, &old.media_mime),
        media_file_name: fill_text(&new.media_file_name, &old.media_file_name),
        media_local_path: fill_text(&new.media_local_path, &old.media_local_path),
        media_direct_path: fill_text(&new.media_direct_path, &old.media_direct_path),
        media_media_key: fill_blob(&new.media_media_key, &old.media_media_key),
        media_file_sha256: fill_blob(&new.media_file_sha256, &old.media_file_sha256),
        media_file_enc_sha256: fill_blob(&new.media_file_enc_sha256, &old.media_file_enc_sha256),
        media_width: fill_number(new.media_width, old.media_width),
        media_height: fill_number(new.media_height, old.media_height),
        media_size: fill_number(new.media_size, old.media_size),
        media_duration: fill_number(new.media_duration, old.media_duration),
        media_voice: old.media_voice || new.media_voice,
        media_animated: old.media_animated || new.media_animated,
        media_caption: fill_text(&new.media_caption, &old.media_caption),
        contacts_json: fill_text(&new.contacts_json, &old.contacts_json),
        location_lat,
        location_lng,
        location_name: fill_text(&new.location_name, &old.location_name),
        location_address: fill_text(&new.location_address, &old.location_address),
        location_url: fill_text(&new.location_url, &old.location_url),
        location_live: old.location_live || new.location_live,
        link_preview_url: fill_text(&new.link_preview_url, &old.link_preview_url),
        link_preview_title: fill_text(&new.link_preview_title, &old.link_preview_title),
        link_preview_description: fill_text(
            &new.link_preview_description,
            &old.link_preview_description,
        ),
        link_preview_thumbnail: fill_blob(&new.link_preview_thumbnail, &old.link_preview_thumbnail),
        link_preview_width: fill_number(new.link_preview_width, old.link_preview_width),
        link_preview_height: fill_number(new.link_preview_height, old.link_preview_height),
    }
}

fn fill_text(new: &str, old: &str) -> String {
    if new.is_empty() {
        old.to_owned()
    } else {
        new.to_owned()
    }
}

fn fill_blob(new: &[u8], old: &[u8]) -> Vec<u8> {
    if new.is_empty() {
        old.to_vec()
    } else {
        new.to_vec()
    }
}

fn fill_number(new: i64, old: i64) -> i64 {
    if new > 0 { new } else { old }
}

/// The Go `clampLimit`: zero means the default page, and no page exceeds 200.
fn clamp_limit(limit: u32) -> u32 {
    if limit == 0 { 50 } else { limit.min(200) }
}

/// A validation failure with the message the caller sees on the wire.
fn invalid_parameter(message: &str) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
        Some(message.to_owned()),
    )
}
/// A candidate row of the message pre-filter, with the fields the fuzzy
/// scorer reads.
struct SearchCandidate {
    chat_jid: String,
    message_id: String,
    timestamp: i64,
    text: String,
    image_caption: String,
    media_file_name: String,
    contacts_json: String,
    location_name: String,
    location_address: String,
    score: i64,
}

/// The Go sidecar's fuzzy matcher (`internal/search`): normalization, the
/// prefix/word/contains ladder, a bounded Damerau-Levenshtein distance for
/// typos, and a subsequence fallback. Search pre-filters with trigrams and
/// ranks with this, exactly like the oracle.
struct SearchMatcher {
    query: String,
    runes: Vec<char>,
}

impl SearchMatcher {
    fn new(query: &str) -> Self {
        let query = normalize_search_text(query);
        let runes = query.chars().collect();
        Self { query, runes }
    }

    /// The Go `Matcher.Score`; `None` is its `NoMatch` (-1).
    fn score(&self, candidate: &str) -> Option<i64> {
        if self.query.is_empty() {
            return None;
        }
        let candidate = normalize_search_text(candidate);
        if candidate.is_empty() {
            return None;
        }
        if candidate == self.query {
            return Some(1000);
        }
        if candidate.starts_with(&self.query) {
            return Some(930);
        }
        for word in candidate.split_whitespace() {
            if word == self.query {
                return Some(910);
            }
            if word.starts_with(&self.query) {
                return Some(880);
            }
        }
        if candidate.contains(&self.query) {
            return Some(820);
        }
        let threshold = (self.runes.len() / 4).max(1);
        let mut parts: Vec<&str> = candidate.split_whitespace().collect();
        parts.push(&candidate);
        let mut best: Option<i64> = None;
        for part in parts {
            let part_runes: Vec<char> = part.chars().collect();
            let delta = part_runes.len().abs_diff(self.runes.len());
            if delta > threshold {
                continue;
            }
            let distance = damerau_levenshtein(&self.runes, &part_runes, threshold);
            if distance <= threshold {
                let score = 760 - (distance as i64) * 80 - (delta as i64) * 10;
                best = Some(best.map_or(score, |best| best.max(score)));
            }
        }
        if best.is_some() {
            return best;
        }
        let candidate_runes: Vec<char> = candidate.chars().collect();
        subsequence_score(&self.runes, &candidate_runes)
    }
}

/// The best field score plus the field's bonus, the Go `SearchMessages`
/// ranking rule.
fn score_search_candidate(matcher: &SearchMatcher, candidate: &SearchCandidate) -> Option<i64> {
    let fields = [
        (candidate.text.as_str(), 200),
        (candidate.image_caption.as_str(), 180),
        (candidate.media_file_name.as_str(), 160),
        (candidate.contacts_json.as_str(), 150),
        (candidate.location_name.as_str(), 150),
        (candidate.location_address.as_str(), 150),
    ];
    fields
        .iter()
        .filter(|(value, _)| !value.is_empty())
        .filter_map(|(value, bonus)| matcher.score(value).map(|score| score + bonus))
        .max()
}

/// The unique trigrams of a normalized query. The cap keeps the LIKE fallback
/// bounded; a hit only needs one shared trigram, which is also true of the
/// FTS5 OR query the Go built.
fn search_trigrams(query: &str) -> Vec<String> {
    let runes: Vec<char> = query.chars().collect();
    if runes.len() < 3 {
        return Vec::new();
    }
    let mut seen = std::collections::HashSet::new();
    let mut terms = Vec::new();
    for window in runes.windows(3) {
        let term: String = window.iter().collect();
        if seen.insert(term.clone()) && terms.len() < 12 {
            terms.push(term);
        }
    }
    terms
}

/// Escapes the LIKE wildcards so a literal `%` or `_` in the query stays a
/// literal, the guard the Go got from the FTS5 tokenizer.
fn escape_like(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '%' | '_' | '\\' => {
                out.push('\\');
                out.push(character);
            }
            other => out.push(other),
        }
    }
    out
}

/// Lowercase, trim, collapse internal whitespace, and fold the Latin accents
/// the Go's NFD normalization stripped.
fn normalize_search_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut space = false;
    for character in value.trim().chars().flat_map(char::to_lowercase) {
        if character.is_whitespace() {
            space = !out.is_empty();
            continue;
        }
        if space {
            out.push(' ');
            space = false;
        }
        out.push(fold_accent(character));
    }
    out
}

fn fold_accent(character: char) -> char {
    match character {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'č' => 'c',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' => 'e',
        'ì' | 'í' | 'î' | 'ï' | 'ī' | 'į' => 'i',
        'ñ' | 'ń' | 'ň' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => 'o',
        'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' => 'u',
        'ý' | 'ÿ' => 'y',
        'ś' | 'š' | 'ş' => 's',
        'ź' | 'ż' | 'ž' => 'z',
        'ł' => 'l',
        'đ' | 'ď' => 'd',
        'ť' | 'ţ' => 't',
        'ř' => 'r',
        'ğ' => 'g',
        _ => character,
    }
}

/// Damerau-Levenshtein with the Go matcher's early exit: a row whose minimum
/// already exceeds the cutoff stops the scan.
fn damerau_levenshtein(left: &[char], right: &[char], cutoff: usize) -> usize {
    if left.is_empty() {
        return right.len();
    }
    let mut previous_previous = vec![0usize; right.len() + 1];
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0usize; right.len() + 1];
    for i in 1..=left.len() {
        current[0] = i;
        let mut row_min = current[0];
        for j in 1..=right.len() {
            let cost = usize::from(left[i - 1] != right[j - 1]);
            current[j] = (previous[j] + 1)
                .min(current[j - 1] + 1)
                .min(previous[j - 1] + cost);
            if i > 1 && j > 1 && left[i - 1] == right[j - 2] && left[i - 2] == right[j - 1] {
                current[j] = current[j].min(previous_previous[j - 2] + 1);
            }
            row_min = row_min.min(current[j]);
        }
        if row_min > cutoff {
            return cutoff + 1;
        }
        std::mem::swap(&mut previous_previous, &mut previous);
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

/// The Go `subsequenceScore`: matched in order with a gap penalty.
fn subsequence_score(query: &[char], candidate: &[char]) -> Option<i64> {
    if query.is_empty() || query.len() > candidate.len() {
        return None;
    }
    let mut matched = 0;
    let mut first = -1i64;
    let mut previous = -1i64;
    let mut gaps = 0i64;
    for (index, character) in candidate.iter().enumerate() {
        if *character != query[matched] {
            continue;
        }
        let index = index as i64;
        if first == -1 {
            first = index;
        }
        if previous >= 0 {
            gaps += index - previous - 1;
        }
        previous = index;
        matched += 1;
        if matched == query.len() {
            return Some(650 - first * 4 - gaps * 8);
        }
    }
    None
}

/// Joins the reactions of every row in place, one query for the batch.
fn attach_reactions(conn: &Connection, rows: &mut [MessageRow]) -> rusqlite::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    // The page is one chat by construction in every caller.
    let chat_jid = rows[0].chat_jid.clone();
    let mut statement = conn.prepare(
        "SELECT message_id,sender_jid,emoji,timestamp,from_me
         FROM reactions WHERE chat_jid=?1 AND emoji<>''
         ORDER BY timestamp ASC, sender_jid ASC",
    )?;
    let mut all: Vec<ReactionRow> = statement
        .query_map(params![chat_jid], |row| {
            Ok(ReactionRow {
                chat_jid: chat_jid.clone(),
                message_id: row.get(0)?,
                sender_jid: row.get(1)?,
                emoji: row.get(2)?,
                timestamp: row.get(3)?,
                from_me: row.get::<_, i64>(4)? != 0,
            })
        })?
        .collect::<Result<_, _>>()?;
    all.sort_by(|a, b| a.message_id.cmp(&b.message_id));
    for row in rows.iter_mut() {
        let start = all.partition_point(|reaction| reaction.message_id.as_str() < row.id.as_str());
        let end = all.partition_point(|reaction| reaction.message_id.as_str() <= row.id.as_str());
        row.reactions = all[start..end].to_vec();
    }
    Ok(())
}

/// The chat flag columns the desktop owns, named so `set_chat_flag` cannot
/// be handed an arbitrary column.
#[derive(Debug, Clone, Copy)]
pub enum ChatFlag {
    Archived,
    Pinned,
    Blocked,
}

impl ChatFlag {
    fn as_str(self) -> &'static str {
        match self {
            ChatFlag::Archived => "archived",
            ChatFlag::Pinned => "pinned",
            ChatFlag::Blocked => "blocked",
        }
    }
}

/// One row of `history_coverage` plus the live counts the Go derived on
/// every read: the local message total and the oldest timestamp.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HistoryCoverageData {
    pub chat_id: String,
    pub state: String,
    pub revision: u64,
    pub local_message_count: u64,
    pub oldest_message_timestamp: i64,
    pub on_demand_added: u64,
    pub last_requested_count: u32,
    pub last_received_count: u32,
    pub last_added_count: u32,
    pub last_requested_at: i64,
    pub retry_after: i64,
    pub detail: String,
}

/// The five counters of `HistoryOverview`, in the Go's order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HistoryOverviewData {
    pub local_message_count: u64,
    pub chats_with_messages: u64,
    pub chats_checked: u64,
    pub chats_unknown: u64,
    pub chats_without_anchor: u64,
}

/// The local health the Go `IntegrityStatus` reported.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IntegrityStatusData {
    pub database_ok: bool,
    pub detail: String,
    pub pending_inbound: u64,
    pub failed_inbound: u64,
    pub oldest_pending_at: i64,
    pub last_message_persisted_at: i64,
    pub last_reconciliation_at: i64,
    pub inconsistent_chats: u64,
}

/// The key material a poll vote needs: who created the poll and the secret
/// its options were encrypted with. Rows written before this port stored
/// votes carry neither; voting on those answers that the key is missing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PollMeta {
    pub creator_jid: String,
    pub message_secret: Vec<u8>,
}

/// One pending placeholder resend the reaction repair replays.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LegacyReplayRow {
    pub chat_jid: String,
    pub transport_jid: String,
    pub event_message_id: String,
    pub sender_jid: String,
    pub timestamp: i64,
    pub from_me: bool,
    pub status: i64,
    pub request_id: String,
    pub requested_at: i64,
    pub attempts: u32,
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
