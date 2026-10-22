//! The panel of the newsletters: the channels the account is subscribed to and
//! the posts of the open one.
//!
//! port: taken from the origin's `tauri/src/components/NewsletterPanel.tsx`
//! and from the list of channels of its sidebar (`tauri/src/components/
//! Sidebar.tsx`, the block `newsletter-list`). The lengths come from the
//! stylesheet of the origin (`tauri/src/styles.css`, the classes
//! `newsletter-panel`, `newsletter-posts`, `newsletter-post`,
//! `newsletter-subscribe` and `newsletter-footer`).
//!
//! The data is not here: the channels live in the store of the application
//! (`store.newsletters()` and `store.newsletter_messages()`), and the panel
//! only draws them. The actions it needs already exist in `app.rs`:
//! `load_newsletters()`, `subscribe_newsletter()`, `unsubscribe_newsletter()`
//! and `select_newsletter()`.

#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AppContext as _, Context, ElementId, Entity, FontWeight, InteractiveElement as _, ObjectFit,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, StyledImage as _,
    Subscription, Window, div, img, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};

use whatfast_core::message_format::{DateLocale, LocalTimestamp, format_time};
use whatfast_model::model::message_text;
use whatfast_protocol::{Message, Newsletter, message::Content};

use crate::app::Whatfast;
use crate::helpers::{avatar_element, one_line};

/// The side of the avatar of the header, in pixels, before the scale.
pub(crate) const HEADER_AVATAR_SIZE: f32 = 41.0;

/// The channels that the client suggests to follow.
///
/// port: the reference draws, under the list of the channels that the account
/// follows, a section with the channels that can be followed. The protocol
/// has no RPC that asks for the suggestions, so the client draws this fixed
/// list: the name of the channel and the number of followers.
const SUGGESTED_CHANNELS: [(&str, &str); 4] = [
    ("Whatfast Sports", "42K followers"),
    ("Whatfast Tech", "2M followers"),
    ("Whatfast Music", "234M followers"),
    ("Whatfast Cinema", "1M followers"),
];

/// The state of the panel of the newsletters.
#[derive(Default)]
pub(crate) struct NewsletterPanelState {
    /// Whether the dialog to follow a channel is open.
    pub(crate) subscribe_open: bool,
    /// Whether the dialog to create a local channel is open.
    pub(crate) create_open: bool,
    /// The channels that were created locally, with their description.
    ///
    /// port: the origin keeps the local channels in the state of the client,
    /// because a linked device has no RPC for a channel.
    pub(crate) local: HashMap<String, String>,
    /// Whether the channels were already asked for.
    pub(crate) requested: bool,
}

/// The letter that stands for a channel in its avatar.
///
/// port: `avatarLabel` of the origin: the first letter of the name, in upper
/// case, or a "C" when the name has no letters at all.
pub(crate) fn avatar_label(value: &str) -> String {
    let trimmed = value.trim();
    match trimmed.chars().next() {
        Some(letter) => letter.to_uppercase().collect(),
        None => "C".to_string(),
    }
}

/// The channel that is open in the panel.
///
/// port: the origin keeps the whole channel in its state; the application
/// keeps only its jid, so the channel is looked up in the store.
pub(crate) fn selected_newsletter<'a>(
    newsletters: &'a [Newsletter],
    selected: Option<&str>,
) -> Option<&'a Newsletter> {
    let selected = selected?;
    newsletters
        .iter()
        .find(|newsletter| newsletter.jid == selected)
}

/// The label of the channel under its name.
///
/// port: the origin writes "Channel", with " ┬À muted" when the channel is
/// muted.
/// The line under the name of a channel.
///
/// port: the `newsletter-row` of the origin: the name, the hour of the
/// subscription at the right, and the preview below.
pub(crate) fn channel_subtitle(newsletter: &Newsletter) -> String {
    if newsletter.muted {
        "Channel · muted".to_string()
    } else {
        "Channel".to_string()
    }
}

/// The line under the name of a channel.
///
/// port: the reference draws the preview of the last post of the channel. The
/// client only has the posts of the channel that is open, so the other ones
/// show the label of the channel.
fn channel_preview(app: &Whatfast, newsletter: &Newsletter) -> String {
    app.store
        .newsletter_messages(&newsletter.jid)
        .iter()
        .max_by_key(|post| post.timestamp_ms)
        .map(|post| one_line(message_text(post)))
        .filter(|preview| !preview.is_empty())
        .unwrap_or_else(|| channel_subtitle(newsletter))
}

/// The jid of a channel that was created locally.
///
/// port: `createLocalChannel` of the origin: the jid carries the moment of
/// the creation, so two channels never share the same one.
pub(crate) fn local_channel_jid(timestamp_ms: i64) -> String {
    format!("local-{timestamp_ms}@newsletter")
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

/// The panel of the newsletters.
pub(crate) struct NewsletterPanel {
    /// The state of the panel.
    state: Rc<RefCell<NewsletterPanelState>>,
    /// The invite key of the channel the user wants to follow.
    invite_input: Entity<InputState>,
    /// The name of the channel the user wants to create.
    name_input: Entity<InputState>,
    /// The description of the channel the user wants to create.
    description_input: Entity<InputState>,
    /// The search of the list of channels.
    search_input: Entity<InputState>,
    /// The subscriptions of the inputs.
    _subscriptions: Vec<Subscription>,
}

impl NewsletterPanel {
    /// Creates the panel with its inputs.
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Whatfast>) -> Self {
        let state = Rc::new(RefCell::new(NewsletterPanelState::default()));
        let invite_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Paste the invite link of the channel")
                .clean_on_escape()
        });
        let name_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Channel name")
                .clean_on_escape()
        });
        let description_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Description (optional)")
                .clean_on_escape()
        });
        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search channels")
                .clean_on_escape()
        });

        let invite_state = state.clone();
        let invite_subscription =
            cx.subscribe_in(&invite_input, window, move |app, input, event, _, cx| {
                if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                    let key = input.read(cx).value().to_string();
                    subscribe(app, &invite_state, &key, cx);
                }
                cx.notify();
            });
        let create_state = state.clone();
        let create_subscription =
            cx.subscribe_in(&name_input, window, move |app, input, event, _, cx| {
                if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                    let name = input.read(cx).value().to_string();
                    create_local_channel(app, &create_state, &name, "", cx);
                }
                cx.notify();
            });

        Self {
            state,
            invite_input,
            name_input,
            description_input,
            search_input,
            _subscriptions: vec![invite_subscription, create_subscription],
        }
    }

    /// The text of the search of the list of channels.
    pub(crate) fn search_query(&self, cx: &mut Context<Whatfast>) -> String {
        self.search_input.read(cx).value().to_string()
    }

    /// The input of the search of the list of channels.
    pub(crate) fn search_input(&self) -> &Entity<InputState> {
        &self.search_input
    }

    /// Whether the dialog to follow a channel is open.
    pub(crate) fn subscribe_open(&self) -> bool {
        self.state.borrow().subscribe_open
    }

    /// Whether the dialog to create a local channel is open.
    pub(crate) fn create_open(&self) -> bool {
        self.state.borrow().create_open
    }

    /// Opens the dialog to create a local channel.
    pub(crate) fn open_create(&self) {
        self.state.borrow_mut().create_open = true;
    }

    /// Draws the panel of the newsletters.
    pub(crate) fn render(&self, app: &Whatfast, cx: &mut Context<Whatfast>) -> gpui::Div {
        self.ensure_loaded(app, cx);

        let newsletters = app.store.newsletters();
        let selected = selected_newsletter(newsletters, app.selected_newsletter.as_deref());
        let jid = selected.map(|newsletter| newsletter.jid.clone());
        let posts = jid
            .as_deref()
            .map(|jid| app.store.newsletter_messages(jid).to_vec())
            .unwrap_or_default();
        let loading = app.newsletters_loading;

        let mut root = v_flex()
            .size_full()
            .relative()
            .child(self.render_header(app, selected, cx))
            .child(self.render_posts(app, selected, &posts, loading, cx));

        if !self.subscribe_open() && !self.create_open() {
            root = root.child(self.render_footer(cx));
        }
        if self.subscribe_open() {
            root = root.child(self.render_subscribe_dialog(cx));
        }
        if self.create_open() {
            root = root.child(self.render_create_dialog(cx));
        }
        root
    }

    /// Draws the list of the channels of the account.
    ///
    /// port: the list of channels of the sidebar of the origin
    /// (`newsletter-list`).
    pub(crate) fn render_list(
        &self,
        app: &Whatfast,
        cx: &mut Context<Whatfast>,
    ) -> gpui::Stateful<gpui::Div> {
        let scale = app.ui_scale;
        let newsletters = app.store.newsletters();
        let selected = app.selected_newsletter.clone();
        // The search of the list keeps the channels whose name has the text.
        let query = self.search_query(cx).trim().to_lowercase();
        let visible: Vec<Newsletter> = newsletters
            .iter()
            .filter(|newsletter| {
                query.is_empty() || newsletter.name.to_lowercase().contains(&query)
            })
            .cloned()
            .collect();
        let hover = cx.theme().muted;
        v_flex()
            .id("newsletter-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .children(visible.iter().enumerate().map(|(index, newsletter)| {
                let active = selected.as_deref() == Some(newsletter.jid.as_str());
                let jid = newsletter.jid.clone();
                let badge = self.state.borrow().local.contains_key(&jid);
                let preview = channel_preview(app, newsletter);
                // La hora de la suscripcion va a la derecha de la fila, y no
                // pegada al nombre, como la dibuja la referencia.
                let time = message_time(newsletter.subscribed_at_ms);
                h_flex()
                    .id(ElementId::named_usize("newsletter-row", index))
                    .w_full()
                    .gap(px(12. * scale))
                    .px(px(12. * scale))
                    .py(px(8. * scale))
                    .items_center()
                    .rounded_md()
                    .bg(if active {
                        cx.theme().accent.opacity(0.12)
                    } else {
                        gpui::transparent_black()
                    })
                    .hover(move |style| style.bg(hover))
                    .on_click(cx.listener(move |app, _, _, cx| {
                        app.select_newsletter(jid.clone());
                        cx.notify();
                    }))
                    .child(avatar_element(&newsletter.name, "", px(48. * scale), cx))
                    .child(
                        // El texto del canal: el nombre con la hora a la
                        // derecha de la fila y el preview abajo.
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(2. * scale))
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap(px(8. * scale))
                                    .items_center()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(newsletter.name.clone()),
                                    )
                                    .when(badge, |row| {
                                        row.child(
                                            div()
                                                .flex_none()
                                                .px(px(6.))
                                                .py(px(3.))
                                                .rounded_sm()
                                                .bg(cx.theme().accent.opacity(0.12))
                                                .text_color(cx.theme().accent)
                                                .text_xs()
                                                .child("local"),
                                        )
                                    })
                                    .child(
                                        div()
                                            .flex_none()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(time),
                                    ),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(preview),
                            ),
                    )
            }))
            // La seccion de los canales que se pueden seguir, bajo la lista.
            .child(self.render_suggestions(app, cx))
    }

    /// The section of the channels that the account can follow.
    ///
    /// port: the reference draws a title and, under it, a row per channel
    /// with its logo, its name with the green tick of the verified ones, the
    /// number of followers and the green button to follow it. The protocol
    /// has no RPC that asks for the suggestions, so the client draws a fixed
    /// list.
    fn render_suggestions(
        &self,
        app: &Whatfast,
        cx: &mut Context<Whatfast>,
    ) -> gpui::Stateful<gpui::Div> {
        let scale = app.ui_scale;
        let muted = cx.theme().muted_foreground;
        let accent = cx.theme().accent;
        let accent_foreground = cx.theme().accent_foreground;
        v_flex()
            .id("newsletter-suggestions")
            .flex_none()
            .px(px(12. * scale))
            .pt(px(18. * scale))
            .pb(px(6. * scale))
            .gap(px(12. * scale))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(muted)
                    .child("Search channels to follow"),
            )
            .children(
                SUGGESTED_CHANNELS
                    .iter()
                    .enumerate()
                    .map(|(index, (name, followers))| {
                        let state = self.state.clone();
                        h_flex()
                            .id(ElementId::named_usize("newsletter-suggestion-row", index))
                            .w_full()
                            .gap(px(12. * scale))
                            .py(px(6. * scale))
                            .items_center()
                            .child(avatar_element(name, "", px(48. * scale), cx))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap(px(2. * scale))
                                    .child(
                                        h_flex()
                                            .gap(px(6. * scale))
                                            .items_center()
                                            .child(
                                                div()
                                                    .min_w_0()
                                                    .truncate()
                                                    .child((*name).to_string()),
                                            )
                                            // El tilde verde del canal verificado.
                                            .child(
                                                div()
                                                    .flex_none()
                                                    .size(px(16. * scale))
                                                    .rounded_full()
                                                    .bg(accent)
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .child(
                                                        Icon::new(IconName::Check)
                                                            .with_size(px(11. * scale))
                                                            .text_color(accent_foreground),
                                                    ),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .text_sm()
                                            .text_color(muted)
                                            .child((*followers).to_string()),
                                    ),
                            )
                            // El boton verde de seguir, a la derecha de la fila.
                            .child(
                                Button::new(ElementId::named_usize(
                                    "newsletter-suggestion-follow",
                                    index,
                                ))
                                .primary()
                                .rounded(px(9999.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .label("Follow")
                                .on_click(cx.listener(
                                    move |_app, _, _, cx| {
                                        state.borrow_mut().subscribe_open = true;
                                        cx.notify();
                                    },
                                )),
                            )
                    }),
            )
    }

    /// The row of the buttons that follows the list of channels.
    ///
    /// The reference draws a button to follow a channel and another one to
    /// create a local one, at the foot of the list.
    pub(crate) fn render_list_footer(&self, cx: &mut Context<Whatfast>) -> gpui::Div {
        let state = self.state.clone();
        let create_state = self.state.clone();
        h_flex()
            .flex_none()
            .gap(px(8.))
            .justify_center()
            .p(px(10.))
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("newsletter-follow")
                    .primary()
                    .small()
                    .icon(Icon::new(IconName::Plus))
                    .label("Follow")
                    .on_click(cx.listener(move |_app, _, _, cx| {
                        state.borrow_mut().subscribe_open = true;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("newsletter-create")
                    .secondary()
                    .small()
                    .icon(Icon::new(IconName::Plus))
                    .label("New local channel")
                    .on_click(cx.listener(move |_app, _, _, cx| {
                        create_state.borrow_mut().create_open = true;
                        cx.notify();
                    })),
            )
    }

    /// The header of the panel: the channel that is open.
    fn render_header(
        &self,
        app: &Whatfast,
        selected: Option<&Newsletter>,
        cx: &mut Context<Whatfast>,
    ) -> gpui::Div {
        let scale = app.ui_scale;
        let Some(newsletter) = selected else {
            return h_flex()
                .flex_none()
                .gap(px(10.))
                .px(px(14.))
                .py(px(10.))
                .items_center()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Channel"),
                );
        };
        let jid = newsletter.jid.clone();
        let mute_jid = jid.clone();
        let muted = newsletter.muted;
        let badge = self.state.borrow().local.contains_key(&jid);
        let description = self
            .state
            .borrow()
            .local
            .get(&jid)
            .cloned()
            .unwrap_or_default();

        h_flex()
            .flex_none()
            .gap(px(10.))
            .px(px(14.))
            .py(px(10.))
            .items_center()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(10.))
                    .child(avatar_element(
                        &newsletter.name,
                        "",
                        px(HEADER_AVATAR_SIZE * scale),
                        cx,
                    ))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                h_flex()
                                    .gap(px(6.))
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .truncate()
                                            .child(newsletter.name.clone()),
                                    )
                                    .when(badge, |row| {
                                        row.child(
                                            div()
                                                .px(px(6.))
                                                .py(px(3.))
                                                .rounded_sm()
                                                .bg(cx.theme().accent.opacity(0.12))
                                                .text_color(cx.theme().accent)
                                                .text_xs()
                                                .child("local"),
                                        )
                                    })
                                    .when(muted, |row| {
                                        row.child(
                                            div()
                                                .px(px(6.))
                                                .py(px(3.))
                                                .rounded_sm()
                                                .bg(cx.theme().accent.opacity(0.12))
                                                .text_color(cx.theme().accent)
                                                .text_xs()
                                                .child("muted"),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .truncate()
                                    .child(if description.is_empty() {
                                        "Channel posts".to_string()
                                    } else {
                                        description
                                    }),
                            ),
                    ),
            )
            .child(
                Button::new("newsletter-mute")
                    .secondary()
                    .small()
                    .label(if muted { "Unmute" } else { "Mute" })
                    .on_click(cx.listener(move |app, _, _, cx| {
                        toggle_mute(app, &mute_jid);
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("newsletter-unfollow")
                    .size(px(28.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(|style| style.bg(cx.theme().muted))
                    .on_click(cx.listener(move |app, _, _, cx| {
                        app.unsubscribe_newsletter(jid.clone());
                        cx.notify();
                    }))
                    .child(
                        Icon::new(IconName::Close)
                            .with_size(px(18.))
                            .text_color(cx.theme().foreground),
                    ),
            )
    }

    /// The list of the posts of the channel that is open.
    fn render_posts(
        &self,
        app: &Whatfast,
        selected: Option<&Newsletter>,
        posts: &[Message],
        loading: bool,
        cx: &mut Context<Whatfast>,
    ) -> gpui::Stateful<gpui::Div> {
        let scale = app.ui_scale;
        let name = selected.map(|newsletter| newsletter.name.clone());
        if loading {
            return v_flex()
                .id("newsletter-posts-loading")
                .flex_1()
                .min_h_0()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("Loading channel postsÔÇª");
        }
        if posts.is_empty() {
            return v_flex()
                .id("newsletter-posts-empty")
                .flex_1()
                .min_h_0()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("This channel has no posts yet.");
        }
        v_flex()
            .id("newsletter-posts")
            .flex_1()
            .min_h_0()
            .gap(px(16. * scale))
            .p(px(12. * scale))
            .overflow_y_scroll()
            .children(posts.iter().map(|post| {
                v_flex()
                    .gap(px(6. * scale))
                    .p(px(10. * scale))
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().popover)
                    .child(
                        h_flex()
                            .gap(px(8. * scale))
                            .items_center()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(name.clone().unwrap_or_default()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(message_time(post.timestamp_ms)),
                            ),
                    )
                    .child(render_post(post, scale, cx))
            }))
    }

    /// The buttons that follow a channel or that create one.
    fn render_footer(&self, cx: &mut Context<Whatfast>) -> gpui::Div {
        let state = self.state.clone();
        let create_state = self.state.clone();
        h_flex()
            .flex_none()
            .gap(px(8.))
            .justify_center()
            .p(px(10.))
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("newsletter-follow")
                    .primary()
                    .small()
                    .icon(Icon::new(IconName::Plus))
                    .label("Follow")
                    .on_click(cx.listener(move |_app, _, _, cx| {
                        state.borrow_mut().subscribe_open = true;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("newsletter-create")
                    .secondary()
                    .small()
                    .icon(Icon::new(IconName::Plus))
                    .label("New local channel")
                    .on_click(cx.listener(move |_app, _, _, cx| {
                        create_state.borrow_mut().create_open = true;
                        cx.notify();
                    })),
            )
    }

    /// The dialog that follows a channel with its invite key.
    fn render_subscribe_dialog(&self, cx: &mut Context<Whatfast>) -> gpui::Stateful<gpui::Div> {
        let state = self.state.clone();
        let confirm_state = state.clone();
        let invite = self.invite_input.clone();
        let key = self.invite_input.read(cx).value().to_string();
        v_flex()
            .id("newsletter-subscribe-dialog")
            .absolute()
            .inset_0()
            .gap_3()
            .p(px(24.))
            .items_center()
            .justify_center()
            .bg(cx.theme().overlay)
            .child(
                v_flex()
                    .w(px(420.))
                    .gap_3()
                    .p(px(16.))
                    .rounded_lg()
                    .bg(cx.theme().popover)
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Follow a channel"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Paste the invite link of the channel you want to follow."),
                    )
                    .child(Input::new(&self.invite_input).w_full())
                    .child(
                        h_flex()
                            .gap_2()
                            .justify_end()
                            .child(
                                Button::new("newsletter-subscribe-cancel")
                                    .secondary()
                                    .small()
                                    .label("Cancel")
                                    .on_click(cx.listener(move |_app, _, _, cx| {
                                        state.borrow_mut().subscribe_open = false;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("newsletter-subscribe-confirm")
                                    .primary()
                                    .small()
                                    .label("Follow")
                                    .disabled(key.trim().is_empty())
                                    .on_click(cx.listener(move |app, _, _, cx| {
                                        let key = invite.read(cx).value().to_string();
                                        subscribe(app, &confirm_state, &key, cx);
                                    })),
                            ),
                    ),
            )
    }

    /// The dialog that creates a channel in the memory of the client.
    fn render_create_dialog(&self, cx: &mut Context<Whatfast>) -> gpui::Stateful<gpui::Div> {
        let state = self.state.clone();
        let cancel_state = state.clone();
        let confirm_state = state.clone();
        let name_input = self.name_input.clone();
        let description_input = self.description_input.clone();
        let name = self.name_input.read(cx).value().to_string();
        v_flex()
            .id("newsletter-create-dialog")
            .absolute()
            .inset_0()
            .gap_3()
            .p(px(24.))
            .items_center()
            .justify_center()
            .bg(cx.theme().overlay)
            .child(
                v_flex()
                    .w(px(420.))
                    .gap_3()
                    .p(px(16.))
                    .rounded_lg()
                    .bg(cx.theme().popover)
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("New local channel"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Stored in memory only; a linked device has no channel RPC."),
                    )
                    .child(Input::new(&self.name_input).w_full())
                    .child(Input::new(&self.description_input).w_full())
                    .child(
                        h_flex()
                            .gap_2()
                            .justify_end()
                            .child(
                                Button::new("newsletter-create-cancel")
                                    .secondary()
                                    .small()
                                    .label("Cancel")
                                    .on_click(cx.listener(move |_app, _, _, cx| {
                                        cancel_state.borrow_mut().create_open = false;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("newsletter-create-confirm")
                                    .primary()
                                    .small()
                                    .label("Create")
                                    .disabled(name.trim().is_empty())
                                    .on_click(cx.listener(move |app, _, _, cx| {
                                        let name = name_input.read(cx).value().to_string();
                                        let description =
                                            description_input.read(cx).value().to_string();
                                        create_local_channel(
                                            app,
                                            &confirm_state,
                                            &name,
                                            &description,
                                            cx,
                                        );
                                    })),
                            ),
                    ),
            )
    }

    /// Asks for the channels when they are not there yet.
    fn ensure_loaded(&self, app: &Whatfast, cx: &mut Context<Whatfast>) {
        if !app.store.newsletters().is_empty() {
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
                app.load_newsletters();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

/// Follows a channel with its invite key.
///
/// port: the origin subscribes with the trimmed key, and it only does it
/// when the key has something in it.
fn subscribe(
    app: &mut Whatfast,
    state: &Rc<RefCell<NewsletterPanelState>>,
    key: &str,
    cx: &mut Context<Whatfast>,
) {
    if key.trim().is_empty() {
        return;
    }
    app.subscribe_newsletter(key.trim().to_string());
    state.borrow_mut().subscribe_open = false;
    cx.notify();
}

/// Creates a channel in the memory of the client.
///
/// port: `createLocalChannel` of the origin: a linked device has no RPC for
/// a channel, so the channel stays in the memory of the client.
fn create_local_channel(
    app: &mut Whatfast,
    state: &Rc<RefCell<NewsletterPanelState>>,
    name: &str,
    description: &str,
    cx: &mut Context<Whatfast>,
) {
    let title = name.trim();
    if title.is_empty() {
        return;
    }
    let timestamp = now_ms();
    let jid = local_channel_jid(timestamp);
    app.store.add_newsletter(Newsletter {
        jid: jid.clone(),
        name: title.to_string(),
        subscribed_at_ms: timestamp,
        muted: false,
    });
    state
        .borrow_mut()
        .local
        .insert(jid.clone(), description.trim().to_string());
    state.borrow_mut().create_open = false;
    // port: the origin does not open the channel it creates: a local channel
    // has no posts, and there is no RPC that could ask for them.
    cx.notify();
}

/// Turns the sound of a channel on or off.
///
/// port: `toggleNewsletterMute` of the origin: the flag lives in the state
/// of the client, because the protocol has no RPC for it.
fn toggle_mute(app: &mut Whatfast, jid: &str) {
    let mut newsletters = app.store.newsletters().to_vec();
    if let Some(newsletter) = newsletters
        .iter_mut()
        .find(|newsletter| newsletter.jid == jid)
    {
        newsletter.muted = !newsletter.muted;
    }
    app.store.set_newsletters(newsletters);
}

/// Draws the content of one post of a channel.
fn render_post(post: &Message, scale: f32, cx: &mut Context<Whatfast>) -> gpui::Div {
    if let Some(Content::Image(image)) = &post.content {
        let path = if image.thumbnail_path.is_empty() {
            image.local_path.clone()
        } else {
            image.thumbnail_path.clone()
        };
        if !path.is_empty() {
            return v_flex()
                .gap_2()
                .child(
                    img(PathBuf::from(path))
                        .max_w(px(320. * scale))
                        .max_h(px(320. * scale))
                        .object_fit(ObjectFit::Contain),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().foreground)
                        .child(image.caption.clone()),
                );
        }
    }
    v_flex().gap_2().child(
        div()
            .text_sm()
            .text_color(cx.theme().foreground)
            .child(message_text(post).to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn newsletter(jid: &str, name: &str, muted: bool) -> Newsletter {
        Newsletter {
            jid: jid.to_string(),
            name: name.to_string(),
            subscribed_at_ms: 1_900_000_000_000,
            muted,
        }
    }

    #[test]
    fn the_letter_of_the_avatar_is_the_first_letter_of_the_name() {
        assert_eq!(avatar_label("Whatfast News"), "W");
        assert_eq!(avatar_label("  whatfast"), "W");
    }

    #[test]
    fn a_channel_without_a_name_shows_a_c() {
        assert_eq!(avatar_label(""), "C");
        assert_eq!(avatar_label("   "), "C");
    }

    #[test]
    fn the_channel_that_is_open_is_the_one_that_was_chosen() {
        let newsletters = vec![
            newsletter("news@newsletter", "Whatfast News", false),
            newsletter("dev@newsletter", "Whatfast Dev", false),
        ];
        assert_eq!(
            selected_newsletter(&newsletters, Some("dev@newsletter")).map(|item| item.name.clone()),
            Some("Whatfast Dev".to_string())
        );
    }

    #[test]
    fn there_is_no_channel_when_none_was_chosen() {
        let newsletters = vec![newsletter("news@newsletter", "Whatfast News", false)];
        assert!(selected_newsletter(&newsletters, None).is_none());
        assert!(selected_newsletter(&newsletters, Some("gone@newsletter")).is_none());
    }

    #[test]
    fn a_muted_channel_says_that_it_is_muted() {
        let channel = newsletter("news@newsletter", "Whatfast News", true);
        assert!(channel_subtitle(&channel).ends_with("· muted"));
        let channel = newsletter("news@newsletter", "Whatfast News", false);
        assert!(!channel_subtitle(&channel).contains("muted"));
    }

    #[test]
    fn a_local_channel_carries_the_moment_of_its_creation() {
        assert_eq!(local_channel_jid(42), "local-42@newsletter");
    }

    #[test]
    fn two_local_channels_never_share_the_same_jid() {
        assert_ne!(local_channel_jid(42), local_channel_jid(43));
    }
}
