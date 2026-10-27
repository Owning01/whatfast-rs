//! The fast mode of the chat: one question about the conversation, put to an
//! agent that writes without a window.
//!
//! The answer never leaves the composer: the backend asks the harness and
//! brings the text back, and the user decides what to do with it. That is why
//! this module holds a small card and not a modal: the conversation stays
//! readable behind it, because the question is written while the messages that
//! answer it are in front.
//!
//! The agents that can answer are the connected ones that have a headless
//! mode: a harness that only lives inside a window cannot write a paragraph on
//! demand, so it is not offered.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement as _,
    ParentElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    v_flex,
};
use whatfast_i18n as i18n;
use whatfast_model::model::message_text;
use whatfast_protocol::Message;

use crate::agents_panel::{database_path, open_database};
use crate::app::Whatfast;

/// How many messages of the conversation go to the agent.
pub(crate) const CONTEXT_MESSAGES: usize = 60;

/// The longest context that goes to the agent, in characters.
pub(crate) const CONTEXT_CHARS: usize = 6_000;

/// The width of the card, before the scale of the screen.
pub(crate) const CARD_WIDTH: f32 = 420.0;

/// One agent of the profile that answers without a window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AskAgent {
    /// The id of the harness: `claude-code`, `codex`.
    pub(crate) id: String,
    /// The name the selector shows.
    pub(crate) name: &'static str,
}

/// The connected agents of the profile that have a headless mode.
///
/// The question goes to `agents.db`, so the caller runs this when a screen
/// opens and not on every frame.
pub(crate) fn headless_agents() -> Vec<AskAgent> {
    let Ok(db) = open_database(&database_path()) else {
        return Vec::new();
    };
    let Ok(rows) = db.agents() else {
        return Vec::new();
    };
    rows.iter()
        .filter(|row| row.live())
        .filter_map(|row| {
            let spec = whatfast_agents::by_id(&row.id)?;
            spec.run.as_ref()?;
            Some(AskAgent {
                id: row.id.clone(),
                name: spec.name,
            })
        })
        .collect()
}

/// The lines of context of one conversation, oldest first.
///
/// The agent reads the newest messages and not the whole history: sixty
/// messages and six thousand characters are what the prompt of a question can
/// carry without pushing the answer out of the window of the harness.
pub(crate) fn question_context(messages: &[Message]) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut used = 0usize;
    for message in messages.iter().rev().take(CONTEXT_MESSAGES) {
        // A message without content says nothing to the agent: the row is a
        // placeholder that the model would read as text.
        if message.content.is_none() {
            continue;
        }
        let text = message_text(message);
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            continue;
        }
        let sender = if message.from_me {
            i18n::t("You").to_string()
        } else if message.sender_name.trim().is_empty() {
            i18n::t("Unknown contact").to_string()
        } else {
            message.sender_name.trim().to_string()
        };
        let line = format!("{sender}: {text}");
        let size = line.chars().count();
        // The newest line that does not fit ends the context: a half line of
        // a long message would read as a different message.
        if used + size > CONTEXT_CHARS && !lines.is_empty() {
            break;
        }
        used += size;
        lines.push(line);
    }
    lines.reverse();
    lines
}

/// What the card is waiting for.
#[derive(Default)]
struct QuickAskState {
    /// The chat of the question, when the card is open.
    chat_id: Option<String>,
    /// The agents that can answer.
    agents: Vec<AskAgent>,
    /// The agent the user picked.
    agent: Option<String>,
    /// The chat of the question in flight, when there is one.
    pending: Option<String>,
    /// The line of the last error.
    error: Option<String>,
}

/// The card of the fast mode and its one field of text.
pub(crate) struct QuickAsk {
    /// The state of the card.
    state: Rc<RefCell<QuickAskState>>,
    /// The question the user is typing.
    input: Entity<InputState>,
}

impl QuickAsk {
    /// Creates the card with its field.
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Whatfast>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(i18n::t("Ask about this chat"))
                .clean_on_escape()
        });
        Self {
            state: Rc::new(RefCell::new(QuickAskState::default())),
            input,
        }
    }

    /// The field of the question, so the app can listen for the return key.
    pub(crate) fn input(&self) -> &Entity<InputState> {
        &self.input
    }

    /// Whether the card is on the screen.
    pub(crate) fn is_open(&self) -> bool {
        self.state.borrow().chat_id.is_some()
    }

    /// Whether the card waits for the answer of an agent.
    pub(crate) fn is_waiting(&self) -> bool {
        self.state.borrow().pending.is_some()
    }

    /// Opens the card over one conversation.
    pub(crate) fn open(&self, chat_id: String, window: &mut Window, cx: &mut Context<Whatfast>) {
        let agents = headless_agents();
        {
            let mut state = self.state.borrow_mut();
            state.chat_id = Some(chat_id);
            state.error = None;
            state.pending = None;
            // The agent of the last question stays while it is still
            // connected; otherwise the card lands on the first one.
            if !agents
                .iter()
                .any(|agent| Some(&agent.id) == state.agent.as_ref())
            {
                state.agent = agents.first().map(|agent| agent.id.clone());
            }
            state.agents = agents;
        }
        self.input.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
    }

    /// Closes the card and forgets what it held.
    pub(crate) fn close(&self) {
        let mut state = self.state.borrow_mut();
        state.chat_id = None;
        state.pending = None;
        state.error = None;
    }

    /// Puts one agent in front of the question.
    pub(crate) fn select(&self, id: &str) {
        let mut state = self.state.borrow_mut();
        if state.agents.iter().any(|agent| agent.id == id) {
            state.agent = Some(id.to_string());
            state.error = None;
        }
    }

    /// The text of the field.
    pub(crate) fn question(&self, cx: &mut Context<Whatfast>) -> String {
        self.input.read(cx).value().trim().to_string()
    }

    /// The question that is ready to go to the backend.
    ///
    /// `None` when the card is closed, when no agent can answer or when the
    /// field is empty.
    pub(crate) fn take_question(
        &self,
        cx: &mut Context<Whatfast>,
    ) -> Option<(String, String, String)> {
        let (chat_id, agent) = {
            let state = self.state.borrow();
            if state.pending.is_some() {
                return None;
            }
            (state.chat_id.clone()?, state.agent.clone()?)
        };
        let question = self.question(cx);
        if question.is_empty() {
            return None;
        }
        self.state.borrow_mut().pending = Some(chat_id.clone());
        Some((agent, chat_id, question))
    }

    /// Records that the question of one chat is on its way.
    pub(crate) fn mark_pending(&self, chat_id: &str) {
        let mut state = self.state.borrow_mut();
        state.pending = Some(chat_id.to_string());
        state.error = None;
    }

    /// Takes the answer of the backend and closes the card.
    ///
    /// `None` when the card is not waiting for anything: the answer of a
    /// question that the user cancelled belongs to nobody.
    pub(crate) fn take_answer(&self, text: String) -> Option<(String, String)> {
        let mut state = self.state.borrow_mut();
        let chat_id = state.pending.take()?;
        state.chat_id = None;
        state.error = None;
        Some((chat_id, text))
    }

    /// Shows the reason of a question that got no answer.
    pub(crate) fn fail(&self, message: String) {
        let mut state = self.state.borrow_mut();
        state.pending = None;
        state.error = Some(message);
    }

    /// Discards the wait without closing the card.
    ///
    /// The answer that comes later is thrown away: the user said it is not
    /// wanted any more.
    pub(crate) fn cancel(&self) {
        let mut state = self.state.borrow_mut();
        state.pending = None;
        state.error = None;
    }

    /// Draws the card over the conversation.
    pub(crate) fn render(
        &self,
        app: &Whatfast,
        cx: &mut Context<Whatfast>,
    ) -> gpui::Stateful<gpui::Div> {
        let scale = app.ui_scale;
        let danger = cx.theme().danger;
        let accent = cx.theme().accent;
        let muted = cx.theme().muted_foreground;
        let (agents, agent, waiting, error) = {
            let state = self.state.borrow();
            (
                state.agents.clone(),
                state.agent.clone(),
                state.pending.is_some(),
                state.error.clone(),
            )
        };
        let typed = self.input.read(cx).value().trim().to_string();
        let mut card = v_flex()
            .id("quick-ask")
            .absolute()
            .top(px(120. * scale))
            .right(px(20. * scale))
            .w(px(CARD_WIDTH * scale))
            .gap_2()
            .p(px(12. * scale))
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .occlude()
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(i18n::t("Ask an agent")),
                    )
                    .child(
                        Button::new("quick-ask-close")
                            .secondary()
                            .small()
                            .label("×")
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.close_quick_ask(window, cx);
                                cx.notify();
                            })),
                    ),
            );
        if agents.is_empty() {
            return card
                .child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child(i18n::t("Connect an agent first.")),
                )
                .when_some(error, |this, message| {
                    this.child(div().text_xs().text_color(danger).child(message))
                });
        }
        if agents.len() > 1 {
            card = card.child(h_flex().w_full().gap_1().items_center().children(
                agents.into_iter().enumerate().map(|(index, choice)| {
                    let selected = agent.as_deref() == Some(choice.id.as_str());
                    let own_id = choice.id.clone();
                    let button =
                        Button::new(gpui::ElementId::named_usize("quick-ask-agent", index))
                            .small()
                            .label(choice.name)
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.quick_ask.select(&own_id);
                                cx.notify();
                            }));
                    if selected {
                        button.primary()
                    } else {
                        button.secondary()
                    }
                    .into_any_element()
                }),
            ));
        }
        card = card
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&self.input).small()),
                    )
                    .child(
                        Button::new("quick-ask-send")
                            .primary()
                            .small()
                            .label(i18n::t("Ask"))
                            .disabled(waiting || typed.is_empty())
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.submit_quick_ask(cx);
                                cx.notify();
                            })),
                    ),
            )
            .when(waiting, |this| {
                this.child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(accent)
                                .child(i18n::t("Thinking…")),
                        )
                        .child(
                            Button::new("quick-ask-cancel")
                                .secondary()
                                .small()
                                .label(i18n::t("Cancel"))
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.quick_ask.cancel();
                                    cx.notify();
                                })),
                        ),
                )
            })
            .when_some(error, |this, message| {
                this.child(div().text_xs().text_color(danger).child(message))
            });
        card
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use whatfast_protocol::{TextContent, message};

    /// One text message of the conversation.
    fn text_message(sender: &str, from_me: bool, text: &str) -> Message {
        Message {
            id: format!("{sender}-{text}"),
            chat_id: "chat".into(),
            sender_name: sender.into(),
            from_me,
            content: Some(message::Content::Text(TextContent {
                text: text.into(),
                ..TextContent::default()
            })),
            ..Message::default()
        }
    }

    #[test]
    fn the_context_names_the_sender_of_every_message() {
        let messages = vec![
            text_message("Ana", false, "hola"),
            text_message("me", true, "que tal"),
        ];
        let context = question_context(&messages);
        assert_eq!(
            context,
            vec!["Ana: hola".to_string(), "You: que tal".to_string()]
        );
    }

    #[test]
    fn the_context_keeps_the_newest_messages() {
        let messages: Vec<Message> = (0..CONTEXT_MESSAGES + 10)
            .map(|index| text_message("Ana", false, &format!("m{index}")))
            .collect();
        let context = question_context(&messages);
        assert_eq!(context.len(), CONTEXT_MESSAGES);
        // Oldest first, and the oldest of the kept ones is the tenth.
        assert!(context.first().is_some_and(|line| line.ends_with("m10")));
        assert!(context.last().is_some_and(|line| line.ends_with("m69")));
    }

    #[test]
    fn the_context_stops_at_the_character_limit() {
        let long = "a".repeat(1_000);
        let messages: Vec<Message> = (0..20).map(|_| text_message("Ana", false, &long)).collect();
        let context = question_context(&messages);
        let total: usize = context.iter().map(|line| line.chars().count()).sum();
        assert!(total <= CONTEXT_CHARS);
        assert_eq!(context.len(), CONTEXT_CHARS / 1_005);
    }

    #[test]
    fn a_message_without_text_is_left_out() {
        let empty = Message {
            id: "empty".into(),
            chat_id: "chat".into(),
            ..Message::default()
        };
        let context = question_context(&[empty, text_message("Ana", false, "hola")]);
        assert_eq!(context, vec!["Ana: hola".to_string()]);
    }
}
