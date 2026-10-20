//! Port of `tauri/src/state/tab-keyboard.ts` (SolidJS frontend, frozen oracle).
//!
//! Translates the tab-strip keyboard gestures into commands without touching
//! the DOM.

use crate::workspace::Pane;

/// Which way a tab moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabDirection {
    Left,
    Right,
}

/// What a tab-strip gesture asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TabKeyboardCommand {
    /// Move the tab inside its own pane.
    Reorder {
        index: usize,
        direction: TabDirection,
    },
    /// Move the tab to the adjacent pane.
    MovePane {
        pane_id: String,
        index: usize,
        direction: TabDirection,
    },
    /// Close the tab.
    Close,
    /// The gesture has nowhere to go.
    Boundary { message: String },
}

/// The gesture that was pressed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabKeyboardInput {
    pub key: String,
    pub alt_key: bool,
    pub shift_key: bool,
    pub platform_modifier: bool,
}

/// Translate tab-strip keyboard gestures without touching the DOM.
pub fn tab_keyboard_command(
    input: &TabKeyboardInput,
    panes: &[Pane],
    pane_id: &str,
    chat_id: &str,
) -> Option<TabKeyboardCommand> {
    let pane_index = panes.iter().position(|pane| pane.id == pane_id)?;
    let pane = &panes[pane_index];
    let tab_index = pane.tab_chat_ids.iter().position(|id| id == chat_id)?;

    if input.key == "Delete" && !input.alt_key && !input.platform_modifier {
        return Some(TabKeyboardCommand::Close);
    }

    if input.alt_key && input.shift_key && !input.platform_modifier {
        if input.key != "ArrowLeft" && input.key != "ArrowRight" {
            return None;
        }
        let delta = if input.key == "ArrowLeft" { -1 } else { 1 };
        let next_index = tab_index as i64 + delta;
        if next_index < 0 || next_index >= pane.tab_chat_ids.len() as i64 {
            return Some(TabKeyboardCommand::Boundary {
                message: format!(
                    "Tab is already {} in this pane",
                    if delta < 0 { "first" } else { "last" }
                ),
            });
        }
        return Some(TabKeyboardCommand::Reorder {
            index: next_index as usize,
            direction: if delta < 0 {
                TabDirection::Left
            } else {
                TabDirection::Right
            },
        });
    }

    if input.platform_modifier && input.shift_key && !input.alt_key {
        if input.key != "ArrowLeft" && input.key != "ArrowRight" {
            return None;
        }
        let delta = if input.key == "ArrowLeft" { -1 } else { 1 };
        let target_index = pane_index as i64 + delta;
        let target_pane = if target_index < 0 {
            None
        } else {
            panes.get(target_index as usize)
        };
        let Some(target_pane) = target_pane else {
            return Some(TabKeyboardCommand::Boundary {
                message: format!(
                    "There is no pane to the {}",
                    if delta < 0 { "left" } else { "right" }
                ),
            });
        };
        return Some(TabKeyboardCommand::MovePane {
            pane_id: target_pane.id.clone(),
            index: tab_index.min(target_pane.tab_chat_ids.len()),
            direction: if delta < 0 {
                TabDirection::Left
            } else {
                TabDirection::Right
            },
        });
    }

    None
}
