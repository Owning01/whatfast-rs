//! Port of `tauri/src/lib/participant-avatar-queue.ts` (SolidJS frontend,
//! frozen oracle).
//!
//! The queue is a scheduler: it decides what to fetch, what to retry and
//! what to give up on. Fetching the avatar, waiting for the backoff timer
//! and publishing the result are effects, so they belong to the UI crate:
//! this module decides and the UI executes.
//!
//! port: the original runs the fetches itself and arms the retry timers
//! with `setTimeout`. The port splits it in two:
//! - [`ParticipantAvatarQueue::next_requests`] hands over the fetches to
//!   start now, honoring the concurrency and the admission bound.
//! - [`ParticipantAvatarQueue::complete`] takes the outcome of one fetch and
//!   returns the decision the caller must execute
//!   ([`QueueDecision`]): publish the avatar, retry after a delay, cache a
//!   terminal failure or drop the work.
//! - [`ParticipantAvatarQueue::on_retry_timer`] is what the UI calls when
//!   the timer it armed fires.
//!
//! Fidelity notes (the odd bits are deliberate, they are the oracle's):
//! - a retryable failure that runs out of attempts is *not* cached as a
//!   terminal failure: the chat may ask again later.
//! - the terminal failures are evicted from the oldest when the map grows
//!   past its bound, so a chat that failed a long time ago becomes eligible
//!   again.
//! - the admission bound counts the queued, the running and the backoff work
//!   together: `maxQueued + concurrency` participants at most.

use serde_json::Value;

/// The default number of avatars fetched at the same time.
pub const DEFAULT_CONCURRENCY: u32 = 4;

/// The default number of participants waiting for their avatar.
pub const DEFAULT_MAX_QUEUED: u32 = 128;

/// The default number of attempts of one avatar.
pub const DEFAULT_MAX_ATTEMPTS: u32 = 4;

/// The default base of the backoff, in milliseconds.
pub const DEFAULT_RETRY_BASE_MS: i64 = 250;

/// The default number of terminal failures the session remembers.
pub const DEFAULT_MAX_TERMINAL_FAILURES: u32 = 2_048;

/// The state of one participant inside the queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueStatus {
    Queued,
    Running,
    Backoff,
}

/// The counters of the queue.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QueueStats {
    pub running: u32,
    pub queued: u32,
    pub backoff: u32,
    pub tracked: u32,
    pub terminal_failures: u32,
}

/// The limits of the queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParticipantAvatarQueueOptions {
    pub concurrency: u32,
    pub max_queued: u32,
    pub max_attempts: u32,
    pub retry_base_ms: i64,
    pub max_terminal_failures: u32,
}

impl Default for ParticipantAvatarQueueOptions {
    fn default() -> Self {
        Self {
            concurrency: DEFAULT_CONCURRENCY,
            max_queued: DEFAULT_MAX_QUEUED,
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            retry_base_ms: DEFAULT_RETRY_BASE_MS,
            max_terminal_failures: DEFAULT_MAX_TERMINAL_FAILURES,
        }
    }
}

/// A limit of the queue that is not a positive integer.
///
/// port: the original throws `new Error(\`${name} must be a positive
/// integer\`)`; the crate takes no new dependency, so the error is written
/// by hand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueConfigError {
    pub field: &'static str,
}

impl std::fmt::Display for QueueConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{} must be a positive integer", self.field)
    }
}

impl std::error::Error for QueueConfigError {}

/// The shape of the error the backend answers with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AvatarError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

/// Whether a failure of the backend is worth another attempt.
///
/// Only the busy and the timeout failures are retried.
pub fn is_retryable_avatar_error(error: &Value) -> bool {
    let Some(normalized) = avatar_error_shape(error) else {
        return false;
    };
    if !normalized.retryable {
        return false;
    }
    if normalized.code == "busy"
        || normalized.code == "timeout"
        || normalized.code == "avatar_unavailable"
    {
        return true;
    }
    normalized.code == "transport" && matches_timeout(&normalized.message)
}

/// The shape of the error, when it carries one.
///
/// port: the original accepts an `unknown`: a string is parsed as JSON and
/// the shape is read from the object.
pub fn avatar_error_shape(error: &Value) -> Option<AvatarError> {
    if let Some(text) = error.as_str() {
        let Ok(parsed) = serde_json::from_str::<Value>(text) else {
            return None;
        };
        return avatar_error_shape(&parsed);
    }
    let candidate = error.as_object()?;
    let code = candidate.get("code").and_then(Value::as_str)?;
    let message = candidate.get("message").and_then(Value::as_str)?;
    let retryable = candidate.get("retryable").and_then(Value::as_bool)?;
    Some(AvatarError {
        code: code.to_string(),
        message: message.to_string(),
        retryable,
    })
}

/// port: `/timed?\s*out|timeout/i`.
fn matches_timeout(message: &str) -> bool {
    let lowered = message.to_lowercase();
    let bytes = lowered.as_bytes();
    let mut index = 0usize;
    while index + 4 <= bytes.len() {
        if &bytes[index..index + 4] == b"time" {
            let mut cursor = index + 4;
            if cursor < bytes.len() && bytes[cursor] == b'd' {
                cursor += 1;
            }
            while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            if lowered[cursor..].starts_with("out") {
                return true;
            }
        }
        index += 1;
    }
    false
}

/// What the caller must do with the outcome of a fetch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueueDecision {
    /// The avatar is ready: the caller publishes it.
    Hydrated {
        participant_id: String,
        avatar_path: String,
    },
    /// The failure was transient: the caller asks again after the delay.
    RetryAfter {
        participant_id: String,
        delay_ms: i64,
    },
    /// The failure is permanent: the caller must not ask again.
    TerminalFailure { participant_id: String },
    /// The failure was transient but the attempts ran out: the caller may ask
    /// again later, because the original does not cache it as a terminal
    /// failure.
    Dropped { participant_id: String },
    /// The result was ignored: the participant was cancelled meanwhile.
    Ignored,
}

/// One fetch the queue asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AvatarRequest {
    pub participant_id: String,
    /// The attempt this fetch is: the first one is 1.
    pub attempt: u32,
    /// The identity of the fetch, so a late result of a cancelled request is
    /// ignored.
    pub request_id: u64,
}

/// The handle of a subscription: the caller keeps it to stop waiting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subscription {
    participant_id: String,
    subscriber_id: Option<u64>,
}

impl Subscription {
    /// The participant the subscription waits for.
    pub fn participant_id(&self) -> &str {
        &self.participant_id
    }

    /// Whether the subscription was admitted by the queue.
    pub fn is_active(&self) -> bool {
        self.subscriber_id.is_some()
    }
}

/// One participant of the queue.
#[derive(Clone, Debug)]
struct QueueEntry {
    participant_id: String,
    status: QueueStatus,
    attempts: u32,
    subscribers: Vec<(u64, String)>,
    request_id: u64,
}

/// The visibility-driven, bounded scheduler shared by every participant
/// roster.
///
/// port: the running Tauri invokes cannot be aborted, but the cancellation
/// removes their subscribers, so the late results and the retries are
/// ignored.
#[derive(Debug)]
pub struct ParticipantAvatarQueue {
    concurrency: u32,
    max_queued: u32,
    max_attempts: u32,
    retry_base_ms: i64,
    max_terminal_failures: u32,
    entries: Vec<QueueEntry>,
    queued: Vec<String>,
    terminal_failures: Vec<String>,
    next_subscriber_id: u64,
    next_request_id: u64,
    running: u32,
}

impl ParticipantAvatarQueue {
    /// A queue with the given limits.
    ///
    /// port: the original throws when a limit is not a positive integer.
    pub fn new(options: ParticipantAvatarQueueOptions) -> Result<Self, QueueConfigError> {
        let concurrency = positive_integer(options.concurrency, "concurrency")?;
        let max_queued = positive_integer(options.max_queued, "maxQueued")?;
        let max_attempts = positive_integer(options.max_attempts, "maxAttempts")?;
        let max_terminal_failures =
            positive_integer(options.max_terminal_failures, "maxTerminalFailures")?;
        // port: the base of the backoff is clamped to zero, not validated.
        let retry_base_ms = options.retry_base_ms.max(0);
        Ok(Self::with_limits(
            concurrency,
            max_queued,
            max_attempts,
            retry_base_ms,
            max_terminal_failures,
        ))
    }

    /// A queue with the default limits.
    pub fn with_defaults() -> Self {
        let options = ParticipantAvatarQueueOptions::default();
        Self::with_limits(
            options.concurrency,
            options.max_queued,
            options.max_attempts,
            options.retry_base_ms,
            options.max_terminal_failures,
        )
    }

    fn with_limits(
        concurrency: u32,
        max_queued: u32,
        max_attempts: u32,
        retry_base_ms: i64,
        max_terminal_failures: u32,
    ) -> Self {
        Self {
            concurrency,
            max_queued,
            max_attempts,
            retry_base_ms,
            max_terminal_failures,
            entries: Vec::new(),
            queued: Vec::new(),
            terminal_failures: Vec::new(),
            next_subscriber_id: 1,
            next_request_id: 0,
            running: 0,
        }
    }

    /// Wait for the avatar of one participant.
    ///
    /// port: the queue refuses the subscription when the participant is
    /// already known as a terminal failure, or when the admission bound is
    /// reached. The returned handle tells whether it was admitted.
    pub fn subscribe(&mut self, participant_id: &str, scope_id: &str) -> Subscription {
        if participant_id.is_empty()
            || scope_id.is_empty()
            || self
                .terminal_failures
                .iter()
                .any(|failure| failure == participant_id)
        {
            return Subscription {
                participant_id: participant_id.to_string(),
                subscriber_id: None,
            };
        }
        if !self.is_tracked(participant_id) {
            // port: the queued, the running and the backoff work share one
            // hard admission bound.
            if self.entries.len() >= (self.max_queued + self.concurrency) as usize {
                return Subscription {
                    participant_id: participant_id.to_string(),
                    subscriber_id: None,
                };
            }
            self.entries.push(QueueEntry {
                participant_id: participant_id.to_string(),
                status: QueueStatus::Queued,
                attempts: 0,
                subscribers: Vec::new(),
                request_id: 0,
            });
            self.queued.push(participant_id.to_string());
        }
        let subscriber_id = self.next_subscriber_id;
        self.next_subscriber_id += 1;
        if let Some(index) = self.entry_index(participant_id) {
            self.entries[index]
                .subscribers
                .push((subscriber_id, scope_id.to_string()));
        }
        Subscription {
            participant_id: participant_id.to_string(),
            subscriber_id: Some(subscriber_id),
        }
    }

    /// Stop waiting for the avatar of one participant.
    pub fn unsubscribe(&mut self, subscription: &Subscription) {
        let Some(subscriber_id) = subscription.subscriber_id else {
            return;
        };
        self.remove_subscriber(&subscription.participant_id, subscriber_id);
    }

    /// Drop the subscribers of one scope.
    pub fn cancel_scope(&mut self, scope_id: &str) {
        let participant_ids: Vec<String> = self
            .entries
            .iter()
            .map(|entry| entry.participant_id.clone())
            .collect();
        for participant_id in participant_ids {
            if let Some(index) = self.entry_index(&participant_id) {
                self.entries[index]
                    .subscribers
                    .retain(|(_, scope)| scope != scope_id);
            }
            self.discard_if_unobserved(&participant_id);
        }
    }

    /// Forget every participant.
    ///
    /// port: the original also clears the retry timers; the timers belong to
    /// the UI crate, so the caller cancels them.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.queued.clear();
        self.terminal_failures.clear();
    }

    /// The counters of the queue.
    pub fn stats(&self) -> QueueStats {
        let backoff = self
            .entries
            .iter()
            .filter(|entry| entry.status == QueueStatus::Backoff)
            .count() as u32;
        let queued = self
            .queued
            .iter()
            .filter(|participant_id| self.is_tracked(participant_id))
            .count() as u32;
        QueueStats {
            running: self.running,
            queued,
            backoff,
            tracked: self.entries.len() as u32,
            terminal_failures: self.terminal_failures.len() as u32,
        }
    }

    /// Whether the queue still tracks one participant.
    pub fn is_tracked(&self, participant_id: &str) -> bool {
        self.entry_index(participant_id).is_some()
    }

    /// The fetches the caller must start now.
    ///
    /// port: the original pumps the queue as soon as a participant is
    /// subscribed and as soon as a fetch finishes; the port hands the fetches
    /// over and the caller starts them.
    pub fn next_requests(&mut self) -> Vec<AvatarRequest> {
        let mut requests: Vec<AvatarRequest> = Vec::new();
        while self.running < self.concurrency && !self.queued.is_empty() {
            let participant_id = self.queued.remove(0);
            let Some(index) = self.entry_index(&participant_id) else {
                continue;
            };
            if self.entries[index].subscribers.is_empty() {
                continue;
            }
            self.entries[index].status = QueueStatus::Running;
            self.entries[index].attempts += 1;
            self.running += 1;
            self.next_request_id += 1;
            let request_id = self.next_request_id;
            self.entries[index].request_id = request_id;
            requests.push(AvatarRequest {
                participant_id,
                attempt: self.entries[index].attempts,
                request_id,
            });
        }
        requests
    }

    /// Take the outcome of one fetch and decide what happens next.
    ///
    /// port: the original catches the error of the fetch and keeps the queue
    /// alive; the caller does the same with the decision it gets back.
    pub fn complete(
        &mut self,
        request: &AvatarRequest,
        outcome: Result<String, Value>,
    ) -> QueueDecision {
        let mut decision = QueueDecision::Ignored;
        let index = self
            .entry_index(&request.participant_id)
            .filter(|index| self.entries[*index].request_id == request.request_id);
        match index {
            // port: the original ignores the result when the entry is gone
            // or when nobody watches it anymore.
            Some(index) if !self.entries[index].subscribers.is_empty() => match outcome {
                Ok(avatar_path) => {
                    self.entries.remove(index);
                    if avatar_path.is_empty() {
                        self.record_terminal_failure(&request.participant_id);
                        decision = QueueDecision::TerminalFailure {
                            participant_id: request.participant_id.clone(),
                        };
                    } else {
                        decision = QueueDecision::Hydrated {
                            participant_id: request.participant_id.clone(),
                            avatar_path,
                        };
                    }
                }
                Err(error) => {
                    let retryable = is_retryable_avatar_error(&error);
                    let attempts = self.entries[index].attempts;
                    if retryable && attempts < self.max_attempts {
                        self.entries[index].status = QueueStatus::Backoff;
                        let delay_ms = self.retry_delay_ms(attempts);
                        decision = QueueDecision::RetryAfter {
                            participant_id: request.participant_id.clone(),
                            delay_ms,
                        };
                    } else {
                        self.entries.remove(index);
                        if retryable {
                            decision = QueueDecision::Dropped {
                                participant_id: request.participant_id.clone(),
                            };
                        } else {
                            self.record_terminal_failure(&request.participant_id);
                            decision = QueueDecision::TerminalFailure {
                                participant_id: request.participant_id.clone(),
                            };
                        }
                    }
                }
            },
            _ => {}
        }

        // port: the `finally` of the original: the running count goes down
        // and the entry is dropped when nobody watches it anymore.
        self.running = self.running.saturating_sub(1);
        if let Some(index) = self
            .entry_index(&request.participant_id)
            .filter(|index| self.entries[*index].request_id == request.request_id)
        {
            if self.entries[index].subscribers.is_empty() {
                self.entries.remove(index);
            }
        }
        decision
    }

    /// The backoff of one attempt, in milliseconds.
    ///
    /// port: `this.#retryBaseMs * 2 ** (entry.attempts - 1)`.
    pub fn retry_delay_ms(&self, attempts: u32) -> i64 {
        let exponent = attempts.saturating_sub(1);
        self.retry_base_ms
            .saturating_mul(2i64.saturating_pow(exponent))
    }

    /// The timer of the backoff fired: the participant is ready again.
    ///
    /// port: the callback of the `setTimeout` of the original. It returns
    /// `false` when the participant was cancelled meanwhile.
    pub fn on_retry_timer(&mut self, participant_id: &str) -> bool {
        let Some(index) = self.entry_index(participant_id) else {
            return false;
        };
        if self.entries[index].subscribers.is_empty() {
            self.discard_if_unobserved(participant_id);
            return false;
        }
        self.entries[index].status = QueueStatus::Queued;
        self.entries[index].request_id = 0;
        self.queued.push(participant_id.to_string());
        true
    }

    fn entry_index(&self, participant_id: &str) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.participant_id == participant_id)
    }

    fn remove_subscriber(&mut self, participant_id: &str, subscriber_id: u64) {
        let Some(index) = self.entry_index(participant_id) else {
            return;
        };
        self.entries[index]
            .subscribers
            .retain(|(id, _)| *id != subscriber_id);
        self.discard_if_unobserved(participant_id);
    }

    fn discard_if_unobserved(&mut self, participant_id: &str) {
        let Some(index) = self.entry_index(participant_id) else {
            return;
        };
        if !self.entries[index].subscribers.is_empty()
            || self.entries[index].status == QueueStatus::Running
        {
            return;
        }
        self.entries.remove(index);
        self.queued.retain(|queued| queued != participant_id);
    }

    fn record_terminal_failure(&mut self, participant_id: &str) {
        self.terminal_failures
            .retain(|failure| failure != participant_id);
        self.terminal_failures.push(participant_id.to_string());
        if self.terminal_failures.len() as u32 <= self.max_terminal_failures {
            return;
        }
        if !self.terminal_failures.is_empty() {
            self.terminal_failures.remove(0);
        }
    }
}

/// port: `positiveInteger`.
fn positive_integer(value: u32, name: &'static str) -> Result<u32, QueueConfigError> {
    if value == 0 {
        return Err(QueueConfigError { field: name });
    }
    Ok(value)
}
