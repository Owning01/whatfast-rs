//! The linked-device session: the device store, the bot behind it, and the
//! state the bridge reports.
//!
//! One session at a time. It exists only while the store holds a linked
//! device or the user is pairing one: an empty store connects to nothing, so
//! an unpaired desktop stays offline until `StartPairing` builds the bot and
//! the QR loop starts (the library emits `PairingQrCode` events on its own
//! while unpaired).
//!
//! The connection state the bridge reports is never inferred from a button:
//! it tracks the library's own events (`Connected`, `Disconnected`,
//! `LoggedOut`, `ConnectFailure`), the same rule the Go sidecar followed.

use std::{
    path::Path,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use wacore::store::DevicePropsOverride;
use wacore::types::events::{Event, EventHandler};
use whatfast_protocol::{BackendEvent, BackendProblem, ConnectionState, PairingQr};
use whatsapp_rust::bot::{Bot, BotHandle};
use whatsapp_rust::client::Client;
use whatsapp_rust::store::SqliteStore;

use crate::db::Store;

/// The reported state, shared between the request handlers and the event
/// pump. Every field is written by exactly one side: the pump writes
/// connection and pairing, the handlers read them.
pub struct SessionState {
    connection: Mutex<ConnectionState>,
    paired: AtomicBool,
    own_user_id: Mutex<String>,
    /// The sync phase the desktop's status view reads, written only by the
    /// pump's history walks.
    sync: Mutex<whatfast_protocol::SyncStatus>,
}

impl SessionState {
    fn new(paired: bool, own_user_id: String) -> Self {
        Self {
            connection: Mutex::new(if paired {
                ConnectionState::Connecting
            } else {
                ConnectionState::Offline
            }),
            paired: AtomicBool::new(paired),
            own_user_id: Mutex::new(own_user_id),
            sync: Mutex::new(whatfast_protocol::SyncStatus {
                phase: whatfast_protocol::SyncPhase::NotStarted as i32,
                ..Default::default()
            }),
        }
    }

    /// A fresh sync-status answer.
    pub fn sync_status(&self) -> whatfast_protocol::SyncStatus {
        self.sync.lock().expect("sync lock").clone()
    }

    fn set_sync(&self, status: whatfast_protocol::SyncStatus) {
        *self.sync.lock().expect("sync lock") = status;
    }

    pub fn connection(&self) -> ConnectionState {
        *self.connection.lock().expect("connection lock")
    }

    pub fn own_user_id(&self) -> String {
        self.own_user_id.lock().expect("own id lock").clone()
    }

    pub fn is_paired(&self) -> bool {
        self.paired.load(Ordering::SeqCst)
    }

    pub(crate) fn set_connection(&self, state: ConnectionState) {
        *self.connection.lock().expect("connection lock") = state;
    }

    fn set_paired(&self, paired: bool) {
        self.paired.store(paired, Ordering::SeqCst);
    }
}

/// One running session: the client handle, the bot that owns the connection,
/// and the shared state.
pub struct Session {
    pub(crate) client: Arc<Client>,
    /// The call registry: one live call, the offers still ringing.
    pub calls: Arc<crate::calls::Calls>,
    /// The read-only window over the phone's identity data: the agenda
    /// names and the LID↔phone map of the fork's session database.
    pub identity: Arc<crate::identity::Identity>,
    /// Taken by [`Session::shutdown`]; `None` afterwards.
    handle: Mutex<Option<BotHandle>>,
    pub state: Arc<SessionState>,
}

impl Session {
    /// Opens the device store and reports whether a linked device lives in
    /// it, without touching the network. `Ok(false)` means: nothing to
    /// connect, wait for `StartPairing`.
    pub async fn has_linked_device(data_dir: &Path) -> anyhow::Result<bool> {
        let store = open_store(data_dir).await?;
        let linked = store
            .database()
            .list_devices()
            .await?
            .iter()
            .any(|device| device.linked);
        drop(store);
        Ok(linked)
    }

    /// Builds and starts the session over an existing (possibly empty)
    /// device store. The bot connects on its own; while unpaired the
    /// connection is the QR loop, whose codes leave through `events`.
    /// `store` is the chats.db the data events write into.
    pub async fn start(
        data_dir: &Path,
        store: Arc<Store>,
        events: async_channel::Sender<BackendEvent>,
    ) -> anyhow::Result<Arc<Self>> {
        let device_store = open_store(data_dir).await?;
        let paired = device_store
            .database()
            .list_devices()
            .await?
            .iter()
            .any(|device| device.linked);
        let client_slot = Arc::new(OnceLock::new());
        let calls = Arc::new(crate::calls::Calls::new());
        let identity = Arc::new(crate::identity::Identity::open(data_dir));
        let pump = EventPump {
            events,
            state: Arc::new(SessionState::new(paired, String::new())),
            store,
            identity: Arc::clone(&identity),
            client: Arc::clone(&client_slot),
            calls: Arc::clone(&calls),
        };
        let state = Arc::clone(&pump.state);
        // WhatsApp reads the linked-device name, version, and platform at
        // pairing; the same identity the Go sidecar presented.
        let bot = Bot::builder()
            .with_backend(device_store)
            .with_event_handler(pump)
            .with_device_props(
                DevicePropsOverride::new()
                    .with_os("Whatfast")
                    .with_version(app_version()),
            )
            .build()
            .await?;
        let client = bot.client();
        if let Some(pn) = client.pn() {
            *state.own_user_id.lock().expect("own id lock") = pn.to_string();
        }
        let _ = client_slot.set(Arc::clone(&client));
        let handle = bot.spawn();
        Ok(Arc::new(Self {
            client,
            calls,
            identity,
            handle: Mutex::new(Some(handle)),
            state,
        }))
    }

    /// A fresh auth-state answer from the live session.
    pub fn auth_state(&self) -> whatfast_protocol::AuthStateResponse {
        whatfast_protocol::AuthStateResponse {
            paired: self.state.is_paired(),
            logged_in: self.state.connection() == ConnectionState::Connected,
            own_user_id: self.state.own_user_id(),
            connection_state: self.state.connection() as i32,
        }
    }

    /// Asks the connection to come back. True when an attempt actually
    /// started, the Go contract for `Reconnect`.
    pub async fn reconnect(&self) -> bool {
        if self.state.connection() == ConnectionState::Connected {
            return false;
        }
        self.client.reconnect().await;
        true
    }

    /// Logs the device out: the server forgets it, and this session is over
    /// for the process. The bridge answers, the desktop decides what next.
    /// The report is only logged: the deregistration itself either happened
    /// or is retried by the library, and the desktop moves on either way.
    pub async fn logout(&self) {
        let report = self.client.logout().await;
        tracing::info!(
            "logout: deregistration={:?} shutdown={:?}",
            report.deregistration,
            report.shutdown
        );
        self.state.set_paired(false);
        self.state.set_connection(ConnectionState::LoggedOut);
    }

    /// Stops the bot gracefully; the store flushes under it. A second call
    /// is a no-op: the handle was already taken.
    pub async fn shutdown(self: Arc<Self>) {
        let handle = self.handle.lock().expect("handle lock").take();
        if let Some(handle) = handle {
            handle.shutdown().await;
        }
    }
}

/// The version the device reports to WhatsApp, as the five-part shape the
/// protocol expects. A version with fewer parts pads with zeros. The proto
/// structs are non-exhaustive, so the value starts from `Default` and is
/// assigned field by field.
fn app_version() -> whatsapp_rust::waproto::whatsapp::device_props::AppVersion {
    let mut parts = env!("CARGO_PKG_VERSION")
        .split('.')
        .map(|part| part.parse::<u32>().unwrap_or(0));
    let mut version =
        <whatsapp_rust::waproto::whatsapp::device_props::AppVersion as Default>::default();
    version.primary = Some(parts.next().unwrap_or(0));
    version.secondary = Some(parts.next().unwrap_or(0));
    version.tertiary = Some(parts.next().unwrap_or(0));
    version.quaternary = Some(parts.next().unwrap_or(0));
    version.quinary = Some(parts.next().unwrap_or(0));
    version
}

async fn open_store(data_dir: &Path) -> anyhow::Result<SqliteStore> {
    let path = data_dir.join("session.db");
    SqliteStore::open(path.to_str().context("session path is not utf-8")?)
        .await
        .context("opening the device store")
}

/// Forwards the library events the bridge cares about, writes the data ones
/// into the chats store through [`crate::ingest`], and keeps
/// [`SessionState`] truthful. Connection events move the state machine;
/// everything else is data.
struct EventPump {
    events: async_channel::Sender<BackendEvent>,
    state: Arc<SessionState>,
    store: Arc<Store>,
    /// The call registry, shared with the session: offers ring from here.
    calls: Arc<crate::calls::Calls>,
    /// The phone's identity data (agenda names, LID↔phone map), shared with
    /// the session: the canonical chat of every event resolves through it.
    identity: Arc<crate::identity::Identity>,
    /// Filled by [`Session::start`] the moment the client exists: encrypted
    /// reactions need the message secrets only the client's backend has.
    client: Arc<OnceLock<Arc<Client>>>,
}

impl EventPump {
    fn push(&self, event: BackendEvent) {
        if self.events.send_blocking(event).is_err() {
            // The serve loop is gone; the process is exiting. Dropping the
            // event is the right move, and quiet.
            tracing::debug!("event dropped: the bridge is closing");
        }
    }

    /// Runs the identity reconciliation off the event thread: it writes in
    /// bulk and the live events must not queue behind it.
    fn spawn_reconcile(&self) {
        let store = Arc::clone(&self.store);
        let identity = Arc::clone(&self.identity);
        let events = self.events.clone();
        tokio::spawn(async move {
            crate::reconcile::reconcile(&store, &identity, &events).await;
        });
    }

    /// Writes a history-sync chunk into the store and announces it. The
    /// initial sync arrives in compressed chunks; each chunk is one pass.
    ///
    /// The Go's completeness rule is the oracle: only `FULL` and `RECENT`
    /// chunks that reach 100% can finish the initial sync, and a chunk that
    /// could not be decoded or stored fails it. The counters are the Go's:
    /// received, decoded and failed message totals beside the processed ones.
    fn ingest_history(&self, lazy: &wacore::types::events::LazyHistorySync) {
        let sync_type = lazy.sync_type();
        let on_demand = lazy.peer_data_request_session_id().is_some();
        // HistorySyncType: INITIAL_BOOTSTRAP = 0, FULL = 2, RECENT = 3.
        let contributes = matches!(sync_type, 0 | 2 | 3);
        let terminal = matches!(sync_type, 2 | 3);
        let progress = lazy.progress().unwrap_or(0);
        let Some(parsed) = lazy.get() else {
            tracing::warn!("a history chunk failed to decode");
            if contributes {
                self.record_history(
                    HistoryCounters {
                        progress,
                        terminal,
                        ..HistoryCounters::default()
                    },
                    true,
                );
            }
            return;
        };
        crate::ingest::learn_history_contacts(&self.store, parsed);
        let mut counters = HistoryCounters {
            progress,
            terminal,
            ..HistoryCounters::default()
        };
        for conversation in &parsed.conversations {
            for entry in &conversation.messages {
                counters.received += 1;
                // What the Go counted as decoded is what carries a body; the
                // rest failed its parse and must fail the chunk.
                let decodable = entry
                    .message
                    .as_option()
                    .is_some_and(|web| web.message.as_option().is_some());
                if decodable {
                    counters.decoded += 1;
                } else {
                    counters.failed += 1;
                }
            }
        }
        let conversations = crate::ingest::history_conversations(parsed);
        for conversation in &conversations {
            match crate::ingest::store_conversation(&self.store, &self.identity, conversation) {
                Ok((event, stored)) => {
                    counters.chats += 1;
                    counters.stored += stored;
                    self.push(event);
                }
                Err(error) => {
                    counters.failed += conversation.messages.len() as u64;
                    tracing::warn!("a history conversation failed to store: {error}");
                }
            }
        }
        let complete = !on_demand
            && conversations
                .iter()
                .any(|conversation| conversation.end_of_history_transfer);
        if complete {
            // The history is where the twins are born: once it lands, the
            // reconciliation folds whatever it split.
            self.spawn_reconcile();
        }
        if on_demand {
            tracing::info!(
                "on-demand history: chats={} messages={}",
                counters.chats,
                counters.stored
            );
            return;
        }
        if contributes {
            self.record_history(counters.clone(), counters.failed > 0);
        }
        tracing::info!(
            "history chunk: chats={} messages={}",
            counters.chats,
            counters.stored
        );
        self.push(whatfast_protocol::BackendEvent {
            sequence: 0,
            event: Some(whatfast_protocol::backend_event::Event::SyncProgress(
                whatfast_protocol::SyncProgress {
                    chats_processed: counters.chats,
                    messages_processed: counters.stored,
                    // The Go's terminal rule: only a FULL/RECENT chunk at
                    // 100% without a failure completes the sync.
                    complete: counters.terminal && counters.progress >= 100 && counters.failed == 0,
                },
            )),
        });
    }

    /// Applies one contributing chunk to the reported sync status and
    /// announces the change, the way the Go `updateSyncStatus` did.
    fn record_history(&self, counters: HistoryCounters, failed_chunk: bool) {
        let mut status = self.state.sync_status();
        if status.started_at_ms == 0 {
            status.started_at_ms = unix_ms();
        }
        status.phase = whatfast_protocol::SyncPhase::InitialHistory as i32;
        status.detail = "Receiving WhatsApp history".into();
        status.chats_processed += counters.chats;
        status.messages_processed += counters.stored;
        status.messages_received += counters.received;
        status.messages_decoded += counters.decoded;
        status.messages_failed += counters.failed;
        if counters.progress > status.whatsapp_progress {
            status.whatsapp_progress = counters.progress;
        }
        if failed_chunk {
            status.phase = whatfast_protocol::SyncPhase::Failed as i32;
            status.detail = format!(
                "Some chat history could not be decoded or saved locally ({} of {} messages failed)",
                counters.failed, counters.received
            );
            status.completed_at_ms = 0;
        } else if counters.terminal && counters.progress >= 100 {
            status.phase = whatfast_protocol::SyncPhase::Complete as i32;
            status.detail = "Up to date".into();
            status.completed_at_ms = unix_ms();
        }
        self.state.set_sync(status.clone());
        self.push(whatfast_protocol::BackendEvent {
            sequence: 0,
            event: Some(whatfast_protocol::backend_event::Event::SyncStatusChanged(
                whatfast_protocol::SyncStatusChanged {
                    status: Some(status),
                },
            )),
        });
    }

    /// Records one favourite sticker the phone synced, the Go
    /// `reduceFavoriteSticker`: the id is the hex of the encrypted file hash,
    /// because WhatsApp never syncs a favourite's plaintext one.
    fn reduce_favorite_sticker(&self, update: &wacore::types::events::FavoriteStickerUpdate) {
        let action = &update.action;
        let encrypted = action.file_enc_sha256.clone().unwrap_or_default();
        if encrypted.is_empty() {
            return;
        }
        let id = crate::db::hex_lower(&encrypted);
        let changed = if action.is_favorite.unwrap_or(false) {
            let favorite = crate::db::FavoriteStickerRow {
                id,
                mime: action.mimetype.clone().unwrap_or_default(),
                direct_path: action.direct_path.clone().unwrap_or_default(),
                media_key: action.media_key.clone().unwrap_or_default(),
                file_enc_sha256: encrypted,
                width: action.width.map(i64::from).unwrap_or(0),
                height: action.height.map(i64::from).unwrap_or(0),
                animated: action.is_lottie.unwrap_or(false),
                file_size: action.file_length.map(|size| size as i64).unwrap_or(0),
                local_path: String::new(),
                updated_at: update.timestamp.timestamp_millis(),
            };
            match self.store.upsert_favorite_sticker(&favorite) {
                Ok(()) => true,
                Err(error) => {
                    tracing::error!("record favourite sticker: {error}");
                    false
                }
            }
        } else {
            match self.store.remove_favorite_sticker(&id) {
                Ok(changed) => changed,
                Err(error) => {
                    tracing::error!("remove favourite sticker: {error}");
                    false
                }
            }
        };
        if changed {
            self.push(whatfast_protocol::BackendEvent {
                sequence: 0,
                event: Some(whatfast_protocol::backend_event::Event::StickersChanged(
                    whatfast_protocol::StickersChanged {},
                )),
            });
        }
    }
}

/// One history chunk's message counters, the Go `historyReductionStats` plus
/// the chunk's own progress and sync type.
#[derive(Clone, Default)]
struct HistoryCounters {
    chats: u64,
    stored: u64,
    received: u64,
    decoded: u64,
    failed: u64,
    progress: u32,
    terminal: bool,
}

impl EventHandler for EventPump {
    fn handle_event(&self, event: Arc<Event>) {
        match event.as_ref() {
            // ---- connection events (session state) ----
            Event::PairingQrCode(qr) => {
                self.state.set_connection(ConnectionState::Pairing);
                self.push(BackendEvent {
                    sequence: 0,
                    event: Some(whatfast_protocol::backend_event::Event::PairingQr(
                        PairingQr {
                            code: qr.code.clone(),
                            expires_at_ms: unix_ms() + qr.timeout.as_millis() as i64,
                        },
                    )),
                });
            }
            Event::Connected(_) => {
                self.state.set_paired(true);
                self.state.set_connection(ConnectionState::Connected);
                self.push(connection_changed(
                    ConnectionState::Connected,
                    String::new(),
                ));
                if let Some(client) = self.client.get().cloned() {
                    crate::meta::spawn_group_backfill(
                        Arc::clone(&client),
                        Arc::clone(&self.store),
                        self.events.clone(),
                    );
                }
                // The twins the earlier sessions left behind and the agenda
                // names both settle here, before the history re-lands them.
                self.spawn_reconcile();
            }
            Event::Disconnected(reason) => {
                let detail = format!("{reason:?}");
                self.state.set_connection(ConnectionState::Offline);
                self.push(connection_changed(ConnectionState::Offline, detail));
            }
            Event::ConnectFailure(failure) => {
                let detail = failure.message.clone().unwrap_or_default();
                self.state.set_connection(ConnectionState::Failed);
                self.push(connection_changed(ConnectionState::Failed, detail));
            }
            Event::LoggedOut(logged_out) => {
                self.state.set_paired(false);
                self.state.set_connection(ConnectionState::LoggedOut);
                self.push(connection_changed(
                    ConnectionState::LoggedOut,
                    format!("{:?}", logged_out.reason),
                ));
            }
            Event::PairingQrCodesExhausted(exhausted) => {
                self.state.set_connection(ConnectionState::Offline);
                self.push(BackendEvent {
                    sequence: 0,
                    event: Some(whatfast_protocol::backend_event::Event::Problem(
                        BackendProblem {
                            code: "PAIRING_QR_EXHAUSTED".into(),
                            message: "every pairing code was used up; retry the pairing".into(),
                            fatal: exhausted.disconnected,
                        },
                    )),
                });
            }
            Event::QrScannedWithoutMultidevice(_) => {
                self.push(BackendEvent {
                    sequence: 0,
                    event: Some(whatfast_protocol::backend_event::Event::Problem(
                        BackendProblem {
                            code: "PAIRING_NO_MULTIDEVICE".into(),
                            message: "the phone scanned the code but multi-device is off".into(),
                            fatal: false,
                        },
                    )),
                });
            }
            // ---- data events (the store) ----
            Event::HistorySync(lazy) => self.ingest_history(lazy),
            Event::FavoriteStickerUpdate(update) => self.reduce_favorite_sticker(update),
            Event::IncomingCall(incoming) => {
                let offer = self.calls.offer(*incoming.clone());
                self.push(offer);
            }
            Event::Messages(batch) => {
                crate::ingest::spawn_enc_reactions(
                    Arc::clone(&self.store),
                    Arc::clone(&self.client),
                    Arc::clone(&self.identity),
                    self.events.clone(),
                    batch,
                );
                let (events, _ingested) =
                    crate::ingest::apply_event(&self.store, &self.identity, &event);
                for backend_event in events {
                    self.push(backend_event);
                }
            }
            Event::ContactUpdate(update) => {
                // The phone renamed or saved a contact: the agenda copy in
                // the fork's database is authoritative, so the names refresh
                // and the chats that changed announce themselves.
                let jid = update.jid.to_non_ad_string();
                tracing::debug!("contact update for {jid}");
                self.spawn_reconcile();
            }
            other => {
                let (events, _ingested) =
                    crate::ingest::apply_event(&self.store, &self.identity, other);
                for backend_event in events {
                    self.push(backend_event);
                }
                if !matches!(other, Event::Receipt(_) | Event::ChatPresence(_)) {
                    tracing::debug!("event not carried by the bridge yet: {other:?}");
                }
            }
        }
    }
}

fn connection_changed(state: ConnectionState, detail: String) -> BackendEvent {
    BackendEvent {
        sequence: 0,
        event: Some(whatfast_protocol::backend_event::Event::ConnectionChanged(
            whatfast_protocol::ConnectionChanged {
                state: state as i32,
                detail,
            },
        )),
    }
}

fn unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as i64
}

/// The `context` combinator used when opening the device store.
use anyhow::Context as _;

#[cfg(test)]
mod tests {
    use super::{EventPump, HistoryCounters, SessionState};
    use std::sync::{Arc, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    /// One pump over a throwaway profile: no bot, no network, the store and
    /// the event channel only.
    fn pump(label: &str) -> EventPump {
        let dir = std::env::temp_dir().join(format!(
            "whatfast-wa-sync-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Arc::new(crate::db::Store::open(&dir.join("chats.db")).unwrap());
        let (events, _receiver) = async_channel::bounded(32);
        EventPump {
            events,
            state: Arc::new(SessionState::new(true, String::new())),
            store,
            identity: Arc::new(crate::identity::Identity::open(&dir)),
            calls: Arc::new(crate::calls::Calls::new()),
            client: Arc::new(OnceLock::new()),
        }
    }

    /// The Go completes the initial sync only on a FULL/RECENT chunk that
    /// reached 100% without a failure.
    #[test]
    fn a_terminal_chunk_at_full_progress_completes_the_initial_sync() {
        let pump = pump("complete");
        pump.record_history(
            HistoryCounters {
                chats: 3,
                stored: 40,
                received: 42,
                decoded: 42,
                progress: 100,
                terminal: true,
                ..HistoryCounters::default()
            },
            false,
        );
        let status = pump.state.sync_status();
        assert_eq!(
            status.phase,
            whatfast_protocol::SyncPhase::Complete as i32,
            "phase {}",
            status.phase
        );
        assert!(status.completed_at_ms > 0);
        assert_eq!(status.messages_received, 42);
        assert_eq!(status.messages_processed, 40);
        assert_eq!(status.whatsapp_progress, 100);
    }

    /// A non-terminal chunk (the server-pushed bootstrap) never finishes the
    /// sync, whatever its progress says.
    #[test]
    fn a_bootstrap_chunk_stays_in_the_initial_phase() {
        let pump = pump("bootstrap");
        pump.record_history(
            HistoryCounters {
                chats: 1,
                stored: 2,
                received: 2,
                decoded: 2,
                progress: 100,
                terminal: false,
                ..HistoryCounters::default()
            },
            false,
        );
        let status = pump.state.sync_status();
        assert_eq!(
            status.phase,
            whatfast_protocol::SyncPhase::InitialHistory as i32
        );
        assert_eq!(status.completed_at_ms, 0);
    }

    /// A chunk that could not be stored fails the phase and keeps the failure
    /// counters the desktop shows.
    #[test]
    fn a_failed_chunk_reports_the_failed_message_count() {
        let pump = pump("failed");
        pump.record_history(
            HistoryCounters {
                chats: 1,
                stored: 0,
                received: 10,
                decoded: 8,
                failed: 2,
                progress: 100,
                terminal: true,
            },
            true,
        );
        let status = pump.state.sync_status();
        assert_eq!(status.phase, whatfast_protocol::SyncPhase::Failed as i32);
        assert_eq!(status.completed_at_ms, 0);
        assert_eq!(status.messages_failed, 2);
        assert!(
            status.detail.contains("(2 of 10 messages failed)"),
            "{}",
            status.detail
        );
    }
}
