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
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    ClickEvent, Context, ElementId, Focusable as _, FontWeight, InteractiveElement as _,
    IntoElement, KeyDownEvent, ModifiersChangedEvent, ObjectFit, ParentElement as _, Pixels,
    Render, ScrollStrategy, SharedString, StatefulInteractiveElement as _, Styled as _,
    StyledImage as _, Window, canvas, div, img, point, px, rgb, rgba, size, uniform_list,
};
use gpui_component::{
    ActiveTheme as _, Colorize as _, Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    scroll::{ScrollableElement as _, ScrollbarAxis},
    v_flex, v_virtual_list,
};
use qrcode::QrCode;
// The origin's `crate::proto` is now the protocol crate: the same types, with
// the same names, at the root of the crate.
use whatfast_protocol as proto;

use crate::attachments;
use crate::icons::WhatfastIcon;

use whatfast_model::emoji_picker::EmojiCategory;
use whatfast_model::geom;
use whatfast_model::model::{
    ChatFilter, Screen, image_row_path, image_viewer_path, reaction_counts, reaction_sender_detail,
    reaction_sender_name,
};

use crate::app::{
    ChatContextAction, ChatMenuAction, CycleRecentChat, CycleRecentChatReverse, DARK_GREEN,
    EMOJI_COLUMNS, EmojiTarget, MAX_UI_SCALE, MIN_UI_SCALE, PANEL, PanelTab, ScrollSurface,
    SearchResultRow, ToggleArchived, ToggleChannels, ToggleStates, UI_SCALE_STEP, Whatfast,
};
use crate::emoji_recent;
use crate::helpers::{
    avatar_element, avatar_element_of, connection_label, format_disappearing_timer,
    format_epoch_date, identity_hue, image_render_size, message_preview, message_status, one_line,
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
                    .child("Link Whatfast"),
            )
            .child("WhatsApp → Linked devices → Link a device")
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
                    .child("Waiting for QR…"),
            })
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("The code refreshes automatically."),
            )
            .child(
                Button::new("retry-pairing")
                    .label("Refresh pairing code")
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
        let compact = window.viewport_size().width < px(800. * self.ui_scale);
        let show_sidebar = !compact || selected.is_none();
        let show_conversation = !compact || selected.is_some();
        let states_open = self.states_open;
        let channels_open = self.channels_open;
        let archived_open = self.archived_open;
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
            .when(show_sidebar, |root| {
                if states_open {
                    // The states are their own view: they take the place of
                    // the column of chats.
                    root.child(self.render_states(cx))
                } else if channels_open {
                    // The channels are their own view too.
                    root.child(self.render_channels(cx))
                } else if archived_open {
                    // The archived conversations are their own view.
                    root.child(self.render_archived(cx))
                } else {
                    root.child(self.render_sidebar(cx))
                }
            })
            .when(show_conversation, |root| {
                if self.selected_newsletter.is_some() {
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
                                    .child(filter.label()),
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
                    .child(filter.label())
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

    /// The view of the states, the column that replaces the chats.
    fn render_states(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let scale = self.ui_scale;
        let foreground = cx.theme().foreground;
        let hover = cx.theme().muted;
        v_flex()
            .h_full()
            .w(px(384. * scale))
            .min_w(px(300. * scale))
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
                                    .child("States"),
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
            .w(px(384. * scale))
            .min_w(px(300. * scale))
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
                                    .child("Channels"),
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
                    .child("Discover channels"),
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
                            .child("Entertainment, news, sports, lifestyle, people and much more."),
                    )
                    .child(
                        div()
                            .text_size(px(15. * scale))
                            .text_color(muted)
                            .text_center()
                            .child("Follow the channels you like."),
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
            .w(px(384. * scale))
            .min_w(px(300. * scale))
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
                            .child("Archived"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child("When they receive new messages, these chats stay archived."),
                    ),
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
                        .h(px(72.))
                        .px_4()
                        .flex()
                        .items_center()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("Loading more chats…");
                };
                let selected = self.store.selected_chat_id.as_deref() == Some(chat.id.as_str());
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
                    .flex()
                    .gap_3()
                    .items_center()
                    .rounded_md()
                    .bg(if selected {
                        selected_background
                    } else {
                        gpui::transparent_black()
                    })
                    .hover(move |style| style.bg(hover_background))
                    .on_click(cx.listener({
                        let chat_id = chat.id.clone();
                        move |this, _, window, cx| {
                            this.open_chat(chat_id.clone(), window, cx);
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
                "Contact info".into(),
                Icon::new(IconName::Info),
            ))),
            ChatMenuEntry::Item(Box::new((
                Some(ChatMenuAction::Search),
                "Search".into(),
                Icon::new(IconName::Search),
            ))),
            ChatMenuEntry::Item(Box::new((
                None,
                "Select messages".into(),
                Icon::new(WhatfastIcon::Check),
            ))),
            ChatMenuEntry::Item(Box::new((
                Some(ChatMenuAction::Mute),
                if chat.muted {
                    "Unmute notifications"
                } else {
                    "Mute notifications"
                }
                .into(),
                Icon::new(WhatfastIcon::Bell),
            ))),
            ChatMenuEntry::Item(Box::new((
                None,
                "Temporary messages".into(),
                Icon::new(WhatfastIcon::Clock),
            ))),
            ChatMenuEntry::Item(Box::new((
                None,
                "Restrict chat".into(),
                Icon::new(WhatfastIcon::Shield),
            ))),
            ChatMenuEntry::Item(Box::new((
                None,
                "Chat style".into(),
                Icon::new(WhatfastIcon::Palette),
            ))),
            ChatMenuEntry::Item(Box::new((
                Some(ChatMenuAction::Favorite),
                if is_favorite {
                    "Remove from favorites"
                } else {
                    "Add to favorites"
                }
                .into(),
                Icon::new(IconName::Heart),
            ))),
            ChatMenuEntry::Item(Box::new((
                None,
                "Add to list".into(),
                Icon::new(IconName::File),
            ))),
            ChatMenuEntry::Item(Box::new((
                None,
                "Export chat".into(),
                Icon::new(IconName::ArrowDown),
            ))),
            ChatMenuEntry::Item(Box::new((
                None,
                "Close chat".into(),
                Icon::new(IconName::CircleX),
            ))),
            ChatMenuEntry::Separator,
            ChatMenuEntry::Item(Box::new((
                None,
                "Send call link".into(),
                Icon::new(IconName::ExternalLink),
            ))),
            ChatMenuEntry::Item(Box::new((
                None,
                "Schedule call".into(),
                Icon::new(IconName::Calendar),
            ))),
            ChatMenuEntry::Item(Box::new((
                None,
                "New group call".into(),
                Icon::new(IconName::User),
            ))),
            ChatMenuEntry::Separator,
            ChatMenuEntry::Item(Box::new((
                None,
                "Report".into(),
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
                "Clear chat".into(),
                Icon::new(IconName::CircleX),
            ))),
            ChatMenuEntry::Item(Box::new((
                None,
                "Delete chat".into(),
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
            let enabled = action.is_some();
            let color = if !enabled {
                muted
            } else if is_block {
                danger
            } else {
                foreground
            };
            let action = action.unwrap();
            panel = panel.child(
                div()
                    .id(("chat-menu-item", index))
                    .w_full()
                    .px_3()
                    .py(px(8.))
                    .flex()
                    .gap_3()
                    .items_center()
                    .text_sm()
                    .text_color(color)
                    .when(enabled, |row| {
                        row.cursor_pointer().hover(move |style| style.bg(hover))
                    })
                    .when(enabled, |row| {
                        row.on_click(cx.listener({
                            let chat_id = chat_id.clone();
                            move |this, _, window, cx| {
                                this.run_chat_menu_action(action, chat_id.clone(), window, cx);
                            }
                        }))
                    })
                    .child(icon.with_size(px(16. * ui_scale)).text_color(color))
                    .child(div().flex_1().truncate().text_color(color).child(label)),
            );
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
                    "Unpin chat"
                } else {
                    "Pin chat"
                }
                .into(),
                Icon::new(WhatfastIcon::Pin),
            ),
            (
                ChatContextAction::Mute,
                if chat.muted {
                    "Unmute notifications"
                } else {
                    "Mute notifications"
                }
                .into(),
                Icon::new(WhatfastIcon::Bell),
            ),
            (
                ChatContextAction::Archive,
                if chat.archived {
                    "Unarchive chat"
                } else {
                    "Archive chat"
                }
                .into(),
                Icon::new(WhatfastIcon::Archive),
            ),
            (
                ChatContextAction::Favorite,
                if is_favorite {
                    "Remove from favorites"
                } else {
                    "Add to favorites"
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
                    "Mark as read"
                } else {
                    "Mark as unread"
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
            .w(px(384. * ui_scale))
            .min_w(px(300. * ui_scale))
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
                                    .child("Chats"),
                            )
                            .child(
                                h_flex()
                                    .gap_1()
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
                                .border_1()
                                .border_color(cx.theme().border)
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
            // muestra arriba del todo).
            .child(
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
                                    div().h_full().w_full().rounded_full().bg(cx.theme().accent),
                                )
                            }),
                    ),
            )
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
                    .child("Searching local data…"),
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
                        "No contacts or groups found · type 3 characters to search messages"
                    } else {
                        "No local results"
                    }),
            );
        }

        let rows = Rc::new(self.search_results.rows());
        let row_sizes = Rc::new(
            rows.iter()
                .map(|row| {
                    let height = match row {
                        SearchResultRow::Header { .. } => 30.,
                        SearchResultRow::Contact { .. } | SearchResultRow::Group { .. } => 58.,
                        SearchResultRow::Message { .. } => 68.,
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
                    .child("Type 3 characters to include messages"),
            );
        }
        content
    }

    fn render_search_result_row(
        &self,
        row: SearchResultRow,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let selected_background = cx.theme().secondary;
        let hover_background = cx.theme().muted;
        match row {
            SearchResultRow::Header { title, count } => Some(
                h_flex()
                    .h(px(30. * self.ui_scale))
                    .px_3()
                    .items_center()
                    .justify_between()
                    .bg(cx.theme().secondary)
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title)
                    .child(count.to_string())
                    .into_any_element(),
            ),
            SearchResultRow::Contact {
                result_index,
                source_index,
            } => {
                let contact = self.search_results.contacts.get(source_index)?.clone();
                let subtitle = match (
                    contact.secondary_name.is_empty(),
                    contact.phone_number.is_empty(),
                ) {
                    (false, false) => {
                        format!("{} · {}", contact.secondary_name, contact.phone_number)
                    }
                    (false, true) => contact.secondary_name.clone(),
                    (true, false) => contact.phone_number.clone(),
                    (true, true) => "WhatsApp contact".into(),
                };
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
                        .h(px(58. * self.ui_scale))
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
                            px(38.),
                            cx,
                        ))
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
                                        .child(one_line(&contact.display_name)),
                                )
                                .child(
                                    div()
                                        .w_full()
                                        .min_w_0()
                                        .truncate()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(one_line(&subtitle)),
                                ),
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
                    "No messages yet".to_string()
                } else {
                    chat.last_message_preview.clone()
                };
                let cached_avatar = self.store.chat(&chat.id).map(|c| c.avatar_path.as_str());
                let avatar_path = cached_avatar.unwrap_or(chat.avatar_path.as_str());
                Some(
                    h_flex()
                        .id(("search-group", result_index))
                        .h(px(58. * self.ui_scale))
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
                        .child(avatar_element(&chat.title, avatar_path, px(38.), cx))
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
                        .child(
                            div()
                                .flex_none()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(if chat.archived { "Archived" } else { "Group" }),
                        )
                        .into_any_element(),
                )
            }
            SearchResultRow::Message {
                result_index,
                source_index,
            } => {
                let message = self.search_results.messages.get(source_index)?.clone();
                let meta = if message.archived {
                    format!("{} · Archived", message.sender_name)
                } else {
                    message.sender_name.clone()
                };
                Some(
                    v_flex()
                        .id(("search-message", result_index))
                        .h(px(68. * self.ui_scale))
                        .overflow_hidden()
                        .px_3()
                        .py_2()
                        .cursor_pointer()
                        .when(result_index == self.search_highlighted, |row| {
                            row.bg(selected_background)
                        })
                        .hover(move |style| style.bg(hover_background))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.search_highlighted = result_index;
                            this.activate_search_result(window, cx);
                        }))
                        .child(
                            h_flex()
                                .justify_between()
                                .gap_2()
                                .child(
                                    div()
                                        .min_w_0()
                                        .flex_1()
                                        .truncate()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(one_line(&message.chat_title)),
                                )
                                .child(
                                    div()
                                        .flex_none()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(one_line(&meta)),
                                ),
                        )
                        .child(
                            div()
                                .w_full()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(one_line(&message.snippet)),
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
                .render_center("Whatfast", "Choose a conversation to start messaging")
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
        let has_older = self.store.has_older_messages;
        let has_newer = self.store.has_newer_messages || self.store.newer_activity;
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
        // port: the `conversationSubtitle` of the origin
        // (`tauri/src/components/Conversation.tsx`): the header says that the
        // tap opens the info of the contact or of the group. The reference
        // draws the same hint.
        let contact_subtitle = if chat.kind() == proto::ChatKind::Group {
            "Tap for group info".to_string()
        } else {
            "Tap for contact info".to_string()
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
                        "Original message".to_string(),
                        "Message unavailable".to_string(),
                    )
                },
                |message| {
                    let sender = if message.from_me {
                        "You".to_string()
                    } else if message.sender_name.trim().is_empty() {
                        "Unknown contact".to_string()
                    } else {
                        message.sender_name.trim().to_string()
                    };
                    (sender, message_preview(message))
                },
            )
        });
        let header_avatar =
            avatar_element(&chat.title, &chat.avatar_path, px(41. * self.ui_scale), cx);
        // El nombre de la conversacion se mueve al pintarlo: estos dos datos
        // se sacan antes.
        let is_group = chat.kind() == proto::ChatKind::Group;
        let header_chat_id = chat.id.clone();
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
                    // Los botones de la derecha de la cabecera, como en la
                    // referencia: la videollamada, la llamada, el buscador y
                    // el menu de la conversacion.
                    .child(
                        h_flex()
                            .flex_none()
                            .gap_1()
                            .child(
                                Button::new("video-call")
                                    .small()
                                    .icon(WhatfastIcon::Video)
                                    .tooltip("Video call")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.store.toast_error =
                                            Some("The backend has no calls yet.".to_string());
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("voice-call")
                                    .small()
                                    .icon(WhatfastIcon::Phone)
                                    .tooltip("Call")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.store.toast_error =
                                            Some("The backend has no calls yet.".to_string());
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("search-in-chat")
                                    .small()
                                    .icon(IconName::Search)
                                    .tooltip("Search in chat")
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
                                        .tooltip("Show members")
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
                                    .tooltip("Conversation menu")
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
                    .child(self.conversation_wallpaper(wallpaper_width, wallpaper_height, cx))
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
                                    .child("Load older messages")
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
                                |this, range: Range<usize>, window, cx| {
                                    let visible_images = range
                                        .clone()
                                        .filter_map(|index| this.store.messages.get(index))
                                        .filter_map(|message| match message.content.as_ref() {
                                            Some(proto::message::Content::Image(image))
                                                if image.thumbnail_path.is_empty()
                                                    && image.downloadable =>
                                            {
                                                Some((message.chat_id.clone(), message.id.clone()))
                                            }
                                            _ => None,
                                        })
                                        .collect::<Vec<_>>();
                                    for (chat_id, message_id) in visible_images {
                                        this.load_message_image(chat_id, message_id);
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
                                    if range.end == this.store.messages.len()
                                        && (this.store.has_newer_messages
                                            || this.store.newer_activity)
                                    {
                                        this.load_newer_messages();
                                    } else if window.is_window_active()
                                        && range.end == this.store.messages.len()
                                    {
                                        this.mark_read();
                                    }
                                    range.map(|index| this.render_message(index, cx)).collect()
                                },
                            )
                            .track_scroll(&self.message_scroll)
                            .p_4()
                            .gap_2(),
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
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.refresh_latest();
                                        cx.notify();
                                    })),
                            ),
                        )
                    }),
            )
            .when(!connected, |column| {
                column.child(
                    div()
                        .px_4()
                        .py_2()
                        .bg(if dark { rgb(0x594918) } else { rgb(0xffe9b5) })
                        .child("Offline — messages will be available after reconnection"),
                )
            })
            .when(self.emoji_target.is_some(), |column| {
                column.child(self.render_emoji_picker(cx))
            })
            .when(self.sticker_tray_open, |column| {
                // The tray of the stickers opens over the composer, the same
                // way the panel of the emojis does.
                column.child(self.sticker_tray.render(self, cx))
            })
            .when(self.mention_popup_visible(), |column| {
                column.child(self.render_mention_picker(cx))
            })
            // The line that says who is writing and the bar of the answer
            // float above the composer.
            .when_some(composer_typing_line, |column, typing| {
                column.child(
                    div()
                        .absolute()
                        .bottom(px(56. * self.ui_scale))
                        .left_0()
                        .right_0()
                        .h(px(24. * self.ui_scale))
                        .px_4()
                        .pt_1()
                        .text_xs()
                        .text_color(if dark {
                            gpui::Hsla::from(rgb(0x25d366))
                        } else {
                            gpui::Hsla::from(rgb(DARK_GREEN))
                        })
                        .child(typing),
                )
            })
            .when_some(reply_context, |column, (sender, preview)| {
                column.child(
                    div()
                        .absolute()
                        .bottom(px(56. * self.ui_scale))
                        .left_0()
                        .right_0()
                        .child(
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
                                                .child(format!("Replying to {sender}")),
                                        )
                                        .child(
                                            div()
                                                .truncate()
                                                .text_sm()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(preview),
                                        ),
                                )
                                .child(Button::new("cancel-reply").small().label("×").on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.cancel_reply();
                                        cx.notify();
                                    }),
                                )),
                        ),
                )
            })
            .child(
                v_flex()
                    // The composer has a fixed height and never shrinks: the
                    // list of the messages is the one that gives room.
                    .flex_shrink_0()
                    .h(px(56. * self.ui_scale))
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        // The pill of the message: the clip, the emoji, the
                        // text and the button of sending, the same way the
                        // reference draws it.
                        h_flex().p_2().gap_2().child(
                            h_flex()
                                .flex_1()
                                .h(px(42. * self.ui_scale))
                                .rounded_full()
                                .bg(cx.theme().input)
                                .px_2()
                                .gap_1()
                                .items_center()
                                .child(self.attachment_bar.clone())
                                .child(
                                    Button::new("emoji-picker")
                                        .ghost()
                                        .icon(WhatfastIcon::Smile)
                                        .tooltip("Emoji")
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
                                .child(
                                    Button::new("send")
                                        .primary()
                                        .label("Send")
                                        .disabled(!connected)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.send_text(window, cx)
                                        })),
                                ),
                        ),
                    ),
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
                        .child("Loading group members…"),
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
        const COMPOSER_HEIGHT: f32 = 56.;
        const REPLY_BAR_HEIGHT: f32 = 60.;
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
            "Smileys and people"
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
        // The bottom of the box: above the composer, with the gap of the
        // reference. When there is a reply in the air, the bar of the reply
        // is under the box and the box goes a little higher.
        let bottom = COMPOSER_HEIGHT
            + EMOJI_PANEL_GAP
            + if self.replying_to_message_id.is_some() {
                REPLY_BAR_HEIGHT
            } else {
                0.
            };
        v_flex()
            // The box floats over the list of the messages, above the button
            // of the composer that opens it.
            .absolute()
            .left(px(EMOJI_PANEL_MARGIN * scale))
            .bottom(px(bottom * scale))
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
                                .label("Remove mine")
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
                                            .child("Recent"),
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
                                            .child("No emoji found");
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
                        .child("Choose one emoji to react"),
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
                            .child("GIF"),
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
    /// draws the settings as the column of the left: the name of the user at
    /// the top, the search with the border of the accent below it, the photo
    /// of the profile in the middle and the list of the sections, each one
    /// with a drawing, the title in bold and the description in the muted
    /// color.
    fn render_settings(&self, cx: &mut Context<Self>) -> gpui::Div {
        let scale = self.ui_scale;
        let scale_percent = (self.ui_scale * 100.0).round() as u32;
        let logout_pending = self.rpc.logout_pending();
        let dark = cx.theme().is_dark();
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        // The app does not know the name of the user: the panel carries the
        // name of the app, the same way the panel of the info of a contact
        // carries the name of the conversation.
        let name = "Whatfast";

        // The scale of the interface: the same buttons as before, under the
        // title of their section.
        let scale_control = v_flex()
            .gap_2()
            .mt_1()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("decrease-ui-scale")
                            .label("−")
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
                            .label("Reset")
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
                    .child("Saved automatically for future launches."),
            );

        // The theme: the same button as before, under the title of its
        // section. The row keeps the button at its own width instead of
        // stretching it to the whole column.
        let theme_control = h_flex().mt_1().child(
            Button::new("theme")
                .small()
                .icon(if dark { IconName::Sun } else { IconName::Moon })
                .label(if dark {
                    "Use the light theme"
                } else {
                    "Use the dark theme"
                })
                .on_click(cx.listener(|this, _, window, cx| {
                    this.toggle_theme(window, cx);
                })),
        );

        // The account: the same button as before, under the title of its
        // section.
        let account_control = h_flex().mt_1().child(
            Button::new("logout")
                .small()
                .label(if logout_pending {
                    "Clearing…"
                } else if self.confirming_logout {
                    "Confirm"
                } else {
                    "Log out"
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
                    // The name of the user, in big and bold.
                    .child(
                        h_flex()
                            .px_5()
                            .pt_5()
                            .pb_4()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(22. * scale))
                                    .font_weight(FontWeight::BOLD)
                                    .child(name),
                            )
                            .child(
                                Button::new("close-settings")
                                    .label("×")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.settings_open = false;
                                        cx.notify();
                                    })),
                            ),
                    )
                    // The search of the reference: the border of the accent
                    // and the magnifier inside.
                    .child(
                        div().px_5().child(
                            h_flex()
                                .h(px(40. * scale))
                                .px_3()
                                .gap_2()
                                .items_center()
                                .rounded_full()
                                .bg(cx.theme().input)
                                .border_1()
                                .border_color(cx.theme().accent)
                                .child(
                                    Icon::new(IconName::Search)
                                        .with_size(px(17. * scale))
                                        .text_color(muted),
                                )
                                .child(div().text_sm().text_color(muted).child("Search")),
                        ),
                    )
                    // The photo of the profile, in the middle. The app has no
                    // photo of the user: the circle carries the glyph of the
                    // person.
                    .child(
                        div().flex().justify_center().pt_6().child(
                            div()
                                .size(px(100. * scale))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(cx.theme().secondary)
                                .child(
                                    Icon::new(IconName::User)
                                        .with_size(px(46. * scale))
                                        .text_color(muted),
                                ),
                        ),
                    )
                    // The list of the sections.
                    .child(
                        v_flex()
                            .id("settings-sections")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .px_5()
                            .pt_6()
                            .pb_6()
                            .gap_6()
                            .child(section(
                                WhatfastIcon::Settings,
                                "Interface scale",
                                "Increase text, controls, and interface spacing.",
                                scale_control,
                            ))
                            .child(section(
                                WhatfastIcon::Palette,
                                "Appearance",
                                "The dark and the light palette of WhatsApp.",
                                theme_control,
                            ))
                            .child(section(
                                WhatfastIcon::Warning,
                                "Account",
                                "Sign out of this device and clear the local data.",
                                account_control,
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
            .unwrap_or_else(|| "Conversation".into());
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
            format!("Group · {} members", info.participant_count)
        } else if is_group {
            "Group".to_string()
        } else {
            chat.as_ref()
                .map(|chat| chat.phone_number.clone())
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
                    .child("Loading chat details…"),
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
                            .label("Retry")
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
                    body = body.child(section("About", info.about.clone()));
                }
                if let Some(chat) = chat.as_ref() {
                    if !chat.phone_number.is_empty() {
                        body = body.child(section("Phone", chat.phone_number.clone()));
                    }
                    if !info.verified_name.is_empty() {
                        body = body.child(section("Verified business", info.verified_name.clone()));
                    }
                    if !chat.business_name.is_empty() && chat.business_name != chat.title {
                        body = body.child(section("Business name", chat.business_name.clone()));
                    }
                    if !chat.push_name.is_empty() && chat.push_name != chat.title {
                        body = body.child(section("Profile name", format!("~{}", chat.push_name)));
                    }
                }
            } else {
                let description = if info.description.is_empty() {
                    "No description".to_string()
                } else {
                    info.description.clone()
                };
                body = body.child(section("Description", description));
                if info.created_at_ms > 0 {
                    let mut created = format!("Created {}", format_epoch_date(info.created_at_ms));
                    if !info.created_by.is_empty() {
                        created.push_str(&format!(" by {}", info.created_by));
                    }
                    body = body.child(div().text_xs().text_color(muted).child(created));
                }
                if info.disappearing_timer_seconds > 0 {
                    body = body.child(section(
                        "Disappearing messages",
                        format_disappearing_timer(info.disappearing_timer_seconds),
                    ));
                }
                let mut notes: Vec<&'static str> = Vec::new();
                if info.is_community {
                    notes.push("Community group");
                }
                if info.announce_only {
                    notes.push("Only admins can send messages");
                }
                if info.locked {
                    notes.push("Only admins can edit group info");
                }
                if info.join_approval_required {
                    notes.push("Admins approve new members");
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
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("{} members", info.participant_count)),
                    );
                    for (index, participant) in
                        info.participants.iter().take(MAX_MEMBER_ROWS).enumerate()
                    {
                        let display_name = if participant.is_me {
                            "You".to_string()
                        } else if participant.display_name.is_empty() {
                            participant.phone_number.clone()
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
                            Some("Owner")
                        } else if participant.is_admin {
                            Some("Admin")
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
                                                div()
                                                    .text_xs()
                                                    .text_color(muted)
                                                    .truncate()
                                                    .child(participant.phone_number.clone()),
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
                            "Remove from favorites"
                        } else {
                            "Add to favorites"
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_favorite(chat_id.clone(), cx);
                            cx.notify();
                        })),
                );
            }
            if !info.address.is_empty() {
                body = body.child(section("WhatsApp address", info.address.clone()));
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
                                "Group info"
                            } else {
                                "Contact info"
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
                                    .child("Recent chats"),
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
                                chat.phone_number.clone()
                            }
                        } else {
                            chat.last_message_preview.clone()
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
                                        .child("Archived"),
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
                            .child("Release Ctrl to switch · Esc to cancel"),
                    ),
            )
    }

    fn render_image_viewer(&self, cx: &mut Context<Self>) -> gpui::Div {
        let Some(viewer) = self.image_viewer.as_ref() else {
            return div();
        };
        let chat_id = viewer.chat_id.clone();
        let message_id = viewer.message_id.clone();
        let path = viewer.path.clone();
        let caption = viewer.caption.trim().to_string();
        let media_name = if viewer.sticker { "Sticker" } else { "Photo" };
        let opening_label = if viewer.sticker {
            "Opening sticker…"
        } else {
            "Opening photo…"
        };
        let fallback_label = if viewer.sticker {
            "This sticker couldn't be decoded. Try Reload."
        } else {
            "This photo couldn't be decoded. Try Reload."
        };
        div()
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .flex_col()
            .p_4()
            .bg(rgba(0x050807f2))
            .text_color(rgb(0xffffff))
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(media_name))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Button::new("reload-viewed-image").label("Reload").on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.retry_message_image(chat_id.clone(), message_id.clone());
                                    cx.notify();
                                }),
                            ))
                            .child(Button::new("close-image-viewer").label("×").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.image_viewer = None;
                                    cx.notify();
                                }),
                            )),
                    ),
            )
            .child(
                div().flex_1().min_h_0().min_w_0().p_4().child(
                    img(PathBuf::from(path))
                        .size_full()
                        .object_fit(ObjectFit::Contain)
                        .with_loading(move || {
                            div()
                                .size_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(opening_label)
                                .into_any_element()
                        })
                        .with_fallback(move || {
                            div()
                                .size_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(fallback_label)
                                .into_any_element()
                        }),
                ),
            )
            .when(!caption.is_empty(), |viewer| {
                viewer.child(
                    div()
                        .w_full()
                        .max_h(px(96.))
                        .overflow_hidden()
                        .px_4()
                        .py_2()
                        .text_sm()
                        .child(caption),
                )
            })
    }

    fn render_linkified_text(
        &self,
        message_id: &str,
        text: &str,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let mut lines = Vec::new();
        for (line_index, line) in text.split('\n').enumerate() {
            let mut row = h_flex().flex_wrap();
            if line.is_empty() {
                row = row.child(div().child(" "));
            }
            for (token_index, token) in line.split_whitespace().enumerate() {
                if let Some((prefix, url, suffix)) = split_url_token(token) {
                    if !prefix.is_empty() {
                        row = row.child(div().child(prefix.to_string()));
                    }
                    let target = url.to_string();
                    row = row.child(
                        div()
                            .id(format!(
                                "message-link-{message_id}-{line_index}-{token_index}"
                            ))
                            .cursor_pointer()
                            .text_color(rgb(0x0b57d0))
                            .underline()
                            .child(url.to_string())
                            .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&target))),
                    );
                    row = row.child(div().child(format!("{suffix} ")));
                } else {
                    row = row.child(div().child(format!("{token} ")));
                }
            }
            lines.push(row.into_any_element());
        }
        div().child(v_flex().children(lines))
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
        v_flex()
            .id(("link-preview", index))
            .w(px(320.))
            .max_w_full()
            .overflow_hidden()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().secondary)
            .when_some(thumbnail, |card, thumbnail| {
                card.child(
                    img(thumbnail)
                        .w_full()
                        .h(px(144.))
                        .object_fit(ObjectFit::Cover),
                )
            })
            .child(
                v_flex()
                    .p_2()
                    .gap_1()
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .when(!preview.description.trim().is_empty(), |details| {
                        details.child(
                            div()
                                .truncate()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(preview.description),
                        )
                    })
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(host),
                    ),
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
        let (bubble_in, bubble_out) = tokens::bubble_colors(if cx.theme().is_dark() {
            whatfast_core::theme::ThemeAppearance::Dark
        } else {
            whatfast_core::theme::ThemeAppearance::Light
        });
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
        let sticker = image_content.as_ref().is_some_and(|image| image.sticker);
        let text = if sticker {
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
        let body_message_id = message.id.clone();
        let search_target = self.search_target_message_id.as_deref() == Some(message.id.as_str());
        let reply_action_message_id = message_id.clone();
        let more_action_message_id = message_id.clone();
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
        let meta_text = {
            let mut segments: Vec<&str> = Vec::new();
            if message.edited {
                segments.push("edited");
            }
            if message.from_me {
                let status = message_status(message.status());
                if !status.is_empty() {
                    segments.push(status);
                }
            }
            segments.join(" · ")
        };
        // port: the reference writes the hour of the message at the end of
        // the text, inside the bubble, and it is the first thing that
        // changes when the day of the message is not the day of the one
        // before it.
        let time_label = bubble_time_label(message.timestamp_ms);
        // The tail of the bubble has the colour of the bubble: a sticker has
        // no bubble, so it has no tail either.
        let bubble_color = if sticker {
            gpui::transparent_black()
        } else if message.from_me {
            bubble_out
        } else {
            bubble_in
        };
        v_flex()
            .w_full()
            .gap_2()
            // The pill of the day: the reference draws it centered above the
            // first message of every day of the conversation.
            .when_some(self.day_separator_label(index), |column, label| {
                column.child(
                    div().flex().justify_center().child(
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
            .child(
                h_flex()
                    .w_full()
                    .items_start()
                    .gap_2()
                    .when(message.from_me, |row| row.justify_end())
                    .when(group_incoming, |row| {
                        let avatar_path = if message.sender_avatar_path.is_empty() {
                            self.store
                                .participant_avatar_path(&message.sender_id)
                                .unwrap_or_default()
                        } else {
                            message.sender_avatar_path.as_str()
                        };
                        row.child(avatar_element(
                            &message.sender_name,
                            avatar_path,
                            px(34.),
                            cx,
                        ))
                    })
                    .child(
                        v_flex()
                            .max_w(px(560.))
                            .gap_1()
                            // port: the reference paints the actions of a
                            // message outside of the bubble, below it.
                            .child(
                                v_flex()
                                    .max_w(px(560.))
                                    .when(!sticker, |bubble| bubble.px_3().py_2())
                                    .rounded_lg()
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
                                    // The point of the corner: at the left of the
                                    // received bubbles and at the right of the
                                    // sent ones. The tail sticks out of the
                                    // corner, the way the reference draws it.
                                    .when(!sticker, |bubble| {
                                        bubble.child(
                                            div()
                                                .absolute()
                                                .top(px(BUBBLE_TAIL_OFFSET * self.ui_scale))
                                                .when(message.from_me, |tail| {
                                                    tail.right(px(
                                                        BUBBLE_TAIL_OFFSET * self.ui_scale
                                                    ))
                                                })
                                                .when(!message.from_me, |tail| {
                                                    tail.left(
                                                        px(BUBBLE_TAIL_OFFSET * self.ui_scale),
                                                    )
                                                })
                                                .child(bubble_tail(
                                                    bubble_color,
                                                    message.from_me,
                                                    self.ui_scale,
                                                )),
                                        )
                                    })
                                    .when(group_incoming, |bubble| {
                                        let name_color =
                                            cx.theme().blue.hue(identity_hue(&message.sender_name));
                                        let show_number = !message.sender_phone_number.is_empty()
                                            && message.sender_phone_number != message.sender_name;
                                        bubble.child(
                                            h_flex()
                                                .items_baseline()
                                                .gap_2()
                                                .child(
                                                    div()
                                                        .text_sm()
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                        .text_color(name_color)
                                                        .child(message.sender_name.clone()),
                                                )
                                                .when(show_number, |header| {
                                                    header.child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(cx.theme().muted_foreground)
                                                            .child(
                                                                message.sender_phone_number.clone(),
                                                            ),
                                                    )
                                                }),
                                        )
                                    })
                                    .when(!reply_target_id.is_empty(), |bubble| {
                                        let target_id = reply_target_id.clone();
                                        let (sender, preview) = reply_target.as_ref().map_or_else(
                                            || {
                                                (
                                                    "Original message".to_string(),
                                                    "Message unavailable".to_string(),
                                                )
                                            },
                                            |target| {
                                                let sender = if target.from_me {
                                                    "You".to_string()
                                                } else if !target.sender_name.trim().is_empty() {
                                                    target.sender_name.trim().to_string()
                                                } else {
                                                    "Unknown contact".to_string()
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
                                    .when_some(image_content, |bubble, image| {
                                        let media_name =
                                            if image.sticker { "sticker" } else { "photo" };
                                        let (image_width, image_height) = image_render_size(&image);
                                        let image_path = image_row_path(&image).to_string();
                                        let viewer_path = image_viewer_path(&image).to_string();
                                        let image_is_cached = !image_path.is_empty()
                                            && !viewer_path.is_empty()
                                            && PathBuf::from(&image_path).is_file()
                                            && PathBuf::from(&viewer_path).is_file();
                                        let cached_path_missing = (!image_path.is_empty()
                                            || !viewer_path.is_empty())
                                            && !image_is_cached;
                                        let retryable = image.downloadable
                                            && (image_failure.is_some() || cached_path_missing);
                                        let placeholder = if cached_path_missing {
                                            format!("Cached {media_name} missing · Click to reload")
                                        } else if let Some(failure) = image_failure.clone() {
                                            failure
                                        } else if image.downloadable {
                                            format!("Loading {media_name}…")
                                        } else if image.sticker {
                                            "Sticker unavailable".to_string()
                                        } else {
                                            "Photo unavailable".to_string()
                                        };
                                        let viewer_chat_id = message_image_chat_id.clone();
                                        let viewer_message_id = message_image_id.clone();
                                        let viewer_caption = image.caption.clone();
                                        let viewer_sticker = image.sticker;
                                        let retry_chat_id = message_image_chat_id.clone();
                                        let retry_message_id = message_image_id.clone();
                                        let loading_color = cx.theme().muted_foreground;
                                        let fallback_color = cx.theme().muted_foreground;
                                        let decoding_label = format!("Decoding {media_name}…");
                                        let decode_failure_label = format!(
                                            "Couldn't display {media_name} · Open to retry"
                                        );
                                        bubble.child(
                                            div()
                                                .id(("message-image", index))
                                                .w(image_width)
                                                .h(image_height)
                                                .max_w_full()
                                                .overflow_hidden()
                                                .rounded_md()
                                                .when(!image.sticker, |container| {
                                                    container
                                                        .border_1()
                                                        .border_color(cx.theme().border)
                                                        .bg(cx.theme().secondary)
                                                })
                                                .when(image_is_cached, |container| {
                                                    container.child(
                                                        img(PathBuf::from(image_path.clone()))
                                                            .size_full()
                                                            .object_fit(ObjectFit::Contain)
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
                                                                    .child(
                                                                        decode_failure_label
                                                                            .clone(),
                                                                    )
                                                                    .into_any_element()
                                                            }),
                                                    )
                                                })
                                                .when(image_is_cached, |container| {
                                                    container.cursor_pointer().on_click(
                                                        cx.listener(move |this, _, _, cx| {
                                                            this.open_image_viewer(
                                                                viewer_chat_id.clone(),
                                                                viewer_message_id.clone(),
                                                                viewer_path.clone(),
                                                                viewer_caption.clone(),
                                                                viewer_sticker,
                                                            );
                                                            cx.notify();
                                                        }),
                                                    )
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
                                                    container.cursor_pointer().on_click(
                                                        cx.listener(move |this, _, _, cx| {
                                                            this.retry_message_image(
                                                                retry_chat_id.clone(),
                                                                retry_message_id.clone(),
                                                            );
                                                            cx.notify();
                                                        }),
                                                    )
                                                }),
                                        )
                                    })
                                    .when_some(
                                        text_content.and_then(|text| text.link_preview),
                                        |bubble, preview| {
                                            bubble
                                                .child(self.render_link_preview(index, preview, cx))
                                        },
                                    )
                                    // La tarjeta del adjunto y las de la
                                    // ubicacion y de los contactos van antes
                                    // del texto, como en el origen.
                                    .when_some(attachment_content.clone(), |bubble, attachment| {
                                        bubble.child(self.render_attachment_card(
                                            index,
                                            message,
                                            &attachment,
                                            cx,
                                        ))
                                    })
                                    .when_some(
                                        card_content.as_ref().and_then(|content| {
                                            ubicaciones::render_content_card(
                                                content,
                                                index,
                                                self.ui_scale,
                                                cx,
                                            )
                                        }),
                                        |bubble, card| bubble.child(card),
                                    )
                                    .when(!text.is_empty(), |bubble| {
                                        // The hour goes at the end of the text,
                                        // inside the bubble, the way the
                                        // reference writes it.
                                        bubble.child(
                                            h_flex()
                                                .items_end()
                                                .gap_1()
                                                .child(self.render_linkified_text(
                                                    &body_message_id,
                                                    &text,
                                                    cx,
                                                ))
                                                .child(
                                                    div()
                                                        .flex_none()
                                                        .text_xs()
                                                        .text_color(cx.theme().muted_foreground)
                                                        .child(time_label.clone()),
                                                ),
                                        )
                                    })
                                    .when(!reaction_chips.is_empty(), |bubble| {
                                        bubble.child(h_flex().gap_1().children(
                                            reaction_chips.into_iter().enumerate().map(
                                                |(chip_index, reaction)| {
                                                    let emoji = reaction.emoji.to_string();
                                                    let clicked_emoji = emoji.clone();
                                                    let clicked_chat_id = reaction_chat_id.clone();
                                                    let clicked_message_id =
                                                        reaction_message_id.clone();
                                                    div()
                                                        .id(format!(
                                                            "reaction-chip-{index}-{chip_index}"
                                                        ))
                                                        .px_2()
                                                        .py_1()
                                                        .rounded_lg()
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
                                                        .hover(move |style| {
                                                            style.bg(reaction_hover)
                                                        })
                                                        .cursor_pointer()
                                                        .text_xs()
                                                        .child(format!(
                                                            "{emoji} {}",
                                                            reaction.count
                                                        ))
                                                        .on_click(cx.listener(
                                                            move |this, _, _, cx| {
                                                                this.open_reaction_details(
                                                                    clicked_chat_id.clone(),
                                                                    clicked_message_id.clone(),
                                                                    clicked_emoji.clone(),
                                                                );
                                                                cx.notify();
                                                            },
                                                        ))
                                                },
                                            ),
                                        ))
                                    }),
                            )
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_1()
                                    .when(!meta_text.is_empty(), |footer| {
                                        footer.child(
                                            div()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(meta_text.clone()),
                                        )
                                    })
                                    .when(reply_count > 0, |footer| {
                                        let reply_id = first_reply_id.clone().unwrap_or_default();
                                        footer.child(
                                            Button::new(("view-replies", index))
                                                .ghost()
                                                .small()
                                                .label(if reply_count == 1 {
                                                    "1 reply".to_string()
                                                } else {
                                                    format!("{reply_count} replies")
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
                                            .tooltip("Reply")
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
                                            .tooltip("React")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.open_reaction_picker(
                                                    chat_id.clone(),
                                                    message_id.clone(),
                                                );
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
                                            .tooltip("More")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.open_message_menu(
                                                    more_action_message_id.clone(),
                                                    cx,
                                                );
                                                cx.notify();
                                            })),
                                    ),
                            ),
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
                    (
                        message.id.clone(),
                        attachments::bubble_text(message).to_string(),
                    )
                };
                // The hour goes at the end of the text, inside the bubble:
                // the text wraps before it, so the width it takes is not
                // there for the text.
                let time_width = if text.is_empty() {
                    px(0.)
                } else {
                    let label = bubble_time_label(self.store.messages[index].timestamp_ms);
                    bubble_time_width(window, &label, font_size)
                };
                let text_wrap_width = (wrap_width - time_width).max(px(60.));
                let text_height = if text.is_empty() {
                    px(0.)
                } else {
                    let text: SharedString = text.into();
                    let run = window.text_style().to_run(text.len());
                    window
                        .text_system()
                        .shape_text(text, font_size, &[run], Some(text_wrap_width), None)
                        .map(|lines| {
                            lines.iter().fold(px(0.), |height, line| {
                                height + line.size(line_height).height
                            })
                        })
                        .unwrap_or(line_height)
                };
                let image_height = match self.store.messages[index].content.as_ref() {
                    Some(proto::message::Content::Image(image)) => {
                        image_render_size(image).1
                            + if text_height > px(0.) { px(4.) } else { px(0.) }
                    }
                    _ => px(0.),
                };
                let link_preview_height = match self.store.messages[index].content.as_ref() {
                    Some(proto::message::Content::Text(text)) if text.link_preview.is_some() => {
                        let thumbnail = text
                            .link_preview
                            .as_ref()
                            .is_some_and(|preview| !preview.jpeg_thumbnail.is_empty());
                        px(if thumbnail { 210. } else { 70. } * self.ui_scale)
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
                let height = text_height
                    + image_height
                    + link_preview_height
                    + attachment_height
                    + card_height
                    + px(40. * self.ui_scale)
                    + reaction_height
                    + reply_height
                    + sender_header_height;
                // The model stores the heights as plain numbers: GPUI's
                // `Pixels` is a newtype over the same `f32`.
                let height = f32::from(height);
                self.store.cache_message_height(id, width_bucket, height);
                height
            };
            // The pill of the day is not part of the cached height: the day
            // of a message depends on the message before it, and that one
            // changes when older messages arrive.
            let separator_height = if self.starts_a_day(index) {
                px(DAY_SEPARATOR_HEIGHT + DAY_SEPARATOR_GAP)
            } else {
                px(0.)
            };
            sizes.push(geom::size(
                geom::px(1.),
                height + f32::from(separator_height),
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
    ///
    /// port: the origin (`tauri/src/components/Conversation.tsx`) paints the
    /// pill of the day when the day of the message is not the day of the
    /// message before it.
    fn starts_a_day(&self, index: usize) -> bool {
        let Some(message) = self.store.messages.get(index) else {
            return false;
        };
        if message.timestamp_ms == 0 {
            return false;
        }
        let key = message_day_key(message.timestamp_ms);
        match index
            .checked_sub(1)
            .and_then(|previous| self.store.messages.get(previous))
        {
            Some(previous) if previous.timestamp_ms != 0 => {
                message_day_key(previous.timestamp_ms) != key
            }
            _ => true,
        }
    }

    /// The label of the pill that separates two days of the conversation.
    fn day_separator_label(&self, index: usize) -> Option<String> {
        if !self.starts_a_day(index) {
            return None;
        }
        let timestamp = self.store.messages.get(index)?.timestamp_ms;
        Some(message_day_label(timestamp, now_ms()))
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
                    .unwrap_or_else(|| "Unknown backend error".into()),
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
            .on_modifiers_changed(
                cx.listener(|this, event: &ModifiersChangedEvent, window, cx| {
                    if this.chat_switcher.is_some() && !event.modifiers.control {
                        this.commit_chat_switcher(window, cx);
                        cx.notify();
                    }
                }),
            )
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
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
            .when(self.image_viewer.is_some(), |root| {
                root.child(self.render_image_viewer(cx))
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
            .when_some(self.media_editor.clone(), |root, editor| root.child(editor))
            .when_some(self.message_menu.clone(), |root, menu| root.child(menu))
            .when_some(self.forward_picker.clone(), |root, picker| {
                root.child(picker)
            })
            .when_some(self.poll_composer.clone(), |root, composer| {
                root.child(composer)
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
    match filter {
        ChatFilter::All => "No conversations yet",
        ChatFilter::Unread => "No unread conversations",
        ChatFilter::Favorites => "No favorite chats yet",
        ChatFilter::Groups => "No groups",
        ChatFilter::Archived => "No archived conversations",
    }
}

/// The current instant, in milliseconds of the epoch.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(0)
}

/// The time of the last message of a conversation, the way the reference
/// writes it at the right of the title.
///
/// port: `formatChatTime` of the origin (`tauri/src/lib/format.ts`): the
/// hour of the day, "Yesterday", the day of the week, or the short date.
fn chat_time_label(timestamp_ms: i64, now_ms: i64) -> String {
    whatfast_core::message_format::format_chat_time(
        whatfast_core::message_format::LocalTimestamp::new(timestamp_ms, 0),
        whatfast_core::message_format::LocalTimestamp::new(now_ms, 0),
        whatfast_core::message_format::DateLocale::EnUs,
    )
}

/// The line under the title of a conversation.
///
/// port: `chatSubtitle` of the origin (`tauri/src/lib/format.ts`): the
/// preview of the last message, the phone number when there is no preview,
/// and the mark of the messages the user sent.
fn chat_subtitle(chat: &proto::Chat) -> String {
    let preview = if !chat.last_message_preview.is_empty() {
        chat.last_message_preview.clone()
    } else if !chat.phone_number.is_empty() {
        chat.phone_number.clone()
    } else {
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

/// How far the tail of a bubble sticks out of its corner, in unscaled pixels.
const BUBBLE_TAIL_OFFSET: f32 = -6.;

/// The hour of a message, the way the reference writes it inside the bubble.
///
/// The reference writes the hour of the twenty-four hours ("10:45"). The
/// crate has no zone of the machine, so the hour is the one of UTC, the same
/// as the rest of the times of the app.
fn bubble_time_label(timestamp_ms: i64) -> String {
    let millis = timestamp_ms.rem_euclid(86_400_000);
    format!("{:02}:{:02}", millis / 3_600_000, (millis / 60_000) % 60)
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
/// port: `dayKey` of the origin (`tauri/src/lib/format.ts`).
fn message_day_key(timestamp_ms: i64) -> String {
    whatfast_core::message_format::day_key(whatfast_core::message_format::LocalTimestamp::new(
        timestamp_ms,
        0,
    ))
}

/// The day of a message, the way the reference writes it in the pill that
/// separates two days.
///
/// port: `formatDay` of the origin (`tauri/src/lib/format.ts`).
fn message_day_label(timestamp_ms: i64, now_ms: i64) -> String {
    whatfast_core::message_format::format_day(
        whatfast_core::message_format::LocalTimestamp::new(timestamp_ms, 0),
        whatfast_core::message_format::LocalTimestamp::new(now_ms, 0),
        whatfast_core::message_format::DateLocale::EnUs,
    )
}
