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
        let pump = EventPump {
            events,
            state: Arc::new(SessionState::new(paired, String::new())),
            store,
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

    /// Writes a history-sync chunk into the store and announces it. The
    /// initial sync arrives in compressed chunks; each chunk is one pass.
    fn ingest_history(&self, lazy: &wacore::types::events::LazyHistorySync) {
        let Some(parsed) = lazy.get() else {
            tracing::warn!("a history chunk failed to decode");
            return;
        };
        crate::ingest::learn_history_contacts(&self.store, parsed);
        let on_demand = lazy.peer_data_request_session_id().is_some();
        if !on_demand {
            let mut status = self.state.sync_status();
            status.phase = whatfast_protocol::SyncPhase::InitialHistory as i32;
            status.revision += 1;
            self.state.set_sync(status.clone());
            let _ = status;
        }
        let conversations = crate::ingest::history_conversations(parsed);
        let mut chats: u64 = 0;
        let mut messages: u64 = 0;
        for conversation in &conversations {
            match crate::ingest::store_conversation(&self.store, conversation) {
                Ok((event, stored)) => {
                    chats += 1;
                    messages += stored;
                    self.push(event);
                }
                Err(error) => {
                    tracing::warn!("a history conversation failed to store: {error}");
                }
            }
        }
        if on_demand {
            tracing::info!("on-demand history: chats={chats} messages={messages}");
        } else {
            tracing::info!("history chunk: chats={chats} messages={messages}");
            {
                let complete = conversations
                    .iter()
                    .any(|conversation| conversation.end_of_history_transfer);
                let mut status = self.state.sync_status();
                status.chats_processed += chats;
                status.messages_processed += messages;
                status.phase = if complete {
                    whatfast_protocol::SyncPhase::Complete as i32
                } else {
                    whatfast_protocol::SyncPhase::InitialHistory as i32
                };
                self.state.set_sync(status);
            }
            self.push(whatfast_protocol::BackendEvent {
                sequence: 0,
                event: Some(whatfast_protocol::backend_event::Event::SyncProgress(
                    whatfast_protocol::SyncProgress {
                        chats_processed: chats,
                        messages_processed: messages,
                        complete: conversations
                            .iter()
                            .any(|conversation| conversation.end_of_history_transfer),
                    },
                )),
            });
        }
    }
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
            Event::IncomingCall(incoming) => {
                let offer = self.calls.offer(*incoming.clone());
                self.push(offer);
            }
            Event::Messages(batch) => {
                crate::ingest::spawn_enc_reactions(
                    Arc::clone(&self.store),
                    Arc::clone(&self.client),
                    self.events.clone(),
                    batch,
                );
                let (events, _ingested) = crate::ingest::apply_event(&self.store, &event);
                for backend_event in events {
                    self.push(backend_event);
                }
            }
            other => {
                let (events, _ingested) = crate::ingest::apply_event(&self.store, other);
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

use anyhow::Context as _;
