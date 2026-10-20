//! Rust port of `tauri/tests/tab-keyboard.test.ts` (frozen oracle).
//!
//! Every case of the TypeScript suite is here with the same name translated
//! to snake_case and the same assertions.

use whatfast_core::tab_keyboard::{
    TabDirection, TabKeyboardCommand, TabKeyboardInput, tab_keyboard_command,
};
use whatfast_core::workspace::Pane;

fn panes() -> Vec<Pane> {
    vec![
        Pane {
            id: "pane-1".to_string(),
            tab_chat_ids: vec!["a".to_string(), "b".to_string(), "c".to_string()],
            active_chat_id: "b".to_string(),
        },
        Pane {
            id: "pane-2".to_string(),
            tab_chat_ids: vec!["d".to_string()],
            active_chat_id: "d".to_string(),
        },
    ]
}

fn gesture(key: &str, alt_key: bool, shift_key: bool, platform_modifier: bool) -> TabKeyboardInput {
    TabKeyboardInput {
        key: key.to_string(),
        alt_key,
        shift_key,
        platform_modifier,
    }
}

#[test]
fn alt_shift_arrow_reorders_within_a_pane() {
    assert_eq!(
        tab_keyboard_command(
            &gesture("ArrowLeft", true, true, false),
            &panes(),
            "pane-1",
            "b"
        ),
        Some(TabKeyboardCommand::Reorder {
            index: 0,
            direction: TabDirection::Left,
        })
    );
    assert_eq!(
        tab_keyboard_command(
            &gesture("ArrowRight", true, true, false),
            &panes(),
            "pane-1",
            "b"
        ),
        Some(TabKeyboardCommand::Reorder {
            index: 2,
            direction: TabDirection::Right,
        })
    );
}

#[test]
fn reorder_reports_first_and_last_tab_boundaries() {
    assert_eq!(
        tab_keyboard_command(
            &gesture("ArrowLeft", true, true, false),
            &panes(),
            "pane-1",
            "a"
        ),
        Some(TabKeyboardCommand::Boundary {
            message: "Tab is already first in this pane".to_string(),
        })
    );
    assert_eq!(
        tab_keyboard_command(
            &gesture("ArrowRight", true, true, false),
            &panes(),
            "pane-1",
            "c"
        ),
        Some(TabKeyboardCommand::Boundary {
            message: "Tab is already last in this pane".to_string(),
        })
    );
}

#[test]
fn control_shift_arrow_moves_to_an_adjacent_pane_at_a_bounded_index() {
    assert_eq!(
        tab_keyboard_command(
            &gesture("ArrowRight", false, true, true),
            &panes(),
            "pane-1",
            "c"
        ),
        Some(TabKeyboardCommand::MovePane {
            pane_id: "pane-2".to_string(),
            index: 1,
            direction: TabDirection::Right,
        })
    );
    assert_eq!(
        tab_keyboard_command(
            &gesture("ArrowLeft", false, true, true),
            &panes(),
            "pane-2",
            "d"
        ),
        Some(TabKeyboardCommand::MovePane {
            pane_id: "pane-1".to_string(),
            index: 0,
            direction: TabDirection::Left,
        })
    );
}

#[test]
fn cross_pane_movement_reports_both_outer_boundaries() {
    assert_eq!(
        tab_keyboard_command(
            &gesture("ArrowLeft", false, true, true),
            &panes(),
            "pane-1",
            "a"
        ),
        Some(TabKeyboardCommand::Boundary {
            message: "There is no pane to the left".to_string(),
        })
    );
    assert_eq!(
        tab_keyboard_command(
            &gesture("ArrowRight", false, true, true),
            &panes(),
            "pane-2",
            "d"
        ),
        Some(TabKeyboardCommand::Boundary {
            message: "There is no pane to the right".to_string(),
        })
    );
}

#[test]
fn delete_closes_while_modified_delete_and_unrelated_keys_pass_through() {
    assert_eq!(
        tab_keyboard_command(
            &gesture("Delete", false, false, false),
            &panes(),
            "pane-1",
            "b"
        ),
        Some(TabKeyboardCommand::Close)
    );
    assert_eq!(
        tab_keyboard_command(
            &gesture("Delete", false, false, true),
            &panes(),
            "pane-1",
            "b"
        ),
        None
    );
    assert_eq!(
        tab_keyboard_command(
            &gesture("Enter", false, false, false),
            &panes(),
            "pane-1",
            "b"
        ),
        None
    );
}
