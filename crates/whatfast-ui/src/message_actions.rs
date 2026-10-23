//! The actions of a message: the menu of a bubble, the picker of the forward,
//! the form of the poll and the button of the clip.
//!
//! port: the four components of this file come from the web client:
//! - the menu of a message: `tauri/src/components/MessageBubble.tsx` (the
//!   `message-context-menu` of the right click),
//! - the picker of the forward: `tauri/src/components/Screens.tsx`
//!   (`ForwardDialog`),
//! - the form of the poll: `tauri/src/components/Composer.tsx` (the popover
//!   `poll-composer`),
//! - the button of the clip: `tauri/src/components/Composer.tsx` (the popover
//!   `attachment-menu`).
//!
//! Every component of this file is a component of its own: the app owns an
//! `Entity<MessageMenu>` (or `Entity<ForwardPicker>`, `Entity<PollComposer>`
//! or `Entity<AttachmentBar>`), keeps its state in sync and drops the entity
//! in the layout. Each component reports what the user chose to the handler
//! the app installs with `on_action`; a component never changes the state of
//! the app by itself.
//!
//! The menu of a message is the same for a message of the user and for a
//! message of somebody else: the rows that do not apply to the message are
//! drawn but they cannot be pressed.

// The module arrives before its caller: the app of the batch B-U9 adds the
// four components to the layout.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use whatfast_i18n as i18n;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, PathPromptOptions, Pixels, Point, Render, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, div, point, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};

use whatfast_model::model::message_text;
use whatfast_protocol::{AttachmentKind, Chat, Message};

use crate::icons::WhatfastIcon;
use crate::tokens::normalized_ui_scale;

// ---------------------------------------------------------------------------
// The menu of a message
// ---------------------------------------------------------------------------

/// The window of time in which a message can still be edited, in
/// milliseconds.
///
/// port: `EDIT_WINDOW_MS` of `MessageBubble.tsx`: `20 * 60_000`.
const EDIT_WINDOW_MS: i64 = 20 * 60_000;

/// The smallest width of the menu, before the scale.
///
/// port: `.chat-context-menu { min-width: 210px }`.
const MENU_MIN_WIDTH: f32 = 210.0;

/// The width of the menu, before the scale.
///
/// The menu is a floating panel: it takes the width of its rows and not the
/// width of the window it is drawn in.
const MENU_WIDTH: f32 = 230.0;

/// The tallest height of the menu, before the scale: the rows, the divider and
/// the padding. It keeps the panel inside the window when the pointer is near
/// the bottom edge.
const MENU_MAX_HEIGHT: f32 = 340.0;

/// The padding of the menu, before the scale.
///
/// port: `.chat-context-menu { padding: 4px }`.
const MENU_PADDING: f32 = 4.0;

/// The padding of a row of the menu, before the scale.
///
/// port: `.chat-context-item { padding: 8px 12px }`.
const MENU_ITEM_PADDING_Y: f32 = 8.0;

/// The padding of a row of the menu, before the scale.
///
/// port: `.chat-context-item { padding: 8px 12px }`.
const MENU_ITEM_PADDING_X: f32 = 12.0;

/// The size of the text of a row of the menu, before the scale.
///
/// port: `.chat-context-item { font-size: 13px }`.
const MENU_ITEM_FONT_SIZE: f32 = 13.0;

/// The margin of the divider of the menu, before the scale.
///
/// port: `.chat-context-divider { margin: 4px 0 }`.
const MENU_DIVIDER_MARGIN: f32 = 4.0;

/// What the menu of a message asks the app to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MessageAction {
    /// Answer the message.
    Reply,
    /// Send the message to another conversation.
    Forward,
    /// Change the text of the message.
    Edit,
    /// Delete the message of this device.
    Delete,
    /// Delete the message for everybody.
    DeleteForEveryone,
    /// Copy the text of the message to the clipboard.
    Copy,
    /// Pin the message in the conversation.
    Pin,
    /// Take the message out of the pinned ones.
    Unpin,
}

/// One row of the menu of a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MessageMenuItem {
    /// What the row does.
    pub(crate) action: MessageAction,
    /// The label of the row.
    ///
    /// port: the labels of the oracle: `message.reply`, `message.forward`,
    /// `message.edit`, `message.copy`, `message.pin`, `message.unpin`,
    /// `message.deleteForMe` and `message.deleteForEveryone`.
    pub(crate) label: &'static str,
    /// Whether the row can be pressed.
    pub(crate) enabled: bool,
    /// Whether the row is one of the deletions.
    pub(crate) danger: bool,
}

/// The handler the rows of the menu report to.
pub(crate) type MessageActionHandler = Option<Rc<dyn Fn(MessageAction, &mut Window, &mut App)>>;

/// The menu of a message.
///
/// The menu is the same for a message of the user and for a message of
/// somebody else: the rows that do not apply are drawn but they cannot be
/// pressed.
pub(crate) struct MessageMenu {
    /// The conversation the message is in.
    chat_id: String,
    /// The message the menu is of.
    message_id: String,
    /// The message is one of the user.
    from_me: bool,
    /// The message was deleted.
    revoked: bool,
    /// The message carries a content.
    has_content: bool,
    /// The text that the copy of the message puts in the clipboard.
    text: String,
    /// The message has a text to copy.
    has_text: bool,
    /// The message can still be edited.
    editable: bool,
    /// The message is pinned in the conversation.
    pinned: bool,
    /// The instant in which the window of the edit closes.
    edit_deadline_ms: i64,
    /// The scale of the interface.
    scale: f32,
    /// Where the pointer was when the menu opened, in pixels of the window.
    position: Point<Pixels>,
    /// The handler the rows report to.
    on_action: MessageActionHandler,
}

/// The corner of a floating panel: where the pointer is, moved inside the
/// window when the panel would not fit.
fn floating_origin(
    position: Point<Pixels>,
    viewport: gpui::Size<Pixels>,
    width: Pixels,
    height: Pixels,
    margin: Pixels,
) -> Point<Pixels> {
    point(
        position.x.min(viewport.width - width - margin).max(margin),
        position
            .y
            .min(viewport.height - height - margin)
            .max(margin),
    )
}

impl std::fmt::Debug for MessageMenu {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MessageMenu")
            .field("chat_id", &self.chat_id)
            .field("message_id", &self.message_id)
            .field("from_me", &self.from_me)
            .field("revoked", &self.revoked)
            .field("has_content", &self.has_content)
            .field("has_text", &self.has_text)
            .field("editable", &self.editable)
            .field("pinned", &self.pinned)
            .field("edit_deadline_ms", &self.edit_deadline_ms)
            .field("scale", &self.scale)
            .finish_non_exhaustive()
    }
}

impl MessageMenu {
    /// The menu of one message.
    ///
    /// `now_ms` is the instant the menu is built at: the window of the edit
    /// is a deadline, not a snapshot, so the menu has to know the time.
    pub(crate) fn new(
        chat_id: impl Into<String>,
        message: &Message,
        pinned: bool,
        now_ms: i64,
        scale: f32,
    ) -> Self {
        // port: the oracle computes `editable` with the content of the
        // message, its own flag and the window of twenty minutes.
        let editable = message.from_me
            && !message.revoked
            && matches!(
                message.content,
                Some(whatfast_protocol::message::Content::Text(_))
            )
            && message.timestamp_ms + EDIT_WINDOW_MS > now_ms;
        let text = message_text(message).to_string();
        Self {
            chat_id: chat_id.into(),
            message_id: message.id.clone(),
            from_me: message.from_me,
            revoked: message.revoked,
            has_content: message.content.is_some(),
            // port: the oracle copies `messageText(message)`, which is the
            // caption of a picture and the name of a file too.
            has_text: !text.is_empty(),
            text,
            editable,
            pinned,
            edit_deadline_ms: message.timestamp_ms + EDIT_WINDOW_MS,
            scale: normalized_ui_scale(scale),
            position: point(px(0.), px(0.)),
            on_action: None,
        }
    }

    /// The place of the window the menu opens at: the pointer of the right
    /// click, or the button that opens it.
    pub(crate) fn at(mut self, position: Point<Pixels>) -> Self {
        self.position = position;
        self
    }

    /// The handler the rows of the menu report to.
    pub(crate) fn on_action(
        mut self,
        handler: impl Fn(MessageAction, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_action = Some(Rc::new(handler));
        self
    }

    /// The conversation the message is in.
    pub(crate) fn chat_id(&self) -> &str {
        &self.chat_id
    }

    /// The message the menu is of.
    pub(crate) fn message_id(&self) -> &str {
        &self.message_id
    }

    /// The message is one of the user.
    pub(crate) fn is_from_me(&self) -> bool {
        self.from_me
    }

    /// The text that the copy of the message puts in the clipboard.
    ///
    /// port: the oracle copies `messageText(message)`.
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// The message is pinned in the conversation.
    pub(crate) fn pinned(&self) -> bool {
        self.pinned
    }

    /// The message is pinned, or it is not.
    pub(crate) fn set_pinned(&mut self, pinned: bool) {
        self.pinned = pinned;
    }

    /// The message can still be edited.
    pub(crate) fn editable(&self) -> bool {
        self.editable
    }

    /// The instant in which the window of the edit closes.
    pub(crate) fn edit_deadline_ms(&self) -> i64 {
        self.edit_deadline_ms
    }

    /// The scale of the interface.
    pub(crate) fn scale(&self) -> f32 {
        self.scale
    }

    /// The rows of the menu, in the order the oracle draws them.
    ///
    /// port: the menu of the oracle draws the answer, the forward, the pin,
    /// the edit and the copy, and then a divider and the deletion.
    pub(crate) fn items(&self) -> Vec<MessageMenuItem> {
        let forwardable = self.has_content && !self.revoked;
        vec![
            MessageMenuItem {
                action: MessageAction::Reply,
                label: "Reply",
                enabled: true,
                danger: false,
            },
            MessageMenuItem {
                action: MessageAction::Forward,
                label: "Forward",
                enabled: forwardable,
                danger: false,
            },
            MessageMenuItem {
                action: MessageAction::Edit,
                label: "Edit",
                enabled: self.editable,
                danger: false,
            },
            MessageMenuItem {
                action: MessageAction::Copy,
                label: "Copy",
                enabled: self.has_text,
                danger: false,
            },
            MessageMenuItem {
                action: MessageAction::Pin,
                label: "Pin message",
                enabled: forwardable && !self.pinned,
                danger: false,
            },
            MessageMenuItem {
                action: MessageAction::Unpin,
                label: "Unpin message",
                enabled: self.pinned,
                danger: false,
            },
            MessageMenuItem {
                action: MessageAction::Delete,
                label: "Delete for me",
                enabled: !self.revoked,
                danger: true,
            },
            MessageMenuItem {
                action: MessageAction::DeleteForEveryone,
                label: "Delete for everyone",
                // port: only the messages of the user can be deleted for
                // everybody.
                enabled: !self.revoked && self.from_me,
                danger: true,
            },
        ]
    }

    /// The menu of the message.
    ///
    /// The panel floats: it is drawn where the pointer was and it keeps the
    /// width of its rows. Added as one more child of the column of the layout it
    /// would stretch to the whole window, which is not what a menu is.
    pub(crate) fn render(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let foreground = theme.foreground;
        let muted = theme.muted_foreground;
        let hover = theme.muted;
        let danger = theme.danger;
        let radius = theme.radius;

        // El menu no sale de la ventana: se corre para adentro cuando el puntero
        // esta cerca del borde.
        let width = px(MENU_WIDTH * self.scale);
        let origin = floating_origin(
            self.position,
            window.viewport_size(),
            width,
            px(MENU_MAX_HEIGHT * self.scale),
            px(8.),
        );
        let left = origin.x;
        let top = origin.y;

        let mut menu = v_flex()
            .absolute()
            .left(left)
            .top(top)
            .w(width)
            .min_w(px(MENU_MIN_WIDTH * self.scale))
            .p(px(MENU_PADDING))
            .rounded(radius)
            .border_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .shadow_lg()
            .text_color(theme.popover_foreground);

        let mut divider = false;
        for item in self.items() {
            // port: the oracle draws a divider before the deletion.
            if item.danger && !divider {
                menu = menu.child(div().h(px(1.)).my(px(MENU_DIVIDER_MARGIN)).bg(theme.border));
                divider = true;
            }
            let action = item.action;
            let enabled = item.enabled;
            let handler = self.on_action.clone();
            let color = if item.danger { danger } else { foreground };
            menu = menu.child(
                div()
                    .id(format!("message-menu-item-{}", item.label))
                    .flex()
                    .items_center()
                    .w_full()
                    .px(px(MENU_ITEM_PADDING_X))
                    .py(px(MENU_ITEM_PADDING_Y))
                    .rounded(radius)
                    .text_size(px(MENU_ITEM_FONT_SIZE))
                    .cursor_pointer()
                    .text_color(if enabled { color } else { muted })
                    .when(enabled, |row| row.hover(|style| style.bg(hover)))
                    .when(!enabled, |row| row.opacity(0.45).cursor_default())
                    .on_click(move |_, window, cx| {
                        if !enabled {
                            return;
                        }
                        if let Some(handler) = handler.clone() {
                            handler(action, window, cx);
                        }
                    })
                    .child(i18n::t(item.label)),
            );
        }

        menu
    }
}

impl Render for MessageMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        MessageMenu::render(self, window, cx)
    }
}

// ---------------------------------------------------------------------------
// The picker of the forward
// ---------------------------------------------------------------------------

/// The width of the dialog of the forward, before the scale.
///
/// port: `.forward-dialog { width: min(440px, ...) }`.
const FORWARD_DIALOG_WIDTH: f32 = 440.0;

/// The tallest height of the dialog of the forward, before the scale.
///
/// port: `.forward-dialog { max-height: min(680px, ...) }`.
const FORWARD_DIALOG_MAX_HEIGHT: f32 = 680.0;

/// The padding of the dialog of the forward, before the scale.
///
/// port: `.dialog-card { padding: 22px }`.
const FORWARD_DIALOG_PADDING: f32 = 22.0;

/// The margin above the search of the dialog, before the scale.
///
/// port: `.forward-search { margin-top: 16px }`.
const FORWARD_SEARCH_MARGIN_TOP: f32 = 16.0;

/// The padding of the search of the dialog, before the scale.
///
/// port: `.forward-search { padding: 8px 10px }`.
const FORWARD_SEARCH_PADDING_Y: f32 = 8.0;

/// The padding of the search of the dialog, before the scale.
///
/// port: `.forward-search { padding: 8px 10px }`.
const FORWARD_SEARCH_PADDING_X: f32 = 10.0;

/// The margin above the list of the chats, before the scale.
///
/// port: `.forward-chat-list { margin-top: 10px }`.
const FORWARD_LIST_MARGIN_TOP: f32 = 10.0;

/// The smallest height of the list of the chats, before the scale.
///
/// port: `.forward-chat-list { min-height: 100px }`.
const FORWARD_LIST_MIN_HEIGHT: f32 = 100.0;

/// The padding of a row of the list of the chats, before the scale.
///
/// port: `.forward-chat-row { padding: 10px 4px }`.
const FORWARD_ROW_PADDING_Y: f32 = 10.0;

/// The padding of a row of the list of the chats, before the scale.
///
/// port: `.forward-chat-row { padding: 10px 4px }`.
const FORWARD_ROW_PADDING_X: f32 = 4.0;

/// The gap of a row of the list of the chats, before the scale.
///
/// port: `.forward-chat-row { gap: 11px }`.
const FORWARD_ROW_GAP: f32 = 11.0;

/// The size of the box of the selection of a row, before the scale.
const FORWARD_CHECK_SIZE: f32 = 16.0;

/// What the picker of the forward asks the app to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ForwardAction {
    /// The user confirmed: the message goes to these conversations.
    Forward(Vec<String>),
    /// The user closed the dialog without forwarding.
    Cancel,
}

/// The handler the picker of the forward reports to.
pub(crate) type ForwardActionHandler = Option<Rc<dyn Fn(ForwardAction, &mut Window, &mut App)>>;

/// Whether a chat matches the needle of the search.
///
/// port: the oracle filters with
/// `` `${chat.title} ${chat.phoneNumber}`.toLocaleLowerCase().includes(needle) ``.
pub(crate) fn chat_matches(chat: &Chat, needle: &str) -> bool {
    let needle = needle.to_lowercase();
    if needle.is_empty() {
        return true;
    }
    let haystack = format!("{} {}", chat.title, chat.phone_number).to_lowercase();
    haystack.contains(&needle)
}

/// The chats that match a query of the search.
///
/// port: the oracle filters the chats of the state with the needle of the
/// search, in lower case and without the spaces around it.
pub(crate) fn filter_chats<'a>(chats: &'a [Chat], query: &str) -> Vec<&'a Chat> {
    let needle = query.trim();
    chats
        .iter()
        .filter(|chat| chat_matches(chat, needle))
        .collect()
}

/// Adds a conversation to the selection of the forward, or takes it out when
/// it was already there.
pub(crate) fn toggle_selection(selected: &mut Vec<String>, chat_id: &str) {
    match selected.iter().position(|id| id == chat_id) {
        Some(index) => {
            selected.remove(index);
        }
        None => selected.push(chat_id.to_string()),
    }
}

/// The dialog that chooses the conversations a message is sent to.
pub(crate) struct ForwardPicker {
    /// The conversation the message comes from.
    source_chat_id: String,
    /// The message that is forwarded.
    message_id: String,
    /// The conversations the user can choose.
    chats: Vec<Chat>,
    /// The needle of the search.
    query: String,
    /// The conversations the user chose.
    selected: Vec<String>,
    /// The field of the search.
    search_input: Entity<InputState>,
    /// The scale of the interface.
    scale: f32,
    /// The handler the picker reports to.
    on_action: ForwardActionHandler,
    /// The subscription to the field of the search.
    _search_subscription: Subscription,
}

impl std::fmt::Debug for ForwardPicker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ForwardPicker")
            .field("source_chat_id", &self.source_chat_id)
            .field("message_id", &self.message_id)
            .field("chats", &self.chats.len())
            .field("query", &self.query)
            .field("selected", &self.selected)
            .field("scale", &self.scale)
            .finish_non_exhaustive()
    }
}

impl ForwardPicker {
    /// The dialog of the forward of one message.
    pub(crate) fn new(
        source_chat_id: impl Into<String>,
        message_id: impl Into<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(i18n::t("Search chats")));
        let field = search_input.clone();
        let subscription = cx.subscribe_in(&search_input, window, move |this, _, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                let query = field.read(cx).value().to_string();
                this.query = query;
            }
        });
        Self {
            source_chat_id: source_chat_id.into(),
            message_id: message_id.into(),
            chats: Vec::new(),
            query: String::new(),
            selected: Vec::new(),
            search_input,
            scale: 1.0,
            on_action: None,
            _search_subscription: subscription,
        }
    }

    /// The handler the picker reports to.
    pub(crate) fn on_action(
        mut self,
        handler: impl Fn(ForwardAction, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_action = Some(Rc::new(handler));
        self
    }

    /// The scale of the interface.
    pub(crate) fn scale(mut self, scale: f32) -> Self {
        self.scale = normalized_ui_scale(scale);
        self
    }

    /// The conversation the message comes from.
    pub(crate) fn source_chat_id(&self) -> &str {
        &self.source_chat_id
    }

    /// The message that is forwarded.
    pub(crate) fn message_id(&self) -> &str {
        &self.message_id
    }

    /// The conversations the user can choose.
    pub(crate) fn set_chats(&mut self, chats: Vec<Chat>) {
        self.chats = chats;
    }

    /// The conversations the user can choose.
    pub(crate) fn chats(&self) -> &[Chat] {
        &self.chats
    }

    /// The needle of the search.
    pub(crate) fn query(&self) -> &str {
        &self.query
    }

    /// The needle of the search.
    pub(crate) fn set_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
    }

    /// The conversations the user chose.
    pub(crate) fn selected(&self) -> &[String] {
        &self.selected
    }

    /// Whether a conversation is chosen.
    pub(crate) fn is_selected(&self, chat_id: &str) -> bool {
        self.selected.iter().any(|id| id == chat_id)
    }

    /// Chooses a conversation, or takes it out when it was already chosen.
    pub(crate) fn toggle(&mut self, chat_id: &str) {
        toggle_selection(&mut self.selected, chat_id);
    }

    /// The conversations that match the needle of the search.
    pub(crate) fn visible_chats(&self) -> Vec<&Chat> {
        filter_chats(&self.chats, &self.query)
    }

    /// The message can be forwarded: at least one conversation is chosen.
    pub(crate) fn can_forward(&self) -> bool {
        !self.selected.is_empty()
    }

    /// Puts the caret in the field of the search.
    pub(crate) fn focus_search(&self, window: &mut Window, cx: &mut App) {
        self.search_input
            .update(cx, |input, cx| input.focus(window, cx));
    }

    /// The user confirmed: the message goes to the chosen conversations.
    pub(crate) fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_forward() {
            return;
        }
        let selected = self.selected.clone();
        self.report(ForwardAction::Forward(selected), window, cx);
    }

    /// The user closed the dialog without forwarding.
    pub(crate) fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.report(ForwardAction::Cancel, window, cx);
    }

    /// The dialog of the forward.
    pub(crate) fn render(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let foreground = theme.foreground;
        let muted = theme.muted_foreground;
        let hover = theme.muted;
        let radius = theme.radius;
        let chosen = self.selected.len();

        let mut list = v_flex()
            .id("forward-chat-list")
            .min_h(px(FORWARD_LIST_MIN_HEIGHT))
            .mt(px(FORWARD_LIST_MARGIN_TOP))
            .flex_1()
            .overflow_y_scroll()
            .border_y_1()
            .border_color(theme.border);

        for chat in self.visible_chats() {
            let chat_id = chat.id.clone();
            let title = if chat.title.trim().is_empty() {
                chat.phone_number.clone()
            } else {
                chat.title.clone()
            };
            let preview = chat.last_message_preview.clone();
            let checked = self.is_selected(&chat_id);
            list = list.child(
                div()
                    .id(format!("forward-chat-row-{chat_id}"))
                    .flex()
                    .items_center()
                    .gap(px(FORWARD_ROW_GAP))
                    .w_full()
                    .px(px(FORWARD_ROW_PADDING_X))
                    .py(px(FORWARD_ROW_PADDING_Y))
                    .cursor_pointer()
                    .hover(|style| style.bg(hover))
                    // port: the oracle toggles the conversation of the row
                    // when the row is pressed.
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle(&chat_id);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .size(px(FORWARD_CHECK_SIZE))
                            .flex_shrink_0()
                            .rounded(radius * 0.5)
                            .border_1()
                            .border_color(if checked { theme.primary } else { theme.input })
                            .bg(if checked {
                                theme.primary
                            } else {
                                gpui::transparent_black()
                            })
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(checked, |check_box| {
                                check_box.child(
                                    Icon::new(IconName::Check)
                                        .with_size(px(FORWARD_CHECK_SIZE * 0.75))
                                        .text_color(theme.primary_foreground),
                                )
                            }),
                    )
                    .child(
                        v_flex()
                            .min_w_0()
                            .flex_1()
                            .child(
                                div()
                                    .truncate()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(foreground)
                                    .child(title),
                            )
                            .child(div().truncate().text_sm().text_color(muted).child(preview)),
                    ),
            );
        }

        let confirm_label = if chosen > 0 {
            format!("Forward to {chosen}")
        } else {
            "Forward".to_string()
        };

        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .bg(theme.overlay)
            .child(
                v_flex()
                    .w(px(FORWARD_DIALOG_WIDTH))
                    .max_h(px(FORWARD_DIALOG_MAX_HEIGHT))
                    .p(px(FORWARD_DIALOG_PADDING))
                    .rounded(theme.radius_lg)
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .shadow_lg()
                    .gap_2()
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .child(i18n::t("Forward message")),
                    )
                    .child(
                        div()
                            .mt(px(FORWARD_SEARCH_MARGIN_TOP))
                            .px(px(FORWARD_SEARCH_PADDING_X))
                            .py(px(FORWARD_SEARCH_PADDING_Y))
                            .rounded(radius)
                            .border_1()
                            .border_color(theme.border)
                            .bg(theme.background)
                            .child(Input::new(&self.search_input).w_full()),
                    )
                    .child(list)
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("forward-cancel")
                                    .label(i18n::t("Cancel"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.cancel(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("forward-confirm")
                                    .primary()
                                    .label(confirm_label)
                                    .disabled(!self.can_forward())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.confirm(window, cx);
                                    })),
                            ),
                    ),
            )
    }

    /// Reports an action to the handler of the app.
    fn report(&self, action: ForwardAction, window: &mut Window, cx: &mut App) {
        if let Some(handler) = self.on_action.clone() {
            handler(action, window, cx);
        }
    }
}

impl Render for ForwardPicker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        ForwardPicker::render(self, cx)
    }
}

// ---------------------------------------------------------------------------
// The form of the poll
// ---------------------------------------------------------------------------

/// The width of the form of the poll, before the scale.
///
/// port: `.poll-composer { width: min(360px, ...) }`.
const POLL_WIDTH: f32 = 360.0;

/// The padding of the form of the poll, before the scale.
///
/// port: `.poll-composer { padding: 14px }`.
const POLL_PADDING: f32 = 14.0;

/// The gap between the rows of the form of the poll, before the scale.
///
/// port: `.poll-composer { gap: 10px }`.
const POLL_GAP: f32 = 10.0;

/// The size of the text of the label of the multiple answer, before the scale.
///
/// port: `.poll-composer label { font-size: 13px }`.
const POLL_LABEL_FONT_SIZE: f32 = 13.0;

/// What the form of the poll asks the app to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PollAction {
    /// The user created the poll.
    Create {
        /// The question of the poll.
        question: String,
        /// The options of the poll, without the empty ones and without the
        /// duplicates.
        options: Vec<String>,
        /// How many options the user can pick.
        selectable_options_count: u32,
    },
    /// The user closed the form without creating the poll.
    Cancel,
}

/// The handler the form of the poll reports to.
pub(crate) type PollActionHandler = Option<Rc<dyn Fn(PollAction, &mut Window, &mut App)>>;

/// The state of the form of a poll, without the widgets.
///
/// port: the oracle keeps the question in a field and the options in a
/// textarea with one option per line. The native client draws one field per
/// option, so the options are a list.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PollForm {
    /// The question of the poll.
    question: String,
    /// The options of the poll.
    options: Vec<String>,
    /// Whether the user can pick more than one option.
    multiple: bool,
}

impl Default for PollForm {
    fn default() -> Self {
        // port: the oracle starts with "Yes\nNo" in the textarea of the
        // options.
        Self {
            question: String::new(),
            options: vec!["Yes".to_string(), "No".to_string()],
            multiple: false,
        }
    }
}

impl PollForm {
    /// The form of a poll with the options of the oracle.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// The question of the poll.
    pub(crate) fn question(&self) -> &str {
        &self.question
    }

    /// The question of the poll.
    pub(crate) fn set_question(&mut self, question: impl Into<String>) {
        self.question = question.into();
    }

    /// The options of the poll.
    pub(crate) fn options(&self) -> &[String] {
        &self.options
    }

    /// The text of one option of the poll.
    pub(crate) fn set_option(&mut self, index: usize, value: impl Into<String>) {
        if let Some(option) = self.options.get_mut(index) {
            *option = value.into();
        }
    }

    /// Adds an empty option to the poll.
    pub(crate) fn add_option(&mut self) {
        self.options.push(String::new());
    }

    /// Takes one option out of the poll. Returns whether it was there.
    pub(crate) fn remove_option(&mut self, index: usize) -> bool {
        if index >= self.options.len() {
            return false;
        }
        self.options.remove(index);
        true
    }

    /// Whether the user can pick more than one option.
    pub(crate) fn multiple(&self) -> bool {
        self.multiple
    }

    /// Whether the user can pick more than one option.
    pub(crate) fn set_multiple(&mut self, multiple: bool) {
        self.multiple = multiple;
    }

    /// The options of the poll that are not empty, without the duplicates.
    ///
    /// port: the oracle sends
    /// `[...new Set(options.map(trim).filter(Boolean))]`.
    pub(crate) fn unique_options(&self) -> Vec<String> {
        let mut unique: Vec<String> = Vec::new();
        for option in &self.options {
            let option = option.trim();
            if option.is_empty() || unique.iter().any(|other| other == option) {
                continue;
            }
            unique.push(option.to_string());
        }
        unique
    }

    /// The poll can be created: the question is there and there are two
    /// options at least.
    ///
    /// port: the oracle disables the button when the question is empty or
    /// when there are less than two options.
    pub(crate) fn can_create(&self) -> bool {
        !self.question.trim().is_empty() && self.unique_options().len() >= 2
    }

    /// How many options the user can pick.
    ///
    /// port: the oracle sends the number of the options when the poll allows
    /// more than one answer, and one when it does not.
    pub(crate) fn selectable_options_count(&self) -> u32 {
        if self.multiple {
            self.unique_options().len() as u32
        } else {
            1
        }
    }

    /// Empties the form and puts the options of the oracle back.
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

/// One option of the form of a poll.
struct PollOption {
    /// The identity of the option: the fields of the form move when an
    /// option is taken out, so the subscription of a field cannot rely on
    /// its position.
    id: u64,
    /// The field of the option.
    input: Entity<InputState>,
}

/// The form that creates a poll.
pub(crate) struct PollComposer {
    /// The state of the form.
    form: PollForm,
    /// The field of the question.
    question_input: Entity<InputState>,
    /// The fields of the options.
    options: Vec<PollOption>,
    /// The identity of the next option.
    next_option_id: u64,
    /// The subscription to the field of the question.
    _question_subscription: Subscription,
    /// The subscription to the field of every option.
    _option_subscriptions: HashMap<u64, Subscription>,
    /// The scale of the interface.
    scale: f32,
    /// The handler the form reports to.
    on_action: PollActionHandler,
}

impl std::fmt::Debug for PollComposer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PollComposer")
            .field("form", &self.form)
            .field("options", &self.options.len())
            .field("scale", &self.scale)
            .finish_non_exhaustive()
    }
}

impl PollComposer {
    /// The form of a poll.
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let question_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(i18n::t("Question")));
        let field = question_input.clone();
        let question_subscription =
            cx.subscribe_in(&question_input, window, move |this, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let question = field.read(cx).value().to_string();
                    this.form.set_question(question);
                }
            });

        let mut composer = Self {
            form: PollForm::new(),
            question_input,
            options: Vec::new(),
            next_option_id: 0,
            _question_subscription: question_subscription,
            _option_subscriptions: HashMap::new(),
            scale: 1.0,
            on_action: None,
        };
        // port: the oracle starts with two options, "Yes" and "No".
        let values = composer.form.options().to_vec();
        for value in values {
            composer.push_option(window, cx, value);
        }
        composer
    }

    /// The handler the form reports to.
    pub(crate) fn on_action(
        mut self,
        handler: impl Fn(PollAction, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_action = Some(Rc::new(handler));
        self
    }

    /// The scale of the interface.
    pub(crate) fn scale(mut self, scale: f32) -> Self {
        self.scale = normalized_ui_scale(scale);
        self
    }

    /// The state of the form.
    pub(crate) fn form(&self) -> &PollForm {
        &self.form
    }

    /// The state of the form.
    pub(crate) fn form_mut(&mut self) -> &mut PollForm {
        &mut self.form
    }

    /// How many options the form has.
    pub(crate) fn option_count(&self) -> usize {
        self.options.len()
    }

    /// The poll can be created.
    pub(crate) fn can_create(&self) -> bool {
        self.form.can_create()
    }

    /// Adds an option to the form.
    pub(crate) fn add_option(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.form.add_option();
        self.push_option(window, cx, String::new());
    }

    /// Takes one option out of the form.
    pub(crate) fn remove_option(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.options.len() {
            return;
        }
        let removed = self.options.remove(index);
        self._option_subscriptions.remove(&removed.id);
        self.form.remove_option(index);
        cx.notify();
    }

    /// The user created the poll.
    pub(crate) fn create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.form.can_create() {
            return;
        }
        let question = self.form.question().trim().to_string();
        let options = self.form.unique_options();
        let selectable_options_count = self.form.selectable_options_count();
        self.report(
            PollAction::Create {
                question,
                options,
                selectable_options_count,
            },
            window,
            cx,
        );
        // port: the oracle empties the form when the poll is created.
        self.reset(window, cx);
    }

    /// The user closed the form without creating the poll.
    pub(crate) fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.report(PollAction::Cancel, window, cx);
    }

    /// Empties the form and puts the options of the oracle back.
    pub(crate) fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.form.reset();
        let values = self.form.options().to_vec();
        for (option, value) in self.options.iter().zip(values) {
            option.input.update(cx, |input, cx| {
                input.set_value(value, window, cx);
            });
        }
        self.question_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
    }

    /// Creates the field of one option and keeps it in the form.
    fn push_option(&mut self, window: &mut Window, cx: &mut Context<Self>, value: String) {
        let id = self.next_option_id;
        self.next_option_id += 1;
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(i18n::t("Option")));
        input.update(cx, |input, cx| {
            input.set_value(value, window, cx);
        });
        let field = input.clone();
        let subscription = cx.subscribe_in(&input, window, move |this, _, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                let text = field.read(cx).value().to_string();
                if let Some(index) = this.option_index(id) {
                    this.form.set_option(index, text);
                }
            }
        });
        self._option_subscriptions.insert(id, subscription);
        self.options.push(PollOption { id, input });
    }

    /// The position of the option of an identity.
    fn option_index(&self, id: u64) -> Option<usize> {
        self.options.iter().position(|option| option.id == id)
    }

    /// The form of the poll.
    pub(crate) fn render(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        let mut options = v_flex().gap(px(POLL_GAP));
        for (index, option) in self.options.iter().enumerate() {
            let input = option.input.clone();
            options = options.child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(Input::new(&input).flex_1())
                    .child(
                        Button::new(("poll-option-remove", index))
                            .small()
                            .label("×")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.remove_option(index, cx);
                            })),
                    ),
            );
        }

        v_flex()
            .w(px(POLL_WIDTH))
            .p(px(POLL_PADDING))
            .gap(px(POLL_GAP))
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .shadow_lg()
            .text_color(theme.popover_foreground)
            .child(
                div()
                    .font_weight(FontWeight::BOLD)
                    .child(i18n::t("Create a poll")),
            )
            .child(Input::new(&self.question_input).w_full())
            .child(options)
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(POLL_LABEL_FONT_SIZE))
                    .child(
                        Checkbox::new("poll-multiple")
                            .checked(self.form.multiple())
                            .label(i18n::t("Allow multiple answers"))
                            .on_click(cx.listener(|this, checked, _, cx| {
                                this.form.set_multiple(*checked);
                                cx.notify();
                            })),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("poll-add-option")
                            .label(i18n::t("Add option"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.add_option(window, cx);
                            })),
                    )
                    .child(
                        Button::new("poll-cancel")
                            .label(i18n::t("Cancel"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.cancel(window, cx);
                            })),
                    )
                    .child(
                        Button::new("poll-create")
                            .primary()
                            .label(i18n::t("Create poll"))
                            .disabled(!self.can_create())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.create(window, cx);
                            })),
                    ),
            )
    }

    /// Reports an action to the handler of the app.
    fn report(&self, action: PollAction, window: &mut Window, cx: &mut App) {
        if let Some(handler) = self.on_action.clone() {
            handler(action, window, cx);
        }
    }
}

impl Render for PollComposer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        PollComposer::render(self, cx)
    }
}

// ---------------------------------------------------------------------------
// The button of the clip
// ---------------------------------------------------------------------------

/// The width of the menu of the clip, before the scale.
///
/// port: `.attachment-menu { width: calc(220px * var(--scale)) }`.
const ATTACHMENT_MENU_WIDTH: f32 = 220.0;

/// The padding of the menu of the clip, before the scale.
///
/// port: `.attachment-menu { padding: calc(6px * var(--scale)) }`.
const ATTACHMENT_MENU_PADDING: f32 = 6.0;

/// The smallest height of a row of the menu of the clip, before the scale.
///
/// port: `.popover-menu-item { min-height: calc(40px * var(--scale)) }`.
const ATTACHMENT_ITEM_MIN_HEIGHT: f32 = 40.0;

/// The gap of a row of the menu of the clip, before the scale.
///
/// port: `.popover-menu-item { gap: calc(10px * var(--scale)) }`.
const ATTACHMENT_ITEM_GAP: f32 = 10.0;

/// The padding of a row of the menu of the clip, before the scale.
///
/// port: `.popover-menu-item { padding: 0 calc(10px * var(--scale)) }`.
const ATTACHMENT_ITEM_PADDING_X: f32 = 10.0;

/// The size of the glyph of the button of the clip, before the scale.
///
/// port: `<ThemeIcon icon={Paperclip} name="attach" size={22} />`.
const ATTACHMENT_ICON_SIZE: f32 = 22.0;

/// What the button of the clip asks the app to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AttachmentAction {
    /// The user chose a file: it goes to the open conversation.
    Send {
        /// The file the user chose.
        path: PathBuf,
        /// The kind of the file.
        kind: AttachmentKind,
    },
    /// The user asked to open a file that was already downloaded.
    Open {
        /// The file to open.
        path: PathBuf,
    },
    /// The user saved a file that was already downloaded.
    Saved {
        /// Where the file was saved.
        path: PathBuf,
    },
    /// The file could not be saved.
    SaveFailed {
        /// Why the file could not be saved.
        message: String,
    },
    /// The user asked for a picture of the device.
    Photo,
    /// The user asked for a poll.
    Poll,
    /// The user asked for the tray of the stickers.
    Sticker,
    /// The user asked to make a sticker of a picture.
    MakeSticker,
}

/// The handler the button of the clip reports to.
pub(crate) type AttachmentActionHandler =
    Option<Rc<dyn Fn(AttachmentAction, &mut Window, &mut App)>>;

/// One kind of file the user can send.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AttachmentKindItem {
    /// The kind of the file.
    pub(crate) kind: AttachmentKind,
    /// The label of the row.
    ///
    /// port: the labels of the oracle: "Document", "Video" and "Audio".
    pub(crate) label: &'static str,
    /// The title of the selector of files.
    ///
    /// port: the oracle asks "Choose documents", "Choose videos" and "Choose
    /// audio files".
    pub(crate) prompt: &'static str,
}

/// The kinds of file the button of the clip can send.
///
/// port: the menu of the oracle has a document, a video and an audio.
pub(crate) fn attachment_kinds() -> Vec<AttachmentKindItem> {
    vec![
        AttachmentKindItem {
            kind: AttachmentKind::Document,
            label: "Document",
            prompt: "Choose documents",
        },
        AttachmentKindItem {
            kind: AttachmentKind::Video,
            label: "Video",
            prompt: "Choose videos",
        },
        AttachmentKindItem {
            kind: AttachmentKind::Audio,
            label: "Audio",
            prompt: "Choose audio files",
        },
    ]
}

/// The button of the clip of the composer.
///
/// The button opens the selector of files of the system and reports the file
/// the user chose; the app sends it with `send_attachment`.
pub(crate) struct AttachmentBar {
    /// The menu of the kinds is open.
    open: bool,
    /// The button can be pressed.
    enabled: bool,
    /// The scale of the interface.
    scale: f32,
    /// The handler the button reports to.
    on_action: AttachmentActionHandler,
}

impl std::fmt::Debug for AttachmentBar {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AttachmentBar")
            .field("open", &self.open)
            .field("enabled", &self.enabled)
            .field("scale", &self.scale)
            .finish_non_exhaustive()
    }
}

impl AttachmentBar {
    /// The button of the clip.
    pub(crate) fn new(scale: f32) -> Self {
        Self {
            open: false,
            enabled: true,
            scale: normalized_ui_scale(scale),
            on_action: None,
        }
    }

    /// The handler the button reports to.
    pub(crate) fn on_action(
        mut self,
        handler: impl Fn(AttachmentAction, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_action = Some(Rc::new(handler));
        self
    }

    /// The button can be pressed.
    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    /// The button can be pressed, or it cannot.
    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// The menu of the kinds is open.
    pub(crate) fn open(&self) -> bool {
        self.open
    }

    /// The menu of the kinds is open, or it is closed.
    pub(crate) fn set_open(&mut self, open: bool) {
        self.open = open;
    }

    /// The scale of the interface.
    pub(crate) fn scale(&self) -> f32 {
        self.scale
    }

    /// Opens the selector of files of the system and sends the file the user
    /// chose.
    ///
    /// The app answers to [`AttachmentAction::Send`] with `send_attachment`.
    pub(crate) fn choose(
        &mut self,
        kind: AttachmentKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.enabled {
            return;
        }
        let prompt = attachment_kinds()
            .into_iter()
            .find(|item| item.kind == kind)
            .map(|item| item.prompt.to_string())
            .unwrap_or_else(|| "Choose a file to send".to_string());
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(prompt.into()),
        });
        cx.spawn_in(window, async move |weak, window| {
            let path = selected.await.ok()?.ok()??.into_iter().next()?;
            window
                .update(|window, cx| {
                    weak.update(cx, |this, cx| {
                        this.report(AttachmentAction::Send { path, kind }, window, cx);
                        cx.notify();
                    })
                })
                .ok()
        })
        .detach();
    }

    /// Opens a file that was already downloaded with the program of the
    /// system.
    pub(crate) fn open_attachment(path: &Path, cx: &mut App) {
        cx.open_with_system(path);
    }

    /// Asks the user where to save a file that was already downloaded and
    /// copies it there.
    pub(crate) fn save_attachment(
        &mut self,
        path: PathBuf,
        file_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let directory = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let selected = cx.prompt_for_new_path(&directory, Some(file_name.as_str()));
        cx.spawn_in(window, async move |weak, window| {
            let destination = selected.await.ok()?.ok()??;
            let outcome = std::fs::copy(&path, &destination);
            window
                .update(|window, cx| {
                    weak.update(cx, |this, cx| {
                        let action = match outcome {
                            Ok(_) => AttachmentAction::Saved { path: destination },
                            Err(error) => AttachmentAction::SaveFailed {
                                message: error.to_string(),
                            },
                        };
                        this.report(action, window, cx);
                        cx.notify();
                    })
                })
                .ok()
        })
        .detach();
    }

    /// The button of the clip and its menu of the kinds.
    pub(crate) fn render(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let scale = self.scale;
        let theme = cx.theme();
        let foreground = theme.foreground;
        let muted = theme.muted_foreground;
        let hover = theme.muted;
        let radius = theme.radius;

        let mut bar = h_flex().items_center().gap_2().relative().child(
            div()
                .id("attachment-button")
                .flex()
                .items_center()
                .justify_center()
                .size(px(ATTACHMENT_ICON_SIZE * 2.))
                .rounded(radius)
                .cursor_pointer()
                .text_color(if self.enabled { foreground } else { muted })
                .opacity(if self.enabled { 1.0 } else { 0.5 })
                .when(self.enabled, |button| {
                    button
                        .hover(|style| style.bg(hover))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_open(!this.open);
                            cx.notify();
                        }))
                })
                .aria_label("Attach")
                .child(Icon::new(WhatfastIcon::Attach).with_size(px(ATTACHMENT_ICON_SIZE * scale))),
        );

        if self.open {
            let mut menu = v_flex()
                .absolute()
                .bottom(px(ATTACHMENT_ICON_SIZE * 2.))
                .left_0()
                .w(px(ATTACHMENT_MENU_WIDTH * scale))
                .p(px(ATTACHMENT_MENU_PADDING * scale))
                .rounded(radius)
                .border_1()
                .border_color(theme.border)
                .bg(theme.popover)
                .shadow_lg();
            // The kinds of the files the user can send.
            for item in attachment_kinds() {
                let kind = item.kind;
                menu = menu.child(
                    div()
                        .id(format!("attachment-kind-{}", item.label))
                        .flex()
                        .items_center()
                        .gap(px(ATTACHMENT_ITEM_GAP * scale))
                        .w_full()
                        .min_h(px(ATTACHMENT_ITEM_MIN_HEIGHT * scale))
                        .px(px(ATTACHMENT_ITEM_PADDING_X * scale))
                        .rounded(radius)
                        .cursor_pointer()
                        .hover(|style| style.bg(hover))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.set_open(false);
                            this.choose(kind, window, cx);
                            cx.notify();
                        }))
                        .child(i18n::t(item.label)),
                );
            }
            // The things the app can do that are not files: the picture of
            // the device, the poll and the stickers.
            for (label, action) in [
                ("Photo or video", AttachmentAction::Photo),
                ("Poll", AttachmentAction::Poll),
                ("Sticker", AttachmentAction::Sticker),
                ("Make a sticker", AttachmentAction::MakeSticker),
            ] {
                menu = menu.child(
                    div()
                        .id(format!("attachment-action-{label}"))
                        .flex()
                        .items_center()
                        .gap(px(ATTACHMENT_ITEM_GAP * scale))
                        .w_full()
                        .min_h(px(ATTACHMENT_ITEM_MIN_HEIGHT * scale))
                        .px(px(ATTACHMENT_ITEM_PADDING_X * scale))
                        .rounded(radius)
                        .cursor_pointer()
                        .hover(|style| style.bg(hover))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.set_open(false);
                            this.report(action.clone(), window, cx);
                            cx.notify();
                        }))
                        .child(label),
                );
            }
            bar = bar.child(menu);
        }

        bar
    }

    /// Reports an action to the handler of the app.
    fn report(&self, action: AttachmentAction, window: &mut Window, cx: &mut App) {
        if let Some(handler) = self.on_action.clone() {
            handler(action, window, cx);
        }
    }
}

impl Render for AttachmentBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        AttachmentBar::render(self, cx)
    }
}

// ---------------------------------------------------------------------------
// The tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use gpui::{point, px};
    use whatfast_protocol::message::Content;
    use whatfast_protocol::{Chat, ChatKind, Message, TextContent};

    use super::{
        AttachmentBar, EDIT_WINDOW_MS, MessageAction, MessageMenu, MessageMenuItem, PollForm,
        attachment_kinds, chat_matches, filter_chats, floating_origin, toggle_selection,
    };

    /// A message of the user, with a text.
    fn own_message(timestamp_ms: i64) -> Message {
        Message {
            id: "message-1".to_string(),
            chat_id: "chat-1".to_string(),
            from_me: true,
            timestamp_ms,
            content: Some(Content::Text(TextContent {
                text: "hello".to_string(),
                ..TextContent::default()
            })),
            ..Message::default()
        }
    }

    /// A message of somebody else, with a text.
    fn other_message(timestamp_ms: i64) -> Message {
        Message {
            from_me: false,
            ..own_message(timestamp_ms)
        }
    }

    /// A conversation.
    fn chat(id: &str, title: &str, phone: &str) -> Chat {
        Chat {
            id: id.to_string(),
            kind: ChatKind::Direct as i32,
            title: title.to_string(),
            phone_number: phone.to_string(),
            ..Chat::default()
        }
    }

    /// The action of a row of a menu.
    fn action_of(item: &MessageMenuItem) -> MessageAction {
        item.action
    }

    /// Whether a row of a menu can be pressed.
    fn enabled(menu: &MessageMenu, action: MessageAction) -> bool {
        menu.items()
            .into_iter()
            .find(|item| item.action == action)
            .is_some_and(|item| item.enabled)
    }

    #[test]
    fn the_menu_has_the_eight_rows_of_the_oracle() {
        let menu = MessageMenu::new("chat-1", &own_message(0), false, 0, 1.0);
        let actions: Vec<MessageAction> = menu.items().iter().map(action_of).collect();
        assert_eq!(
            actions,
            vec![
                MessageAction::Reply,
                MessageAction::Forward,
                MessageAction::Edit,
                MessageAction::Copy,
                MessageAction::Pin,
                MessageAction::Unpin,
                MessageAction::Delete,
                MessageAction::DeleteForEveryone,
            ]
        );
    }

    #[test]
    fn the_menu_is_the_same_for_a_message_of_the_user_and_for_one_of_another() {
        let mine = MessageMenu::new("chat-1", &own_message(0), false, 0, 1.0);
        let other = MessageMenu::new("chat-1", &other_message(0), false, 0, 1.0);
        let labels = |menu: &MessageMenu| {
            menu.items()
                .into_iter()
                .map(|item| item.label)
                .collect::<Vec<&str>>()
        };
        assert_eq!(labels(&mine), labels(&other));
    }

    #[test]
    fn the_rows_of_a_message_of_another_cannot_be_pressed() {
        let menu = MessageMenu::new("chat-1", &other_message(0), false, 0, 1.0);
        assert!(enabled(&menu, MessageAction::Reply));
        assert!(enabled(&menu, MessageAction::Forward));
        assert!(enabled(&menu, MessageAction::Copy));
        // port: only the messages of the user can be edited or deleted for
        // everybody.
        assert!(!enabled(&menu, MessageAction::Edit));
        assert!(!enabled(&menu, MessageAction::DeleteForEveryone));
        assert!(enabled(&menu, MessageAction::Delete));
    }

    #[test]
    fn the_rows_of_a_message_of_the_user_can_be_pressed() {
        let menu = MessageMenu::new("chat-1", &own_message(0), false, 0, 1.0);
        assert!(enabled(&menu, MessageAction::Edit));
        assert!(enabled(&menu, MessageAction::DeleteForEveryone));
    }

    #[test]
    fn the_edit_is_closed_after_the_window_of_twenty_minutes() {
        let now = 1_000_000i64;
        let fresh = MessageMenu::new("chat-1", &own_message(now), false, now, 1.0);
        assert!(fresh.editable());
        assert_eq!(fresh.edit_deadline_ms(), now + EDIT_WINDOW_MS);

        let late = MessageMenu::new(
            "chat-1",
            &own_message(now),
            false,
            now + EDIT_WINDOW_MS,
            1.0,
        );
        assert!(!late.editable());
        assert!(!enabled(&late, MessageAction::Edit));
    }

    #[test]
    fn the_edit_is_closed_when_the_message_is_not_a_text() {
        let mut message = own_message(0);
        message.content = Some(Content::Image(Default::default()));
        let menu = MessageMenu::new("chat-1", &message, false, 0, 1.0);
        assert!(!menu.editable());
    }

    #[test]
    fn a_pinned_message_can_be_unpinned_and_not_pinned_again() {
        let pinned = MessageMenu::new("chat-1", &own_message(0), true, 0, 1.0);
        assert!(enabled(&pinned, MessageAction::Unpin));
        assert!(!enabled(&pinned, MessageAction::Pin));

        let unpinned = MessageMenu::new("chat-1", &own_message(0), false, 0, 1.0);
        assert!(enabled(&unpinned, MessageAction::Pin));
        assert!(!enabled(&unpinned, MessageAction::Unpin));
    }

    #[test]
    fn a_deleted_message_cannot_be_deleted_again() {
        let mut message = own_message(0);
        message.revoked = true;
        let menu = MessageMenu::new("chat-1", &message, false, 0, 1.0);
        assert!(!enabled(&menu, MessageAction::Delete));
        assert!(!enabled(&menu, MessageAction::DeleteForEveryone));
        assert!(!enabled(&menu, MessageAction::Forward));
    }

    #[test]
    fn a_message_without_content_cannot_be_forwarded() {
        let message = Message {
            id: "message-1".to_string(),
            from_me: true,
            ..Message::default()
        };
        let menu = MessageMenu::new("chat-1", &message, false, 0, 1.0);
        assert!(!enabled(&menu, MessageAction::Forward));
        assert!(!enabled(&menu, MessageAction::Pin));
    }

    #[test]
    fn the_menu_keeps_the_message_it_is_of() {
        let menu = MessageMenu::new("chat-7", &own_message(0), false, 0, 1.0);
        assert_eq!(menu.chat_id(), "chat-7");
        assert_eq!(menu.message_id(), "message-1");
        assert!(menu.is_from_me());
        assert!(!menu.pinned());
        // port: the copy of the message puts the text of the message in the
        // clipboard.
        assert_eq!(menu.text(), "hello");

        let mut pinned = MessageMenu::new("chat-7", &own_message(0), false, 0, 1.0);
        pinned.set_pinned(true);
        assert!(pinned.pinned());
    }

    #[test]
    fn the_search_of_the_forward_finds_the_title_and_the_phone() {
        let chats = vec![
            chat("chat-1", "Fast friend", "5493800000001"),
            chat("chat-2", "Family", "5493800000002"),
        ];
        assert_eq!(filter_chats(&chats, "fast").len(), 1);
        assert_eq!(filter_chats(&chats, "FRIEND").len(), 1);
        assert_eq!(filter_chats(&chats, "5493800000002").len(), 1);
        assert_eq!(filter_chats(&chats, "").len(), 2);
        assert_eq!(filter_chats(&chats, "  ").len(), 2);
        assert_eq!(filter_chats(&chats, "nobody").len(), 0);
    }

    #[test]
    fn the_search_of_the_forward_ignores_the_case_and_the_spaces_around() {
        let chats = [chat("chat-1", "Fast friend", "5493800000001")];
        assert!(chat_matches(&chats[0], "fast friend"));
        assert!(chat_matches(&chats[0], ""));
        assert!(!chat_matches(&chats[0], "family"));
    }

    #[test]
    fn the_selection_of_the_forward_adds_and_takes_out() {
        let mut selected = Vec::new();
        toggle_selection(&mut selected, "chat-1");
        assert_eq!(selected, vec!["chat-1".to_string()]);
        toggle_selection(&mut selected, "chat-2");
        assert_eq!(selected.len(), 2);
        toggle_selection(&mut selected, "chat-1");
        assert_eq!(selected, vec!["chat-2".to_string()]);
    }

    #[test]
    fn the_poll_starts_with_the_options_of_the_oracle() {
        let form = PollForm::new();
        assert_eq!(form.question(), "");
        assert_eq!(form.options(), &["Yes".to_string(), "No".to_string()]);
        assert!(!form.multiple());
    }

    #[test]
    fn the_poll_needs_a_question_and_two_options() {
        let mut form = PollForm::new();
        assert!(!form.can_create());
        form.set_question("Where do we eat?");
        assert!(form.can_create());
        form.set_option(1, "");
        assert!(!form.can_create());
        form.set_option(1, "No");
        assert!(form.can_create());
    }

    #[test]
    fn the_options_of_the_poll_are_trimmed_and_without_duplicates() {
        let mut form = PollForm::new();
        form.set_option(0, " Yes ");
        form.set_option(1, "Yes");
        // port: the oracle compares the options as they are, so two options
        // that differ in the case are two options.
        assert_eq!(form.unique_options(), vec!["Yes".to_string()]);
        form.set_option(1, "");
        assert!(!form.can_create());
    }

    #[test]
    fn the_options_of_the_poll_can_be_added_and_taken_out() {
        let mut form = PollForm::new();
        form.add_option();
        assert_eq!(form.options().len(), 3);
        assert!(form.remove_option(2));
        assert_eq!(form.options().len(), 2);
        assert!(!form.remove_option(9));
    }

    #[test]
    fn the_poll_allows_one_answer_unless_it_is_asked_for_more() {
        let mut form = PollForm::new();
        form.set_question("Where do we eat?");
        assert_eq!(form.selectable_options_count(), 1);
        form.set_multiple(true);
        assert_eq!(form.selectable_options_count(), 2);
    }

    #[test]
    fn the_poll_is_empty_again_after_it_is_created() {
        let mut form = PollForm::new();
        form.set_question("Where do we eat?");
        form.reset();
        assert_eq!(form.question(), "");
        assert_eq!(form.options(), &["Yes".to_string(), "No".to_string()]);
    }

    #[test]
    fn the_menu_of_the_clip_has_the_three_kinds_of_the_oracle() {
        let kinds = attachment_kinds();
        assert_eq!(kinds.len(), 3);
        assert_eq!(kinds[0].label, "Document");
        assert_eq!(kinds[1].label, "Video");
        assert_eq!(kinds[2].label, "Audio");
        assert_eq!(
            kinds.iter().map(|item| item.prompt).collect::<Vec<&str>>(),
            vec!["Choose documents", "Choose videos", "Choose audio files"]
        );
    }

    #[test]
    fn the_button_of_the_clip_opens_and_closes_its_menu() {
        let mut bar = AttachmentBar::new(1.0);
        assert!(!bar.open());
        bar.set_open(true);
        assert!(bar.open());
        bar.set_open(false);
        assert!(!bar.open());
        assert!(bar.enabled());
        bar.set_enabled(false);
        assert!(!bar.enabled());
    }

    /// The panel of the menu is drawn where the pointer is: the bug was that it
    /// landed below the app, as wide as the window.
    #[test]
    fn the_menu_opens_where_the_pointer_is() {
        let origin = floating_origin(
            point(px(320.), px(240.)),
            gpui::size(px(1000.), px(700.)),
            px(230.),
            px(340.),
            px(8.),
        );
        assert_eq!(origin, point(px(320.), px(240.)));
    }

    #[test]
    fn the_menu_does_not_leave_the_window() {
        let origin = floating_origin(
            point(px(980.), px(690.)),
            gpui::size(px(1000.), px(700.)),
            px(230.),
            px(340.),
            px(8.),
        );
        assert_eq!(origin, point(px(762.), px(352.)));
    }

    #[test]
    fn a_window_smaller_than_the_menu_keeps_the_margin() {
        let origin = floating_origin(
            point(px(300.), px(300.)),
            gpui::size(px(200.), px(180.)),
            px(230.),
            px(340.),
            px(8.),
        );
        assert_eq!(origin, point(px(8.), px(8.)));
    }
}
