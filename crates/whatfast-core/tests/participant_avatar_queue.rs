//! Port of the frozen suite `tauri/tests/participant-avatar-queue.test.ts`.
//!
//! The oracle fetches the avatars with promises and arms the retry timers
//! with `setTimeout`; the port decides and the caller executes: the queue
//! hands the fetches over ([`next_requests`]) and takes the outcome back
//! ([`complete`]), which returns what the caller must do.

use whatfast_core::participant_avatar_queue::{
    ParticipantAvatarQueue, ParticipantAvatarQueueOptions, QueueDecision, QueueStats,
    is_retryable_avatar_error,
};

fn new_queue(options: ParticipantAvatarQueueOptions) -> ParticipantAvatarQueue {
    ParticipantAvatarQueue::new(options).expect("the limits of the suite are valid")
}

fn avatar_error(code: &str, message: &str, retryable: bool) -> serde_json::Value {
    serde_json::json!({ "code": code, "message": message, "retryable": retryable })
}

#[test]
fn avatar_hydration_bounds_concurrency_and_deduplicates_subscribers() {
    let mut queue = new_queue(ParticipantAvatarQueueOptions {
        concurrency: 2,
        ..ParticipantAvatarQueueOptions::default()
    });
    let mut calls: Vec<String> = Vec::new();
    let mut hydrated: Vec<String> = Vec::new();

    queue.subscribe("a", "group");
    queue.subscribe("a", "group");
    queue.subscribe("b", "group");
    queue.subscribe("c", "group");

    let requests = queue.next_requests();
    calls.extend(
        requests
            .iter()
            .map(|request| request.participant_id.clone()),
    );
    assert_eq!(calls, vec!["a".to_string(), "b".to_string()]);
    assert_eq!(
        queue.stats(),
        QueueStats {
            running: 2,
            queued: 1,
            backoff: 0,
            tracked: 3,
            terminal_failures: 0,
        }
    );

    let decision = queue.complete(&requests[0], Ok("/a.jpg".to_string()));
    if let QueueDecision::Hydrated {
        participant_id,
        avatar_path: _,
    } = &decision
    {
        hydrated.push(participant_id.clone());
    }
    let requests = queue.next_requests();
    calls.extend(
        requests
            .iter()
            .map(|request| request.participant_id.clone()),
    );

    assert_eq!(
        calls,
        vec!["a".to_string(), "b".to_string(), "c".to_string()]
    );
    assert_eq!(hydrated, vec!["a".to_string()]);
    queue.clear();
}

#[test]
fn unsubscribing_drops_queued_work_and_ignores_a_late_running_result() {
    let mut queue = new_queue(ParticipantAvatarQueueOptions {
        concurrency: 1,
        ..ParticipantAvatarQueueOptions::default()
    });
    let mut calls: Vec<String> = Vec::new();
    let mut hydrated: Vec<String> = Vec::new();

    let cancel_running = queue.subscribe("running", "old-group");
    let cancel_queued = queue.subscribe("queued", "old-group");
    let requests = queue.next_requests();
    calls.extend(
        requests
            .iter()
            .map(|request| request.participant_id.clone()),
    );
    queue.unsubscribe(&cancel_queued);
    queue.unsubscribe(&cancel_running);

    let decision = queue.complete(&requests[0], Ok("/late.jpg".to_string()));
    if let QueueDecision::Hydrated { participant_id, .. } = &decision {
        hydrated.push(participant_id.clone());
    }

    assert_eq!(calls, vec!["running".to_string()]);
    assert_eq!(hydrated, Vec::<String>::new());
    assert_eq!(queue.stats().tracked, 0);
}

#[test]
fn cancel_scope_removes_only_the_switched_groups_subscribers() {
    let mut queue = new_queue(ParticipantAvatarQueueOptions::default());
    let mut hydrated: Vec<String> = Vec::new();

    queue.subscribe("same-person", "old-group");
    queue.subscribe("same-person", "visible-group");
    queue.cancel_scope("old-group");
    let requests = queue.next_requests();
    let decision = queue.complete(&requests[0], Ok("/avatar.jpg".to_string()));
    if let QueueDecision::Hydrated { participant_id, .. } = &decision {
        hydrated.push(participant_id.clone());
    }

    assert_eq!(hydrated, vec!["same-person".to_string()]);
}

#[test]
fn queued_running_and_backoff_work_share_one_hard_admission_bound() {
    let mut queue = new_queue(ParticipantAvatarQueueOptions {
        concurrency: 2,
        max_queued: 3,
        ..ParticipantAvatarQueueOptions::default()
    });

    for index in 0..20 {
        queue.subscribe(&format!("member-{index}"), "group");
    }
    let requests = queue.next_requests();
    assert_eq!(requests.len(), 2);

    assert_eq!(
        queue.stats(),
        QueueStats {
            running: 2,
            queued: 3,
            backoff: 0,
            tracked: 5,
            terminal_failures: 0,
        }
    );
    queue.clear();
}

#[test]
fn busy_and_timeout_errors_retry_with_backoff_while_privacy_failures_are_terminal() {
    let mut attempts = 0;
    let mut hydrated: Vec<String> = Vec::new();
    let mut queue = new_queue(ParticipantAvatarQueueOptions {
        concurrency: 1,
        max_attempts: 3,
        retry_base_ms: 1,
        ..ParticipantAvatarQueueOptions::default()
    });

    queue.subscribe("eventual", "group");
    let requests = queue.next_requests();
    attempts += 1;
    assert_eq!(
        queue.complete(
            &requests[0],
            Err(avatar_error("busy", "media pool busy", true))
        ),
        QueueDecision::RetryAfter {
            participant_id: "eventual".to_string(),
            delay_ms: 1,
        }
    );
    assert!(queue.on_retry_timer("eventual"));

    let requests = queue.next_requests();
    attempts += 1;
    assert_eq!(
        queue.complete(
            &requests[0],
            Err(avatar_error("transport", "backend request timed out", true))
        ),
        QueueDecision::RetryAfter {
            participant_id: "eventual".to_string(),
            delay_ms: 2,
        }
    );
    assert!(queue.on_retry_timer("eventual"));

    let requests = queue.next_requests();
    attempts += 1;
    let decision = queue.complete(&requests[0], Ok("/eventual.jpg".to_string()));
    if let QueueDecision::Hydrated { avatar_path, .. } = &decision {
        hydrated.push(avatar_path.clone());
    }

    assert_eq!(attempts, 3);
    assert_eq!(hydrated, vec!["/eventual.jpg".to_string()]);

    let mut privacy_calls = 0;
    let mut privacy_queue = new_queue(ParticipantAvatarQueueOptions {
        retry_base_ms: 1,
        ..ParticipantAvatarQueueOptions::default()
    });
    privacy_queue.subscribe("private", "group");
    let requests = privacy_queue.next_requests();
    privacy_calls += 1;
    assert_eq!(
        privacy_queue.complete(
            &requests[0],
            Err(avatar_error("privacy", "avatar hidden", false))
        ),
        QueueDecision::TerminalFailure {
            participant_id: "private".to_string(),
        }
    );
    privacy_queue.subscribe("private", "group");
    let requests = privacy_queue.next_requests();
    assert!(requests.is_empty());

    assert_eq!(privacy_calls, 1);
    assert_eq!(privacy_queue.stats().terminal_failures, 1);
}

#[test]
fn retryable_backend_avatar_failures_are_retried_and_never_session_cached() {
    let mut attempts = 0;
    let mut queue = new_queue(ParticipantAvatarQueueOptions {
        concurrency: 1,
        max_attempts: 2,
        retry_base_ms: 1,
        ..ParticipantAvatarQueueOptions::default()
    });

    queue.subscribe("temporary", "group");
    let requests = queue.next_requests();
    attempts += 1;
    assert_eq!(
        queue.complete(
            &requests[0],
            Err(avatar_error(
                "avatar_unavailable",
                "temporary download failure",
                true
            ))
        ),
        QueueDecision::RetryAfter {
            participant_id: "temporary".to_string(),
            delay_ms: 1,
        }
    );
    assert!(queue.on_retry_timer("temporary"));

    let requests = queue.next_requests();
    attempts += 1;
    assert_eq!(
        queue.complete(
            &requests[0],
            Err(avatar_error(
                "avatar_unavailable",
                "temporary download failure",
                true
            ))
        ),
        QueueDecision::Dropped {
            participant_id: "temporary".to_string(),
        }
    );

    assert_eq!(attempts, 2);
    assert_eq!(queue.stats().terminal_failures, 0);
}

#[test]
fn retry_classification_is_restricted_to_retryable_busy_and_timeout_failures() {
    assert!(is_retryable_avatar_error(&avatar_error(
        "busy", "busy", true
    )));
    assert!(is_retryable_avatar_error(&avatar_error(
        "timeout", "timeout", true
    )));
    assert!(is_retryable_avatar_error(&avatar_error(
        "transport",
        "request timed out",
        true
    )));
    assert!(!is_retryable_avatar_error(&avatar_error(
        "transport",
        "backend stopped",
        true
    )));
    assert!(!is_retryable_avatar_error(&avatar_error(
        "privacy", "hidden", false
    )));
}

#[test]
fn exhausted_transient_retries_are_not_session_cached_as_privacy_failures() {
    let mut calls = 0;
    let mut queue = new_queue(ParticipantAvatarQueueOptions {
        concurrency: 1,
        max_attempts: 1,
        retry_base_ms: 0,
        ..ParticipantAvatarQueueOptions::default()
    });

    queue.subscribe("retry-later", "group");
    let requests = queue.next_requests();
    calls += 1;
    assert_eq!(
        queue.complete(&requests[0], Err(avatar_error("busy", "busy", true))),
        QueueDecision::Dropped {
            participant_id: "retry-later".to_string(),
        }
    );

    queue.subscribe("retry-later", "group");
    let requests = queue.next_requests();
    calls += 1;
    queue.complete(&requests[0], Err(avatar_error("busy", "busy", true)));

    assert_eq!(calls, 2);
    assert_eq!(queue.stats().terminal_failures, 0);
}

#[test]
fn terminal_privacy_failures_evict_oldest_entries_at_the_configured_bound() {
    let mut calls: Vec<String> = Vec::new();
    let mut queue = new_queue(ParticipantAvatarQueueOptions {
        max_terminal_failures: 2,
        ..ParticipantAvatarQueueOptions::default()
    });

    for participant_id in ["a", "b", "c"] {
        queue.subscribe(participant_id, "group");
        let requests = queue.next_requests();
        calls.extend(
            requests
                .iter()
                .map(|request| request.participant_id.clone()),
        );
        queue.complete(&requests[0], Err(avatar_error("privacy", "hidden", false)));
    }
    assert_eq!(queue.stats().terminal_failures, 2);

    // `a` was the oldest negative entry, so the eviction makes it eligible
    // for a later request while the still-cached `b` remains suppressed.
    queue.subscribe("a", "group");
    queue.subscribe("b", "group");
    let requests = queue.next_requests();
    calls.extend(
        requests
            .iter()
            .map(|request| request.participant_id.clone()),
    );
    queue.complete(&requests[0], Err(avatar_error("privacy", "hidden", false)));

    assert_eq!(
        calls,
        vec![
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "a".to_string()
        ]
    );
    assert_eq!(queue.stats().terminal_failures, 2);
}
