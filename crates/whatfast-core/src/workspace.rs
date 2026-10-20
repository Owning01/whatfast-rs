//! Port of `tauri/src/state/workspace.ts` (SolidJS frontend, frozen oracle).
//!
//! Panes, tab strips, the chat switcher, conversation eviction and the
//! session snapshot. Everything is a plain function over plain data.
//!
//! Fidelity notes (the odd bits are deliberate, they are the oracle's):
//! - `closeTab` falls back to the left-hand neighbour only when the closed tab
//!   was the first one of the strip; otherwise the right-hand neighbour wins.
//! - `conversationsToEvict` evicts the chats that are not open in any pane
//!   first, and only then the least recently focused survivors. The number of
//!   evicted survivors is computed from *all* the survivors, not from the
//!   ones that are not protected, so a protected chat can leave the window
//!   above the cap.
//! - `normalizeWorkspaceSnapshot` throws away any payload it cannot trust and
//!   repairs the ones it can: dangling active chat ids, duplicated chat ids
//!   across panes and unknown pane shapes.
//! - `readWorkspaceSnapshot` / `writeWorkspaceSnapshot` read and write
//!   `sessionStorage`. The pure part (parse, normalize, serialize) lives here;
//!   the storage effect belongs to the UI crate.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Session history caps at this many chats, matching the GPUI reference app.
pub const MAX_RECENT_CHATS: usize = 10;

/// `sessionStorage` key holding the workspace snapshot.
pub const WORKSPACE_STORAGE_KEY: &str = "whatfast-workspace";

/// A pane of the split view.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pane {
    /// Stable identifier, currently always "pane-1" or "pane-2".
    pub id: String,
    /// Tab order within the pane.
    pub tab_chat_ids: Vec<String>,
    /// Which tab is showing; empty when the pane has no tabs.
    pub active_chat_id: String,
}

/// The Ctrl+Tab switcher state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Switcher {
    pub chat_ids: Vec<String>,
    pub highlighted: usize,
}

/// The persisted shape of the workspace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSnapshot {
    pub panes: Vec<Pane>,
    pub focused_pane_id: String,
}

/// The outcome of closing a tab inside the workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloseTabResult {
    pub panes: Vec<Pane>,
    /// Set when closing the tab emptied its pane and that pane was removed.
    pub removed_pane_id: Option<String>,
}

/// A pane with no tabs.
pub fn empty_pane(id: &str) -> Pane {
    Pane {
        id: id.to_string(),
        tab_chat_ids: Vec::new(),
        active_chat_id: String::new(),
    }
}

/// The pane that owns the viewport of a chat.
///
/// A chat owns one pane-local viewport. Keeping it unique across panes avoids
/// one exact-message navigation replacing another pane's shared message window.
pub fn pane_containing_chat(panes: &[Pane], chat_id: &str) -> Option<Pane> {
    panes
        .iter()
        .find(|pane| pane.tab_chat_ids.iter().any(|id| id == chat_id))
        .cloned()
}

/// Repair legacy or merged workspaces so one chat cannot drive two independent
/// pane viewports through one shared conversation window. First pane wins.
pub fn unique_chat_panes(panes: &[Pane]) -> Vec<Pane> {
    let mut seen_chats: HashSet<String> = HashSet::new();
    panes
        .iter()
        .map(|pane| {
            let tab_chat_ids: Vec<String> = pane
                .tab_chat_ids
                .iter()
                .filter(|chat_id| seen_chats.insert((*chat_id).clone()))
                .cloned()
                .collect();
            let active_chat_id = if tab_chat_ids.iter().any(|id| id == &pane.active_chat_id) {
                pane.active_chat_id.clone()
            } else {
                tab_chat_ids.first().cloned().unwrap_or_default()
            };
            Pane {
                id: pane.id.clone(),
                tab_chat_ids,
                active_chat_id,
            }
        })
        .collect()
}

/// Load a chat into a pane's currently active tab slot. A chat already open
/// somewhere in this pane is merely focused, matching browser tab behaviour;
/// otherwise the active slot is replaced in place so that clicking through a
/// chat list does not pile up tabs the way deliberately opening a new tab
/// ([`open_tab`]) does.
pub fn select_tab(pane: &Pane, chat_id: &str) -> Pane {
    if pane.tab_chat_ids.iter().any(|id| id == chat_id) {
        return Pane {
            active_chat_id: chat_id.to_string(),
            ..pane.clone()
        };
    }
    let active_index = pane
        .tab_chat_ids
        .iter()
        .position(|id| id == &pane.active_chat_id);
    match active_index {
        None => Pane {
            tab_chat_ids: [pane.tab_chat_ids.clone(), vec![chat_id.to_string()]].concat(),
            active_chat_id: chat_id.to_string(),
            ..pane.clone()
        },
        Some(index) => {
            let mut tab_chat_ids = pane.tab_chat_ids.clone();
            tab_chat_ids[index] = chat_id.to_string();
            Pane {
                tab_chat_ids,
                active_chat_id: chat_id.to_string(),
                ..pane.clone()
            }
        }
    }
}

/// Open a chat as its own tab: focuses an existing tab rather than duplicating it.
pub fn open_tab(pane: &Pane, chat_id: &str) -> Pane {
    if pane.tab_chat_ids.iter().any(|id| id == chat_id) {
        return Pane {
            active_chat_id: chat_id.to_string(),
            ..pane.clone()
        };
    }
    Pane {
        tab_chat_ids: [pane.tab_chat_ids.clone(), vec![chat_id.to_string()]].concat(),
        active_chat_id: chat_id.to_string(),
        ..pane.clone()
    }
}

/// Close a tab. When the closed tab was active, the neighbour to its right
/// takes over, or failing that the neighbour to its left — ordinary browser
/// tab-strip behaviour. An empty `active_chat_id` means the pane now has no tabs.
pub fn close_tab(pane: &Pane, chat_id: &str) -> Pane {
    let Some(index) = pane.tab_chat_ids.iter().position(|id| id == chat_id) else {
        return pane.clone();
    };
    let tab_chat_ids: Vec<String> = pane
        .tab_chat_ids
        .iter()
        .filter(|id| id.as_str() != chat_id)
        .cloned()
        .collect();
    if pane.active_chat_id != chat_id {
        return Pane {
            tab_chat_ids,
            ..pane.clone()
        };
    }
    let neighbour = tab_chat_ids
        .get(index)
        .or_else(|| {
            if index > 0 {
                tab_chat_ids.get(index - 1)
            } else {
                None
            }
        })
        .cloned()
        .unwrap_or_default();
    Pane {
        tab_chat_ids,
        active_chat_id: neighbour,
        ..pane.clone()
    }
}

/// Insert (or relocate) a tab at a specific index, then activate it.
fn place_tab(pane: &Pane, chat_id: &str, index: i64) -> Pane {
    let mut without_chat: Vec<String> = pane
        .tab_chat_ids
        .iter()
        .filter(|id| id.as_str() != chat_id)
        .cloned()
        .collect();
    let clamped = index.clamp(0, without_chat.len() as i64) as usize;
    without_chat.insert(clamped, chat_id.to_string());
    Pane {
        tab_chat_ids: without_chat,
        active_chat_id: chat_id.to_string(),
        ..pane.clone()
    }
}

/// Move a tab between panes (or reorder it within the same pane) to a target
/// index. Operates over the whole pane list because a cross-pane move touches
/// two panes at once; panes not involved pass through unchanged.
pub fn move_tab_between_panes(
    panes: &[Pane],
    chat_id: &str,
    from_pane_id: &str,
    to_pane_id: &str,
    index: i64,
) -> Vec<Pane> {
    if from_pane_id == to_pane_id {
        return panes
            .iter()
            .map(|pane| {
                if pane.id == from_pane_id {
                    place_tab(pane, chat_id, index)
                } else {
                    pane.clone()
                }
            })
            .collect();
    }
    panes
        .iter()
        .map(|pane| {
            if pane.id == from_pane_id {
                close_tab(pane, chat_id)
            } else if pane.id == to_pane_id {
                place_tab(pane, chat_id, index)
            } else {
                pane.clone()
            }
        })
        .collect()
}

/// Convert a DOM drop boundary into the insertion index after removing the dragged tab.
pub fn same_pane_drop_index(tab_chat_ids: &[String], chat_id: &str, drop_boundary: i64) -> i64 {
    let source_index = tab_chat_ids.iter().position(|id| id == chat_id);
    match source_index {
        Some(index) if (index as i64) < drop_boundary => drop_boundary - 1,
        _ => drop_boundary,
    }
}

/// Close a tab within a specific pane and, per the redesign contract, drop
/// that pane entirely when it was the last tab and another pane still exists —
/// a lone empty pane is kept (it renders the empty-conversation screen)
/// because the workspace must never drop to zero panes.
pub fn close_tab_in_workspace(panes: &[Pane], chat_id: &str, pane_id: &str) -> CloseTabResult {
    let Some(pane_index) = panes.iter().position(|pane| pane.id == pane_id) else {
        return CloseTabResult {
            panes: panes.to_vec(),
            removed_pane_id: None,
        };
    };
    let pane = &panes[pane_index];
    if !pane.tab_chat_ids.iter().any(|id| id == chat_id) {
        return CloseTabResult {
            panes: panes.to_vec(),
            removed_pane_id: None,
        };
    }
    let closed = close_tab(pane, chat_id);
    if closed.tab_chat_ids.is_empty() && panes.len() > 1 {
        return CloseTabResult {
            panes: panes
                .iter()
                .filter(|candidate| candidate.id != pane_id)
                .cloned()
                .collect(),
            removed_pane_id: Some(pane_id.to_string()),
        };
    }
    CloseTabResult {
        panes: panes
            .iter()
            .map(|candidate| {
                if candidate.id == pane_id {
                    closed.clone()
                } else {
                    candidate.clone()
                }
            })
            .collect(),
        removed_pane_id: None,
    }
}

/// Remap a chat id inside one pane's tabs, used when the backend merges two chat ids.
pub fn remap_pane_chat_id(pane: &Pane, old_id: &str, new_id: &str) -> Pane {
    if !pane.tab_chat_ids.iter().any(|id| id == old_id) {
        return pane.clone();
    }
    let has_new = pane.tab_chat_ids.iter().any(|id| id == new_id);
    let tab_chat_ids: Vec<String> = if has_new {
        pane.tab_chat_ids
            .iter()
            .filter(|id| id.as_str() != old_id)
            .cloned()
            .collect()
    } else {
        pane.tab_chat_ids
            .iter()
            .map(|id| {
                if id == old_id {
                    new_id.to_string()
                } else {
                    id.clone()
                }
            })
            .collect()
    };
    let active_chat_id = if pane.active_chat_id == old_id {
        new_id.to_string()
    } else {
        pane.active_chat_id.clone()
    };
    Pane {
        tab_chat_ids,
        active_chat_id,
        ..pane.clone()
    }
}

/// Session-recency candidates for the Ctrl+Tab switcher, matching the GPUI
/// desktop app's `recent_chat_candidates`: seed from the visible (already
/// recency-ordered) chat list until enough session history has accumulated,
/// then guarantee the active chat leads the list.
pub fn recent_chat_candidates(
    recent_chat_ids: &[String],
    chat_exists: &impl Fn(&str) -> bool,
    visible_chat_ids: &[String],
    selected_chat_id: &str,
) -> Vec<String> {
    let mut candidates: Vec<String> = recent_chat_ids
        .iter()
        .filter(|id| chat_exists(id))
        .cloned()
        .collect();
    if candidates.len() < 2 {
        for chat_id in visible_chat_ids {
            if !candidates.iter().any(|id| id == chat_id) {
                candidates.push(chat_id.clone());
            }
            if candidates.len() >= MAX_RECENT_CHATS {
                break;
            }
        }
    }
    // port: `if (selectedChatId)` — an empty id is falsy in JS and skips the
    // whole block.
    if !selected_chat_id.is_empty() {
        match candidates.iter().position(|id| id == selected_chat_id) {
            Some(index) => {
                // Swap into the lead position (rather than remove-and-reinsert) to
                // match the GPUI reference implementation exactly.
                candidates.swap(0, index);
            }
            None => {
                if chat_exists(selected_chat_id) {
                    candidates.insert(0, selected_chat_id.to_string());
                }
            }
        }
    }
    candidates.truncate(MAX_RECENT_CHATS);
    candidates
}

/// Build the switcher's initial state, or `None` when there is nothing to
/// switch to. Initial highlight: reverse selects the last candidate; forward
/// selects index 1 when the active chat already leads the list (skipping past
/// it), otherwise index 0.
pub fn open_switcher(
    candidate_chat_ids: &[String],
    reverse: bool,
    selected_is_first: bool,
) -> Option<Switcher> {
    if candidate_chat_ids.len() < 2 {
        return None;
    }
    let highlighted = if reverse {
        candidate_chat_ids.len() - 1
    } else if selected_is_first {
        1
    } else {
        0
    };
    Some(Switcher {
        chat_ids: candidate_chat_ids.to_vec(),
        highlighted,
    })
}

/// Move the switcher's highlight only; it never changes which chat is open.
pub fn cycle_switcher(switcher: &Switcher, reverse: bool) -> Switcher {
    let length = switcher.chat_ids.len();
    if length == 0 {
        return switcher.clone();
    }
    let highlighted = if reverse {
        switcher
            .highlighted
            .saturating_add(length)
            .saturating_sub(1)
            % length
    } else {
        switcher.highlighted.saturating_add(1) % length
    };
    Switcher {
        chat_ids: switcher.chat_ids.clone(),
        highlighted,
    }
}

/// Choose which hydrated conversations to drop after a pane/tab mutation:
/// first anything no longer open in any pane's tab list, then — once the
/// survivors still exceed the cap — the least recently focused of the rest.
pub fn conversations_to_evict(
    hydrated_chat_ids: &[String],
    open_chat_ids: &HashSet<String>,
    last_focused_at: &HashMap<String, i64>,
    max_hydrated: usize,
    protected_chat_ids: &HashSet<String>,
) -> Vec<String> {
    let mut evicted: Vec<String> = Vec::new();
    let mut survivors: Vec<String> = Vec::new();
    for chat_id in hydrated_chat_ids {
        if open_chat_ids.contains(chat_id) {
            survivors.push(chat_id.clone());
        } else {
            evicted.push(chat_id.clone());
        }
    }
    if survivors.len() > max_hydrated {
        let mut ordered: Vec<&String> = survivors
            .iter()
            .filter(|chat_id| !protected_chat_ids.contains(*chat_id))
            .collect();
        ordered.sort_by(|left, right| {
            let left_focus = last_focused_at.get(*left).copied().unwrap_or(0);
            let right_focus = last_focused_at.get(*right).copied().unwrap_or(0);
            left_focus.cmp(&right_focus)
        });
        // port: the slice length comes from *all* the survivors, not from the
        // unprotected ones, so a protected chat leaves the window above the cap.
        let count = survivors.len().saturating_sub(max_hydrated);
        evicted.extend(ordered.into_iter().take(count).cloned());
    }
    evicted
}

/// Validate an untrusted parsed value read back from `sessionStorage` into a
/// usable workspace snapshot, or `None` when it cannot be trusted. Never
/// assume the shape survived a hand edit, an old app version, or storage
/// shared between tabs.
pub fn normalize_workspace_snapshot(value: &Value) -> Option<WorkspaceSnapshot> {
    let candidate = value.as_object()?;
    let panes_value = candidate.get("panes")?.as_array()?;
    if panes_value.is_empty() || panes_value.len() > 2 {
        return None;
    }
    let mut panes: Vec<Pane> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    for raw_pane in panes_value {
        let pane = normalize_pane(raw_pane)?;
        if !seen_ids.insert(pane.id.clone()) {
            return None;
        }
        panes.push(pane);
    }
    let unique_panes = unique_chat_panes(&panes);
    let focused_pane_id = candidate
        .get("focusedPaneId")
        .and_then(Value::as_str)
        .filter(|focused| unique_panes.iter().any(|pane| &pane.id == focused))
        .map(str::to_string)
        .or_else(|| unique_panes.first().map(|pane| pane.id.clone()))
        .unwrap_or_default();
    Some(WorkspaceSnapshot {
        panes: unique_panes,
        focused_pane_id,
    })
}

/// Validate one pane of an untrusted snapshot.
fn normalize_pane(value: &Value) -> Option<Pane> {
    let candidate = value.as_object()?;
    let id = candidate.get("id").and_then(Value::as_str)?;
    if id.is_empty() {
        return None;
    }
    let tab_chat_ids_value = candidate.get("tabChatIds")?;
    let items = tab_chat_ids_value.as_array()?;
    let mut seen: HashSet<String> = HashSet::new();
    let mut tab_chat_ids: Vec<String> = Vec::new();
    for item in items {
        let Some(item_id) = item.as_str() else {
            continue;
        };
        if item_id.is_empty() {
            continue;
        }
        if seen.insert(item_id.to_string()) {
            tab_chat_ids.push(item_id.to_string());
        }
    }
    let active_chat_id = candidate
        .get("activeChatId")
        .and_then(Value::as_str)
        .filter(|active| tab_chat_ids.iter().any(|id| id == active))
        .map(str::to_string)
        .or_else(|| tab_chat_ids.first().cloned())
        .unwrap_or_default();
    Some(Pane {
        id: id.to_string(),
        tab_chat_ids,
        active_chat_id,
    })
}

/// Read a workspace snapshot out of the raw JSON stored in `sessionStorage`.
///
/// port: the original wraps `JSON.parse` in a `try`, a broken payload yields
/// `undefined` instead of throwing.
pub fn parse_workspace_snapshot(raw: &str) -> Option<WorkspaceSnapshot> {
    let parsed: Value = serde_json::from_str(raw).ok()?;
    normalize_workspace_snapshot(&parsed)
}

/// Serialize a workspace snapshot for `sessionStorage`.
pub fn serialize_workspace_snapshot(snapshot: &WorkspaceSnapshot) -> String {
    // port: `JSON.stringify` of a plain object never throws.
    serde_json::to_string(snapshot).unwrap_or_default()
}
