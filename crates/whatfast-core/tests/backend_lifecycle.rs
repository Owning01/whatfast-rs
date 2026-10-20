//! Rust port of `tauri/tests/backend-lifecycle.test.ts` (frozen oracle).
//!
//! Every case of the TypeScript suite is here with the same name translated
//! to snake_case and the same assertions.
//!
//! The two retry cases are adapted to the synchronous signature of this crate:
//! the original is `async` and awaits an injected waiter, here the waiter is a
//! synchronous callback that records the delay.

use std::fmt;

use whatfast_core::backend_lifecycle::{
    BackendLifecycleDecision, BootstrapFailureDecision, BoundedRetryError, BridgeLifecycle,
    BridgeLifecycleState, RequestGeneration, RestartEpochQueue, active_conversation_ids,
    backend_lifecycle_decision, bootstrap_failure_decision, bounded_retry_delays,
    run_bounded_retry,
};
use whatfast_core::workspace::Pane;

#[derive(Debug, PartialEq, Eq)]
enum TestError {
    Invalid(BoundedRetryError),
    Failed(String),
}

impl From<BoundedRetryError> for TestError {
    fn from(error: BoundedRetryError) -> Self {
        TestError::Invalid(error)
    }
}

impl fmt::Display for TestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TestError::Invalid(error) => write!(formatter, "{error}"),
            TestError::Failed(message) => formatter.write_str(message),
        }
    }
}

fn lifecycle(
    epoch: u64,
    state: BridgeLifecycleState,
    attempt: u32,
    max_attempts: u32,
    message: &str,
) -> BridgeLifecycle {
    BridgeLifecycle {
        epoch,
        state,
        attempt,
        max_attempts,
        message: message.to_string(),
    }
}

fn pane(id: &str, active_chat_id: &str) -> Pane {
    Pane {
        id: id.to_string(),
        tab_chat_ids: Vec::new(),
        active_chat_id: active_chat_id.to_string(),
    }
}

#[test]
fn restart_lifecycle_exposes_bounded_progress_and_a_fresh_resync_epoch() {
    assert_eq!(
        backend_lifecycle_decision(&lifecycle(
            4,
            BridgeLifecycleState::Reconnecting,
            2,
            5,
            "signal 9"
        )),
        BackendLifecycleDecision::Reconnecting {
            detail: "Backend restart 2/5".to_string(),
        }
    );
    assert_eq!(
        backend_lifecycle_decision(&lifecycle(5, BridgeLifecycleState::Reconnected, 2, 5, "")),
        BackendLifecycleDecision::Resync { epoch: 5 }
    );
}

#[test]
fn a_restart_queued_during_another_resync_is_retained_and_coalesced_to_the_latest_epoch() {
    let mut queue = RestartEpochQueue::new();
    queue.push(2);
    assert_eq!(queue.take(), Some(2));
    queue.push(3);
    queue.push(5);
    queue.push(4);
    assert_eq!(queue.take(), Some(5));
    assert_eq!(queue.take(), None);
}

#[test]
fn a_superseding_list_refresh_invalidates_old_epoch_completion_and_cleanup() {
    let mut generation = RequestGeneration::new();
    let old_epoch = generation.begin();
    let fresh_epoch = generation.begin();
    assert!(!generation.is_current(old_epoch));
    assert!(generation.is_current(fresh_epoch));
    generation.invalidate();
    assert!(!generation.is_current(fresh_epoch));
}

#[test]
fn transient_bootstrap_failures_wait_for_lifecycle_recovery_while_terminal_failures_stay_fatal() {
    assert_eq!(
        bootstrap_failure_decision(true),
        BootstrapFailureDecision::Reconnecting
    );
    assert_eq!(
        bootstrap_failure_decision(false),
        BootstrapFailureDecision::Fatal
    );
}

#[test]
fn terminal_and_exhausted_failures_are_actionable_and_never_resync() {
    for (state, prefix) in [
        (BridgeLifecycleState::Fatal, "cannot start safely"),
        (BridgeLifecycleState::RetryExhausted, "repeatedly stopped"),
    ] {
        let decision = backend_lifecycle_decision(&lifecycle(3, state, 5, 5, "profile error"));
        assert!(
            matches!(
                &decision,
                BackendLifecycleDecision::Fatal { message }
                    if message.contains(prefix) && message.contains("profile error")
            ),
            "expected a fatal decision containing {prefix:?}, got {decision:?}"
        );
    }
}

#[test]
fn epoch_resync_refetches_every_distinct_active_pane_conversation() {
    assert_eq!(
        active_conversation_ids(&[
            pane("pane-1", "family"),
            pane("pane-2", "work"),
            pane("pane-3", "family"),
            pane("pane-4", ""),
        ]),
        vec!["family".to_string(), "work".to_string()]
    );
}

#[test]
fn epoch_refresh_retries_with_bounded_exponential_delays_and_returns_once_complete() {
    let mut attempts: Vec<u32> = Vec::new();
    let mut delays: Vec<u64> = Vec::new();
    let result = run_bounded_retry(
        3,
        |attempt| {
            attempts.push(attempt);
            if attempt < 3 {
                Err(TestError::Failed(format!("transient {attempt}")))
            } else {
                Ok("ready")
            }
        },
        |delay_ms| delays.push(delay_ms),
    );
    assert_eq!(result, Ok("ready"));
    assert_eq!(attempts, vec![1, 2, 3]);
    assert_eq!(delays, vec![100, 200]);
    assert_eq!(bounded_retry_delays(3), Ok(vec![0, 100, 200]));
}

#[test]
fn epoch_refresh_exposes_the_final_error_after_its_retry_budget() {
    let mut attempts = 0u32;
    let result: Result<(), TestError> = run_bounded_retry(
        2,
        |_attempt| {
            attempts += 1;
            Err(TestError::Failed(format!("failed {attempts}")))
        },
        |_delay_ms| {},
    );
    assert_eq!(attempts, 2);
    assert!(
        matches!(&result, Err(TestError::Failed(message)) if message.contains("failed 2")),
        "expected the final error, got {result:?}"
    );
}
