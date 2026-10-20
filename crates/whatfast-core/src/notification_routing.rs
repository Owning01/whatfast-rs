//! Port of `tauri/src/lib/notification-routing.ts` (SolidJS frontend, frozen
//! oracle).
//!
//! The routing of a notification activation is pure: the queue decides what
//! to route and when, and the UI crate routes it. The original drains the
//! queue with promises; the port keeps the same order and the same
//! deduplication, and hands the targets over one at a time, because the
//! original awaits each route before taking the next one.
//!
//! Fidelity notes (the odd bits are deliberate, they are the oracle's):
//! - the pending queue is bounded at 32 activations and the delivered set at
//!   256, and both evict the oldest first.
//! - `mergeChatId` re-keys the pending queue and the delivered set with the
//!   canonical chat id, so an activation queued before the merge and one
//!   queued after it collapse into one.
//! - `NotificationPermissionGate` caches the denial of the session: the read
//!   of the OS permission only happens when the gate has no decision, or
//!   when the user asks for an explicit retry.

use std::collections::VecDeque;

/// The pending activations the queue keeps before it drops the oldest.
pub const MAX_PENDING_ACTIVATIONS: usize = 32;

/// The delivered activations the queue remembers to deduplicate them.
pub const MAX_DELIVERED_ACTIVATIONS: usize = 256;

/// The chat and the message a notification activation points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationTarget {
    pub chat_id: String,
    pub message_id: String,
}

/// What the app can do with the activation of a notification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationTargetAvailability {
    Available,
    MissingChat,
    MissingMessage,
}

/// Whether the activation still points at something the app can show.
pub fn notification_target_availability(
    chat_exists: bool,
    message_exists: bool,
) -> NotificationTargetAvailability {
    if !chat_exists {
        return NotificationTargetAvailability::MissingChat;
    }
    if message_exists {
        NotificationTargetAvailability::Available
    } else {
        NotificationTargetAvailability::MissingMessage
    }
}

/// The target of an activation, when the payload carries one.
///
/// port: the original takes an `unknown`, because the payload comes from the
/// platform; the port takes the parsed JSON of that payload.
pub fn normalized_target(value: &serde_json::Value) -> Option<NotificationTarget> {
    // port: `if (!value || typeof value !== "object") return undefined`. An
    // array is an object in JS, but it carries neither `chatId` nor
    // `messageId`, so it is rejected the same way.
    let candidate = value.as_object()?;
    let chat_id = candidate
        .get("chatId")
        .and_then(serde_json::Value::as_str)?;
    let message_id = candidate
        .get("messageId")
        .and_then(serde_json::Value::as_str)?;
    let normalized = NotificationTarget {
        chat_id: js_trim(chat_id).to_string(),
        message_id: js_trim(message_id).to_string(),
    };
    if normalized.chat_id.is_empty() || normalized.message_id.is_empty() {
        return None;
    }
    Some(normalized)
}

/// The queue of the notification activations that arrive before the app is
/// ready.
///
/// port: the original serializes the activations while the bootstrap is still
/// loading. The platform can deliver the same activation through a native
/// event and through the cold-start drain, so the completed targets are
/// bounded and deduplicated as well.
#[derive(Debug, Default)]
pub struct NotificationActivationQueue {
    ready: bool,
    pending: VecDeque<(TargetKey, NotificationTarget)>,
    delivered: VecDeque<TargetKey>,
    aliases: Vec<(String, String)>,
    routing: Option<NotificationTarget>,
}

/// The identity of an activation: the chat and the message.
type TargetKey = (String, String);

impl NotificationActivationQueue {
    /// An empty queue, waiting for the bootstrap.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the queue routes the activations it holds.
    pub fn is_ready(&self) -> bool {
        self.ready
    }

    /// How many activations wait for the bootstrap.
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// Accept an activation.
    ///
    /// Returns `false` when the payload is not a target or when the target
    /// is already queued or already delivered.
    pub fn enqueue(&mut self, value: &serde_json::Value) -> bool {
        let Some(parsed) = normalized_target(value) else {
            return false;
        };
        let target = self.canonicalize(parsed);
        let key = target_key(&target);
        if self.pending.iter().any(|(pending, _)| *pending == key) {
            return false;
        }
        if self.delivered.iter().any(|delivered| *delivered == key) {
            return false;
        }
        // port: the oldest activation is dropped when the queue is full.
        while self.pending.len() >= MAX_PENDING_ACTIVATIONS {
            self.pending.pop_front();
        }
        self.pending.push_back((key, target));
        true
    }

    /// The bootstrap is done: the queue starts routing.
    pub fn mark_ready(&mut self) {
        self.ready = true;
    }

    /// Pause the routing while a fresh backend epoch is being hydrated.
    pub fn mark_not_ready(&mut self) {
        self.ready = false;
    }

    /// Keep the pre-bootstrap clicks valid when WhatsApp reports a JID merge.
    pub fn merge_chat_id(&mut self, old_chat_id: &str, new_chat_id: &str) {
        if old_chat_id.is_empty() || new_chat_id.is_empty() || old_chat_id == new_chat_id {
            return;
        }
        let resolved = self.resolve_chat_id(new_chat_id);
        self.set_alias(old_chat_id, &resolved);

        // port: the delivered set is re-keyed with the canonical chat id, so
        // the activation delivered before the merge also suppresses the one
        // that arrives after it.
        let mut delivered: VecDeque<TargetKey> = VecDeque::new();
        for key in &self.delivered {
            let canonical = self.canonicalize(NotificationTarget {
                chat_id: key.0.clone(),
                message_id: key.1.clone(),
            });
            let key = target_key(&canonical);
            if !delivered.iter().any(|existing| *existing == key) {
                delivered.push_back(key);
            }
        }
        self.delivered = delivered;

        // port: the pending queue is re-keyed the same way, and the targets
        // that are already delivered are dropped.
        let pending: Vec<NotificationTarget> = self
            .pending
            .iter()
            .map(|(_, target)| target.clone())
            .collect();
        let mut remapped: VecDeque<(TargetKey, NotificationTarget)> = VecDeque::new();
        for target in pending {
            let canonical = self.canonicalize(target);
            let key = target_key(&canonical);
            if self.delivered.iter().any(|existing| *existing == key) {
                continue;
            }
            match remapped.iter_mut().find(|(existing, _)| *existing == key) {
                Some(entry) => entry.1 = canonical,
                None => remapped.push_back((key, canonical)),
            }
        }
        self.pending = remapped;
    }

    /// The next activation the caller must route.
    ///
    /// port: the original drains the queue inside `enqueue` and awaits every
    /// route before taking the next one, so the port hands the targets over
    /// one at a time: the caller routes the target it gets and then calls
    /// [`NotificationActivationQueue::route_completed`]. The caller asks for
    /// the next target right after an enqueue and right after a route, which
    /// is what the drain of the original does.
    pub fn next_target(&mut self) -> Option<NotificationTarget> {
        if !self.ready || self.routing.is_some() {
            return None;
        }
        let (_, target) = self.pending.pop_front()?;
        self.delivered.push_back(target_key(&target));
        // port: the oldest delivered activation is dropped when the set is
        // full.
        while self.delivered.len() > MAX_DELIVERED_ACTIVATIONS {
            self.delivered.pop_front();
        }
        self.routing = Some(target.clone());
        Some(target)
    }

    /// The route of the last target finished, with or without an error.
    ///
    /// port: one stale OS activation must not block the later clicks of the
    /// queue, so the original catches the error of the route and keeps
    /// draining.
    pub fn route_completed(&mut self) {
        self.routing = None;
    }

    /// The target the queue is routing right now, if any.
    pub fn routing_target(&self) -> Option<&NotificationTarget> {
        self.routing.as_ref()
    }

    fn set_alias(&mut self, old_chat_id: &str, new_chat_id: &str) {
        match self
            .aliases
            .iter_mut()
            .find(|(alias, _)| alias == old_chat_id)
        {
            Some(alias) => alias.1 = new_chat_id.to_string(),
            None => self
                .aliases
                .push((old_chat_id.to_string(), new_chat_id.to_string())),
        }
    }

    fn resolve_chat_id(&self, chat_id: &str) -> String {
        let mut visited: Vec<String> = Vec::new();
        let mut current = chat_id.to_string();
        loop {
            if visited.iter().any(|seen| seen == &current) {
                break;
            }
            let next = self
                .aliases
                .iter()
                .find(|(alias, _)| alias == &current)
                .map(|(_, target)| target.clone());
            let Some(next) = next else {
                break;
            };
            visited.push(current.clone());
            current = next;
        }
        current
    }

    fn canonicalize(&self, target: NotificationTarget) -> NotificationTarget {
        NotificationTarget {
            chat_id: self.resolve_chat_id(&target.chat_id),
            message_id: target.message_id,
        }
    }
}

/// The identity of an activation.
fn target_key(target: &NotificationTarget) -> TargetKey {
    (target.chat_id.clone(), target.message_id.clone())
}

/// The gate of the permission of the notifications.
///
/// port: the denial is cached for the session and only an explicit settings
/// action may retry, so the read of the OS permission only happens when the
/// gate holds no decision.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NotificationPermissionGate {
    decision: Option<bool>,
}

impl NotificationPermissionGate {
    /// A gate without a decision yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The cached decision, if the gate holds one.
    pub fn decision(&self) -> Option<bool> {
        self.decision
    }

    /// The decision of the permission.
    ///
    /// The `read` closure asks the OS for the permission; it only runs when
    /// the gate holds no decision, or when `refresh` asks for a new one.
    ///
    /// port: the original turns a rejected read into `false`; the UI passes
    /// that answer in.
    pub fn check(&mut self, read: impl FnOnce() -> bool, refresh: bool) -> bool {
        if refresh {
            self.decision = None;
        }
        match self.decision {
            Some(decision) => decision,
            None => {
                let decision = read();
                self.decision = Some(decision);
                decision
            }
        }
    }
}

/// port: `String.prototype.trim`.
fn js_trim(value: &str) -> &str {
    value.trim()
}
