//! Rust port of `tauri/tests/sync.test.ts` (frozen oracle).
//!
//! Every case of the TypeScript suite is here with the same name translated
//! to snake_case and the same assertions.
//!
//! port: the suite builds a whole `SyncStatus` of the bridge; the port
//! builds the `SyncStatusView` the module reads, the six fields the
//! original declares locally.

use whatfast_core::sync::{
    SyncLocale, SyncPhase, SyncStatusView, empty_conversation_sync_message,
    should_apply_sync_status, sync_status_active, sync_status_label,
};

fn status(phase: SyncPhase, revision: u32) -> SyncStatusView {
    SyncStatusView {
        phase,
        revision,
        messages_processed: 345,
        whats_app_progress: 64,
        completed_at_ms: 0,
        detail: String::new(),
    }
}

#[test]
fn only_active_transfer_phases_render_as_syncing() {
    for phase in [
        SyncPhase::Connecting,
        SyncPhase::InitialHistory,
        SyncPhase::AppState,
        SyncPhase::CatchingUp,
    ] {
        assert!(sync_status_active(&status(phase, 1)));
    }
    for phase in [
        SyncPhase::NotStarted,
        SyncPhase::Complete,
        SyncPhase::Partial,
        SyncPhase::Failed,
        SyncPhase::Offline,
    ] {
        assert!(!sync_status_active(&status(phase, 1)));
    }
}

#[test]
fn stale_snapshots_cannot_overwrite_newer_sync_events() {
    assert!(!should_apply_sync_status(
        &status(SyncPhase::Partial, 5),
        &status(SyncPhase::Connecting, 4)
    ));
    assert!(should_apply_sync_status(
        &status(SyncPhase::Partial, 5),
        &status(SyncPhase::Complete, 6)
    ));
}

#[test]
fn partial_and_failed_states_remain_visible_and_actionable() {
    let mut partial = status(SyncPhase::Partial, 1);
    partial.detail = "WhatsApp has not finished sending initial chat history".to_string();
    partial.messages_processed = 10;
    assert_eq!(
        sync_status_label(&partial, SyncLocale::EnUs, 0),
        format!("Sync incomplete \u{b7} {}", partial.detail)
    );
    assert_eq!(
        sync_status_label(&status(SyncPhase::Failed, 1), SyncLocale::EnUs, 0),
        "Sync failed"
    );
    assert_eq!(
        sync_status_label(&status(SyncPhase::Offline, 1), SyncLocale::EnUs, 0),
        "Offline \u{b7} sync will resume when connected"
    );
}

#[test]
fn an_empty_conversation_explains_incomplete_account_history() {
    let syncing = empty_conversation_sync_message(&status(SyncPhase::InitialHistory, 1));
    assert!(syncing.unwrap_or_default().contains("still syncing"));

    let mut partial = status(SyncPhase::Partial, 1);
    partial.detail = "WhatsApp has not finished sending initial chat history".to_string();
    partial.messages_processed = 0;
    partial.whats_app_progress = 0;
    assert!(
        empty_conversation_sync_message(&partial)
            .unwrap_or_default()
            .contains("was not received")
    );
    assert!(sync_status_label(&partial, SyncLocale::EnUs, 0).contains("relink this device"));
    assert_eq!(
        empty_conversation_sync_message(&status(SyncPhase::Complete, 1)),
        None
    );
}
