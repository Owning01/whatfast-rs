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

// The module arrives before its caller: the shell of the batch B-U6 adds the
// column to the layout.
#![allow(dead_code)]

use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, Context, InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Render,
    StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _};

use crate::tokens::normalized_ui_scale;

/// The width of the column, before the scale.
///
/// port: the first column of the grid of the shell:
/// `grid-template-columns: calc(56px * var(--scale)) ...`.
const RAIL_WIDTH: f32 = 56.0;

/// The size of a button of the column, before the scale.
///
/// port: `.icon-button { width: calc(34px * var(--scale)); height: ... }`.
const BUTTON_SIZE: f32 = 34.0;

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
    pub(crate) fn icon(self) -> IconName {
        match self {
            NavAction::Chats => IconName::Folder,
            NavAction::Search => IconName::Search,
            NavAction::Privacy => IconName::Eye,
            NavAction::Settings => IconName::Settings,
        }
    }

    /// The size of the glyph of the action, before the scale.
    pub(crate) fn icon_size(self) -> f32 {
        match self {
            NavAction::Chats => CHATS_ICON_SIZE,
            NavAction::Search | NavAction::Privacy | NavAction::Settings => ICON_SIZE,
        }
    }

    /// The label of the action: the tooltip and the accessible name.
    ///
    /// port: the labels of the oracle: "Chats", `titlebar.quickSearch`,
    /// `titlebar.privacy` and `titlebar.settings`.
    pub(crate) fn label(self) -> &'static str {
        match self {
            NavAction::Chats => "Chats",
            NavAction::Search => "Quick search (Ctrl+K)",
            NavAction::Privacy => "Privacy architecture",
            NavAction::Settings => "Settings",
        }
    }
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
}

/// The handler the buttons report to.
pub(crate) type NavActionHandler = Option<Rc<dyn Fn(NavAction, &mut Window, &mut App)>>;

/// The column of icons at the left of the window.
pub(crate) struct NavRail {
    /// The state of the link with the device.
    link: LinkState,
    /// The list of chats is open.
    sidebar_open: bool,
    /// The settings panel is open.
    settings_open: bool,
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
            .field("settings_open", &self.settings_open)
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
            settings_open: false,
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

    /// The state of the link with the device.
    pub(crate) fn link(&self) -> LinkState {
        self.link
    }

    /// The app is pairing, or the device is linked.
    pub(crate) fn set_link(&mut self, link: LinkState) {
        self.link = link;
    }

    /// Whether the list of chats is open.
    pub(crate) fn sidebar_open(&self) -> bool {
        self.sidebar_open
    }

    /// The list of chats is open, or it is collapsed.
    pub(crate) fn set_sidebar_open(&mut self, open: bool) {
        self.sidebar_open = open;
    }

    /// Whether the settings panel is open.
    pub(crate) fn settings_open(&self) -> bool {
        self.settings_open
    }

    /// The settings panel is open, or it is closed.
    pub(crate) fn set_settings_open(&mut self, open: bool) {
        self.settings_open = open;
    }

    /// The scale of the interface.
    pub(crate) fn scale(&self) -> f32 {
        self.scale
    }

    /// The width of the column.
    ///
    /// port: the first column of the grid of the shell:
    /// `calc(56px * var(--scale))`.
    pub(crate) fn width(&self) -> Pixels {
        column_width(self.scale)
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
                // is not collapsed.
                active: self.sidebar_open,
                enabled: self.link == LinkState::Linked,
            },
            NavButton {
                action: NavAction::Search,
                active: false,
                enabled: self.link == LinkState::Linked,
            },
        ]
    }

    /// The buttons of the column, below the spacer.
    pub(crate) fn bottom_buttons(&self) -> Vec<NavButton> {
        vec![
            NavButton {
                action: NavAction::Privacy,
                active: false,
                enabled: self.link == LinkState::Linked,
            },
            NavButton {
                action: NavAction::Settings,
                // port: the button of the oracle is active when the settings
                // are open.
                active: self.settings_open,
                enabled: true,
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
        let accent = theme.accent;
        let accent_soft = theme.selection;
        let radius = theme.radius;

        let action = button.action;
        let handler = self.on_action.clone();
        let active = button.active;
        let enabled = button.enabled;

        div()
            .id(format!("nav-rail-button-{}", action.label()))
            .size(px(BUTTON_SIZE * scale))
            .flex()
            .items_center()
            .justify_center()
            .rounded(radius)
            .cursor_pointer()
            .when(active, |this| this.bg(accent_soft).text_color(accent))
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

#[cfg(test)]
mod tests {
    use gpui::px;

    use super::{
        BUTTON_GAP, BUTTON_SIZE, ICON_SIZE, LinkState, NavAction, NavRail, RAIL_PADDING,
        RAIL_WIDTH, column_width,
    };

    /// The column of icons of a linked app.
    fn rail() -> NavRail {
        let mut rail = NavRail::new(1.0);
        rail.set_link(LinkState::Linked);
        rail
    }

    #[test]
    fn the_width_of_the_column_is_the_first_column_of_the_grid() {
        assert_eq!(RAIL_WIDTH, 56.0);
        assert_eq!(column_width(1.0), px(56.0));
        assert_eq!(column_width(1.5), px(84.0));
    }

    #[test]
    fn the_width_of_the_column_normalizes_the_scale_first() {
        assert_eq!(column_width(2.4), px(84.0));
        assert_eq!(column_width(0.0), px(56.0));
    }

    #[test]
    fn the_width_of_the_column_of_the_rail_is_the_width_of_the_column() {
        let mut rail = NavRail::new(1.3);
        assert_eq!(rail.width(), px(RAIL_WIDTH * 1.3));
        rail.set_link(LinkState::Linked);
        assert_eq!(rail.width(), column_width(1.3));
    }

    #[test]
    fn the_measures_of_the_column_come_from_the_stylesheet() {
        assert_eq!(BUTTON_SIZE, 34.0);
        assert_eq!(BUTTON_GAP, 7.0);
        assert_eq!(RAIL_PADDING, 12.0);
        assert_eq!(ICON_SIZE, 20.0);
    }

    #[test]
    fn the_column_has_the_four_buttons_of_the_oracle() {
        let rail = rail();
        let top: Vec<NavAction> = rail
            .top_buttons()
            .iter()
            .map(|button| button.action)
            .collect();
        let bottom: Vec<NavAction> = rail
            .bottom_buttons()
            .iter()
            .map(|button| button.action)
            .collect();
        assert_eq!(
            top,
            vec![NavAction::Chats, NavAction::Search],
            "the chats and the search are above the spacer"
        );
        assert_eq!(
            bottom,
            vec![NavAction::Privacy, NavAction::Settings],
            "the privacy and the settings are below the spacer"
        );
    }

    #[test]
    fn the_button_of_the_chats_is_active_when_the_list_is_open() {
        let mut rail = rail();
        assert!(rail.top_buttons()[0].active);
        rail.set_sidebar_open(false);
        assert!(!rail.top_buttons()[0].active);
        assert!(!rail.sidebar_open());
    }

    #[test]
    fn the_button_of_the_settings_is_active_when_the_settings_are_open() {
        let mut rail = rail();
        assert!(!rail.bottom_buttons()[1].active);
        rail.set_settings_open(true);
        assert!(rail.bottom_buttons()[1].active);
        assert!(rail.settings_open());
    }

    #[test]
    fn the_pairing_leaves_the_chats_the_search_and_the_privacy_without_action() {
        let rail = NavRail::new(1.0);
        assert_eq!(rail.link(), LinkState::Pairing);
        assert!(!rail.top_buttons()[0].enabled, "the chats");
        assert!(!rail.top_buttons()[1].enabled, "the search");
        assert!(!rail.bottom_buttons()[0].enabled, "the privacy");
        assert!(
            rail.bottom_buttons()[1].enabled,
            "the settings are always reachable"
        );
    }

    #[test]
    fn the_linked_app_reaches_the_chats_the_search_and_the_privacy() {
        let rail = rail();
        assert_eq!(rail.link(), LinkState::Linked);
        assert!(rail.top_buttons()[0].enabled);
        assert!(rail.top_buttons()[1].enabled);
        assert!(rail.bottom_buttons()[0].enabled);
    }
}
