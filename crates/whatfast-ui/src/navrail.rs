//! The column of icons at the left of the window.
//!
//! port: the `nav` column of the grid of the shell (`tauri/src/styles.css`,
//! `.app-shell` and `.nav-rail`) with the four buttons the oracle draws in it
//! (`tauri/src/App.tsx`, the `<nav class="nav-rail">`).
//!
//! The column is a component of its own: the app owns an `Entity<NavRail>`,
//! keeps its state in sync and drops the entity in the layout of the shell.
//! Every button reports what the user pressed to the handler the app
//! installs with [`NavRail::on_action`]; the column never changes the state of
//! the app by itself.

use std::rc::Rc;
use whatfast_i18n as i18n;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    Render, StatefulInteractiveElement as _, Styled as _, Window, div, px, rgba,
};
use gpui_component::{ActiveTheme as _, Icon, Sizable as _};

use crate::icons::WhatfastIcon;
use crate::tokens::normalized_ui_scale;

/// The width of the column, before the scale.
///
/// port: the first column of the grid of the shell:
/// `grid-template-columns: calc(56px * var(--scale)) ...`.
const RAIL_WIDTH: f32 = 56.0;

/// The size of a button of the column, before the scale.
///
/// port: `.icon-button { width: calc(34px * var(--scale)); height: ... }`.
/// The button of the reference is the grey circle around the glyph, so the
/// circle is a little wider than the plain button of the web client.
const BUTTON_SIZE: f32 = 40.0;

/// The gap between the buttons, before the scale.
///
/// port: `.nav-rail { gap: calc(7px * var(--scale)) }`.
const BUTTON_GAP: f32 = 7.0;

/// The padding above and below the column, before the scale.
///
/// port: `.nav-rail { padding: calc(12px * var(--scale)) 0 }`.
const RAIL_PADDING: f32 = 12.0;

/// The size of the glyph of the list of chats, before the scale.
///
/// port: `<ThemeIcon icon={MessageCircle} name="chat" size={22} />`.
const CHATS_ICON_SIZE: f32 = 22.0;

/// The size of the other glyphs, before the scale.
///
/// port: the search, the shield and the settings are drawn with `size={20}`.
const ICON_SIZE: f32 = 20.0;

/// The state of the link with the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinkState {
    /// The app is still pairing: there are no chats to show yet.
    Pairing,
    /// The device is linked: the chats, the search and the privacy are
    /// reachable.
    Linked,
}

/// What a button of the column does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NavAction {
    /// The list of chats: opens the chat list and shows every chat.
    Chats,
    /// The quick search of the chats.
    Search,
    /// The states of the contacts: they are not conversations, so they live
    /// in their own view.
    States,
    /// The channels of the newsletters: they are not conversations, so they
    /// live in their own view.
    Channels,
    /// The agent harnesses of this machine and their drafts: they have their
    /// own view too.
    Agents,
    /// The conversations that were archived.
    Archived,
    /// The privacy section of the settings.
    Privacy,
    /// The settings.
    Settings,
}

impl NavAction {
    /// The glyph of the action.
    ///
    /// port: the oracle draws a `MessageCircle`, a `Search`, a `ShieldCheck`
    /// and a `Settings` (lucide). The set of `gpui_component` has no chat
    /// bubble and no shield, so the folder and the eye of the set stand in
    /// for them.
    pub(crate) fn icon(self) -> Icon {
        match self {
            // port: the oracle draws a `MessageCircle` for the chats and a
            // `ShieldCheck` for the privacy. El set propio de Whatfast trae
            // la burbuja del chat y el escudo, que son los mismos dibujos.
            NavAction::Chats => WhatfastIcon::Chat.into(),
            NavAction::Search => WhatfastIcon::Search.into(),
            // The states are the ring of the reference: two circles, one
            // inside the other, the same glyph of WhatsApp.
            NavAction::States => WhatfastIcon::Status.into(),
            // The channels are the bubble with the dot of the reference.
            NavAction::Channels => WhatfastIcon::Channels.into(),
            // The agents are a robot: the set of Whatfast has its own.
            NavAction::Agents => WhatfastIcon::Bot.into(),
            // The archived conversations go into a box: the set of Whatfast
            // has the archive, the one of gpui_component no.
            NavAction::Archived => WhatfastIcon::Archive.into(),
            NavAction::Privacy => WhatfastIcon::Shield.into(),
            NavAction::Settings => WhatfastIcon::Settings.into(),
        }
    }

    /// The size of the glyph of the action, before the scale.
    pub(crate) fn icon_size(self) -> f32 {
        match self {
            NavAction::Chats => CHATS_ICON_SIZE,
            NavAction::Search
            | NavAction::States
            | NavAction::Channels
            | NavAction::Agents
            | NavAction::Archived
            | NavAction::Privacy
            | NavAction::Settings => ICON_SIZE,
        }
    }

    /// The label of the action: the tooltip and the accessible name.
    ///
    /// port: the labels of the oracle: "Chats", `titlebar.quickSearch`,
    /// `titlebar.privacy` and `titlebar.settings`.
    pub(crate) fn label(self) -> &'static str {
        i18n::t(match self {
            NavAction::Chats => "Chats",
            NavAction::Search => "Quick search (Ctrl+K)",
            NavAction::States => "States",
            NavAction::Channels => "Channels",
            NavAction::Agents => "Agents",
            NavAction::Archived => "Archived",
            NavAction::Privacy => "Privacy architecture",
            NavAction::Settings => "Settings",
        })
    }
}

/// The view that occupies the column next to the rail.
///
/// One value only: five independent booleans let a command open the settings
/// while the states were still in front, and the rail then lit two buttons.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SidebarView {
    #[default]
    Chats,
    States,
    Channels,
    Agents,
    Archived,
    Settings,
}

/// A button of the column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NavButton {
    /// What the button does.
    pub(crate) action: NavAction,
    /// Whether the button is in its active state.
    pub(crate) active: bool,
    /// Whether the button can be pressed.
    pub(crate) enabled: bool,
    /// The unread messages of the destination, or zero for none.
    pub(crate) badge: u32,
    /// The green dot of the reference: there is something new there.
    pub(crate) dot: bool,
}

/// The handler the buttons report to.
pub(crate) type NavActionHandler = Option<Rc<dyn Fn(NavAction, &mut Window, &mut App)>>;

/// The column of icons at the left of the window.
pub(crate) struct NavRail {
    /// The state of the link with the device.
    link: LinkState,
    /// The list of chats is open.
    sidebar_open: bool,
    /// The view in front of the column.
    view: SidebarView,
    /// The unread messages of the list of chats.
    unread: u32,
    /// Whether there are states that were not seen.
    states_unseen: bool,
    /// The scale of the interface.
    scale: f32,
    /// The handler the buttons report to.
    on_action: NavActionHandler,
}

impl std::fmt::Debug for NavRail {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NavRail")
            .field("link", &self.link)
            .field("sidebar_open", &self.sidebar_open)
            .field("view", &self.view)
            .field("scale", &self.scale)
            .finish_non_exhaustive()
    }
}

impl NavRail {
    /// The column of icons for an interface scale.
    pub(crate) fn new(scale: f32) -> Self {
        Self {
            link: LinkState::Pairing,
            sidebar_open: true,
            view: SidebarView::Chats,
            unread: 0,
            states_unseen: false,
            scale: normalized_ui_scale(scale),
            on_action: None,
        }
    }

    /// The handler the buttons report to.
    pub(crate) fn on_action(
        mut self,
        handler: impl Fn(NavAction, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_action = Some(Rc::new(handler));
        self
    }

    /// The app is pairing, or the device is linked.
    pub(crate) fn set_link(&mut self, link: LinkState) {
        self.link = link;
    }

    /// The list of chats is open, or it is collapsed.
    pub(crate) fn set_sidebar_open(&mut self, open: bool) {
        self.sidebar_open = open;
    }

    /// Opens one view; the other views are closed by construction.
    pub(crate) fn set_view(&mut self, view: SidebarView) {
        self.view = view;
    }

    /// The unread messages of the list of chats.
    pub(crate) fn set_unread(&mut self, unread: u32) {
        self.unread = unread;
    }

    /// Whether there are states that were not seen.
    pub(crate) fn set_states_unseen(&mut self, unseen: bool) {
        self.states_unseen = unseen;
    }

    /// The buttons of the column, above the spacer.
    ///
    /// port: the oracle draws the list of chats and the quick search above the
    /// spacer, and the privacy and the settings below it.
    pub(crate) fn top_buttons(&self) -> Vec<NavButton> {
        vec![
            NavButton {
                action: NavAction::Chats,
                // port: the button of the oracle is active when the chat list
                // is not collapsed. Los estados, los canales y los archivados
                // son vistas propias: el boton de los chats no queda activo
                // mientras cualquiera de ellas esta en frente.
                active: self.sidebar_open && self.view == SidebarView::Chats,
                enabled: self.link == LinkState::Linked,
                badge: self.unread,
                dot: false,
            },
            NavButton {
                action: NavAction::Search,
                active: false,
                enabled: self.link == LinkState::Linked,
                badge: 0,
                dot: false,
            },
            NavButton {
                action: NavAction::States,
                // The states are their own view: the button is active when it
                // is the one in front and the column is showing. Un panel
                // cerrado no deja ningun boton encendido.
                active: self.view == SidebarView::States && self.sidebar_open,
                enabled: self.link == LinkState::Linked,
                badge: 0,
                // El punto verde de la referencia: hay estados sin ver.
                dot: self.states_unseen,
            },
            NavButton {
                action: NavAction::Channels,
                // The channels are their own view too.
                active: self.view == SidebarView::Channels && self.sidebar_open,
                enabled: self.link == LinkState::Linked,
                badge: 0,
                dot: false,
            },
            NavButton {
                action: NavAction::Agents,
                // The agents are their own view: the screen of the harnesses
                // and the tray of their drafts.
                active: self.view == SidebarView::Agents && self.sidebar_open,
                enabled: true,
                badge: 0,
                dot: false,
            },
        ]
    }

    /// The buttons of the column, below the spacer.
    pub(crate) fn bottom_buttons(&self) -> Vec<NavButton> {
        vec![
            NavButton {
                action: NavAction::Archived,
                // The archived conversations are their own view.
                active: self.view == SidebarView::Archived && self.sidebar_open,
                enabled: self.link == LinkState::Linked,
                badge: 0,
                dot: false,
            },
            NavButton {
                action: NavAction::Privacy,
                active: false,
                enabled: self.link == LinkState::Linked,
                badge: 0,
                dot: false,
            },
            NavButton {
                action: NavAction::Settings,
                // port: the button of the oracle is active when the settings
                // are open.
                active: self.view == SidebarView::Settings,
                enabled: true,
                badge: 0,
                dot: false,
            },
        ]
    }

    /// The column of icons.
    pub(crate) fn render(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let scale = self.scale;
        let theme = cx.theme();
        let background = theme.sidebar;
        let border = theme.border;

        let mut column = div()
            .id("nav-rail")
            .h_full()
            .w(column_width(scale))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(BUTTON_GAP * scale))
            .py(px(RAIL_PADDING * scale))
            .bg(background)
            .border_r_1()
            .border_color(border);

        for button in self.top_buttons() {
            column = column.child(self.render_button(button, cx));
        }

        // port: the spacer of the oracle (`.nav-rail .nav-spacer`) pushes the
        // privacy and the settings to the bottom of the column.
        column = column.child(div().flex_1());

        for button in self.bottom_buttons() {
            column = column.child(self.render_button(button, cx));
        }

        column
    }

    /// A button of the column.
    fn render_button(&self, button: NavButton, cx: &mut Context<Self>) -> impl IntoElement {
        let scale = self.scale;
        let theme = cx.theme();
        let foreground = theme.foreground;
        let muted = theme.muted_foreground;
        let hover = theme.muted;

        let action = button.action;
        let handler = self.on_action.clone();
        let active = button.active;
        let enabled = button.enabled;

        div()
            .id(format!("nav-rail-button-{}", action.label()))
            .relative()
            .size(px(BUTTON_SIZE * scale))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .cursor_pointer()
            // El boton activo de la referencia es un cuadro gris claro con
            // el glifo claro: no lleva el color de acento.
            .when(active, |this| {
                this.bg(cx.theme().muted).text_color(foreground)
            })
            .when(!active, |this| this.text_color(muted))
            .when(enabled && !active, |this| {
                this.hover(|style| style.bg(hover).text_color(foreground))
            })
            .when(!enabled, |this| this.opacity(0.4).cursor_default())
            .aria_label(action.label())
            .on_click(move |_, window, cx| {
                if !enabled {
                    return;
                }
                if let Some(handler) = handler.clone() {
                    handler(action, window, cx);
                }
            })
            .child(Icon::new(action.icon()).with_size(px(action.icon_size() * scale)))
            // El circulo verde de los no leidos: el numero va blanco sobre el
            // verde de WhatsApp, como en la referencia.
            .when(button.badge > 0, |this| {
                this.child(
                    div()
                        .absolute()
                        .top(px(-3. * scale))
                        .right(px(-3. * scale))
                        .h(px(16. * scale))
                        .min_w(px(16. * scale))
                        .px(px(3. * scale))
                        .rounded_full()
                        .bg(rgba(0x25d366ff))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(10. * scale))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgba(0xffffffff))
                        .child(if button.badge > 99 {
                            "99+".to_string()
                        } else {
                            button.badge.to_string()
                        }),
                )
            })
            // El punto del mismo verde: hay estados sin ver.
            .when(button.dot, |this| {
                this.child(
                    div()
                        .absolute()
                        .top(px(1. * scale))
                        .right(px(1. * scale))
                        .size(px(9. * scale))
                        .rounded_full()
                        .bg(rgba(0x25d366ff)),
                )
            })
    }
}

impl Render for NavRail {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        NavRail::render(self, cx)
    }
}

/// The width of the column of icons, for an interface scale.
///
/// port: the first column of the grid of the shell:
/// `grid-template-columns: calc(56px * var(--scale)) ...`.
pub(crate) fn column_width(scale: f32) -> Pixels {
    px(RAIL_WIDTH * normalized_ui_scale(scale))
}

