//! The drawing of the Whatfast client.
//!
//! port: taken from the origin's `desktop/src/main.rs`. Every `render_*`
//! method of the GPUI client lives here, with the same colours, sizes,
//! spacing, texts and behaviour: the origin is the visual and functional
//! reference, this file is its translation.
//!
//! The state lives in `app.rs` (the other half of the origin's `main.rs`),
//! the free helpers of the drawing in `helpers.rs`.

use std::collections::HashSet;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    ClickEvent, Context, CursorStyle, ElementId, Focusable as _, FontWeight,
    InteractiveElement as _, IntoElement, KeyDownEvent, ModifiersChangedEvent, MouseButton,
    MouseMoveEvent, MouseUpEvent, ObjectFit, ParentElement as _, Pixels, Render, ScrollStrategy,
    SharedString, StatefulInteractiveElement as _, Styled as _, StyledImage as _, Window, canvas,
    div, img, point, px, relative, rgb, rgba, size, uniform_list,
};
use gpui_component::{
    ActiveTheme as _, Colorize as _, Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    scroll::{ScrollableElement as _, ScrollbarAxis},
    slider::Slider,
    switch::Switch,
    text::{TextView, TextViewStyle},
    tooltip::Tooltip,
    v_flex, v_virtual_list,
};
use qrcode::QrCode;
use whatfast_core::theme::{Theme as AppTheme, ThemeAppearance, ThemeToken};
use whatfast_i18n as i18n;
use whatfast_model::settings::Density;
// The origin's `crate::proto` is now the protocol crate: the same types, with
// the same names, at the root of the crate.
use whatfast_protocol as proto;

use crate::attachments;
use crate::icons::WhatfastIcon;
use crate::local_time::local_offset_minutes;

use whatfast_model::emoji_picker::EmojiCategory;
use whatfast_model::geom;
use whatfast_model::model::{
    ChatFilter, Screen, image_row_path, image_viewer_path, reaction_counts, reaction_sender_detail,
    reaction_sender_name,
};

use crate::app::{
    ACCENT_SWATCHES, ChatContextAction, ChatMenuAction, CycleRecentChat, CycleRecentChatReverse,
    DARK_GREEN, EMOJI_COLUMNS, EmojiTarget, EscapeAction, FocusComposerAction, FocusSearch,
    MAX_UI_SCALE, MIN_UI_SCALE, OpenQuickAsk, OpenSettingsAction, PANEL, PageDownMessages,
    PageUpMessages, PanelTab, PinDialogMode, ScrollSurface, ScrollToBottomAction,
    ScrollToTopAction, SearchResultRow, SelectNextChat, SelectPreviousChat, ToggleAgents,
    ToggleArchived, ToggleChannels, ToggleStates, UI_SCALE_STEP, UpdatePhase, Whatfast,
};
use crate::emoji_recent;
use crate::helpers::{
    StatusGlyph, avatar_element, avatar_element_of, connection_label, format_disappearing_timer,
    format_epoch_date, format_epoch_date_numeric, identity_hue, image_needs_download,
    image_render_size, image_source_candidates, message_preview, message_status_glyph, one_line,
    qr_canvas,
};
use crate::rpc::PendingRequest;
use crate::text::{is_safe_web_url, link_preview_host, mention_display_name, split_url_token};
use crate::tokens;
use crate::ubicaciones;

/// One entry of the menu of a row of the list of conversations.
enum ChatContextEntry {
    /// An action of the menu, with its text and its icon.
    Item(Box<(ChatContextAction, SharedString, Icon)>),
    /// The line that separates two groups of actions.
    Separator,
}

/// One entry of the menu of the options of a conversation.
enum ChatMenuEntry {
    /// A row of the menu: the action is `None` when the app can not do it.
    Item(Box<(Option<ChatMenuAction>, SharedString, Icon)>),
    /// The line that separates two groups of rows.
    Separator,
}

/// The state of the menu of the filters of the column of chats.
///
/// The state of the application lives in `app.rs`; this one belongs to the
/// menu of the filters, which is drawn here, so it is a global of GPUPI: the
/// menu is a thing of the interface, not of the application.
#[derive(Default)]
struct ChatFilterMenu {
    /// Whether the menu is open.
    open: bool,
    /// Where the pointer was when the menu opened.
    position: gpui::Point<Pixels>,
}

impl gpui::Global for ChatFilterMenu {}

/// The height of a line of text, as a multiple of the size of its font.
///
/// The font of the system, que es el que usa la aplicacion, pide esta
/// altura para su linea: la columna de la derecha de una fila de
/// conversaciones (la hora y los dibujitos) tiene que alinearse con las dos
/// lineas del texto, asi que sus dos lineas usan esta altura.
const LINE_HEIGHT: f32 = 1.53;

/// The size of the preview of a conversation, as a multiple of the size of
/// the font of the interface (the same that `text_sm` puts).
const PREVIEW_FONT_RATIO: f32 = 0.875;

impl Whatfast {
    fn render_pairing(&self, cx: &mut Context<Self>) -> gpui::Div {
        let qr = self
            .store
            .qr_code
            .as_deref()
            .and_then(|code| QrCode::new(code.as_bytes()).ok());
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_4()
            .child(
                div()
                    .text_2xl()
                    .font_weight(FontWeight::BOLD)
                    .child(i18n::t("Link Whatfast")),
            )
            .child(i18n::t("WhatsApp → Linked devices → Link a device"))
            .child(match qr {
                Some(qr) => qr_canvas(qr),
                None => div()
                    .size(px(240.))
                    .rounded_lg()
                    .bg(if cx.theme().is_dark() {
                        cx.theme().secondary
                    } else {
                        rgb(PANEL).into()
                    })
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(i18n::t("Waiting for QR…")),
            })
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(i18n::t("The code refreshes automatically.")),
            )
            .child(
                Button::new("retry-pairing")
                    .label(i18n::t("Refresh pairing code"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.begin_pairing();
                        cx.notify();
                    })),
            )
    }

    fn render_center(
        &self,
        title: impl Into<SharedString>,
        detail: impl Into<SharedString>,
    ) -> gpui::Div {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::BOLD)
                    .child(title.into()),
            )
            .child(div().text_sm().child(detail.into()))
    }

    fn render_chats(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui::Div {
        let selected = self.store.selected_chat_id.clone();
        // The split view keeps the list beside the conversation: on a narrow
        // window the list would hide when a chat opens.
        let compact = !self.split_view && window.viewport_size().width < px(800. * self.ui_scale);
        // La columna de los chats se esconde con su boton, pero no cuando no
        // hay ninguna conversacion abierta: ahi no quedaria nada en la
        // ventana y no habria donde tocar para traerla de vuelta.
        let show_sidebar =
            (!self.sidebar_hidden || selected.is_none()) && (!compact || selected.is_none());
        // Mientras se arrastra el borde de la columna, el puntero se sigue
        // con un escucha de la ventana entera: a los pocos pixeles el puntero
        // sale del asa y el borde no recibiria mas movimientos.
        if self.sidebar_dragging {
            let moved = cx.entity().downgrade();
            window.on_mouse_event(move |event: &MouseMoveEvent, _phase, _window, cx| {
                let _ = moved.update(cx, |this, cx| {
                    if !this.sidebar_dragging {
                        return;
                    }
                    let rail = crate::navrail::column_width(this.ui_scale);
                    let width = (f32::from(event.position.x) - f32::from(rail)) / this.ui_scale;
                    this.sidebar_width =
                        width.clamp(crate::app::MIN_SIDEBAR_WIDTH, crate::app::MAX_SIDEBAR_WIDTH);
                    cx.notify();
                });
            });
            let released = cx.entity().downgrade();
            window.on_mouse_event(move |_: &MouseUpEvent, _phase, _window, cx| {
                let _ = released.update(cx, |this, cx| {
                    if this.sidebar_dragging {
                        this.sidebar_dragging = false;
                        cx.notify();
                    }
                });
            });
        }
        let show_conversation = !compact || selected.is_some();
        let states_open = self.states_open;
        let channels_open = self.channels_open;
        let archived_open = self.archived_open;
        let agents_open = self.agents_open;
        // El menu de los filtros: lo abre la flecha del final de la fila de
        // los chips y elige el filtro de la columna.
        let filter_menu = cx.try_global::<ChatFilterMenu>();
        let filter_menu_open = filter_menu.is_some_and(|menu| menu.open);
        let filter_menu_position = filter_menu
            .map(|menu| menu.position)
            .unwrap_or(point(px(0.), px(0.)));
        let filter_menu_active = self.store.chat_filter();
        let filter_menu_scale = self.ui_scale;
        let filter_menu_hover = cx.theme().muted;
        let filter_menu_foreground = cx.theme().foreground;
        let filter_menu_accent = cx.theme().accent;
        let filter_menu_panel = cx.theme().popover;
        let filter_menu_border = cx.theme().muted_foreground.opacity(0.22);
        let filter_menu_viewport = window.viewport_size();
        h_flex()
            .size_full()
            // The menu de los filtros se dibuja encima de la columna, asi
            // que la raiz es la que lo posiciona.
            .relative()
            // El menu de los filtros se cierra con la tecla de escape, como
            // los otros menus de la aplicacion.
            .capture_key_down(cx.listener(|_this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape"
                    && cx
                        .try_global::<ChatFilterMenu>()
                        .is_some_and(|menu| menu.open)
                {
                    cx.default_global::<ChatFilterMenu>().open = false;
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            // The column of icons is the left edge of the window: it is the
            // navigation of the web client, the same way.
            .child(self.nav_rail.clone())
            // La barra de la llamada, arriba de todo: aparece con una
            // llamada viva y convive con lo que se este viendo.
            .when_some(self.store.call.clone(), |root, call| {
                root.child(
                    div()
                        .absolute()
                        .top_2()
                        .left_0()
                        .right_0()
                        .flex()
                        .justify_center()
                        .child(
                            v_flex()
                                .gap_2()
                                .occlude()
                                .child(render_call_bar(self, &call, cx))
                                // The video rides under the bar, in the same
                                // floating column, once the media is up: a
                                // ringing call does not open the camera yet.
                                .when(
                                    call.video
                                        && matches!(call.phase.as_str(), "connecting" | "active"),
                                    |column| column.child(render_call_video(self, cx)),
                                ),
                        ),
                )
            })
            .when(show_sidebar, |root| {
                root.child(div().relative().h_full().flex_none().child(if agents_open {
                    // The screen of the agents is its own view: the column of
                    // the harnesses takes the place of the chats.
                    self.render_agents(cx)
                } else if states_open {
                    // The states are their own view: they take the place of
                    // the column of chats.
                    self.render_states(cx)
                } else if channels_open {
                    // The channels are their own view too.
                    self.render_channels(cx)
                } else if archived_open {
                    // The archived conversations are their own view.
                    self.render_archived(cx)
                } else {
                    self.render_sidebar(cx)
                }))
            })
            .when(show_conversation, |root| {
                if agents_open {
                    // The screen of the agents is open: the permissions and
                    // the tray of drafts sit next to the list of the harnesses.
                    root.child(self.agents_panel.render_panel(self, cx))
                } else if self.selected_newsletter.is_some() {
                    // A channel is open: its posts take the place of the
                    // conversation.
                    root.child(self.newsletter_panel.render(self, cx))
                } else if states_open {
                    // The view of the states is open: the panel of the
                    // composer sits next to the list of the states.
                    root.child(self.status_strip.render_panel(self, cx))
                } else if channels_open {
                    // The view of the channels is open: the panel of the
                    // discovery sits next to the list of the channels.
                    root.child(self.render_channels_panel(cx))
                } else {
                    root.child(self.render_conversation(compact, window, cx))
                        // The panel of the members is docked next to the
                        // conversation of a group.
                        .child(self.member_panel.render(self, cx))
                }
            })
            // El asa del ancho de la columna: seis pixeles sobre su borde
            // derecho y encima de todo, porque la barra de desplazamiento de
            // la lista de los chats queda justo debajo.
            .when(show_sidebar, |root| {
                let rail = crate::navrail::column_width(self.ui_scale);
                let left = rail + px(self.sidebar_width * self.ui_scale) - px(3.);
                root.child(
                    div()
                        .id("sidebar-resize")
                        .absolute()
                        .top_0()
                        .left(left)
                        .h_full()
                        .w(px(6. * self.ui_scale))
                        .cursor(CursorStyle::ResizeLeftRight)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.sidebar_dragging = true;
                                cx.notify();
                            }),
                        ),
                )
            })
            .when(filter_menu_open, |root| {
                // Los filtros del menu: los de la fila de los chips y el de
                // los archivados, que es su propia vista.
                let filters = [
                    ChatFilter::All,
                    ChatFilter::Unread,
                    ChatFilter::Favorites,
                    ChatFilter::Groups,
                    ChatFilter::Archived,
                ];
                let width = px(210. * filter_menu_scale);
                let height = px(34. * filter_menu_scale * filters.len() as f32)
                    + px(12. * filter_menu_scale);
                // El menu no sale de la ventana.
                let left = filter_menu_position
                    .x
                    .min(filter_menu_viewport.width - width - px(8.))
                    .max(px(8.));
                let top = filter_menu_position
                    .y
                    .min(filter_menu_viewport.height - height - px(8.))
                    .max(px(8.));
                let mut menu = v_flex()
                    .w(width)
                    .rounded_md()
                    .border_1()
                    .border_color(filter_menu_border)
                    .bg(filter_menu_panel)
                    .py(px(4.));
                for (index, filter) in filters.into_iter().enumerate() {
                    let selected = filter == filter_menu_active;
                    let color = if selected {
                        filter_menu_accent
                    } else {
                        filter_menu_foreground
                    };
                    menu = menu.child(
                        div()
                            .id(("chat-filter-menu-item", index))
                            .w_full()
                            .px_3()
                            .py(px(6.))
                            .flex()
                            .gap_3()
                            .items_center()
                            .text_sm()
                            .cursor_pointer()
                            .hover(move |style| style.bg(filter_menu_hover))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.set_chat_filter(filter, window, cx);
                                // Los archivados son su propia vista: la
                                // columna los muestra en la de los archivados.
                                if filter == ChatFilter::Archived {
                                    this.archived_open = true;
                                    this.states_open = false;
                                    this.channels_open = false;
                                }
                                cx.default_global::<ChatFilterMenu>().open = false;
                                // El clic no sigue hacia la fila de la lista,
                                // que esta debajo del menu: al abrir una
                                // conversacion el filtro vuelve a ser el de
                                // todos.
                                cx.stop_propagation();
                                cx.notify();
                            }))
                            // El filtro que esta puesto lleva el visto.
                            .child(if selected {
                                div().flex_none().w(px(16. * filter_menu_scale)).child(
                                    Icon::new(IconName::Check)
                                        .with_size(px(14. * filter_menu_scale))
                                        .text_color(filter_menu_accent),
                                )
                            } else {
                                div().flex_none().w(px(16. * filter_menu_scale))
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .truncate()
                                    .text_color(color)
                                    .child(i18n::t(filter.label())),
                            ),
                    );
                }
                root.child(
                    // El fondo que cierra el menu con cualquier clic fuera de
                    // el, el mismo patron que el menu de la fila.
                    div()
                        .id("chat-filter-menu-backdrop")
                        .absolute()
                        .inset_0()
                        .on_click(cx.listener(|_this, _, _, cx| {
                            cx.default_global::<ChatFilterMenu>().open = false;
                            // El clic no sigue hacia la fila de la lista:
                            // abrir el menu no abre una conversacion.
                            cx.stop_propagation();
                            cx.notify();
                        }))
                        .on_aux_click(cx.listener(|_this, _, _, cx| {
                            cx.default_global::<ChatFilterMenu>().open = false;
                            cx.stop_propagation();
                            cx.notify();
                        }))
                        .child(menu.absolute().left(left).top(top)),
                )
            })
    }

    /// The row of filters of the column of chats.
    ///
    /// port: the `.chat-filters` row of the origin
    /// (`tauri/src/styles.css`): the chips scroll sideways, the active one
    /// wears the accent of the theme.
    ///
    /// The reference draws a round button with an arrow at the end of the
    /// row: it opens the menu of the filters that do not fit in the row.
    fn render_chat_filters(&mut self, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let active = self.store.chat_filter();
        let accent = cx.theme().accent;
        let border = cx.theme().border;
        let hover = cx.theme().muted;
        let foreground = cx.theme().muted_foreground;
        let ui_scale = self.ui_scale;
        h_flex()
            .id("chat-filters")
            .flex_none()
            .gap(px(6.))
            .pt(px(2.))
            .pb(px(8.))
            .overflow_x_scroll()
            .children(ChatFilter::ALL.into_iter().map(|filter| {
                let selected = filter == active;
                div()
                    .id(ElementId::Name(
                        format!("chat-filter-{}", filter.label()).into(),
                    ))
                    .flex_none()
                    .px(px(11.))
                    .py(px(5.))
                    .rounded_full()
                    .border_1()
                    .border_color(if selected { accent } else { border })
                    .text_xs()
                    .cursor_pointer()
                    // El chip activo de la referencia lleva el verde del
                    // tema de fondo y el texto del mismo verde.
                    .when(selected, |chip| {
                        chip.bg(cx.theme().selection).text_color(accent)
                    })
                    .when(!selected, |chip| chip.text_color(cx.theme().foreground))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_chat_filter(filter, window, cx);
                        // Eligir un chip cierra el menu de los filtros.
                        cx.default_global::<ChatFilterMenu>().open = false;
                        cx.notify();
                    }))
                    .child(i18n::t(filter.label()))
            }))
            // El boton de la flecha de la referencia: esta al final de la
            // fila y abre el menu de los filtros de la columna.
            .child(
                div()
                    .id("chat-filter-menu-button")
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(30. * ui_scale))
                    .rounded_full()
                    .border_1()
                    .border_color(border)
                    .text_color(foreground)
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .on_click(cx.listener(|_this, event: &ClickEvent, _, cx| {
                        let menu = cx.default_global::<ChatFilterMenu>();
                        menu.open = !menu.open;
                        // El menu se abre donde esta el puntero, como el
                        // menu de la fila de la conversacion.
                        menu.position = event.mouse_position().unwrap_or(point(px(0.), px(0.)));
                        cx.notify();
                    }))
                    .child(Icon::new(IconName::ChevronDown).with_size(px(14. * ui_scale))),
            )
    }

    /// The view of the agents, the column that replaces the chats.
    ///
    /// The column lists the harnesses of this machine; the pane of the
    /// permissions and the tray of drafts are drawn by the panel of the right.
    fn render_agents(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        self.agents_panel.render_list(self, cx)
    }

    /// The view of the states, the column that replaces the chats.
    fn render_states(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let scale = self.ui_scale;
        let foreground = cx.theme().foreground;
        let hover = cx.theme().muted;
        v_flex()
            .h_full()
            .w(px(self.sidebar_width * scale))
            .flex_none()
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                v_flex()
                    .flex_none()
                    .px(px(20. * scale))
                    .py_2()
                    .gap_1()
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(24. * scale))
                                    .font_weight(FontWeight::BOLD)
                                    .child(i18n::t("States")),
                            )
                            .child(
                                // El boton del estado nuevo: en la referencia es
                                // un circulo con el borde, no el relleno verde.
                                div()
                                    .id("new-status")
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(px(22. * scale))
                                    .rounded_full()
                                    .border(px(1.5 * scale))
                                    .border_color(foreground)
                                    .text_color(foreground)
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(hover))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.status_strip.publish_photo_status(window, cx);
                                    }))
                                    .child(Icon::new(IconName::Plus).with_size(px(12. * scale))),
                            ),
                    ),
            )
            .child(self.status_strip.render_view(self, cx))
    }

    /// The view of the channels, the column that replaces the chats.
    fn render_channels(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let scale = self.ui_scale;
        // El borde del boton del canal nuevo: en la referencia es un anillo
        // gris, no el relleno verde del boton de la columna de los chats.
        let ring = cx.theme().muted_foreground.opacity(0.55);
        let hover = cx.theme().muted;
        v_flex()
            .h_full()
            .w(px(self.sidebar_width * scale))
            .flex_none()
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                v_flex()
                    .flex_none()
                    .px_3()
                    .py_2()
                    .gap_1()
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(24. * scale))
                                    .font_weight(FontWeight::BOLD)
                                    .child(i18n::t("Channels")),
                            )
                            .child(
                                // El boton de crear un canal: la referencia lo
                                // dibuja con el borde y no con el relleno
                                // verde del boton de la columna de los chats.
                                div()
                                    .id("new-channel")
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(px(22. * scale))
                                    .rounded_full()
                                    .border_2()
                                    .border_color(ring)
                                    .text_color(cx.theme().foreground)
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(hover))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.newsletter_panel.open_create();
                                        cx.notify();
                                    }))
                                    .child(Icon::new(IconName::Plus).with_size(px(12. * scale))),
                            ),
                    )
                    // El buscador va con el titulo: en la referencia los dos
                    // arrancan en la misma columna.
                    .child(
                        div().pt_1().child(
                            Input::new(self.newsletter_panel.search_input())
                                .w_full()
                                .h(px(38. * scale))
                                .rounded_full()
                                .bg(cx.theme().input)
                                .border_1()
                                .border_color(cx.theme().border)
                                .prefix(
                                    Icon::new(IconName::Search)
                                        .with_size(px(17. * scale))
                                        .text_color(cx.theme().muted_foreground),
                                ),
                        ),
                    ),
            )
            .child(self.newsletter_panel.render_list(self, cx))
            .child(self.newsletter_panel.render_list_footer(cx))
    }

    /// The panel that sits next to the list of channels.
    ///
    /// The reference draws the bubble of the brand at the top, the big title
    /// of the discovery and the phrase that explains what a channel is.
    fn render_channels_panel(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let scale = self.ui_scale;
        let muted = cx.theme().muted_foreground;
        v_flex()
            .flex_1()
            .min_h_0()
            .items_center()
            .justify_center()
            .p_4()
            // El dibujo de la burbuja va arriba del todo.
            .child(
                div()
                    .mb(px(26. * scale))
                    .child(self.channels_bubble(64. * scale, cx)),
            )
            .child(
                div()
                    .mb(px(10. * scale))
                    .text_size(px(30. * scale))
                    .text_color(cx.theme().foreground)
                    .text_center()
                    .child(i18n::t("Discover channels")),
            )
            .child(
                v_flex()
                    .items_center()
                    .gap(px(2. * scale))
                    .child(
                        div()
                            .text_size(px(15. * scale))
                            .text_color(muted)
                            .text_center()
                            .child(i18n::t(
                                "Entertainment, news, sports, lifestyle, people and much more.",
                            )),
                    )
                    .child(
                        div()
                            .text_size(px(15. * scale))
                            .text_color(muted)
                            .text_center()
                            .child(i18n::t("Follow the channels you like.")),
                    ),
            )
    }

    /// The bubble of the discovery of the channels.
    ///
    /// The reference draws a big bubble with the eyes of the brand: a circle
    /// with a point at the bottom left, two arcs and a dot inside. The
    /// drawing is a canvas: the body is one path, so the gray of the bubble
    /// does not pile up where the shapes meet, and the eyes are holes of the
    /// color of the panel.
    fn channels_bubble(&self, diameter: f32, cx: &mut Context<Self>) -> gpui::Div {
        // El cuerpo de la burbuja es un gris apagado y los ojos son el fondo
        // del panel: el dibujo es un agujero, no una mancha.
        let body_color = cx.theme().muted_foreground.opacity(0.33);
        let hole = cx.theme().background;
        div().size(px(diameter)).child(
            canvas(
                move |bounds, _, _| bounds,
                move |bounds, _, window, _| {
                    let side = bounds.size.width.to_f64() as f32;
                    let origin_x = bounds.origin.x.to_f64() as f32;
                    let origin_y = bounds.origin.y.to_f64() as f32;
                    let radius = side * 0.42;
                    let center_x = origin_x + side * 0.56;
                    let center_y = origin_y + side * 0.44;

                    // El cuerpo: el circulo de la burbuja y, pegada a el,
                    // la punta de abajo a la izquierda. Es un solo camino
                    // para que el gris no se junte dos veces.
                    let mut body_path = gpui::PathBuilder::default();
                    body_path.move_to(point(
                        px(center_x - radius * 0.94),
                        px(center_y + radius * 0.34),
                    ));
                    // El arco largo del circulo: desde la punta de la
                    // izquierda, por arriba, por la derecha y por abajo,
                    // hasta el notch de la punta.
                    body_path.arc_to(
                        point(px(radius), px(radius)),
                        px(0.),
                        true,
                        true,
                        point(px(center_x - radius * 0.34), px(center_y + radius * 0.94)),
                    );
                    // La punta: la arista de adentro y la de afuera.
                    body_path.cubic_bezier_to(
                        point(px(center_x - radius * 0.92), px(center_y + radius * 1.03)),
                        point(px(center_x - radius * 0.62), px(center_y + radius * 1.02)),
                        point(px(center_x - radius * 0.74), px(center_y + radius * 1.02)),
                    );
                    body_path.cubic_bezier_to(
                        point(px(center_x - radius * 0.94), px(center_y + radius * 0.34)),
                        point(px(center_x - radius * 1.02), px(center_y + radius * 0.85)),
                        point(px(center_x - radius * 1.00), px(center_y + radius * 0.55)),
                    );
                    body_path.close();
                    if let Ok(path) = body_path.build() {
                        window.paint_path(path, body_color);
                    }

                    // Los dos ojos: un arco a cada lado del punto.
                    for side in [-1., 1.] {
                        let mut eye = gpui::PathBuilder::stroke(px(radius * 0.17));
                        eye.move_to(point(
                            px(center_x + side * radius * 0.53),
                            px(center_y - radius * 0.36),
                        ));
                        eye.cubic_bezier_to(
                            point(
                                px(center_x + side * radius * 0.53),
                                px(center_y + radius * 0.36),
                            ),
                            point(
                                px(center_x + side * radius * 0.70),
                                px(center_y - radius * 0.13),
                            ),
                            point(
                                px(center_x + side * radius * 0.70),
                                px(center_y + radius * 0.13),
                            ),
                        );
                        if let Ok(path) = eye.build() {
                            window.paint_path(path, hole);
                        }
                    }

                    // El punto del medio.
                    window.paint_quad(gpui::PaintQuad {
                        bounds: gpui::Bounds {
                            origin: point(
                                px(center_x - radius * 0.15),
                                px(center_y - radius * 0.15),
                            ),
                            size: size(px(radius * 0.30), px(radius * 0.30)),
                        },
                        corner_radii: gpui::Corners::all(px(radius * 0.15)),
                        background: hole.into(),
                        border_widths: gpui::Edges::default(),
                        border_color: gpui::transparent_black(),
                        border_style: gpui::BorderStyle::default(),
                    });
                },
            )
            .size_full(),
        )
    }

    /// The view of the archived conversations, the column that replaces the
    /// chats.
    ///
    /// The reference explains at the top that the archived conversations stay
    /// archived when they receive a message.
    fn render_archived(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let scale = self.ui_scale;
        let muted = cx.theme().muted_foreground;
        let visible_chat_ids = self.store.chat_ids(ChatFilter::Archived);
        let visible_for_list = visible_chat_ids.clone();
        let list_id = "archived-chat-list";
        v_flex()
            .h_full()
            .w(px(self.sidebar_width * scale))
            .flex_none()
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                v_flex()
                    .flex_none()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .child(
                        div()
                            .text_size(px(24. * scale))
                            .font_weight(FontWeight::BOLD)
                            .child(i18n::t("Archived")),
                    )
                    .child(div().text_sm().text_color(muted).child(i18n::t(
                        "When they receive new messages, these chats stay archived.",
                    ))),
            )
            .child(
                div().relative().flex_1().min_h_0().child(
                    uniform_list(
                        list_id,
                        visible_for_list.len(),
                        cx.processor(move |this, range: Range<usize>, _, cx| {
                            this.render_chat_row(&visible_for_list, range, cx)
                        }),
                    )
                    .h_full()
                    .track_scroll(&self.chat_scroll),
                ),
            )
    }

    /// One row of the list of conversations.
    ///
    /// The same row is drawn by the column of chats and by the view of the
    /// archived conversations.
    fn render_chat_row(
        &mut self,
        visible: &[String],
        range: Range<usize>,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::Stateful<gpui::Div>> {
        let ui_scale = self.ui_scale;
        let dark = cx.theme().is_dark();
        // La fila elegida y la que tiene el cursor por encima usan los
        // colores del tema: asi respetan la paleta de la referencia.
        let selected_background = cx.theme().secondary;
        let hover_background = cx.theme().muted;
        // La altura de las dos lineas de la fila: la del nombre y la del
        // preview. La columna de la derecha (la hora y los dibujitos) las
        // usa como altura de sus dos lineas, para que quede alineada con el
        // texto.
        let font_size = cx.theme().font_size;
        let title_line = font_size * LINE_HEIGHT;
        let preview_line = font_size * PREVIEW_FONT_RATIO * LINE_HEIGHT;
        range
            .map(|index| {
                let Some(chat) = visible
                    .get(index)
                    .and_then(|id| self.store.chat(id))
                    .cloned()
                else {
                    return div()
                        .id(("loading-chat", index))
                        .h(px(if self.density == Density::Compact {
                            64.
                        } else {
                            72.
                        }))
                        .px_4()
                        .flex()
                        .items_center()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(i18n::t("Loading more chats…"));
                };
                let selected = self.store.selected_chat_id.as_deref() == Some(chat.id.as_str());
                let selecting = self.selection.is_some();
                let checked = self
                    .selection
                    .as_ref()
                    .is_some_and(|selection| selection.ids.contains(&chat.id));
                let highlighted = checked || (selected && !selecting);
                let avatar = avatar_element_of(
                    &chat.title,
                    &chat.avatar_path,
                    px(49. * ui_scale),
                    chat.kind() == proto::ChatKind::Group,
                    cx,
                );
                let typing_preview =
                    self.typing_label(&chat.id, chat.kind() == proto::ChatKind::Group);
                let row_typing = typing_preview.is_some();
                let preview = typing_preview.unwrap_or_else(|| chat_subtitle(&chat));
                // Whether the conversation is a favorite of the profile: the
                // star of the row.
                let is_favorite = self.store.is_favorite(&chat.id);
                // The time of the last message goes at the right of the title,
                // the same way the reference draws it.
                let chat_time = chat_time_label(chat.last_message_timestamp_ms, now_ms());
                div()
                    .id(ElementId::named_usize("chat-row", index))
                    .w_full()
                    .px_3()
                    .py_2()
                    .when(self.density == Density::Compact, |row| row.py_1())
                    .flex()
                    .gap_3()
                    .items_center()
                    .rounded_md()
                    .bg(if highlighted {
                        selected_background
                    } else {
                        gpui::transparent_black()
                    })
                    .hover(move |style| style.bg(hover_background))
                    .on_click(cx.listener({
                        let chat_id = chat.id.clone();
                        move |this, _, window, cx| {
                            if this.selection.is_some() {
                                this.toggle_chat_selection(chat_id.clone(), cx);
                            } else {
                                this.open_chat(chat_id.clone(), window, cx);
                            }
                            cx.notify();
                        }
                    }))
                    // El boton derecho abre el menu de la conversacion, el
                    // mismo que el origen abre con el `contextmenu`.
                    .on_aux_click(cx.listener({
                        let chat_id = chat.id.clone();
                        move |this, event: &ClickEvent, _, cx| {
                            if event.is_secondary() {
                                let position =
                                    event.mouse_position().unwrap_or(point(px(0.), px(0.)));
                                this.open_chat_context_menu(chat_id.clone(), position, cx);
                            }
                        }
                    }))
                    .when(selecting, |row| {
                        row.child(
                            div()
                                .flex_none()
                                .size(px(22. * ui_scale))
                                .rounded_full()
                                .border_1()
                                .border_color(if checked {
                                    cx.theme().accent
                                } else {
                                    cx.theme().muted_foreground
                                })
                                .bg(if checked {
                                    cx.theme().accent
                                } else {
                                    gpui::transparent_black()
                                })
                                .flex()
                                .items_center()
                                .justify_center()
                                .when(checked, |dot| {
                                    dot.child(
                                        Icon::new(IconName::Check)
                                            .with_size(px(13. * ui_scale))
                                            .text_color(gpui::white()),
                                    )
                                }),
                        )
                    })
                    .child(avatar)
                    .child(
                        // El texto de la conversacion: el nombre en la
                        // primera linea y el preview en la segunda.
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                h_flex().w_full().items_center().child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(one_line(&chat.title)),
                                ),
                            )
                            .child(
                                h_flex().w_full().items_center().child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_sm()
                                        .text_color(if row_typing {
                                            if dark {
                                                gpui::Hsla::from(rgb(0x25d366))
                                            } else {
                                                gpui::Hsla::from(rgb(DARK_GREEN))
                                            }
                                        } else {
                                            cx.theme().muted_foreground
                                        })
                                        .child(one_line(&preview)),
                                ),
                            ),
                    )
                    .child(
                        // La columna de la derecha: la hora arriba, alineada
                        // con el nombre, y los dibujitos de la conversacion
                        // abajo, como los dibuja la referencia.
                        v_flex()
                            .flex_none()
                            .items_end()
                            .gap_1()
                            .child(
                                h_flex()
                                    .flex_none()
                                    .h(title_line)
                                    .items_center()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(chat_time),
                            )
                            .child(
                                h_flex()
                                    .flex_none()
                                    .h(preview_line)
                                    .items_center()
                                    .gap_1()
                                    // port: the flags of the row of the origin
                                    // (`tauri/src/components/Sidebar.tsx`, the
                                    // `chat-flags` block): the silence, the
                                    // favorite and the pin of the conversation.
                                    .when(chat.muted, |line| {
                                        line.child(
                                            Icon::new(WhatfastIcon::Bell)
                                                .with_size(px(14. * ui_scale))
                                                .text_color(cx.theme().muted_foreground),
                                        )
                                    })
                                    .when(is_favorite, |line| {
                                        line.child(
                                            Icon::new(IconName::StarFill)
                                                .with_size(px(13. * ui_scale))
                                                .text_color(cx.theme().accent),
                                        )
                                    })
                                    .when(chat.pinned, |line| {
                                        line.child(
                                            Icon::new(WhatfastIcon::Pin)
                                                .with_size(px(14. * ui_scale))
                                                .text_color(cx.theme().accent),
                                        )
                                    })
                                    .when(chat.unread_count > 0, |line| {
                                        line.child(
                                            div()
                                                .flex_shrink_0()
                                                .min_w(px(18.))
                                                .h(px(18.))
                                                .px_1()
                                                .rounded_full()
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .bg(rgb(0x25d366))
                                                .text_xs()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(rgb(0x0b141a))
                                                .child(chat.unread_count.to_string()),
                                        )
                                    }),
                            ),
                    )
            })
            .collect::<Vec<_>>()
    }

    /// The menu of the options of a conversation.
    ///
    /// The reference of WhatsApp Desktop has many more rows (the calls, the
    /// temporary messages, the export, the report): the menu only draws the
    /// ones the app can do, in the same order.
    fn render_chat_menu(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let Some(menu) = self.chat_menu.as_ref() else {
            return div().id("chat-menu-empty");
        };
        let Some(chat) = self.store.chat(&menu.chat_id).cloned() else {
            return div().id("chat-menu-empty");
        };
        let ui_scale = self.ui_scale;
        let is_favorite = self.store.is_favorite(&chat.id);
        // The rows of the menu of the reference (`docs/referencia/
        // 07-paneles-del-compositor.md`): the ones the app can not do are
        // drawn too, but they cannot be pressed, the same way the menu of a
        // message draws the rows that do not apply.
        let entries: Vec<ChatMenuEntry> = vec![
            ChatMenuEntry::Item(Box::new((
                Some(ChatMenuAction::Info),
                i18n::t("Contact info").into(),
                Icon::new(IconName::Info),
            ))),
            ChatMenuEntry::Item(Box::new((
                Some(ChatMenuAction::Search),
                i18n::t("Search").into(),
                Icon::new(IconName::Search),
            ))),
            ChatMenuEntry::Item(Box::new((
                None,
                i18n::t("Select messages").into(),
                Icon::new(WhatfastIcon::Check),
            ))),
            ChatMenuEntry::Item(Box::new((
                Some(ChatMenuAction::Mute),
                if chat.muted {
                    i18n::t("Unmute notifications")
                } else {
                    i18n::t("Mute notifications")
                }
                .into(),
                Icon::new(WhatfastIcon::Bell),
            ))),
            ChatMenuEntry::Item(Box::new((
                None,
                i18n::t("Temporary messages").into(),
                Icon::new(WhatfastIcon::Clock),
            ))),
            ChatMenuEntry::Item(Box::new((
                None,
                i18n::t("Restrict chat").into(),
                Icon::new(WhatfastIcon::Shield),
            ))),
            ChatMenuEntry::Item(Box::new((
                None,
                i18n::t("Chat style").into(),
                Icon::new(WhatfastIcon::Palette),
            ))),
            ChatMenuEntry::Item(Box::new((
                Some(ChatMenuAction::Favorite),
                if is_favorite {
                    i18n::t("Remove from favorites")
                } else {
                    i18n::t("Add to favorites")
                }
                .into(),
                Icon::new(IconName::Heart),
            ))),
            ChatMenuEntry::Item(Box::new((
                None,
                i18n::t("Add to list").into(),
                Icon::new(IconName::File),
            ))),
            ChatMenuEntry::Item(Box::new((
                None,
                i18n::t("Export chat").into(),
                Icon::new(IconName::ArrowDown),
            ))),
            ChatMenuEntry::Item(Box::new((
                None,
                i18n::t("Close chat").into(),
                Icon::new(IconName::CircleX),
            ))),
            ChatMenuEntry::Separator,
            ChatMenuEntry::Item(Box::new((
                None,
                i18n::t("Send call link").into(),
                Icon::new(IconName::ExternalLink),
            ))),
            ChatMenuEntry::Item(Box::new((
                None,
                i18n::t("Schedule call").into(),
                Icon::new(IconName::Calendar),
            ))),
            ChatMenuEntry::Item(Box::new((
                None,
                i18n::t("New group call").into(),
                Icon::new(IconName::User),
            ))),
            ChatMenuEntry::Separator,
            ChatMenuEntry::Item(Box::new((
                None,
                i18n::t("Report").into(),
                Icon::new(IconName::TriangleAlert),
            ))),
            ChatMenuEntry::Item(Box::new((
                Some(ChatMenuAction::Block),
                if chat.blocked {
                    "Unblock contact"
                } else {
                    "Block contact"
                }
                .into(),
                Icon::new(WhatfastIcon::Shield),
            ))),
            ChatMenuEntry::Item(Box::new((
                None,
                i18n::t("Clear chat").into(),
                Icon::new(IconName::CircleX),
            ))),
            ChatMenuEntry::Item(Box::new((
                None,
                i18n::t("Delete chat").into(),
                Icon::new(WhatfastIcon::Trash),
            ))),
        ];

        let hover = cx.theme().muted;
        let foreground = cx.theme().foreground;
        let muted = cx.theme().muted_foreground;
        let danger = cx.theme().danger;
        let panel = cx.theme().popover;
        let border = cx.theme().muted_foreground.opacity(0.22);
        let width = px(240. * ui_scale);
        let height = px(38. * ui_scale * entries.len() as f32) + px(12. * ui_scale);
        let viewport = window.viewport_size();
        let left = menu
            .position
            .x
            .min(viewport.width - width - px(8.))
            .max(px(8.));
        let top = menu
            .position
            .y
            .min(viewport.height - height - px(8.))
            .max(px(8.));
        let chat_id = chat.id.clone();

        let mut panel = v_flex()
            .w(width)
            .rounded_md()
            .border_1()
            .border_color(border)
            .bg(panel)
            .py(px(4.));
        for (index, entry) in entries.into_iter().enumerate() {
            let (action, label, icon) = match entry {
                ChatMenuEntry::Separator => {
                    panel = panel.child(div().h(px(1.)).my(px(4.)).mx_2().bg(border));
                    continue;
                }
                ChatMenuEntry::Item(item) => *item,
            };
            let is_block = action == Some(ChatMenuAction::Block);
            let color = if action.is_none() {
                muted
            } else if is_block {
                danger
            } else {
                foreground
            };
            // Las filas que la app todavia no puede hacer se dibujan apagadas
            // y sin accion: no alcanza con que no tengan el clic, porque el
            // menu se arma con las dos clases de fila.
            let mut row = div()
                .id(("chat-menu-item", index))
                .w_full()
                .px_3()
                .py(px(8.))
                .flex()
                .gap_3()
                .items_center()
                .text_sm()
                .text_color(color)
                .child(icon.with_size(px(16. * ui_scale)).text_color(color))
                .child(div().flex_1().truncate().text_color(color).child(label));
            if let Some(action) = action {
                row = row
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .on_click(cx.listener({
                        let chat_id = chat_id.clone();
                        move |this, _, window, cx| {
                            this.run_chat_menu_action(action, chat_id.clone(), window, cx);
                        }
                    }));
            }
            panel = panel.child(row);
        }
        // El fondo que cierra el menu con cualquier clic fuera de el.
        div()
            .id("chat-menu-backdrop")
            .absolute()
            .inset_0()
            .on_click(cx.listener(|this, _, _, cx| {
                this.close_chat_menu(cx);
                cx.stop_propagation();
            }))
            .on_aux_click(cx.listener(|this, _, _, cx| {
                this.close_chat_menu(cx);
                cx.stop_propagation();
            }))
            .child(panel.absolute().left(left).top(top))
    }

    /// The menu that the right click opens on a row of the list of
    /// conversations.
    ///
    /// port: the `chat-context-menu` of the origin
    /// (`tauri/src/components/Sidebar.tsx`): the items are the ones of the
    /// origin, in the same order, and the menu opens where the pointer is.
    fn render_chat_context_menu(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let Some(menu) = self.chat_context_menu.as_ref() else {
            return div().id("chat-context-menu-empty");
        };
        let Some(chat) = self.store.chat(&menu.chat_id).cloned() else {
            return div().id("chat-context-menu-empty");
        };
        let ui_scale = self.ui_scale;
        let is_group = chat.kind() == proto::ChatKind::Group;
        let is_favorite = self.store.is_favorite(&chat.id);
        // Los items del menu, en el orden del origen.
        let items: Vec<(ChatContextAction, SharedString, Icon)> = vec![
            (
                ChatContextAction::Pin,
                if chat.pinned {
                    i18n::t("Unpin chat")
                } else {
                    i18n::t("Pin chat")
                }
                .into(),
                Icon::new(WhatfastIcon::Pin),
            ),
            (
                ChatContextAction::Mute,
                if chat.muted {
                    i18n::t("Unmute notifications")
                } else {
                    i18n::t("Mute notifications")
                }
                .into(),
                Icon::new(WhatfastIcon::Bell),
            ),
            (
                ChatContextAction::Archive,
                if chat.archived {
                    i18n::t("Unarchive chat")
                } else {
                    i18n::t("Archive chat")
                }
                .into(),
                Icon::new(WhatfastIcon::Archive),
            ),
            (
                ChatContextAction::Favorite,
                if is_favorite {
                    i18n::t("Remove from favorites")
                } else {
                    i18n::t("Add to favorites")
                }
                .into(),
                Icon::new(if is_favorite {
                    IconName::StarFill
                } else {
                    IconName::Star
                }),
            ),
            (
                if chat.unread_count > 0 {
                    ChatContextAction::MarkRead
                } else {
                    ChatContextAction::MarkUnread
                },
                if chat.unread_count > 0 {
                    i18n::t("Mark as read")
                } else {
                    i18n::t("Mark as unread")
                }
                .into(),
                Icon::new(WhatfastIcon::CheckDouble),
            ),
        ];
        // El origen pone un separador antes de bloquear, y solo deja
        // bloquear a los contactos que no son grupos.
        let mut entries: Vec<ChatContextEntry> = items
            .into_iter()
            .map(|item| ChatContextEntry::Item(Box::new(item)))
            .collect();
        if !is_group {
            entries.push(ChatContextEntry::Separator);
            entries.push(ChatContextEntry::Item(Box::new((
                ChatContextAction::Block,
                if chat.blocked {
                    "Unblock contact"
                } else {
                    "Block contact"
                }
                .into(),
                Icon::new(WhatfastIcon::Shield),
            ))));
        }

        let hover = cx.theme().muted;
        let foreground = cx.theme().foreground;
        let danger = cx.theme().danger;
        let panel = cx.theme().popover;
        // El borde del menu y el separador son una linea clara sobre el
        // panel: con el color del borde del tema no se ven.
        let border = cx.theme().muted_foreground.opacity(0.22);
        let width = px(240. * ui_scale);
        // El menu no sale de la ventana.
        let viewport = window.viewport_size();
        let height = px(34. * ui_scale * entries.len() as f32) + px(24. * ui_scale);
        let left = menu
            .position
            .x
            .min(viewport.width - width - px(8.))
            .max(px(8.));
        let top = menu
            .position
            .y
            .min(viewport.height - height - px(8.))
            .max(px(8.));
        let chat_id = chat.id.clone();

        let mut panel = v_flex()
            .w(width)
            .rounded_md()
            .border_1()
            .border_color(border)
            .bg(panel)
            .py(px(4.));
        for (index, entry) in entries.into_iter().enumerate() {
            let (action, label, icon) = match entry {
                ChatContextEntry::Separator => {
                    panel = panel.child(div().h(px(1.)).my(px(4.)).mx_2().bg(border));
                    continue;
                }
                ChatContextEntry::Item(item) => *item,
            };
            let is_block = action == ChatContextAction::Block;
            let color = if is_block { danger } else { foreground };
            panel = panel.child(
                div()
                    .id(("chat-context-item", index))
                    .w_full()
                    .px_3()
                    .py(px(6.))
                    .flex()
                    .gap_3()
                    .items_center()
                    .text_sm()
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .on_click(cx.listener({
                        let chat_id = chat_id.clone();
                        move |this, _, _, cx| {
                            this.run_chat_context_action(action, chat_id.clone(), cx);
                        }
                    }))
                    .child(icon.with_size(px(16. * ui_scale)).text_color(color))
                    .child(div().flex_1().truncate().text_color(color).child(label)),
            );
        }
        // El fondo que cierra el menu con cualquier clic fuera de el.
        div()
            .id("chat-context-backdrop")
            .absolute()
            .inset_0()
            .on_click(cx.listener(|this, _, _, cx| {
                this.close_chat_context_menu(cx);
                cx.stop_propagation();
            }))
            .on_aux_click(cx.listener(|this, _, _, cx| {
                this.close_chat_context_menu(cx);
                cx.stop_propagation();
            }))
            .child(panel.absolute().left(left).top(top))
    }

    fn render_sidebar(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let ui_scale = self.ui_scale;
        let total = self.store.total_chats;
        let chat_filter = self.store.chat_filter();
        let visible_chat_ids = self.store.chat_ids(chat_filter);
        let search_active = self.search_query.trim().chars().count() >= 2;
        let visible_count = visible_chat_ids.len();
        let has_more = !self.store.next_chat_cursor.is_empty();
        let virtual_count = visible_count + usize::from(has_more);
        let list_id = "inbox-chat-list";
        let local_status = if self.store.sync_active {
            format!(
                "{} chats · scanned {} chats · {} messages",
                total, self.store.sync_chats, self.store.sync_messages
            )
        } else {
            format!("{} chats", total)
        };
        let visible_for_list = visible_chat_ids.clone();
        v_flex()
            .h_full()
            .w(px(self.sidebar_width * ui_scale))
            .relative()
            .flex_none()
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                v_flex()
                    // The header grows with its content: the strip of the
                    // states is a part of it.
                    .px_3()
                    .py_2()
                    .gap_1()
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(24. * ui_scale))
                                    .font_weight(FontWeight::BOLD)
                                    .child(i18n::t("Chats")),
                            )
                            .child(
                                h_flex()
                                    .gap_1()
                                    // El boton que esconde la columna: la
                                    // conversacion ocupa entonces toda la
                                    // ventana.
                                    .child(
                                        Button::new("hide-sidebar")
                                            .small()
                                            .icon(WhatfastIcon::Sidebar)
                                            .tooltip(i18n::t("Hide chat list"))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.sidebar_hidden = true;
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        Button::new("settings")
                                            .small()
                                            .icon(IconName::Settings)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.settings_open = !this.settings_open;
                                                this.reaction_details = None;
                                                this.image_viewer = None;
                                                cx.notify();
                                            })),
                                    )
                                    // El menu de tres puntos de la columna:
                                    // nuevo grupo, destacados, seleccion,
                                    // marcar todo leido, bloqueo y salir.
                                    .child(
                                        Button::new("chat-list-menu")
                                            .small()
                                            .icon(IconName::EllipsisVertical)
                                            .tooltip(i18n::t("Menu"))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.chat_list_menu = !this.chat_list_menu;
                                                cx.notify();
                                            })),
                                    )
                                    // El boton verde abre un chat nuevo: le
                                    // da el foco al buscador, como en la
                                    // referencia.
                                    .child({
                                        let accent_hover = cx.theme().primary_hover;
                                        div()
                                            .id("new-chat")
                                            .flex_none()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .size(px(36. * ui_scale))
                                            .rounded_full()
                                            .bg(cx.theme().accent)
                                            .text_color(cx.theme().accent_foreground)
                                            .cursor_pointer()
                                            .hover(move |style| style.bg(accent_hover))
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.focus_search(window, cx);
                                            }))
                                            .child(
                                                Icon::new(IconName::Plus)
                                                    .with_size(px(20. * ui_scale)),
                                            )
                                    }),
                            ),
                    )
                    // La barra de la seleccion multiple, cuando esta
                    // encendida: cuantos hay y que se les puede hacer.
                    .when(self.selection.is_some(), |column| {
                        column.child(self.render_chat_selection_bar(cx))
                    })
                    .child(
                        // El buscador de la referencia lleva la lupa dentro y
                        // es una pastilla con el fondo un poco mas claro que
                        // el de la columna.
                        div().pt_1().child(
                            Input::new(&self.search_input)
                                .w_full()
                                .h(px(38. * ui_scale))
                                .rounded_full()
                                .bg(cx.theme().input)
                                .cleanable(true)
                                .prefix(
                                    Icon::new(IconName::Search)
                                        .with_size(px(17. * ui_scale))
                                        .text_color(cx.theme().muted_foreground),
                                ),
                        ),
                    )
                    // The row of filters sits under the search, the same way
                    // the web client draws it.
                    .child(self.render_chat_filters(cx)),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .when(!search_active, |list| {
                        list.child(
                            uniform_list(
                                list_id,
                                virtual_count,
                                cx.processor(move |this, range: Range<usize>, _, cx| {
                                    let loaded = visible_for_list.len();
                                    if range.end >= loaded
                                        && !this.store.next_chat_cursor.is_empty()
                                    {
                                        this.load_chats(this.store.next_chat_cursor.clone());
                                    }
                                    if this.store.connection == proto::ConnectionState::Connected
                                        && this.rpc.pending_media_count() < 4
                                        && let Some(chat_id) = range.clone().find_map(|index| {
                                            visible_for_list.get(index).and_then(|chat_id| {
                                                this.store.chat(chat_id).and_then(|chat| {
                                                    // port: the origin asks for the
                                                    // avatar of every chat it
                                                    // draws (`tauri/src/components/
                                                    // Sidebar.tsx`).
                                                    (chat.avatar_path.is_empty()
                                                        && !this
                                                            .avatar_attempted
                                                            .contains(&chat.id))
                                                    .then(|| chat.id.clone())
                                                })
                                            })
                                        })
                                    {
                                        this.load_avatar(chat_id);
                                    }
                                    this.render_chat_row(&visible_for_list, range, cx)
                                }),
                            )
                            .h_full()
                            .track_scroll(&self.chat_scroll)
                            .on_scroll_wheel(cx.listener(
                                |this, event, window, cx| {
                                    this.handle_smooth_scroll_input(
                                        ScrollSurface::ChatList,
                                        event,
                                        window,
                                        cx,
                                    );
                                },
                            )),
                        )
                    })
                    .when(!search_active && visible_count == 0 && !has_more, |list| {
                        list.child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .px_4()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(empty_label(chat_filter)),
                        )
                    })
                    .when(search_active, |list| {
                        list.child(self.render_search_results(cx))
                    }),
            )
            // El pie de la columna lleva el estado de la sincronizacion, como
            // la franja del cliente web y como la referencia (que no la
            // muestra arriba del todo). Mientras el buscador esta abierto no
            // se pinta: la referencia solo muestra los resultados.
            .when(!search_active, |column| {
                column.child(
                    v_flex()
                        .flex_none()
                        .border_t_1()
                        .border_color(cx.theme().border)
                        .child(
                            div()
                                .px_3()
                                .pt_2()
                                .truncate()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(local_status),
                        )
                        .child(
                            div()
                                .w_full()
                                .h(px(3.))
                                .rounded_full()
                                .overflow_hidden()
                                .bg(cx.theme().secondary)
                                .when(self.store.sync_active, |track| {
                                    track.flex().justify_center().child(
                                        div()
                                            .h_full()
                                            .w(px(72.))
                                            .rounded_full()
                                            .bg(cx.theme().accent),
                                    )
                                })
                                .when(self.store.sync_complete, |track| {
                                    track.child(
                                        div()
                                            .h_full()
                                            .w_full()
                                            .rounded_full()
                                            .bg(cx.theme().accent),
                                    )
                                }),
                        ),
                )
            })
            // El menu flota sobre la columna, anclado arriba a la derecha.
            .when(self.chat_list_menu, |column| {
                column.child(self.render_chat_list_menu(cx))
            })
    }

    /// La barra de la seleccion multiple, debajo del titulo de la columna:
    /// cuantos chats hay marcados y las acciones en lote.
    fn render_chat_selection_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self
            .selection
            .as_ref()
            .map_or(0, |selection| selection.ids.len());
        let enabled = count > 0;
        h_flex()
            .items_center()
            .gap_1()
            .py_1()
            .child(
                Button::new("cancel-chat-selection")
                    .small()
                    .icon(WhatfastIcon::Close)
                    .tooltip(i18n::t("Cancel"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.cancel_chat_selection(cx);
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(i18n::t_args(
                        "{count} selected",
                        &[("count", &count.to_string())],
                    )),
            )
            .child(
                Button::new("selection-mark-read")
                    .small()
                    .icon(IconName::Check)
                    .tooltip(i18n::t("Mark as read"))
                    .disabled(!enabled)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.selection_mark_read(cx);
                    })),
            )
            .child(
                Button::new("selection-mute")
                    .small()
                    .icon(WhatfastIcon::Bell)
                    .tooltip(i18n::t("Mute"))
                    .disabled(!enabled)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.selection_mute(cx);
                    })),
            )
            .child(
                Button::new("selection-archive")
                    .small()
                    .icon(WhatfastIcon::Archive)
                    .tooltip(i18n::t("Archive"))
                    .disabled(!enabled)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.selection_archive(cx);
                    })),
            )
            .child(
                Button::new("selection-pin")
                    .small()
                    .icon(WhatfastIcon::Pin)
                    .tooltip(i18n::t("Pin"))
                    .disabled(!enabled)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.selection_pin(cx);
                    })),
            )
    }

    /// El menu de tres puntos de la columna de los chats, con las acciones
    /// que muestra WhatsApp: nuevo grupo, destacados, seleccionar, marcar
    /// todo leido, bloqueo y cerrar sesion.
    fn render_chat_list_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let scale = self.ui_scale;
        let foreground = cx.theme().foreground;
        let panel = cx.theme().popover;
        let border = cx.theme().muted_foreground.opacity(0.22);
        let hover = cx.theme().muted;
        let danger = cx.theme().danger;
        let width = px(248. * scale);
        let height = px(6. * 38. * scale + 16. * scale);
        // Las acciones que todavia no tienen backend o pantalla: se ven
        // apagadas y el clic dice por que.
        let pending = i18n::t("Not available yet");
        let row = |id: (&'static str, usize),
                   icon: Icon,
                   label: &'static str,
                   color: gpui::Hsla,
                   enabled: bool| {
            div()
                .id(id)
                .w_full()
                .h(px(38. * scale))
                .px(px(14. * scale))
                .gap(px(12. * scale))
                .flex()
                .items_center()
                .text_sm()
                .cursor_pointer()
                .when(!enabled, |row| row.opacity(0.45))
                .hover(move |style| style.bg(hover))
                .child(icon.with_size(px(17. * scale)).text_color(color))
                .child(div().flex_1().truncate().text_color(color).child(label))
        };
        let panel = v_flex()
            .id("chat-list-menu-panel")
            .absolute()
            .top(px(54. * scale))
            .right(px(10. * scale))
            .w(width)
            .max_h(height)
            .py(px(8. * scale))
            .rounded_lg()
            .border_1()
            .border_color(border)
            .bg(panel)
            .shadow_lg()
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                row(
                    ("chat-list-new-group", 0),
                    Icon::new(IconName::User),
                    i18n::t("New group"),
                    foreground,
                    true,
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.open_create_group(window, cx);
                })),
            )
            .child(
                row(
                    ("chat-list-starred", 0),
                    Icon::new(IconName::StarFill),
                    i18n::t("Starred messages"),
                    foreground,
                    false,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.store.toast_info = Some(pending.into());
                    this.clear_toast_info_later(cx);
                    this.chat_list_menu = false;
                    cx.notify();
                })),
            )
            .child(
                row(
                    ("chat-list-select", 0),
                    Icon::new(IconName::CircleCheck),
                    i18n::t("Select chats"),
                    foreground,
                    true,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.start_chat_selection(cx);
                })),
            )
            .child(
                row(
                    ("chat-list-read", 0),
                    Icon::new(IconName::Check),
                    i18n::t("Mark all as read"),
                    foreground,
                    true,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.mark_all_chats_read(cx);
                })),
            )
            .child(
                row(
                    ("chat-list-lock", 0),
                    Icon::new(WhatfastIcon::Shield),
                    i18n::t("App lock"),
                    foreground,
                    true,
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.open_pin_dialog(window, cx);
                })),
            )
            .child(div().h(px(1.)).my(px(6.)).mx(px(12.)).bg(border))
            .child(
                row(
                    ("chat-list-logout", 0),
                    Icon::new(IconName::CircleX),
                    i18n::t("Log out"),
                    danger,
                    true,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.ask_logout(cx);
                })),
            );
        div()
            .id("chat-list-menu-backdrop")
            .absolute()
            .inset_0()
            .on_click(cx.listener(|this, _, _, cx| {
                this.chat_list_menu = false;
                cx.stop_propagation();
                cx.notify();
            }))
            .on_aux_click(cx.listener(|this, _, _, cx| {
                this.chat_list_menu = false;
                cx.stop_propagation();
                cx.notify();
            }))
            .child(panel)
    }

    /// La confirmacion de "Cerrar sesion", cuando sale del menu de la columna
    /// de los chats: la sesion del dispositivo se desvincula y los mensajes
    /// que ya estan en disco no se tocan.
    fn render_logout_confirm(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let scale = self.ui_scale;
        let muted = cx.theme().muted_foreground;
        let panel = cx.theme().popover;
        div()
            .id("logout-confirm-backdrop")
            .absolute()
            .inset_0()
            .bg(rgba(0x000000a6))
            .flex()
            .items_center()
            .justify_center()
            .on_click(cx.listener(|this, _, _, cx| {
                this.confirm_logout_open = false;
                cx.stop_propagation();
                cx.notify();
            }))
            .child(
                v_flex()
                    .id("logout-confirm")
                    .w(px(380. * scale))
                    .rounded_lg()
                    .bg(panel)
                    .p(px(22. * scale))
                    .gap_3()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(i18n::t("Log out of WhatsApp?")),
                    )
                    .child(div().text_sm().text_color(muted).child(i18n::t(
                        "The messages of this device stay here. To use Whatfast again you have to link the device with the QR code.",
                    )))
                    .child(
                        h_flex().justify_end().gap_2().child(
                            Button::new("logout-cancel")
                                .label(i18n::t("Cancel"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.confirm_logout_open = false;
                                    cx.notify();
                                })),
                        ).child(
                            Button::new("logout-confirm-button")
                                .primary()
                                .label(i18n::t("Log out"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.confirm_logout_open = false;
                                    this.request_logout();
                                    cx.notify();
                                })),
                        ),
                    ),
            )
            .into_any_element()
    }

    /// La pantalla que cubre la ventana mientras el PIN no se escribe: el
    /// fondo opaco no deja ver ni un mensaje detras.
    fn render_app_lock(&self, _window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let scale = self.ui_scale;
        let muted = cx.theme().muted_foreground;
        let error = self.lock_error.clone();
        div()
            .id("app-lock")
            .absolute()
            .inset_0()
            .bg(cx.theme().background)
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_4()
            .on_click(|_, _, cx| cx.stop_propagation())
            .on_aux_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .size(px(64. * scale))
                    .rounded_full()
                    .bg(cx.theme().muted)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        Icon::new(WhatfastIcon::Shield)
                            .with_size(px(30. * scale))
                            .text_color(cx.theme().foreground),
                    ),
            )
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(i18n::t("App lock")),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(muted)
                    .child(i18n::t("Enter your PIN to open WhatsApp")),
            )
            .child(h_flex().h(px(40. * scale)).gap_3().items_center().children(
                (0..crate::app_lock::MIN_PIN_LENGTH.max(self.pin_buffer.len())).map(|index| {
                    let filled = index < self.pin_buffer.len();
                    div()
                        .size(px(12. * scale))
                        .rounded_full()
                        .border_1()
                        .border_color(muted)
                        .when(filled, |dot| dot.bg(cx.theme().foreground))
                }),
            ))
            .when_some(error, |column, error| {
                column.child(div().text_sm().text_color(cx.theme().danger).child(error))
            })
            .child(
                Button::new("app-lock-unlock")
                    .primary()
                    .label(i18n::t("Unlock"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.try_unlock(cx);
                    })),
            )
            .child(div().text_xs().text_color(muted).child(i18n::t(
                "The PIN is local to this device. It is not the PIN of your WhatsApp account.",
            )))
            .into_any_element()
    }

    /// El dialogo que activa el PIN o lo quita, segun el perfil tenga uno.
    fn render_pin_dialog(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let Some(dialog) = self.pin_dialog.as_ref() else {
            return div().into_any_element();
        };
        let scale = self.ui_scale;
        let muted = cx.theme().muted_foreground;
        let panel = cx.theme().popover;
        let enabling = dialog.mode == PinDialogMode::Enable;
        let error = dialog.error.clone();
        div()
            .id("pin-dialog-backdrop")
            .absolute()
            .inset_0()
            .bg(rgba(0x000000a6))
            .flex()
            .items_center()
            .justify_center()
            .on_click(cx.listener(|this, _, _, cx| {
                this.close_pin_dialog(cx);
            }))
            .child(
                v_flex()
                    .id("pin-dialog")
                    .w(px(380. * scale))
                    .rounded_lg()
                    .bg(panel)
                    .p(px(22. * scale))
                    .gap_3()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(if enabling {
                                i18n::t("Turn on the app lock")
                            } else {
                                i18n::t("Turn off the app lock")
                            }),
                    )
                    .child(div().text_sm().text_color(muted).child(if enabling {
                        i18n::t("Ask for a PIN of 4 to 8 digits when the app starts.")
                    } else {
                        i18n::t("Write the PIN you already have to remove the lock.")
                    }))
                    .child(Input::new(&dialog.pin).w_full())
                    .when(enabling, |column| {
                        column.child(Input::new(&dialog.repeat).w_full())
                    })
                    .when_some(error, |column, error| {
                        column.child(div().text_sm().text_color(cx.theme().danger).child(error))
                    })
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("pin-dialog-cancel")
                                    .label(i18n::t("Cancel"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.close_pin_dialog(cx);
                                    })),
                            )
                            .child(
                                Button::new("pin-dialog-submit")
                                    .primary()
                                    .label(if enabling {
                                        i18n::t("Turn on")
                                    } else {
                                        i18n::t("Turn off")
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.submit_pin_dialog(cx);
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }

    /// La tarjeta que explica los mensajes de visualizacion unica: el aviso
    /// de WhatsApp en el telefono, traido al dispositivo vinculado.
    fn render_view_once_info(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let scale = self.ui_scale;
        let muted = cx.theme().muted_foreground;
        let panel = cx.theme().popover;
        div()
            .id("view-once-info-backdrop")
            .absolute()
            .inset_0()
            .bg(rgba(0x000000a6))
            .flex()
            .items_center()
            .justify_center()
            .on_click(cx.listener(|this, _, _, cx| {
                this.view_once_info_open = false;
                cx.stop_propagation();
                cx.notify();
            }))
            .child(
                v_flex()
                    .id("view-once-info")
                    .w(px(400. * scale))
                    .rounded_lg()
                    .bg(panel)
                    .p(px(22. * scale))
                    .gap_3()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        h_flex()
                            .gap_3()
                            .items_center()
                            .child(
                                div()
                                    .flex_none()
                                    .size(px(34. * scale))
                                    .rounded_full()
                                    .border_1()
                                    .border_dashed()
                                    .border_color(muted)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_sm()
                                    .text_color(muted)
                                    .child("1"),
                            )
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(i18n::t("View once messages")),
                            ),
                    )
                    .child(div().text_sm().text_color(muted).child(i18n::t(
                        "For added privacy, view once photos and videos can only be opened on your phone. As a linked device, Whatfast receives only this notice.",
                    )))
                    .child(
                        h_flex().justify_end().child(
                            Button::new("view-once-info-close")
                                .primary()
                                .label(i18n::t("Got it"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.view_once_info_open = false;
                                    cx.notify();
                                })),
                        ),
                    ),
            )
            .into_any_element()
    }

    /// El dialogo que crea un grupo: el nombre y la lista de contactos con
    /// los que se marca quien entra.
    fn render_create_group(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(state) = self.create_group.as_ref() else {
            return div().into_any_element();
        };
        let scale = self.ui_scale;
        let muted = cx.theme().muted_foreground;
        let panel = cx.theme().popover;
        let accent = cx.theme().accent;
        let background = cx.theme().background;
        let hover = cx.theme().muted;
        let name = state.name.read(cx).value().trim().to_string();
        let pending = state.pending;
        let selected_count = state.selected.len();
        let can_create = !pending
            && !name.is_empty()
            && selected_count > 0
            && self.store.connection == proto::ConnectionState::Connected;
        // Los contactos de la lista: los chats directos con su jid de
        // transporte, que es el que entiende el backend.
        let mut candidates: Vec<(String, String, String, bool)> = Vec::new();
        for chat in &self.store.chats {
            if chat.kind() != proto::ChatKind::Direct || chat.transport_jid.is_empty() {
                continue;
            }
            let jid = chat.transport_jid.clone();
            candidates.push((
                jid.clone(),
                chat.title.clone(),
                chat.avatar_path.clone(),
                state.selected.contains(&jid),
            ));
            if candidates.len() >= 200 {
                break;
            }
        }
        let has_candidates = !candidates.is_empty();
        div()
            .id("create-group-backdrop")
            .absolute()
            .inset_0()
            .bg(rgba(0x000000a6))
            .flex()
            .items_center()
            .justify_center()
            .on_click(cx.listener(|this, _, _, cx| {
                this.close_create_group(cx);
                cx.stop_propagation();
            }))
            .child(
                v_flex()
                    .id("create-group-dialog")
                    .w(px(440. * scale))
                    .max_h(px(560. * scale))
                    .rounded_lg()
                    .bg(panel)
                    .p(px(20. * scale))
                    .gap_3()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(i18n::t("New group")),
                    )
                    .child(
                        Input::new(&state.name)
                            .w_full()
                            .h(px(40. * scale))
                            .disabled(pending),
                    )
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(i18n::t("Participants")),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(format!("{selected_count}")),
                            ),
                    )
                    .child(
                        v_flex()
                            .id("create-group-contacts")
                            .flex_1()
                            .min_h_0()
                            .max_h(px(300. * scale))
                            .overflow_y_scroll()
                            .gap(px(2. * scale))
                            .when(!has_candidates, |list| {
                                list.child(
                                    div()
                                        .py_4()
                                        .text_sm()
                                        .text_color(muted)
                                        .child(i18n::t("There is no direct chat to add yet")),
                                )
                            })
                            .children(candidates.into_iter().map(
                                |(jid, title, avatar, selected)| {
                                    let click_jid = jid.clone();
                                    h_flex()
                                        .id(format!("group-contact-{jid}"))
                                        .w_full()
                                        .h(px(44. * scale))
                                        .px(px(6. * scale))
                                        .gap_3()
                                        .items_center()
                                        .rounded_md()
                                        .cursor_pointer()
                                        .hover(move |style| style.bg(hover))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.toggle_group_participant(click_jid.clone(), cx);
                                        }))
                                        .child(avatar_element(&title, &avatar, px(30. * scale), cx))
                                        .child(div().flex_1().truncate().text_sm().child(title))
                                        .child(
                                            div()
                                                .flex_none()
                                                .size(px(20. * scale))
                                                .rounded_full()
                                                .border_1()
                                                .border_color(if selected { accent } else { muted })
                                                .bg(if selected { accent } else { background })
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .when(selected, |dot| {
                                                    dot.child(
                                                        Icon::new(IconName::Check)
                                                            .with_size(px(12. * scale))
                                                            .text_color(gpui::white()),
                                                    )
                                                }),
                                        )
                                },
                            )),
                    )
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("create-group-cancel")
                                    .label(i18n::t("Cancel"))
                                    .disabled(pending)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.close_create_group(cx);
                                    })),
                            )
                            .child(
                                Button::new("create-group-submit")
                                    .primary()
                                    .label(if pending {
                                        i18n::t("Creating the group…")
                                    } else {
                                        i18n::t("Create group")
                                    })
                                    .disabled(!can_create)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.submit_create_group(cx);
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_search_results(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let pending = self.rpc.pending_requests().any(|pending| {
            matches!(pending, PendingRequest::Search { generation, .. } if *generation == self.search_generation)
        });
        let total = self.search_results.len();
        let query_len = self.search_query.trim().chars().count();
        let mut content = v_flex().absolute().inset_0().bg(cx.theme().background);
        if pending && total == 0 {
            return content.child(
                div()
                    .p_4()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(i18n::t("Searching local data…")),
            );
        }
        if let Some(error) = self.search_error.clone() {
            return content.child(div().p_4().text_sm().text_color(rgb(0xb91c1c)).child(error));
        }
        if total == 0 {
            return content.child(
                div()
                    .p_4()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(if query_len < 3 {
                        i18n::t("No contacts or chats found")
                    } else {
                        i18n::t("No local results")
                    }),
            );
        }

        let rows = Rc::new(self.search_results.rows());
        let row_sizes = Rc::new(
            rows.iter()
                .map(|row| {
                    let height = match row {
                        SearchResultRow::Header { .. } => SEARCH_HEADER_HEIGHT,
                        SearchResultRow::Contact { .. } => SEARCH_CONTACT_HEIGHT,
                        SearchResultRow::Group { .. } | SearchResultRow::Message { .. } => {
                            SEARCH_RESULT_HEIGHT
                        }
                    };
                    size(px(1.), px(height * self.ui_scale))
                })
                .collect::<Vec<_>>(),
        );
        let rows_for_list = rows.clone();
        content = content.child(
            v_flex()
                .relative()
                .flex_1()
                .min_h_0()
                .on_scroll_wheel(cx.listener(|this, event, window, cx| {
                    this.handle_smooth_scroll_input(ScrollSurface::Search, event, window, cx);
                }))
                .child(
                    v_virtual_list(
                        cx.entity().clone(),
                        "search-result-list",
                        row_sizes,
                        move |this, visible_range, _, cx| {
                            if this.store.connection == proto::ConnectionState::Connected
                                && this.rpc.pending_media_count() < 4
                            {
                                for row_index in visible_range.clone() {
                                    match rows_for_list.get(row_index) {
                                        Some(SearchResultRow::Contact { source_index, .. }) => {
                                            if let Some(contact) =
                                                this.search_results.contacts.get(*source_index)
                                            {
                                                if !contact.chat_id.is_empty() {
                                                    let has_avatar = this
                                                        .store
                                                        .chat(&contact.chat_id)
                                                        .is_some_and(|c| !c.avatar_path.is_empty());
                                                    if !has_avatar
                                                        && !this
                                                            .avatar_attempted
                                                            .contains(&contact.chat_id)
                                                    {
                                                        this.load_avatar(contact.chat_id.clone());
                                                    }
                                                } else if !contact.contact_jid.is_empty() {
                                                    let has_avatar = this
                                                        .store
                                                        .participant_avatar_path(
                                                            &contact.contact_jid,
                                                        )
                                                        .is_some();
                                                    if !has_avatar
                                                        && !this
                                                            .participant_avatar_attempted
                                                            .contains(&contact.contact_jid)
                                                    {
                                                        this.load_participant_avatar(
                                                            contact.contact_jid.clone(),
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                        Some(SearchResultRow::Group { source_index, .. }) => {
                                            if let Some(chat) =
                                                this.search_results.groups.get(*source_index)
                                            {
                                                let has_avatar = this
                                                    .store
                                                    .chat(&chat.id)
                                                    .is_some_and(|c| !c.avatar_path.is_empty())
                                                    || !chat.avatar_path.is_empty();
                                                if !has_avatar
                                                    && !this.avatar_attempted.contains(&chat.id)
                                                {
                                                    this.load_avatar(chat.id.clone());
                                                }
                                            }
                                        }
                                        Some(SearchResultRow::Message { source_index, .. }) => {
                                            if let Some(chat) = this
                                                .search_results
                                                .messages
                                                .get(*source_index)
                                                .and_then(|message| message.chat.as_ref())
                                            {
                                                let has_avatar = this
                                                    .store
                                                    .chat(&chat.id)
                                                    .is_some_and(|c| !c.avatar_path.is_empty())
                                                    || !chat.avatar_path.is_empty();
                                                if !has_avatar
                                                    && !this.avatar_attempted.contains(&chat.id)
                                                {
                                                    this.load_avatar(chat.id.clone());
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            visible_range
                                .filter_map(|row_index| rows_for_list.get(row_index).cloned())
                                .filter_map(|row| this.render_search_result_row(row, cx))
                                .collect::<Vec<_>>()
                        },
                    )
                    .track_scroll(&self.search_scroll),
                )
                .scrollbar(&self.search_scroll, ScrollbarAxis::Vertical),
        );
        if query_len < 3 {
            content = content.child(
                div()
                    .flex_none()
                    .px_3()
                    .py_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(i18n::t("Type 3 characters to include messages")),
            );
        }
        content
    }

    /// One row of the list of results. Every section paints the same shape of
    /// the reference: the avatar, the name, the time of the last message and
    /// one line of text, with the part the query matched in the color of the
    /// accent.
    fn render_search_result_row(
        &self,
        row: SearchResultRow,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let selected_background = cx.theme().secondary;
        let hover_background = cx.theme().muted;
        let muted = cx.theme().muted_foreground;
        let accent = cx.theme().accent;
        let query = self.search_query.trim().to_string();
        match row {
            SearchResultRow::Header { title } => Some(
                h_flex()
                    .w_full()
                    .h(px(SEARCH_HEADER_HEIGHT * self.ui_scale))
                    .px_3()
                    .items_end()
                    .pb_1()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(muted)
                    .child(title)
                    .into_any_element(),
            ),
            SearchResultRow::Contact {
                result_index,
                source_index,
            } => {
                let contact = self.search_results.contacts.get(source_index)?.clone();
                let cached_avatar = if !contact.chat_id.is_empty() {
                    self.store
                        .chat(&contact.chat_id)
                        .map(|c| c.avatar_path.as_str())
                } else {
                    None
                };
                let avatar_path = cached_avatar
                    .or_else(|| self.store.participant_avatar_path(&contact.contact_jid))
                    .unwrap_or("");
                Some(
                    h_flex()
                        .id(("search-contact", result_index))
                        .w_full()
                        .h(px(SEARCH_CONTACT_HEIGHT * self.ui_scale))
                        .overflow_hidden()
                        .px_3()
                        .gap_3()
                        .items_center()
                        .cursor_pointer()
                        .when(result_index == self.search_highlighted, |row| {
                            row.bg(selected_background)
                        })
                        .hover(move |style| style.bg(hover_background))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.search_highlighted = result_index;
                            this.activate_search_result(window, cx);
                        }))
                        .child(avatar_element(
                            &contact.display_name,
                            avatar_path,
                            px(49. * self.ui_scale),
                            cx,
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_weight(FontWeight::MEDIUM)
                                .child(search_highlighted(&contact.display_name, &query, accent)),
                        )
                        .into_any_element(),
                )
            }
            SearchResultRow::Group {
                result_index,
                source_index,
            } => {
                let chat = self.search_results.groups.get(source_index)?.clone();
                let preview = if chat.last_message_preview.is_empty() {
                    i18n::t("No messages yet").to_string()
                } else if chat.last_message_from_me {
                    format!(
                        "{}: {}",
                        i18n::t("You"),
                        chat_preview(&chat.last_message_preview)
                    )
                } else {
                    chat_preview(&chat.last_message_preview)
                };
                let cached_avatar = self.store.chat(&chat.id).map(|c| c.avatar_path.as_str());
                let avatar_path = cached_avatar.unwrap_or(chat.avatar_path.as_str());
                let time = chat_time_label(chat.last_message_timestamp_ms, now_ms());
                let group = chat.kind() == proto::ChatKind::Group;
                Some(
                    h_flex()
                        .id(("search-chat", result_index))
                        .w_full()
                        .h(px(SEARCH_RESULT_HEIGHT * self.ui_scale))
                        .overflow_hidden()
                        .px_3()
                        .gap_3()
                        .items_center()
                        .cursor_pointer()
                        .when(result_index == self.search_highlighted, |row| {
                            row.bg(selected_background)
                        })
                        .hover(move |style| style.bg(hover_background))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.search_highlighted = result_index;
                            this.activate_search_result(window, cx);
                        }))
                        .child(avatar_element_of(
                            &chat.title,
                            avatar_path,
                            px(49. * self.ui_scale),
                            group,
                            cx,
                        ))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_1()
                                .child(
                                    h_flex()
                                        .w_full()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .truncate()
                                                .font_weight(FontWeight::MEDIUM)
                                                .child(search_highlighted(
                                                    &chat.title,
                                                    &query,
                                                    accent,
                                                )),
                                        )
                                        .child(
                                            div()
                                                .flex_none()
                                                .text_xs()
                                                .text_color(muted)
                                                .child(time),
                                        ),
                                )
                                .child(
                                    div()
                                        .w_full()
                                        .min_w_0()
                                        .truncate()
                                        .text_sm()
                                        .text_color(muted)
                                        .child(search_highlighted(&preview, &query, accent)),
                                ),
                        )
                        .into_any_element(),
                )
            }
            SearchResultRow::Message {
                result_index,
                source_index,
            } => {
                let message = self.search_results.messages.get(source_index)?.clone();
                let group = message
                    .chat
                    .as_ref()
                    .is_some_and(|chat| chat.kind() == proto::ChatKind::Group);
                let avatar_path = message
                    .chat
                    .as_ref()
                    .and_then(|chat| {
                        self.store
                            .chat(&chat.id)
                            .map(|stored| stored.avatar_path.clone())
                            .filter(|path| !path.is_empty())
                            .or_else(|| {
                                (!chat.avatar_path.is_empty()).then(|| chat.avatar_path.clone())
                            })
                    })
                    .unwrap_or_default();
                let sender = if message.sender_name == "You" {
                    i18n::t("You").to_string()
                } else {
                    message.sender_name.clone()
                };
                let snippet = one_line(&message.snippet);
                let preview = if group && !sender.is_empty() {
                    format!("{sender}: {snippet}")
                } else {
                    snippet
                };
                let time = chat_time_label(message.timestamp_ms, now_ms());
                Some(
                    h_flex()
                        .id(("search-message", result_index))
                        .w_full()
                        .h(px(SEARCH_RESULT_HEIGHT * self.ui_scale))
                        .overflow_hidden()
                        .px_3()
                        .gap_3()
                        .items_center()
                        .cursor_pointer()
                        .when(result_index == self.search_highlighted, |row| {
                            row.bg(selected_background)
                        })
                        .hover(move |style| style.bg(hover_background))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.search_highlighted = result_index;
                            this.activate_search_result(window, cx);
                        }))
                        .child(avatar_element_of(
                            &message.chat_title,
                            &avatar_path,
                            px(49. * self.ui_scale),
                            group,
                            cx,
                        ))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_1()
                                .child(
                                    h_flex()
                                        .w_full()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .truncate()
                                                .font_weight(FontWeight::MEDIUM)
                                                .child(search_highlighted(
                                                    &message.chat_title,
                                                    &query,
                                                    accent,
                                                )),
                                        )
                                        .child(
                                            div()
                                                .flex_none()
                                                .text_xs()
                                                .text_color(muted)
                                                .child(time),
                                        ),
                                )
                                .child(
                                    div()
                                        .w_full()
                                        .min_w_0()
                                        .truncate()
                                        .text_sm()
                                        .text_color(muted)
                                        .child(search_highlighted(&preview, &query, accent)),
                                ),
                        )
                        .into_any_element(),
                )
            }
        }
    }

    fn render_conversation(
        &mut self,
        compact: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let Some(chat) = self.store.selected_chat().cloned() else {
            return self
                .render_center(
                    "Whatfast",
                    i18n::t("Choose a conversation to start messaging"),
                )
                .flex_1();
        };
        if chat.avatar_path.is_empty()
            && chat.kind() != proto::ChatKind::Other
            && self.store.connection == proto::ConnectionState::Connected
        {
            self.load_avatar(chat.id.clone());
        }
        let available = window.viewport_size().width - if compact { px(56.) } else { px(396.) };
        let wrap_width = if available < px(160.) {
            px(160.)
        } else if available > px(536.) {
            px(536.)
        } else {
            available
        };
        // The wallpaper covers the whole area of the messages, not only the
        // width of the bubbles: the pattern of the reference reaches the
        // borders of the conversation.
        let wallpaper_width = if available < px(0.) {
            px(0.)
        } else {
            available
        };
        let wallpaper_height = window.viewport_size().height;
        // The wallpaper is a grid of glyphs: the battery saver does not paint
        // it, which is the biggest saving of the conversation.
        let wallpaper = (!self.battery_saver)
            .then(|| self.conversation_wallpaper(wallpaper_width, wallpaper_height, cx));
        self.measure_message_heights(wrap_width, cx.theme().font_size, window);
        let sizes = self.store.message_sizes();
        if let Some(generation) = self.scroll_to_bottom_generation.take() {
            self.stop_smooth_scroll(ScrollSurface::Messages);
            let item_count = sizes.len();
            cx.on_next_frame(window, move |this, _, cx| {
                if item_count > 0
                    && this.message_generation == generation
                    && this.store.messages.len() == item_count
                {
                    this.message_scroll
                        .scroll_to_item(item_count.saturating_sub(1), ScrollStrategy::Bottom);
                    cx.notify();
                }
            });
        }
        if let Some((anchor_id, old_offset)) = self.pending_prepend_anchor.take()
            && let Some(anchor_index) = self
                .store
                .messages
                .iter()
                .position(|message| message.id == anchor_id)
        {
            self.stop_smooth_scroll(ScrollSurface::Messages);
            let inserted_height = sizes[..anchor_index]
                .iter()
                .fold(px(0.), |height, size| height + px(size.height) + px(8.));
            self.message_scroll
                .set_offset(point(old_offset.x, old_offset.y - inserted_height));
        }
        if let Some(target_id) = self.pending_search_scroll_id.take()
            && let Some(index) = self
                .store
                .messages
                .iter()
                .position(|message| message.id == target_id)
        {
            self.stop_smooth_scroll(ScrollSurface::Messages);
            self.message_scroll
                .scroll_to_item(index, ScrollStrategy::Center);
        }
        if let Some(target_id) = self.pending_top_scroll_id.take()
            && let Some(index) = self
                .store
                .messages
                .iter()
                .position(|message| message.id == target_id)
        {
            self.stop_smooth_scroll(ScrollSurface::Messages);
            self.message_scroll
                .scroll_to_item(index, ScrollStrategy::Top);
        }
        let connected = self.store.connection == proto::ConnectionState::Connected;
        // La barra amarilla avisa de una conexion perdida, no del arranque:
        // la ventana pinta la lista de la base local antes de que WhatsApp
        // termine el handshake, y el estado "connecting" no es "sin conexion".
        let connection_lost = matches!(
            self.store.connection,
            proto::ConnectionState::Offline
                | proto::ConnectionState::Reconnecting
                | proto::ConnectionState::Failed
        );
        let has_older = self.store.has_older_messages;
        let has_newer =
            self.messages_scrolled_up || self.store.has_newer_messages || self.store.newer_activity;
        let dark = cx.theme().is_dark();
        // El fondo de la conversacion es el del tema: en la referencia es el
        // mismo gris de la columna de chats.
        let conversation_background = cx.theme().background;
        let connection_detail = if self.store.connection_detail.is_empty() {
            connection_label(self.store.connection).to_string()
        } else {
            self.store.connection_detail.clone()
        };
        let typing_line = self.typing_label(&chat.id, chat.kind() == proto::ChatKind::Group);
        let header_typing = typing_line.is_some();
        let composer_typing_line = typing_line.clone();
        // La nota que el microfono esta guardando: el campo de escribir deja
        // su lugar a la pildora del grabador.
        let recording = self.recording.as_ref();
        let recording_active = recording.is_some();
        // El boton de mandar solo esta cuando hay algo que mandar: sin texto
        // ni fotos, la referencia pone el microfono en su lugar.
        let composer_has_content =
            !self.composer.read(cx).value().trim().is_empty() || !self.pending_images.is_empty();
        // port: the `conversationSubtitle` of the origin
        // (`tauri/src/components/Conversation.tsx`): the header says that the
        // tap opens the info of the contact or of the group. The reference
        // draws the same hint.
        let contact_subtitle = if chat.kind() == proto::ChatKind::Group {
            if let Some(participants) = self.mention_directories.get(&chat.id)
                && !participants.is_empty()
            {
                let names: Vec<String> = participants
                    .iter()
                    .filter_map(|p| {
                        let name = p.display_name.trim();
                        if !name.is_empty() {
                            Some(name.to_string())
                        } else if !p.phone_number.trim().is_empty() {
                            Some(whatfast_core::phone::format(p.phone_number.trim()))
                        } else {
                            let user = p.participant_id.split('@').next().unwrap_or("").trim();
                            if !user.is_empty() {
                                Some(user.to_string())
                            } else {
                                None
                            }
                        }
                    })
                    .collect();
                if !names.is_empty() {
                    names.join(", ")
                } else {
                    i18n::t("Tap for group info").to_string()
                }
            } else {
                i18n::t("Tap for group info").to_string()
            }
        } else {
            i18n::t("Tap for contact info").to_string()
        };
        // Without connection the state of the backend is what the user
        // needs to read under the name.
        let subtitle = if connected {
            contact_subtitle
        } else {
            connection_detail
        };
        let identity_line = typing_line.unwrap_or(subtitle);
        let reply_context = self.replying_to_message_id.as_deref().map(|message_id| {
            self.store.message(message_id).map_or_else(
                || {
                    (
                        i18n::t("Original message").to_string(),
                        i18n::t("Message unavailable").to_string(),
                    )
                },
                |message| {
                    let sender = if message.from_me {
                        i18n::t("You").to_string()
                    } else if message.sender_name.trim().is_empty() {
                        i18n::t("Unknown contact").to_string()
                    } else {
                        message.sender_name.trim().to_string()
                    };
                    (sender, message_preview(message))
                },
            )
        });
        let header_avatar =
            avatar_element(&chat.title, &chat.avatar_path, px(41. * self.ui_scale), cx);
        // Las capas que flotan sobre la lista, pegadas al borde de arriba del
        // campo: el panel de los emojis y el de los stickers, el de las
        // menciones, la linea de quien escribe y la barra de la respuesta.
        // Cuelgan de un ancla que se apoya en el campo, asi crecen hacia
        // arriba y siguen al campo cuando se agranda con un mensaje largo.
        let mut composer_overlays: Vec<gpui::AnyElement> = Vec::new();
        if self.emoji_target.is_some() {
            composer_overlays.push(self.render_emoji_picker(cx).into_any_element());
        }
        if self.sticker_tray_open {
            // The tray of the stickers opens over the composer, the same way
            // the panel of the emojis does.
            composer_overlays.push(self.sticker_tray.render(self, cx).into_any_element());
        }
        if self.mention_popup_visible() {
            composer_overlays.push(self.render_mention_picker(cx).into_any_element());
        }
        if let Some(typing) = composer_typing_line.clone() {
            composer_overlays.push(
                div()
                    .h(px(24. * self.ui_scale))
                    .px_4()
                    .pt_1()
                    .text_xs()
                    .text_color(if dark {
                        gpui::Hsla::from(rgb(0x25d366))
                    } else {
                        gpui::Hsla::from(rgb(DARK_GREEN))
                    })
                    .child(typing)
                    .into_any_element(),
            );
        }
        if let Some((sender, preview)) = reply_context.clone() {
            composer_overlays.push(
                h_flex()
                    .mx_3()
                    .mt_2()
                    .px_3()
                    .py_2()
                    .gap_3()
                    .rounded_md()
                    .bg(cx.theme().secondary)
                    .child(
                        v_flex()
                            .min_w_0()
                            .flex_1()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(0x25d366))
                                    .child(i18n::t_args(
                                        "Replying to {sender}",
                                        &[("sender", &sender)],
                                    )),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(preview),
                            ),
                    )
                    .child(
                        Button::new("cancel-reply")
                            .small()
                            .label("×")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.cancel_reply();
                                cx.notify();
                            })),
                    )
                    .into_any_element(),
            );
        }
        // El nombre de la conversacion se mueve al pintarlo: estos dos datos
        // se sacan antes.
        let is_group = chat.kind() == proto::ChatKind::Group;
        let header_chat_id = chat.id.clone();
        // La barra de los mensajes fijados: la referencia la pinta arriba de
        // la lista y vuelve al mensaje con un clic. Los fijados ya estaban en
        // el modelo (el menu del mensaje los fija y el backend los manda),
        // pero faltaba la barra que los muestra.
        let pinned = self.store.pinned_messages(&chat.id);
        let pinned_count = pinned.len();
        let pinned_target = pinned.first().map(|pin| pin.message_id.clone());
        let pinned_preview = pinned
            .first()
            .and_then(|pin| pin.message.as_ref())
            .map(message_preview)
            .unwrap_or_default();
        let pinned_author = pinned
            .first()
            .and_then(|pin| pin.message.as_ref())
            .map_or_else(
                || "Pinned message".to_string(),
                |message| {
                    if message.from_me {
                        "You pinned a message".to_string()
                    } else if message.sender_name.trim().is_empty() {
                        "Pinned message".to_string()
                    } else {
                        format!("{} pinned a message", message.sender_name.trim())
                    }
                },
            );
        let pinned_bar = pinned_target.map(|target| {
            let scale = self.ui_scale;
            let label = if pinned_count == 1 {
                pinned_author.clone()
            } else {
                format!("{pinned_count} pinned messages")
            };
            let accent = cx.theme().accent;
            h_flex()
                .id("pinned-bar")
                .flex_none()
                .h(px(44. * scale))
                .px_4()
                .gap_3()
                .items_center()
                .border_b_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().secondary)
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.scroll_to_message(&target);
                    cx.notify();
                }))
                .child(
                    Icon::new(WhatfastIcon::Pin)
                        .with_size(px(16. * scale))
                        .text_color(accent),
                )
                .child(
                    v_flex()
                        .min_w_0()
                        .flex_1()
                        .child(
                            div()
                                .truncate()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(accent)
                                .child(label),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(pinned_preview.clone()),
                        ),
                )
        });
        let list_sizes = Rc::new(
            sizes
                .iter()
                .map(|entry| size(px(entry.width), px(entry.height)))
                .collect::<Vec<_>>(),
        );
        v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .relative()
            // Ctrl+V con una imagen en el portapapeles la manda como foto de
            // la conversacion. La accion se toma antes que el campo de
            // escritura, que pega su texto cuando no hay imagen.
            .capture_action(
                cx.listener(|this, _: &gpui_component::input::Paste, window, cx| {
                    if this.paste_image_from_clipboard(window, cx) {
                        cx.stop_propagation();
                        cx.notify();
                    }
                }),
            )
            // The strip of the open chats sits above the header of the
            // conversation, the same way the web client draws it.
            .child(self.tab_strip.clone())
            .child(
                h_flex()
                    .h(px(64. * self.ui_scale))
                    .px_4()
                    .gap_3()
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .when(compact, |header| {
                        header.child(Button::new("back").label("‹").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.store.selected_chat_id = None;
                                // The band of the drafts belongs to the chat
                                // that was open, so it goes away with it.
                                this.reload_agent_drafts();
                                this.reaction_details = None;
                                this.cancel_reply();
                                cx.notify();
                            },
                        )))
                    })
                    .child(
                        h_flex()
                            .id("chat-info-open")
                            .flex_1()
                            .min_w_0()
                            .gap_3()
                            .items_center()
                            .cursor_pointer()
                            .on_click(cx.listener({
                                let chat_id = chat.id.clone();
                                move |this, _, _, cx| {
                                    this.open_chat_info(chat_id.clone());
                                    cx.notify();
                                }
                            }))
                            .child(header_avatar)
                            .child(
                                v_flex()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .truncate()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(chat.title),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .text_xs()
                                            .text_color(if header_typing {
                                                if dark {
                                                    gpui::Hsla::from(rgb(0x25d366))
                                                } else {
                                                    gpui::Hsla::from(rgb(DARK_GREEN))
                                                }
                                            } else {
                                                cx.theme().muted_foreground
                                            })
                                            .child(identity_line),
                                    ),
                            ),
                    )
                    // Los botones de la derecha de la cabecera: el de la
                    // columna de los chats cuando esta escondida, el del
                    // llamado, el buscador y el menu de la conversacion.
                    // Las llamadas son de voz: el icono de video queda para
                    // cuando el video llegue.
                    .child(
                        h_flex()
                            .flex_none()
                            .gap_1()
                            .when(!is_group, |row| {
                                row.child(
                                    Button::new("call-chat")
                                        .small()
                                        .icon(WhatfastIcon::Phone)
                                        .tooltip(i18n::t("Voice call"))
                                        .on_click(cx.listener({
                                            let chat_id = header_chat_id.clone();
                                            move |this, _, _, cx| {
                                                this.call_place(chat_id.clone(), false);
                                                cx.notify();
                                            }
                                        })),
                                )
                                .child(
                                    Button::new("call-video-chat")
                                        .small()
                                        .icon(WhatfastIcon::Video)
                                        .tooltip(i18n::t("Video call"))
                                        .on_click(cx.listener({
                                            let chat_id = header_chat_id.clone();
                                            move |this, _, _, cx| {
                                                this.call_place(chat_id.clone(), true);
                                                cx.notify();
                                            }
                                        })),
                                )
                            })
                            .when(self.sidebar_hidden, |row| {
                                row.child(
                                    Button::new("show-sidebar")
                                        .small()
                                        .icon(WhatfastIcon::Sidebar)
                                        .tooltip(i18n::t("Show chat list"))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.sidebar_hidden = false;
                                            cx.notify();
                                        })),
                                )
                            })
                            .child(
                                Button::new("search-in-chat")
                                    .small()
                                    .icon(IconName::Search)
                                    .tooltip(i18n::t("Search in chat"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.focus_search(window, cx);
                                    })),
                            )
                            // port: the origin shows the members of a group
                            // in a panel that this button opens.
                            .when(is_group, |row| {
                                row.child(
                                    Button::new("show-members")
                                        .small()
                                        .icon(IconName::User)
                                        .tooltip(i18n::t("Show members"))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            let open = !this.member_panel.is_open();
                                            this.member_panel.set_open(open);
                                            cx.notify();
                                        })),
                                )
                            })
                            .child(
                                Button::new("conversation-menu")
                                    .small()
                                    .icon(IconName::EllipsisVertical)
                                    .tooltip(i18n::t("Conversation menu"))
                                    .on_click(cx.listener({
                                        let chat_id = header_chat_id.clone();
                                        move |this, _, window, cx| {
                                            // El menu abre donde esta el
                                            // puntero, como el de la
                                            // referencia.
                                            let position = window.mouse_position();
                                            this.open_chat_menu(chat_id.clone(), position, cx);
                                            cx.notify();
                                        }
                                    })),
                            ),
                    ),
            )
            // La barra de los fijados, entre la cabecera y la lista.
            .when_some(pinned_bar, |column, bar| column.child(bar))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .bg(conversation_background)
                    .on_scroll_wheel(cx.listener(|this, event, window, cx| {
                        this.handle_smooth_scroll_input(ScrollSurface::Messages, event, window, cx);
                    }))
                    // port: the reference paints a wallpaper of line drawings
                    // behind the messages. The layer is the first child of the
                    // area, so it stays behind the bubbles, and it is
                    // absolute, so it does not move when the list scrolls.
                    .when_some(wallpaper, |area, layer| area.child(layer))
                    // port: the button that loads the older messages is a
                    // small pill above the list of the messages, the same way
                    // the reference draws it. It is not over the list: the
                    // pill of the day of the first message lives there.
                    .when(has_older, |area| {
                        area.child(
                            div().flex().justify_center().px_4().pt_2().pb_1().child(
                                div()
                                    .id("load-older")
                                    .px_3()
                                    .py_1()
                                    .rounded_full()
                                    .bg(cx.theme().secondary)
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .cursor_pointer()
                                    .child(i18n::t("Load older messages"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.load_older_messages();
                                        cx.notify();
                                    })),
                            ),
                        )
                    })
                    .child(
                        div().flex_1().min_h_0().child(
                            v_virtual_list(
                                cx.entity().clone(),
                                "messages",
                                list_sizes,
                                |this, range: Range<usize>, _window, cx| {
                                    let visible_images = range
                                        .clone()
                                        .filter_map(|index| this.store.messages.get(index))
                                        .filter_map(|message| match message.content.as_ref() {
                                            Some(proto::message::Content::Image(image))
                                                if image_needs_download(image) =>
                                            {
                                                Some((message.chat_id.clone(), message.id.clone()))
                                            }
                                            _ => None,
                                        })
                                        .collect::<Vec<_>>();
                                    for (chat_id, message_id) in visible_images {
                                        this.load_message_image(chat_id, message_id);
                                    }
                                    // The audios of the window download on
                                    // their own: without the file the bubble
                                    // cannot draw the player and falls back to
                                    // the card with the text "Voice message".
                                    let visible_audios = range
                                        .clone()
                                        .filter_map(|index| this.store.messages.get(index))
                                        .filter_map(|message| match message.content.as_ref() {
                                            Some(proto::message::Content::Attachment(
                                                attachment,
                                            )) if attachments::audio_needs_download(attachment) => {
                                                Some((message.chat_id.clone(), message.id.clone()))
                                            }
                                            _ => None,
                                        })
                                        .collect::<Vec<_>>();
                                    for (chat_id, message_id) in visible_audios {
                                        this.load_message_attachment(chat_id, message_id);
                                    }
                                    if this
                                        .store
                                        .selected_chat()
                                        .is_some_and(|chat| chat.kind() == proto::ChatKind::Group)
                                    {
                                        let participants = range
                                            .clone()
                                            .filter_map(|index| this.store.messages.get(index))
                                            .filter(|message| {
                                                !message.from_me
                                                    && message.sender_avatar_path.is_empty()
                                                    && this
                                                        .store
                                                        .participant_avatar_path(&message.sender_id)
                                                        .is_none()
                                                    && !message.sender_id.is_empty()
                                            })
                                            .map(|message| message.sender_id.clone())
                                            .collect::<HashSet<_>>();
                                        for participant_id in participants {
                                            this.load_participant_avatar(participant_id);
                                        }
                                    }
                                    this.messages_scrolled_up =
                                        range.end < this.store.messages.len();
                                    if range.end == this.store.messages.len()
                                        && (this.store.has_newer_messages
                                            || this.store.newer_activity)
                                    {
                                        this.load_newer_messages();
                                    } else if range.end == this.store.messages.len() {
                                        this.mark_read();
                                    }
                                    range.map(|index| this.render_message(index, cx)).collect()
                                },
                            )
                            .track_scroll(&self.message_scroll)
                            .p_4()
                            .gap(px(2. * self.ui_scale)),
                        ),
                    )
                    // The button that jumps to the latest messages floats over
                    // the list of the messages, the same way the web client
                    // draws it. It comes after the list, so the messages do
                    // not paint over it.
                    .when(has_newer, |area| {
                        area.child(
                            div().absolute().bottom_4().right_4().child(
                                // port: the reference draws a round button
                                // with an arrow at the bottom right of the
                                // list, not a wide bar in the middle.
                                div()
                                    .id("jump-latest")
                                    .size(px(40. * self.ui_scale))
                                    .rounded_full()
                                    .bg(cx.theme().accent)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .child(
                                        Icon::new(IconName::ArrowDown)
                                            .with_size(px(20. * self.ui_scale))
                                            .text_color(cx.theme().accent_foreground),
                                    )
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.jump_to_bottom(window, cx);
                                    })),
                            ),
                        )
                    }),
            )
            .when(connection_lost, |column| {
                column.child(
                    div()
                        .px_4()
                        .py_2()
                        .bg(if dark { rgb(0x594918) } else { rgb(0xffe9b5) })
                        .child(i18n::t(
                            "Offline — messages will be available after reconnection",
                        )),
                )
            })
            // El pie del chat: la bandeja de las fotos, el ancla de las capas
            // que flotan y el campo. El campo crece con el texto, asi que el
            // pie no tiene altura fija: la lista de los mensajes es la que
            // cede el lugar.
            .child(
                v_flex()
                    .relative()
                    .flex_shrink_0()
                    // La bandeja de las fotos va entre la lista y el campo,
                    // como en la referencia: el campo nunca la tapa y la lista
                    // le hace lugar.
                    .when(!self.pending_images.is_empty(), |block| {
                        block.child(self.render_pending_images(cx))
                    })
                    // El ancla de lo que flota: se apoya en el borde de
                    // arriba del campo y crece hacia arriba, asi un campo de
                    // seis lineas no tapa los paneles.
                    .child(
                        div()
                            .absolute()
                            .bottom(relative(1.))
                            .left_0()
                            .right_0()
                            .children(composer_overlays),
                    )
                    // La banda de los borradores de los agentes va pegada
                    // arriba del campo: no flota y no tapa la conversacion, y
                    // el usuario la responde sin salir del chat.
                    .when(!self.agent_drafts.is_empty(), |block| {
                        block.child(crate::chat_drafts::render(self, cx))
                    })
                    .child(
                        v_flex()
                            .min_h(px(56. * self.ui_scale))
                            .border_t_1()
                            .border_color(cx.theme().border)
                            // Mientras el microfono graba, el campo de
                            // escribir deja su lugar a la pildora flotante,
                            // centrada, con la onda y los botones.
                            .when(recording_active, |footer| {
                                footer.child(self.render_recording_pill(cx))
                            })
                            .when(!recording_active, |footer| {
                                footer.child(
                                    // The pill of the message: the clip, the
                                    // emoji, the text and the button of
                                    // sending, the same way the reference
                                    // draws it.
                                    h_flex().p_2().gap_2().child(
                                        h_flex()
                                            .flex_1()
                                            .min_h(px(42. * self.ui_scale))
                                            .rounded_lg()
                                            .bg(cx.theme().input)
                                            .px_2()
                                            .py_1()
                                            .gap_1()
                                            .items_end()
                                            .child(self.attachment_bar.clone())
                                            .child(
                                                Button::new("emoji-picker")
                                                    .ghost()
                                                    .icon(WhatfastIcon::Smile)
                                                    .tooltip(i18n::t("Emoji"))
                                                    .disabled(!connected)
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        this.toggle_composer_emoji();
                                                        cx.notify();
                                                    })),
                                            )
                                            .child(
                                                Input::new(&self.composer)
                                                    .appearance(false)
                                                    .disabled(!connected)
                                                    .flex_1(),
                                            )
                                            .when(composer_has_content, |pill| {
                                                pill.child(
                                                    Button::new("send")
                                                        .primary()
                                                        // El boton cuenta las
                                                        // fotos que van a
                                                        // salir, como el de la
                                                        // referencia.
                                                        .label(
                                                            crate::composer_attachments::send_label(
                                                                self.pending_images.len(),
                                                            ),
                                                        )
                                                        .disabled(!connected)
                                                        .on_click(cx.listener(
                                                            |this, _, window, cx| {
                                                                this.submit_composer(window, cx)
                                                            },
                                                        )),
                                                )
                                            })
                                            .when(!composer_has_content, |pill| {
                                                pill.child(
                                                    Button::new("record-voice-note")
                                                        .ghost()
                                                        .icon(WhatfastIcon::Mic)
                                                        .tooltip(i18n::t("Record a voice note"))
                                                        .disabled(!connected)
                                                        .on_click(cx.listener(
                                                            |this, _, window, cx| {
                                                                this.start_voice_recording(
                                                                    window, cx,
                                                                );
                                                                cx.notify();
                                                            },
                                                        )),
                                                )
                                            }),
                                    ),
                                )
                            }),
                    ),
            )
    }

    /// La pildora del grabador: el tacho a la izquierda, el punto y el reloj,
    /// la onda en el centro y los botones de pausa y de mandar a la derecha.
    /// Flota centrada en el pie, como la de WhatsApp, en vez de ocupar el
    /// ancho entero del campo.
    fn render_recording_pill(&self, cx: &mut Context<Self>) -> gpui::Div {
        let scale = self.ui_scale;
        let Some(state) = self.recording.as_ref() else {
            return h_flex();
        };
        let dark = cx.theme().mode.is_dark();
        let danger = cx.theme().danger;
        let muted = cx.theme().muted_foreground;
        let coral: gpui::Hsla = rgb(0xf15c6d).into();
        let timer = attachments::format_clock(state.elapsed_seconds());
        let finishing = state.finishing;
        let paused = state.paused;
        let levels = state.levels.clone();
        let bars = crate::audio::WAVE_BARS;
        let background = if dark {
            gpui::Hsla::from(rgb(0x1f2c33))
        } else {
            gpui::Hsla::from(rgb(0xffffff))
        };
        let foreground = if dark {
            gpui::Hsla::from(rgb(0xffffff))
        } else {
            gpui::Hsla::from(rgb(0x111b21))
        };
        let wave_color = if dark {
            rgba(0xffffff73)
        } else {
            rgba(0x00000059)
        };
        let hover = rgba(0xffffff1f);
        h_flex().p_2().w_full().justify_center().child(
            h_flex()
                .id("recording-pill")
                .w(px(420. * scale))
                .max_w_full()
                .h(px(48. * scale))
                .px(px(8. * scale))
                .gap(px(6. * scale))
                .items_center()
                .rounded_full()
                .bg(background)
                .shadow_lg()
                .when(!finishing, |pill| {
                    pill.child(
                        div()
                            .id("recording-trash")
                            .size(px(34. * scale))
                            .flex_none()
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(move |style| style.bg(hover))
                            .child(
                                Icon::new(WhatfastIcon::TrashOutline)
                                    .with_size(px(19. * scale))
                                    .text_color(muted),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.cancel_voice_recording(cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        h_flex()
                            .flex_none()
                            .items_center()
                            .gap(px(6. * scale))
                            .child(
                                div()
                                    .size(px(8. * scale))
                                    .flex_none()
                                    .rounded_full()
                                    .bg(if paused { muted } else { danger }),
                            )
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(foreground)
                                    .child(timer.clone()),
                            ),
                    )
                    // La onda: la barra mas nueva entra por la derecha y la
                    // mas vieja sale por la izquierda. Pausada, la onda queda
                    // quieta y apagada.
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w(px(60. * scale))
                            .h(px(26. * scale))
                            .items_center()
                            .gap(px(2. * scale))
                            .overflow_hidden()
                            .children((0..bars).map(|index| {
                                let level = levels
                                    .get(levels.len().saturating_sub(bars) + index)
                                    .copied()
                                    .unwrap_or(0.08);
                                let height = (4. + level.clamp(0.08, 1.) * 20.) * scale;
                                div()
                                    .w(px(3. * scale))
                                    .flex_none()
                                    .h(px(height))
                                    .rounded_full()
                                    .bg(if paused { rgba(0xffffff2e) } else { wave_color })
                            })),
                    )
                    .child(
                        div()
                            .id("recording-pause")
                            .size(px(34. * scale))
                            .flex_none()
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(move |style| style.bg(hover))
                            .child(
                                Icon::new(if paused {
                                    WhatfastIcon::Play
                                } else {
                                    WhatfastIcon::Pause
                                })
                                .with_size(px(18. * scale))
                                .text_color(coral),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.recording.as_ref().is_some_and(|state| state.paused) {
                                    this.resume_voice_recording(cx);
                                } else {
                                    this.pause_voice_recording(cx);
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .id("recording-send")
                            .size(px(38. * scale))
                            .flex_none()
                            .rounded_full()
                            .bg(cx.theme().accent)
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|style| style.opacity(0.9))
                            .child(
                                Icon::new(WhatfastIcon::Send)
                                    .with_size(px(18. * scale))
                                    .text_color(cx.theme().accent_foreground),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.stop_voice_recording(cx);
                                cx.notify();
                            })),
                    )
                })
                .when(finishing, |pill| {
                    pill.child(
                        h_flex()
                            .flex_1()
                            .justify_center()
                            .gap(px(6. * scale))
                            .items_center()
                            .child(div().size(px(8. * scale)).rounded_full().bg(danger))
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(foreground)
                                    .child(timer.clone()),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(muted)
                                    .child(i18n::t("Sending voice note…")),
                            ),
                    )
                }),
        )
    }

    fn render_mention_picker(&self, cx: &mut Context<Self>) -> gpui::Div {
        let matches = self.mention_matches();
        let highlighted = self
            .mention_picker
            .as_ref()
            .map_or(0, |picker| picker.highlighted);
        let selected_background = cx.theme().secondary;
        let dark = cx.theme().mode.is_dark();
        let hover_background = if dark { rgb(0x2a3942) } else { rgb(0xf0f2f5) };
        v_flex()
            .mx_3()
            .mt_2()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .when(matches.is_empty(), |column| {
                column.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(i18n::t("Loading group members…")),
                )
            })
            .children(matches.into_iter().enumerate().map(|(index, participant)| {
                let name = mention_display_name(&participant);
                let phone = participant.phone_number.clone();
                h_flex()
                    .id(("mention-row", index))
                    .px_3()
                    .py_2()
                    .gap_3()
                    .items_center()
                    .cursor_pointer()
                    .when(index == highlighted, |row| row.bg(selected_background))
                    .hover(move |style| style.bg(hover_background))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.apply_mention(index, window, cx);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(name),
                    )
                    .when(!phone.is_empty(), |row| {
                        row.child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(phone),
                        )
                    })
            }))
    }

    /// The panel of the emojis.
    ///
    /// The reference draws it as a floating box above the composer, with
    /// the rounded corners, the border and the shadow: the box is painted
    /// over the list of the messages and does not push it.
    fn render_emoji_picker(&self, cx: &mut Context<Self>) -> gpui::Div {
        // The measures of the box and of the grid.
        const EMOJI_CELL: f32 = 44.;
        const EMOJI_GLYPH: f32 = 24.;
        const EMOJI_GAP: f32 = 2.;
        const EMOJI_PADDING: f32 = 8.;
        const EMOJI_PANEL_HEIGHT: f32 = 380.;
        const EMOJI_PANEL_GAP: f32 = 8.;
        const EMOJI_PANEL_MARGIN: f32 = 12.;
        let selected = self.emoji_category;
        let reaction_mode = matches!(self.emoji_target, Some(EmojiTarget::Reaction { .. }));
        let has_own_reaction = match &self.emoji_target {
            Some(EmojiTarget::Reaction { message_id, .. }) => self
                .store
                .messages
                .iter()
                .find(|message| &message.id == message_id)
                .is_some_and(|message| message.reactions.iter().any(|reaction| reaction.from_me)),
            _ => false,
        };
        let scale = self.ui_scale;
        // The grid of the reference is dense: the cell is bigger than the
        // one of the origin and the panel has one column more.
        let columns = EMOJI_COLUMNS + 1;
        let cell = px(EMOJI_CELL * scale);
        let gap = px(EMOJI_GAP * scale);
        let padding = px(EMOJI_PADDING * scale);
        // The section of the recent ones goes at the top of the panel, when
        // the category that is active is the one of all.
        let recent = emoji_recent::recent_emojis();
        let show_recent = selected == EmojiCategory::All && !recent.is_empty();
        let recent_rows = recent.len().div_ceil(columns);
        // The rows of the list: the title of the recent ones, the emojis
        // used lately, the title of the section and the grid of the emojis.
        let title_row = if show_recent { 1 + recent_rows } else { 0 };
        let empty = self.emoji_results.is_empty();
        let grid_rows = self.emoji_results.len().div_ceil(columns);
        let total_rows = title_row + 1 + grid_rows;
        // The title of the section: the one of the reference when the
        // category is the one of all, the name of the category otherwise.
        let label: &'static str = if selected == EmojiCategory::All {
            i18n::t("Smileys and people")
        } else {
            selected.label()
        };
        let hover = cx.theme().muted;
        let foreground = cx.theme().foreground;
        let muted_foreground = cx.theme().muted_foreground;
        // The width of the box: the grid of the emojis and the padding of
        // its rows.
        let width = px((columns as f32 * (EMOJI_CELL + EMOJI_GAP) - EMOJI_GAP
            + EMOJI_PADDING * 2.)
            * scale);
        // El panel cuelga del ancla que se apoya en el borde de arriba del
        // campo: solo deja el hueco de la referencia contra el campo. La
        // barra de la respuesta es hermana suya y va debajo, asi que no hay
        // que sumar su alto ni el de la bandeja de las fotos.
        v_flex()
            .ml(px(EMOJI_PANEL_MARGIN * scale))
            .mb(px(EMOJI_PANEL_GAP * scale))
            .w(width)
            .max_w_full()
            .h(px(EMOJI_PANEL_HEIGHT * scale))
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .shadow_lg()
            .overflow_hidden()
            // La fila de las categorias, con el dibujo de cada una, como la
            // referencia.
            .child(self.render_emoji_categories(cx))
            // El buscador, con la lupa dentro.
            .child(
                h_flex()
                    .px_3()
                    .pb_2()
                    .gap_2()
                    .child(
                        Input::new(&self.emoji_search)
                            .w_full()
                            .h(px(36. * scale))
                            .rounded_md()
                            .bg(cx.theme().input)
                            .border_1()
                            .border_color(cx.theme().accent)
                            .prefix(
                                Icon::new(IconName::Search)
                                    .with_size(px(16. * scale))
                                    .text_color(cx.theme().muted_foreground),
                            ),
                    )
                    .when(has_own_reaction, |row| {
                        row.child(
                            Button::new("remove-reaction")
                                .label(i18n::t("Remove mine"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.remove_reaction();
                                    cx.notify();
                                })),
                        )
                    }),
            )
            // La rejilla: la seccion de los recientes, el titulo de la
            // seccion y los emojis de la categoria.
            .child(
                div().flex_1().min_h_0().child(
                    uniform_list(
                        "emoji-results",
                        total_rows,
                        cx.processor(move |this, rows: Range<usize>, _, cx| {
                            rows.map(|row| {
                                // El titulo de la seccion de los
                                // recientes.
                                if show_recent && row == 0 {
                                    return h_flex().h(cell).px(padding).items_center().child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(foreground)
                                            .child(i18n::t("Recent")),
                                    );
                                }
                                // Los emojis usados hace poco.
                                if show_recent && row <= recent_rows {
                                    let first = (row - 1) * columns;
                                    return h_flex().h(cell).px(padding).gap(gap).children(
                                        (0..columns).filter_map(|column| {
                                            let index = first + column;
                                            let value = recent.get(index)?.clone();
                                            let picked = value.clone();
                                            Some(
                                                div()
                                                    .id(("recent-emoji", index))
                                                    .size(cell)
                                                    .rounded_md()
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .text_size(px(EMOJI_GLYPH * scale))
                                                    .cursor_pointer()
                                                    .hover(move |style| style.bg(hover))
                                                    .on_click(cx.listener(
                                                        move |this, _, window, cx| {
                                                            emoji_recent::remember_emoji(&picked);
                                                            this.choose_emoji(&picked, window, cx);
                                                            cx.notify();
                                                        },
                                                    ))
                                                    .child(value),
                                            )
                                        }),
                                    );
                                }
                                // El titulo de la seccion.
                                if row == title_row {
                                    if empty {
                                        return h_flex()
                                            .h(cell)
                                            .px(padding)
                                            .items_center()
                                            .justify_center()
                                            .text_sm()
                                            .text_color(muted_foreground)
                                            .child(i18n::t("No emoji found"));
                                    }
                                    return h_flex().h(cell).px(padding).items_center().child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(foreground)
                                            .child(label),
                                    );
                                }
                                // La rejilla de los emojis de la
                                // categoria.
                                let grid_row = row - title_row - 1;
                                h_flex().h(cell).px(padding).gap(gap).children(
                                    (0..columns).filter_map(|column| {
                                        let index = grid_row * columns + column;
                                        let emoji = this.emoji_results.get(index)?;
                                        let value = emoji.as_str().to_string();
                                        let picked = value.clone();
                                        Some(
                                            div()
                                                .id(("emoji", index))
                                                .size(cell)
                                                .rounded_md()
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .text_size(px(EMOJI_GLYPH * scale))
                                                .cursor_pointer()
                                                .hover(move |style| style.bg(hover))
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        emoji_recent::remember_emoji(&picked);
                                                        this.choose_emoji(&picked, window, cx);
                                                        cx.notify();
                                                    },
                                                ))
                                                .child(value),
                                        )
                                    }),
                                )
                            })
                            .collect::<Vec<_>>()
                        }),
                    )
                    .h_full(),
                ),
            )
            .when(reaction_mode, |panel| {
                panel.child(
                    div()
                        .px_3()
                        .pb_1()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(i18n::t("Choose one emoji to react")),
                )
            })
            // Los tabs del pie: los emojis, las imagenes con movimiento y los
            // stickers.
            .when(!reaction_mode, |panel| {
                panel.child(self.render_panel_tabs(cx))
            })
    }

    /// The row of the categories of emojis, with the drawing of each one.
    ///
    /// The reference underlines the category that is active with a green
    /// line under its drawing.
    fn render_emoji_categories(&self, cx: &mut Context<Self>) -> gpui::Div {
        // The measures of the row and of the line of the active category.
        const CATEGORY_SIZE: f32 = 34.;
        const CATEGORY_GLYPH: f32 = 19.;
        const CATEGORY_LINE: f32 = 20.;
        let selected = self.emoji_category;
        let hover = cx.theme().muted;
        let accent = cx.theme().accent;
        let scale = self.ui_scale;
        h_flex().flex_none().px_2().py_2().gap_1().children(
            EmojiCategory::ALL
                .into_iter()
                .enumerate()
                .map(|(index, category)| {
                    let active = category == selected;
                    v_flex()
                        .id(("emoji-category", index))
                        .w(px(CATEGORY_SIZE * scale))
                        .h(px(CATEGORY_SIZE * scale))
                        .rounded_md()
                        .items_center()
                        .justify_center()
                        .gap(px(2.))
                        .cursor_pointer()
                        .hover(move |style| style.bg(hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_emoji_category(category);
                            cx.notify();
                        }))
                        .child(
                            div()
                                .text_size(px(CATEGORY_GLYPH * scale))
                                .child(emoji_category_glyph(category)),
                        )
                        // The green line under the drawing of the category
                        // that is active.
                        .child(
                            div()
                                .h(px(2.))
                                .w(px(CATEGORY_LINE * scale))
                                .rounded_full()
                                .when(active, |line| line.bg(accent)),
                        )
                }),
        )
    }

    /// The tabs of the panel of the composer.
    ///
    /// port: the reference has three tabs at the foot of the panel: the
    /// emojis, the pictures with movement and the stickers. The app has no
    /// pictures with movement, so that tab cannot be pressed.
    pub(crate) fn render_panel_tabs(&self, cx: &mut Context<Self>) -> gpui::Div {
        // The measures of the pill and of its tabs.
        const TAB_SIZE: f32 = 28.;
        const TAB_WIDTH: f32 = 44.;
        let active = self.panel_tab;
        let hover = cx.theme().muted;
        let accent = cx.theme().accent;
        let muted_foreground = cx.theme().muted_foreground;
        let scale = self.ui_scale;
        h_flex()
            .flex_none()
            .justify_center()
            .border_t_1()
            .border_color(cx.theme().border)
            .py(px(8. * scale))
            // The three tabs live in a pill, like in the reference.
            .child(
                h_flex()
                    .rounded_full()
                    .bg(cx.theme().secondary)
                    .gap(px(2.))
                    .p(px(3. * scale))
                    .child(
                        div()
                            .id("panel-tab-emoji")
                            .h(px(TAB_SIZE * scale))
                            .w(px(TAB_WIDTH * scale))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .when(active == PanelTab::Emoji, |tab| {
                                tab.bg(accent.opacity(0.18))
                            })
                            .hover(move |style| style.bg(hover))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_panel_tab(PanelTab::Emoji, cx);
                            }))
                            .child(
                                Icon::new(WhatfastIcon::Smile)
                                    .with_size(px(18. * scale))
                                    .text_color(if active == PanelTab::Emoji {
                                        accent
                                    } else {
                                        muted_foreground
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .id("panel-tab-gif")
                            .h(px(TAB_SIZE * scale))
                            .w(px(TAB_WIDTH * scale))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_xs()
                            .text_color(muted_foreground)
                            .child(i18n::t("GIF")),
                    )
                    .child(
                        div()
                            .id("panel-tab-sticker")
                            .h(px(TAB_SIZE * scale))
                            .w(px(TAB_WIDTH * scale))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .when(active == PanelTab::Sticker, |tab| {
                                tab.bg(accent.opacity(0.18))
                            })
                            .hover(move |style| style.bg(hover))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_panel_tab(PanelTab::Sticker, cx);
                            }))
                            .child(
                                Icon::new(WhatfastIcon::Sticker)
                                    .with_size(px(18. * scale))
                                    .text_color(if active == PanelTab::Sticker {
                                        accent
                                    } else {
                                        muted_foreground
                                    }),
                            ),
                    ),
            )
    }

    fn render_reaction_details(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let Some(details) = self.reaction_details.as_ref() else {
            return div();
        };
        let emoji = details.emoji.clone();
        let count = details.reactions.len();
        let list_height = px((count.clamp(1, 7) as f32) * 58.);
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .bg(rgba(0x00000088))
            .child(
                v_flex()
                    .w(px(420.))
                    .max_w_full()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .child(
                        h_flex()
                            .h(px(58.))
                            .px_4()
                            .items_center()
                            .justify_between()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(
                                v_flex()
                                    .child(
                                        div()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(format!("{emoji} Reactions")),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(format!(
                                                "{count} {}",
                                                if count == 1 { "person" } else { "people" }
                                            )),
                                    ),
                            )
                            .child(Button::new("close-reaction-details").label("×").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.reaction_details = None;
                                    cx.notify();
                                }),
                            )),
                    )
                    .child(
                        uniform_list(
                            "reaction-details-reactors",
                            count,
                            cx.processor(|this, range: Range<usize>, _, cx| {
                                let reactions = this
                                    .reaction_details
                                    .as_ref()
                                    .map(|details| details.reactions.clone())
                                    .unwrap_or_default();
                                let visible = range
                                    .filter_map(|index| {
                                        reactions
                                            .get(index)
                                            .cloned()
                                            .map(|reaction| (index, reaction))
                                    })
                                    .collect::<Vec<_>>();
                                for (_, reaction) in &visible {
                                    if reaction.sender_avatar_path.is_empty()
                                        && this
                                            .store
                                            .participant_avatar_path(&reaction.sender_id)
                                            .is_none()
                                    {
                                        this.load_participant_avatar(reaction.sender_id.clone());
                                    }
                                }
                                visible
                                    .into_iter()
                                    .map(|(index, reaction)| {
                                        let name = reaction_sender_name(&reaction).to_string();
                                        let detail = reaction_sender_detail(&reaction).to_string();
                                        let cached_avatar = this
                                            .store
                                            .participant_avatar_path(&reaction.sender_id)
                                            .unwrap_or_default();
                                        let avatar_path = if reaction.sender_avatar_path.is_empty()
                                        {
                                            cached_avatar
                                        } else {
                                            reaction.sender_avatar_path.as_str()
                                        };
                                        let avatar =
                                            avatar_element(&name, avatar_path, px(38.), cx);
                                        h_flex()
                                            .id(("reaction-detail-row", index))
                                            .h(px(58.))
                                            .px_4()
                                            .gap_3()
                                            .items_center()
                                            .child(avatar)
                                            .child(
                                                v_flex()
                                                    .min_w_0()
                                                    .child(
                                                        div()
                                                            .truncate()
                                                            .font_weight(if reaction.from_me {
                                                                FontWeight::SEMIBOLD
                                                            } else {
                                                                FontWeight::NORMAL
                                                            })
                                                            .child(name),
                                                    )
                                                    .child(
                                                        div()
                                                            .truncate()
                                                            .text_xs()
                                                            .text_color(cx.theme().muted_foreground)
                                                            .child(detail),
                                                    ),
                                            )
                                    })
                                    .collect::<Vec<_>>()
                            }),
                        )
                        .h(list_height),
                    ),
            )
    }

    /// The panel of the settings.
    ///
    /// The reference of WhatsApp Desktop (`docs/referencia/04-whatsapp-perfil.png`)
    /// draws the settings as the column of the left, with the profile at the
    /// top and one section per group of preferences. The sections keep the
    /// drawing on the left, the title in bold and the description in the
    /// muted color.
    fn render_settings(&self, cx: &mut Context<Self>) -> gpui::Div {
        let scale = self.ui_scale;
        let scale_percent = (self.ui_scale * 100.0).round() as u32;
        let logout_pending = self.rpc.logout_pending();
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let foreground = cx.theme().foreground;
        let secondary = cx.theme().secondary;
        let hover = cx.theme().muted;
        let accent = cx.theme().accent;
        let accent_foreground = cx.theme().accent_foreground;
        let view = cx.entity();

        // A section of the panel: the drawing on the left, the title in bold
        // and the description in the muted color, as the reference draws them.
        let section = |icon: WhatfastIcon,
                       heading: &'static str,
                       description: &'static str,
                       control: gpui::Div| {
            h_flex()
                .items_start()
                .gap_3()
                .child(
                    div()
                        .flex_none()
                        .w(px(26. * scale))
                        .pt_1()
                        .child(Icon::new(icon).with_size(px(22. * scale)).text_color(muted)),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(heading))
                        .child(div().text_sm().text_color(muted).child(description))
                        .child(control),
                )
        };

        // A row of a switch: the label at the left and the switch at the
        // right. The switch hands its value to one method of the app.
        let toggle =
            |id: &'static str,
             label: &'static str,
             checked: bool,
             disabled: bool,
             action: fn(&mut Whatfast, bool, &mut Context<Whatfast>)| {
                let view = view.clone();
                h_flex()
                    .justify_between()
                    .items_center()
                    .gap_3()
                    .py_1()
                    .child(div().text_sm().child(label))
                    .child(
                        Switch::new(id)
                            .checked(checked)
                            .disabled(disabled)
                            .on_click(move |value, _window, cx| {
                                let value = *value;
                                view.update(cx, |this, cx| action(this, value, cx));
                            }),
                    )
            };

        // The scale of the interface.
        let scale_control = v_flex()
            .gap_1()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("decrease-ui-scale")
                            .label("\u{2212}")
                            .disabled(self.ui_scale <= MIN_UI_SCALE)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.adjust_ui_scale(-UI_SCALE_STEP, window, cx);
                            })),
                    )
                    .child(
                        div()
                            .w(px(64.))
                            .text_center()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("{scale_percent}%")),
                    )
                    .child(
                        Button::new("increase-ui-scale")
                            .label("+")
                            .disabled(self.ui_scale >= MAX_UI_SCALE)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.adjust_ui_scale(UI_SCALE_STEP, window, cx);
                            })),
                    )
                    .child(
                        Button::new("reset-ui-scale")
                            .small()
                            .label(i18n::t("Reset"))
                            .disabled(self.ui_scale == 1.0)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.set_ui_scale(1.0, window, cx);
                            })),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(i18n::t("Saved automatically for future launches.")),
            );

        // The density: the two pills of the reference.
        let mut density_control = h_flex().gap_2();
        for (index, (density, label)) in [
            (Density::Comfortable, i18n::t("Comfortable")),
            (Density::Compact, i18n::t("Compact")),
        ]
        .into_iter()
        .enumerate()
        {
            let selected = self.density == density;
            density_control = density_control.child(
                div()
                    .id(("density", index))
                    .px_3()
                    .py_1()
                    .rounded_full()
                    .text_sm()
                    .cursor_pointer()
                    .when(selected, |this| {
                        this.bg(accent).text_color(accent_foreground)
                    })
                    .when(!selected, |this| this.bg(hover).text_color(foreground))
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_density(density, cx);
                    })),
            );
        }

        // The section of the interface: scale, density, layout and battery.
        let interface_control = v_flex()
            .gap_3()
            .mt_1()
            .child(scale_control)
            .child(density_control)
            .child(toggle(
                "split-view",
                i18n::t("Keep the chat list beside the conversation"),
                self.split_view,
                false,
                Whatfast::set_split_view,
            ))
            .child(toggle(
                "battery-saver",
                i18n::t("Battery saver"),
                self.battery_saver,
                false,
                Whatfast::set_battery_saver,
            ))
            .child(div().text_xs().text_color(muted).child(i18n::t(
                "The battery saver turns off the wallpaper and the smooth scrolling.",
            )));

        // The language: every embedded file is one row, with its name in its
        // own language.
        let mut language_rows = v_flex().gap_1().mt_1();
        for language in i18n::languages() {
            let active = i18n::current_language() == language.code;
            let code = language.code.to_string();
            language_rows = language_rows.child(
                h_flex()
                    .id(ElementId::Name(
                        format!("language-{}", language.code).into(),
                    ))
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .cursor_pointer()
                    .when(active, |this| this.bg(secondary))
                    .hover(move |this| this.bg(hover))
                    .child(
                        v_flex()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(language.name),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(language.code.to_uppercase()),
                            ),
                    )
                    .when(active, |this| {
                        this.child(
                            Icon::new(WhatfastIcon::Check)
                                .with_size(px(18. * scale))
                                .text_color(accent),
                        )
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_language(code.clone(), window, cx);
                        cx.stop_propagation();
                    })),
            );
        }

        // The chats: how the composer sends.
        let chats_control = v_flex()
            .mt_1()
            .child(toggle(
                "enter-to-send",
                i18n::t("Enter sends the message"),
                self.enter_to_send,
                false,
                Whatfast::set_enter_to_send,
            ))
            .child(div().text_xs().text_color(muted).child(i18n::t(
                "Shift+Enter adds a line. With this off, Ctrl+Enter sends.",
            )));

        // The notifications: the system toasts.
        let notifications_control = v_flex()
            .mt_1()
            .child(toggle(
                "notifications",
                i18n::t("Show notifications"),
                self.settings.notifications_enabled,
                false,
                Whatfast::set_notifications,
            ))
            .child(toggle(
                "notification-previews",
                i18n::t("Show a preview of the message"),
                self.settings.notification_previews,
                !self.settings.notifications_enabled,
                Whatfast::set_notification_previews,
            ));

        // The folder of "Save to folder".
        let download_text: SharedString = if self.settings.download_dir.is_empty() {
            i18n::t("Ask every time").into()
        } else {
            self.settings.download_dir.clone().into()
        };
        let storage_control = v_flex()
            .mt_1()
            .gap_2()
            .child(div().text_sm().text_color(muted).child(download_text))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("choose-download-dir")
                            .small()
                            .label(i18n::t("Choose folder"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.choose_download_dir(window, cx);
                            })),
                    )
                    .child(
                        Button::new("clear-download-dir")
                            .small()
                            .label(i18n::t("Ask every time"))
                            .disabled(self.settings.download_dir.is_empty())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_download_dir(None, cx);
                            })),
                    ),
            );

        // The health of the local database: check it, repair it, and see
        // the last answer. The repair rebuilds the derived counters, so a
        // badge stuck by an old build clears here without reopening chats.
        let integrity_line: SharedString = if self.integrity_checking {
            i18n::t("Checking...").into()
        } else if let Some(status) = self.store.integrity_status.as_ref() {
            if status.database_ok && status.inconsistent_chats == 0 {
                i18n::t("The local database is consistent.").into()
            } else {
                format!(
                    "{} {}",
                    status.inconsistent_chats,
                    i18n::t("chats need repair")
                )
                .into()
            }
        } else {
            SharedString::from("")
        };
        let storage_health_control = v_flex()
            .mt_1()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("check-storage-health")
                            .small()
                            .label(i18n::t("Check now"))
                            .disabled(self.integrity_checking)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.load_integrity_status();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("repair-storage")
                            .small()
                            .label(i18n::t("Repair"))
                            .disabled(self.integrity_checking)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.repair_local_cache();
                                cx.notify();
                            })),
                    ),
            )
            .when(!integrity_line.is_empty(), |this| {
                this.child(div().text_sm().text_color(muted).child(integrity_line))
            });

        // The space the media cache takes on this device. The numbers come
        // from the sidecar, which only counts the files of this computer:
        // clearing them never touches the account, the phone or a message.
        let usage = self.storage_usage.as_ref();
        let total_bytes = usage.map(|usage| usage.total_bytes).unwrap_or(0);
        let total_files = usage.map(|usage| usage.total_files).unwrap_or(0);
        let total_line: SharedString = match usage {
            None => i18n::t("Reading the space of the media...").into(),
            Some(_) if total_files == 0 => i18n::t("There are no downloaded files.").into(),
            Some(_) => {
                let size_text = attachments::format_bytes(total_bytes);
                let files_text = total_files.to_string();
                i18n::t_args(
                    "{size} in {files} files",
                    &[("size", size_text.as_str()), ("files", files_text.as_str())],
                )
                .into()
            }
        };
        let mut storage_media_control = v_flex()
            .mt_1()
            .gap_2()
            .child(div().text_sm().child(total_line));
        if let Some(usage) = usage {
            // One row per kind: the label of the kind and what it takes. A
            // kind with no file is not a row, so the panel stays short.
            for bucket in &usage.buckets {
                if bucket.files == 0 {
                    continue;
                }
                let label: SharedString = match bucket.kind.as_str() {
                    "photo" => i18n::t("Photos").into(),
                    "video" => i18n::t("Videos").into(),
                    "audio" => i18n::t("Audio").into(),
                    "document" => i18n::t("Documents").into(),
                    "avatar" => i18n::t("Avatars").into(),
                    _ => i18n::t("Other files").into(),
                };
                storage_media_control = storage_media_control.child(
                    h_flex()
                        .justify_between()
                        .gap_4()
                        .child(div().text_sm().text_color(muted).child(label))
                        .child(
                            div()
                                .text_sm()
                                .text_color(muted)
                                .child(attachments::format_bytes(bucket.bytes)),
                        ),
                );
            }
            if !usage.folder.is_empty() {
                storage_media_control = storage_media_control.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(usage.folder.clone()),
                );
            }
        }
        storage_media_control = storage_media_control.child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("refresh-media-cache")
                        .small()
                        .label(i18n::t("Refresh"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.load_storage_usage();
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("clear-media-cache")
                        .small()
                        .label(i18n::t("Clear media cache"))
                        .disabled(self.storage_clearing || total_files == 0)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.clear_media_cache();
                            cx.notify();
                        })),
                ),
        );

        // The catalog: the builtin themes and the ones the user made.
        let mut themes: Vec<AppTheme> = whatfast_core::theme::builtin_themes().to_vec();
        themes.extend(self.custom_themes.iter().cloned());
        let mut theme_rows = v_flex().gap_1().mt_1();
        for (index, theme) in themes.iter().enumerate() {
            let selected = theme.id == self.theme_id;
            let custom = theme.builtin != Some(true);
            let delete_id = theme.id.clone();
            let select_id = theme.id.clone();
            let appearance = match theme.appearance {
                ThemeAppearance::Dark => i18n::t("Dark"),
                ThemeAppearance::Light => i18n::t("Light"),
            };
            let row = h_flex()
                .id(("theme-row", index))
                .gap_3()
                .items_center()
                .px_2()
                .py_2()
                .rounded_md()
                .cursor_pointer()
                .when(selected, |this| this.bg(secondary))
                .hover(move |this| this.bg(hover))
                .child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .justify_center()
                        .w(px(46. * scale))
                        .h(px(30. * scale))
                        .rounded_md()
                        .bg(tokens::token_color(theme, ThemeToken::BgApp))
                        .border_1()
                        .border_color(tokens::token_color(theme, ThemeToken::Border))
                        .child(
                            div()
                                .size(px(11. * scale))
                                .rounded_full()
                                .bg(tokens::token_color(theme, ThemeToken::BubbleOutBg)),
                        )
                        .child(
                            div()
                                .size(px(11. * scale))
                                .rounded_full()
                                .bg(tokens::token_color(theme, ThemeToken::Accent)),
                        ),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .child(theme.name.clone()),
                        )
                        .child(div().text_xs().text_color(muted).child(appearance)),
                )
                .when(custom, |this| {
                    this.child(
                        div()
                            .id(("delete-theme", index))
                            .p_1()
                            .rounded_md()
                            .cursor_pointer()
                            .hover(move |style| style.bg(hover))
                            .child(
                                Icon::new(WhatfastIcon::Trash)
                                    .with_size(px(16. * scale))
                                    .text_color(muted),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.delete_custom_theme(&delete_id, window, cx);
                            })),
                    )
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.select_theme(select_id.clone(), window, cx);
                }));
            theme_rows = theme_rows.child(row);
        }

        // The editor of a custom theme: the accent of the palette.
        let mut accent_editor = v_flex().gap_2();
        if let Some(editing) = self.editing_theme.as_deref()
            && self.custom_themes.iter().any(|theme| theme.id == editing)
        {
            let id = editing.to_string();
            accent_editor = accent_editor
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(i18n::t("Accent color")),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .children(ACCENT_SWATCHES.iter().enumerate().map(|(index, color)| {
                            let id = id.clone();
                            let color = *color;
                            div()
                                .id(("accent-swatch", index))
                                .size(px(24. * scale))
                                .rounded_full()
                                .bg(rgb(color))
                                .border_1()
                                .border_color(border)
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.set_theme_accent(&id, color, window, cx);
                                }))
                        })),
                );
        }

        let themes_control = v_flex()
            .mt_1()
            .gap_2()
            .child(theme_rows)
            .child(
                h_flex().child(
                    Button::new("new-theme")
                        .small()
                        .label(i18n::t("New theme"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.create_custom_theme(window, cx);
                        })),
                ),
            )
            .child(accent_editor);

        // The account.
        let account_control = h_flex().mt_1().child(
            Button::new("logout")
                .small()
                .label(if logout_pending {
                    i18n::t("Clearing\u{2026}")
                } else if self.confirming_logout {
                    i18n::t("Confirm")
                } else {
                    i18n::t("Log out")
                })
                .disabled(logout_pending)
                .on_click(cx.listener(|this, _, _, cx| {
                    if this.confirming_logout {
                        this.request_logout();
                    } else {
                        this.confirming_logout = true;
                    }
                    cx.notify();
                })),
        );

        // The self-update: the version of this copy and the steps that
        // replace it. A check of the start only leaves a note when it finds
        // something; the panel is where the rest happens.
        let update_line = match self.update_phase {
            UpdatePhase::Idle => {
                i18n::t("The app asks the release for a newer version when it starts.").to_string()
            }
            UpdatePhase::Checking => i18n::t("Checking for updates\u{2026}").to_string(),
            UpdatePhase::UpToDate => i18n::t("Whatfast is up to date.").to_string(),
            UpdatePhase::Available => i18n::t_args(
                "Whatfast {version} is available.",
                &[("version", &self.update_version)],
            ),
            UpdatePhase::Downloading => {
                let received = self.update_received as f64 / (1024.0 * 1024.0);
                if self.update_total > 0 {
                    let total = self.update_total as f64 / (1024.0 * 1024.0);
                    i18n::t_args(
                        "Downloading {received} of {total} MB\u{2026}",
                        &[
                            ("received", &format!("{received:.1}")),
                            ("total", &format!("{total:.1}")),
                        ],
                    )
                } else {
                    i18n::t_args(
                        "Downloading {received} MB\u{2026}",
                        &[("received", &format!("{received:.1}"))],
                    )
                }
            }
            UpdatePhase::Ready => i18n::t_args(
                "Whatfast {version} is ready. Restart to install it.",
                &[("version", &self.update_version)],
            ),
            UpdatePhase::Installing => i18n::t("Installing the update\u{2026}").to_string(),
            UpdatePhase::Unsupported => {
                i18n::t("This platform has no package for the automatic update.").to_string()
            }
            UpdatePhase::Failed => i18n::t_args(
                "The update failed: {error}",
                &[("error", &self.update_error)],
            ),
        };
        let mut update_buttons = h_flex().gap_2();
        match self.update_phase {
            UpdatePhase::Available => {
                update_buttons = update_buttons.child(
                    Button::new("download-update")
                        .small()
                        .label(i18n::t("Download"))
                        .on_click(cx.listener(|this, _, _, cx| this.start_update_download(cx))),
                );
            }
            UpdatePhase::Ready => {
                update_buttons = update_buttons.child(
                    Button::new("install-update")
                        .small()
                        .label(i18n::t("Restart and install"))
                        .on_click(cx.listener(|this, _, _, cx| this.install_update(cx))),
                );
            }
            UpdatePhase::Checking | UpdatePhase::Downloading | UpdatePhase::Installing => {
                update_buttons = update_buttons.child(
                    Button::new("update-busy")
                        .small()
                        .label(i18n::t("Please wait\u{2026}"))
                        .disabled(true),
                );
            }
            _ => {
                update_buttons = update_buttons.child(
                    Button::new("check-updates")
                        .small()
                        .label(i18n::t("Check now"))
                        .on_click(cx.listener(|this, _, _, cx| this.start_update_check(false, cx))),
                );
            }
        }
        let update_control = v_flex()
            .mt_1()
            .gap_2()
            .child(div().text_sm().child(update_line))
            .child(div().text_xs().text_color(muted).child(i18n::t(
                "The update replaces the two executables and restarts Whatfast.",
            )))
            .child(update_buttons)
            .child(toggle(
                "check-updates",
                i18n::t("Check for updates when Whatfast starts"),
                self.check_updates,
                false,
                Whatfast::set_check_updates,
            ));

        // The about: the version and the privacy line.
        let about_control = v_flex()
            .mt_1()
            .gap_1()
            .child(
                div()
                    .text_sm()
                    .child(format!("Whatfast {}", env!("CARGO_PKG_VERSION"))),
            )
            .child(div().text_xs().text_color(muted).child(i18n::t(
                "The messages stay on this device and the credentials never enter the interface.",
            )));

        div()
            .absolute()
            // The column of the settings sits next to the column of icons,
            // the same way the reference draws it.
            .left(px(56. * scale))
            .right_0()
            .top_0()
            .bottom_0()
            // port: el panel de la referencia no oscurece el resto de la
            // ventana: la conversacion sigue visible al lado.
            // Without occlusion the clicks fall through to the conversation
            // below, which is behind the panel.
            .occlude()
            .flex()
            .justify_start()
            .child(
                v_flex()
                    .w(px(420. * scale))
                    .max_w_full()
                    .h_full()
                    .border_r_1()
                    .border_color(border)
                    .bg(cx.theme().background)
                    .child(
                        h_flex()
                            .px_5()
                            .pt_5()
                            .pb_3()
                            .gap_3()
                            .items_center()
                            .child(
                                div()
                                    .size(px(52. * scale))
                                    .rounded_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .bg(secondary)
                                    .child(
                                        Icon::new(IconName::User)
                                            .with_size(px(26. * scale))
                                            .text_color(muted),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .text_size(px(20. * scale))
                                            .font_weight(FontWeight::BOLD)
                                            .child("Whatfast"),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(muted)
                                            .child(i18n::t("Local profile")),
                                    ),
                            )
                            .child(Button::new("close-settings").label("\u{00d7}").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.settings_open = false;
                                    cx.notify();
                                }),
                            )),
                    )
                    .child(
                        v_flex()
                            .id("settings-sections")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .track_scroll(&self.settings_scroll)
                            .on_scroll_wheel(cx.listener(|this, event, window, cx| {
                                this.handle_smooth_scroll_input(
                                    ScrollSurface::Settings,
                                    event,
                                    window,
                                    cx,
                                );
                            }))
                            .px_5()
                            .pt_3()
                            .pb_6()
                            .gap_6()
                            .child(section(
                                WhatfastIcon::Globe,
                                i18n::t("Language"),
                                i18n::t("The language of the interface."),
                                language_rows,
                            ))
                            .child(section(
                                WhatfastIcon::Settings,
                                i18n::t("Interface"),
                                i18n::t("Scale, spacing and the layout of the window."),
                                interface_control,
                            ))
                            .child(section(
                                WhatfastIcon::Messages,
                                i18n::t("Chats"),
                                i18n::t("How the composer sends a message."),
                                chats_control,
                            ))
                            .child(section(
                                WhatfastIcon::Bell,
                                i18n::t("Notifications"),
                                i18n::t("Toasts when a message arrives in the background."),
                                notifications_control,
                            ))
                            .child(section(
                                WhatfastIcon::Download,
                                i18n::t("Downloads"),
                                i18n::t("The folder of \"Save to folder\"."),
                                storage_control,
                            ))
                            .child(section(
                                WhatfastIcon::Storage,
                                i18n::t("Media cache"),
                                i18n::t(
                                    "The photos, videos, audio and documents this device downloaded.",
                                ),
                                storage_media_control,
                            ))
                            .child(section(
                                WhatfastIcon::CheckDouble,
                                i18n::t("Storage health"),
                                i18n::t(
                                    "Check the local database and repair unread counters if needed.",
                                ),
                                storage_health_control,
                            ))
                            .child(section(
                                WhatfastIcon::Palette,
                                i18n::t("Themes"),
                                i18n::t("The palette of the app and the themes you made."),
                                themes_control,
                            ))
                            .child(section(
                                WhatfastIcon::Warning,
                                i18n::t("Account"),
                                i18n::t("Sign out of this device and clear the local data."),
                                account_control,
                            ))
                            .child(section(
                                WhatfastIcon::Download,
                                i18n::t("Updates"),
                                i18n::t("The version of Whatfast and its updates."),
                                update_control,
                            ))
                            .child(section(
                                WhatfastIcon::Shield,
                                i18n::t("About"),
                                i18n::t("Whatfast keeps the messages on this device."),
                                about_control,
                            )),
                    ),
            )
    }

    /// The panel with the information of the contact or of the group.
    ///
    /// The style is the one of WhatsApp Desktop: the photo big and centered
    /// at the top, the name under it and then the rows of the information,
    /// each one with the title in bold and the value in the muted color.
    fn render_chat_info(&mut self, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let Some(state) = self.chat_info.as_ref() else {
            return div().id("chat-info-backdrop");
        };
        let chat_id = state.chat_id.clone();
        let loading = state.loading;
        let error = state.error.clone();
        let info = state.info.clone();
        let chat = self.store.chat(&chat_id).cloned();
        let is_group = chat
            .as_ref()
            .is_some_and(|chat| chat.kind() == proto::ChatKind::Group);
        let title = chat
            .as_ref()
            .map(|chat| chat.title.clone())
            .unwrap_or_else(|| i18n::t("Conversation").into());
        let avatar_path = chat
            .as_ref()
            .map(|chat| chat.avatar_path.clone())
            .unwrap_or_default();
        let scale = self.ui_scale;

        // Warm the avatars for the members that are about to be shown; the
        // media budget in the RPC layer keeps this from flooding the backend.
        if let Some(info) = info.as_ref() {
            let pending: Vec<String> = info
                .participants
                .iter()
                .take(24)
                .filter(|participant| {
                    self.store
                        .participant_avatar_path(&participant.participant_id)
                        .is_none()
                })
                .map(|participant| participant.participant_id.clone())
                .collect();
            for participant_id in pending {
                self.load_participant_avatar(participant_id);
            }
        }

        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let hover_background = cx.theme().muted;
        // A row of the panel: the title in bold and the value in the muted
        // color, the same way the reference draws the sections.
        let section = |label: &'static str, value: String| {
            v_flex()
                .gap_1()
                .child(div().font_weight(FontWeight::SEMIBOLD).child(label))
                .child(div().text_sm().text_color(muted).child(value))
        };

        let subtitle = if let Some(info) = info.as_ref().filter(|_| is_group) {
            format!(
                "{} · {} {}",
                i18n::t("Group"),
                info.participant_count,
                i18n::t("members")
            )
        } else if is_group {
            i18n::t("Group").to_string()
        } else {
            chat.as_ref()
                .map(|chat| whatfast_core::phone::format(&chat.phone_number))
                .unwrap_or_default()
        };
        let identity_avatar = avatar_element(&title, &avatar_path, px(140. * scale), cx);

        let mut body = v_flex()
            .id("chat-info-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.chat_info_scroll)
            .on_scroll_wheel(cx.listener(|this, event, window, cx| {
                this.handle_smooth_scroll_input(ScrollSurface::ChatInfo, event, window, cx);
            }))
            .px_5()
            .pt_5()
            .pb_6()
            .gap_5()
            // The photo, the name and the subtitle, in the middle.
            .child(
                v_flex()
                    .items_center()
                    .gap_2()
                    .pb_2()
                    .child(identity_avatar)
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_center()
                            .child(title.clone()),
                    )
                    .when(!subtitle.is_empty(), |identity| {
                        identity.child(
                            div()
                                .text_sm()
                                .text_color(muted)
                                .text_center()
                                .child(subtitle),
                        )
                    }),
            );
        if loading {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(muted)
                    .text_center()
                    .child(i18n::t("Loading chat details…")),
            );
        }
        if let Some(error) = error {
            body = body.child(
                v_flex()
                    .items_center()
                    .gap_2()
                    .child(div().text_sm().text_color(rgb(0xef4444)).child(error))
                    .child(
                        Button::new("chat-info-retry")
                            .small()
                            .label(i18n::t("Retry"))
                            .on_click(cx.listener({
                                let chat_id = chat_id.clone();
                                move |this, _, _, cx| {
                                    this.open_chat_info(chat_id.clone());
                                    cx.notify();
                                }
                            })),
                    ),
            );
        }
        if let Some(info) = info {
            if !is_group {
                if !info.about.is_empty() {
                    body = body.child(section(i18n::t("About"), info.about.clone()));
                }
                if let Some(chat) = chat.as_ref() {
                    if !chat.phone_number.is_empty() {
                        body = body.child(section(
                            i18n::t("Phone"),
                            whatfast_core::phone::format(&chat.phone_number),
                        ));
                    }
                    if !info.verified_name.is_empty() {
                        body = body.child(section(
                            i18n::t("Verified business"),
                            info.verified_name.clone(),
                        ));
                    }
                    if !chat.business_name.is_empty() && chat.business_name != chat.title {
                        body = body.child(section(
                            i18n::t("Business name"),
                            chat.business_name.clone(),
                        ));
                    }
                    if !chat.push_name.is_empty() && chat.push_name != chat.title {
                        body = body.child(section(
                            i18n::t("Profile name"),
                            format!("~{}", chat.push_name),
                        ));
                    }
                }
            } else {
                let description = if info.description.is_empty() {
                    i18n::t("No description").to_string()
                } else {
                    info.description.clone()
                };
                body = body.child(section(i18n::t("Description"), description));
                if info.created_at_ms > 0 {
                    let mut created = format!(
                        "{} {}",
                        i18n::t("Created"),
                        format_epoch_date(info.created_at_ms)
                    );
                    if !info.created_by.is_empty() {
                        created.push_str(&format!(" {} {}", i18n::t("by"), info.created_by));
                    }
                    body = body.child(div().text_xs().text_color(muted).child(created));
                }
                if info.disappearing_timer_seconds > 0 {
                    body = body.child(section(
                        i18n::t("Disappearing messages"),
                        format_disappearing_timer(info.disappearing_timer_seconds),
                    ));
                }
                let mut notes: Vec<&'static str> = Vec::new();
                if info.is_community {
                    notes.push(i18n::t("Community group"));
                }
                if info.announce_only {
                    notes.push(i18n::t("Only admins can send messages"));
                }
                if info.locked {
                    notes.push(i18n::t("Only admins can edit group info"));
                }
                if info.join_approval_required {
                    notes.push(i18n::t("Admins approve new members"));
                }
                if !notes.is_empty() {
                    body =
                        body.child(v_flex().gap_1().children(notes.into_iter().map(|note| {
                            div().text_xs().text_color(muted).child(format!("• {note}"))
                        })));
                }
                if !info.participants.is_empty() {
                    const MAX_MEMBER_ROWS: usize = 200;
                    let mut members = v_flex().gap_1().child(
                        div().font_weight(FontWeight::SEMIBOLD).child(format!(
                            "{} {}",
                            info.participant_count,
                            i18n::t("members")
                        )),
                    );
                    for (index, participant) in
                        info.participants.iter().take(MAX_MEMBER_ROWS).enumerate()
                    {
                        let display_name = if participant.is_me {
                            i18n::t("You").to_string()
                        } else if participant.display_name.is_empty() {
                            whatfast_core::phone::format(&participant.phone_number)
                        } else {
                            participant.display_name.clone()
                        };
                        let member_avatar_path = self
                            .store
                            .participant_avatar_path(&participant.participant_id)
                            .filter(|path| !path.is_empty())
                            .unwrap_or_default();
                        let member_avatar =
                            avatar_element(&display_name, member_avatar_path, px(32.), cx);
                        let badge = if participant.is_super_admin {
                            Some(i18n::t("Owner"))
                        } else if participant.is_admin {
                            Some(i18n::t("Admin"))
                        } else {
                            None
                        };
                        let clickable =
                            !participant.is_me && !participant.participant_id.is_empty();
                        let mut row = h_flex()
                            .id(index)
                            .px_2()
                            .py_1()
                            .gap_2()
                            .items_center()
                            .rounded_md()
                            .child(member_avatar)
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(div().text_sm().truncate().child(display_name))
                                    .when(
                                        !participant.phone_number.is_empty() && !participant.is_me,
                                        |details| {
                                            details.child(
                                                div().text_xs().text_color(muted).truncate().child(
                                                    whatfast_core::phone::format(
                                                        &participant.phone_number,
                                                    ),
                                                ),
                                            )
                                        },
                                    ),
                            )
                            .when_some(badge, |row, badge| {
                                row.child(
                                    div()
                                        .px_2()
                                        .py_0p5()
                                        .rounded_sm()
                                        .border_1()
                                        .border_color(border)
                                        .text_xs()
                                        .text_color(muted)
                                        .child(badge),
                                )
                            });
                        if clickable {
                            row = row
                                .cursor_pointer()
                                .hover(move |style| style.bg(hover_background))
                                .on_click(cx.listener({
                                    let contact_jid = participant.participant_id.clone();
                                    move |this, _, _, cx| {
                                        this.chat_info = None;
                                        this.request(
                                            proto::rpc_request::Request::OpenContact(
                                                proto::OpenContactRequest {
                                                    contact_jid: contact_jid.clone(),
                                                },
                                            ),
                                            PendingRequest::OpenContact,
                                        );
                                        cx.notify();
                                    }
                                }));
                        }
                        members = members.child(row);
                    }
                    if info.participants.len() > MAX_MEMBER_ROWS {
                        members = members.child(div().text_xs().text_color(muted).child(format!(
                            "…and {} more",
                            info.participants.len() - MAX_MEMBER_ROWS
                        )));
                    }
                    body = body.child(members);
                }
            }
            let mut flags = Vec::new();
            if let Some(chat) = chat.as_ref() {
                if chat.pinned {
                    flags.push("Pinned");
                }
                if chat.muted {
                    flags.push("Muted");
                }
                if chat.archived {
                    flags.push("Archived");
                }
            }
            if !flags.is_empty() {
                body = body.child(h_flex().gap_2().children(flags.into_iter().map(|flag| {
                    div()
                        .px_2()
                        .py_0p5()
                        .rounded_sm()
                        .border_1()
                        .border_color(border)
                        .text_xs()
                        .text_color(muted)
                        .child(flag)
                })));
            }
            // The favorite is a mark of the profile: the button toggles it and
            // the column of the chats shows the conversations that carry it.
            if let Some(chat) = chat.as_ref() {
                let is_favorite = self.store.is_favorite(&chat.id);
                let chat_id = chat.id.clone();
                body = body.child(
                    Button::new("chat-favorite")
                        .small()
                        .icon(if is_favorite {
                            IconName::StarFill
                        } else {
                            IconName::Star
                        })
                        .label(if is_favorite {
                            i18n::t("Remove from favorites")
                        } else {
                            i18n::t("Add to favorites")
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_favorite(chat_id.clone(), cx);
                            cx.notify();
                        })),
                );
            }
            if !info.address.is_empty() {
                body = body.child(section(i18n::t("WhatsApp address"), info.address.clone()));
            }
        }

        div()
            .id("chat-info-backdrop")
            .absolute()
            .inset_0()
            // Without occlusion, clicks fall through to the conversation
            // header below, whose click target immediately reopens the pane.
            .occlude()
            .bg(rgba(0x00000066))
            .flex()
            .justify_end()
            .on_click(cx.listener(|this, _, _, cx| {
                this.chat_info = None;
                cx.notify();
            }))
            .child(
                v_flex()
                    .w(px(400. * scale))
                    .max_w_full()
                    .h_full()
                    // Occlude the backdrop so clicks inside the pane don't hit
                    // its close-on-click handler.
                    .occlude()
                    .bg(cx.theme().background)
                    .border_l_1()
                    .border_color(border)
                    .child(
                        h_flex()
                            .h(px(48. * scale))
                            .px_5()
                            .items_center()
                            .justify_between()
                            .border_b_1()
                            .border_color(border)
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(if is_group {
                                i18n::t("Group info")
                            } else {
                                i18n::t("Contact info")
                            }))
                            .child(Button::new("close-chat-info").label("×").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.chat_info = None;
                                    cx.notify();
                                }),
                            )),
                    )
                    .child(body),
            )
    }

    fn render_chat_switcher(&self, cx: &mut Context<Self>) -> gpui::Div {
        let Some(switcher) = self.chat_switcher.as_ref() else {
            return div();
        };
        let highlighted = switcher.highlighted;
        let chats = switcher
            .chat_ids
            .iter()
            .filter_map(|chat_id| self.store.chat(chat_id).cloned())
            .collect::<Vec<_>>();
        let selected_background = if cx.theme().is_dark() {
            rgb(0x173f35)
        } else {
            rgb(0xd9fdd3)
        };
        let hover_background = if cx.theme().is_dark() {
            rgb(0x202c33)
        } else {
            rgb(0xf0f2f5)
        };
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .bg(rgba(0x00000088))
            .child(
                v_flex()
                    .w(px(500. * self.ui_scale))
                    .max_w_full()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .overflow_hidden()
                    .child(
                        h_flex()
                            .h(px(56. * self.ui_scale))
                            .px_4()
                            .items_center()
                            .justify_between()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(i18n::t("Recent chats")),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!("{} of {}", highlighted + 1, chats.len())),
                            ),
                    )
                    .children(chats.into_iter().enumerate().map(|(index, chat)| {
                        let chat_id = chat.id.clone();
                        let preview = if chat.last_message_preview.trim().is_empty() {
                            if chat.phone_number.trim().is_empty() {
                                "No messages yet".to_string()
                            } else {
                                whatfast_core::phone::format(&chat.phone_number)
                            }
                        } else {
                            chat_preview(&chat.last_message_preview)
                        };
                        let avatar = avatar_element(
                            &chat.title,
                            &chat.avatar_path,
                            px(40. * self.ui_scale),
                            cx,
                        );
                        h_flex()
                            .id(("recent-chat", index))
                            .h(px(64. * self.ui_scale))
                            .px_4()
                            .gap_3()
                            .items_center()
                            .overflow_hidden()
                            .cursor_pointer()
                            .when(index == highlighted, |row| row.bg(selected_background))
                            .hover(move |style| style.bg(hover_background))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.commit_chat_switcher_to(chat_id.clone(), window, cx);
                                cx.notify();
                            }))
                            .child(avatar)
                            .child(
                                v_flex()
                                    .min_w_0()
                                    .flex_1()
                                    .child(
                                        div()
                                            .w_full()
                                            .min_w_0()
                                            .truncate()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(one_line(&chat.title)),
                                    )
                                    .child(
                                        div()
                                            .w_full()
                                            .min_w_0()
                                            .truncate()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(one_line(&preview)),
                                    ),
                            )
                            .when(chat.archived, |row| {
                                row.child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(i18n::t("Archived")),
                                )
                            })
                    }))
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .border_t_1()
                            .border_color(cx.theme().border)
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(i18n::t("Release Ctrl to switch · Esc to cancel")),
                    ),
            )
    }

    /// La foto a pantalla completa, como la abre WhatsApp: el encabezado con
    /// quien la mando y la fecha, la tira de iconos, las flechas y el
    /// carrusel de miniaturas al pie. El fondo tapa la ventana entera y es
    /// negro neutro para no alterar el color de la foto.
    fn render_image_viewer(&self, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let Some(viewer) = self.image_viewer.as_ref() else {
            return div().into_any_element();
        };
        let scale = self.ui_scale;
        let chat_id = viewer.chat_id.clone();
        let message_id = viewer.message_id.clone();
        let path = viewer.path.clone();
        let caption = viewer.caption.trim().to_string();
        let sticker = viewer.sticker;
        let index = viewer.index;
        let total = viewer.gallery.len();
        let (sender_name, sender_avatar, timestamp_ms) = viewer
            .current()
            .map(|item| {
                (
                    item.sender_name.clone(),
                    item.sender_avatar.clone(),
                    item.timestamp_ms,
                )
            })
            .unwrap_or_default();
        let sender_name = if sender_name.trim().is_empty() {
            i18n::t("Unknown contact").to_string()
        } else {
            sender_name
        };
        let offset = local_offset_minutes(timestamp_ms);
        let day = format_epoch_date_numeric(timestamp_ms);
        let time = bubble_time_label(timestamp_ms, offset);
        let date_label = i18n::t_args("{day} at {time}", &[("day", &day), ("time", &time)]);
        let starred = self.store.message_is_starred(&chat_id, &message_id);
        let pinned = self.store.message_is_pinned(&chat_id, &message_id);
        let opening_label = if sticker {
            i18n::t("Opening the sticker…")
        } else {
            i18n::t("Opening the photo…")
        };
        let fallback_label = if sticker {
            i18n::t("This sticker couldn't be decoded. Try Reload.")
        } else {
            i18n::t("This photo couldn't be decoded. Try Reload.")
        };
        let viewport = window.viewport_size();
        let header_height = px(64. * scale);
        let footer_height = if total > 1 { px(112. * scale) } else { px(0.) };
        let reactions_height = if self.viewer_reactions_open {
            px(48. * scale)
        } else {
            px(0.)
        };
        let caption_height = if caption.is_empty() {
            px(0.)
        } else {
            px(56. * scale)
        };
        // La caja de la foto descuenta las barras y los botones laterales: la
        // foto se mide en pixeles, no en porcentajes anidados, que es lo que
        // pintaba la imagen mas grande que su caja.
        let available_width = (viewport.width - px(160. * scale)).max(px(120.));
        let available_height = (viewport.height
            - header_height
            - footer_height
            - reactions_height
            - caption_height
            - px(48. * scale))
        .max(px(120.));
        let (fit_width, fit_height) =
            crate::helpers::viewer_picture_size(viewer, available_width, available_height);
        let picture_width = fit_width * viewer.zoom;
        let picture_height = fit_height * viewer.zoom;

        // El encabezado: la foto es de alguien y de una fecha, no de un
        // "Foto" generico.
        let header = h_flex()
            .flex_none()
            .h(header_height)
            .w_full()
            .px(px(16. * scale))
            .items_center()
            .justify_between()
            .gap_4()
            .bg(rgb(VIEWER_BAR))
            .child(
                h_flex()
                    .min_w_0()
                    .items_center()
                    .gap_3()
                    .child(avatar_element(
                        &sender_name,
                        &sender_avatar,
                        px(40. * scale),
                        cx,
                    ))
                    .child(
                        v_flex()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .truncate()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(0xffffff))
                                    .child(sender_name.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_xs()
                                    .text_color(rgb(VIEWER_MUTED))
                                    .child(date_label.clone()),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .child(
                        viewer_icon_button(
                            ("viewer-zoom-out", index),
                            Icon::new(IconName::Minus),
                            scale,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.zoom_image_viewer(0.8);
                            cx.notify();
                        })),
                    )
                    .child(
                        viewer_icon_button(
                            ("viewer-zoom-in", index),
                            Icon::new(IconName::Plus),
                            scale,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.zoom_image_viewer(1.25);
                            cx.notify();
                        })),
                    )
                    .child(
                        viewer_icon_button(
                            ("viewer-reply", index),
                            Icon::new(WhatfastIcon::Reply),
                            scale,
                        )
                        .on_click(cx.listener({
                            let message_id = message_id.clone();
                            move |this, _, _, cx| {
                                this.start_reply(message_id.clone());
                                this.image_viewer = None;
                                cx.notify();
                            }
                        })),
                    )
                    .child(
                        viewer_icon_button(
                            ("viewer-star", index),
                            Icon::new(if starred {
                                IconName::StarFill
                            } else {
                                IconName::Star
                            }),
                            scale,
                        )
                        .on_click(cx.listener({
                            let chat_id = chat_id.clone();
                            let message_id = message_id.clone();
                            move |this, _, _, cx| {
                                this.set_message_star(
                                    chat_id.clone(),
                                    message_id.clone(),
                                    !starred,
                                    cx,
                                );
                                cx.notify();
                            }
                        })),
                    )
                    .child(
                        viewer_icon_button(
                            ("viewer-pin", index),
                            Icon::new(WhatfastIcon::Pin),
                            scale,
                        )
                        .on_click(cx.listener({
                            let message_id = message_id.clone();
                            move |this, _, _, cx| {
                                this.set_message_pin(message_id.clone(), !pinned);
                                cx.notify();
                            }
                        })),
                    )
                    .child(
                        viewer_icon_button(
                            ("viewer-react", index),
                            Icon::new(WhatfastIcon::Smile),
                            scale,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.viewer_reactions_open = !this.viewer_reactions_open;
                            cx.notify();
                        })),
                    )
                    .child(
                        viewer_icon_button(
                            ("viewer-delete", index),
                            Icon::new(WhatfastIcon::Trash),
                            scale,
                        )
                        .on_click(cx.listener({
                            let message_id = message_id.clone();
                            move |this, _, _, cx| {
                                this.delete_message(message_id.clone(), false);
                                this.image_viewer = None;
                                cx.notify();
                            }
                        })),
                    )
                    .child(
                        viewer_icon_button(
                            ("viewer-forward", index),
                            Icon::new(WhatfastIcon::Forward),
                            scale,
                        )
                        .on_click(cx.listener({
                            let chat_id = chat_id.clone();
                            let message_id = message_id.clone();
                            move |this, _, window, cx| {
                                this.open_forward_picker(
                                    chat_id.clone(),
                                    message_id.clone(),
                                    window,
                                    cx,
                                );
                                cx.notify();
                            }
                        })),
                    )
                    .child(
                        viewer_icon_button(
                            ("viewer-more", index),
                            Icon::new(IconName::EllipsisVertical),
                            scale,
                        )
                        .on_click(cx.listener(
                            move |this, event: &ClickEvent, window, cx| {
                                let position = event
                                    .mouse_position()
                                    .unwrap_or_else(|| window.mouse_position());
                                this.image_context_menu =
                                    Some(crate::app::ImageContextMenu { position });
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        viewer_icon_button(
                            ("viewer-close", index),
                            Icon::new(WhatfastIcon::Close),
                            scale,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.image_viewer = None;
                            cx.notify();
                        })),
                    ),
            );

        // La fila de reacciones rapidas, que abre el boton de la cara.
        let reactions = h_flex()
            .flex_none()
            .h(reactions_height)
            .w_full()
            .items_center()
            .justify_center()
            .gap_1()
            .bg(rgb(VIEWER_BAR))
            .children(
                VIEWER_QUICK_REACTIONS
                    .iter()
                    .enumerate()
                    .map(|(position, emoji)| {
                        let emoji = *emoji;
                        div()
                            .id(("viewer-reaction", position))
                            .size(px(34. * scale))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_lg()
                            .cursor_pointer()
                            .hover(|style| style.bg(rgba(0xffffff1f)))
                            .child(emoji)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.react_to_viewed_image(emoji);
                                cx.notify();
                            }))
                    }),
            );

        let picture = img(PathBuf::from(path.clone()))
            .w(picture_width)
            .h(picture_height)
            .object_fit(ObjectFit::Contain)
            .with_loading(move || {
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .text_color(rgb(VIEWER_MUTED))
                    .child(opening_label)
                    .into_any_element()
            })
            .with_fallback(move || {
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .px_4()
                    .text_sm()
                    .text_color(rgb(VIEWER_MUTED))
                    .child(fallback_label)
                    .into_any_element()
            });

        let picture_column = v_flex()
            .id(("viewer-picture-column", index))
            .flex_1()
            .min_w_0()
            .h_full()
            .items_center()
            .justify_center()
            .gap_3()
            .child(
                div()
                    .id(("viewer-image", index))
                    .overflow_hidden()
                    .flex()
                    .items_center()
                    .justify_center()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(picture)
                    .on_aux_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        if !event.is_secondary() {
                            return;
                        }
                        let position = event
                            .mouse_position()
                            .unwrap_or_else(|| window.mouse_position());
                        this.image_context_menu = Some(crate::app::ImageContextMenu { position });
                        cx.stop_propagation();
                        cx.notify();
                    })),
            )
            .when(!caption.is_empty(), |column| {
                column.child(
                    div()
                        .max_w(px(640. * scale))
                        .px_6()
                        .text_sm()
                        .text_color(rgb(0xe9edef))
                        .child(caption.clone()),
                )
            })
            .on_click(|_, _, cx| cx.stop_propagation());

        let body = h_flex()
            .id("viewer-body")
            .flex_1()
            .min_h_0()
            .w_full()
            .items_center()
            .justify_center()
            // El fondo que rodea a la foto la cierra con un clic, como la
            // referencia; los botones y la foto cortan la propagacion.
            .on_click(cx.listener(|this, _, _, cx| {
                this.image_viewer = None;
                cx.notify();
            }))
            .child(
                div()
                    .w(px(72. * scale))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(viewer.has_previous(), |slot| {
                        slot.child(
                            viewer_arrow_button(("viewer-previous", index), true, scale).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.step_image_viewer(false);
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            ),
                        )
                    }),
            )
            .child(picture_column)
            .child(
                div()
                    .w(px(72. * scale))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(viewer.has_next(), |slot| {
                        slot.child(
                            viewer_arrow_button(("viewer-next", index), false, scale).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.step_image_viewer(true);
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            ),
                        )
                    }),
            );

        // El pie: la posicion de la foto y la tira de miniaturas, con la
        // activa marcada en verde.
        let current_label = (index + 1).to_string();
        let total_label = total.to_string();
        let footer = v_flex()
            .flex_none()
            .w_full()
            .items_center()
            .gap_2()
            .pt(px(8. * scale))
            .pb(px(12. * scale))
            .bg(rgb(VIEWER_BAR))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(VIEWER_MUTED))
                    .child(i18n::t_args(
                        "{current} of {total}",
                        &[("current", &current_label), ("total", &total_label)],
                    )),
            )
            .child(
                // La tira se centra cuando entra en la ventana y se desplaza
                // de lado cuando la conversacion tiene muchas fotos.
                div()
                    .id("viewer-thumbs")
                    .w_full()
                    .overflow_x_scroll()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .px(px(16. * scale))
                            .when(total <= 10, |strip| strip.w_full().justify_center())
                            .children(viewer.gallery.iter().enumerate().map(|(position, item)| {
                                let thumb_path = item.path.clone();
                                div()
                                    .id(("viewer-thumb", position))
                                    .size(px(56. * scale))
                                    .flex_none()
                                    .rounded_md()
                                    .overflow_hidden()
                                    .border_2()
                                    .border_color(if position == index {
                                        rgb(VIEWER_ACCENT)
                                    } else {
                                        rgba(0xffffff14)
                                    })
                                    .cursor_pointer()
                                    .hover(|style| style.border_color(rgb(0x8696a0)))
                                    .child(
                                        img(PathBuf::from(thumb_path))
                                            .size_full()
                                            .object_fit(ObjectFit::Cover),
                                    )
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(viewer) = this.image_viewer.as_mut() {
                                            viewer.show(position);
                                        }
                                        cx.stop_propagation();
                                        cx.notify();
                                    }))
                            })),
                    ),
            );

        // La tarjeta de la info del mensaje: quien mando la foto, cuando y
        // en que estado esta.
        let status = self
            .store
            .message(&message_id)
            .map(|message| message.status)
            .unwrap_or_default();
        let info_card = div()
            .id("viewer-info-backdrop")
            .absolute()
            .inset_0()
            .bg(rgba(0x000000b8))
            .flex()
            .items_center()
            .justify_center()
            .on_click(cx.listener(|this, _, _, cx| {
                this.viewer_info_open = false;
                cx.stop_propagation();
                cx.notify();
            }))
            .child(
                v_flex()
                    .id("viewer-info-card")
                    .w(px(340. * scale))
                    .rounded_lg()
                    .bg(rgb(0x1f2c33))
                    .p(px(20. * scale))
                    .gap_3()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(i18n::t("Message info")),
                            )
                            .child(
                                div()
                                    .id("viewer-info-close")
                                    .cursor_pointer()
                                    .child(
                                        Icon::new(WhatfastIcon::Close)
                                            .with_size(px(18. * scale))
                                            .text_color(rgb(0xd1d7db)),
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.viewer_info_open = false;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(viewer_info_row(i18n::t("Sender"), sender_name.clone()))
                    .child(viewer_info_row(i18n::t("Date"), date_label.to_string()))
                    .child(viewer_info_row(
                        i18n::t("Status"),
                        viewer_status_label(status).to_string(),
                    )),
            );

        div()
            .id("image-viewer-backdrop")
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .flex_col()
            .bg(rgb(VIEWER_BACKGROUND))
            .text_color(rgb(0xffffff))
            .child(header)
            .when(self.viewer_reactions_open, |overlay| {
                overlay.child(reactions)
            })
            .child(body)
            .when(total > 1, |overlay| overlay.child(footer))
            .when(self.viewer_info_open, |overlay| overlay.child(info_card))
            .into_any_element()
    }

    /// El visor del video: el cuadro ocupa la ventana y los controles van
    /// abajo, sobre el mismo fondo oscuro de WhatsApp.
    ///
    /// El reproductor es el de la burbuja: el mismo hilo decodifica, el mismo
    /// estado dice si corre y el mismo boton del centro pausa. El visor solo
    /// le da mas lugar al cuadro.
    fn render_video_viewer(&self, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let Some(key) = self.video.key.clone().filter(|_| self.video_fullscreen) else {
            return div().into_any_element();
        };
        let Some(attachment) = self.video_attachment(&key.1) else {
            return div().into_any_element();
        };
        let scale = self.ui_scale;
        let viewport = window.viewport_size();
        let (width, height) = crate::video::viewer_box_size(
            attachment.width,
            attachment.height,
            (viewport.width - px(120. * scale)).max(px(320.)).into(),
            (viewport.height - px(140. * scale)).max(px(180.)).into(),
        );
        let frame = self.video.frame.clone();
        let has_frame = frame.is_some();
        let playing = self.video.is_running();
        let waiting = self.video.waiting;
        let silent = self.video.muted || self.video.volume <= 0.0;
        let position = self.video.position;
        let duration = self
            .video
            .duration
            .max(0.0)
            .max(attachment.duration_seconds as f32);
        let progress = if duration > 0.0 {
            (position / duration).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let title = if attachment.file_name.trim().is_empty() {
            i18n::t("Video").to_string()
        } else {
            attachment.file_name.clone()
        };
        let error = self.video.error.clone();
        let play_chat_id = key.0.clone();
        let play_message_id = key.1.clone();

        div()
            .id("video-viewer-backdrop")
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .flex_col()
            .bg(rgb(0x0b0f10))
            .text_color(rgb(0xffffff))
            .child(
                h_flex()
                    .flex_none()
                    .h(px(64. * scale))
                    .px(px(20. * scale))
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(
                        div()
                            .id("video-viewer-close")
                            .p(px(6. * scale))
                            .rounded_md()
                            .cursor_pointer()
                            .hover(|style| style.bg(rgba(0xffffff1a)))
                            .child(
                                Icon::new(WhatfastIcon::Close)
                                    .with_size(px(20. * scale))
                                    .text_color(rgb(0xffffff)),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.close_video_fullscreen(cx);
                            })),
                    ),
            )
            .child(
                div()
                    .id("video-viewer-body")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .px(px(24. * scale))
                    // El fondo que rodea al cuadro lo cierra con un clic.
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.close_video_fullscreen(cx);
                    }))
                    .child(
                        div()
                            .id("video-viewer-frame")
                            .relative()
                            .w(px(width))
                            .max_w_full()
                            .h(px(height))
                            .max_h_full()
                            .rounded_lg()
                            .overflow_hidden()
                            .bg(rgb(0x000000))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.toggle_video_playback(
                                    play_chat_id.clone(),
                                    play_message_id.clone(),
                                    window,
                                    cx,
                                );
                            }))
                            .when_some(frame, |player, frame| {
                                player.child(
                                    img(frame).w_full().h_full().object_fit(ObjectFit::Contain),
                                )
                            })
                            .when(!has_frame, |player| {
                                player.child(
                                    div()
                                        .absolute()
                                        .inset_0()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(
                                            Icon::new(WhatfastIcon::Video)
                                                .with_size(px(56. * scale))
                                                .text_color(rgba(0xffffff59)),
                                        ),
                                )
                            })
                            // El boton del centro: reproducir o pausar.
                            .child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        div()
                                            .size(px(68. * scale))
                                            .rounded_full()
                                            .bg(rgba(0x00000099))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .child(
                                                Icon::new(if playing {
                                                    WhatfastIcon::Pause
                                                } else {
                                                    WhatfastIcon::Play
                                                })
                                                .with_size(px(32. * scale))
                                                .text_color(rgb(0xffffff)),
                                            ),
                                    ),
                            )
                            // El reloj y la barra, pegados al borde de abajo.
                            .child(
                                v_flex()
                                    .absolute()
                                    .bottom_0()
                                    .left_0()
                                    .right_0()
                                    .px(px(12. * scale))
                                    .pb(px(8. * scale))
                                    .gap(px(5. * scale))
                                    .child(
                                        h_flex()
                                            .id("video-viewer-volume")
                                            .gap(px(8. * scale))
                                            .items_center()
                                            .text_xs()
                                            .text_color(rgba(0xffffffcc))
                                            .on_click(cx.listener(|_, _, _, cx| {
                                                cx.stop_propagation();
                                            }))
                                            .child(
                                                div()
                                                    .id("video-viewer-mute")
                                                    .p(px(4. * scale))
                                                    .rounded_md()
                                                    .cursor_pointer()
                                                    .hover(|style| style.bg(rgba(0xffffff1a)))
                                                    .tooltip({
                                                        let label: gpui::SharedString = if silent {
                                                            i18n::t("Unmute")
                                                        } else {
                                                            i18n::t("Mute")
                                                        }
                                                        .into();
                                                        move |window, cx| {
                                                            Tooltip::new(label.clone())
                                                                .build(window, cx)
                                                        }
                                                    })
                                                    .child(
                                                        Icon::new(if silent {
                                                            WhatfastIcon::VolumeMute
                                                        } else {
                                                            WhatfastIcon::Volume
                                                        })
                                                        .with_size(px(18. * scale))
                                                        .text_color(rgb(0xffffff)),
                                                    )
                                                    .on_click(cx.listener(
                                                        |this, _, window, cx| {
                                                            cx.stop_propagation();
                                                            this.toggle_video_mute(window, cx);
                                                        },
                                                    )),
                                            )
                                            .child(div().w(px(120. * scale)).child(
                                                Slider::new(&self.volume_slider).horizontal(),
                                            ))
                                            .child(div().flex_1())
                                            .child(format!(
                                                "{} / {}",
                                                attachments::format_clock(position),
                                                attachments::format_clock(duration)
                                            ))
                                            .when(waiting, |row| {
                                                row.child(i18n::t("Downloading the video"))
                                            }),
                                    )
                                    .child(
                                        div()
                                            .h(px(3. * scale))
                                            .w_full()
                                            .rounded_full()
                                            .bg(rgba(0xffffff33))
                                            .child(
                                                div()
                                                    .h_full()
                                                    .w(relative(progress))
                                                    .rounded_full()
                                                    .bg(rgb(0x25d366)),
                                            ),
                                    ),
                            ),
                    ),
            )
            .when_some(error, |viewer, error| {
                viewer.child(
                    div()
                        .flex_none()
                        .px(px(20. * scale))
                        .pb(px(12. * scale))
                        .text_sm()
                        .text_color(rgba(0xff8a8aff))
                        .child(error),
                )
            })
            .into_any_element()
    }

    /// El menu de tres puntos del visor, con las mismas acciones que ofrece
    /// WhatsApp sobre la foto: compartir, abrir con, info del mensaje, ir al
    /// mensaje, guardar como, buscar en la web y copiar.
    fn render_image_menu(&self, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let Some(menu) = self.image_context_menu.as_ref() else {
            return div().into_any_element();
        };
        let Some(viewer) = self.image_viewer.as_ref() else {
            return div().into_any_element();
        };
        let scale = self.ui_scale;
        let chat_id = viewer.chat_id.clone();
        let message_id = viewer.message_id.clone();
        let path = viewer.path.clone();
        let width = px(248. * scale);
        let height = px((7. * 36. + 16.) * scale);
        let viewport = window.viewport_size();
        let left = menu
            .position
            .x
            .min(viewport.width - width - px(8.))
            .max(px(8.));
        let top = menu
            .position
            .y
            .min(viewport.height - height - px(8.))
            .max(px(8.));
        let hover = rgba(0xffffff1f);
        div()
            .id("image-menu-backdrop")
            .absolute()
            .inset_0()
            .on_click(cx.listener(|this, _, _, cx| {
                this.image_context_menu = None;
                cx.stop_propagation();
                cx.notify();
            }))
            .child(
                v_flex()
                    .id("image-context-menu")
                    .absolute()
                    .left(left)
                    .top(top)
                    .w(width)
                    .py(px(8. * scale))
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(0x2f2f2f))
                    .bg(rgb(0x1c1c1c))
                    .text_color(rgb(0xffffff))
                    .shadow_lg()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        image_menu_row(
                            ("image-menu-copy", 0),
                            Icon::new(WhatfastIcon::Copy),
                            i18n::t("Copy"),
                            hover,
                            scale,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.copy_viewed_image(cx);
                            cx.notify();
                        })),
                    )
                    .child(
                        image_menu_row(
                            ("image-menu-share", 0),
                            Icon::new(WhatfastIcon::Forward),
                            i18n::t("Share"),
                            hover,
                            scale,
                        )
                        .on_click(cx.listener({
                            let chat_id = chat_id.clone();
                            let message_id = message_id.clone();
                            move |this, _, window, cx| {
                                this.open_forward_picker(
                                    chat_id.clone(),
                                    message_id.clone(),
                                    window,
                                    cx,
                                );
                                this.image_context_menu = None;
                                cx.notify();
                            }
                        })),
                    )
                    .child(
                        image_menu_row(
                            ("image-menu-open-with", 0),
                            Icon::new(IconName::ExternalLink),
                            i18n::t("Open with"),
                            hover,
                            scale,
                        )
                        .on_click(cx.listener({
                            let path = path.clone();
                            move |this, _, _, cx| {
                                this.open_file_externally(&path, cx);
                                cx.notify();
                            }
                        })),
                    )
                    .child(
                        image_menu_row(
                            ("image-menu-info", 0),
                            Icon::new(IconName::Info),
                            i18n::t("Message info"),
                            hover,
                            scale,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.viewer_info_open = true;
                            this.image_context_menu = None;
                            cx.notify();
                        })),
                    )
                    .child(
                        image_menu_row(
                            ("image-menu-go", 0),
                            Icon::new(WhatfastIcon::Chat),
                            i18n::t("Go to message"),
                            hover,
                            scale,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.go_to_viewed_message();
                            cx.notify();
                        })),
                    )
                    .child(
                        image_menu_row(
                            ("image-menu-save", 0),
                            Icon::new(WhatfastIcon::Download),
                            i18n::t("Save as…"),
                            hover,
                            scale,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.save_viewed_image_as(window, cx);
                            cx.notify();
                        })),
                    )
                    .child(
                        image_menu_row(
                            ("image-menu-search", 0),
                            Icon::new(IconName::Search),
                            i18n::t("Search the web"),
                            hover,
                            scale,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.search_viewed_image(cx);
                            cx.notify();
                        })),
                    ),
            )
            .into_any_element()
    }

    fn render_linkified_text(&self, message_id: &str, text: &str) -> TextView {
        // La vista de texto de gpui-component (HTML simple) es la que sabe
        // seleccionar y copiar: el texto de la burbuja deja de ser una fila de
        // divs por palabra y pasa a ser un solo bloque seleccionable. Los
        // enlaces se siguen abriendo con el clic y el resto se escapa.
        //
        // El ancho se topa aca y no solo en la burbuja: la vista se mide con
        // el ancho que le ofrece la burbuja, y una burbuja que se encoge al
        // contenido le ofrece el ancho infinito. Sin el tope, un mensaje
        // largo se pintaba en una sola linea y se salia de la ventana.
        let participants = self
            .store
            .selected_chat_id
            .as_ref()
            .and_then(|id| self.mention_directories.get(id))
            .map(|v| v.as_slice());
        let mut text_style = TextViewStyle::default().paragraph_gap(gpui::rems(0.));
        text_style.code_block = gpui::StyleRefinement::default()
            .p_0()
            .bg(gpui::transparent_black())
            .rounded_none();
        TextView::html(
            ElementId::Name(format!("message-text-{message_id}").into()),
            message_html_with_mentions(text, participants, &self.store.chats),
        )
        .selectable(true)
        .max_w(px(TEXT_MAX_WIDTH * self.ui_scale))
        .style(text_style)
    }

    fn render_link_preview(
        &self,
        index: usize,
        preview: proto::LinkPreview,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let target = is_safe_web_url(&preview.url).then(|| preview.url.clone());
        let host = link_preview_host(&preview.url).to_string();
        let title = if preview.title.trim().is_empty() {
            host.clone()
        } else {
            preview.title.clone()
        };
        let thumbnail = (!preview.jpeg_thumbnail.is_empty()).then(|| {
            Arc::new(gpui::Image::from_bytes(
                gpui::ImageFormat::Jpeg,
                preview.jpeg_thumbnail,
            ))
        });
        // La tarjeta vive dentro de la burbuja: toma el color de su burbuja y
        // el color de su texto, no los del tema de la ventana. El tema de
        // `gpui_component` puede estar en claro mientras la burbuja es oscura,
        // y la tarjeta quedaba blanca con el titulo blanco encima.
        let from_me = self
            .store
            .messages
            .get(index)
            .is_some_and(|message| message.from_me);
        let theme = self.active_theme();
        let (incoming, outgoing) = tokens::bubble_colors(&theme);
        let (incoming_text, outgoing_text) = tokens::bubble_text_colors(&theme);
        let bubble: gpui::Hsla = if from_me {
            outgoing.into()
        } else {
            incoming.into()
        };
        let text: gpui::Hsla = if from_me {
            outgoing_text.into()
        } else {
            incoming_text.into()
        };
        // El fondo de la tarjeta es la burbuja un poco mas oscura: el borde
        // de la imagen no se pierde y el titulo sigue teniendo contraste.
        let card_bg = bubble.blend(gpui::black().opacity(0.18));
        let soft = text.opacity(0.78);
        let faint = text.opacity(0.58);
        v_flex()
            .id(("link-preview", index))
            .w(px(320.))
            .max_w_full()
            .overflow_hidden()
            .rounded_md()
            .bg(card_bg)
            .when_some(thumbnail, |card, thumbnail| {
                card.child(
                    div()
                        .w_full()
                        .h(px(150.))
                        .overflow_hidden()
                        .child(img(thumbnail).size_full().object_fit(ObjectFit::Cover)),
                )
            })
            .child(
                v_flex()
                    .px_3()
                    .py_2()
                    .gap_1()
                    .child(
                        div()
                            .line_clamp(2)
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(text)
                            .child(title),
                    )
                    .when(!preview.description.trim().is_empty(), |details| {
                        details.child(
                            div()
                                .line_clamp(2)
                                .text_xs()
                                .text_color(soft)
                                .child(preview.description),
                        )
                    })
                    .child(div().truncate().text_xs().text_color(faint).child(host)),
            )
            .when_some(target, |card, target| {
                card.cursor_pointer()
                    .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&target)))
            })
    }

    fn render_message(&self, index: usize, cx: &mut Context<Self>) -> gpui::Div {
        let message = &self.store.messages[index];
        // The bubbles of the messages: the gray of the received ones and the
        // green of the sent ones, the same colors of the reference.
        let (bubble_in, bubble_out) = tokens::bubble_colors(&self.active_theme());
        let bubble_in = gpui::Hsla::from(bubble_in);
        let bubble_out = gpui::Hsla::from(bubble_out);
        let text_content = match message.content.as_ref() {
            Some(proto::message::Content::Text(text)) => Some(text.clone()),
            _ => None,
        };
        let image_content = match message.content.as_ref() {
            Some(proto::message::Content::Image(image)) => Some(image.clone()),
            _ => None,
        };
        // El adjunto (un documento, un video o un audio) y las tarjetas de
        // la ubicacion y de los contactos.
        let attachment_content = match message.content.as_ref() {
            Some(proto::message::Content::Attachment(attachment)) => Some(attachment.clone()),
            _ => None,
        };
        let card_content = message
            .content
            .as_ref()
            .filter(|content| ubicaciones::is_card_content(content))
            .cloned();
        // El mensaje de visualizacion unica no trae el contenido: la burbuja
        // es el aviso de WhatsApp.
        let view_once = is_view_once_message(message);
        let sticker = image_content.as_ref().is_some_and(|image| image.sticker);
        let text = if sticker || view_once {
            String::new()
        } else {
            // port: el texto de la burbuja no repite el nombre del archivo ni
            // el del lugar: las tarjetas ya los dicen.
            attachments::bubble_text(message).to_string()
        };
        let reaction_chips = reaction_counts(message);
        let reply_target_id = message.reply_to_message_id.clone();
        let reply_target = self.store.message(&reply_target_id).cloned();
        let reply_count = self.store.reply_count(&message.id);
        let first_reply_id = self.store.first_reply_id(&message.id).map(str::to_owned);
        let chat_id = message.chat_id.clone();
        let message_id = message.id.clone();
        let starred = self.store.message_is_starred(&chat_id, &message_id);
        // El nombre del grupo que muestra los botones del mensaje con solo
        // pasarle el cursor por encima: la fila entera es el grupo, asi los
        // botones siguen visibles cuando el puntero baja hasta ellos.
        let actions_group = format!("message-actions-{}", message.id);
        let body_message_id = message.id.clone();
        let search_target = self.search_target_message_id.as_deref() == Some(message.id.as_str());
        let reply_action_message_id = message_id.clone();
        let more_action_message_id = message_id.clone();
        let bubble_menu_message_id = message_id.clone();
        let message_image_chat_id = chat_id.clone();
        let message_image_id = message_id.clone();
        let image_failure = self
            .image_failures
            .get(&(chat_id.clone(), message_id.clone()))
            .cloned();
        let reaction_chat_id = chat_id.clone();
        let reaction_message_id = message_id.clone();
        let own_reaction_background: gpui::Hsla = if cx.theme().is_dark() {
            rgb(0x164e3d).into()
        } else {
            rgb(0xd9fdd3).into()
        };
        let reaction_hover = if cx.theme().is_dark() {
            rgb(0x245c4e)
        } else {
            rgb(0xc8f4c1)
        };
        let group_incoming = !message.from_me
            && self
                .store
                .selected_chat()
                .is_some_and(|chat| chat.kind() == proto::ChatKind::Group);
        let meta_text = if message.edited {
            "edited".to_string()
        } else {
            String::new()
        };
        // La tilde de la referencia: el reloj mientras sale, una para el
        // enviado, dos para el entregado y dos azules para el leido. Antes
        // esto se escribia con palabras debajo de la burbuja.
        let status_glyph = if message.from_me {
            message_status_glyph(message.status())
        } else {
            None
        };
        let status_color = match status_glyph {
            Some(StatusGlyph::Read) => {
                if cx.theme().is_dark() {
                    rgb(0x53bdeb).into()
                } else {
                    rgb(0x34b7f1).into()
                }
            }
            Some(StatusGlyph::Failed) => cx.theme().danger,
            _ => cx.theme().muted_foreground,
        };
        // port: the reference writes the hour of the message at the end of
        // the text, inside the bubble, and it is the first thing that
        // changes when the day of the message is not the day of the one
        // before it.
        let time_label = bubble_time_label(
            message.timestamp_ms,
            local_offset_minutes(message.timestamp_ms),
        );
        // The tail of the bubble has the colour of the bubble: a sticker has
        // no bubble, so it has no tail either.
        let bubble_color = if sticker {
            gpui::transparent_black()
        } else if message.from_me {
            bubble_out
        } else {
            bubble_in
        };
        let starts_cluster = self.starts_cluster(index);
        let vertical_padding = px(if self.density == Density::Compact {
            4.
        } else {
            6.
        } * self.ui_scale);
        let bubble_element = v_flex()
            .id(("message-bubble", index))
            .max_w(px(560.))
            .when(!sticker, |bubble| bubble.px_3().py(vertical_padding))
            .when(starts_cluster, |bubble| {
                if message.from_me {
                    bubble.rounded_l_lg().rounded_b_lg().rounded_tr_sm()
                } else {
                    bubble.rounded_r_lg().rounded_b_lg().rounded_tl_sm()
                }
            })
            .when(!starts_cluster, |bubble| bubble.rounded_lg())
            // The tail of the bubble is painted in the
            // corner of the bubble, so the bubble has
            // to be the place where it is measured.
            .relative()
            // The bubble hugs its content, the way the
            // reference draws it: the hour goes right
            // after the text and not at the end of a
            // bubble as wide as the row of the actions.
            .when(message.from_me, |bubble| bubble.self_end())
            .when(!message.from_me, |bubble| bubble.self_start())
            .when(search_target, |bubble| {
                bubble.border_2().border_color(cx.theme().accent)
            })
            .gap_1()
            .bg(if sticker {
                gpui::transparent_black()
            } else if message.from_me {
                // port: the bubble of the sent messages of the
                // reference is the green of the theme.
                bubble_out
            } else {
                // The received messages have their own bubble:
                // the gray of the reference.
                bubble_in
            })
            .on_aux_click(cx.listener({
                let message_id = bubble_menu_message_id.clone();
                move |this, event: &ClickEvent, window, cx| {
                    if !event.is_secondary() {
                        return;
                    }
                    let position = event
                        .mouse_position()
                        .unwrap_or_else(|| window.mouse_position());
                    this.open_message_menu(message_id.clone(), position, cx);
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            // The point of the corner: at the left of the
            // received bubbles and at the right of the
            // sent ones. The tail sticks out of the
            // corner, the way the reference draws it.
            .when(!sticker && starts_cluster, |bubble| {
                bubble.child(
                    div()
                        .absolute()
                        .top(px(BUBBLE_TAIL_OFFSET * self.ui_scale))
                        .when(message.from_me, |tail| {
                            tail.right(px(BUBBLE_TAIL_OFFSET * self.ui_scale))
                        })
                        .when(!message.from_me, |tail| {
                            tail.left(px(BUBBLE_TAIL_OFFSET * self.ui_scale))
                        })
                        .child(bubble_tail(bubble_color, message.from_me, self.ui_scale)),
                )
            })
            .when(group_incoming, |bubble| {
                let sender_name = if !message.sender_name.trim().is_empty() {
                    message.sender_name.trim().to_string()
                } else if !message.sender_phone_number.trim().is_empty() {
                    whatfast_core::phone::format(message.sender_phone_number.trim())
                } else if let Some(participant) = self
                    .mention_directories
                    .get(&message.chat_id)
                    .and_then(|members| {
                        members.iter().find(|m| {
                            m.participant_id == message.sender_id
                                || (!m.phone_number.is_empty()
                                    && message.sender_id.starts_with(&m.phone_number))
                                || (!m.participant_id.is_empty()
                                    && message.sender_id.split('@').next()
                                        == m.participant_id.split('@').next())
                        })
                    })
                {
                    crate::text::mention_display_name(participant)
                } else if let Some(chat) = self.store.chats.iter().find(|c| {
                    let c_id = c.id.split('@').next().unwrap_or("");
                    let s_id = message.sender_id.split('@').next().unwrap_or("");
                    !c_id.is_empty() && c_id == s_id
                }) {
                    chat.title.clone()
                } else if !message.sender_id.trim().is_empty() {
                    message
                        .sender_id
                        .split('@')
                        .next()
                        .unwrap_or("")
                        .to_string()
                } else {
                    i18n::t("Unknown contact").to_string()
                };
                let identity_key = if !message.sender_id.is_empty() {
                    &message.sender_id
                } else {
                    &sender_name
                };
                let name_color = cx.theme().blue.hue(identity_hue(identity_key));
                let show_number = !message.sender_phone_number.is_empty()
                    && whatfast_core::phone::format(&message.sender_phone_number) != sender_name;
                bubble.child(
                    h_flex()
                        .items_baseline()
                        .gap_2()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(name_color)
                                .child(sender_name),
                        )
                        .when(show_number, |header| {
                            header.child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(whatfast_core::phone::format(
                                        &message.sender_phone_number,
                                    )),
                            )
                        }),
                )
            })
            .when(!reply_target_id.is_empty(), |bubble| {
                let target_id = reply_target_id.clone();
                let (sender, preview) = reply_target.as_ref().map_or_else(
                    || {
                        (
                            i18n::t("Original message").to_string(),
                            i18n::t("Message unavailable").to_string(),
                        )
                    },
                    |target| {
                        let sender = if target.from_me {
                            i18n::t("You").to_string()
                        } else if !target.sender_name.trim().is_empty() {
                            target.sender_name.trim().to_string()
                        } else {
                            i18n::t("Unknown contact").to_string()
                        };
                        (sender, message_preview(target))
                    },
                );
                bubble.child(
                    v_flex()
                        .id(("quoted-message", index))
                        .w_full()
                        .min_w(px(180.))
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .border_l_2()
                        .border_color(cx.theme().accent)
                        .bg(cx.theme().secondary)
                        .cursor_pointer()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(cx.theme().accent)
                                .child(sender),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(preview),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.scroll_to_message(&target_id);
                            cx.notify();
                        })),
                )
            })
            // El aviso va antes de cualquier otro contenido: un mensaje de
            // visualizacion unica no tiene foto ni texto que mostrar.
            .when(view_once, |bubble| {
                bubble.child(self.render_view_once(index, cx))
            })
            .when_some(image_content, |bubble, image| {
                let (image_width, image_height) = image_render_size(&image);
                let row_path = image_row_path(&image).to_string();
                let viewer_path = image_viewer_path(&image).to_string();
                // El sticker animado se pinta desde el archivo
                // entero: el thumbnail es un solo cuadro y con
                // el no se mueve nada.
                let image_path = image_source_candidates(&image, &row_path, &viewer_path)
                    .into_iter()
                    .find(|path| !path.is_empty() && Path::new(*path).is_file())
                    .unwrap_or_default()
                    .to_string();
                let image_is_cached = !image_path.is_empty();
                let cached_path_missing =
                    (!row_path.is_empty() || !viewer_path.is_empty()) && !image_is_cached;
                let retryable =
                    image.downloadable && (image_failure.is_some() || cached_path_missing);
                let placeholder = if cached_path_missing {
                    if image.sticker {
                        i18n::t("The cached sticker is missing. Click to reload.").to_string()
                    } else {
                        i18n::t("The cached photo is missing. Click to reload.").to_string()
                    }
                } else if let Some(failure) = image_failure.clone() {
                    failure
                } else if image.downloadable {
                    if image.sticker {
                        i18n::t("Loading the sticker…").to_string()
                    } else {
                        i18n::t("Loading the photo…").to_string()
                    }
                } else if image.sticker {
                    i18n::t("That sticker is no longer available").to_string()
                } else {
                    i18n::t("Photo is no longer available").to_string()
                };
                let viewer_chat_id = message_image_chat_id.clone();
                let viewer_message_id = message_image_id.clone();
                let retry_chat_id = message_image_chat_id.clone();
                let retry_message_id = message_image_id.clone();
                let loading_color = cx.theme().muted_foreground;
                let fallback_color = cx.theme().muted_foreground;
                let decoding_label = if image.sticker {
                    i18n::t("Decoding the sticker…").to_string()
                } else {
                    i18n::t("Decoding the photo…").to_string()
                };
                let decode_failure_label = if image.sticker {
                    i18n::t("Couldn't display the sticker · Open to retry").to_string()
                } else {
                    i18n::t("Couldn't display the photo · Open to retry").to_string()
                };
                bubble.child(
                    div()
                        .id(("message-image", index))
                        .w(image_width)
                        .h(image_height)
                        .max_w_full()
                        .overflow_hidden()
                        .rounded_md()
                        .when(!image.sticker, |container| {
                            container.bg(cx.theme().secondary)
                        })
                        .when(image_is_cached, |container| {
                            let fit = if image.sticker {
                                ObjectFit::Contain
                            } else {
                                ObjectFit::Cover
                            };
                            container.child(
                                img(PathBuf::from(image_path.clone()))
                                    .size_full()
                                    .object_fit(fit)
                                    .with_loading(move || {
                                        div()
                                            .size_full()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .text_sm()
                                            .text_color(loading_color)
                                            .child(decoding_label.clone())
                                            .into_any_element()
                                    })
                                    .with_fallback(move || {
                                        div()
                                            .size_full()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .px_3()
                                            .text_sm()
                                            .text_color(fallback_color)
                                            .child(decode_failure_label.clone())
                                            .into_any_element()
                                    }),
                            )
                        })
                        .when(image_is_cached, |container| {
                            container.cursor_pointer().on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.open_chat_image(
                                        viewer_chat_id.clone(),
                                        viewer_message_id.clone(),
                                    );
                                    cx.notify();
                                },
                            ))
                        })
                        .when(!image_is_cached, |container| {
                            container.child(
                                div()
                                    .size_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .px_3()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(placeholder),
                            )
                        })
                        .when(retryable, |container| {
                            container.cursor_pointer().on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.retry_message_image(
                                        retry_chat_id.clone(),
                                        retry_message_id.clone(),
                                    );
                                    cx.notify();
                                },
                            ))
                        }),
                )
            })
            .when_some(
                text_content.and_then(|text| text.link_preview),
                |bubble, preview| bubble.child(self.render_link_preview(index, preview, cx)),
            )
            // La tarjeta del adjunto y las de la
            // ubicacion y de los contactos van antes
            // del texto, como en el origen.
            .when_some(attachment_content.clone(), |bubble, attachment| {
                bubble.child(self.render_attachment_card(index, message, &attachment, cx))
            })
            .when_some(
                card_content.as_ref().and_then(|content| {
                    ubicaciones::render_content_card(content, index, self.ui_scale, cx)
                }),
                |bubble, card| bubble.child(card),
            )
            .when(!text.is_empty(), |bubble| {
                // The hour goes at the end of the text,
                // inside the bubble, the way the
                // reference writes it, with the ticks
                // of the delivery state right after.
                let scale = self.ui_scale;
                bubble.child(
                    h_flex()
                        .items_end()
                        .gap_1()
                        .child(self.render_linkified_text(&body_message_id, &text))
                        .when(starred, |row| {
                            row.child(
                                Icon::new(IconName::StarFill)
                                    .with_size(px(11. * scale))
                                    .text_color(cx.theme().accent),
                            )
                        })
                        .child(
                            h_flex()
                                .flex_none()
                                .items_center()
                                .gap(px(3. * scale))
                                .when(!meta_text.is_empty(), |row| {
                                    row.child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(meta_text.clone()),
                                    )
                                })
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(time_label.clone()),
                                )
                                .when_some(status_glyph, |row, glyph| {
                                    row.child(status_tick(glyph, status_color, scale))
                                }),
                        ),
                )
            })
            // Una foto o un sticker sin epigrafe no
            // tienen la linea del texto: la hora y la
            // tilde van solas, abajo a la derecha.
            .when(text.is_empty(), |bubble| {
                let scale = self.ui_scale;
                bubble.child(
                    h_flex()
                        .w_full()
                        .justify_end()
                        .items_center()
                        .gap(px(3. * scale))
                        .when(starred, |row| {
                            row.child(
                                Icon::new(IconName::StarFill)
                                    .with_size(px(11. * scale))
                                    .text_color(cx.theme().accent),
                            )
                        })
                        .when(!meta_text.is_empty(), |row| {
                            row.child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(meta_text.clone()),
                            )
                        })
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(time_label.clone()),
                        )
                        .when_some(status_glyph, |row, glyph| {
                            row.child(status_tick(glyph, status_color, scale))
                        }),
                )
            })
            .when(!reaction_chips.is_empty(), |bubble| {
                bubble.child(
                    h_flex()
                        .gap_1()
                        .children(reaction_chips.into_iter().enumerate().map(
                            |(chip_index, reaction)| {
                                let emoji = reaction.emoji.to_string();
                                // El clic saca la reaccion propia o suma
                                // la del que toca; la lista de quienes
                                // reaccionaron queda en el boton derecho.
                                let click_chat = reaction_chat_id.clone();
                                let click_message = reaction_message_id.clone();
                                let click_emoji = emoji.clone();
                                let aux_chat = reaction_chat_id.clone();
                                let aux_message = reaction_message_id.clone();
                                let aux_emoji = emoji.clone();
                                let mine = reaction.from_me;
                                let label = if reaction.count <= 1 {
                                    emoji.clone()
                                } else {
                                    format!("{emoji} {}", reaction.count)
                                };
                                div()
                                    .id(format!("reaction-chip-{index}-{chip_index}"))
                                    .px_2()
                                    .py_0p5()
                                    .rounded_full()
                                    .border_1()
                                    .border_color(if reaction.from_me {
                                        cx.theme().accent
                                    } else {
                                        cx.theme().border
                                    })
                                    .bg(if reaction.from_me {
                                        own_reaction_background
                                    } else {
                                        cx.theme().secondary
                                    })
                                    .hover(move |style| style.bg(reaction_hover))
                                    .cursor_pointer()
                                    .text_xs()
                                    .child(label)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.click_reaction_chip(
                                            click_chat.clone(),
                                            click_message.clone(),
                                            click_emoji.clone(),
                                            mine,
                                            cx,
                                        );
                                        cx.notify();
                                    }))
                                    .on_aux_click(cx.listener(move |this, _, _, cx| {
                                        this.open_reaction_details(
                                            aux_chat.clone(),
                                            aux_message.clone(),
                                            aux_emoji.clone(),
                                        );
                                        cx.notify();
                                    }))
                            },
                        )),
                )
            });

        let hover_actions = h_flex()
            .flex_none()
            .items_center()
            .gap_1()
            // Los tres botones de la referencia (responder,
            // reaccionar y el menu) solo aparecen cuando
            // el puntero esta sobre el mensaje.
            .opacity(0.)
            .group_hover(actions_group.clone(), |style| style.opacity(1.))
            .when(reply_count > 0, |footer| {
                let reply_id = first_reply_id.clone().unwrap_or_default();
                footer.child(
                    Button::new(("view-replies", index))
                        .ghost()
                        .small()
                        .label(if reply_count == 1 {
                            i18n::t("1 reply").to_string()
                        } else {
                            i18n::t_args("{count} replies", &[("count", &reply_count.to_string())])
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.scroll_to_message(&reply_id);
                            cx.notify();
                        })),
                )
            })
            .child(
                Button::new(("reply-message", index))
                    .ghost()
                    .small()
                    .icon(IconName::Undo)
                    .tooltip(i18n::t("Reply"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.start_reply(reply_action_message_id.clone());
                        cx.notify();
                    })),
            )
            .child(
                Button::new(("react-message", index))
                    .ghost()
                    .small()
                    .icon(IconName::Heart)
                    .tooltip(i18n::t("React"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_reaction_picker(chat_id.clone(), message_id.clone());
                        cx.notify();
                    })),
            )
            .child(
                // The menu of the message: reply, forward,
                // edit, delete and pin.
                Button::new(("more-message", index))
                    .ghost()
                    .small()
                    .label("…")
                    .tooltip(i18n::t("More"))
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        let position = event
                            .mouse_position()
                            .unwrap_or_else(|| window.mouse_position());
                        this.open_message_menu(more_action_message_id.clone(), position, cx);
                        cx.notify();
                    })),
            );

        v_flex()
            .w_full()
            .when(
                index > 0 && self.starts_cluster(index) && !self.starts_a_day(index),
                |col| col.pt(px(CLUSTER_SEPARATOR_GAP * self.ui_scale)),
            )
            .when_some(self.day_separator_label(index), |column, label| {
                column.child(
                    div().flex().justify_center().py_2().child(
                        div()
                            .h(px(DAY_SEPARATOR_HEIGHT))
                            .px_3()
                            .items_center()
                            .rounded_full()
                            .bg(cx.theme().secondary)
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(label),
                    ),
                )
            })
            .child({
                let mut row = h_flex()
                    .w_full()
                    .items_center()
                    .gap_2()
                    .group(actions_group);
                if message.from_me {
                    row = row.justify_end().child(hover_actions).child(bubble_element);
                } else {
                    row = row
                        .justify_start()
                        .child(bubble_element)
                        .child(hover_actions);
                }
                row
            })
    }

    /// El aviso de WhatsApp para un mensaje de visualizacion unica: el
    /// telefono retiene el contenido y el dispositivo vinculado solo puede
    /// avisar que llego.
    fn render_view_once(&self, index: usize, cx: &mut Context<Self>) -> gpui::Div {
        let scale = self.ui_scale;
        let muted = cx.theme().muted_foreground;
        let accent = cx.theme().accent;
        let accent_hover = cx.theme().accent.opacity(0.75);
        let notice = if self.store.messages[index].from_me {
            i18n::t(
                "You sent a view once message. For added privacy, it can only be opened on your phone.",
            )
        } else {
            i18n::t(
                "You received a view once message. For added privacy, you can only open it on your phone.",
            )
        };
        h_flex()
            .w(px(VIEW_ONCE_WIDTH * scale))
            .max_w_full()
            .items_start()
            .gap_2()
            .child(
                div()
                    .flex_none()
                    .size(px(VIEW_ONCE_BADGE * scale))
                    .rounded_full()
                    .border_1()
                    .border_dashed()
                    .border_color(muted)
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_xs()
                    .text_color(muted)
                    .child("1"),
            )
            .child(
                v_flex()
                    .min_w_0()
                    .flex_1()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .italic()
                            .text_color(cx.theme().foreground)
                            .child(notice.to_string()),
                    )
                    .child(
                        div()
                            .id(("view-once-link", index))
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(accent)
                            .cursor_pointer()
                            .hover(move |style| style.text_color(accent_hover))
                            .child(i18n::t("More information").to_string())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.view_once_info_open = true;
                                cx.notify();
                            })),
                    ),
            )
    }

    /// The height of every message, measured with the text system of the
    /// window and cached by the width bucket of the bubble.
    fn measure_message_heights(
        &mut self,
        wrap_width: Pixels,
        font_size: Pixels,
        window: &mut Window,
    ) {
        let width_bucket = (wrap_width / px(8.)).round() as i32;
        if !self.store.message_sizes_need_measurement(width_bucket) {
            return;
        }
        let line_height = window.line_height();
        let mut sizes = Vec::with_capacity(self.store.messages.len());
        for index in 0..self.store.messages.len() {
            let height = if let Some(height) = {
                let message = &self.store.messages[index];
                self.store.cached_message_height(&message.id, width_bucket)
            } {
                height
            } else {
                let (id, text) = {
                    let message = &self.store.messages[index];
                    // El aviso de visualizacion unica tiene su propia medida:
                    // el texto de la fila es solo el adelanto de la lista.
                    let text = if is_view_once_message(message) {
                        String::new()
                    } else {
                        attachments::bubble_text(message).to_string()
                    };
                    // La medida y la vista del texto tienen que cortar los
                    // renglones en el mismo lugar (ver `message_html`): el
                    // tabulador se expande a cuatro espacios y el CR cuenta
                    // como salto de linea.
                    (
                        message.id.clone(),
                        text.replace("\r\n", "\n")
                            .replace('\r', "\n")
                            .replace('\t', "    "),
                    )
                };
                // The hour goes at the end of the text, inside the bubble:
                // the text wraps before it, so the width it takes is not
                // there for the text.
                let time_width = if text.is_empty() {
                    px(0.)
                } else {
                    let timestamp_ms = self.store.messages[index].timestamp_ms;
                    let label = bubble_time_label(timestamp_ms, local_offset_minutes(timestamp_ms));
                    // La tilde va pegada a la hora: la medida del texto le
                    // tiene que dejar el lugar o el texto se le va encima.
                    let ticks = if self.store.messages[index].from_me {
                        px(18. * self.ui_scale)
                    } else {
                        px(0.)
                    };
                    bubble_time_width(window, &label, font_size) + ticks
                };
                let text_wrap_width = (wrap_width - time_width).max(px(60.));
                let text_height = if text.is_empty() {
                    px(0.)
                } else {
                    let text_shared: SharedString = text.as_str().into();
                    let run = window.text_style().to_run(text.len());
                    window
                        .text_system()
                        .shape_text(text_shared, font_size, &[run], Some(text_wrap_width), None)
                        .map(|lines| {
                            lines.iter().fold(px(0.), |height, line| {
                                height + line.size(line_height).height
                            })
                        })
                        .unwrap_or(line_height)
                };
                let image_height = match self.store.messages[index].content.as_ref() {
                    Some(proto::message::Content::Image(image)) => image_render_size(image).1,
                    _ => px(0.),
                };
                let link_preview_height = match self.store.messages[index].content.as_ref() {
                    Some(proto::message::Content::Text(text)) if text.link_preview.is_some() => {
                        let preview = text.link_preview.as_ref();
                        let thumbnail =
                            preview.is_some_and(|preview| !preview.jpeg_thumbnail.is_empty());
                        let description =
                            preview.is_some_and(|preview| !preview.description.trim().is_empty());
                        // La tarjeta suma el borde, el relleno, los huecos y
                        // las lineas (titulo, descripcion si hay, dominio).
                        // Medirla de menos corria la fila siguiente encima de
                        // la ultima linea del texto.
                        let card = if description { 82. } else { 62. };
                        px((if thumbnail { 144. } else { 0. } + card) * self.ui_scale)
                    }
                    _ => px(0.),
                };
                let reaction_height = if reaction_counts(&self.store.messages[index]).is_empty() {
                    px(0.)
                } else {
                    px(30. * self.ui_scale)
                };
                let reply_height = if self.store.messages[index].reply_to_message_id.is_empty() {
                    px(0.)
                } else {
                    px(54. * self.ui_scale)
                };
                // Group messages carry a sender name/number line above the body.
                let sender_header_height = if !self.store.messages[index].from_me
                    && self
                        .store
                        .selected_chat()
                        .is_some_and(|chat| chat.kind() == proto::ChatKind::Group)
                {
                    px(22. * self.ui_scale)
                } else {
                    px(0.)
                };
                // The card of the attachment and the cards of the place and of
                // the contacts take room of their own.
                let attachment_height = self.attachment_height(&self.store.messages[index]);
                let card_height = match self.store.messages[index].content.as_ref() {
                    Some(content) if ubicaciones::is_card_content(content) => {
                        px(ubicaciones::card_height(content, self.ui_scale))
                    }
                    _ => px(0.),
                };
                let bubble_vertical_padding = px(if self.density == Density::Compact {
                    8.
                } else {
                    12.
                } * self.ui_scale);
                // El aviso y el enlace del mensaje de visualizacion unica se
                // miden como un solo bloque de texto: el circulo del 1 deja
                // su lugar a la izquierda.
                let view_once_height = if is_view_once_message(&self.store.messages[index]) {
                    let notice = if self.store.messages[index].from_me {
                        i18n::t(
                            "You sent a view once message. For added privacy, it can only be opened on your phone.",
                        )
                    } else {
                        i18n::t(
                            "You received a view once message. For added privacy, you can only open it on your phone.",
                        )
                    };
                    let block = format!("{notice}\n{}", i18n::t("More information"));
                    let wrap =
                        px((VIEW_ONCE_WIDTH - VIEW_ONCE_BADGE - 8.) * self.ui_scale).max(px(60.));
                    let shared: SharedString = block.as_str().into();
                    let run = window.text_style().to_run(block.len());
                    let measured = window
                        .text_system()
                        .shape_text(shared, font_size, &[run], Some(wrap), None)
                        .map(|lines| {
                            lines.iter().fold(px(0.), |height, line| {
                                height + line.size(line_height).height
                            })
                        })
                        .unwrap_or(line_height * 3.);
                    measured + px(4. * self.ui_scale)
                } else {
                    px(0.)
                };
                let time_fallback = if text.is_empty()
                    && (image_height > px(0.)
                        || attachment_height > px(0.)
                        || view_once_height > px(0.))
                {
                    px(18. * self.ui_scale)
                } else {
                    px(0.)
                };
                // La burbuja apila sus hijos con un hueco de 4 px entre uno y
                // otro. Ese hueco no estaba en la cuenta y una burbuja con
                // tarjeta y texto se dibujaba sobre la fila siguiente.
                let mut blocks = 0;
                if sender_header_height > px(0.) {
                    blocks += 1;
                }
                if reply_height > px(0.) {
                    blocks += 1;
                }
                if view_once_height > px(0.) {
                    blocks += 1;
                }
                if image_height > px(0.) {
                    blocks += 1;
                }
                if link_preview_height > px(0.) {
                    blocks += 1;
                }
                if attachment_height > px(0.) {
                    blocks += 1;
                }
                if card_height > px(0.) {
                    blocks += 1;
                }
                // La fila de la hora existe siempre: sola, o al lado del
                // texto.
                blocks += 1;
                if reaction_height > px(0.) {
                    blocks += 1;
                }
                let children_gap = if blocks > 1 {
                    px(4. * self.ui_scale * (blocks - 1) as f32)
                } else {
                    px(0.)
                };
                let height = text_height
                    + image_height
                    + link_preview_height
                    + attachment_height
                    + card_height
                    + view_once_height
                    + bubble_vertical_padding
                    + time_fallback
                    + reaction_height
                    + reply_height
                    + sender_header_height
                    + children_gap;
                // The model stores the heights as plain numbers: GPUI's
                // `Pixels` is a newtype over the same `f32`.
                let height = f32::from(height);
                self.store.cache_message_height(id, width_bucket, height);
                height
            };
            // The pill of the day is not part of the cached height: the day
            // of a message depends on the message before it, and that one
            // changes when older messages arrive.
            let cluster_extra =
                if index > 0 && self.starts_cluster(index) && !self.starts_a_day(index) {
                    px(CLUSTER_SEPARATOR_GAP * self.ui_scale)
                } else {
                    px(0.)
                };
            let separator_gap = if self.density == Density::Compact {
                4.
            } else {
                DAY_SEPARATOR_GAP
            };
            let separator_height = if self.starts_a_day(index) {
                px(DAY_SEPARATOR_HEIGHT + separator_gap)
            } else {
                px(0.)
            };
            sizes.push(geom::size(
                geom::px(1.),
                height + f32::from(separator_height) + f32::from(cluster_extra),
            ));
        }
        self.store.set_measured_message_sizes(width_bucket, sizes);
    }

    /// The wallpaper of the conversation.
    ///
    /// port: the reference (`docs/referencia/01-whatsapp-chats.png`) paints a
    /// pattern of line drawings behind the messages of the conversation.
    /// GPUI cannot repeat an image, so the pattern is a grid of small glyphs
    /// of the icon set, painted with a very low opacity. The layer is
    /// absolute, so it does not move when the list of the messages scrolls.
    fn conversation_wallpaper(
        &self,
        width: Pixels,
        height: Pixels,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let color = cx.theme().muted_foreground.opacity(WALLPAPER_OPACITY);
        let glyphs = wallpaper_glyphs();
        let columns = (f32::from(width) / f32::from(WALLPAPER_STEP_X)).ceil() as usize + 1;
        let rows = (f32::from(height) / f32::from(WALLPAPER_STEP_Y)).ceil() as usize + 1;
        let mut layer = div().absolute().inset_0().overflow_hidden();
        for row in 0..rows {
            for column in 0..columns {
                let index = row * columns + column;
                let glyph = glyphs[index % glyphs.len()].clone();
                // The pattern of the reference is not a perfect grid: a small
                // offset per cell keeps it from looking like a table.
                let offset_x = ((gpui::hash(&index) % 21) as f32 - 10.) * 2.5;
                let offset_y = ((gpui::hash(&(index + 1)) % 15) as f32 - 7.) * 2.5;
                let size = px(f32::from(WALLPAPER_GLYPH_SIZE) * (0.85 + (index % 4) as f32 * 0.1));
                layer = layer.child(
                    div()
                        .absolute()
                        .left(px(f32::from(WALLPAPER_STEP_X) * column as f32 + offset_x))
                        .top(px(f32::from(WALLPAPER_STEP_Y) * row as f32 + offset_y))
                        .size(size)
                        .child(glyph.with_size(size).text_color(color)),
                );
            }
        }
        layer
    }

    /// Whether the message is the first one of its day.
    fn starts_a_day(&self, index: usize) -> bool {
        starts_a_day(&self.store.messages, index)
    }

    /// The label of the pill that separates two days of the conversation.
    fn day_separator_label(&self, index: usize) -> Option<String> {
        if !self.starts_a_day(index) {
            return None;
        }
        let timestamp = self.store.messages.get(index)?.timestamp_ms;
        Some(message_day_label(timestamp, now_ms()))
    }

    /// Whether the message begins a new cluster from a sender.
    /// In WhatsApp Desktop, consecutive messages from the same sender within 1 minute
    /// have tight 2px spacing and no repeated bubble tail.
    fn starts_cluster(&self, index: usize) -> bool {
        starts_cluster(&self.store.messages, index)
    }
}

/// Un mensaje de visualizacion unica: el backend manda el tipo de contenido
/// con `view_once` y sin la foto ni el texto.
/// La foto de un mensaje, con lo que el visor necesita para el encabezado y
/// el carrusel. Responde `None` cuando el mensaje no es una foto o cuando el
/// archivo no esta en el disco: el visor no puede mostrar lo que no tiene.
pub(crate) fn viewer_item(message: &proto::Message) -> Option<crate::app::ViewerItem> {
    let Some(proto::message::Content::Image(image)) = message.content.as_ref() else {
        return None;
    };
    let row_path = image_row_path(image);
    let viewer_path = image_viewer_path(image);
    // El visor abre el archivo entero cuando esta en el disco: el thumbnail
    // se ve borroso a pantalla completa. Si no esta, muestra el mismo archivo
    // que la burbuja pinta.
    let path = if Path::new(viewer_path).is_file() {
        viewer_path.to_string()
    } else {
        image_source_candidates(image, &row_path, &viewer_path)
            .into_iter()
            .find(|path| !path.is_empty() && Path::new(*path).is_file())?
            .to_string()
    };
    Some(crate::app::ViewerItem {
        message_id: message.id.clone(),
        path,
        caption: image.caption.clone(),
        sticker: image.sticker,
        width: image.width,
        height: image.height,
        from_me: message.from_me,
        sender_name: message.sender_name.trim().to_string(),
        sender_avatar: message.sender_avatar_path.clone(),
        timestamp_ms: message.timestamp_ms,
    })
}

pub(crate) fn is_view_once_message(message: &proto::Message) -> bool {
    matches!(
        message.content.as_ref(),
        Some(proto::message::Content::Unsupported(unsupported))
            if unsupported.type_name == "view_once"
    )
}

/// Whether the message begins a new cluster from a sender.
fn starts_cluster(messages: &[proto::Message], index: usize) -> bool {
    if index == 0 {
        return true;
    }
    if starts_a_day(messages, index) {
        return true;
    }
    let Some(message) = messages.get(index) else {
        return true;
    };
    let Some(previous) = messages.get(index - 1) else {
        return true;
    };
    if message.from_me != previous.from_me {
        return true;
    }
    if !message.from_me && message.sender_id != previous.sender_id {
        return true;
    }
    (message.timestamp_ms - previous.timestamp_ms).abs() > 60_000
}

/// Whether the message is the first one of its day.
fn starts_a_day(messages: &[proto::Message], index: usize) -> bool {
    let Some(message) = messages.get(index) else {
        return false;
    };
    if message.timestamp_ms == 0 {
        return false;
    }
    let key = message_day_key(message.timestamp_ms);
    match index
        .checked_sub(1)
        .and_then(|previous| messages.get(previous))
    {
        Some(previous) if previous.timestamp_ms != 0 => {
            message_day_key(previous.timestamp_ms) != key
        }
        _ => true,
    }
}

impl Render for Whatfast {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match self.store.screen {
            Screen::Starting => self.render_center(
                "Starting Whatfast",
                "Connecting to the local WhatsApp backend",
            ),
            Screen::Pairing => self.render_pairing(cx),
            Screen::Syncing => self.render_center(
                "Syncing your chats",
                format!(
                    "{} chats · {} messages",
                    self.store.sync_chats, self.store.sync_messages
                ),
            ),
            Screen::Chats => self.render_chats(window, cx),
            Screen::Fatal => self.render_center(
                "Whatfast needs attention",
                self.store
                    .fatal_error
                    .clone()
                    .unwrap_or_else(|| i18n::t("Unknown backend error").into()),
            ),
        };
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &CycleRecentChat, window, cx| {
                this.cycle_recent_chat(false, window, cx);
                cx.stop_propagation();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CycleRecentChatReverse, window, cx| {
                this.cycle_recent_chat(true, window, cx);
                cx.stop_propagation();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleStates, window, cx| {
                this.open_states(window, cx);
                cx.stop_propagation();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleChannels, window, cx| {
                this.open_channels(window, cx);
                cx.stop_propagation();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleArchived, window, cx| {
                this.open_archived(window, cx);
                cx.stop_propagation();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleAgents, window, cx| {
                this.open_agents(window, cx);
                cx.stop_propagation();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                this.focus_search(window, cx);
                cx.stop_propagation();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &OpenQuickAsk, window, cx| {
                this.open_quick_ask(window, cx);
                cx.stop_propagation();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &EscapeAction, window, cx| {
                this.handle_escape(window, cx);
                cx.stop_propagation();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ScrollToBottomAction, window, cx| {
                this.jump_to_bottom(window, cx);
                cx.stop_propagation();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ScrollToTopAction, _window, cx| {
                this.load_older_messages();
                cx.stop_propagation();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &PageUpMessages, _window, cx| {
                this.page_messages(false);
                cx.stop_propagation();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &PageDownMessages, _window, cx| {
                this.page_messages(true);
                cx.stop_propagation();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SelectPreviousChat, window, cx| {
                this.select_adjacent_chat(false, window, cx);
                cx.stop_propagation();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SelectNextChat, window, cx| {
                this.select_adjacent_chat(true, window, cx);
                cx.stop_propagation();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &OpenSettingsAction, window, cx| {
                this.toggle_settings(window, cx);
                cx.stop_propagation();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &FocusComposerAction, window, cx| {
                if this.store.selected_chat_id.is_some() {
                    this.composer
                        .update(cx, |input, cx| input.focus(window, cx));
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .on_modifiers_changed(
                cx.listener(|this, event: &ModifiersChangedEvent, window, cx| {
                    if this.chat_switcher.is_some() && !event.modifiers.control {
                        this.commit_chat_switcher(window, cx);
                        cx.notify();
                    }
                }),
            )
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                // La pantalla de bloqueo se queda con las teclas: los digitos
                // no llegan a la conversacion que hay detras.
                if this.locked {
                    this.lock_key(&event.keystroke.key, cx);
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                if event.keystroke.modifiers.secondary() && event.keystroke.key == "k" {
                    this.focus_search(window, cx);
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                // El menu de la fila se cierra con la tecla de escape.
                if this.chat_context_menu.is_some() && event.keystroke.key == "escape" {
                    this.chat_context_menu = None;
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                // El menu del mensaje tambien.
                if this.message_menu.is_some() && event.keystroke.key == "escape" {
                    this.message_menu = None;
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                // La foto que se esta mirando se cierra con escape, como el
                // resto de las capas que flotan sobre la conversacion.
                if this.image_viewer.is_some() && event.keystroke.key == "escape" {
                    this.image_viewer = None;
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                // Las flechas del teclado recorren el carrusel del visor,
                // como en WhatsApp.
                if this.image_viewer.is_some()
                    && matches!(
                        event.keystroke.key.as_str(),
                        "arrowleft" | "left" | "arrowright" | "right"
                    )
                {
                    let forward = matches!(event.keystroke.key.as_str(), "arrowright" | "right");
                    this.step_image_viewer(forward);
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                if this.status_strip.viewer().is_some() {
                    if event.keystroke.key == "escape" {
                        this.stop_video();
                        this.status_strip.close_viewer(cx);
                        cx.stop_propagation();
                        cx.notify();
                        return;
                    }
                    if event.keystroke.key == "arrowleft" || event.keystroke.key == "left" {
                        this.previous_status_viewer(cx);
                        cx.stop_propagation();
                        return;
                    }
                    if event.keystroke.key == "arrowright" || event.keystroke.key == "right" {
                        this.advance_status_viewer(cx);
                        cx.stop_propagation();
                        return;
                    }
                }
                let composer_focused = this.composer.read(cx).focus_handle(cx).is_focused(window);
                let emoji_focused = this
                    .emoji_search
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window);
                let search_focused = this
                    .search_input
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window);
                if event.keystroke.key == "/"
                    && !event.keystroke.modifiers.modified()
                    && !composer_focused
                    && !emoji_focused
                    && !search_focused
                {
                    this.focus_search(window, cx);
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                if composer_focused && this.mention_popup_visible() {
                    let matches_len = this.mention_matches().len();
                    let key = event.keystroke.key.as_str();
                    let handled = match key {
                        "down" | "up" if matches_len > 0 => {
                            if let Some(picker) = this.mention_picker.as_mut() {
                                picker.highlighted = if key == "down" {
                                    (picker.highlighted + 1) % matches_len
                                } else {
                                    (picker.highlighted + matches_len - 1) % matches_len
                                };
                            }
                            true
                        }
                        "enter" | "tab" if matches_len > 0 => {
                            let index = this
                                .mention_picker
                                .as_ref()
                                .map_or(0, |picker| picker.highlighted.min(matches_len - 1));
                            this.apply_mention(index, window, cx);
                            true
                        }
                        "escape" => {
                            this.mention_picker = None;
                            true
                        }
                        _ => false,
                    };
                    if handled {
                        cx.stop_propagation();
                        cx.notify();
                        return;
                    }
                }
                if search_focused && this.search_query.trim().chars().count() >= 2 {
                    match event.keystroke.key.as_str() {
                        "down" => this.move_search_selection(false),
                        "up" => this.move_search_selection(true),
                        "enter" => this.activate_search_result(window, cx),
                        "escape" => this.clear_search(window, cx),
                        _ => return,
                    }
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                if this.chat_info.is_some() && event.keystroke.key == "escape" {
                    this.chat_info = None;
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                if this.chat_switcher.is_some() && event.keystroke.key == "escape" {
                    this.cancel_chat_switcher(window, cx);
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .child(content)
            .when_some(self.store.toast_info.clone(), |root, info| {
                root.child(
                    div()
                        .absolute()
                        .bottom_4()
                        .right_4()
                        .max_w(px(420.))
                        .p_3()
                        .rounded_lg()
                        .bg(rgb(0x14532d))
                        .text_color(rgb(0xffffff))
                        .child(info),
                )
            })
            .when_some(self.store.toast_error.clone(), |root, error| {
                root.child(
                    div()
                        .absolute()
                        .bottom_4()
                        .right_4()
                        .max_w(px(420.))
                        .p_3()
                        .rounded_lg()
                        .bg(rgb(0x7f1d1d))
                        .text_color(rgb(0xffffff))
                        .child(error),
                )
            })
            .when(self.reaction_details.is_some(), |root| {
                root.child(self.render_reaction_details(cx))
            })
            .when(self.video_fullscreen, |root| {
                root.child(self.render_video_viewer(window, cx))
            })
            .when(self.image_viewer.is_some(), |root| {
                root.child(self.render_image_viewer(window, cx))
            })
            .when(self.image_context_menu.is_some(), |root| {
                root.child(self.render_image_menu(window, cx))
            })
            .when(self.confirm_logout_open, |root| {
                root.child(self.render_logout_confirm(window, cx))
            })
            .when(self.view_once_info_open, |root| {
                root.child(self.render_view_once_info(cx))
            })
            .when(self.create_group.is_some(), |root| {
                root.child(self.render_create_group(window, cx))
            })
            .when(self.status_strip.viewer().is_some(), |root| {
                if let Some(viewer) = self.status_strip.render_viewer(self, cx) {
                    root.child(viewer)
                } else {
                    root
                }
            })
            .when(self.settings_open, |root| {
                root.child(self.render_settings(cx))
            })
            .when(self.chat_info.is_some(), |root| {
                root.child(self.render_chat_info(cx))
            })
            .when(self.chat_context_menu.is_some(), |root| {
                root.child(self.render_chat_context_menu(window, cx))
            })
            .when(self.chat_menu.is_some(), |root| {
                root.child(self.render_chat_menu(window, cx))
            })
            .when(self.chat_switcher.is_some(), |root| {
                root.child(self.render_chat_switcher(cx))
            })
            // La tarjeta del modo rapido flota sobre la conversacion: la
            // respuesta va al cuadro de escribir, asi que el chat se sigue
            // leyendo detras.
            .when(self.quick_ask.is_open(), |root| {
                root.child(self.quick_ask.render(self, cx))
            })
            .when_some(self.media_editor.clone(), |root, editor| root.child(editor))
            .when_some(self.message_menu.clone(), |root, menu| {
                // El fondo que cierra el menu con un clic fuera de el. Va antes
                // que el panel para que el panel quede arriba y reciba el clic.
                root.child(
                    div()
                        .id("message-menu-backdrop")
                        .absolute()
                        .inset_0()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.message_menu = None;
                            cx.stop_propagation();
                            cx.notify();
                        }))
                        .on_aux_click(cx.listener(|this, _, _, cx| {
                            this.message_menu = None;
                            cx.stop_propagation();
                            cx.notify();
                        })),
                )
                .child(menu)
            })
            .when_some(self.forward_picker.clone(), |root, picker| {
                root.child(picker)
            })
            .when_some(self.poll_composer.clone(), |root, composer| {
                root.child(composer)
            })
            // Las dos capas del bloqueo van al final: nada se dibuja sobre la
            // pantalla del PIN.
            .when(self.pin_dialog.is_some(), |root| {
                root.child(self.render_pin_dialog(cx))
            })
            .when(self.locked, |root| {
                root.child(self.render_app_lock(window, cx))
            })
    }
}

/// The message the column shows when the filter keeps no conversation.
///
/// port: `emptyLabel` of the origin (`tauri/src/components/Sidebar.tsx`).
/// The drawing of a category of emojis.
///
/// The reference draws a row of categories at the top of the panel, each one
/// with its own drawing.
fn emoji_category_glyph(category: EmojiCategory) -> &'static str {
    match category {
        EmojiCategory::All => "🕐",
        EmojiCategory::Smileys => "😀",
        EmojiCategory::People => "👋",
        EmojiCategory::Nature => "🌿",
        EmojiCategory::Food => "🍎",
        EmojiCategory::Travel => "✈️",
        EmojiCategory::Activities => "⚽",
        EmojiCategory::Objects => "💡",
        EmojiCategory::Symbols => "❤️",
        EmojiCategory::Flags => "🚩",
    }
}

fn empty_label(filter: ChatFilter) -> &'static str {
    i18n::t(match filter {
        ChatFilter::All => "No conversations yet",
        ChatFilter::Unread => "No unread conversations",
        ChatFilter::Favorites => "No favorite chats yet",
        ChatFilter::Groups => "No groups",
        ChatFilter::Archived => "No archived conversations",
    })
}

/// The current instant, in milliseconds of the epoch.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(0)
}

/// La altura de la etiqueta que separa las secciones del buscador.
const SEARCH_HEADER_HEIGHT: f32 = 42.;

/// La altura de un resultado de conversacion o de mensaje.
const SEARCH_RESULT_HEIGHT: f32 = 76.;

/// La altura de un contacto del buscador: el nombre es la unica linea.
const SEARCH_CONTACT_HEIGHT: f32 = 68.;

/// Las partes de un texto que el buscador marco, como rangos de bytes.
///
/// La consulta entera gana cuando esta en el texto. Cuando no esta —el store
/// encontro una errata o un fragmento— se marca el trozo mas largo que los dos
/// comparten, que es lo que resalta la referencia en sus coincidencias difusas.
fn search_match_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    let query: Vec<char> = query.trim().chars().collect();
    if query.is_empty() || text.is_empty() {
        return Vec::new();
    }
    let characters: Vec<char> = text.chars().collect();
    let mut found = occurrences(&characters, &query);
    if found.is_empty() {
        let Some(fragment) = longest_common_fragment(&characters, &query) else {
            return Vec::new();
        };
        found = occurrences(&characters, &fragment);
    }
    let offsets: Vec<usize> = text
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(text.len()))
        .collect();
    found
        .into_iter()
        .map(|(start, end)| offsets[start]..offsets[end])
        .collect()
}

/// La minuscula de un caracter, como la compara el buscador.
fn search_lower(character: char) -> char {
    character.to_lowercase().next().unwrap_or(character)
}

/// Las posiciones de `needle` dentro de `haystack`, de izquierda a derecha y
/// sin solaparse.
fn occurrences(haystack: &[char], needle: &[char]) -> Vec<(usize, usize)> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut index = 0;
    while index + needle.len() <= haystack.len() {
        if needle.iter().enumerate().all(|(offset, character)| {
            search_lower(haystack[index + offset]) == search_lower(*character)
        }) {
            found.push((index, index + needle.len()));
            index += needle.len();
        } else {
            index += 1;
        }
    }
    found
}

/// El trozo mas largo de caracteres que el texto y la consulta comparten,
/// cuando es lo bastante largo como para merecer un resaltado.
fn longest_common_fragment(text: &[char], query: &[char]) -> Option<Vec<char>> {
    let mut best = (0usize, 0usize);
    let mut previous = vec![0usize; text.len() + 1];
    let mut current = vec![0usize; text.len() + 1];
    for character in query {
        for (index, candidate) in text.iter().enumerate() {
            current[index + 1] = if search_lower(*character) == search_lower(*candidate) {
                previous[index] + 1
            } else {
                0
            };
            if current[index + 1] > best.0 {
                best = (current[index + 1], index + 1);
            }
        }
        std::mem::swap(&mut previous, &mut current);
        current.fill(0);
    }
    let minimum = query.len().clamp(2, 3);
    (best.0 >= minimum).then(|| text[best.1 - best.0..best.1].to_vec())
}

/// El texto de una fila con las partes que coinciden con la consulta en el
/// color del acento, como la referencia pinta su propio buscador.
fn search_highlighted(text: &str, query: &str, accent: gpui::Hsla) -> gpui::StyledText {
    let ranges = search_match_ranges(text, query);
    gpui::StyledText::new(text.to_string()).with_highlights(ranges.into_iter().map(|range| {
        (
            range,
            gpui::HighlightStyle {
                color: Some(accent),
                ..Default::default()
            },
        )
    }))
}

/// The time of the last message of a conversation, the way the reference
/// writes it at the right of the title.
///
/// port: `formatChatTime` of the origin (`tauri/src/lib/format.ts`): the
/// hour of the day, "Yesterday", the day of the week, or the short date.
/// The hour is the one of the machine: the offset of the zone is asked for
/// both instants, because a zone with daylight saving time does not have the
/// same offset in the day of the message and in the day of "now".
fn chat_time_label(timestamp_ms: i64, now_ms: i64) -> String {
    use whatfast_core::message_format::{DateLocale, LocalTimestamp, day_key, format_chat_time};
    let offset = local_offset_minutes(timestamp_ms);
    let now_offset = local_offset_minutes(now_ms);
    let value = LocalTimestamp::new(timestamp_ms, offset);
    let now = LocalTimestamp::new(now_ms, now_offset);
    // El mensaje de hoy lleva la hora de la conversacion, en el mismo reloj de
    // veinticuatro horas que la burbuja: "12:53" y no "12:53 AM", que es como
    // lo escribe la referencia. Los dias anteriores siguen con la palabra del
    // dia o la fecha.
    if day_key(value) == day_key(now) {
        return bubble_time_label(timestamp_ms, offset);
    }
    format_chat_time(value, now, DateLocale::EnUs)
}

/// El texto guardado de un aviso de visualizacion unica viaja en ingles
/// porque es la clave del i18n: la lista de chats lo muestra traducido.
fn chat_preview(preview: &str) -> String {
    if preview == "One-time message" {
        i18n::t("One-time message").to_string()
    } else {
        preview.to_string()
    }
}

/// The line under the title of a conversation.
///
/// port: `chatSubtitle` of the origin (`tauri/src/lib/format.ts`): the
/// preview of the last message, the phone number when there is no preview,
/// and the mark of the messages the user sent.
fn chat_subtitle(chat: &proto::Chat) -> String {
    let number = whatfast_core::phone::format(&chat.phone_number);
    let preview = if !chat.last_message_preview.is_empty() {
        chat_preview(&chat.last_message_preview)
    } else if !number.is_empty() && number != chat.title {
        number
    } else {
        // The title is already the number of a contact that is not in the
        // address book: the same number twice says nothing.
        "No messages yet".to_string()
    };
    if !chat.last_message_from_me || chat.last_message_preview.is_empty() {
        return preview;
    }
    format!("You: {preview}")
}

/// The height of the pill that separates two days of the conversation.
const DAY_SEPARATOR_HEIGHT: f32 = 22.;

/// The gap between the pill of the day and the message below it.
const DAY_SEPARATOR_GAP: f32 = 8.;

/// The extra gap above a message that starts a new cluster from a sender.
const CLUSTER_SEPARATOR_GAP: f32 = 6.;

/// The opacity of the pattern of the background of the conversation.
const WALLPAPER_OPACITY: f32 = 0.055;

/// The distance between two glyphs of the pattern, from left to right.
const WALLPAPER_STEP_X: Pixels = px(120.);

/// The distance between two glyphs of the pattern, from top to bottom.
const WALLPAPER_STEP_Y: Pixels = px(104.);

/// The size of a glyph of the pattern.
const WALLPAPER_GLYPH_SIZE: Pixels = px(54.);

/// The size of the tail of a bubble, in unscaled pixels.
const BUBBLE_TAIL_SIZE: f32 = 14.;

/// The width of the notice of a view-once message, in unscaled pixels.
const VIEW_ONCE_WIDTH: f32 = 330.;

/// The circle of the "1" of the notice, in unscaled pixels.
const VIEW_ONCE_BADGE: f32 = 22.;

/// Los colores del visor de fotos: negro neutro, para no alterar el color de
/// la foto, con las barras apenas mas claras.
const VIEWER_BACKGROUND: u32 = 0x0c0c0c;
const VIEWER_BAR: u32 = 0x1c1c1c;

/// El gris claro de la fecha y de la posicion en el visor.
const VIEWER_MUTED: u32 = 0xa8a8a8;

/// El verde de WhatsApp con el que el carrusel marca la foto activa.
const VIEWER_ACCENT: u32 = 0x00a884;

/// Las reacciones rapidas que ofrece el visor, en el orden de WhatsApp.
const VIEWER_QUICK_REACTIONS: [&str; 6] = ["👍", "❤️", "😂", "😮", "😢", "🙏"];

/// El boton de icono de la barra del visor: sin fondo, con el icono claro que
/// se enciende al pasarle el cursor.
fn viewer_icon_button(
    id: (&'static str, usize),
    icon: Icon,
    scale: f32,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .size(px(34. * scale))
        .flex_none()
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|style| style.bg(rgba(0xffffff1f)))
        .child(icon.with_size(px(19. * scale)).text_color(rgb(0xe9edef)))
}

/// La flecha circular que pasa a la foto anterior o siguiente.
fn viewer_arrow_button(
    id: (&'static str, usize),
    back: bool,
    scale: f32,
) -> gpui::Stateful<gpui::Div> {
    let icon = if back {
        IconName::ChevronLeft
    } else {
        IconName::ChevronRight
    };
    div()
        .id(id)
        .size(px(40. * scale))
        .rounded_full()
        .bg(rgba(0xffffff14))
        .hover(|style| style.bg(rgba(0xffffff2b)))
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .child(
            Icon::new(icon)
                .with_size(px(22. * scale))
                .text_color(rgb(0xffffff)),
        )
}

/// Una fila del menu del visor: el icono, la accion y el fondo que se enciende
/// al pasarle el cursor.
fn image_menu_row(
    id: (&'static str, usize),
    icon: Icon,
    label: &'static str,
    hover: gpui::Rgba,
    scale: f32,
) -> gpui::Stateful<gpui::Div> {
    h_flex()
        .id(id)
        .h(px(36. * scale))
        .px(px(14. * scale))
        .gap_3()
        .items_center()
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(icon.with_size(px(18. * scale)).text_color(rgb(0xe9edef)))
        .child(div().text_sm().child(label))
}

/// Un dato de la tarjeta de la info del mensaje: el rotulo arriba, en gris, y
/// el valor abajo, en blanco.
fn viewer_info_row(label: &'static str, value: String) -> gpui::Div {
    v_flex()
        .gap_1()
        .child(div().text_xs().text_color(rgb(VIEWER_MUTED)).child(label))
        .child(div().text_sm().text_color(rgb(0xffffff)).child(value))
}

/// El estado del mensaje de la foto, escrito como lo escribe WhatsApp.
fn viewer_status_label(status: i32) -> &'static str {
    match proto::MessageStatus::try_from(status).unwrap_or_default() {
        proto::MessageStatus::Pending => i18n::t("Pending"),
        proto::MessageStatus::Sent => i18n::t("Sent"),
        proto::MessageStatus::Delivered => i18n::t("Delivered"),
        proto::MessageStatus::Read => i18n::t("Read"),
        proto::MessageStatus::Failed => i18n::t("Failed"),
        _ => i18n::t("Unknown"),
    }
}

/// How far the tail of a bubble sticks out of its corner, in unscaled pixels.
const BUBBLE_TAIL_OFFSET: f32 = -6.;

/// El ancho maximo del texto de un mensaje, sin escalar.
///
/// Es la burbuja de la referencia (560) menos el relleno de los dos lados y
/// el hueco de la hora, para que la hora entre al final de la ultima linea y
/// el texto envuelva antes de salirse de la burbuja.
const TEXT_MAX_WIDTH: f32 = 512.;

/// The hour of a message, the way the reference writes it inside the bubble.
///
/// The reference writes the hour of the twenty-four hours ("10:45"), in the
/// zone of the machine. The offset is a parameter so the test can fix it: the
/// answer of `local_offset_minutes` is the zone of the machine that runs it.
fn bubble_time_label(timestamp_ms: i64, utc_offset_minutes: i32) -> String {
    let local = timestamp_ms + i64::from(utc_offset_minutes) * 60_000;
    let millis = local.rem_euclid(86_400_000);
    format!("{:02}:{:02}", millis / 3_600_000, (millis / 60_000) % 60)
}

/// The tick of the delivery state of an outgoing message: the clock while
/// it goes out, one tick for the send, two for the delivery and two blue for
/// the read, the same way WhatsApp draws them.
fn status_tick(glyph: StatusGlyph, color: gpui::Hsla, scale: f32) -> gpui::AnyElement {
    let icon = match glyph {
        StatusGlyph::Clock => WhatfastIcon::Clock,
        StatusGlyph::Single => WhatfastIcon::Check,
        StatusGlyph::Double | StatusGlyph::Read => WhatfastIcon::CheckDouble,
        StatusGlyph::Failed => WhatfastIcon::Warning,
    };
    Icon::new(icon)
        .with_size(px(15. * scale))
        .text_color(color)
        .into_any_element()
}

/// The width the hour of a message takes inside its bubble.
///
/// The measure is done with the text system of the window, at the size of
/// the small text of the interface. The gap between the text and the hour
/// comes with it, and a couple of pixels of margin: the text of the bubble
/// has to wrap before the hour, and a measure that is too small would put
/// the text under the hour.
fn bubble_time_width(window: &mut Window, label: &str, font_size: Pixels) -> Pixels {
    let line_height = window.line_height();
    let size = font_size * 0.75;
    let text: SharedString = label.into();
    let run = window.text_style().to_run(text.len());
    window
        .text_system()
        .shape_text(text, size, &[run], None, None)
        .map(|lines| {
            lines
                .iter()
                .fold(px(0.), |width, line| width + line.size(line_height).width)
        })
        .unwrap_or(px(0.))
        // The gap of the row that holds the text and the hour.
        + font_size * 0.25
        // The margin of the measure.
        + px(2.)
}

/// The tail of a bubble: the point of its corner.
///
/// The reference paints a small triangle at the top corner of every bubble:
/// at the left of the received ones and at the right of the sent ones. The
/// tail has the same colour as the bubble.
fn bubble_tail(color: gpui::Hsla, from_me: bool, scale: f32) -> impl IntoElement {
    canvas(
        move |bounds, _, _| bounds,
        move |bounds, _, window, _| {
            let size = bounds.size.width;
            let mut path = gpui::PathBuilder::fill();
            if from_me {
                path.move_to(point(bounds.origin.x + size, bounds.origin.y));
                path.line_to(point(bounds.origin.x + size, bounds.origin.y + size));
                path.line_to(point(bounds.origin.x, bounds.origin.y + size));
            } else {
                path.move_to(point(bounds.origin.x, bounds.origin.y));
                path.line_to(point(bounds.origin.x, bounds.origin.y + size));
                path.line_to(point(bounds.origin.x + size, bounds.origin.y + size));
            }
            path.close();
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size(px(BUBBLE_TAIL_SIZE * scale))
}

/// The glyphs of the pattern of the background of the conversation.
fn wallpaper_glyphs() -> Vec<Icon> {
    vec![
        Icon::new(WhatfastIcon::Smile),
        Icon::new(IconName::Heart),
        Icon::new(IconName::Star),
        Icon::new(WhatfastIcon::Bell),
        Icon::new(WhatfastIcon::Clock),
        Icon::new(WhatfastIcon::Globe),
        Icon::new(WhatfastIcon::Chat),
        Icon::new(IconName::Moon),
    ]
}

/// The key of the day of a message.
///
/// port: `dayKey` of the origin (`tauri/src/lib/format.ts`). The day is the
/// one of the machine: a message of 23:30 in Buenos Aires is of that day and
/// not of the next one, which is the day it has in UTC.
fn message_day_key(timestamp_ms: i64) -> String {
    whatfast_core::message_format::day_key(whatfast_core::message_format::LocalTimestamp::new(
        timestamp_ms,
        local_offset_minutes(timestamp_ms),
    ))
}

/// The day of a message, the way the reference writes it in the pill that
/// separates two days.
///
/// port: `formatDay` of the origin (`tauri/src/lib/format.ts`).
fn message_day_label(timestamp_ms: i64, now_ms: i64) -> String {
    whatfast_core::message_format::format_day(
        whatfast_core::message_format::LocalTimestamp::new(
            timestamp_ms,
            local_offset_minutes(timestamp_ms),
        ),
        whatfast_core::message_format::LocalTimestamp::new(now_ms, local_offset_minutes(now_ms)),
        whatfast_core::message_format::DateLocale::EnUs,
    )
}

/// Escapa el texto de un mensaje para el HTML de la vista de texto.
fn escape_html(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            other => escaped.push(other),
        }
    }
    escaped
}

fn resolve_mention<'a>(
    token: &'a str,
    participants: Option<&[proto::ChatParticipant]>,
    chats: &[proto::Chat],
) -> Option<(&'a str, String, &'a str)> {
    let at_offset = token.find('@')?;
    let prefix = &token[..at_offset];
    let rest = &token[at_offset + 1..];
    if rest.is_empty() {
        return None;
    }
    let id_len = rest
        .find(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
        .unwrap_or(rest.len());
    if id_len == 0 {
        return None;
    }
    let candidate_id = &rest[..id_len];
    let suffix = &rest[id_len..];

    let matched_name = participants.and_then(|members| {
        members.iter().find_map(|p| {
            let p_id = p
                .participant_id
                .split('@')
                .next()
                .unwrap_or("")
                .split(':')
                .next()
                .unwrap_or("");
            let phone = p.phone_number.replace(['+', ' ', '-'], "");
            if (!p_id.is_empty() && p_id == candidate_id)
                || (!phone.is_empty() && (phone == candidate_id || candidate_id.ends_with(&phone)))
            {
                let name = p.display_name.trim();
                if !name.is_empty() {
                    Some(name.to_string())
                } else if !p.phone_number.trim().is_empty() {
                    Some(p.phone_number.trim().to_string())
                } else {
                    Some(p_id.to_string())
                }
            } else {
                None
            }
        })
    });

    let matched_name = matched_name.or_else(|| {
        chats.iter().find_map(|c| {
            let c_id =
                c.id.split('@')
                    .next()
                    .unwrap_or("")
                    .split(':')
                    .next()
                    .unwrap_or("");
            if !c_id.is_empty() && c_id == candidate_id && !c.title.trim().is_empty() {
                Some(c.title.trim().to_string())
            } else {
                None
            }
        })
    });

    matched_name.map(|name| (prefix, name, suffix))
}

/// El HTML del texto de un mensaje: los saltos de linea, los espacios, cada
/// enlace en su ancla y menciones a participantes o contactos.
fn message_html_with_mentions(
    text: &str,
    participants: Option<&[proto::ChatParticipant]>,
    chats: &[proto::Chat],
) -> String {
    let mut html = String::new();
    // WhatsApp mixes CRLF, LF and a lone CR. The text view reads the CR as a
    // line break, but the measure of the bubble only knows `\n`: a message
    // with a lone CR measured one line and the rows after it overlapped.
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    for raw_line in text.split('\n') {
        // El tabulador no ocupa ancho en la vista de texto: se expande a
        // cuatro espacios para que la sangria se vea como se pego.
        let line = raw_line.replace('\t', "    ");
        // El cuerpo va dentro de `<pre>`: la vista de texto minifica el HTML
        // antes de leerlo y, fuera de `<pre>`, colapsa los espacios y la
        // sangria. El `<div>` es el que separa los renglones.
        html.push_str("<div><pre>");
        if line.trim().is_empty() {
            // Un renglon en blanco se mide con alto cero: el espacio de ancho
            // cero le da la altura de una linea sin sumar algo que se vea.
            html.push_str("&#8203;");
            // La sangria de un renglon en blanco se respeta igual.
            html.push_str(&escape_html(&line));
        } else {
            // `split(' ')` conserva los espacios vacios, asi que los espacios
            // repetidos y la sangria vuelven a salir tal como estaban.
            for (token_index, token) in line.split(' ').enumerate() {
                if token_index > 0 {
                    html.push(' ');
                }
                if let Some((prefix, url, suffix)) = split_url_token(token) {
                    html.push_str(&escape_html(prefix));
                    html.push_str("<a href=\"");
                    html.push_str(&escape_html(url));
                    html.push_str("\">");
                    html.push_str(&escape_html(url));
                    html.push_str("</a>");
                    html.push_str(&escape_html(suffix));
                } else if let Some((prefix, mention_name, suffix)) =
                    resolve_mention(token, participants, chats)
                {
                    html.push_str(&escape_html(prefix));
                    html.push_str("<a href=\"#\">@");
                    html.push_str(&escape_html(&mention_name));
                    html.push_str("</a>");
                    html.push_str(&escape_html(suffix));
                } else {
                    html.push_str(&escape_html(token));
                }
            }
        }
        html.push_str("</pre></div>");
    }
    html
}

#[cfg(test)]
fn message_html(text: &str) -> String {
    message_html_with_mentions(text, None, &[])
}

/// La barra de una llamada viva: quien llama, la fase, silencio y cortar; o
/// contestar y rechazar cuando suena una entrante.
fn render_call_bar(
    this: &mut Whatfast,
    call: &whatfast_model::model::CallState,
    cx: &mut gpui::Context<Whatfast>,
) -> gpui::AnyElement {
    use gpui::{black, white};
    let phase_label: SharedString = match call.phase.as_str() {
        "placing" | "dialing" => i18n::t("Calling…").into(),
        "ringing" => i18n::t("Ringing…").into(),
        "incoming" => i18n::t("Incoming call").into(),
        "connecting" => i18n::t("Connecting…").into(),
        "active" => i18n::t("In call").into(),
        other => other.to_owned().into(),
    };
    // El nombre del contacto: la barra flota sobre cualquier pantalla y sin
    // el no se sabe a quien se esta llamando. La llamada trae el id del chat
    // y el chat trae el titulo que muestra la columna.
    let name: SharedString = this
        .store
        .chat(&call.chat_id)
        .map(|chat| chat.title.trim())
        .filter(|title| !title.is_empty())
        .map(SharedString::from)
        .unwrap_or_else(|| phase_label.clone());
    let theme = cx.theme().clone();
    let pill = h_flex()
        .gap_2()
        .px_3()
        .py_2()
        .rounded_lg()
        .shadow_lg()
        .bg(black().opacity(0.85))
        .text_color(white())
        // El icono dice si la llamada lleva video desde el primer timbre.
        .child(Icon::new(if call.video {
            WhatfastIcon::Video
        } else {
            WhatfastIcon::Phone
        }))
        .child(name.clone())
        // La fase solo se agrega cuando hay un nombre: sin nombre ya es la
        // linea principal.
        .when(name != phase_label, |row| {
            row.child(
                div()
                    .text_xs()
                    .text_color(white().opacity(0.7))
                    .child(phase_label.clone()),
            )
        });
    let pill = if call.incoming && call.phase == "incoming" {
        pill.child(
            Button::new("call-answer")
                .small()
                .label(i18n::t("Answer"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.call_accept();
                    cx.notify();
                })),
        )
        .child(
            Button::new("call-decline")
                .small()
                .label(i18n::t("Decline"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.call_reject();
                    cx.notify();
                })),
        )
    } else {
        pill.child(
            Button::new("call-mute")
                .small()
                .label(if call.muted {
                    SharedString::from(i18n::t("Unmute"))
                } else {
                    SharedString::from(i18n::t("Mute"))
                })
                .on_click(cx.listener(|this, _, _, cx| {
                    this.call_toggle_mute();
                    cx.notify();
                })),
        )
        // La camara solo existe cuando la llamada lleva video y la media ya
        // esta arriba: apagarla corta el envio, no la llamada.
        .when(
            call.video && matches!(call.phase.as_str(), "connecting" | "active"),
            |row| {
                let camera_on = this.call_video.camera_on;
                row.child(
                    Button::new("call-camera")
                        .small()
                        .label(if camera_on {
                            SharedString::from(i18n::t("Camera off"))
                        } else {
                            SharedString::from(i18n::t("Camera on"))
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.call_toggle_camera();
                            cx.notify();
                        })),
                )
            },
        )
        .child(
            Button::new("call-hangup")
                .small()
                .label(i18n::t("Hang up"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.call_hangup();
                    cx.notify();
                })),
        )
    };
    let _ = theme;
    pill.into_any_element()
}

/// El panel del video de una llamada: la imagen del otro lado, la vista chica
/// de la camara propia y, cuando algo falla, el motivo.
fn render_call_video(this: &mut Whatfast, _cx: &mut gpui::Context<Whatfast>) -> gpui::AnyElement {
    let scale = this.ui_scale;
    let remote = this.call_video.remote.clone();
    let local = this.call_video.local.clone();
    let error = this.call_video.error.clone();
    let has_remote = remote.is_some();
    let width = 480. * scale;
    let height = 270. * scale;
    div()
        .relative()
        .w(px(width))
        .h(px(height))
        .rounded_lg()
        .overflow_hidden()
        .shadow_lg()
        .bg(rgb(0x111111))
        .when_some(remote, |panel, frame| {
            panel.child(img(frame).w_full().h_full().object_fit(ObjectFit::Cover))
        })
        .when(!has_remote, |panel| {
            panel.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .child(
                        Icon::new(WhatfastIcon::Video)
                            .with_size(px(34. * scale))
                            .text_color(rgb(0xffffff66)),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0xffffff99))
                            .child(i18n::t("Waiting for video")),
                    ),
            )
        })
        // La camara propia, chica, sobre la esquina de la imagen del otro.
        .when_some(local, |panel, frame| {
            panel.child(
                div()
                    .absolute()
                    .bottom(px(8. * scale))
                    .right(px(8. * scale))
                    .w(px(144. * scale))
                    .h(px(81. * scale))
                    .rounded_md()
                    .overflow_hidden()
                    .border_1()
                    .border_color(rgba(0xffffff44))
                    .bg(rgb(0x000000))
                    .child(img(frame).w_full().h_full().object_fit(ObjectFit::Cover)),
            )
        })
        .when_some(error, |panel, message| {
            panel.child(
                div()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .px_2()
                    .py_1()
                    .bg(rgba(0x000000b3))
                    .text_xs()
                    .text_color(rgb(0xffffffcc))
                    .child(message),
            )
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{bubble_time_label, message_html, search_match_ranges};

    #[test]
    fn the_search_marks_the_query_and_its_fuzzy_fragment() {
        let text = "EL MERCADO DEL CERDO";
        let ranges = search_match_ranges(text, "cerdo");
        assert_eq!(ranges, vec![15..20], "the whole query wins");
        assert_eq!(&text[15..20], "CERDO");

        // The store matches "cerda" against "cerdo"; the row marks the run
        // both share, like the reference.
        let text = "Ya esta cerda";
        let ranges = search_match_ranges(text, "cerdo");
        assert_eq!(ranges, vec![8..12]);
        assert_eq!(&text[8..12], "cerd");

        // A text with nothing in common keeps its color.
        assert!(search_match_ranges("Papa", "cerdo").is_empty());
    }

    #[test]
    fn the_html_of_a_message_escapes_the_markup() {
        assert_eq!(
            message_html("<b> & \"x\""),
            "<div><pre>&lt;b&gt; &amp; &quot;x&quot;</pre></div>"
        );
    }

    #[test]
    fn the_html_of_a_message_splits_a_carriage_return_in_lines() {
        // WhatsApp sends some bodies with a lone CR between the lines.
        assert_eq!(
            message_html("uno\rdos"),
            "<div><pre>uno</pre></div><div><pre>dos</pre></div>"
        );
        assert_eq!(
            message_html("uno\r\ndos"),
            "<div><pre>uno</pre></div><div><pre>dos</pre></div>"
        );
    }

    #[test]
    fn the_html_of_a_message_keeps_one_block_per_line() {
        assert_eq!(
            message_html("uno\ndos"),
            "<div><pre>uno</pre></div><div><pre>dos</pre></div>"
        );
    }

    #[test]
    fn the_html_of_a_message_keeps_the_spacing_it_was_pasted_with() {
        // La forma del texto pegado: sangria, columnas y el renglon en blanco.
        assert_eq!(
            message_html("Presupuesto:\n\n    Item uno        Q 1.500"),
            "<div><pre>Presupuesto:</pre></div><div><pre>&#8203;</pre></div>\
             <div><pre>    Item uno        Q 1.500</pre></div>"
        );
    }

    #[test]
    fn the_html_of_a_message_wraps_the_links_in_anchors() {
        assert_eq!(
            message_html("ver https://example.com ahora"),
            "<div><pre>ver <a href=\"https://example.com\">https://example.com</a> ahora</pre></div>"
        );
    }

    #[test]
    fn the_hour_of_a_bubble_is_the_hour_of_the_zone_of_the_offset() {
        // The 15:45 of UTC is the 12:45 of Buenos Aires.
        let instant = 15 * 3_600_000 + 45 * 60_000;
        assert_eq!(bubble_time_label(instant, 0), "15:45");
        assert_eq!(bubble_time_label(instant, -180), "12:45");
        assert_eq!(bubble_time_label(instant, 120), "17:45");
    }

    #[test]
    fn an_hour_close_to_midnight_moves_to_the_day_of_the_zone() {
        // The 02:10 of UTC is the 23:10 of the day before in Buenos Aires,
        // and the 04:10 of the same day in Berlin.
        let instant = 2 * 3_600_000 + 10 * 60_000;
        assert_eq!(bubble_time_label(instant, -180), "23:10");
        assert_eq!(bubble_time_label(instant, 120), "04:10");
    }

    #[test]
    fn the_hour_of_a_bubble_before_the_epoch_is_the_one_of_the_zone() {
        // The epoch is midnight of UTC, so the day before in Buenos Aires.
        assert_eq!(bubble_time_label(0, -180), "21:00");
        assert_eq!(bubble_time_label(0, 0), "00:00");
    }

    #[test]
    fn the_html_of_a_message_resolves_mentions() {
        use super::message_html_with_mentions;
        let participant = whatfast_protocol::ChatParticipant {
            participant_id: "107503115358282@lid".to_string(),
            display_name: "Mauricio Rodríguez".to_string(),
            phone_number: "+54 9 11 1234 5678".to_string(),
            is_admin: false,
            is_super_admin: false,
            is_me: false,
        };
        let html = message_html_with_mentions(
            "@107503115358282 me dijo que no me gaste",
            Some(&[participant]),
            &[],
        );
        assert_eq!(
            html,
            "<div><pre><a href=\"#\">@Mauricio Rodríguez</a> me dijo que no me gaste</pre></div>"
        );
    }

    #[test]
    fn consecutive_messages_from_same_sender_do_not_start_cluster() {
        use whatfast_model::model::Store;
        use whatfast_protocol::{Message, TextContent, message::Content};

        let mut store = Store::default();
        store.selected_chat_id = Some("chat1".to_string());
        for (i, text) in [
            "Bueno amor",
            "Dormi mi bebe",
            "A la noche ya entrenamos",
            "Cuanto??",
        ]
        .iter()
        .enumerate()
        {
            let msg = Message {
                id: format!("msg_{i}"),
                chat_id: "chat1".to_string(),
                from_me: true,
                timestamp_ms: 1700000000000 + i as i64 * 5000,
                content: Some(Content::Text(TextContent {
                    text: text.to_string(),
                    ..Default::default()
                })),
                ..Default::default()
            };
            store.upsert_message(msg);
        }
        assert!(super::starts_cluster(&store.messages, 0));
        assert!(!super::starts_cluster(&store.messages, 1));
        assert!(!super::starts_cluster(&store.messages, 2));
        assert!(!super::starts_cluster(&store.messages, 3));
    }
}
