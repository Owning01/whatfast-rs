//! Port of `tauri/src/state/backend-lifecycle.ts` (SolidJS frontend, frozen oracle).
//!
//! Epochs, generations and bounded retries.
//!
//! Fidelity notes (the odd bits are deliberate, they are the oracle's):
//! - `RestartEpochQueue` treats the epoch `0` as "nothing pending", so pushing
//!   a `0` is lost.
//! - `activeConversationIds` filters with `Boolean`, which drops the empty
//!   chat id of a pane without tabs.
//! - `runBoundedRetry` is `async` in the original because it awaits the
//!   injected waiter. `whatfast-core` is pure: there is no clock and no
//!   runtime here, so the port splits the policy in two.
//!   [`bounded_retry_delays`] is the pure schedule and [`run_bounded_retry`]
//!   runs the loop synchronously, handing the delay to wait to a callback the
//!   UI crate implements with its own timer.

use std::collections::HashSet;
use std::fmt;

use crate::workspace::Pane;

/// The state the bridge reports about the backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BridgeLifecycleState {
    Reconnecting,
    Reconnected,
    RetryExhausted,
    Fatal,
}

/// The lifecycle payload the bridge emits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BridgeLifecycle {
    pub epoch: u64,
    pub state: BridgeLifecycleState,
    pub attempt: u32,
    pub max_attempts: u32,
    pub message: String,
}

/// What the UI has to show for a lifecycle payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackendLifecycleDecision {
    Reconnecting { detail: String },
    Resync { epoch: u64 },
    Fatal { message: String },
}

/// Keep lifecycle presentation deterministic and exhaustive across platforms.
pub fn backend_lifecycle_decision(lifecycle: &BridgeLifecycle) -> BackendLifecycleDecision {
    match lifecycle.state {
        BridgeLifecycleState::Reconnecting => BackendLifecycleDecision::Reconnecting {
            detail: format!(
                "Backend restart {}/{}",
                lifecycle.attempt, lifecycle.max_attempts
            ),
        },
        BridgeLifecycleState::Reconnected => BackendLifecycleDecision::Resync {
            epoch: lifecycle.epoch,
        },
        BridgeLifecycleState::RetryExhausted => BackendLifecycleDecision::Fatal {
            message: format!(
                "The WhatsApp backend repeatedly stopped: {}",
                lifecycle.message
            ),
        },
        BridgeLifecycleState::Fatal => BackendLifecycleDecision::Fatal {
            message: format!(
                "The WhatsApp backend cannot start safely: {}",
                lifecycle.message
            ),
        },
    }
}

/// The conversations the epoch resync has to refetch.
pub fn active_conversation_ids(panes: &[Pane]) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut ids: Vec<String> = Vec::new();
    for pane in panes {
        // port: the original filters with `Boolean`, so the empty chat id of a
        // pane without tabs is dropped.
        if pane.active_chat_id.is_empty() {
            continue;
        }
        if seen.insert(pane.active_chat_id.clone()) {
            ids.push(pane.active_chat_id.clone());
        }
    }
    ids
}

/// What a bootstrap failure means for the lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootstrapFailureDecision {
    Reconnecting,
    Fatal,
}

/// Decide whether a bootstrap failure is worth waiting for.
pub fn bootstrap_failure_decision(retryable: bool) -> BootstrapFailureDecision {
    if retryable {
        BootstrapFailureDecision::Reconnecting
    } else {
        BootstrapFailureDecision::Fatal
    }
}

/// Coalesce overlapping lifecycle events without ever dropping the newest epoch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RestartEpochQueue {
    pending_epoch: u64,
}

impl RestartEpochQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Remember the newest epoch seen.
    pub fn push(&mut self, epoch: u64) {
        self.pending_epoch = self.pending_epoch.max(epoch);
    }

    /// Take the pending epoch, if any.
    pub fn take(&mut self) -> Option<u64> {
        // port: the epoch `0` means "nothing pending" in the original too.
        if self.pending_epoch == 0 {
            return None;
        }
        let epoch = self.pending_epoch;
        self.pending_epoch = 0;
        Some(epoch)
    }
}

/// Tokens stale in-flight reads so only the newest epoch may update shared state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RequestGeneration {
    current: u64,
}

impl RequestGeneration {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a new request and return its token.
    pub fn begin(&mut self) -> u64 {
        self.current += 1;
        self.current
    }

    /// Invalidate the token in flight.
    pub fn invalidate(&mut self) {
        self.current += 1;
    }

    /// Whether the token is the one in flight.
    pub fn is_current(&self, generation: u64) -> bool {
        generation == self.current
    }
}

/// The retry budget is not a positive integer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundedRetryError {
    InvalidMaxAttempts,
}

impl fmt::Display for BoundedRetryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BoundedRetryError::InvalidMaxAttempts => {
                formatter.write_str("maxAttempts must be a positive integer")
            }
        }
    }
}

impl std::error::Error for BoundedRetryError {}

/// The delay the original waits before the next attempt, in milliseconds.
pub const RETRY_BASE_DELAY_MS: u64 = 100;

/// The delay to wait after the given attempt failed.
///
/// port: the original computes `100 * 2 ** (attempt - 1)`.
pub fn retry_delay_ms(failed_attempt: u32) -> u64 {
    let exponent = failed_attempt.saturating_sub(1).min(40);
    RETRY_BASE_DELAY_MS.saturating_mul(1u64 << exponent)
}

/// The pure schedule of a bounded retry: the delay to wait *before* each
/// attempt. The first attempt never waits.
pub fn bounded_retry_delays(max_attempts: u32) -> Result<Vec<u64>, BoundedRetryError> {
    if max_attempts < 1 {
        return Err(BoundedRetryError::InvalidMaxAttempts);
    }
    let mut delays = Vec::with_capacity(max_attempts as usize);
    for attempt in 1..=max_attempts {
        delays.push(if attempt == 1 {
            0
        } else {
            retry_delay_ms(attempt - 1)
        });
    }
    Ok(delays)
}

/// Retry one epoch refresh a bounded number of times.
///
/// port: the original is `async` and awaits the injected waiter between the
/// attempts. This crate is pure, so the waiter is a synchronous callback and
/// the delays it receives are the ones [`bounded_retry_delays`] publishes; the
/// UI crate turns them into real waits.
pub fn run_bounded_retry<T, E, F, W>(
    max_attempts: u32,
    mut operation: F,
    mut wait: W,
) -> Result<T, E>
where
    E: From<BoundedRetryError>,
    F: FnMut(u32) -> Result<T, E>,
    W: FnMut(u64),
{
    if max_attempts < 1 {
        return Err(E::from(BoundedRetryError::InvalidMaxAttempts));
    }
    let mut attempt = 1u32;
    loop {
        match operation(attempt) {
            Ok(value) => return Ok(value),
            Err(error) => {
                if attempt >= max_attempts {
                    return Err(error);
                }
                wait(retry_delay_ms(attempt));
                attempt += 1;
            }
        }
    }
}
