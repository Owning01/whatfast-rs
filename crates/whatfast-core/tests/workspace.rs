//! Rust port of `tauri/tests/workspace.test.ts` (frozen oracle).
//!
//! Every case of the TypeScript suite is here with the same name translated
//! to snake_case and the same assertions.

use std::collections::{HashMap, HashSet};

use serde_json::json;
use whatfast_core::workspace::{
    Pane, Switcher, close_tab_in_workspace, conversations_to_evict, cycle_switcher,
    move_tab_between_panes, normalize_workspace_snapshot, open_switcher, open_tab,
    recent_chat_candidates, remap_pane_chat_id, same_pane_drop_index, select_tab,
    unique_chat_panes,
};

fn pane(id: &str, tab_chat_ids: &[&str], active_chat_id: &str) -> Pane {
    Pane {
        id: id.to_string(),
        tab_chat_ids: tab_chat_ids.iter().map(|id| id.to_string()).collect(),
        active_chat_id: active_chat_id.to_string(),
    }
}

fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

#[test]
fn opening_a_tab_that_is_already_open_focuses_it_instead_of_duplicating_it() {
    let first = open_tab(&pane("pane-1", &["a"], "a"), "b");
    assert_eq!(
        first,
        Pane {
            id: "pane-1".to_string(),
            tab_chat_ids: ids(&["a", "b"]),
            active_chat_id: "b".to_string(),
        }
    );
    let second = open_tab(&first, "a");
    assert_eq!(
        second,
        Pane {
            id: "pane-1".to_string(),
            tab_chat_ids: ids(&["a", "b"]),
            active_chat_id: "a".to_string(),
        }
    );
}

#[test]
fn selecting_an_already_open_tab_focuses_it_without_touching_the_tab_list() {
    let result = select_tab(&pane("pane-1", &["a", "b"], "a"), "b");
    assert_eq!(result.tab_chat_ids, ids(&["a", "b"]));
    assert_eq!(result.active_chat_id, "b");
}

#[test]
fn selecting_a_new_chat_replaces_the_active_tab_in_place_not_appending() {
    let result = select_tab(&pane("pane-1", &["a", "b"], "a"), "c");
    assert_eq!(result.tab_chat_ids, ids(&["c", "b"]));
    assert_eq!(result.active_chat_id, "c");
}

#[test]
fn closing_the_active_tab_activates_the_right_hand_neighbour() {
    let result = close_tab_in_workspace(&[pane("pane-1", &["a", "b", "c"], "b")], "b", "pane-1");
    assert_eq!(result.removed_pane_id, None);
    assert_eq!(result.panes[0].tab_chat_ids, ids(&["a", "c"]));
    assert_eq!(result.panes[0].active_chat_id, "c");
}

#[test]
fn closing_the_last_tab_in_a_strip_falls_back_to_the_left_hand_neighbour() {
    let result = close_tab_in_workspace(&[pane("pane-1", &["a", "b", "c"], "c")], "c", "pane-1");
    assert_eq!(result.panes[0].tab_chat_ids, ids(&["a", "b"]));
    assert_eq!(result.panes[0].active_chat_id, "b");
}

#[test]
fn closing_an_inactive_tab_leaves_the_active_tab_untouched() {
    let result = close_tab_in_workspace(&[pane("pane-1", &["a", "b", "c"], "b")], "a", "pane-1");
    assert_eq!(result.panes[0].tab_chat_ids, ids(&["b", "c"]));
    assert_eq!(result.panes[0].active_chat_id, "b");
}

#[test]
fn closing_the_last_tab_of_the_second_pane_removes_that_pane() {
    let panes = vec![pane("pane-1", &["a"], "a"), pane("pane-2", &["b"], "b")];
    let result = close_tab_in_workspace(&panes, "b", "pane-2");
    assert_eq!(result.removed_pane_id, Some("pane-2".to_string()));
    assert_eq!(result.panes.len(), 1);
    assert_eq!(result.panes[0].id, "pane-1");
}

#[test]
fn closing_the_last_tab_of_the_only_pane_leaves_an_empty_pane_not_zero_panes() {
    let result = close_tab_in_workspace(&[pane("pane-1", &["a"], "a")], "a", "pane-1");
    assert_eq!(result.removed_pane_id, None);
    assert_eq!(result.panes.len(), 1);
    assert_eq!(
        result.panes[0],
        Pane {
            id: "pane-1".to_string(),
            tab_chat_ids: Vec::new(),
            active_chat_id: String::new(),
        }
    );
}

#[test]
fn every_close_boundary_leaves_a_deterministic_selected_tab_for_focus_restoration() {
    let first = close_tab_in_workspace(&[pane("pane-1", &["a", "b", "c"], "a")], "a", "pane-1");
    assert_eq!(first.panes[0].active_chat_id, "b");

    let middle = close_tab_in_workspace(&[pane("pane-1", &["a", "b", "c"], "b")], "b", "pane-1");
    assert_eq!(middle.panes[0].active_chat_id, "c");

    let last = close_tab_in_workspace(&[pane("pane-1", &["a", "b", "c"], "c")], "c", "pane-1");
    assert_eq!(last.panes[0].active_chat_id, "b");

    let removed = close_tab_in_workspace(
        &[pane("pane-1", &["a"], "a"), pane("pane-2", &["b"], "b")],
        "b",
        "pane-2",
    );
    assert_eq!(removed.panes[0].active_chat_id, "a");
}

#[test]
fn moving_a_tab_to_another_pane_removes_it_from_the_source_and_appends_it_to_the_destination() {
    let panes = vec![
        pane("pane-1", &["a", "b"], "a"),
        pane("pane-2", &["c"], "c"),
    ];
    let moved = move_tab_between_panes(&panes, "b", "pane-1", "pane-2", 1);
    let source = moved.iter().find(|candidate| candidate.id == "pane-1");
    let destination = moved.iter().find(|candidate| candidate.id == "pane-2");
    assert_eq!(
        source.map(|pane| pane.tab_chat_ids.clone()),
        Some(ids(&["a"]))
    );
    assert_eq!(
        destination.map(|pane| pane.tab_chat_ids.clone()),
        Some(ids(&["c", "b"]))
    );
    assert_eq!(
        destination.map(|pane| pane.active_chat_id.clone()),
        Some("b".to_string())
    );
}

#[test]
fn moving_a_tab_within_the_same_pane_reorders_it_to_the_requested_index() {
    let panes = vec![pane("pane-1", &["a", "b", "c"], "a")];
    let moved = move_tab_between_panes(&panes, "c", "pane-1", "pane-1", 0);
    assert_eq!(moved[0].tab_chat_ids, ids(&["c", "a", "b"]));
    assert_eq!(moved[0].active_chat_id, "c");
}

#[test]
fn same_pane_rightward_drops_account_for_removing_the_source_tab() {
    let tabs = ids(&["a", "b", "c"]);
    assert_eq!(same_pane_drop_index(&tabs, "a", 2), 1);
    assert_eq!(same_pane_drop_index(&tabs, "a", 3), 2);
    assert_eq!(same_pane_drop_index(&tabs, "c", 0), 0);
}

#[test]
fn remapping_a_chat_id_updates_a_pane_s_tabs_and_active_chat() {
    let result = remap_pane_chat_id(&pane("pane-1", &["old", "b"], "old"), "old", "new");
    assert_eq!(result.tab_chat_ids, ids(&["new", "b"]));
    assert_eq!(result.active_chat_id, "new");
}

#[test]
fn remapping_onto_a_chat_id_already_open_drops_the_duplicate_instead_of_colliding() {
    let result = remap_pane_chat_id(&pane("pane-1", &["old", "new"], "old"), "old", "new");
    assert_eq!(result.tab_chat_ids, ids(&["new"]));
    assert_eq!(result.active_chat_id, "new");
}

#[test]
fn switcher_initial_highlight_reverse_always_lands_on_the_last_candidate() {
    let switcher = open_switcher(&ids(&["a", "b", "c"]), true, true);
    assert_eq!(switcher.map(|switcher| switcher.highlighted), Some(2));
}

#[test]
fn switcher_initial_highlight_forward_with_the_active_chat_leading_skips_to_index_1() {
    let switcher = open_switcher(&ids(&["a", "b", "c"]), false, true);
    assert_eq!(switcher.map(|switcher| switcher.highlighted), Some(1));
}

#[test]
fn switcher_initial_highlight_forward_with_the_active_chat_not_leading_starts_at_index_0() {
    let switcher = open_switcher(&ids(&["a", "b", "c"]), false, false);
    assert_eq!(switcher.map(|switcher| switcher.highlighted), Some(0));
}

#[test]
fn the_switcher_does_not_open_with_fewer_than_two_candidates() {
    assert_eq!(open_switcher(&ids(&["a"]), false, true), None);
    assert_eq!(open_switcher(&ids(&[]), true, false), None);
}

#[test]
fn cycling_the_switcher_only_moves_the_highlight_wrapping_in_both_directions() {
    let switcher = Switcher {
        chat_ids: ids(&["a", "b", "c"]),
        highlighted: 0,
    };
    assert_eq!(cycle_switcher(&switcher, false).highlighted, 1);
    assert_eq!(cycle_switcher(&switcher, true).highlighted, 2);
    let last = Switcher {
        chat_ids: ids(&["a", "b", "c"]),
        highlighted: 2,
    };
    assert_eq!(cycle_switcher(&last, false).highlighted, 0);
}

#[test]
fn candidate_seeding_falls_back_to_the_visible_chat_list_until_session_history_has_two_entries() {
    let candidates = recent_chat_candidates(&[], &|_| true, &ids(&["v1", "v2", "v3"]), "");
    assert_eq!(candidates, ids(&["v1", "v2", "v3"]));
}

#[test]
fn candidate_seeding_stops_once_session_history_already_has_two_or_more_entries() {
    let candidates = recent_chat_candidates(
        &ids(&["r1", "r2"]),
        &|_| true,
        &ids(&["v1", "v2", "v3"]),
        "",
    );
    assert_eq!(candidates, ids(&["r1", "r2"]));
}

#[test]
fn candidate_seeding_drops_history_entries_for_chats_that_no_longer_exist() {
    let candidates = recent_chat_candidates(
        &ids(&["gone", "r1"]),
        &|chat_id: &str| chat_id != "gone",
        &ids(&["v1", "v2"]),
        "",
    );
    assert_eq!(candidates, ids(&["r1", "v1", "v2"]));
}

#[test]
fn the_active_chat_is_swapped_to_the_front_of_the_candidate_list_when_already_present() {
    let candidates = recent_chat_candidates(&ids(&["r1", "r2", "r3"]), &|_| true, &[], "r3");
    assert_eq!(candidates, ids(&["r3", "r2", "r1"]));
}

#[test]
fn the_active_chat_is_inserted_at_the_front_when_it_is_not_yet_in_session_history() {
    let candidates = recent_chat_candidates(&ids(&["r1", "r2"]), &|_| true, &[], "active");
    assert_eq!(candidates, ids(&["active", "r1", "r2"]));
}

#[test]
fn conversations_no_longer_open_in_any_pane_are_evicted_first() {
    let evicted = conversations_to_evict(
        &ids(&["a", "b", "c"]),
        &HashSet::from(["a".to_string(), "c".to_string()]),
        &HashMap::new(),
        8,
        &HashSet::new(),
    );
    assert_eq!(evicted, ids(&["b"]));
}

#[test]
fn once_every_hydrated_chat_is_still_open_the_cap_evicts_the_least_recently_focused() {
    let last_focused_at = HashMap::from([
        ("a".to_string(), 100),
        ("b".to_string(), 300),
        ("c".to_string(), 200),
    ]);
    let evicted = conversations_to_evict(
        &ids(&["a", "b", "c"]),
        &HashSet::from(["a".to_string(), "b".to_string(), "c".to_string()]),
        &last_focused_at,
        2,
        &HashSet::new(),
    );
    assert_eq!(evicted, ids(&["a"]));
}

#[test]
fn conversation_eviction_never_drops_a_chat_currently_visible_in_a_pane() {
    let last_focused_at = HashMap::from([
        ("visible".to_string(), 1),
        ("inactive-old".to_string(), 2),
        ("inactive-new".to_string(), 3),
    ]);
    let evicted = conversations_to_evict(
        &ids(&["visible", "inactive-old", "inactive-new"]),
        &HashSet::from([
            "visible".to_string(),
            "inactive-old".to_string(),
            "inactive-new".to_string(),
        ]),
        &last_focused_at,
        2,
        &HashSet::from(["visible".to_string()]),
    );
    assert_eq!(evicted, ids(&["inactive-old"]));
}

#[test]
fn normalizing_a_workspace_snapshot_rejects_untrusted_shapes() {
    assert_eq!(normalize_workspace_snapshot(&json!(null)), None);
    assert_eq!(normalize_workspace_snapshot(&json!({})), None);
    assert_eq!(normalize_workspace_snapshot(&json!({ "panes": [] })), None);
    assert_eq!(
        normalize_workspace_snapshot(&json!({ "panes": [{}, {}, {}] })),
        None
    );
    assert_eq!(
        normalize_workspace_snapshot(
            &json!({ "panes": [{ "id": "pane-1", "tabChatIds": "not-an-array" }] })
        ),
        None
    );
    assert_eq!(
        normalize_workspace_snapshot(&json!({ "panes": [{ "id": "x" }, { "id": "x" }] })),
        None
    );
}

#[test]
fn normalizing_a_workspace_snapshot_accepts_a_well_formed_payload_and_repairs_a_dangling_active_id()
{
    let snapshot = normalize_workspace_snapshot(&json!({
        "panes": [{ "id": "pane-1", "tabChatIds": ["a", "b"], "activeChatId": "not-in-list" }],
        "focusedPaneId": "pane-1",
    }));
    assert_eq!(
        snapshot.as_ref().map(|snapshot| snapshot.panes[0].clone()),
        Some(Pane {
            id: "pane-1".to_string(),
            tab_chat_ids: ids(&["a", "b"]),
            active_chat_id: "a".to_string(),
        })
    );
    assert_eq!(
        snapshot
            .as_ref()
            .map(|snapshot| snapshot.focused_pane_id.clone()),
        Some("pane-1".to_string())
    );
}

#[test]
fn normalizing_a_workspace_gives_each_chat_one_pane_local_viewport() {
    assert_eq!(
        normalize_workspace_snapshot(&json!({
            "panes": [
                { "id": "pane-1", "tabChatIds": ["shared", "left"], "activeChatId": "shared" },
                { "id": "pane-2", "tabChatIds": ["shared", "right"], "activeChatId": "shared" },
            ],
            "focusedPaneId": "pane-2",
        })),
        Some(whatfast_core::workspace::WorkspaceSnapshot {
            panes: vec![
                Pane {
                    id: "pane-1".to_string(),
                    tab_chat_ids: ids(&["shared", "left"]),
                    active_chat_id: "shared".to_string(),
                },
                Pane {
                    id: "pane-2".to_string(),
                    tab_chat_ids: ids(&["right"]),
                    active_chat_id: "right".to_string(),
                },
            ],
            focused_pane_id: "pane-2".to_string(),
        })
    );
}

#[test]
fn chat_merge_deduplicates_a_canonical_chat_across_panes() {
    assert_eq!(
        unique_chat_panes(&[
            Pane {
                id: "pane-1".to_string(),
                tab_chat_ids: ids(&["new", "left"]),
                active_chat_id: "new".to_string(),
            },
            Pane {
                id: "pane-2".to_string(),
                tab_chat_ids: ids(&["new", "right"]),
                active_chat_id: "new".to_string(),
            },
        ]),
        vec![
            Pane {
                id: "pane-1".to_string(),
                tab_chat_ids: ids(&["new", "left"]),
                active_chat_id: "new".to_string(),
            },
            Pane {
                id: "pane-2".to_string(),
                tab_chat_ids: ids(&["right"]),
                active_chat_id: "right".to_string(),
            },
        ]
    );
}
