//! The screen of the agent harnesses, and the tray of the drafts they leave.
//!
//! The catalog of the harnesses lives in `whatfast_agents`, and the permissions
//! and the drafts live in `whatfast_auto`: this module asks both of them what
//! the profile has and draws the answer. Nothing of the protocol of a client is
//! here: the spec says how one harness is detected and how its server is
//! registered, and [`connect`] runs exactly that.
//!
//! The screen is a sibling of the states and the channels: the column of the
//! left lists the harnesses of this machine, and the pane of the right carries
//! the permissions of the one in front and the drafts that wait for the user.
//!
//! `agents.db` belongs to the profile and not to the window: a missing file or
//! a file of another version leaves the screen with one line of error and the
//! rest of the app working.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{
    AppContext as _, Context, ElementId, Entity, FontWeight, Hsla, InteractiveElement as _,
    IntoElement as _, ParentElement as _, PathPromptOptions, StatefulInteractiveElement as _,
    Styled as _, Window, div, prelude::FluentBuilder as _, px, uniform_list,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    v_flex,
};
use whatfast_agents::{ClientSpec, McpCommand, token::Token};
use whatfast_auto::{AgentRow, ChatFilter, Db, Draft, DraftState, Grant, Scope, Window as Hours};
use whatfast_core::message_format::{DateLocale, LocalTimestamp, format_day, format_time};
use whatfast_i18n as i18n;
use whatfast_model::model::ChatFilter as StoreFilter;

use crate::app::Whatfast;
use crate::local_time::local_offset_minutes;
use crate::rpc::PendingRequest;

/// The name of the database of the agents inside the profile folder.
pub(crate) const AGENTS_DB: &str = "agents.db";

/// How many drafts of each state the tray asks for.
pub(crate) const DRAFT_LIMIT: u32 = 50;

/// The width of the field of the hour ceiling, before the scale.
pub(crate) const NUMBER_FIELD_WIDTH: f32 = 96.0;

/// The height of one row of the list of chats, before the scale.
pub(crate) const CHAT_ROW_HEIGHT: f32 = 30.0;

/// The height of the list of chats, before the scale.
pub(crate) const CHAT_LIST_HEIGHT: f32 = 180.0;

/// The side of the tick of one chat of the grant, before the scale.
pub(crate) const TICK_SIZE: f32 = 15.0;

/// The longest preview of a draft, in characters.
///
/// The layout has no clamp of lines, so the text of a draft is cut by hand
/// before it is drawn.
pub(crate) const PREVIEW_CHARS: usize = 150;

/// Which chats a grant covers, as the screen offers them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ChatsMode {
    /// Every chat, groups included.
    #[default]
    All,
    /// Every chat except the groups.
    Direct,
    /// Only the chats the user ticked.
    Chosen,
}

/// One harness of the catalog, with what this machine answered.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ClientRow {
    /// The spec of the harness.
    pub(crate) spec: &'static ClientSpec,
    /// Whether the harness exists on this machine.
    pub(crate) installed: bool,
    /// Whether the profile already has a live row for it.
    pub(crate) connected: bool,
}

/// A send of the tray that waits for the answer of the backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingSend {
    /// The row of `drafts` it closes.
    pub(crate) draft_id: i64,
    /// The chat it goes to.
    pub(crate) chat_id: String,
    /// The text it sends. The answer names it, so the row is found again.
    pub(crate) text: String,
}

/// The state of the screen of the agents.
#[derive(Default)]
pub(crate) struct AgentsState {
    /// The harnesses of the catalog.
    pub(crate) clients: Vec<ClientRow>,
    /// The agents the profile already connected.
    pub(crate) connected: Vec<AgentRow>,
    /// The drafts that wait for the user, newest first.
    pub(crate) drafts: Vec<Draft>,
    /// The harness whose permissions are on the screen.
    pub(crate) selected: Option<String>,
    /// The grant being edited.
    pub(crate) edit: Grant,
    /// Which chats the edit covers.
    pub(crate) mode: ChatsMode,
    /// The chats ticked in [`ChatsMode::Chosen`].
    pub(crate) checked: BTreeSet<String>,
    /// The line of the result of the last action.
    pub(crate) notice: Option<String>,
    /// The line of the last error.
    pub(crate) error: Option<String>,
    /// The connection to `agents.db`, when it opened.
    pub(crate) db: Option<Db>,
    /// The sends of drafts that are in flight.
    pub(crate) sends: Vec<PendingSend>,
}

impl AgentsState {
    /// The state of a screen that was never opened.
    fn new() -> Self {
        Self {
            edit: Grant::default(),
            ..Self::default()
        }
    }

    /// The live row of one agent, when the profile has one.
    pub(crate) fn agent(&self, id: &str) -> Option<&AgentRow> {
        self.connected
            .iter()
            .find(|row| row.id == id && row.revoked_ms.is_none())
    }

    /// Whether the agent is connected.
    pub(crate) fn is_connected(&self, id: &str) -> bool {
        self.agent(id).is_some()
    }

    /// The row of one harness of the catalog.
    pub(crate) fn client(&self, id: &str) -> Option<&ClientRow> {
        self.clients.iter().find(|row| row.spec.id == id)
    }

    /// Whether a draft is on its way to the backend.
    pub(crate) fn is_sending(&self, draft_id: i64) -> bool {
        self.sends.iter().any(|send| send.draft_id == draft_id)
    }
}

/// The screen of the agents and the tray of the drafts.
pub(crate) struct AgentsPanel {
    /// The state of the screen.
    state: Rc<RefCell<AgentsState>>,
    /// The hour ceiling of the grant being edited.
    per_hour: Entity<InputState>,
    /// The working hours of the grant being edited.
    hours: Entity<InputState>,
}

impl AgentsPanel {
    /// Creates the screen with the two fields of the permissions.
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Whatfast>) -> Self {
        let per_hour = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(i18n::t("20"))
                .clean_on_escape()
        });
        let hours = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(i18n::t("Every hour"))
                .clean_on_escape()
        });
        Self {
            state: Rc::new(RefCell::new(AgentsState::new())),
            per_hour,
            hours,
        }
    }

    /// Reads the catalog, the connected agents and the drafts of the profile.
    ///
    /// The screen keeps its selection when the agent still exists; otherwise
    /// it lands on the first connected agent, then on the first harness of this
    /// machine and, in a machine with none, on the first of the catalog.
    pub(crate) fn reload(&self, window: &mut Window, cx: &mut Context<Whatfast>) {
        let mut state = self.state.borrow_mut();
        state.clients = whatfast_agents::all()
            .iter()
            .map(|spec| ClientRow {
                spec,
                installed: whatfast_agents::detect::installed(spec),
                connected: false,
            })
            .collect();
        state.notice = None;
        state.error = None;
        let path = database_path();
        state.db = match open_database(&path) {
            Ok(db) => Some(db),
            Err(error) => {
                state.error = Some(i18n::t_args(
                    "The drawer of the agents could not be opened: {error}",
                    &[("error", error.as_str())],
                ));
                None
            }
        };
        state.connected.clear();
        if let Some(db) = state.db.clone() {
            match db.agents() {
                Ok(rows) => state.connected = rows,
                Err(error) => {
                    state.error = Some(i18n::t_args(
                        "The agents of this profile could not be read: {error}",
                        &[("error", &error.to_string())],
                    ));
                }
            }
        }
        let connected_ids: Vec<String> = state
            .connected
            .iter()
            .filter(|row| row.revoked_ms.is_none())
            .map(|row| row.id.clone())
            .collect();
        for row in &mut state.clients {
            row.connected = connected_ids.iter().any(|id| id == row.spec.id);
        }
        let selection = match state.selected.as_deref() {
            Some(id) if whatfast_agents::by_id(id).is_some() => state.selected.clone(),
            _ => None,
        };
        let selection = selection
            .or_else(|| state.connected.first().map(|row| row.id.clone()))
            .or_else(|| {
                state
                    .clients
                    .iter()
                    .find(|row| row.installed)
                    .map(|row| row.spec.id.to_string())
            })
            .or_else(|| state.clients.first().map(|row| row.spec.id.to_string()));
        drop(state);
        self.reload_drafts();
        if let Some(id) = selection {
            self.select(&id, window, cx);
        }
    }

    /// Reads the tray of drafts again.
    pub(crate) fn reload_drafts(&self) {
        let mut state = self.state.borrow_mut();
        let Some(db) = state.db.clone() else {
            return;
        };
        let mut drafts = Vec::new();
        for wanted in [DraftState::Proposed, DraftState::Approved] {
            match db.drafts(wanted, DRAFT_LIMIT) {
                Ok(list) => drafts.extend(list),
                Err(error) => {
                    state.error = Some(i18n::t_args(
                        "The drafts of this profile could not be read: {error}",
                        &[("error", &error.to_string())],
                    ));
                }
            }
        }
        state.drafts = drafts;
    }

    /// Puts the permissions of one harness on the screen.
    pub(crate) fn select(&self, id: &str, window: &mut Window, cx: &mut Context<Whatfast>) {
        let grant = {
            let mut state = self.state.borrow_mut();
            let grant = state
                .agent(id)
                .map(|row| row.grant.clone())
                .unwrap_or_default();
            let (mode, checked) = mode_of(&grant.chats);
            state.selected = Some(id.to_string());
            state.edit = grant.clone();
            state.mode = mode;
            state.checked = checked;
            grant
        };
        self.write_fields(&grant, window, cx);
    }

    /// Puts the two fields of the permissions on the values of one grant.
    fn write_fields(&self, grant: &Grant, window: &mut Window, cx: &mut Context<Whatfast>) {
        let ceiling = grant.per_hour.to_string();
        // A grant without hours leaves the field empty: the placeholder of the
        // field says what empty means.
        let hours = grant
            .window
            .map(|window| window.to_string())
            .unwrap_or_default();
        self.per_hour
            .update(cx, |input, cx| input.set_value(ceiling, window, cx));
        self.hours
            .update(cx, |input, cx| input.set_value(hours, window, cx));
    }

    /// The value of the field of the hour ceiling.
    pub(crate) fn ceiling_field(&self, cx: &mut Context<Whatfast>) -> String {
        self.per_hour.read(cx).value().to_string()
    }

    /// The value of the field of the working hours.
    pub(crate) fn hours_field(&self, cx: &mut Context<Whatfast>) -> String {
        self.hours.read(cx).value().to_string()
    }

    /// Sets the line of the result of the last action.
    pub(crate) fn note(&self, message: String) {
        let mut state = self.state.borrow_mut();
        state.notice = Some(message);
        state.error = None;
    }

    /// Sets the line of the last error.
    pub(crate) fn fail(&self, message: String) {
        let mut state = self.state.borrow_mut();
        state.error = Some(message);
        state.notice = None;
    }

    /// Records that a draft is on its way to the backend.
    pub(crate) fn mark_sending(&self, chat_id: &str, text: &str, draft_id: i64) {
        let mut state = self.state.borrow_mut();
        state.sends.retain(|send| send.draft_id != draft_id);
        state.sends.push(PendingSend {
            draft_id,
            chat_id: chat_id.to_string(),
            text: text.to_string(),
        });
        state.error = None;
        state.notice = None;
    }

    /// Closes the row of a draft with the answer of the backend.
    ///
    /// `None` means the answer belongs to the composer and the tray does not
    /// touch it. `Some(true)` is a draft that left; `Some(false)` is a draft
    /// that failed. Only the tray knows which row an answer closes: the text
    /// RPC is the one of the composer.
    pub(crate) fn take_send_response(
        &self,
        pending: &PendingRequest,
        result: &whatfast_protocol::rpc_response::Result,
    ) -> Option<bool> {
        let PendingRequest::SendText {
            chat_id,
            draft_text,
            ..
        } = pending
        else {
            return None;
        };
        let send = self.take_send(chat_id, draft_text)?;
        let sent = match result {
            whatfast_protocol::rpc_response::Result::Error(error) => {
                self.fail(i18n::t_args(
                    "The draft could not be sent: {error}",
                    &[("error", &error.message)],
                ));
                false
            }
            _ => {
                self.note(i18n::t("The draft was sent.").to_string());
                true
            }
        };
        self.close_send(send, sent);
        Some(sent)
    }

    /// Marks a draft that timed out, so the answer never comes.
    ///
    /// Returns whether the request belonged to the tray.
    pub(crate) fn fail_send(&self, chat_id: &str, text: &str) -> bool {
        let Some(send) = self.take_send(chat_id, text) else {
            return false;
        };
        self.fail(i18n::t("The draft could not be sent in time.").to_string());
        self.close_send(send, false);
        true
    }

    /// Takes the row of a send that is in flight.
    fn take_send(&self, chat_id: &str, text: &str) -> Option<PendingSend> {
        let mut state = self.state.borrow_mut();
        let index = state
            .sends
            .iter()
            .position(|send| send.chat_id == chat_id && send.text == text)?;
        Some(state.sends.remove(index))
    }

    /// Writes the end of one send in the tray and in the database.
    fn close_send(&self, send: PendingSend, sent: bool) {
        if let Some(db) = self.state.borrow().db.clone() {
            let wanted = if sent {
                DraftState::Sent
            } else {
                DraftState::Failed
            };
            if let Err(error) = db.set_draft_state(send.draft_id, wanted) {
                self.fail(i18n::t_args(
                    "The drawer of the agents could not be written: {error}",
                    &[("error", &error.to_string())],
                ));
            }
        }
        self.reload_drafts();
    }

    /// Draws the column of the harnesses, which replaces the list of chats.
    pub(crate) fn render_list(&self, app: &Whatfast, cx: &mut Context<Whatfast>) -> gpui::Div {
        let scale = app.ui_scale;
        let muted = cx.theme().muted_foreground;
        let rows: Vec<ClientRow> = self.state.borrow().clients.clone();
        let mut column = v_flex()
            .h_full()
            .w(px(app.sidebar_width * scale))
            .flex_none()
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                v_flex()
                    .flex_none()
                    .gap_1()
                    .px_3()
                    .py_2()
                    .child(
                        div()
                            .text_size(px(24. * scale))
                            .font_weight(FontWeight::BOLD)
                            .child(i18n::t("Agents")),
                    )
                    .child(div().text_sm().text_color(muted).child(i18n::t(
                        "Connect an AI harness to this WhatsApp account with one click.",
                    ))),
            );
        if rows.is_empty() {
            column = column.child(
                v_flex().flex_none().px_3().py_2().child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child(i18n::t("This build knows no agent harness.")),
                ),
            );
        }
        for (index, row) in rows.into_iter().enumerate() {
            column = column.child(self.render_client_row(row, index, scale, cx));
        }
        column
    }

    /// One row of the column: the harness, its state and its button.
    fn render_client_row(
        &self,
        row: ClientRow,
        index: usize,
        scale: f32,
        cx: &mut Context<Whatfast>,
    ) -> gpui::Stateful<gpui::Div> {
        let selected = self.state.borrow().selected.as_deref() == Some(row.spec.id);
        let muted = cx.theme().muted_foreground;
        let accent = cx.theme().accent;
        let hover = cx.theme().muted;
        let secondary = cx.theme().secondary;
        let background = cx.theme().muted;
        let id = row.spec.id;
        let name = row.spec.name;
        let connected = row.connected;
        let installed = row.installed;
        let status = if connected {
            i18n::t("Connected")
        } else if installed {
            i18n::t("Detected on this machine")
        } else {
            i18n::t("Not installed")
        };
        h_flex()
            .id(ElementId::named_usize("agent-row", index))
            .w_full()
            .gap_3()
            .px_3()
            .py_2()
            .items_center()
            .cursor_pointer()
            .when(selected, |this| this.bg(secondary))
            .hover(move |style| style.bg(hover))
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(32. * scale))
                    .rounded_full()
                    .bg(background)
                    .child(
                        Icon::new(IconName::Bot)
                            .with_size(px(18. * scale))
                            .text_color(muted),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(name),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(if connected { accent } else { muted })
                            .child(status),
                    ),
            )
            .child(h_flex().flex_none().occlude().child(if connected {
                Button::new(ElementId::named_usize("agent-disconnect", index))
                    .secondary()
                    .small()
                    .label(i18n::t("Disconnect"))
                    .on_click(cx.listener(move |app, _, window, cx| {
                        disconnect(app, id, window, cx);
                        cx.notify();
                    }))
            } else {
                Button::new(ElementId::named_usize("agent-connect", index))
                    .primary()
                    .small()
                    .label(i18n::t("Connect"))
                    .disabled(!installed)
                    .on_click(cx.listener(move |app, _, window, cx| {
                        connect(app, id, window, cx);
                        cx.notify();
                    }))
            }))
            .on_click(cx.listener(move |app, _, window, cx| {
                app.agents_panel.select(id, window, cx);
                cx.notify();
            }))
    }

    /// Draws the pane of the right: the permissions and the tray of drafts.
    pub(crate) fn render_panel(
        &self,
        app: &Whatfast,
        cx: &mut Context<Whatfast>,
    ) -> gpui::Stateful<gpui::Div> {
        let scale = app.ui_scale;
        let selected = self.state.borrow().selected.clone();
        let mut pane = v_flex()
            .id("agents-panel")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .h_full()
            .overflow_y_scroll()
            .gap_4()
            .p(px(16. * scale));
        if let Some(id) = selected {
            pane = pane.child(self.render_agent_header(&id, scale, cx));
            pane = pane.child(self.render_permissions(&id, app, scale, cx));
        }
        pane.child(self.render_drafts(app, cx))
            // The automations sit under the drafts: both are the work the
            // agents do while the user is not in front of the window.
            .child(app.automations_panel.render(app, cx))
    }

    /// The head of the pane: the harness in front, its lines and its button.
    fn render_agent_header(&self, id: &str, scale: f32, cx: &mut Context<Whatfast>) -> gpui::Div {
        let muted = cx.theme().muted_foreground;
        let danger = cx.theme().danger;
        let accent = cx.theme().accent;
        let row = self.state.borrow().client(id).copied();
        let connected = self.state.borrow().is_connected(id);
        let name = row.map(|row| row.spec.name).unwrap_or(id);
        let installed = row.is_some_and(|row| row.installed);
        let error = self.state.borrow().error.clone();
        let notice = self.state.borrow().notice.clone();
        let status = if connected {
            i18n::t("Connected")
        } else if installed {
            i18n::t("Detected on this machine")
        } else {
            i18n::t("Not installed")
        };
        let own_id = id.to_string();
        let pick_id = own_id.clone();
        let write_id = own_id.clone();
        v_flex()
            .w_full()
            .gap_2()
            .child(
                h_flex()
                    .w_full()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(20. * scale))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(name.to_string()),
                    )
                    .child(div().flex_none().text_xs().text_color(muted).child(status)),
            )
            .child(div().text_sm().text_color(muted).child(if connected {
                i18n::t("The harness reads and writes through the server of Whatfast.")
            } else {
                i18n::t(
                    "One click writes the server of Whatfast in the configuration of the harness.",
                )
            }))
            .when_some(error, |this, message| {
                this.child(div().text_sm().text_color(danger).child(message))
            })
            .when_some(notice, |this, message| {
                this.child(div().text_sm().text_color(accent).child(message))
            })
            .child(
                h_flex()
                    .gap_2()
                    .child(if connected {
                        Button::new("agent-panel-disconnect")
                            .secondary()
                            .small()
                            .label(i18n::t("Disconnect"))
                            .on_click(cx.listener(move |app, _, window, cx| {
                                disconnect(app, &own_id, window, cx);
                                cx.notify();
                            }))
                    } else {
                        Button::new("agent-panel-connect")
                            .primary()
                            .small()
                            .label(i18n::t("Connect"))
                            .disabled(!installed)
                            .on_click(cx.listener(move |app, _, window, cx| {
                                connect(app, &own_id, window, cx);
                                cx.notify();
                            }))
                    })
                    // The file of the client is a guess about the system. When
                    // the guess is wrong, the user points at the real one.
                    .child(
                        Button::new("agent-panel-pick-config")
                            .ghost()
                            .small()
                            .label(i18n::t("Choose config file…"))
                            .on_click(cx.listener(move |app, _, window, cx| {
                                pick_config(app, &pick_id, window, cx);
                                cx.notify();
                            })),
                    )
                    .when(connected, |this| {
                        let again_id = write_id.clone();
                        this.child(
                            Button::new("agent-panel-write-server")
                                .ghost()
                                .small()
                                .label(i18n::t("Write server again"))
                                .on_click(cx.listener(move |app, _, window, cx| {
                                    write_server(app, &again_id, window, cx);
                                    cx.notify();
                                })),
                        )
                    }),
            )
    }

    /// The form of the permissions of one agent.
    fn render_permissions(
        &self,
        id: &str,
        app: &Whatfast,
        scale: f32,
        cx: &mut Context<Whatfast>,
    ) -> gpui::Div {
        let muted = cx.theme().muted_foreground;
        let connected = self.state.borrow().is_connected(id);
        let (mode, edit) = {
            let state = self.state.borrow();
            (state.mode, state.edit.clone())
        };
        let own_id = id.to_string();
        let mut form = v_flex()
            .w_full()
            .gap_3()
            .p(px(12. * scale))
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(muted)
                    .child(i18n::t("PERMISSIONS")),
            )
            .child(render_label("Scope", muted))
            .child(self.render_scope_row(edit.scope, &own_id, cx))
            .child(render_label("Chats", muted))
            .child(self.render_mode_row(mode, &own_id, cx));
        if mode == ChatsMode::Chosen {
            form = form.child(self.render_chat_picker(app, scale, cx));
        }
        form.child(
            h_flex()
                .gap_4()
                .items_end()
                .child(
                    v_flex()
                        .gap_1()
                        .child(render_label("Messages per hour", muted))
                        .child(
                            div()
                                .w(px(NUMBER_FIELD_WIDTH * scale))
                                .child(Input::new(&self.per_hour).small()),
                        ),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .child(render_label("Working hours", muted))
                        .child(Input::new(&self.hours).small())
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .child(i18n::t("Leave it empty for every hour.")),
                        ),
                ),
        )
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(render_label("Pace", muted))
                .child(
                    Button::new("agent-pace")
                        .secondary()
                        .small()
                        .label(if edit.humanize {
                            i18n::t("Human pace")
                        } else {
                            i18n::t("Instant")
                        })
                        .on_click(cx.listener(|app, _, _, cx| {
                            let mut state = app.agents_panel.state.borrow_mut();
                            state.edit.humanize = !state.edit.humanize;
                            cx.notify();
                        })),
                )
                .child(div().flex_1())
                .child(
                    Button::new("agent-save")
                        .primary()
                        .small()
                        .label(i18n::t("Save permissions"))
                        .disabled(!connected)
                        .on_click(cx.listener(|app, _, window, cx| {
                            save_grant(app, window, cx);
                            cx.notify();
                        })),
                ),
        )
    }

    /// The three buttons of the scope.
    fn render_scope_row(&self, scope: Scope, id: &str, cx: &mut Context<Whatfast>) -> gpui::Div {
        let muted = cx.theme().muted_foreground;
        let own_id = id.to_string();
        let choices = [
            (Scope::Read, "Read only"),
            (Scope::Draft, "Drafts"),
            (Scope::Send, "Send without asking"),
        ];
        h_flex()
            .gap_2()
            .items_center()
            .children(
                choices
                    .into_iter()
                    .enumerate()
                    .map(|(index, (value, key))| {
                        let own_id = own_id.clone();
                        let button = Button::new(ElementId::named_usize("agent-scope", index))
                            .small()
                            .label(i18n::t(key))
                            .on_click(cx.listener(move |app, _, _, cx| {
                                let mut state = app.agents_panel.state.borrow_mut();
                                if state.selected.as_deref() == Some(own_id.as_str()) {
                                    state.edit.scope = value;
                                }
                                cx.notify();
                            }));
                        if scope == value {
                            button.primary()
                        } else {
                            button.secondary()
                        }
                        .into_any_element()
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(muted)
                    .child(scope_hint(scope)),
            )
    }

    /// The three buttons of the chats.
    fn render_mode_row(&self, mode: ChatsMode, id: &str, cx: &mut Context<Whatfast>) -> gpui::Div {
        let muted = cx.theme().muted_foreground;
        let own_id = id.to_string();
        let checked = self.state.borrow().checked.len();
        let choices = [
            (ChatsMode::All, "All chats"),
            (ChatsMode::Direct, "Direct chats only"),
            (ChatsMode::Chosen, "Chosen chats"),
        ];
        h_flex()
            .gap_2()
            .items_center()
            .children(
                choices
                    .into_iter()
                    .enumerate()
                    .map(|(index, (value, key))| {
                        let own_id = own_id.clone();
                        let button = Button::new(ElementId::named_usize("agent-chats", index))
                            .small()
                            .label(i18n::t(key))
                            .on_click(cx.listener(move |app, _, _, cx| {
                                let mut state = app.agents_panel.state.borrow_mut();
                                if state.selected.as_deref() == Some(own_id.as_str()) {
                                    state.mode = value;
                                }
                                cx.notify();
                            }));
                        if mode == value {
                            button.primary()
                        } else {
                            button.secondary()
                        }
                        .into_any_element()
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(muted)
                    .child(match mode {
                        ChatsMode::Chosen => {
                            i18n::t_args("{count} chats", &[("count", &checked.to_string())])
                        }
                        other => mode_hint(other).to_string(),
                    }),
            )
    }

    /// The list of the chats of the profile, with a tick on the covered ones.
    fn render_chat_picker(
        &self,
        app: &Whatfast,
        scale: f32,
        cx: &mut Context<Whatfast>,
    ) -> gpui::Div {
        let ids = app.store.chat_ids(StoreFilter::All);
        let total = ids.len();
        let list_ids = ids.clone();
        div()
            .w_full()
            .h(px(CHAT_LIST_HEIGHT * scale))
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .overflow_hidden()
            .child(
                uniform_list(
                    "agent-chat-picker",
                    total,
                    cx.processor(move |app, range: Range<usize>, _, cx| {
                        range
                            .map(|index| render_chat_tick(app, &list_ids, index, cx))
                            .collect::<Vec<_>>()
                    }),
                )
                .h_full(),
            )
    }

    /// The tray of the drafts that wait for the user.
    fn render_drafts(&self, app: &Whatfast, cx: &mut Context<Whatfast>) -> gpui::Div {
        let scale = app.ui_scale;
        let muted = cx.theme().muted_foreground;
        let drafts: Vec<Draft> = self.state.borrow().drafts.clone();
        let count = drafts.len().to_string();
        let section = v_flex().w_full().gap_2().child(
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(px(16. * scale))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(i18n::t("Drafts waiting for you")),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(i18n::t_args("{count} drafts", &[("count", &count)])),
                ),
        );
        if drafts.is_empty() {
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
                        "No draft waits here. The connected agents leave theirs in this tray.",
                    )),
            );
        }
        let mut list = section;
        for (index, draft) in drafts.iter().enumerate() {
            list = list.child(self.render_draft_row(draft, index, app, cx));
        }
        list
    }

    /// One row of the tray: the chat, the agent, the text and its two buttons.
    fn render_draft_row(
        &self,
        draft: &Draft,
        index: usize,
        app: &Whatfast,
        cx: &mut Context<Whatfast>,
    ) -> gpui::Stateful<gpui::Div> {
        let scale = app.ui_scale;
        let muted = cx.theme().muted_foreground;
        let accent = cx.theme().accent;
        let chat = app
            .store
            .chat(&draft.chat_id)
            .map(|chat| chat.title.clone())
            .unwrap_or_else(|| draft.chat_id.clone());
        let agent = whatfast_agents::by_id(&draft.agent_id)
            .map(|spec| spec.name)
            .unwrap_or(draft.agent_id.as_str());
        let sending = self.state.borrow().is_sending(draft.id);
        let approved = draft.state == DraftState::Approved;
        let draft_id = draft.id;
        let chat_id = draft.chat_id.clone();
        let text = draft.text.clone();
        v_flex()
            .id(ElementId::named_usize("agent-draft", index))
            .w_full()
            .gap(px(4.))
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
                            .child(chat),
                    )
                    .when(approved, |this| {
                        this.child(
                            div()
                                .flex_none()
                                .text_xs()
                                .text_color(accent)
                                .child(i18n::t("Approved")),
                        )
                    })
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(muted)
                            .child(agent.to_string()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(muted)
                            .child(time_label(draft.at_ms, now_ms())),
                    ),
            )
            .child(div().text_sm().child(draft_preview(&draft.text)))
            .child(
                h_flex()
                    .w_full()
                    .gap_3()
                    .items_center()
                    .when_some(draft.reason.clone(), |this, reason| {
                        this.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(muted)
                                .child(i18n::t_args("Why: {reason}", &[("reason", &reason)])),
                        )
                    })
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(muted)
                            .child(confidence_label(draft.confidence)),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new(ElementId::named_usize("agent-draft-send", index))
                            .primary()
                            .small()
                            .label(i18n::t("Send"))
                            .disabled(sending)
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.send_agent_draft(draft_id, chat_id.clone(), text.clone());
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(ElementId::named_usize("agent-draft-discard", index))
                            .secondary()
                            .small()
                            .label(i18n::t("Discard"))
                            .disabled(sending)
                            .on_click(cx.listener(move |app, _, _, cx| {
                                discard_draft(app, draft_id);
                                cx.notify();
                            })),
                    ),
            )
    }
}

/// One label of the form of the permissions.
fn render_label(key: &'static str, muted: Hsla) -> gpui::Div {
    div().text_xs().text_color(muted).child(i18n::t(key))
}

/// The short hint next to the three buttons of the scope.
fn scope_hint(scope: Scope) -> &'static str {
    i18n::t(match scope {
        Scope::Read => "It reads the chats and nothing else.",
        Scope::Draft => "It reads and leaves drafts for you to approve.",
        Scope::Send => "It reads, drafts and sends on its own.",
    })
}

/// The short hint next to the three buttons of the chats.
fn mode_hint(mode: ChatsMode) -> &'static str {
    i18n::t(match mode {
        ChatsMode::All => "Every chat, groups included.",
        ChatsMode::Direct => "Every chat except the groups.",
        ChatsMode::Chosen => "Only the chats you tick.",
    })
}

/// The tick of one chat of the grant.
fn render_chat_tick(
    app: &mut Whatfast,
    chat_ids: &[String],
    index: usize,
    cx: &mut Context<Whatfast>,
) -> gpui::Stateful<gpui::Div> {
    let scale = app.ui_scale;
    let Some(chat_id) = chat_ids.get(index).cloned() else {
        return div()
            .id(ElementId::named_usize("agent-chat-row", index))
            .w_full()
            .h(px(CHAT_ROW_HEIGHT * scale));
    };
    let muted = cx.theme().muted_foreground;
    let accent = cx.theme().accent;
    let accent_foreground = cx.theme().accent_foreground;
    let border = cx.theme().border;
    let secondary = cx.theme().secondary;
    let hover = cx.theme().muted;
    let title = app
        .store
        .chat(&chat_id)
        .map(|chat| chat.title.clone())
        .unwrap_or_else(|| chat_id.clone());
    let group = app
        .store
        .chat(&chat_id)
        .is_some_and(|chat| chat.kind() == whatfast_protocol::ChatKind::Group);
    let checked = app.agents_panel.state.borrow().checked.contains(&chat_id);
    let own_id = chat_id.clone();
    h_flex()
        .id(ElementId::named_usize("agent-chat-row", index))
        .w_full()
        .h(px(CHAT_ROW_HEIGHT * scale))
        .flex_none()
        .gap_2()
        .px(px(8. * scale))
        .items_center()
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .size(px(TICK_SIZE * scale))
                .rounded(px(3. * scale))
                .border_1()
                .border_color(if checked { accent } else { border })
                .bg(if checked { accent } else { secondary })
                .when(checked, |this| {
                    this.child(
                        Icon::new(IconName::Check)
                            .with_size(px(11. * scale))
                            .text_color(accent_foreground),
                    )
                }),
        )
        .child(div().flex_1().min_w_0().truncate().text_xs().child(title))
        .when(group, |this| {
            this.child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(muted)
                    .child(i18n::t("Group")),
            )
        })
        .on_click(cx.listener(move |app, _, _, cx| {
            let mut state = app.agents_panel.state.borrow_mut();
            if !state.checked.remove(&own_id) {
                state.checked.insert(own_id.clone());
            }
            // A tick puts the grant on the chosen list: the list is what the
            // grant covers, so the mode above follows the list.
            if state.selected.as_deref() == Some(own_id.as_str()) && !state.checked.is_empty() {
                state.mode = ChatsMode::Chosen;
            }
            cx.notify();
        }))
}

/// The address of the database of the agents in the profile.
pub(crate) fn database_path() -> PathBuf {
    whatfast_model::paths::data_dir().join(AGENTS_DB)
}

/// Opens the database of the agents, creating the profile folder if needed.
pub(crate) fn open_database(path: &Path) -> Result<Db, String> {
    if let Some(parent) = path.parent()
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        return Err(format!("{}: {error}", parent.display()));
    }
    Db::open(path).map_err(|error| format!("{}: {error}", path.display()))
}

/// The current instant, in milliseconds of the epoch.
pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(0)
}

/// The mode and the ticked chats of one grant.
pub(crate) fn mode_of(chats: &ChatFilter) -> (ChatsMode, BTreeSet<String>) {
    match &chats.allow {
        None if chats.groups => (ChatsMode::All, BTreeSet::new()),
        None => (ChatsMode::Direct, BTreeSet::new()),
        Some(list) => (ChatsMode::Chosen, list.iter().cloned().collect()),
    }
}

/// The chats a grant covers, from the mode of the screen.
pub(crate) fn chats_of(mode: ChatsMode, checked: &BTreeSet<String>) -> ChatFilter {
    match mode {
        ChatsMode::All => ChatFilter::all(),
        ChatsMode::Direct => ChatFilter {
            allow: None,
            groups: false,
        },
        // A tick of a group is a grant over that group, so the groups are part
        // of the list the user picked.
        ChatsMode::Chosen => ChatFilter {
            allow: Some(checked.iter().cloned().collect()),
            groups: true,
        },
    }
}

/// The two lines of the text of a draft, cut to the room of the row.
pub(crate) fn draft_preview(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= PREVIEW_CHARS {
        return flat;
    }
    let mut cut: String = flat.chars().take(PREVIEW_CHARS).collect();
    cut.push('…');
    cut
}

/// The confidence of a draft, as the row shows it.
pub(crate) fn confidence_label(value: Option<f64>) -> String {
    match value {
        Some(value) => {
            let percent = (value.clamp(0.0, 1.0) * 100.0).round();
            i18n::t_args(
                "Confidence {value}%",
                &[("value", &format!("{percent:.0}"))],
            )
        }
        None => i18n::t("No confidence").to_string(),
    }
}

/// The moment of a draft, the way the tray writes it.
pub(crate) fn time_label(timestamp_ms: i64, now_ms: i64) -> String {
    let value = LocalTimestamp::new(timestamp_ms, local_offset_minutes(timestamp_ms));
    let now = LocalTimestamp::new(now_ms, local_offset_minutes(now_ms));
    let day = format_day(value, now, DateLocale::EnUs);
    let time = format_time(value, DateLocale::EnUs);
    i18n::t_args("{day} at {time}", &[("day", &day), ("time", &time)])
}

/// The hours of the field, or the line of error of the screen.
///
/// An empty field means "every hour": the placeholder of the field says it.
pub(crate) fn parse_hours(text: &str) -> Result<Option<Hours>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    Hours::parse(text)
        .map(Some)
        .ok_or_else(|| i18n::t("The working hours look like 08:00-23:00.").to_string())
}

/// Connects one harness: the token, the row of the profile and the config.
pub(crate) fn connect(
    app: &mut Whatfast,
    id: &str,
    window: &mut Window,
    cx: &mut Context<Whatfast>,
) {
    let Some(spec) = whatfast_agents::by_id(id) else {
        app.agents_panel
            .fail(i18n::t("This build does not know that agent.").to_string());
        return;
    };
    let exe = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            app.agents_panel.fail(i18n::t_args(
                "The path of this program could not be read: {error}",
                &[("error", &error.to_string())],
            ));
            return;
        }
    };
    let token = match Token::generate() {
        Ok(token) => token,
        Err(error) => {
            app.agents_panel.fail(i18n::t_args(
                "The token of the agent could not be made: {error}",
                &[("error", &error.to_string())],
            ));
            return;
        }
    };
    if let Err(error) = whatfast_agents::token::write(spec.id, &token) {
        app.agents_panel.fail(i18n::t_args(
            "The token of the agent could not be written: {error}",
            &[("error", &error.to_string())],
        ));
        return;
    }
    let grant = grant_of_edit(app, spec.id);
    let Ok(db) = open_database(&database_path()) else {
        app.agents_panel.fail(i18n::t_args(
            "The drawer of the agents could not be opened: {error}",
            &[("error", &database_path().display().to_string())],
        ));
        return;
    };
    if let Err(error) = db.connect_agent(spec.id, spec.id, &grant, token.as_str(), now_ms()) {
        app.agents_panel.fail(i18n::t_args(
            "The agent could not be saved in the profile: {error}",
            &[("error", &error.to_string())],
        ));
        return;
    }
    let command = McpCommand::new(exe, spec.id);
    let report = whatfast_agents::write::register(spec, &command);
    app.agents_panel.reload(window, cx);
    app.agents_panel.select(spec.id, window, cx);
    match report {
        Ok(report) => app.agents_panel.note(i18n::t_args(
            "{name} is connected. The server is registered in {target}.",
            &[("name", spec.name), ("target", &report.target)],
        )),
        Err(error) => app.agents_panel.fail(i18n::t_args(
            "{name} is connected, but its configuration could not be written: {error}",
            &[("name", spec.name), ("error", &error.to_string())],
        )),
    }
}

/// Writes the server of a harness again, in the file the ficha names or in the
/// one the user picked.
///
/// It is the repair for a connection whose configuration was never written —
/// the client changed its file, the command of the ficha did not run— and it is
/// cheap: the same entry, written again.
pub(crate) fn write_server(
    app: &mut Whatfast,
    id: &str,
    window: &mut Window,
    cx: &mut Context<Whatfast>,
) {
    let Some(spec) = whatfast_agents::by_id(id) else {
        app.agents_panel
            .fail(i18n::t("This build does not know that agent.").to_string());
        return;
    };
    let Ok(exe) = std::env::current_exe() else {
        app.agents_panel
            .fail(i18n::t("The path of this program could not be read.").to_string());
        return;
    };
    let command = McpCommand::new(exe, spec.id);
    match whatfast_agents::write::register(spec, &command) {
        Ok(report) => app.agents_panel.note(i18n::t_args(
            "The server is registered in {target}.",
            &[("target", &report.target)],
        )),
        Err(error) => app.agents_panel.fail(i18n::t_args(
            "The configuration could not be written: {error}",
            &[("error", &error.to_string())],
        )),
    }
    app.agents_panel.reload(window, cx);
}

/// Asks the user for the configuration file of one harness and registers the
/// server there.
///
/// The path of the ficha is a guess about the system: a client that was moved,
/// installed somewhere else or keeps its file under another name is not found,
/// and the user knows where it is. The picked file is remembered per agent, and
/// the next Connect writes there without asking again.
pub(crate) fn pick_config(
    app: &mut Whatfast,
    id: &str,
    window: &mut Window,
    cx: &mut Context<Whatfast>,
) {
    let Some(spec) = whatfast_agents::by_id(id) else {
        app.agents_panel
            .fail(i18n::t("This build does not know that agent.").to_string());
        return;
    };
    let Ok(exe) = std::env::current_exe() else {
        app.agents_panel
            .fail(i18n::t("The path of this program could not be read.").to_string());
        return;
    };
    let selected = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: Some(i18n::t("Choose the configuration file of the agent").into()),
    });
    let view = cx.entity();
    cx.spawn_in(window, async move |_, window| {
        let Some(path) = selected
            .await
            .ok()
            .and_then(|result| result.ok())
            .flatten()
            .and_then(|paths| paths.into_iter().next())
        else {
            return;
        };
        let command = McpCommand::new(exe, spec.id);
        let report =
            whatfast_agents::write::register_at(spec, &command, &path).and_then(|report| {
                whatfast_agents::write::save_path(spec.id, &path)?;
                Ok(report)
            });
        window
            .update(|window, cx| {
                view.update(cx, |app, cx| {
                    match report {
                        Ok(report) => app.agents_panel.note(i18n::t_args(
                            "The server is registered in {target}.",
                            &[("target", &report.target)],
                        )),
                        Err(error) => app.agents_panel.fail(i18n::t_args(
                            "The configuration could not be written: {error}",
                            &[("error", &error.to_string())],
                        )),
                    }
                    app.agents_panel.reload(window, cx);
                });
            })
            .ok();
    })
    .detach();
}

/// Disconnects one harness: the row of the profile and the token.
pub(crate) fn disconnect(
    app: &mut Whatfast,
    id: &str,
    window: &mut Window,
    cx: &mut Context<Whatfast>,
) {
    let name = whatfast_agents::by_id(id)
        .map(|spec| spec.name)
        .unwrap_or(id);
    let mut failed = false;
    match open_database(&database_path()) {
        Ok(db) => {
            if let Err(error) = db.revoke(id, now_ms()) {
                app.agents_panel.fail(i18n::t_args(
                    "{name} could not be disconnected: {error}",
                    &[("name", name), ("error", &error.to_string())],
                ));
                failed = true;
            }
        }
        Err(error) => {
            app.agents_panel.fail(i18n::t_args(
                "The drawer of the agents could not be opened: {error}",
                &[("error", &error)],
            ));
            failed = true;
        }
    }
    if let Err(error) = whatfast_agents::token::revoke(id) {
        app.agents_panel.fail(i18n::t_args(
            "The token of {name} could not be removed: {error}",
            &[("name", name), ("error", &error.to_string())],
        ));
        failed = true;
    }
    app.agents_panel.reload(window, cx);
    if !failed {
        app.agents_panel
            .note(i18n::t_args("{name} is disconnected.", &[("name", name)]));
    }
}

/// Saves the permissions of the agent in front.
pub(crate) fn save_grant(app: &mut Whatfast, window: &mut Window, cx: &mut Context<Whatfast>) {
    let selected = app.agents_panel.state.borrow().selected.clone();
    let Some(id) = selected else {
        return;
    };
    let Ok(per_hour) = app.agents_panel.ceiling_field(cx).trim().parse::<u16>() else {
        app.agents_panel
            .fail(i18n::t("The hour ceiling has to be a number.").to_string());
        return;
    };
    let window_hours = match parse_hours(&app.agents_panel.hours_field(cx)) {
        Ok(hours) => hours,
        Err(message) => {
            app.agents_panel.fail(message);
            return;
        }
    };
    let grant = {
        let state = app.agents_panel.state.borrow();
        Grant {
            scope: state.edit.scope,
            chats: chats_of(state.mode, &state.checked),
            per_hour,
            window: window_hours,
            humanize: state.edit.humanize,
        }
    };
    let result = match open_database(&database_path()) {
        Ok(db) => db.set_grant(&id, &grant),
        Err(error) => {
            app.agents_panel.fail(i18n::t_args(
                "The drawer of the agents could not be opened: {error}",
                &[("error", &error)],
            ));
            return;
        }
    };
    match result {
        Ok(()) => {
            app.agents_panel.reload(window, cx);
            app.agents_panel.select(&id, window, cx);
            app.agents_panel
                .note(i18n::t("The permissions are saved.").to_string());
        }
        Err(error) => app.agents_panel.fail(i18n::t_args(
            "The permissions could not be saved: {error}",
            &[("error", &error.to_string())],
        )),
    }
}

/// Discards one draft of the tray.
pub(crate) fn discard_draft(app: &mut Whatfast, draft_id: i64) {
    let db = match open_database(&database_path()) {
        Ok(db) => db,
        Err(error) => {
            app.agents_panel.fail(i18n::t_args(
                "The drawer of the agents could not be opened: {error}",
                &[("error", &error)],
            ));
            return;
        }
    };
    match db.set_draft_state(draft_id, DraftState::Discarded) {
        Ok(true) => {
            app.agents_panel.reload_drafts();
            app.agents_panel
                .note(i18n::t("The draft is discarded.").to_string());
        }
        Ok(false) => app
            .agents_panel
            .fail(i18n::t("That draft is not in the tray anymore.").to_string()),
        Err(error) => app.agents_panel.fail(i18n::t_args(
            "The draft could not be discarded: {error}",
            &[("error", &error.to_string())],
        )),
    }
}

/// The grant the form holds for one agent, or the prudent one.
fn grant_of_edit(app: &Whatfast, id: &str) -> Grant {
    let state = app.agents_panel.state.borrow();
    if state.selected.as_deref() == Some(id) {
        state.edit.clone()
    } else {
        Grant::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_all_mode_covers_every_chat_and_the_groups() {
        let chats = chats_of(ChatsMode::All, &BTreeSet::new());
        assert_eq!(chats, ChatFilter::all());
        assert!(chats.allows("123@g.us", true));
        assert_eq!(mode_of(&chats), (ChatsMode::All, BTreeSet::new()));
    }

    #[test]
    fn the_direct_mode_leaves_the_groups_out() {
        let chats = chats_of(ChatsMode::Direct, &BTreeSet::new());
        assert!(chats.allows("5491100000000@s.whatsapp.net", false));
        assert!(!chats.allows("123@g.us", true));
        assert_eq!(mode_of(&chats), (ChatsMode::Direct, BTreeSet::new()));
    }

    #[test]
    fn the_chosen_mode_covers_only_the_ticked_chats() {
        let checked = BTreeSet::from(["a@s.whatsapp.net".to_string()]);
        let chats = chats_of(ChatsMode::Chosen, &checked);
        assert!(chats.allows("a@s.whatsapp.net", false));
        assert!(!chats.allows("b@s.whatsapp.net", false));
        assert_eq!(mode_of(&chats), (ChatsMode::Chosen, checked));
    }

    #[test]
    fn an_empty_list_of_chats_is_a_closed_door() {
        let chats = chats_of(ChatsMode::Chosen, &BTreeSet::new());
        assert!(!chats.allows("a@s.whatsapp.net", false));
    }

    #[test]
    fn the_preview_flattens_the_text_and_cuts_the_rest() {
        assert_eq!(draft_preview("  hola\n\n  que   tal "), "hola que tal");
        let long = "a".repeat(PREVIEW_CHARS + 20);
        let preview = draft_preview(&long);
        assert_eq!(preview.chars().count(), PREVIEW_CHARS + 1);
        assert!(preview.ends_with('…'));
    }

    #[test]
    fn the_confidence_goes_in_percent() {
        assert_eq!(confidence_label(None), i18n::t("No confidence"));
        assert!(confidence_label(Some(0.87)).starts_with("Confidence 87%"));
        // A value outside the range is clamped instead of painted as is.
        assert!(confidence_label(Some(4.0)).starts_with("Confidence 100%"));
    }

    #[test]
    fn the_hours_of_the_field_round_trip() {
        assert_eq!(parse_hours("08:00-23:00"), Ok(Some(Hours::PRUDENT)));
        assert_eq!(parse_hours("  "), Ok(None));
        assert!(parse_hours("8-23").is_err());
    }

    #[test]
    fn the_database_of_the_agents_lives_in_the_profile() {
        let path = database_path();
        assert!(path.ends_with(AGENTS_DB));
        assert_eq!(
            path.parent(),
            Some(whatfast_model::paths::data_dir().as_path())
        );
    }
}
