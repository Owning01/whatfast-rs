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
use whatsapp_rust::voip::{
    AudioSink, AudioSource, CallEvent, CallHandle, VideoFrame, VideoSink, VideoSource,
};

/// A bounded queue of media frames, with the receiver's clone kept beside
/// the sender.
///
/// A live call produces frames at a fixed rate and a stalled consumer must
/// not accumulate them: the queue has a ceiling and, when it is full, the
/// oldest frame leaves so the reader always sees the present. The writer
/// never blocks, because the thread that feeds it is real time.
pub struct FramePipe<T> {
    sender: async_channel::Sender<T>,
    oldest: async_channel::Receiver<T>,
}

impl<T> FramePipe<T> {
    /// Splits a bounded queue into the writer and the handle that throws the
    /// oldest frame away.
    fn new(capacity: usize) -> (Self, async_channel::Receiver<T>) {
        let (sender, receiver) = async_channel::bounded(capacity);
        let oldest = receiver.clone();
        (Self { sender, oldest }, receiver)
    }

    /// Enqueues the frame; when the queue is full, the oldest one leaves.
    /// Returns `false` when the reader is gone: the call is over.
    fn push(&self, value: T) -> bool {
        push_latest(&self.sender, &self.oldest, value)
    }
}

impl<T> Clone for FramePipe<T> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            oldest: self.oldest.clone(),
        }
    }
}

/// Enqueues the value and, when the queue is full, throws the oldest one
/// away and retries with the new value.
///
/// Returns `false` when the reader is gone: the call is over.
fn push_latest<T>(
    tx: &async_channel::Sender<T>,
    oldest: &async_channel::Receiver<T>,
    mut value: T,
) -> bool {
    loop {
        match tx.try_send(value) {
            Ok(()) => return true,
            Err(async_channel::TrySendError::Full(returned)) => {
                let _ = oldest.try_recv();
                value = returned;
            }
            Err(async_channel::TrySendError::Closed(_)) => return false,
        }
    }
}

/// The microphone half: the engine reads frames the desktop sends. Muting
/// substitutes zeros at this boundary, so the engine never sees the diff.
struct BridgeMic {
    upstream: async_channel::Receiver<Vec<i16>>,
    muted: Arc<AtomicBool>,
}

impl AudioSource for BridgeMic {
    fn frames(&self) -> async_channel::Receiver<Vec<i16>> {
        // 160 ms of sound: the engine already keeps its own jitter cushion,
        // and anything older is worse than a gap.
        let (tx, rx) = async_channel::bounded(8);
        let drop_rx = rx.clone();
        let muted = Arc::clone(&self.muted);
        let upstream = self.upstream.clone();
        tokio::spawn(async move {
            while let Ok(mut frame) = upstream.recv().await {
                if muted.load(Ordering::SeqCst) {
                    frame.fill(0);
                }
                if !push_latest(&tx, &drop_rx, frame) {
                    return; // the call is over
                }
            }
        });
        rx
    }
}

/// The speaker half: the engine writes playout frames, the bridge drains.
struct BridgeSpeaker {
    downstream: FramePipe<Vec<i16>>,
}

impl AudioSink for BridgeSpeaker {
    fn playout(&self) -> async_channel::Sender<Vec<i16>> {
        let (tx, rx) = async_channel::bounded(16);
        let downstream = self.downstream.clone();
        tokio::spawn(async move {
            while let Ok(frame) = rx.recv().await {
                // Loss tolerant: a full downstream throws the oldest frame.
                if !downstream.push(frame) {
                    return; // the call is over
                }
            }
        });
        tx
    }
}

/// The camera half: the desktop encodes the H.264 access units and the
/// engine sends them. Full queues drop the unit, like the audio.
struct BridgeVideo {
    upstream: async_channel::Receiver<Vec<u8>>,
}

impl VideoSource for BridgeVideo {
    fn frames(&self) -> async_channel::Receiver<Vec<u8>> {
        // The desktop forwards the encoder's byte stream in arbitrary
        // chunks: the splitter of the library cuts the access units, which
        // is what the engine stamps and sends. Eight units is about half a
        // second at fifteen frames per second.
        let (tx, rx) = async_channel::bounded(8);
        let drop_rx = rx.clone();
        let upstream = self.upstream.clone();
        tokio::spawn(async move {
            let mut splitter = wacore::voip::h264::AnnexBAuSplitter::default();
            while let Ok(chunk) = upstream.recv().await {
                let mut units = Vec::new();
                splitter.push(&chunk, &mut units);
                for unit in units {
                    if !push_latest(&tx, &drop_rx, unit) {
                        return; // the call is over
                    }
                }
            }
        });
        rx
    }

    fn rtp_timestamp_stride(&self) -> u32 {
        // The desktop encodes at 15 frames per second, which is the stride
        // the engine stamps between access units.
        wacore::voip_control::ports::VIDEO_TS_STRIDE_15FPS
    }
}

/// The peer's video half: the engine reassembles access units and the bridge
/// drains them toward the desktop, where a decoder paints them.
struct BridgeVideoSink {
    downstream: FramePipe<VideoFrame>,
}

impl VideoSink for BridgeVideoSink {
    fn playout(&self) -> async_channel::Sender<VideoFrame> {
        // Two pictures: a decoder that falls behind shows the present, not
        // a backlog of the past.
        let (tx, rx) = async_channel::bounded(2);
        let downstream = self.downstream.clone();
        tokio::spawn(async move {
            while let Ok(frame) = rx.recv().await {
                if !downstream.push(frame) {
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
    pub mic: FramePipe<Vec<i16>>,
    pub muted: Arc<AtomicBool>,
    /// The camera pipe, when the call carries video: the desktop writes the
    /// access units here and the engine sends them.
    pub video: Option<FramePipe<Vec<u8>>>,
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
    pub fn offer(&self, incoming: IncomingCall, chat_id: String) -> BackendEvent {
        self.inner.offer(incoming, chat_id)
    }

    /// Drops a ringing offer without answering it.
    pub fn forget(&self, call_id: &str) -> bool {
        self.inner.forget(call_id)
    }

    pub async fn place(
        &self,
        client: &Arc<Client>,
        events: async_channel::Sender<BackendEvent>,
        chat_id: &str,
        video: bool,
    ) -> Result<String, (String, String, bool)> {
        self.inner.place(client, events, chat_id, video).await
    }

    pub async fn accept(
        &self,
        client: &Arc<Client>,
        events: async_channel::Sender<BackendEvent>,
        call_id: &str,
        video: bool,
    ) -> Result<(), (String, String, bool)> {
        self.inner.accept(client, events, call_id, video).await
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

    /// Feeds one access unit of the camera from the desktop.
    pub fn video_in(&self, call_id: &str, au: Vec<u8>) -> Result<(), (String, String, bool)> {
        self.inner.video_in(call_id, au)
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

/// Forwards the peer's video access units to the desktop, which decodes them.
fn spawn_video_drain(
    out_rx: async_channel::Receiver<VideoFrame>,
    events: async_channel::Sender<BackendEvent>,
    call_id: String,
) {
    tokio::spawn(async move {
        while let Ok(frame) = out_rx.recv().await {
            let event = BackendEvent {
                sequence: 0,
                event: Some(whatfast_protocol::backend_event::Event::CallVideoOut(
                    whatfast_protocol::CallVideo {
                        call_id: call_id.clone(),
                        au: frame.data,
                        keyframe: frame.keyframe,
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
    ///
    /// The key is the call id of the action, not the stanza id: the accept,
    /// the reject and the terminate of the same call carry that id, and they
    /// are the ones that have to find this offer.
    pub fn offer(&self, incoming: IncomingCall, chat_id: String) -> BackendEvent {
        let action_id = incoming.action.call_id().to_owned();
        let call_id = if action_id.is_empty() {
            incoming.stanza_id.clone()
        } else {
            action_id
        };
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
                    chat_id,
                    video,
                },
            )),
        }
    }

    fn take_offer(&self, call_id: &str) -> Option<IncomingCall> {
        self.pending.lock().expect("pending lock").remove(call_id)
    }

    /// Drops a ringing offer without answering it, and tells whether it was
    /// there: the caller of a call that ended before the answer.
    pub fn forget(&self, call_id: &str) -> bool {
        self.pending
            .lock()
            .expect("pending lock")
            .remove(call_id)
            .is_some()
    }

    /// Starts an outgoing call and reports it live through `events`.
    pub async fn place(
        &self,
        client: &Arc<Client>,
        events: async_channel::Sender<BackendEvent>,
        chat_id: &str,
        video: bool,
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
        // A live call produces audio at a fixed rate: the queues have a
        // ceiling and the oldest frame leaves when they fill. 160 ms of
        // microphone and 320 ms of speaker cover the jitter cushion of the
        // engine without ever keeping a backlog.
        let (mic, mic_rx) = FramePipe::new(8);
        let (out, out_rx) = FramePipe::new(16);
        let muted = Arc::new(AtomicBool::new(false));
        let source = BridgeMic {
            upstream: mic_rx,
            muted: Arc::clone(&muted),
        };
        let sink = BridgeSpeaker { downstream: out };
        let voip = client.voip();
        let mut call = voip.call(&peer).audio(source, sink);
        // The video ports exist only when the call carries video: the engine
        // refuses them on an offer that did not ask for video.
        let video_ports = if video {
            let (au, au_rx) = FramePipe::<Vec<u8>>::new(8);
            let (peer, peer_rx) = FramePipe::<VideoFrame>::new(2);
            call = call.video(
                BridgeVideo { upstream: au_rx },
                BridgeVideoSink { downstream: peer },
            );
            Some((au, peer_rx))
        } else {
            None
        };
        let handle = call
            .start()
            .await
            .map_err(|error| ("call_failed".into(), format!("{error}"), true))?;
        let call_id = handle.call_id().to_owned();
        let handle = Arc::new(handle);
        self.install(ActiveCall {
            call_id: call_id.clone(),
            chat: chat_id.to_owned(),
            handle: Arc::clone(&handle),
            mic,
            muted: Arc::clone(&muted),
            video: video_ports.as_ref().map(|(au, _)| au.clone()),
        });
        let registry = self
            .self_ref
            .lock()
            .expect("self ref lock")
            .upgrade()
            .expect("the registry outlives its calls");
        spawn_call_watcher(
            registry,
            handle,
            events.clone(),
            call_id.clone(),
        );
        spawn_speaker_drain(out_rx, events.clone(), call_id.clone());
        if let Some((_, peer_rx)) = video_ports {
            spawn_video_drain(peer_rx, events, call_id.clone());
        }
        Ok(call_id)
    }

    /// Answers a ringing offer.
    pub async fn accept(
        &self,
        client: &Arc<Client>,
        events: async_channel::Sender<BackendEvent>,
        call_id: &str,
        video: bool,
    ) -> Result<(), (String, String, bool)> {
        if self.busy() {
            return Err(("busy".into(), "a call is already up".into(), false));
        }
        let incoming = self
            .take_offer(call_id)
            .ok_or_else(|| ("not_found".into(), "no such ringing call".into(), false))?;
        // Answering with video needs an offer that carried it: the engine
        // refuses the ports otherwise, and the call would not start at all.
        let with_video = video
            && matches!(
                &incoming.action,
                wacore::types::call::CallAction::Offer { is_video: true, .. }
            );
        // A live call produces audio at a fixed rate: the queues have a
        // ceiling and the oldest frame leaves when they fill. 160 ms of
        // microphone and 320 ms of speaker cover the jitter cushion of the
        // engine without ever keeping a backlog.
        let (mic, mic_rx) = FramePipe::new(8);
        let (out, out_rx) = FramePipe::new(16);
        let muted = Arc::new(AtomicBool::new(false));
        let source = BridgeMic {
            upstream: mic_rx,
            muted: Arc::clone(&muted),
        };
        let sink = BridgeSpeaker { downstream: out };
        let voip = client.voip();
        let mut call = voip.accept(&incoming).audio(source, sink);
        let video_ports = if with_video {
            let (au, au_rx) = FramePipe::<Vec<u8>>::new(8);
            let (peer, peer_rx) = FramePipe::<VideoFrame>::new(2);
            call = call.video(
                BridgeVideo { upstream: au_rx },
                BridgeVideoSink { downstream: peer },
            );
            Some((au, peer_rx))
        } else {
            None
        };
        let handle = call
            .start()
            .await
            .map_err(|error| ("call_failed".into(), format!("{error}"), true))?;
        let handle = Arc::new(handle);
        self.install(ActiveCall {
            call_id: call_id.to_owned(),
            chat: crate::ingest::jid_text(&incoming.from),
            handle: Arc::clone(&handle),
            mic,
            muted: Arc::clone(&muted),
            video: video_ports.as_ref().map(|(au, _)| au.clone()),
        });
        let registry = self
            .self_ref
            .lock()
            .expect("self ref lock")
            .upgrade()
            .expect("the registry outlives its calls");
        spawn_call_watcher(
            registry,
            handle,
            events.clone(),
            call_id.to_owned(),
        );
        spawn_speaker_drain(out_rx, events.clone(), call_id.to_owned());
        if let Some((_, peer_rx)) = video_ports {
            spawn_video_drain(peer_rx, events, call_id.to_owned());
        }
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

    /// Feeds one microphone frame from the desktop. When the queue is full
    /// the oldest frame leaves: the engine's jitter cushion absorbs far more
    /// than one.
    pub fn audio_in(&self, call_id: &str, pcm: Vec<i16>) -> Result<(), (String, String, bool)> {
        let live = self.live.lock().expect("calls lock");
        let call = live
            .as_ref()
            .filter(|call| call.call_id == call_id)
            .ok_or_else(|| ("not_found".into(), "no such live call".into(), false))?;
        let _ = call.mic.push(pcm);
        Ok(())
    }

    /// Feeds one access unit of the camera from the desktop. A call without
    /// video ignores it: the frames of a camera that started late are not an
    /// error.
    pub fn video_in(&self, call_id: &str, au: Vec<u8>) -> Result<(), (String, String, bool)> {
        let live = self.live.lock().expect("calls lock");
        let call = live
            .as_ref()
            .filter(|call| call.call_id == call_id)
            .ok_or_else(|| ("not_found".into(), "no such live call".into(), false))?;
        if let Some(video) = &call.video {
            let _ = video.push(au);
        }
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

#[cfg(test)]
mod tests {
    use super::BridgeVideo;
    use whatsapp_rust::voip::VideoSource as _;

    #[tokio::test]
    async fn the_camera_chunks_become_access_units() {
        // The desktop forwards the encoder's stream in arbitrary chunks and
        // the engine sends whole access units: the splitter in the middle is
        // what makes the two ends agree.
        let (chunks, upstream) = async_channel::unbounded::<Vec<u8>>();
        let source = BridgeVideo { upstream };
        let frames = source.frames();
        let mut stream: Vec<u8> = Vec::new();
        for _ in 0..3 {
            // One access unit: the delimiter and an IDR slice.
            stream.extend_from_slice(&[0, 0, 0, 1, 0x09, 0xf0]);
            stream.extend_from_slice(&[0, 0, 0, 1, 0x65, 1, 2, 3]);
        }
        for chunk in stream.chunks(5) {
            chunks.send(chunk.to_vec()).await.expect("the chunk");
        }
        drop(chunks);
        let first = frames.recv().await.expect("an access unit");
        assert!(first.starts_with(&[0, 0, 0, 1, 0x09]));
        let second = frames.recv().await.expect("a second access unit");
        assert!(second.starts_with(&[0, 0, 0, 1, 0x09]));
        // The last unit has no following delimiter that closes it.
        assert!(frames.try_recv().is_err());
    }
}
