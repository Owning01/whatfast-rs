//! The automations of the profile: the tasks of the timer.
//!
//! A task is a prompt, a chat and a turn: when the turn comes, the backend asks
//! the harness of the task without a window and does with the answer what the
//! task says. The screen lives under the tray of the drafts, because both are
//! the work the agents do while the user is not looking at them.
//!
//! The turns belong to the backend: it knows the offset of the zone of the
//! machine and the window of hours of each task. The screen only parses the one
//! instant the user typed for a task that runs once, and it parses it with the
//! same offset of the system that the rest of the interface writes with.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    AppContext as _, Context, ElementId, Entity, FontWeight, InteractiveElement as _,
    IntoElement as _, ParentElement as _, StatefulInteractiveElement as _, Styled as _, Window,
    div, prelude::FluentBuilder as _, px, uniform_list,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    v_flex,
};
use whatfast_i18n as i18n;
use whatfast_model::model::ChatFilter as StoreFilter;
use whatfast_protocol::{
    Schedule as WireSchedule, ScheduleListRequest, SchedulePauseRequest, SchedulePauseResponse,
    SchedulePutRequest, SchedulePutResponse, ScheduleRemoveRequest, ScheduleRemoveResponse,
    rpc_request,
};

use crate::agents_panel::{now_ms, time_label};
use crate::app::Whatfast;
use crate::local_time::local_offset_minutes;
use crate::quick_ask::{AskAgent, headless_agents};
use crate::rpc::PendingRequest;

/// The height of one row of the list of chats, before the scale.
pub(crate) const CHAT_ROW_HEIGHT: f32 = 26.0;

/// The height of the list of chats of the form, before the scale.
pub(crate) const CHAT_LIST_HEIGHT: f32 = 132.0;

/// The longest preview of a prompt, in characters.
pub(crate) const PREVIEW_CHARS: usize = 120;

/// The window of hours of a new task: the prudent one of the profile.
pub(crate) const WINDOW_DEFAULT: &str = "08:00-23:00";

/// The example of the field of the cron.
pub(crate) const CRON_PLACEHOLDER: &str = "0 9 * * *";

/// The example of the field of the instant of a task that runs once.
pub(crate) const AT_PLACEHOLDER: &str = "2026-10-26 09:00";

/// How many chats the form offers when the history of the window is empty.
pub(crate) const MAX_FORM_CHATS: usize = 40;

/// Whether a task runs once or every so often.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum TaskKind {
    /// One instant.
    #[default]
    Once,
    /// A cron of five fields.
    Every,
}

impl TaskKind {
    /// The name the backend reads.
    fn wire(self) -> &'static str {
        match self {
            Self::Once => "once",
            Self::Every => "cron",
        }
    }
}

/// What a task does with the answer of the agent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum TaskOutput {
    /// It only reads, for a summary.
    Read,
    /// It leaves a draft for the user.
    #[default]
    Draft,
    /// It sends the answer.
    Send,
}

impl TaskOutput {
    /// The name the backend reads.
    fn wire(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Draft => "draft",
            Self::Send => "send",
        }
    }
}

/// The values the fields of the form hold.
pub(crate) struct FormValues {
    /// The expression of a task that runs every so often.
    pub(crate) cron: String,
    /// The instant of a task that runs once, as the user typed it.
    pub(crate) at: String,
    /// The prompt of the task.
    pub(crate) prompt: String,
    /// The window of hours.
    pub(crate) hours: String,
}

/// The state of the screen of the automations.
#[derive(Default)]
struct AutomationsState {
    /// The tasks of the profile.
    schedules: Vec<WireSchedule>,
    /// The agents that can run a task.
    agents: Vec<AskAgent>,
    /// The line of the result of the last action.
    notice: Option<String>,
    /// The line of the last error.
    error: Option<String>,
    /// Whether the form of a new task is open.
    form_open: bool,
    /// The agent of the task being written.
    agent: Option<String>,
    /// The chat of the task being written.
    chat: Option<String>,
    /// The kind of the task being written.
    kind: TaskKind,
    /// The output of the task being written.
    output: TaskOutput,
    /// Whether a task is on its way to the backend.
    saving: bool,
}

/// The screen of the automations and the form of a new task.
pub(crate) struct AutomationsPanel {
    /// The state of the screen.
    state: Rc<RefCell<AutomationsState>>,
    /// The expression of a task that runs every so often.
    cron: Entity<InputState>,
    /// The instant of a task that runs once.
    at: Entity<InputState>,
    /// The prompt of the task.
    prompt: Entity<InputState>,
    /// The window of hours of the task.
    hours: Entity<InputState>,
}

impl AutomationsPanel {
    /// Creates the screen with the four fields of the form.
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Whatfast>) -> Self {
        let cron = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(CRON_PLACEHOLDER)
                .clean_on_escape()
        });
        let at = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(AT_PLACEHOLDER)
                .clean_on_escape()
        });
        let prompt = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(i18n::t("What the agent should do"))
                // The prompt is a paragraph: it grows with the text and the
                // return key writes a new line instead of saving.
                .auto_grow(2, 6)
        });
        let hours = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(WINDOW_DEFAULT)
                .clean_on_escape()
        });
        // A task with no window fires at any hour: the field starts with the
        // prudent one because that is what a user who does not think about it
        // wants.
        hours.update(cx, |input, cx| input.set_value(WINDOW_DEFAULT, window, cx));
        Self {
            state: Rc::new(RefCell::new(AutomationsState::default())),
            cron,
            at,
            prompt,
            hours,
        }
    }

    /// Puts the answer of the backend on the screen.
    pub(crate) fn set_schedules(&self, schedules: Vec<WireSchedule>) {
        let mut state = self.state.borrow_mut();
        state.schedules = schedules;
        state.error = None;
    }

    /// Puts the agents that can run a task on the screen.
    pub(crate) fn set_agents(&self, agents: Vec<AskAgent>) {
        let mut state = self.state.borrow_mut();
        state.agents = agents;
    }

    /// Sets the line of the last error.
    pub(crate) fn fail(&self, message: String) {
        let mut state = self.state.borrow_mut();
        state.error = Some(message);
        state.notice = None;
        state.saving = false;
    }

    /// Reads the values of the fields of the form.
    pub(crate) fn values(&self, cx: &mut Context<Whatfast>) -> FormValues {
        FormValues {
            cron: self.cron.read(cx).value().trim().to_string(),
            at: self.at.read(cx).value().trim().to_string(),
            prompt: self.prompt.read(cx).value().trim().to_string(),
            hours: self.hours.read(cx).value().trim().to_string(),
        }
    }

    /// The agent, the chat, the kind and the output the form holds.
    fn choices(&self) -> (Option<String>, Option<String>, TaskKind, TaskOutput) {
        let state = self.state.borrow();
        (
            state.agent.clone(),
            state.chat.clone(),
            state.kind,
            state.output,
        )
    }

    /// Records that a task is on its way to the backend.
    pub(crate) fn mark_saving(&self) {
        let mut state = self.state.borrow_mut();
        state.saving = true;
        state.error = None;
        state.notice = None;
    }

    /// Whether a task is on its way to the backend.
    fn is_saving(&self) -> bool {
        self.state.borrow().saving
    }

    /// Opens or closes the form of a new task.
    ///
    /// `fallback` is the list of the agents that can run a task, for the first
    /// time the form opens before the screen read the profile.
    pub(crate) fn toggle_form(&self, fallback: Vec<AskAgent>, default_chat: Option<String>) {
        let mut state = self.state.borrow_mut();
        state.form_open = !state.form_open;
        if !state.form_open {
            return;
        }
        state.error = None;
        state.notice = None;
        if state.agents.is_empty() {
            state.agents = fallback;
        }
        if !state
            .agents
            .iter()
            .any(|agent| Some(&agent.id) == state.agent.as_ref())
        {
            state.agent = state.agents.first().map(|agent| agent.id.clone());
        }
        if state.chat.is_none() {
            state.chat = default_chat;
        }
    }

    /// Closes the form and empties the prompt of the task that was saved.
    pub(crate) fn close_form(&self, window: &mut Window, cx: &mut Context<Whatfast>) {
        {
            let mut state = self.state.borrow_mut();
            state.form_open = false;
            state.saving = false;
        }
        self.prompt
            .update(cx, |input, cx| input.set_value("", window, cx));
    }

    /// Takes the answer of one save.
    ///
    /// Returns whether the task was stored, so the caller reloads the list.
    pub(crate) fn take_put(&self, response: &SchedulePutResponse) -> bool {
        {
            let mut state = self.state.borrow_mut();
            state.saving = false;
            if !response.error.is_empty() {
                state.error = Some(i18n::t_args(
                    "The task could not be saved: {error}",
                    &[("error", &response.error)],
                ));
                state.notice = None;
                return false;
            }
            state.error = None;
            state.notice = Some(i18n::t("The task is saved.").to_string());
        }
        true
    }

    /// Takes the answer of one pause.
    ///
    /// Returns whether the list changed, so the caller reads it again.
    pub(crate) fn take_pause(&self, response: &SchedulePauseResponse) -> bool {
        {
            let mut state = self.state.borrow_mut();
            if !response.error.is_empty() {
                state.error = Some(i18n::t_args(
                    "The task could not be paused: {error}",
                    &[("error", &response.error)],
                ));
                state.notice = None;
                return false;
            }
            let paused = response
                .schedule
                .as_ref()
                .is_some_and(|schedule| schedule.paused);
            state.notice = Some(if paused {
                i18n::t("The task is paused.").to_string()
            } else {
                i18n::t("The task runs again.").to_string()
            });
            state.error = None;
        }
        true
    }

    /// Takes the answer of one removal.
    pub(crate) fn take_remove(&self, response: &ScheduleRemoveResponse) {
        let mut state = self.state.borrow_mut();
        if response.removed {
            state.notice = Some(i18n::t("The task is removed.").to_string());
            state.error = None;
        } else {
            state.error = Some(i18n::t("That task is not in the profile anymore.").to_string());
            state.notice = None;
        }
    }

    /// Shows the error of a request of this screen.
    ///
    /// Returns whether the request belonged to the screen.
    pub(crate) fn take_error(&self, pending: &PendingRequest, message: &str) -> bool {
        let key = match pending {
            PendingRequest::Schedules => "The automations could not be read: {error}",
            PendingRequest::SchedulePut => "The task could not be saved: {error}",
            PendingRequest::SchedulePause { .. } => "The task could not be paused: {error}",
            PendingRequest::ScheduleRemove { .. } => "The task could not be removed: {error}",
            _ => return false,
        };
        self.fail(i18n::t_args(key, &[("error", message)]));
        true
    }

    /// Draws the section: its head, the form and the tasks.
    pub(crate) fn render(&self, app: &Whatfast, cx: &mut Context<Whatfast>) -> gpui::Div {
        let scale = app.ui_scale;
        let muted = cx.theme().muted_foreground;
        let danger = cx.theme().danger;
        let accent = cx.theme().accent;
        let (tasks, notice, error, open) = {
            let state = self.state.borrow();
            (
                state.schedules.clone(),
                state.notice.clone(),
                state.error.clone(),
                state.form_open,
            )
        };
        let counting = tasks.len().to_string();
        let mut section = v_flex()
            .w_full()
            .gap_2()
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(16. * scale))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(i18n::t("Automations")),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(i18n::t_args("{count} tasks", &[("count", &counting)])),
                            ),
                    )
                    .child(
                        Button::new("automation-new")
                            .secondary()
                            .small()
                            .label(i18n::t("New task"))
                            .on_click(cx.listener(|app, _, _, cx| {
                                // The form needs the agents that can run a task:
                                // the screen keeps the list it read when it
                                // opened, and the question to the profile is
                                // only asked when that list is empty.
                                let fallback =
                                    if app.automations_panel.state.borrow().agents.is_empty() {
                                        headless_agents()
                                    } else {
                                        Vec::new()
                                    };
                                let chat = app.store.selected_chat_id.clone();
                                app.automations_panel.toggle_form(fallback, chat);
                                cx.notify();
                            })),
                    ),
            )
            .when_some(error, |this, message| {
                this.child(div().text_xs().text_color(danger).child(message))
            })
            .when_some(notice, |this, message| {
                this.child(div().text_xs().text_color(accent).child(message))
            });
        if open {
            section = section.child(self.render_form(app, cx));
        }
        if tasks.is_empty() {
            return section.child(
                div()
                    .w_full()
                    .p(px(12. * scale))
                    .rounded_md()
                    .border_1()
                    .border_color(cx.theme().border)
                    .text_sm()
                    .text_color(muted)
                    .child(i18n::t(
                        "No task yet. A task asks the agent on its own, at the hour you set.",
                    )),
            );
        }
        let now = now_ms();
        for (index, task) in tasks.iter().enumerate() {
            section = section.child(self.render_task(task, index, now, app, cx));
        }
        section
    }

    /// The form of a new task.
    fn render_form(&self, app: &Whatfast, cx: &mut Context<Whatfast>) -> gpui::Div {
        let scale = app.ui_scale;
        let muted = cx.theme().muted_foreground;
        let (agents, agent, kind, output) = {
            let state = self.state.borrow();
            (
                state.agents.clone(),
                state.agent.clone(),
                state.kind,
                state.output,
            )
        };
        let saving = self.is_saving();
        let mut form = v_flex()
            .w_full()
            .gap_2()
            .p(px(12. * scale))
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .child(render_label("Agent", muted));
        if agents.is_empty() {
            form = form.child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(i18n::t("Choose an agent with a headless mode first.")),
            );
        } else {
            form = form.child(h_flex().w_full().gap_1().children(
                agents.into_iter().enumerate().map(|(index, choice)| {
                    let selected = agent.as_deref() == Some(choice.id.as_str());
                    let own_id = choice.id.clone();
                    let button = Button::new(ElementId::named_usize("automation-agent", index))
                        .small()
                        .label(choice.name)
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.automations_panel.state.borrow_mut().agent = Some(own_id.clone());
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
        form = form
            .child(render_label("Chat", muted))
            .child(self.render_chat_picker(app, cx))
            .child(
                h_flex()
                    .w_full()
                    .gap_4()
                    .items_end()
                    .child(
                        v_flex().gap_1().child(render_label("Type", muted)).child(
                            h_flex().gap_1().children(
                                [(TaskKind::Once, "Once"), (TaskKind::Every, "Every")]
                                    .into_iter()
                                    .enumerate()
                                    .map(|(index, (value, key))| {
                                        let button = Button::new(ElementId::named_usize(
                                            "automation-kind",
                                            index,
                                        ))
                                        .small()
                                        .label(i18n::t(key))
                                        .on_click(cx.listener(move |app, _, _, cx| {
                                            app.automations_panel.state.borrow_mut().kind = value;
                                            cx.notify();
                                        }));
                                        if kind == value {
                                            button.primary()
                                        } else {
                                            button.secondary()
                                        }
                                        .into_any_element()
                                    }),
                            ),
                        ),
                    )
                    .child(
                        v_flex().gap_1().child(render_label("Output", muted)).child(
                            h_flex().gap_1().children(
                                [
                                    (TaskOutput::Read, "Read"),
                                    (TaskOutput::Draft, "Draft"),
                                    (TaskOutput::Send, "Send"),
                                ]
                                .into_iter()
                                .enumerate()
                                .map(|(index, (value, key))| {
                                    let button = Button::new(ElementId::named_usize(
                                        "automation-output",
                                        index,
                                    ))
                                    .small()
                                    .label(i18n::t(key))
                                    .on_click(cx.listener(move |app, _, _, cx| {
                                        app.automations_panel.state.borrow_mut().output = value;
                                        cx.notify();
                                    }));
                                    if output == value {
                                        button.primary()
                                    } else {
                                        button.secondary()
                                    }
                                    .into_any_element()
                                }),
                            ),
                        ),
                    ),
            );
        let when = match kind {
            TaskKind::Once => v_flex()
                .w_full()
                .gap_1()
                .child(render_label("At", muted))
                .child(
                    div()
                        .w(px(220. * scale))
                        .child(Input::new(&self.at).small()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(i18n::t("It looks like 2026-10-26 09:00.")),
                ),
            TaskKind::Every => v_flex()
                .w_full()
                .gap_1()
                .child(render_label("Cron", muted))
                .child(
                    div()
                        .w(px(220. * scale))
                        .child(Input::new(&self.cron).small()),
                )
                .child(div().text_xs().text_color(muted).child(i18n::t(
                    "min hour day month weekday · for example 0 9 * * *",
                ))),
        };
        form.child(when)
            .child(render_label("Prompt", muted))
            .child(Input::new(&self.prompt))
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_end()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(render_label("Window", muted))
                            .child(Input::new(&self.hours).small())
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(i18n::t("Leave it empty for every hour.")),
                            ),
                    )
                    .child(
                        Button::new("automation-save")
                            .primary()
                            .small()
                            .label(i18n::t("Save task"))
                            .disabled(saving || agents_len(&self.state) == 0)
                            .on_click(cx.listener(|app, _, _, cx| {
                                // The form stays open until the answer comes:
                                // a task the backend refused keeps the values
                                // the user typed next to the reason.
                                save(app, cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("automation-cancel")
                            .secondary()
                            .small()
                            .label(i18n::t("Cancel"))
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.automations_panel.close_form(window, cx);
                                cx.notify();
                            })),
                    ),
            )
    }

    /// The chats of the profile the form offers, with the chosen one marked.
    fn render_chat_picker(
        &self,
        app: &Whatfast,
        cx: &mut Context<Whatfast>,
    ) -> gpui::Stateful<gpui::Div> {
        let choices = chat_choices(app);
        let total = choices.len();
        let rows = choices.clone();
        div()
            .id("automation-chats")
            .w_full()
            .h(px(CHAT_LIST_HEIGHT * app.ui_scale))
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .overflow_hidden()
            .child(
                uniform_list(
                    "automation-chat-picker",
                    total,
                    cx.processor(move |app, range: Range<usize>, _, cx| {
                        range
                            .map(|index| render_chat_choice(app, &rows, index, cx))
                            .collect::<Vec<_>>()
                    }),
                )
                .h_full(),
            )
    }

    /// One task of the profile, with its two buttons.
    fn render_task(
        &self,
        task: &WireSchedule,
        index: usize,
        now: i64,
        app: &Whatfast,
        cx: &mut Context<Whatfast>,
    ) -> gpui::Stateful<gpui::Div> {
        let scale = app.ui_scale;
        let muted = cx.theme().muted_foreground;
        let accent = cx.theme().accent;
        let agent = whatfast_agents::by_id(&task.agent_id)
            .map(|spec| spec.name)
            .unwrap_or(task.agent_id.as_str());
        let chat = if task.chat_title.trim().is_empty() {
            task.chat_id.as_str()
        } else {
            task.chat_title.as_str()
        };
        let when = task_when(task, now);
        let output = output_label(&task.output);
        let window = if task.window.trim().is_empty() {
            i18n::t("Any hour").to_string()
        } else {
            task.window.clone()
        };
        let next = if task.next_at_ms > 0 {
            i18n::t_args(
                "Next turn {when}",
                &[("when", &time_label(task.next_at_ms, now))],
            )
        } else {
            i18n::t("No turn left").to_string()
        };
        let state = if task.paused {
            i18n::t("Paused")
        } else {
            i18n::t("Running")
        };
        let id = task.id;
        let paused = task.paused;
        v_flex()
            .id(ElementId::named_usize("automation-task", index))
            .w_full()
            .gap(px(3.))
            .px(px(10. * scale))
            .py(px(8. * scale))
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
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
                            .child(format!("{agent} · {chat}")),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(if task.paused { muted } else { accent })
                            .child(state),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(format!("{when} · {output} · {window}")),
            )
            .child(
                div()
                    .w_full()
                    .truncate()
                    .text_xs()
                    .text_color(muted)
                    .child(prompt_preview(&task.prompt)),
            )
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
                            .text_xs()
                            .text_color(muted)
                            .child(next),
                    )
                    .child(
                        Button::new(ElementId::named_usize("automation-pause", index))
                            .secondary()
                            .small()
                            .label(if paused {
                                i18n::t("Resume")
                            } else {
                                i18n::t("Pause")
                            })
                            .on_click(cx.listener(move |app, _, _, cx| {
                                set_paused(app, id, !paused);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(ElementId::named_usize("automation-remove", index))
                            .secondary()
                            .small()
                            .label(i18n::t("Delete"))
                            .on_click(cx.listener(move |app, _, _, cx| {
                                remove(app, id);
                                cx.notify();
                            })),
                    ),
            )
    }
}

/// How many agents the form has to offer.
fn agents_len(state: &Rc<RefCell<AutomationsState>>) -> usize {
    state.borrow().agents.len()
}

/// One label of the form.
fn render_label(key: &'static str, muted: gpui::Hsla) -> gpui::Div {
    div().text_xs().text_color(muted).child(i18n::t(key))
}

/// The chats the form offers: the ones of this window first.
///
/// A task is about a conversation the user already has, so the list of the
/// chats of the window is the one that matters. On a window that never opened
/// a chat the list falls back to the first conversations of the profile.
fn chat_choices(app: &Whatfast) -> Rc<Vec<String>> {
    let mut ids: Vec<String> = app
        .recent_chat_ids
        .iter()
        .filter(|id| app.store.chat(id).is_some())
        .cloned()
        .collect();
    if ids.is_empty() {
        ids = app
            .store
            .chat_ids(StoreFilter::All)
            .iter()
            .take(MAX_FORM_CHATS)
            .cloned()
            .collect();
    }
    Rc::new(ids)
}

/// One chat of the list of the form.
fn render_chat_choice(
    app: &mut Whatfast,
    chat_ids: &[String],
    index: usize,
    cx: &mut Context<Whatfast>,
) -> gpui::Stateful<gpui::Div> {
    let scale = app.ui_scale;
    let Some(chat_id) = chat_ids.get(index).cloned() else {
        return div()
            .id(ElementId::named_usize("automation-chat", index))
            .w_full()
            .h(px(CHAT_ROW_HEIGHT * scale));
    };
    let muted = cx.theme().muted_foreground;
    let accent = cx.theme().accent;
    let border = cx.theme().border;
    let selection = cx.theme().selection;
    let hover = cx.theme().muted;
    let title = app
        .store
        .chat(&chat_id)
        .map(|chat| chat.title.clone())
        .unwrap_or_else(|| chat_id.clone());
    let chosen = app.automations_panel.state.borrow().chat.as_deref() == Some(chat_id.as_str());
    let own_id = chat_id.clone();
    h_flex()
        .id(ElementId::named_usize("automation-chat", index))
        .w_full()
        .h(px(CHAT_ROW_HEIGHT * scale))
        .flex_none()
        .gap_2()
        .px(px(8. * scale))
        .items_center()
        .cursor_pointer()
        .when(chosen, |this| this.bg(selection))
        .hover(move |style| style.bg(hover))
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .size(px(12. * scale))
                .rounded_full()
                .border_1()
                .border_color(if chosen { accent } else { border })
                .when(chosen, |this| {
                    this.child(Icon::new(IconName::Check).with_size(px(9. * scale)))
                }),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_xs()
                .text_color(if chosen { accent } else { muted })
                .child(title),
        )
        .on_click(cx.listener(move |app, _, _, cx| {
            app.automations_panel.state.borrow_mut().chat = Some(own_id.clone());
            cx.notify();
        }))
}

/// Reads the tasks of the profile.
pub(crate) fn load(app: &mut Whatfast) {
    app.request(
        rpc_request::Request::ScheduleList(ScheduleListRequest {}),
        PendingRequest::Schedules,
    );
}

/// Reads the tasks and the agents that can run one.
///
/// The screen calls this when it opens: the list of the agents is a question to
/// the profile, and the tasks a question to the backend.
pub(crate) fn open(app: &mut Whatfast) {
    app.automations_panel.set_agents(headless_agents());
    load(app);
}

/// Saves the task of the form.
pub(crate) fn save(app: &mut Whatfast, cx: &mut Context<Whatfast>) {
    let (agent, chat, kind, output) = app.automations_panel.choices();
    let Some(agent) = agent else {
        app.automations_panel
            .fail(i18n::t("Choose an agent with a headless mode first.").to_string());
        return;
    };
    let Some(chat) = chat else {
        app.automations_panel
            .fail(i18n::t("Choose the chat of the task.").to_string());
        return;
    };
    let form = app.automations_panel.values(cx);
    if form.prompt.is_empty() {
        app.automations_panel
            .fail(i18n::t("The task needs a prompt.").to_string());
        return;
    }
    let (at_ms, cron) = match kind {
        TaskKind::Once => match parse_local_instant(&form.at) {
            Some(at) => (at, String::new()),
            None => {
                app.automations_panel
                    .fail(i18n::t("It looks like 2026-10-26 09:00.").to_string());
                return;
            }
        },
        TaskKind::Every => (0, form.cron),
    };
    app.automations_panel.mark_saving();
    app.request(
        rpc_request::Request::SchedulePut(SchedulePutRequest {
            // Zero creates the task: the screen only writes new ones, and a
            // task the user already has is paused or removed.
            id: 0,
            agent_id: agent,
            chat_id: chat,
            kind: kind.wire().to_string(),
            at_ms,
            cron,
            prompt: form.prompt,
            output: output.wire().to_string(),
            window: form.hours,
        }),
        PendingRequest::SchedulePut,
    );
}

/// Pauses one task, or lets it run again.
pub(crate) fn set_paused(app: &mut Whatfast, id: i64, paused: bool) {
    app.request(
        rpc_request::Request::SchedulePause(SchedulePauseRequest { id, paused }),
        PendingRequest::SchedulePause { id },
    );
}

/// Removes one task for good.
pub(crate) fn remove(app: &mut Whatfast, id: i64) {
    app.request(
        rpc_request::Request::ScheduleRemove(ScheduleRemoveRequest { id }),
        PendingRequest::ScheduleRemove { id },
    );
}

/// When a task fires, as the row writes it.
pub(crate) fn task_when(task: &WireSchedule, now: i64) -> String {
    if task.kind == "cron" {
        return i18n::t_args("Every {cron}", &[("cron", &task.cron)]);
    }
    if task.at_ms <= 0 {
        return i18n::t("No turn left").to_string();
    }
    time_label(task.at_ms, now)
}

/// What a task does with the answer, as the row writes it.
pub(crate) fn output_label(output: &str) -> &'static str {
    i18n::t(match output {
        "read" => "Read",
        "send" => "Send",
        _ => "Draft",
    })
}

/// The prompt of a task, cut to the room of the row.
pub(crate) fn prompt_preview(prompt: &str) -> String {
    let flat = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= PREVIEW_CHARS {
        return flat;
    }
    let mut cut: String = flat.chars().take(PREVIEW_CHARS).collect();
    cut.push('…');
    cut
}

/// The milliseconds of the epoch of a local `AAAA-MM-DD HH:MM`.
///
/// `None` when the text is not a date, or when the date does not exist.
pub(crate) fn parse_local_instant(text: &str) -> Option<i64> {
    let text = text.trim();
    let (date, clock) = text.split_once(' ')?;
    let mut fields = date.split('-');
    let year: i64 = fields.next()?.parse().ok()?;
    let month: i64 = fields.next()?.parse().ok()?;
    let day: i64 = fields.next()?.parse().ok()?;
    if fields.next().is_some() || !(1970..=9999).contains(&year) {
        return None;
    }
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
        return None;
    }
    let (hours, minutes) = clock.split_once(':')?;
    let hours: i64 = hours.trim().parse().ok()?;
    let minutes: i64 = minutes.trim().parse().ok()?;
    if !(0..=23).contains(&hours) || !(0..=59).contains(&minutes) {
        return None;
    }
    // The wall clock the user typed, read as if the zone were UTC.
    let naive =
        days_from_civil(year, month, day) * 86_400_000 + hours * 3_600_000 + minutes * 60_000;
    // The offset belongs to the instant, and the instant is what the offset
    // moves: the first pass asks with the wall clock, the second one with the
    // instant that came out. One step is enough for every zone but the hour of
    // the change, and there the first pass is the one of the hour before.
    let guess = naive - i64::from(local_offset_minutes(naive)) * 60_000;
    Some(naive - i64::from(local_offset_minutes(guess)) * 60_000)
}

/// The days each month of one year has.
fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// The days since 1970-01-01 of a calendar date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let shifted_year = if month <= 2 { year - 1 } else { year };
    let era = if shifted_year >= 0 {
        shifted_year
    } else {
        shifted_year - 399
    }
    .div_euclid(400);
    let year_of_era = shifted_year - era * 400;
    let month_of_year = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_of_year + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_local_instant_is_the_wall_clock_the_user_typed() {
        let at = parse_local_instant("2026-06-10 12:00").expect("a date");
        let local = at + i64::from(local_offset_minutes(at)) * 60_000;
        assert_eq!(local.rem_euclid(86_400_000), 12 * 3_600_000);
    }

    #[test]
    fn two_days_a_day_apart_are_a_day_apart() {
        let first = parse_local_instant("2026-06-10 12:00").expect("a date");
        let second = parse_local_instant("2026-06-11 12:00").expect("a date");
        assert_eq!(second - first, 86_400_000);
    }

    #[test]
    fn a_date_that_does_not_exist_is_refused() {
        assert!(parse_local_instant("2026-02-30 09:00").is_none());
        assert!(parse_local_instant("2026-13-01 09:00").is_none());
        assert!(parse_local_instant("2026-10-26 24:00").is_none());
        assert!(parse_local_instant("2026-10-26").is_none());
        assert!(parse_local_instant("mañana 9").is_none());
        // The leap day of a year that has one is a date like the others.
        assert!(parse_local_instant("2028-02-29 08:00").is_some());
        assert!(parse_local_instant("2026-02-29 08:00").is_none());
    }

    #[test]
    fn the_kind_and_the_output_travel_with_the_names_of_the_backend() {
        assert_eq!(TaskKind::Once.wire(), "once");
        assert_eq!(TaskKind::Every.wire(), "cron");
        assert_eq!(TaskOutput::Read.wire(), "read");
        assert_eq!(TaskOutput::Draft.wire(), "draft");
        assert_eq!(TaskOutput::Send.wire(), "send");
    }

    #[test]
    fn the_time_of_a_task_reads_its_kind() {
        let cron = WireSchedule {
            kind: "cron".into(),
            cron: "0 9 * * *".into(),
            ..WireSchedule::default()
        };
        assert!(task_when(&cron, now_ms()).contains("0 9 * * *"));
        let once = WireSchedule {
            kind: "once".into(),
            at_ms: 0,
            ..WireSchedule::default()
        };
        assert_eq!(task_when(&once, now_ms()), i18n::t("No turn left"));
    }

    #[test]
    fn the_output_of_a_task_is_one_of_three_words() {
        assert_eq!(output_label("read"), i18n::t("Read"));
        assert_eq!(output_label("send"), i18n::t("Send"));
        assert_eq!(output_label("draft"), i18n::t("Draft"));
        // A task of an unknown output reads as a draft, which is what the
        // backend does with it.
        assert_eq!(output_label("otro"), i18n::t("Draft"));
    }

    #[test]
    fn the_preview_of_a_prompt_is_one_short_line() {
        assert_eq!(prompt_preview("  hola\n\nque   tal "), "hola que tal");
        let long = "a".repeat(PREVIEW_CHARS + 5);
        assert_eq!(prompt_preview(&long).chars().count(), PREVIEW_CHARS + 1);
    }
}
