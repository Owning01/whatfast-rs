//! The strip of the statuses of the contacts, and the viewer of one status.
//!
//! port: taken from the origin's `tauri/src/components/StatusStrip.tsx`. The
//! lengths come from the stylesheet of the origin (`tauri/src/styles.css`,
//! the classes `status-strip`, `status-ring`, `status-avatar`,
//! `status-viewer` and `status-progress`).
//!
//! The data is not here: the groups of the statuses live in the store of the
//! application (`store.statuses()`), and the panel only draws them. The
//! action it needs already exists in `app.rs`: `load_statuses()`.
//!
//! Two things of the origin stay in the memory of the client, and they stay
//! here too: the text status and the photo status, because a linked device
//! has no RPC for a status of the user.

#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AppContext as _, Bounds, Context, ElementId, Entity, FontWeight, Hsla, InteractiveElement as _,
    ObjectFit, ParentElement as _, PathBuilder, PathPromptOptions, StatefulInteractiveElement as _,
    Styled as _, StyledImage as _, Subscription, Window, canvas, div, img, point, px, quad, rgba,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use smol::Timer;

use whatfast_core::message_format::{DateLocale, LocalTimestamp, format_day, format_time};
use whatfast_model::model::message_text;
use whatfast_protocol::{
    ImageContent, Message, MessageStatus, StatusGroup, TextContent, message::Content,
};

use crate::app::Whatfast;
use crate::icons::WhatfastIcon;
use crate::tokens::parse_color;

/// How long a status stays open before the viewer moves to the next one.
///
/// port: `STATUS_STEP_MS` of the origin.
pub(crate) const STATUS_STEP_MS: u64 = 3_000;
/// The side of the ring of a status, in pixels, before the scale.
pub(crate) const RING_SIZE: f32 = 46.0;
/// The side of the avatar of the viewer, in pixels, before the scale.
pub(crate) const VIEWER_AVATAR_SIZE: f32 = 34.0;
/// The width of the ring of a status, in pixels, before the scale.
pub(crate) const RING_BORDER: f32 = 3.0;
/// The length of every dash of the ring of a status that was not seen, in
/// pixels, before the scale.
///
/// The reference cuts the circle of a new status in short dashes: about
/// twenty four of them around the ring.
pub(crate) const RING_DASH: f32 = 3.4;
/// The opening between two dashes of the ring of a status that was not seen,
/// in pixels, before the scale.
pub(crate) const RING_DASH_GAP: f32 = 2.4;
/// The gap between two rings, in pixels, before the scale.
pub(crate) const RING_GAP: f32 = 10.0;
/// The height of a segment of the progress bar, in pixels, before the scale.
pub(crate) const PROGRESS_HEIGHT: f32 = 3.0;
/// The side of the buttons that move between the statuses, in pixels, before
/// the scale.
pub(crate) const NAV_SIZE: f32 = 48.0;
/// The side of the big drawing of the panel of the states, in pixels, before
/// the scale.
///
/// The reference draws the circle of dashes of the states at the top of the
/// panel, above the title.
pub(crate) const PANEL_GLYPH_SIZE: f32 = 52.0;
/// The width of the line of the circle of the big drawing, in pixels, before
/// the scale.
pub(crate) const PANEL_GLYPH_STROKE: f32 = 5.0;
/// How many degrees covers one of the three arcs of the big drawing.
///
/// The reference does not draw a closed circle: it draws three arcs of about
/// a hundred degrees, with the opening of the first one at the top.
pub(crate) const PANEL_GLYPH_ARC_DEGREES: f32 = 87.0;
/// How many degrees covers one of the three openings between the arcs of the
/// big drawing.
pub(crate) const PANEL_GLYPH_GAP_DEGREES: f32 = 33.0;
/// Where the first arc of the big drawing starts, in degrees, measured from
/// the top and turning to the right.
pub(crate) const PANEL_GLYPH_FIRST_ARC_DEGREES: f32 = 16.5;
/// The side of the solid circle in the middle of the big drawing, in pixels,
/// before the scale.
pub(crate) const PANEL_GLYPH_DOT_SIZE: f32 = 32.0;
/// The side of the title of the panel, in pixels, before the scale.
///
/// The reference writes the title big: it is the biggest text of the panel.
pub(crate) const PANEL_TITLE_SIZE: f32 = 30.0;
/// The width of the round buttons of the panel, in pixels, before the scale.
pub(crate) const PANEL_BUTTON_WIDTH: f32 = 60.0;
/// The height of the round buttons of the panel, in pixels, before the scale.
pub(crate) const PANEL_BUTTON_HEIGHT: f32 = 48.0;
/// The radius of the corners of the round buttons of the panel, in pixels,
/// before the scale.
pub(crate) const PANEL_BUTTON_RADIUS: f32 = 21.0;
/// The space between the two round buttons of the panel, in pixels, before
/// the scale.
pub(crate) const PANEL_BUTTON_GAP: f32 = 48.0;
/// The side of the drawing inside the round buttons of the panel, in pixels,
/// before the scale.
pub(crate) const PANEL_BUTTON_ICON_SIZE: f32 = 20.0;
/// The space between a round button of the panel and its text, in pixels,
/// before the scale.
pub(crate) const PANEL_BUTTON_LABEL_GAP: f32 = 6.0;
/// The side of the green badge of the row of "My status", in pixels, before
/// the scale.
pub(crate) const OWN_BADGE_SIZE: f32 = 16.0;
/// The space between the two rows of a status, in pixels, before the scale.
///
/// The reference makes every row seventy two pixels tall: the ring of the
/// status and thirteen pixels above and below.
pub(crate) const STATUS_ROW_PADDING: f32 = 13.0;
/// The longest text of a status, in characters.
///
/// port: `MAX_STATUS_LENGTH` of the origin.
pub(crate) const MAX_STATUS_LENGTH: usize = 700;
/// The background of a text status when the user does not pick another one.
pub(crate) const DEFAULT_STATUS_COLOR: &str = "#0f766e";
/// The colors the user can pick for a text status.
pub(crate) const STATUS_COLORS: [&str; 6] = [
    "#0f766e", "#1d4ed8", "#7c3aed", "#b91c1c", "#a16207", "#334155",
];

/// The status that is open in the viewer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct StatusViewer {
    /// The group of the status that is open.
    pub(crate) group_index: usize,
    /// The message of the group that is open.
    pub(crate) message_index: usize,
}

/// The state of the strip of the statuses.
#[derive(Default)]
pub(crate) struct StatusStripState {
    /// The status that is open in the viewer.
    pub(crate) viewer: Option<StatusViewer>,
    /// The background of every status that was published locally.
    pub(crate) colors: HashMap<String, String>,
    /// The senders of the statuses the user already opened.
    ///
    /// The reference draws the ring of a status in green when the user saw
    /// it, and dashed when the status is new. The client of the origin asks
    /// the sidecar to mark them, but the model of this client has no such
    /// field, so the ring follows the viewer of this session.
    pub(crate) seen: HashSet<String>,
    /// The color the next text status will have.
    pub(crate) text_color: String,
    /// How many times the viewer was opened. The task that moves the viewer
    /// stops when the counter moves, so the task of one viewer does not move
    /// the next one.
    pub(crate) generation: u64,
    /// Whether the statuses were already asked for.
    pub(crate) requested: bool,
    /// Whether the composer of a text status is open in the panel.
    ///
    /// The reference draws two buttons, one for the text and one for the
    /// photo: the composer of the text only shows when the first is pressed.
    pub(crate) text_composer_open: bool,
}

impl StatusStripState {
    /// The state of a strip that was never used.
    pub(crate) fn new() -> Self {
        Self {
            text_color: DEFAULT_STATUS_COLOR.to_string(),
            ..Self::default()
        }
    }
}

/// Marks the status of one sender as seen.
///
/// The ring of a sender that was seen is solid; the ring of a sender that was
/// never opened is dashed.
pub(crate) fn mark_seen(state: &mut StatusStripState, sender_id: &str) {
    state.seen.insert(sender_id.to_string());
}

/// Whether the status of one sender was already seen.
pub(crate) fn has_seen(state: &StatusStripState, sender_id: &str) -> bool {
    state.seen.contains(sender_id)
}

/// The letter that stands for a contact in the ring of its status.
///
/// port: `avatarLabel` of the origin: the first letter of the name, in upper
/// case, or a question mark when the name has no letters at all.
pub(crate) fn avatar_label(value: &str) -> String {
    let trimmed = value.trim();
    match trimmed.chars().next() {
        Some(letter) => letter.to_uppercase().collect(),
        None => "?".to_string(),
    }
}

/// The name that stands for the sender of a group of statuses.
///
/// port: the origin writes `group.senderName || group.senderPhoneNumber`.
pub(crate) fn sender_label(group: &StatusGroup) -> String {
    if !group.sender_name.trim().is_empty() {
        group.sender_name.clone()
    } else {
        group.sender_phone_number.clone()
    }
}

/// Moves the viewer to the next status.
///
/// port: `advance` of the origin: first the next message of the group, then
/// the first message of the next group. When there is nothing left the
/// viewer closes.
///
/// Returns whether the viewer stays open.
pub(crate) fn advance_viewer(viewer: &mut StatusViewer, groups: &[StatusGroup]) -> bool {
    let Some(group) = groups.get(viewer.group_index) else {
        return false;
    };
    if viewer.message_index + 1 < group.messages.len() {
        viewer.message_index += 1;
        return true;
    }
    if viewer.group_index + 1 < groups.len() {
        viewer.group_index += 1;
        viewer.message_index = 0;
        return true;
    }
    false
}

/// Moves the viewer to the previous status.
///
/// port: the "previous" button of the origin: the previous message of the
/// group, or the last message of the previous group. The viewer does not
/// move when there is no previous status.
///
/// Returns whether the viewer moved.
pub(crate) fn previous_viewer(viewer: &mut StatusViewer, groups: &[StatusGroup]) -> bool {
    if viewer.message_index > 0 {
        viewer.message_index -= 1;
        return true;
    }
    if viewer.group_index > 0 {
        let count = groups
            .get(viewer.group_index - 1)
            .map(|group| group.messages.len())
            .unwrap_or(0);
        viewer.group_index -= 1;
        viewer.message_index = count.saturating_sub(1);
        return true;
    }
    false
}

/// The message of a text status published locally.
///
/// port: `publishTextStatus` of the origin: the text is capped at seven
/// hundred characters and the message stays in the memory of the client,
/// because a linked device has no RPC for a status of text.
pub(crate) fn text_status_message(chat_id: &str, text: &str, timestamp_ms: i64) -> Message {
    let capped: String = text.chars().take(MAX_STATUS_LENGTH).collect();
    Message {
        id: format!("local-status-{timestamp_ms}"),
        chat_id: chat_id.to_string(),
        sender_id: "me@s.whatsapp.net".to_string(),
        sender_name: "You".to_string(),
        from_me: true,
        timestamp_ms,
        status: MessageStatus::Sent as i32,
        content: Some(Content::Text(TextContent {
            text: capped,
            link_preview: None,
        })),
        ..Default::default()
    }
}

/// The message of a photo status published locally.
///
/// port: the origin sends the photo to the conversation of the statuses; a
/// linked device has no RPC for that either, so the photo stays in the
/// memory of the client.
pub(crate) fn photo_status_message(chat_id: &str, path: &str, timestamp_ms: i64) -> Message {
    Message {
        id: format!("local-status-{timestamp_ms}"),
        chat_id: chat_id.to_string(),
        sender_id: "me@s.whatsapp.net".to_string(),
        sender_name: "You".to_string(),
        from_me: true,
        timestamp_ms,
        status: MessageStatus::Sent as i32,
        content: Some(Content::Image(ImageContent {
            caption: String::new(),
            mime_type: "image/jpeg".to_string(),
            local_path: path.to_string(),
            thumbnail_path: path.to_string(),
            ..Default::default()
        })),
        ..Default::default()
    }
}

/// Adds one message to the groups of the statuses.
///
/// port: the origin appends the message to the group of the sender, and it
/// opens a new group at the front when the sender has none.
pub(crate) fn add_status_message(groups: &mut Vec<StatusGroup>, message: Message) {
    if let Some(group) = groups
        .iter_mut()
        .find(|group| group.sender_id == message.sender_id)
    {
        group.messages.push(message);
        return;
    }
    groups.insert(
        0,
        StatusGroup {
            sender_id: message.sender_id.clone(),
            sender_name: message.sender_name.clone(),
            sender_phone_number: message.sender_phone_number.clone(),
            messages: vec![message],
        },
    );
}

/// The path of the picture of a message, when it has one.
pub(crate) fn message_image_path(message: &Message) -> Option<String> {
    let Some(Content::Image(image)) = &message.content else {
        return None;
    };
    let path = if image.thumbnail_path.is_empty() {
        image.local_path.as_str()
    } else {
        image.thumbnail_path.as_str()
    };
    (!path.is_empty()).then(|| path.to_string())
}

/// The current instant, in milliseconds of the epoch.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(0)
}

/// The time of a message.
///
/// port: `formatTime` of the origin. The zone of the machine is not
/// available in this crate (there is no library of dates), so the hour is
/// the one of UTC.
fn message_time(timestamp_ms: i64) -> String {
    format_time(LocalTimestamp::new(timestamp_ms, 0), DateLocale::EnUs)
}

/// The time of a status, the way the reference writes it in the list.
///
/// port: the reference writes the day and the hour, the same way the origin
/// writes the day of a message (`formatDay` and `formatTime`). The hour
/// goes in the 24 hour clock, like in the reference: "Today at 12:45".
fn status_time(timestamp_ms: i64, now_ms: i64) -> String {
    let value = LocalTimestamp::new(timestamp_ms, 0);
    let now = LocalTimestamp::new(now_ms, 0);
    let day = format_day(value, now, DateLocale::EnUs);
    let hour = status_hour(timestamp_ms);
    if day == "Today" {
        return format!("Today at {hour}");
    }
    format!("{day} at {hour}")
}

/// The hour of a status in the 24 hour clock.
///
/// The zone of the machine is not available in this crate (there is no
/// library of dates), so the hour is the one of UTC, the same way the day
/// of [`status_time`] is.
fn status_hour(timestamp_ms: i64) -> String {
    let minutes = (timestamp_ms / 60_000).rem_euclid(24 * 60);
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// The strip of the statuses of the contacts.
pub(crate) struct StatusStrip {
    /// The state of the strip.
    state: Rc<RefCell<StatusStripState>>,
    /// The text of the status that is being written.
    text_input: Entity<InputState>,
    /// The subscription of the input of the text.
    _subscription: Subscription,
}

impl StatusStrip {
    /// Creates the strip with the input of the text status.
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Whatfast>) -> Self {
        let state = Rc::new(RefCell::new(StatusStripState::new()));
        let text_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Write a text statusÔÇª")
                .clean_on_escape()
        });
        let colors = state.clone();
        let subscription = cx.subscribe_in(&text_input, window, move |app, input, event, _, cx| {
            if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                let text = input.read(cx).value().to_string();
                publish_text_status(app, &colors, &text, cx);
            }
            cx.notify();
        });
        Self {
            state,
            text_input,
            _subscription: subscription,
        }
    }

    /// Whether the viewer of a status is open.
    pub(crate) fn viewer_open(&self) -> bool {
        self.state.borrow().viewer.is_some()
    }

    /// The status that is open in the viewer.
    pub(crate) fn viewer(&self) -> Option<StatusViewer> {
        self.state.borrow().viewer
    }

    /// Asks the user for a photo and publishes it as a status.
    pub(crate) fn publish_photo_status(&self, window: &mut Window, cx: &mut Context<Whatfast>) {
        choose_photo_status(&self.state, window, cx);
    }

    /// Opens or closes the composer of a text status.
    pub(crate) fn toggle_text_composer(&self) {
        let mut state = self.state.borrow_mut();
        state.text_composer_open = !state.text_composer_open;
    }

    /// Draws the panel of the states that sits next to the list.
    ///
    /// The reference draws the circle of dashes of the states at the top,
    /// the title big below it, the explanation of what a status is and two
    /// round buttons: one for the text and one for the photo.
    pub(crate) fn render_panel(
        &self,
        app: &Whatfast,
        cx: &mut Context<Whatfast>,
    ) -> gpui::Stateful<gpui::Div> {
        let scale = app.ui_scale;
        let muted = cx.theme().muted_foreground;
        let text_open = self.state.borrow().text_composer_open;
        let mut panel = v_flex()
            .flex_1()
            .min_h_0()
            .items_center()
            .justify_center()
            .gap_4()
            .p_4()
            // El dibujo grande del panel: el circulo de los estados.
            .child(self.render_status_glyph(scale, cx))
            .child(
                // El titulo va grande y centrado: es el texto mas grande
                // del panel.
                div()
                    .mt_2()
                    .text_size(px(PANEL_TITLE_SIZE * scale))
                    .font_weight(FontWeight::NORMAL)
                    .text_color(cx.theme().foreground)
                    .child("Share status"),
            )
            .child(
                div()
                    .text_size(px(15. * scale))
                    .text_color(muted)
                    .text_center()
                    .child("Share photos, videos and text that disappear after 24 hours."),
            )
            .child(
                // Los dos botones son redondos y llevan el dibujo adentro;
                // el texto de cada uno va debajo, centrado.
                h_flex()
                    .mt_2()
                    .gap(px(PANEL_BUTTON_GAP * scale))
                    .items_start()
                    .child(self.render_panel_button(
                        "status-text",
                        "Text status",
                        PanelGlyph::Text,
                        scale,
                        |this, _, cx| {
                            this.status_strip.toggle_text_composer();
                            cx.notify();
                        },
                        cx,
                    ))
                    .child(self.render_panel_button(
                        "status-photo",
                        "Photo and video",
                        PanelGlyph::Photo,
                        scale,
                        |this, window, cx| {
                            this.status_strip.publish_photo_status(window, cx);
                        },
                        cx,
                    )),
            );
        if text_open {
            panel = panel.child(self.render_form(app, cx));
        }
        panel.id("status-panel")
    }

    /// The big drawing of the panel: the circle of dashes of the states.
    ///
    /// The reference draws three arcs of about a hundred degrees, with the
    /// opening of the first one at the top, and the solid circle of the
    /// middle. A circle of the layout cannot draw the arcs, so the drawing
    /// goes to the canvas of the element.
    fn render_status_glyph(&self, scale: f32, cx: &mut Context<Whatfast>) -> gpui::Div {
        // El color del dibujo: el del tema, apagado, porque en la referencia
        // el circulo es gris y no blanco. Va mezclado con el fondo y opaco:
        // las puntas de los arcos se pintan encima de la linea, y una capa
        // semitransparente dejaria una costura en cada punta.
        let color = cx
            .theme()
            .background
            .blend(cx.theme().foreground.opacity(0.2));
        let size = PANEL_GLYPH_SIZE * scale;
        let stroke = PANEL_GLYPH_STROKE * scale;
        let dot = PANEL_GLYPH_DOT_SIZE * scale;
        div().flex_none().size(px(size)).child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    paint_status_glyph(bounds, size, stroke, dot, color, window);
                },
            )
            .size_full(),
        )
    }

    /// One of the round buttons of the panel: the drawing inside the button
    /// and the text below.
    fn render_panel_button(
        &self,
        id: &'static str,
        label: &'static str,
        glyph: PanelGlyph,
        scale: f32,
        on_click: impl Fn(&mut Whatfast, &mut Window, &mut Context<Whatfast>) + 'static,
        cx: &mut Context<Whatfast>,
    ) -> gpui::Stateful<gpui::Div> {
        let foreground = cx.theme().foreground;
        let muted = cx.theme().muted_foreground;
        let hover = cx.theme().muted;
        let background = cx.theme().secondary;
        v_flex()
            .id(ElementId::named_usize(id, 0))
            .flex_none()
            .w(px(PANEL_BUTTON_WIDTH * scale))
            .items_center()
            .gap(px(PANEL_BUTTON_LABEL_GAP * scale))
            .child(
                div()
                    .id(id)
                    .flex_none()
                    .w(px(PANEL_BUTTON_WIDTH * scale))
                    .h(px(PANEL_BUTTON_HEIGHT * scale))
                    .rounded(px(PANEL_BUTTON_RADIUS * scale))
                    .bg(background)
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(foreground)
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        on_click(this, window, cx);
                    }))
                    .child(render_panel_glyph(glyph, foreground, background, scale)),
            )
            .child(
                div()
                    .text_size(px(13. * scale))
                    .text_color(muted)
                    .whitespace_nowrap()
                    .child(label),
            )
    }

    /// Draws the view of the states.
    ///
    /// The states are not conversations: they have their own view, the same
    /// way the reference draws them, with the state of the user on top and the
    /// list of the contacts below.
    pub(crate) fn render_view(
        &self,
        app: &Whatfast,
        cx: &mut Context<Whatfast>,
    ) -> gpui::Stateful<gpui::Div> {
        let scale = app.ui_scale;
        self.ensure_loaded(app, cx);
        let groups = app.store.statuses();
        let muted = cx.theme().muted_foreground;

        let mut list = v_flex()
            .id("status-view")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .id("status-my-status")
                    .w_full()
                    .gap(px(4.))
                    .px(px(16. * scale))
                    .py(px(12. * scale))
                    .child(
                        h_flex()
                            .gap(px(12. * scale))
                            .items_center()
                            .child(self.render_own_ring(scale, cx))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap(px(2.))
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child("My status"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(muted)
                                            .child("Click to add a status update"),
                                    ),
                            ),
                    ),
            );

        if !groups.is_empty() {
            list = list.child(
                div()
                    .px(px(19. * scale))
                    .pt(px(45. * scale))
                    .pb(px(22. * scale))
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(muted)
                    .child("RECENT"),
            );
            for (index, group) in groups.iter().enumerate() {
                list = list.child(self.render_status_row(group, index, scale, cx));
            }
        }

        list
    }

    /// The row of one contact in the view of the states.
    fn render_status_row(
        &self,
        group: &StatusGroup,
        index: usize,
        scale: f32,
        cx: &mut Context<Whatfast>,
    ) -> gpui::Stateful<gpui::Div> {
        let muted = cx.theme().muted_foreground;
        let state = self.state.clone();
        let label = sender_label(group);
        let sender = group.sender_id.clone();
        let time = group
            .messages
            .first()
            .map(|message| status_time(message.timestamp_ms, now_ms()))
            .unwrap_or_default();
        let hover = cx.theme().muted;
        div()
            .id(ElementId::named_usize("status-row", index))
            .w_full()
            .gap(px(12. * scale))
            .px(px(19. * scale))
            .py(px(STATUS_ROW_PADDING * scale))
            .flex()
            .items_center()
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
            .child(self.render_ring(group, index, scale, cx))
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
                            .child(label),
                    )
                    .child(div().text_xs().text_color(muted).child(time)),
            )
            .on_click(cx.listener(move |_app, _, _, cx| {
                open_viewer(&state, index, &sender, cx);
            }))
    }

    /// The circle of the row of "My status".
    ///
    /// The reference puts the photo of the user in a circle with the green
    /// badge of the "+" in its corner. The app has no photo of the user, so
    /// the circle carries the glyph of the person, the same way the panel of
    /// the settings does.
    fn render_own_ring(&self, scale: f32, cx: &mut Context<Whatfast>) -> gpui::Stateful<gpui::Div> {
        let accent = cx.theme().accent;
        let accent_foreground = cx.theme().accent_foreground;
        let secondary = cx.theme().secondary;
        let muted = cx.theme().muted_foreground;
        let state = self.state.clone();
        div()
            .id("status-own-ring")
            .flex_shrink_0()
            .relative()
            .size(px(RING_SIZE * scale))
            .rounded_full()
            .bg(secondary)
            .flex()
            .items_center()
            .justify_center()
            .child(
                Icon::new(IconName::User)
                    .with_size(px(RING_SIZE * 0.48 * scale))
                    .text_color(muted),
            )
            .child(
                // El circulito verde del "+": esta en la esquina de la foto.
                div()
                    .id("status-own-add")
                    .absolute()
                    .right(px(-2. * scale))
                    .bottom(px(3.5 * scale))
                    .size(px(OWN_BADGE_SIZE * scale))
                    .rounded_full()
                    .bg(accent)
                    .flex()
                    .items_center()
                    .justify_center()
                    .on_click(cx.listener(move |_app, _, window, cx| {
                        choose_photo_status(&state, window, cx);
                    }))
                    .child(
                        Icon::new(IconName::Plus)
                            .with_size(px(11. * scale))
                            .text_color(accent_foreground),
                    ),
            )
    }

    /// The ring of the status of one contact.
    ///
    /// The reference draws the circle of a status that was already seen in
    /// green and the circle of a new status in green dashes. The dashes are
    /// shorter than the ones of a border of the layout, so the circle goes to
    /// the canvas.
    fn render_ring(
        &self,
        group: &StatusGroup,
        index: usize,
        scale: f32,
        cx: &mut Context<Whatfast>,
    ) -> gpui::Stateful<gpui::Div> {
        let accent = cx.theme().accent;
        let secondary = cx.theme().secondary;
        let label = sender_label(group);
        let state = self.state.clone();
        let sender = group.sender_id.clone();
        let seen = has_seen(&self.state.borrow(), &sender);
        let size = RING_SIZE * scale;
        let thickness = RING_BORDER * scale;
        div()
            .id(ElementId::named_usize("status-ring", index))
            .flex_shrink_0()
            .relative()
            .size(px(size))
            .rounded_full()
            .bg(secondary)
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(16. * scale))
            .font_weight(FontWeight::BOLD)
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        paint_status_ring(bounds, size, thickness, seen, accent, scale, window);
                    },
                )
                .absolute()
                .inset_0(),
            )
            .child(avatar_label(&label))
            .on_click(cx.listener(move |_app, _, _, cx| {
                open_viewer(&state, index, &sender, cx);
            }))
    }

    /// The form that publishes a text status.
    fn render_form(&self, app: &Whatfast, cx: &mut Context<Whatfast>) -> gpui::Div {
        let scale = app.ui_scale;
        let text = self.text_input.read(cx).value().to_string();
        let color = self.state.borrow().text_color.clone();
        let state = self.state.clone();
        let input = self.text_input.clone();
        h_flex()
            .flex_none()
            .gap_2()
            .px(px(12. * scale))
            .py(px(8. * scale))
            .border_b_1()
            .border_color(cx.theme().border)
            .child(Input::new(&self.text_input).flex_1())
            .child(self.render_colors(&color, cx))
            .child(
                Button::new("status-post")
                    .secondary()
                    .small()
                    .label("Post")
                    .disabled(text.trim().is_empty())
                    .on_click(cx.listener(move |app, _, _, cx| {
                        let text = input.read(cx).value().to_string();
                        publish_text_status(app, &state, &text, cx);
                    })),
            )
    }

    /// The row of the colors of a text status.
    fn render_colors(&self, color: &str, cx: &mut Context<Whatfast>) -> gpui::Div {
        let foreground = cx.theme().foreground;
        h_flex()
            .gap(px(4.))
            .children(STATUS_COLORS.iter().enumerate().map(|(index, candidate)| {
                let selected = *candidate == color;
                let value = (*candidate).to_string();
                let state = self.state.clone();
                div()
                    .id(ElementId::named_usize("status-color", index))
                    .size(px(18.))
                    .rounded_full()
                    .border_2()
                    .border_color(if selected {
                        foreground
                    } else {
                        gpui::transparent_black()
                    })
                    .bg(parse_color(candidate).unwrap_or_else(|| rgba(0x0f766e)))
                    .on_click(cx.listener(move |_app, _, _, cx| {
                        state.borrow_mut().text_color = value.clone();
                        cx.notify();
                    }))
            }))
    }

    /// Draws the viewer of one status.
    ///
    /// The viewer covers the whole window, so it is drawn apart from the
    /// strip: the layout puts it on top of everything.
    pub(crate) fn render_viewer(
        &self,
        app: &Whatfast,
        cx: &mut Context<Whatfast>,
    ) -> Option<gpui::Div> {
        let viewer = self.viewer()?;
        let scale = app.ui_scale;
        let groups = app.store.statuses();
        let group = groups.get(viewer.group_index)?;
        let message = group.messages.get(viewer.message_index);

        let background = message
            .and_then(|message| self.state.borrow().colors.get(&message.id).cloned())
            .and_then(|color| parse_color(&color))
            .unwrap_or_else(|| cx.theme().background.into());

        let progress =
            h_flex()
                .gap(px(4. * scale))
                .children(group.messages.iter().enumerate().map(|(index, _)| {
                    let color = if index < viewer.message_index {
                        cx.theme().muted_foreground
                    } else if index == viewer.message_index {
                        cx.theme().primary
                    } else {
                        cx.theme().border
                    };
                    div()
                        .flex_1()
                        .h(px(PROGRESS_HEIGHT * scale))
                        .rounded_sm()
                        .bg(color)
                }));

        let sender = sender_label(group);
        let meta = h_flex()
            .gap(px(10. * scale))
            .child(
                div()
                    .size(px(VIEWER_AVATAR_SIZE * scale))
                    .rounded_full()
                    .border(px(RING_BORDER * scale))
                    .border_color(cx.theme().primary)
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(13. * scale))
                    .font_weight(FontWeight::BOLD)
                    .child(avatar_label(&sender)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(if sender.is_empty() {
                                "Status".to_string()
                            } else {
                                sender
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                message
                                    .map(|message| message_time(message.timestamp_ms))
                                    .unwrap_or_default(),
                            ),
                    ),
            )
            .child(self.render_close(cx));

        let content = div()
            .id("status-viewer-content")
            .flex_1()
            .min_h_0()
            .flex()
            .items_center()
            .justify_center()
            .p(px(16. * scale))
            .overflow_y_scroll()
            .child(match message {
                Some(message) => render_message(message, scale, cx),
                None => div(),
            });

        let mut viewer_root = v_flex()
            .absolute()
            .inset_0()
            .bg(background)
            .child(
                v_flex()
                    .flex_none()
                    .gap(px(10. * scale))
                    .px(px(16. * scale))
                    .py(px(12. * scale))
                    .child(progress)
                    .child(meta),
            )
            .child(content);

        if groups.len() > 1 {
            viewer_root = viewer_root.child(self.render_navs(cx));
        }
        Some(viewer_root)
    }

    /// The button that closes the viewer.
    fn render_close(&self, cx: &mut Context<Whatfast>) -> gpui::Stateful<gpui::Div> {
        let state = self.state.clone();
        let foreground = cx.theme().foreground;
        let hover = cx.theme().muted;
        div()
            .id("status-viewer-close")
            .size(px(28.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .hover(move |style| style.bg(hover))
            .child(
                Icon::new(IconName::Close)
                    .with_size(px(20.))
                    .text_color(foreground),
            )
            .on_click(cx.listener(move |_app, _, _, cx| {
                state.borrow_mut().viewer = None;
                cx.notify();
            }))
    }

    /// The buttons that move between the statuses.
    ///
    /// port: the buttons of the origin are fixed on the sides of the viewer,
    /// in the middle of its height.
    fn render_navs(&self, cx: &mut Context<Whatfast>) -> gpui::Stateful<gpui::Div> {
        h_flex()
            .id("status-nav")
            .absolute()
            .inset_0()
            .px(px(12.))
            .items_center()
            .justify_between()
            .child(self.render_nav(false, cx))
            .child(self.render_nav(true, cx))
    }

    /// One of the buttons that move between the statuses.
    fn render_nav(&self, forward: bool, cx: &mut Context<Whatfast>) -> gpui::Stateful<gpui::Div> {
        let state = self.state.clone();
        let foreground = cx.theme().foreground;
        let hover = cx.theme().muted;
        div()
            .id(if forward {
                "status-nav-next"
            } else {
                "status-nav-prev"
            })
            .flex_none()
            .size(px(NAV_SIZE))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .hover(move |style| style.bg(hover))
            .child(
                Icon::new(if forward {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronLeft
                })
                .with_size(px(24.))
                .text_color(foreground),
            )
            .on_click(cx.listener(move |app, _, _, cx| {
                let groups = app.store.statuses().to_vec();
                move_viewer(&state, forward, &groups, cx);
            }))
    }

    /// Asks for the statuses when they are not there yet.
    fn ensure_loaded(&self, app: &Whatfast, cx: &mut Context<Whatfast>) {
        if !app.store.statuses().is_empty() {
            return;
        }
        {
            let mut state = self.state.borrow_mut();
            if state.requested {
                return;
            }
            state.requested = true;
        }
        cx.spawn(async move |weak, cx| {
            weak.update(cx, |app, cx| {
                app.load_statuses();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

/// The drawing inside one of the round buttons of the panel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PanelGlyph {
    /// The pencil of the text status.
    Text,
    /// The camera of the photo status.
    Photo,
}

/// Draws the drawing of one of the round buttons of the panel.
///
/// The text status carries the pencil of the set of the icons. The photo
/// status carries a camera: the set of the icons has no camera (the `Video`
/// glyph is a camcorder), so the camera is drawn here.
fn render_panel_glyph(glyph: PanelGlyph, color: Hsla, background: Hsla, scale: f32) -> gpui::Div {
    let size = PANEL_BUTTON_ICON_SIZE * scale;
    match glyph {
        PanelGlyph::Text => div()
            .flex_none()
            .size(px(size))
            .flex()
            .items_center()
            .justify_center()
            .child(
                Icon::new(WhatfastIcon::Edit)
                    .with_size(px(size))
                    .text_color(color),
            ),
        PanelGlyph::Photo => render_camera(color, background, scale),
    }
}

/// Draws the camera of the button of the photo status.
///
/// The reference draws a camera of line: the body, the hump of the top and
/// the ring of the lens. The body is drawn after the hump, with the color of
/// the button, so it hides the bottom edge of the hump.
fn render_camera(color: Hsla, background: Hsla, scale: f32) -> gpui::Div {
    let width = 21. * scale;
    let height = 18. * scale;
    let body = 15. * scale;
    let top = height - body;
    let hump_width = 8. * scale;
    let hump_height = 4. * scale;
    let lens = 7.5 * scale;
    let stroke = px(1.4 * scale);
    div()
        .flex_none()
        .relative()
        .w(px(width))
        .h(px(height))
        .child(
            div()
                .absolute()
                .left(px((width - hump_width) * 0.5))
                .top(px(0.))
                .w(px(hump_width))
                .h(px(hump_height + top))
                .border(stroke)
                .border_color(color)
                .rounded_t(px(2. * scale)),
        )
        .child(
            div()
                .absolute()
                .left(px(0.))
                .top(px(top))
                .w(px(width))
                .h(px(body))
                .rounded(px(3. * scale))
                .border(stroke)
                .border_color(color)
                .bg(background)
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .flex_none()
                        .size(px(lens))
                        .rounded_full()
                        .border(stroke)
                        .border_color(color),
                ),
        )
}

/// Paints the big drawing of the panel: the three arcs of the circle of the
/// states and the solid circle of the middle.
fn paint_status_glyph(
    bounds: Bounds<gpui::Pixels>,
    size: f32,
    stroke: f32,
    dot: f32,
    color: Hsla,
    window: &mut Window,
) {
    let center = bounds.center();
    let radius = (size - stroke) * 0.5;
    let mut start = PANEL_GLYPH_FIRST_ARC_DEGREES;
    for _ in 0..3 {
        let from = point_on_circle(&center, radius, start);
        let to = point_on_circle(&center, radius, start + PANEL_GLYPH_ARC_DEGREES);
        let mut builder = PathBuilder::stroke(px(stroke));
        builder.move_to(from);
        builder.arc_to(point(px(radius), px(radius)), px(0.), false, true, to);
        if let Ok(path) = builder.build() {
            window.paint_path(path, color);
        }
        // La punta redonda de cada arco: la referencia cierra los arcos con
        // una punta redonda, asi que cada extremo lleva el circulito de la
        // linea.
        paint_circle(window, from, stroke, color);
        paint_circle(window, to, stroke, color);
        start += PANEL_GLYPH_ARC_DEGREES + PANEL_GLYPH_GAP_DEGREES;
    }
    paint_circle(window, center, dot, color);
}

/// One point of a circle, in degrees from the top and turning to the right.
fn point_on_circle(
    center: &gpui::Point<gpui::Pixels>,
    radius: f32,
    degrees: f32,
) -> gpui::Point<gpui::Pixels> {
    let radians = degrees.to_radians();
    point(
        center.x + px(radius * radians.sin()),
        center.y - px(radius * radians.cos()),
    )
}

/// Paints a filled circle of the given side, centered on one point.
fn paint_circle(
    window: &mut Window,
    center: gpui::Point<gpui::Pixels>,
    diameter: f32,
    color: Hsla,
) {
    let half = diameter * 0.5;
    window.paint_quad(quad(
        Bounds::new(
            point(center.x - px(half), center.y - px(half)),
            gpui::size(px(diameter), px(diameter)),
        ),
        px(half),
        color,
        px(0.),
        color,
        gpui::BorderStyle::default(),
    ));
}

/// Paints the ring of the status of one contact.
///
/// A status that was seen carries a closed circle; a status that is new
/// carries the same circle cut in short dashes.
fn paint_status_ring(
    bounds: Bounds<gpui::Pixels>,
    size: f32,
    thickness: f32,
    seen: bool,
    color: Hsla,
    scale: f32,
    window: &mut Window,
) {
    let center = bounds.center();
    let radius = (size - thickness) * 0.5;
    let mut builder = PathBuilder::stroke(px(thickness));
    if !seen {
        builder = builder.dash_array(&[px(RING_DASH * scale), px(RING_DASH_GAP * scale)]);
    }
    // El circulo completo: dos semicirculos, para que el trazo no tenga una
    // costura en el punto de arranque.
    builder.move_to(point_on_circle(&center, radius, 0.));
    builder.arc_to(
        point(px(radius), px(radius)),
        px(0.),
        false,
        true,
        point_on_circle(&center, radius, 180.),
    );
    builder.arc_to(
        point(px(radius), px(radius)),
        px(0.),
        false,
        true,
        point_on_circle(&center, radius, 360.),
    );
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

/// Publishes a text status in the memory of the client.
///
/// port: `publishTextStatus` of the origin.
fn publish_text_status(
    app: &mut Whatfast,
    state: &Rc<RefCell<StatusStripState>>,
    text: &str,
    cx: &mut Context<Whatfast>,
) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }
    let chat_id = statuses_chat_id(app);
    let message = text_status_message(&chat_id, trimmed, now_ms());
    let color = state.borrow().text_color.clone();
    state.borrow_mut().colors.insert(message.id.clone(), color);
    let mut groups = app.store.statuses().to_vec();
    add_status_message(&mut groups, message);
    app.store.set_statuses(chat_id, groups);
    cx.notify();
}

/// Asks the user for a photo and publishes it as a status.
///
/// port: `publishPhotoStatus` of the origin. The photo stays in the memory
/// of the client: the linked device has no RPC for a status of the user.
fn choose_photo_status(
    state: &Rc<RefCell<StatusStripState>>,
    window: &mut Window,
    cx: &mut Context<Whatfast>,
) {
    let selected = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: Some("Choose a photo for your status".into()),
    });
    let view = cx.entity();
    let state = state.clone();
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
        let path = path.to_string_lossy().into_owned();
        window
            .update(|_, cx| {
                view.update(cx, |app, cx| {
                    publish_photo_status(app, &state, &path, cx);
                });
            })
            .ok();
    })
    .detach();
}

/// Publishes a photo status in the memory of the client.
fn publish_photo_status(
    app: &mut Whatfast,
    state: &Rc<RefCell<StatusStripState>>,
    path: &str,
    cx: &mut Context<Whatfast>,
) {
    let chat_id = statuses_chat_id(app);
    let message = photo_status_message(&chat_id, path, now_ms());
    let color = state.borrow().text_color.clone();
    state.borrow_mut().colors.insert(message.id.clone(), color);
    let mut groups = app.store.statuses().to_vec();
    add_status_message(&mut groups, message);
    app.store.set_statuses(chat_id, groups);
    cx.notify();
}

/// The conversation that carries the statuses.
fn statuses_chat_id(app: &Whatfast) -> String {
    let chat_id = app.store.statuses_chat_id();
    if chat_id.is_empty() {
        "status@broadcast".to_string()
    } else {
        chat_id.to_string()
    }
}

/// Opens the viewer of the status of one group.
///
/// The group passes to be a seen status: its ring becomes solid.
fn open_viewer(
    state: &Rc<RefCell<StatusStripState>>,
    group_index: usize,
    sender_id: &str,
    cx: &mut Context<Whatfast>,
) {
    let generation = {
        let mut inner = state.borrow_mut();
        inner.generation += 1;
        inner.viewer = Some(StatusViewer {
            group_index,
            message_index: 0,
        });
        mark_seen(&mut inner, sender_id);
        inner.generation
    };
    cx.notify();
    spawn_advance(state, generation, cx);
}

/// Moves the viewer to the next or to the previous status.
fn move_viewer(
    state: &Rc<RefCell<StatusStripState>>,
    forward: bool,
    groups: &[StatusGroup],
    cx: &mut Context<Whatfast>,
) {
    let mut inner = state.borrow_mut();
    let Some(mut viewer) = inner.viewer else {
        return;
    };
    if forward {
        if advance_viewer(&mut viewer, groups) {
            inner.viewer = Some(viewer);
        } else {
            inner.viewer = None;
        }
    } else if previous_viewer(&mut viewer, groups) {
        inner.viewer = Some(viewer);
    }
    cx.notify();
}

/// The task that moves the viewer every three seconds.
///
/// port: the origin sets an interval while the viewer is open, and it
/// clears it when the viewer closes.
fn spawn_advance(
    state: &Rc<RefCell<StatusStripState>>,
    generation: u64,
    cx: &mut Context<Whatfast>,
) {
    let state = state.clone();
    cx.spawn(async move |weak, cx| {
        loop {
            Timer::after(Duration::from_millis(STATUS_STEP_MS)).await;
            let keep = weak.update(cx, |app, cx| {
                let mut inner = state.borrow_mut();
                if inner.generation != generation {
                    return false;
                }
                let Some(mut viewer) = inner.viewer else {
                    return false;
                };
                let groups = app.store.statuses();
                if !advance_viewer(&mut viewer, groups) {
                    inner.viewer = None;
                    cx.notify();
                    return false;
                }
                inner.viewer = Some(viewer);
                cx.notify();
                true
            });
            match keep {
                Ok(true) => {}
                _ => break,
            }
        }
    })
    .detach();
}

/// Draws the content of one message of a status.
fn render_message(message: &Message, scale: f32, cx: &mut Context<Whatfast>) -> gpui::Div {
    if let Some(path) = message_image_path(message) {
        return div().max_w(px(320. * scale)).child(
            img(PathBuf::from(path))
                .max_w(px(320. * scale))
                .max_h(px(320. * scale))
                .object_fit(ObjectFit::Contain),
        );
    }
    v_flex().gap_2().child(
        div()
            .text_size(px(16. * scale))
            .text_color(cx.theme().foreground)
            .child(message_text(message).to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(sender: &str, messages: usize) -> StatusGroup {
        StatusGroup {
            sender_id: sender.to_string(),
            sender_name: sender.to_string(),
            sender_phone_number: String::new(),
            messages: (0..messages)
                .map(|index| Message {
                    id: format!("{sender}-{index}"),
                    ..Default::default()
                })
                .collect(),
        }
    }

    #[test]
    fn the_letter_of_a_ring_is_the_first_letter_of_the_name() {
        assert_eq!(avatar_label("Ana"), "A");
        assert_eq!(avatar_label("  bravo"), "B");
        assert_eq!(avatar_label("ana maria"), "A");
    }

    #[test]
    fn a_ring_without_a_name_shows_a_question_mark() {
        assert_eq!(avatar_label(""), "?");
        assert_eq!(avatar_label("   "), "?");
        assert_eq!(avatar_label("123"), "1");
    }

    #[test]
    fn the_name_of_the_sender_falls_back_to_the_phone_number() {
        let mut sender = group("Ana", 1);
        assert_eq!(sender_label(&sender), "Ana");
        sender.sender_name = String::new();
        sender.sender_phone_number = "5493811234567".to_string();
        assert_eq!(sender_label(&sender), "5493811234567");
    }

    #[test]
    fn the_viewer_moves_to_the_next_message_of_the_group() {
        let groups = vec![group("ana", 2)];
        let mut viewer = StatusViewer {
            group_index: 0,
            message_index: 0,
        };
        assert!(advance_viewer(&mut viewer, &groups));
        assert_eq!(viewer.message_index, 1);
    }

    #[test]
    fn the_viewer_moves_to_the_next_group() {
        let groups = vec![group("ana", 1), group("bravo", 2)];
        let mut viewer = StatusViewer {
            group_index: 0,
            message_index: 0,
        };
        assert!(advance_viewer(&mut viewer, &groups));
        assert_eq!(viewer.group_index, 1);
        assert_eq!(viewer.message_index, 0);
    }

    #[test]
    fn the_viewer_closes_when_there_is_no_next_status() {
        let groups = vec![group("ana", 1)];
        let mut viewer = StatusViewer {
            group_index: 0,
            message_index: 0,
        };
        assert!(!advance_viewer(&mut viewer, &groups));
    }

    #[test]
    fn the_viewer_does_not_move_when_the_group_is_gone() {
        let mut viewer = StatusViewer {
            group_index: 3,
            message_index: 0,
        };
        assert!(!advance_viewer(&mut viewer, &[]));
    }

    #[test]
    fn the_viewer_goes_back_to_the_previous_message() {
        let groups = vec![group("ana", 3)];
        let mut viewer = StatusViewer {
            group_index: 0,
            message_index: 2,
        };
        assert!(previous_viewer(&mut viewer, &groups));
        assert_eq!(viewer.message_index, 1);
    }

    #[test]
    fn the_viewer_goes_back_to_the_last_message_of_the_previous_group() {
        let groups = vec![group("ana", 2), group("bravo", 3)];
        let mut viewer = StatusViewer {
            group_index: 1,
            message_index: 0,
        };
        assert!(previous_viewer(&mut viewer, &groups));
        assert_eq!(viewer.group_index, 0);
        assert_eq!(viewer.message_index, 1);
    }

    #[test]
    fn the_viewer_does_not_go_back_before_the_first_status() {
        let groups = vec![group("ana", 2)];
        let mut viewer = StatusViewer {
            group_index: 0,
            message_index: 0,
        };
        assert!(!previous_viewer(&mut viewer, &groups));
    }

    #[test]
    fn a_text_status_is_capped_at_seven_hundred_characters() {
        let text = "a".repeat(MAX_STATUS_LENGTH + 10);
        let message = text_status_message("status@broadcast", &text, 42);
        let Some(Content::Text(content)) = &message.content else {
            panic!("the status is a text");
        };
        assert_eq!(content.text.chars().count(), MAX_STATUS_LENGTH);
        assert_eq!(message.id, "local-status-42");
        assert_eq!(message.chat_id, "status@broadcast");
        assert!(message.from_me);
    }

    #[test]
    fn a_text_status_goes_to_the_group_of_its_sender() {
        let mut groups = vec![group("me@s.whatsapp.net", 1)];
        let message = text_status_message("status@broadcast", "hello", 42);
        add_status_message(&mut groups, message);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].messages.len(), 2);
    }

    #[test]
    fn a_text_status_of_a_new_sender_opens_a_group() {
        let mut groups = vec![group("ana", 1)];
        let message = text_status_message("status@broadcast", "hello", 42);
        add_status_message(&mut groups, message);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].sender_id, "me@s.whatsapp.net");
        assert_eq!(groups[0].messages.len(), 1);
    }

    #[test]
    fn a_photo_status_carries_the_path_of_the_picture() {
        let message = photo_status_message("status@broadcast", "/tmp/photo.jpg", 7);
        assert_eq!(
            message_image_path(&message),
            Some("/tmp/photo.jpg".to_string())
        );
    }

    #[test]
    fn a_text_status_has_no_picture() {
        let message = text_status_message("status@broadcast", "hello", 7);
        assert_eq!(message_image_path(&message), None);
    }

    #[test]
    fn a_status_is_new_until_the_user_opens_it() {
        let state = StatusStripState::new();
        assert!(!has_seen(&state, "maya@s.whatsapp.net"));
    }

    #[test]
    fn opening_a_status_marks_it_as_seen_and_leaves_the_others_new() {
        let mut state = StatusStripState::new();
        mark_seen(&mut state, "maya@s.whatsapp.net");
        assert!(has_seen(&state, "maya@s.whatsapp.net"));
        assert!(!has_seen(&state, "priya@s.whatsapp.net"));
    }

    #[test]
    fn the_first_degree_of_the_glyph_starts_at_the_top_of_the_circle() {
        let center = point(px(10.), px(10.));
        assert_eq!(point_on_circle(&center, 4., 0.), point(px(10.), px(6.)));
        assert_eq!(point_on_circle(&center, 4., 90.), point(px(14.), px(10.)));
        assert_eq!(point_on_circle(&center, 4., 180.), point(px(10.), px(14.)));
    }

    #[test]
    fn the_hour_of_a_status_goes_in_the_twenty_four_hour_clock() {
        // The reference writes "Today at 12:45", not "Today at 12:45 PM".
        assert_eq!(status_hour(0), "00:00");
        assert_eq!(status_hour(45 * 60_000), "00:45");
        assert_eq!(status_hour(12 * 3_600_000 + 45 * 60_000), "12:45");
        assert_eq!(status_hour(23 * 3_600_000 + 5 * 60_000), "23:05");
    }

    #[test]
    fn the_time_of_a_status_writes_the_day_and_the_hour() {
        let noon = 12 * 3_600_000;
        assert_eq!(
            status_time(noon, noon),
            format!("Today at {}", status_hour(noon))
        );
        let yesterday = noon - 24 * 3_600_000;
        assert_eq!(
            status_time(yesterday, noon),
            format!("Yesterday at {}", status_hour(yesterday))
        );
    }
}
