//! The drafts an agent proposed for the conversation on screen.
//!
//! The screen of the agents lists every draft of the profile. This module
//! draws the ones of the open chat over the composer, so the user answers them
//! without leaving the conversation: a band with the text, the agent that
//! wrote it and the reason, and three buttons to send it, to edit it or to
//! throw it away.
//!
//! The band reads `agents.db` and keeps nothing of its own: a database that
//! does not open leaves the conversation as it was, without a band.

use std::time::Duration;

use gpui::{
    Context, ElementId, FontWeight, ParentElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use whatfast_auto::{Db, Draft, DraftState};
use whatfast_i18n as i18n;

use crate::agents_panel::{database_path, draft_preview, open_database};
use crate::app::Whatfast;

/// How many drafts of one chat the band lists before it counts the rest.
pub(crate) const MAX_ROWS: usize = 3;

/// How often the band reads `agents.db` again while a chat is open.
pub(crate) const REFRESH: Duration = Duration::from_secs(3);

/// How many drafts of each state the band asks the database for.
///
/// The band only shows the ones of the open chat, but the query is the same
/// the tray of the agents runs: fifty rows of every profile is a cheap read
/// and covers the chats the user moves between.
const DRAFT_LIMIT: u32 = 50;

/// The proposed drafts of one chat, newest first.
///
/// `None` when no chat is open: the band belongs to a conversation, so there
/// is nothing to read without one.
pub(crate) fn drafts_of(chat_id: Option<&str>) -> Vec<Draft> {
    let Some(chat_id) = chat_id else {
        return Vec::new();
    };
    let Ok(db) = open_database(&database_path()) else {
        return Vec::new();
    };
    read(&db, chat_id)
}

/// The proposed drafts of one chat, read from one open database.
///
/// A failed read is an empty band and not an error of the conversation: the
/// drafts come from another process, and the messages of the chat are the
/// ones that matter.
pub(crate) fn read(db: &Db, chat_id: &str) -> Vec<Draft> {
    match db.drafts(DraftState::Proposed, DRAFT_LIMIT) {
        Ok(rows) => of_chat(rows, chat_id),
        Err(_) => Vec::new(),
    }
}

/// The drafts of one chat, in the order the database gave them.
pub(crate) fn of_chat(rows: Vec<Draft>, chat_id: &str) -> Vec<Draft> {
    rows.into_iter()
        .filter(|draft| draft.chat_id == chat_id)
        .collect()
}

/// How many drafts of the chat the band does not list.
pub(crate) fn hidden(count: usize) -> usize {
    count.saturating_sub(MAX_ROWS)
}

/// Sends one draft to its chat and takes it off the band.
///
/// The request, the permission, the state of the row and the timeout are the
/// ones of [`Whatfast::send_agent_draft`]. The row leaves the band right away:
/// the answer of the backend closes it in `agents.db`.
pub(crate) fn send(app: &mut Whatfast, draft_id: i64, chat_id: String, text: String) {
    app.send_agent_draft(draft_id, chat_id, text);
    forget(app, draft_id);
}

/// Puts the text of a draft in the composer and drops the draft.
///
/// The row goes to `discarded` so it does not come back in the next read: the
/// user took the text, and the draft that proposed it has no reason to wait
/// any more. The user sends the edited text with the button of the composer.
pub(crate) fn edit(
    app: &mut Whatfast,
    draft_id: i64,
    text: String,
    window: &mut Window,
    cx: &mut Context<Whatfast>,
) {
    app.apply_agent_answer(text, window, cx);
    discard(app, draft_id);
}

/// Throws one draft away, in the database and in the band.
pub(crate) fn cancel(app: &mut Whatfast, draft_id: i64) {
    discard(app, draft_id);
}

/// Marks the draft as discarded and takes it off the band.
fn discard(app: &mut Whatfast, draft_id: i64) {
    mark_discarded(app, draft_id);
    forget(app, draft_id);
}

/// Writes `discarded` in `agents.db`.
///
/// A failure reaches the user as a toast: the row stays `proposed` and the
/// next read puts it back, which is better than hiding the error.
fn mark_discarded(app: &mut Whatfast, draft_id: i64) {
    let error = match open_database(&database_path()) {
        Ok(db) => match db.set_draft_state(draft_id, DraftState::Discarded) {
            Ok(_) => return,
            Err(error) => error.to_string(),
        },
        Err(error) => error,
    };
    app.store.toast_error = Some(i18n::t_args(
        "The draft could not be discarded: {error}",
        &[("error", error.as_str())],
    ));
}

/// Takes one draft off the list of the band.
fn forget(app: &mut Whatfast, draft_id: i64) {
    app.agent_drafts.retain(|draft| draft.id != draft_id);
}

/// Draws the band of the drafts of the open chat, over the composer.
///
/// The caller draws nothing when [`Whatfast::agent_drafts`] is empty, so the
/// footer of the conversation keeps its height for a chat without drafts.
pub(crate) fn render(app: &Whatfast, cx: &mut Context<Whatfast>) -> gpui::Div {
    let scale = app.ui_scale;
    let muted = cx.theme().muted_foreground;
    let mut band = v_flex()
        .w_full()
        .flex_shrink_0()
        .border_t_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().popover);
    for (index, draft) in app.agent_drafts.iter().take(MAX_ROWS).enumerate() {
        band = band.child(render_row(draft, index, app, cx));
    }
    let hidden = hidden(app.agent_drafts.len());
    if hidden > 0 {
        let count = hidden.to_string();
        band = band.child(
            div()
                .px(px(12. * scale))
                .py(px(4. * scale))
                .text_xs()
                .text_color(muted)
                .child(i18n::t_args(
                    "+{count} more in the Agents tray",
                    &[("count", &count)],
                )),
        );
    }
    band
}

/// One row of the band: the text, the agent with its reason, and the buttons.
fn render_row(
    draft: &Draft,
    index: usize,
    app: &Whatfast,
    cx: &mut Context<Whatfast>,
) -> gpui::Div {
    let scale = app.ui_scale;
    let muted = cx.theme().muted_foreground;
    let accent = cx.theme().accent;
    let agent = whatfast_agents::by_id(&draft.agent_id)
        .map(|spec| spec.name)
        .unwrap_or(draft.agent_id.as_str());
    let draft_id = draft.id;
    let chat_id = draft.chat_id.clone();
    let text = draft.text.clone();
    let edit_text = draft.text.clone();
    h_flex()
        .w_full()
        .gap(px(8. * scale))
        .items_start()
        .px(px(12. * scale))
        .py(px(8. * scale))
        .when(index > 0, |row| {
            row.border_t_1().border_color(cx.theme().border)
        })
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(2. * scale))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child(draft_preview(&draft.text)),
                )
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .flex_none()
                                .text_xs()
                                .text_color(accent)
                                .child(i18n::t_args("Proposed by {agent}", &[("agent", agent)])),
                        )
                        .when_some(draft.reason.clone(), |line, reason| {
                            line.child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(i18n::t_args("Why: {reason}", &[("reason", &reason)])),
                            )
                        }),
                ),
        )
        .child(
            h_flex()
                .flex_none()
                .gap_1()
                .items_center()
                .child(
                    Button::new(ElementId::named_usize("chat-agent-draft-send", index))
                        .primary()
                        .small()
                        .label(i18n::t("Send"))
                        .on_click(cx.listener(move |app, _, _, cx| {
                            send(app, draft_id, chat_id.clone(), text.clone());
                            cx.notify();
                        })),
                )
                .child(
                    Button::new(ElementId::named_usize("chat-agent-draft-edit", index))
                        .secondary()
                        .small()
                        .label(i18n::t("Edit"))
                        .on_click(cx.listener(move |app, _, window, cx| {
                            edit(app, draft_id, edit_text.clone(), window, cx);
                            cx.notify();
                        })),
                )
                .child(
                    Button::new(ElementId::named_usize("chat-agent-draft-cancel", index))
                        .secondary()
                        .small()
                        .label(i18n::t("Cancel"))
                        .on_click(cx.listener(move |app, _, _, cx| {
                            cancel(app, draft_id);
                            cx.notify();
                        })),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One draft of a chat, as the database keeps it.
    fn draft(chat_id: &str, text: &str, state: DraftState) -> Draft {
        Draft {
            id: 0,
            agent_id: "claude-code".into(),
            chat_id: chat_id.into(),
            reply_to: None,
            text: text.into(),
            reason: None,
            confidence: None,
            state,
            at_ms: 0,
        }
    }

    /// A database of one profile, in a folder of its own.
    fn database() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let db = Db::open(&dir.path().join("agents.db")).expect("the database of the agents");
        (dir, db)
    }

    #[test]
    fn the_band_lists_only_the_drafts_of_the_open_chat() {
        let (_dir, db) = database();
        db.add_draft(&draft("chat-a", "first", DraftState::Proposed))
            .expect("the first draft");
        db.add_draft(&draft("chat-b", "other", DraftState::Proposed))
            .expect("the draft of another chat");
        db.add_draft(&draft("chat-a", "second", DraftState::Proposed))
            .expect("the second draft");
        db.add_draft(&draft("chat-a", "gone", DraftState::Discarded))
            .expect("the discarded draft");
        let texts: Vec<String> = read(&db, "chat-a")
            .into_iter()
            .map(|draft| draft.text)
            .collect();
        // The newest first, as the database orders the rows, and neither the
        // other chat nor the draft the user already discarded.
        assert_eq!(texts, vec!["second".to_string(), "first".to_string()]);
    }

    #[test]
    fn a_chat_without_drafts_leaves_the_band_empty() {
        let (_dir, db) = database();
        db.add_draft(&draft("chat-a", "first", DraftState::Proposed))
            .expect("the draft");
        assert!(read(&db, "chat-b").is_empty());
        assert!(drafts_of(None).is_empty());
    }

    #[test]
    fn the_band_counts_the_drafts_that_do_not_fit() {
        assert_eq!(hidden(0), 0);
        assert_eq!(hidden(MAX_ROWS), 0);
        assert_eq!(hidden(MAX_ROWS + 2), 2);
    }

    #[test]
    fn the_order_of_the_rows_survives_the_filter() {
        let rows = vec![
            draft("chat-a", "newest", DraftState::Proposed),
            draft("chat-b", "other", DraftState::Proposed),
            draft("chat-a", "oldest", DraftState::Proposed),
        ];
        let texts: Vec<String> = of_chat(rows, "chat-a")
            .into_iter()
            .map(|draft| draft.text)
            .collect();
        assert_eq!(texts, vec!["newest".to_string(), "oldest".to_string()]);
    }
}
