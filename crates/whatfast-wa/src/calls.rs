//! Calls: the voip control plane of the sidecar, with the audio crossing
//! the bridge as raw 16 kHz mono i16 frames.
//!
//! One call at a time, the same rule the Go sidecar's design implied and
//! the origin app followed: a second call attempt is refused while one is
//! live. The engine (signaling, encryption, relay, codecs) is the library's;
//! this module owns the state the bridge reports and the two frame pipes.
//!
//! - microphone: the desktop sends `CallAudio` requests; late frames are
//!   dropped, never queued. While muted the sidecar feeds the engine zeros,
//!   so muting works even against a desktop that keeps sending.
//! - speaker: the engine's playout frames leave as `CallAudio` events, the
//!   same shape the desktop sends in.

use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use wacore::types::call::IncomingCall;
use whatfast_protocol::{BackendEvent, CallAudio, CallEnded, CallOffer, CallUpdate};
use whatsapp_rust::Jid;
use whatsapp_rust::client::Client;
use whatsapp_rust::voip::{AudioSink, AudioSource, CallEvent, CallHandle};

/// The microphone half: the engine reads frames the desktop sends. Muting
/// substitutes zeros at this boundary, so the engine never sees the diff.
struct BridgeMic {
    upstream: async_channel::Receiver<Vec<i16>>,
    muted: Arc<AtomicBool>,
}

impl AudioSource for BridgeMic {
    fn frames(&self) -> async_channel::Receiver<Vec<i16>> {
        let (tx, rx) = async_channel::unbounded();
        let muted = Arc::clone(&self.muted);
        let upstream = self.upstream.clone();
        tokio::spawn(async move {
            while let Ok(mut frame) = upstream.recv().await {
                if muted.load(Ordering::SeqCst) {
                    frame.fill(0);
                }
                if tx.send(frame).await.is_err() {
                    return; // the call is over
                }
            }
        });
        rx
    }
}

/// The speaker half: the engine writes playout frames, the bridge drains.
struct BridgeSpeaker {
    downstream: async_channel::Sender<Vec<i16>>,
}

impl AudioSink for BridgeSpeaker {
    fn playout(&self) -> async_channel::Sender<Vec<i16>> {
        let (tx, rx) = async_channel::unbounded();
        let downstream = self.downstream.clone();
        tokio::spawn(async move {
            while let Ok(frame) = rx.recv().await {
                // Loss tolerant: a full downstream drops the frame.
                if downstream.send(frame).await.is_err() {
                    return; // the call is over
                }
            }
        });
        tx
    }
}

/// One live call: its handle and its two frame pipes.
pub struct ActiveCall {
    pub call_id: String,
    pub chat: String,
    pub handle: Arc<CallHandle>,
    pub mic: async_channel::Sender<Vec<i16>>,
    pub muted: Arc<AtomicBool>,
}

/// The call registry: at most one live call, plus the offers still ringing.
pub struct Calls {
    inner: Arc<CallsInner>,
}

pub struct CallsInner {
    live: Mutex<Option<ActiveCall>>,
    pending: Mutex<HashMap<String, IncomingCall>>,
    /// Set by [`Calls::new`]: the watcher retires through it.
    self_ref: Mutex<std::sync::Weak<CallsInner>>,
}

impl Default for Calls {
    fn default() -> Self {
        Self::new()
    }
}

impl Calls {
    pub fn new() -> Self {
        Self {
            inner: {
                let inner = Arc::new(CallsInner {
                    live: Mutex::new(None),
                    pending: Mutex::new(HashMap::new()),
                    self_ref: Mutex::new(std::sync::Weak::new()),
                });
                *inner.self_ref.lock().expect("self ref lock") = Arc::downgrade(&inner);
                inner
            },
        }
    }
    pub fn offer(&self, incoming: IncomingCall) -> BackendEvent {
        self.inner.offer(incoming)
    }

    pub async fn place(
        &self,
        client: &Arc<Client>,
        events: async_channel::Sender<BackendEvent>,
        chat_id: &str,
    ) -> Result<String, (String, String, bool)> {
        self.inner.place(client, events, chat_id).await
    }

    pub async fn accept(
        &self,
        client: &Arc<Client>,
        events: async_channel::Sender<BackendEvent>,
        call_id: &str,
    ) -> Result<(), (String, String, bool)> {
        self.inner.accept(client, events, call_id).await
    }

    pub async fn reject(
        &self,
        client: &Arc<Client>,
        call_id: &str,
    ) -> Result<(), (String, String, bool)> {
        self.inner.reject(client, call_id).await
    }

    pub async fn hangup(&self, call_id: &str) -> Result<(), (String, String, bool)> {
        self.inner.hangup(call_id).await
    }

    pub fn mute(&self, call_id: &str, muted: bool) -> Result<(), (String, String, bool)> {
        self.inner.mute(call_id, muted)
    }

    pub fn audio_in(&self, call_id: &str, pcm: Vec<i16>) -> Result<(), (String, String, bool)> {
        self.inner.audio_in(call_id, pcm)
    }
}

/// Watches one call's media events and reports the phases the bridge
/// carries: dialing until the relay is up, active once media connects,
/// ended when the handle dies.
fn spawn_call_watcher(
    registry: Arc<CallsInner>,
    handle: Arc<CallHandle>,
    events: async_channel::Sender<BackendEvent>,
    call_id: String,
) {
    tokio::spawn(async move {
        let mut media = handle.take_events();
        if let Some(media) = media.as_mut() {
            while let Ok(event) = media.recv().await {
                let phase = match &event {
                    CallEvent::RelayAllocated => Some("connecting"),
                    CallEvent::MediaConnected => Some("active"),
                    CallEvent::RelayAllocateFailed(_)
                    | CallEvent::RelayAllocateTimedOut
                    | CallEvent::MediaSetupFailed(_) => Some("failed"),
                    _ => None,
                };
                if let Some(phase) = phase {
                    let _ = events.send(update_event(&call_id, phase, "")).await;
                }
            }
        }
        handle.wait_ended().await;
        registry.retire(&call_id);
        let _ = events.send(ended_event(&call_id, "remote")).await;
    });
}

/// Forwards the engine's playout frames to the desktop.
fn spawn_speaker_drain(
    out_rx: async_channel::Receiver<Vec<i16>>,
    events: async_channel::Sender<BackendEvent>,
    call_id: String,
) {
    tokio::spawn(async move {
        while let Ok(frame) = out_rx.recv().await {
            let event = BackendEvent {
                sequence: 0,
                event: Some(whatfast_protocol::backend_event::Event::CallAudioOut(
                    CallAudio {
                        call_id: call_id.clone(),
                        pcm: to_le_bytes(&frame),
                    },
                )),
            };
            if events.send(event).await.is_err() {
                return; // the bridge is closing
            }
        }
    });
}

pub fn update_event(call_id: &str, phase: &str, detail: &str) -> BackendEvent {
    BackendEvent {
        sequence: 0,
        event: Some(whatfast_protocol::backend_event::Event::CallUpdate(
            CallUpdate {
                call_id: call_id.to_owned(),
                phase: phase.to_owned(),
                detail: detail.to_owned(),
                muted: false,
            },
        )),
    }
}

pub fn ended_event(call_id: &str, reason: &str) -> BackendEvent {
    BackendEvent {
        sequence: 0,
        event: Some(whatfast_protocol::backend_event::Event::CallEnded(
            CallEnded {
                call_id: call_id.to_owned(),
                reason: reason.to_owned(),
            },
        )),
    }
}

fn to_le_bytes(frame: &[i16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(frame.len() * 2);
    for sample in frame {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

pub fn from_le_bytes(pcm: &[u8]) -> Vec<i16> {
    pcm.as_chunks::<2>()
        .0
        .iter()
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
        .collect()
}

impl CallsInner {
    pub fn busy(&self) -> bool {
        self.live.lock().expect("calls lock").is_some()
    }

    /// Registers a ringing offer the desktop may answer. Older offers make
    /// room once the map grows past a handful: the phone stops ringing the
    /// oldest ones anyway.
    pub fn offer(&self, incoming: IncomingCall) -> BackendEvent {
        let call_id = incoming.stanza_id.clone();
        let video = match &incoming.action {
            wacore::types::call::CallAction::Offer { is_video, .. } => *is_video,
            _ => false,
        };
        self.pending
            .lock()
            .expect("pending lock")
            .insert(call_id.clone(), incoming);
        if self.pending.lock().expect("pending lock").len() > 8
            && let Some(oldest) = self
                .pending
                .lock()
                .expect("pending lock")
                .keys()
                .next()
                .cloned()
        {
            self.pending.lock().expect("pending lock").remove(&oldest);
        }
        BackendEvent {
            sequence: 0,
            event: Some(whatfast_protocol::backend_event::Event::CallOffer(
                CallOffer {
                    call_id,
                    chat_id: String::new(),
                    video,
                },
            )),
        }
    }

    fn take_offer(&self, call_id: &str) -> Option<IncomingCall> {
        self.pending.lock().expect("pending lock").remove(call_id)
    }

    /// Starts an outgoing call and reports it live through `events`.
    pub async fn place(
        &self,
        client: &Arc<Client>,
        events: async_channel::Sender<BackendEvent>,
        chat_id: &str,
    ) -> Result<String, (String, String, bool)> {
        if self.busy() {
            return Err(("busy".into(), "a call is already up".into(), false));
        }
        let peer: Jid = chat_id.parse().map_err(|error| {
            (
                "invalid_argument".into(),
                format!("not a WhatsApp JID: {error}"),
                false,
            )
        })?;
        let (mic_tx, mic_rx) = async_channel::unbounded();
        let (out_tx, out_rx) = async_channel::unbounded();
        let muted = Arc::new(AtomicBool::new(false));
        let source = BridgeMic {
            upstream: mic_rx,
            muted: Arc::clone(&muted),
        };
        let sink = BridgeSpeaker { downstream: out_tx };
        let handle = client
            .voip()
            .call(&peer)
            .audio(source, sink)
            .start()
            .await
            .map_err(|error| ("call_failed".into(), format!("{error}"), true))?;
        let call_id = handle.call_id().to_owned();
        let handle = Arc::new(handle);
        self.install(ActiveCall {
            call_id: call_id.clone(),
            chat: chat_id.to_owned(),
            handle: Arc::clone(&handle),
            mic: mic_tx,
            muted: Arc::clone(&muted),
        });
        let registry = self
            .self_ref
            .lock()
            .expect("self ref lock")
            .upgrade()
            .expect("the registry outlives its calls");
        spawn_call_watcher(registry, handle, events.clone(), call_id.clone());
        spawn_speaker_drain(out_rx, events, call_id.clone());
        Ok(call_id)
    }

    /// Answers a ringing offer.
    pub async fn accept(
        &self,
        client: &Arc<Client>,
        events: async_channel::Sender<BackendEvent>,
        call_id: &str,
    ) -> Result<(), (String, String, bool)> {
        if self.busy() {
            return Err(("busy".into(), "a call is already up".into(), false));
        }
        let incoming = self
            .take_offer(call_id)
            .ok_or_else(|| ("not_found".into(), "no such ringing call".into(), false))?;
        let (mic_tx, mic_rx) = async_channel::unbounded();
        let (out_tx, out_rx) = async_channel::unbounded();
        let muted = Arc::new(AtomicBool::new(false));
        let source = BridgeMic {
            upstream: mic_rx,
            muted: Arc::clone(&muted),
        };
        let sink = BridgeSpeaker { downstream: out_tx };
        let handle = client
            .voip()
            .accept(&incoming)
            .audio(source, sink)
            .start()
            .await
            .map_err(|error| ("call_failed".into(), format!("{error}"), true))?;
        let handle = Arc::new(handle);
        self.install(ActiveCall {
            call_id: call_id.to_owned(),
            chat: crate::ingest::jid_text(&incoming.from),
            handle: Arc::clone(&handle),
            mic: mic_tx,
            muted: Arc::clone(&muted),
        });
        let registry = self
            .self_ref
            .lock()
            .expect("self ref lock")
            .upgrade()
            .expect("the registry outlives its calls");
        spawn_call_watcher(registry, handle, events.clone(), call_id.to_owned());
        spawn_speaker_drain(out_rx, events, call_id.to_owned());
        Ok(())
    }

    /// Refuses a ringing offer.
    pub async fn reject(
        &self,
        client: &Arc<Client>,
        call_id: &str,
    ) -> Result<(), (String, String, bool)> {
        let incoming = self
            .take_offer(call_id)
            .ok_or_else(|| ("not_found".into(), "no such ringing call".into(), false))?;
        client
            .voip()
            .reject(&incoming)
            .await
            .map_err(|error| ("call_failed".into(), format!("{error}"), true))
    }

    /// Hangs the live call up, locally.
    pub async fn hangup(&self, call_id: &str) -> Result<(), (String, String, bool)> {
        let live = self.live.lock().expect("calls lock").take();
        match live {
            Some(call) if call.call_id == call_id => {
                call.handle.hangup_local().await;
                Ok(())
            }
            other => {
                // Put it back if the id did not match: the call is not ours
                // to end.
                if let Some(call) = other {
                    *self.live.lock().expect("calls lock") = Some(call);
                }
                Err(("not_found".into(), "no such live call".into(), false))
            }
        }
    }

    /// Mutes (or unmutes) the microphone of the live call.
    pub fn mute(&self, call_id: &str, muted: bool) -> Result<(), (String, String, bool)> {
        let live = self.live.lock().expect("calls lock");
        let call = live
            .as_ref()
            .filter(|call| call.call_id == call_id)
            .ok_or_else(|| ("not_found".into(), "no such live call".into(), false))?;
        call.muted.store(muted, Ordering::SeqCst);
        Ok(())
    }

    /// Feeds one microphone frame from the desktop. Full queues drop the
    /// frame: the engine's jitter cushion absorbs far more than one.
    pub fn audio_in(&self, call_id: &str, pcm: Vec<i16>) -> Result<(), (String, String, bool)> {
        let live = self.live.lock().expect("calls lock");
        let call = live
            .as_ref()
            .filter(|call| call.call_id == call_id)
            .ok_or_else(|| ("not_found".into(), "no such live call".into(), false))?;
        let _ = call.mic.try_send(pcm);
        Ok(())
    }

    fn install(&self, call: ActiveCall) {
        *self.live.lock().expect("calls lock") = Some(call);
    }

    /// Clears the live call if it is still the one recorded.
    pub fn retire(&self, call_id: &str) {
        let mut live = self.live.lock().expect("calls lock");
        if live.as_ref().is_some_and(|call| call.call_id == call_id) {
            *live = None;
        }
    }
}
