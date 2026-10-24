//! The sidecar's own database: chats, messages, and every companion table
//! the bridge contract needs.
//!
//! The schema is the one the Go sidecar shipped (same tables, same columns,
//! same indexes), so the reconciliation and repair logic ported wave by wave
//! reads the same shapes the Go wrote. Timestamps are Unix milliseconds
//! everywhere. One connection, one writer: the bridge serves one desktop,
//! and SQLite in WAL mode serializes the rest.

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
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(SCHEMA)?;
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
    pub fn ensure_chat(&self, jid: &str, preferred_jid: &str) -> rusqlite::Result<()> {
        self.write(|conn| {
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
        })
    }

    /// One chat row with its derived `last_message_from_me` and the address
    /// alias, or `None` when the jid is unknown.
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
    pub fn chats_page(
        &self,
        cursor_at: i64,
        cursor_jid: &str,
        limit: u32,
    ) -> rusqlite::Result<(Vec<ChatRow>, u64, String)> {
        self.read(|conn| {
            let total: i64 = conn.query_row("SELECT COUNT(*) FROM chats", [], |row| row.get(0))?;
            let mut statement = conn.prepare(&format!(
                "{CHAT_SELECT} WHERE (?1 = 0 AND ?2 = '')
                    OR (c.last_message_at < ?1)
                    OR (c.last_message_at = ?1 AND c.jid < ?2)
                 ORDER BY c.last_message_at DESC, c.jid DESC LIMIT ?3"
            ))?;
            let rows = statement
                .query_map(params![cursor_at, cursor_jid, limit], chat_from_row)?
                .collect::<Result<Vec<_>, _>>()?;
            let next_cursor = rows
                .last()
                .map(|chat| format!("{}:{}", chat.last_message_at, chat.jid))
                .unwrap_or_default();
            Ok((rows, total as u64, next_cursor))
        })
    }

    /// Sets the chat display flags the desktop owns (mute/archive/pin/block
    /// live in this profile: the linked-device transport does not expose
    /// app-state for them).
    pub fn set_chat_flag(&self, jid: &str, column: ChatFlag, value: bool) -> rusqlite::Result<()> {
        self.write(|conn| {
            conn.execute(
                &format!("UPDATE chats SET {}=?1 WHERE jid=?2", column.as_str()),
                params![value as i64, jid],
            )?;
            Ok(())
        })
    }

    pub fn set_chat_mute(&self, jid: &str, mute_until: i64) -> rusqlite::Result<()> {
        self.write(|conn| {
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

    /// Stores one message and refreshes the owning chat's last-message
    /// pointer. Returns true when the row is new (a replay is a no-op for
    /// the unread counter).
    pub fn insert_message(&self, message: &MessageRow) -> rusqlite::Result<bool> {
        let inserted = self.write(|conn| {
            let changed = conn.execute(
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
                    message.text, message.timestamp, message.from_me as i64, message.status,
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
            )?;
            Ok(changed == 1)
        })?;
        if inserted {
            self.write(|conn| {
                // The pointer is recomputed from the actual newest row, the
                // way the Go sidecar did: a late-arriving older message must
                // not drag the conversation's preview backwards.
                conn.execute(
                    "UPDATE chats SET
                       last_message_id=(SELECT id FROM messages WHERE chat_jid=?1
                                        ORDER BY timestamp DESC, id DESC LIMIT 1),
                       last_message_text=(SELECT CASE
                                            WHEN text<>'' THEN text
                                            WHEN image_caption<>'' THEN image_caption
                                            WHEN media_caption<>'' THEN media_caption
                                            ELSE ''
                                          END FROM messages WHERE chat_jid=?1
                                          ORDER BY timestamp DESC, id DESC LIMIT 1),
                       last_message_at=COALESCE((SELECT timestamp FROM messages WHERE chat_jid=?1
                                                 ORDER BY timestamp DESC, id DESC LIMIT 1),0)
                     WHERE jid=?1",
                    params![message.chat_jid],
                )?;
                Ok(())
            })?;
        }
        Ok(inserted)
    }

    /// The unread counters: `messages.unread` marks the rows, `chats.unread_count`
    /// caches the total the list shows.
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
        self.read(|conn| {
            let mut statement = conn.prepare(&format!(
                "{MESSAGE_SELECT}
                 WHERE m.chat_jid=?1
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

    /// Ascending page strictly after the cursor.
    pub fn messages_after(
        &self,
        chat_jid: &str,
        after_ts: i64,
        after_id: &str,
        limit: u32,
    ) -> rusqlite::Result<(Vec<MessageRow>, bool)> {
        self.read(|conn| {
            let mut statement = conn.prepare(&format!(
                "{MESSAGE_SELECT}
                 WHERE m.chat_jid=?1
                   AND (m.timestamp > ?2 OR (m.timestamp = ?2 AND m.id > ?3))
                 ORDER BY m.timestamp ASC, m.id ASC LIMIT ?4"
            ))?;
            let mut rows = statement
                .query_map(
                    params![chat_jid, after_ts, after_id, limit],
                    message_from_row,
                )?
                .collect::<Result<Vec<_>, _>>()?;
            attach_reactions(conn, &mut rows)?;
            let has_more = rows.len() as u32 == limit;
            Ok((rows, has_more))
        })
    }

    /// A bounded window around one message: half the page older, half newer.
    pub fn messages_around(
        &self,
        chat_jid: &str,
        message_id: &str,
        limit: u32,
    ) -> rusqlite::Result<Option<(Vec<MessageRow>, bool, bool)>> {
        let anchor = self.read(|conn| {
            conn.query_row(
                "SELECT timestamp FROM messages WHERE chat_jid=?1 AND id=?2",
                params![chat_jid, message_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
        })?;
        let Some(anchor_ts) = anchor else {
            return Ok(None);
        };
        let half = (limit / 2).max(1) as i64;
        self.read(|conn| {
            let mut older = conn.prepare(&format!(
                "{MESSAGE_SELECT}
                 WHERE m.chat_jid=?1 AND (m.timestamp < ?2 OR (m.timestamp = ?2 AND m.id < ?3))
                 ORDER BY m.timestamp DESC, m.id DESC LIMIT ?4"
            ))?;
            let mut older_rows = older
                .query_map(
                    params![chat_jid, anchor_ts, message_id, half],
                    message_from_row,
                )?
                .collect::<Result<Vec<_>, _>>()?;
            let mut newer = conn.prepare(&format!(
                "{MESSAGE_SELECT}
                 WHERE m.chat_jid=?1 AND (m.timestamp > ?2 OR (m.timestamp = ?2 AND m.id >= ?3))
                 ORDER BY m.timestamp ASC, m.id ASC LIMIT ?4"
            ))?;
            let rows = newer
                .query_map(
                    params![chat_jid, anchor_ts, message_id, half + 1],
                    message_from_row,
                )?
                .collect::<Result<Vec<_>, _>>()?;
            let mut all = {
                older_rows.reverse();
                older_rows.extend(rows);
                older_rows
            };
            attach_reactions(conn, &mut all)?;
            let has_older = {
                let first = all.first().map(|row| (row.timestamp, row.id.clone()));
                match first {
                    Some((ts, id)) => conn
                        .query_row(
                            "SELECT 1 FROM messages WHERE chat_jid=?1
                             AND (timestamp < ?2 OR (timestamp = ?2 AND id < ?3)) LIMIT 1",
                            params![chat_jid, ts, id],
                            |_| Ok(()),
                        )
                        .optional()?
                        .is_some(),
                    None => false,
                }
            };
            let has_newer = {
                let last = all.last().map(|row| (row.timestamp, row.id.clone()));
                match last {
                    Some((ts, id)) => conn
                        .query_row(
                            "SELECT 1 FROM messages WHERE chat_jid=?1
                             AND (timestamp > ?2 OR (timestamp = ?2 AND id > ?3)) LIMIT 1",
                            params![chat_jid, ts, id],
                            |_| Ok(()),
                        )
                        .optional()?
                        .is_some(),
                    None => false,
                }
            };
            Ok(Some((all, has_older, has_newer)))
        })
    }

    /// One stored message with its reactions, or none.
    pub fn message(
        &self,
        chat_jid: &str,
        message_id: &str,
    ) -> rusqlite::Result<Option<MessageRow>> {
        let message = self.read(|conn| {
            conn.query_row(
                &format!("{MESSAGE_SELECT} WHERE m.chat_jid=?1 AND m.id=?2"),
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

    /// Marks one message edited or revoked, the way an incoming edit or
    /// revoke event says.
    pub fn mark_message_edited(
        &self,
        chat_jid: &str,
        message_id: &str,
        text: &str,
    ) -> rusqlite::Result<()> {
        self.write(|conn| {
            conn.execute(
                "UPDATE messages SET text=?1, edited_at=?2 WHERE chat_jid=?3 AND id=?4",
                params![text, now_ms(), chat_jid, message_id],
            )?;
            Ok(())
        })
    }

    pub fn mark_message_revoked(&self, chat_jid: &str, message_id: &str) -> rusqlite::Result<()> {
        self.write(|conn| {
            conn.execute(
                "UPDATE messages SET revoked=1, kind='revoked' WHERE chat_jid=?1 AND id=?2",
                params![chat_jid, message_id],
            )?;
            Ok(())
        })
    }

    /// Points one message's attachment (a video, an audio or a document) at
    /// the cache file the download wrote.
    pub fn set_media_local_path(
        &self,
        chat_jid: &str,
        message_id: &str,
        path: &str,
    ) -> rusqlite::Result<()> {
        self.write(|conn| {
            conn.execute(
                "UPDATE messages SET media_local_path=?1 WHERE chat_jid=?2 AND id=?3",
                params![path, chat_jid, message_id],
            )?;
            Ok(())
        })
    }

    /// Points one message's image or sticker at the cache file the download
    /// wrote: the full-resolution entry the image viewer opens.
    pub fn set_image_local_path(
        &self,
        chat_jid: &str,
        message_id: &str,
        path: &str,
    ) -> rusqlite::Result<()> {
        self.write(|conn| {
            conn.execute(
                "UPDATE messages SET image_local_path=?1 WHERE chat_jid=?2 AND id=?3",
                params![path, chat_jid, message_id],
            )?;
            Ok(())
        })
    }

    /// Advances one message's delivery status. Never goes backwards except
    /// into FAILED, which may recover later.
    pub fn set_message_status(
        &self,
        chat_jid: &str,
        message_id: &str,
        status: i64,
    ) -> rusqlite::Result<bool> {
        self.write(|conn| {
            let changed = conn.execute(
                "UPDATE messages SET status=?1
                 WHERE chat_jid=?2 AND id=?3 AND (status<?1 OR ?1=5)",
                params![status, chat_jid, message_id],
            )?;
            Ok(changed > 0)
        })
    }

    // ------------------------------------------------------------------
    // Reactions
    // ------------------------------------------------------------------

    /// Upserts one reaction (empty emoji removes the sender's).
    pub fn set_reaction(&self, reaction: &ReactionRow) -> rusqlite::Result<()> {
        self.write(|conn| {
            if reaction.emoji.is_empty() {
                conn.execute(
                    "DELETE FROM reactions WHERE chat_jid=?1 AND message_id=?2 AND sender_jid=?3",
                    params![reaction.chat_jid, reaction.message_id, reaction.sender_jid],
                )?;
            } else {
                conn.execute(
                    "INSERT INTO reactions(chat_jid,message_id,sender_jid,emoji,timestamp,from_me)
                     VALUES(?1,?2,?3,?4,?5,?6)
                     ON CONFLICT(chat_jid,message_id,sender_jid)
                     DO UPDATE SET emoji=excluded.emoji,timestamp=excluded.timestamp,from_me=excluded.from_me",
                    params![
                        reaction.chat_jid,
                        reaction.message_id,
                        reaction.sender_jid,
                        reaction.emoji,
                        reaction.timestamp,
                        reaction.from_me as i64
                    ],
                )?;
            }
            Ok(())
        })
    }

    // ------------------------------------------------------------------
    // Search
    // ------------------------------------------------------------------

    /// The group conversations whose name matches, ranked the Go way:
    /// score first, then recency, then jid.
    pub fn search_groups(&self, query: &str, limit: u32) -> rusqlite::Result<Vec<ChatRow>> {
        let needle = query.to_lowercase();
        let (rows, _total, _cursor) = self.chats_page(0, "", 500)?;
        let mut ranked: Vec<(i64, ChatRow)> = rows
            .into_iter()
            .filter(|chat| chat.preferred_jid.ends_with("@g.us"))
            .filter_map(|chat| {
                let haystack = chat.name.to_lowercase();
                if haystack.is_empty() {
                    return None;
                }
                let score = if haystack == needle {
                    300
                } else if haystack.starts_with(&needle) {
                    200
                } else if haystack.contains(&needle) {
                    100
                } else {
                    return None;
                };
                Some((score, chat))
            })
            .collect();
        ranked.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then(b.1.last_message_at.cmp(&a.1.last_message_at))
                .then(a.1.jid.cmp(&b.1.jid))
        });
        ranked.truncate(limit as usize);
        Ok(ranked.into_iter().map(|(_, chat)| chat).collect())
    }

    /// The messages whose text matches, newest first. At least three runes
    /// of query, like the Go: shorter needles scan the table for noise.
    pub fn search_messages(
        &self,
        query: &str,
        limit: u32,
    ) -> rusqlite::Result<Vec<(ChatRow, MessageRow)>> {
        if query.chars().count() < 3 {
            return Ok(Vec::new());
        }
        let needle = format!("%{}%", query.to_lowercase().replace('%', ""));
        self.read(|conn| {
            let mut chat_statement = conn.prepare(&format!("{CHAT_SELECT} WHERE c.jid=?1"))?;
            let mut statement = conn.prepare(
                "SELECT chat_jid,id FROM messages
                 WHERE LOWER(text) LIKE ?1 AND kind NOT IN ('reaction','revoked')
                 ORDER BY timestamp DESC, id DESC LIMIT ?2",
            )?;
            let hits = statement
                .query_map(params![needle, limit], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            let mut out = Vec::with_capacity(hits.len());
            for (chat_jid, message_id) in hits {
                let chat = chat_statement
                    .query_row(params![chat_jid], chat_from_row)
                    .optional()?;
                let Some(chat) = chat else { continue };
                let message = conn.query_row(
                    &format!("{MESSAGE_SELECT} WHERE m.chat_jid=?1 AND m.id=?2"),
                    params![chat_jid, message_id],
                    message_from_row,
                )?;
                out.push((chat, message));
            }
            Ok(out)
        })
    }

    /// Learns (or refreshes) one contact's presentation fields. Empty values
    /// never erase known ones, the Go's rule.
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

    /// Names a conversation (the group-subject backfill).
    pub fn set_chat_name(&self, jid: &str, name: &str) -> rusqlite::Result<()> {
        self.write(|conn| {
            conn.execute("UPDATE chats SET name=?1 WHERE jid=?2", params![name, jid])?;
            Ok(())
        })
    }

    /// The canonical chat of one jid: the redirect table first, then the
    /// address table, then the jid itself.
    pub fn resolve_chat_id(&self, jid: &str) -> rusqlite::Result<String> {
        self.read(|conn| {
            let redirected: Option<String> = conn
                .query_row(
                    "SELECT new_chat_jid FROM chat_redirects WHERE old_chat_jid=?1",
                    [jid],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(redirected) = redirected {
                return Ok(redirected);
            }
            conn.query_row(
                "SELECT chat_jid FROM chat_addresses WHERE jid=?1",
                [jid],
                |row| row.get(0),
            )
            .optional()
            .map(|address| address.unwrap_or_else(|| jid.to_owned()))
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
    /// journal tables it also carried are not ported.
    pub fn merge_chats(&self, winner: &str, loser: &str) -> rusqlite::Result<()> {
        self.write(|conn| {
            // The display flags of the loser survive only when the winner
            // never set them; the pointer is recomputed at the end.
            conn.execute(
                "UPDATE chats SET
                   name=CASE WHEN name='' THEN (SELECT name FROM chats WHERE jid=?2) ELSE name END,
                   muted_until=max(muted_until,(SELECT muted_until FROM chats WHERE jid=?2)),
                   archived=max(archived,(SELECT archived FROM chats WHERE jid=?2)),
                   pinned=max(pinned,(SELECT pinned FROM chats WHERE jid=?2)),
                   blocked=max(blocked,(SELECT blocked FROM chats WHERE jid=?2))
                 WHERE jid=?1",
                params![winner, loser],
            )?;
            for table in [
                "reactions",
                "polls",
                "poll_votes",
                "pinned_messages",
                "outgoing_requests",
            ] {
                conn.execute(
                    &format!("UPDATE OR IGNORE {table} SET chat_jid=?1 WHERE chat_jid=?2"),
                    params![winner, loser],
                )?;
                conn.execute(
                    &format!("DELETE FROM {table} WHERE chat_jid=?1"),
                    params![loser],
                )?;
            }
            conn.execute(
                "UPDATE OR IGNORE messages SET chat_jid=?1 WHERE chat_jid=?2",
                params![winner, loser],
            )?;
            conn.execute("DELETE FROM messages WHERE chat_jid=?1", params![loser])?;
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
            conn.execute("DELETE FROM chats WHERE jid=?1", params![loser])?;
            Ok(())
        })?;
        self.refresh_chat_pointer(winner)
    }

    /// Recomputes the last-message pointer of one chat from its rows.
    pub fn refresh_chat_pointer(&self, jid: &str) -> rusqlite::Result<()> {
        self.write(|conn| {
            conn.execute(
                "UPDATE chats SET
                   last_message_id=COALESCE((SELECT id FROM messages WHERE chat_jid=?1 ORDER BY timestamp DESC,id DESC LIMIT 1),''),
                   last_message_text=COALESCE((SELECT text FROM messages WHERE chat_jid=?1 ORDER BY timestamp DESC,id DESC LIMIT 1),''),
                   last_message_at=COALESCE((SELECT timestamp FROM messages WHERE chat_jid=?1 ORDER BY timestamp DESC,id DESC LIMIT 1),last_message_at),
                   last_message_from_me=COALESCE((SELECT from_me FROM messages WHERE chat_jid=?1 ORDER BY timestamp DESC,id DESC LIMIT 1),last_message_from_me)
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

    /// Records the poll content of a message the desktop can vote on.
    pub fn upsert_poll(
        &self,
        chat_jid: &str,
        message_id: &str,
        question: &str,
        options: &[String],
        selectable_count: u32,
    ) -> rusqlite::Result<()> {
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
                "INSERT OR IGNORE INTO polls(chat_jid,message_id,question,selectable_count,options_json)
                 VALUES(?1,?2,?3,?4,?5)",
                rusqlite::params![chat_jid, message_id, question, selectable_count, options_json],
            )?;
            Ok(())
        })
    }

    /// The most recent sticker row matching a sticker id (the plaintext
    /// file hash, hex): the (chat, message) pair that can resend it.
    pub fn message_by_sticker_id(
        &self,
        sticker_id: &str,
    ) -> rusqlite::Result<Option<(String, String)>> {
        self.read(|conn| {
            conn.query_row(
                "SELECT chat_jid,id FROM messages
                 WHERE kind='sticker' AND LOWER(HEX(image_file_sha256))=LOWER(?1)
                 ORDER BY timestamp DESC LIMIT 1",
                params![sticker_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
        })
    }

    /// The subscribed newsletters, oldest subscription first.
    pub fn list_newsletters(&self) -> rusqlite::Result<Vec<whatfast_protocol::Newsletter>> {
        self.read(|conn| {
            let mut statement = conn.prepare(
                "SELECT jid,name,subscribed_at,muted FROM newsletters ORDER BY subscribed_at ASC",
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

    /// Records one newsletter subscription (or a rename).
    pub fn upsert_newsletter(&self, jid: &str, name: &str, muted: bool) -> rusqlite::Result<()> {
        self.write(|conn| {
            conn.execute(
                "INSERT INTO newsletters(jid,name,subscribed_at,muted) VALUES(?1,?2,?3,?4)
                 ON CONFLICT(jid) DO UPDATE SET name=excluded.name,muted=excluded.muted",
                rusqlite::params![jid, name, now_ms(), muted as i64],
            )?;
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

/// Joins the reactions of every row in place, one query for the batch.
fn attach_reactions(conn: &Connection, rows: &mut [MessageRow]) -> rusqlite::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    // The page is one chat by construction in every caller.
    let chat_jid = rows[0].chat_jid.clone();
    let mut statement = conn.prepare(
        "SELECT message_id,sender_jid,emoji,timestamp,from_me
         FROM reactions WHERE chat_jid=?1 ORDER BY timestamp ASC, sender_jid ASC",
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

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
