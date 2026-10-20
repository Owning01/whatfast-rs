//! Rust port of `tauri/tests/scroll-restoration.test.ts` (frozen oracle).
//!
//! Every case of the TypeScript suite is here with the same name translated
//! to snake_case and the same assertions.

use whatfast_core::scroll_restoration::{
    ScrollRestoreTarget, ScrollSnapshot, VirtualRow, capture_scroll_snapshot,
    resolve_scroll_restore,
};

fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

fn row(index: usize, start: f64, end: f64) -> VirtualRow {
    VirtualRow { index, start, end }
}

#[test]
fn captures_a_stable_first_visible_message_anchor_instead_of_a_raw_scroll_offset() {
    let snapshot = capture_scroll_snapshot(
        &ids(&["a", "b", "c"]),
        &[row(0, 0.0, 80.0), row(1, 80.0, 180.0), row(2, 180.0, 260.0)],
        115.0,
        600.0,
        200.0,
        80.0,
    );
    assert_eq!(
        snapshot,
        Some(ScrollSnapshot {
            anchor_message_id: "b".to_string(),
            anchor_offset: -35.0,
            at_latest: false,
        })
    );
}

#[test]
fn marks_a_viewport_near_the_bottom_as_following_latest() {
    let snapshot = capture_scroll_snapshot(
        &ids(&["a"]),
        &[row(0, 500.0, 600.0)],
        350.0,
        600.0,
        200.0,
        80.0,
    );
    assert_eq!(snapshot.map(|snapshot| snapshot.at_latest), Some(true));
}

#[test]
fn restores_an_existing_anchor_and_preserves_its_viewport_offset() {
    assert_eq!(
        resolve_scroll_restore(
            &ids(&["older", "a", "b", "c"]),
            Some(&ScrollSnapshot {
                anchor_message_id: "b".to_string(),
                anchor_offset: -24.0,
                at_latest: false,
            }),
            ""
        ),
        ScrollRestoreTarget::Anchor {
            index: 2,
            offset: -24.0,
        }
    );
}

#[test]
fn a_viewport_that_followed_latest_returns_to_the_newest_message() {
    assert_eq!(
        resolve_scroll_restore(
            &ids(&["a", "b"]),
            Some(&ScrollSnapshot {
                anchor_message_id: "a".to_string(),
                anchor_offset: 0.0,
                at_latest: true,
            }),
            "a"
        ),
        ScrollRestoreTarget::Latest { index: 1 }
    );
}

#[test]
fn a_first_visit_prefers_the_unread_boundary_and_otherwise_uses_latest() {
    assert_eq!(
        resolve_scroll_restore(&ids(&["a", "b", "c"]), None, "b"),
        ScrollRestoreTarget::Unread { index: 1 }
    );
    assert_eq!(
        resolve_scroll_restore(&ids(&["a", "b", "c"]), None, "missing"),
        ScrollRestoreTarget::Latest { index: 2 }
    );
}

#[test]
fn a_trimmed_away_anchor_falls_back_to_the_unread_boundary() {
    assert_eq!(
        resolve_scroll_restore(
            &ids(&["b", "c"]),
            Some(&ScrollSnapshot {
                anchor_message_id: "trimmed".to_string(),
                anchor_offset: -10.0,
                at_latest: false,
            }),
            "b"
        ),
        ScrollRestoreTarget::Unread { index: 0 }
    );
}
