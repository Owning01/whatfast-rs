//! The tray of the library of stickers.
//!
//! port: taken from the origin's `tauri/src/components/StickerTray.tsx`. The
//! lengths come from the stylesheet of the origin
//! (`tauri/src/styles.css`, the classes `sticker-tray`, `sticker-tab-strip`,
//! `sticker-pack-tab`, `sticker-grid` and `sticker-cell`).
//!
//! The look is the one of the reference of the desktop client of WhatsApp
//! (`docs/referencia/07-paneles-del-compositor.md`, "El panel de los
//! stickers"): a floating box over the composer, with the row of the packs,
//! the field of the search, the grid of the stickers and the three tabs of
//! the foot.
//!
//! The data is not here: the packs live in the store of the application
//! (`store.sticker_packs()`), and the panel only draws them. The three
//! actions it needs already exist in `app.rs`: `load_stickers()`,
//! `send_sticker_from_library()` and `choose_sticker()`.

#![allow(dead_code)]

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use whatfast_i18n as i18n;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AppContext as _, Context, ElementId, Entity, InteractiveElement as _, IntoElement as _,
    ObjectFit, ParentElement as _, StatefulInteractiveElement as _, Styled as _, StyledImage as _,
    Subscription, div, img, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    v_flex,
};

use whatfast_protocol::{Sticker, StickerPack};

use crate::app::Whatfast;

/// The height of the tray, in pixels, before the scale.
pub(crate) const TRAY_HEIGHT: f32 = 420.0;
/// The side of one cell of the grid of stickers, in pixels, before the scale.
pub(crate) const CELL_SIZE: f32 = 80.0;
/// The side of one tab of the strip of packs, in pixels, before the scale.
pub(crate) const TAB_SIZE: f32 = 34.0;
/// The gap between two cells of the grid, in pixels, before the scale.
pub(crate) const CELL_GAP: f32 = 6.0;
/// The padding inside one cell of the grid, in pixels, before the scale.
const CELL_PADDING: f32 = 2.0;
/// The height of the composer, in pixels, before the scale: the tray floats
/// over it.
const COMPOSER_HEIGHT: f32 = 56.0;
/// The gap between the tray and the composer, in pixels, before the scale.
const COMPOSER_GAP: f32 = 8.0;
/// The text that the field of the search shows while it is empty.
const SEARCH_PLACEHOLDER: &str = "Search in the WhatsApp sticker store";

/// The state of the tray of the stickers.
#[derive(Default)]
pub(crate) struct StickerTrayState {
    /// The pack that is open in the tray.
    pub(crate) active_pack_id: String,
    /// The sticker that is being sent right now.
    pub(crate) sending: String,
    /// Whether the library was already asked for.
    ///
    /// port: the origin asks for the library the first time the tray opens,
    /// and only when nothing was loaded and nothing failed. The flag is the
    /// memory of that first question, so the tray never hammers the backend.
    pub(crate) requested: bool,
    /// The text of the search of the tray.
    pub(crate) query: String,
    /// The field of the search.
    ///
    /// The state of a field of text is born with a window, and the tray is
    /// drawn while the window is busy: the field waits for the end of the
    /// cycle of the effects (see `ensure_search_input`).
    pub(crate) search_input: Option<Entity<InputState>>,
    /// The subscription that hears the changes of the field of the search.
    pub(crate) search_subscription: Option<Subscription>,
    /// Whether the field of the search is being born right now.
    pub(crate) search_pending: bool,
}

/// The pack that is open in the tray.
///
/// port: `activePack` of the origin: the pack the user chose, or the first
/// one when the choice is not there anymore (the packs are reloaded from the
/// backend every now and then).
pub(crate) fn active_pack<'a>(
    packs: &'a [StickerPack],
    active_pack_id: &str,
) -> Option<&'a StickerPack> {
    if packs.is_empty() {
        return None;
    }
    packs
        .iter()
        .find(|pack| pack.id == active_pack_id)
        .or_else(|| packs.first())
}

/// The label of a pack in the strip of tabs.
///
/// port: the origin writes `pack.name.slice(0, 2).toUpperCase()`.
pub(crate) fn pack_label(name: &str) -> String {
    name.chars().take(2).collect::<String>().to_uppercase()
}

/// Whether a sticker answers to the text of the search.
///
/// The stickers of the protocol do not carry a name yet (the `Sticker` of
/// `bridge.proto` has the id, the path and the measures), so the search
/// looks at the id of the sticker.
pub(crate) fn sticker_matches(sticker: &Sticker, query: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return true;
    }
    sticker.id.to_lowercase().contains(&query.to_lowercase())
}

/// The path of the local copy of one sticker.
///
/// port: `stickerLocalPath` of the origin: the sticker of the pack whose id
/// matches, with the path of its local copy. A sticker that was never
/// downloaded has no path, and the tray draws a placeholder for it.
pub(crate) fn sticker_local_path<'a>(pack: &'a StickerPack, sticker_id: &str) -> Option<&'a str> {
    pack.stickers
        .iter()
        .find(|sticker| sticker.id == sticker_id)
        .map(|sticker| sticker.local_path.as_str())
        .filter(|path| !path.is_empty())
}

/// The tray of the library of stickers.
#[derive(Default)]
pub(crate) struct StickerTray {
    /// The state of the tray.
    state: Rc<RefCell<StickerTrayState>>,
}

impl StickerTray {
    /// The pack that is open in the tray.
    pub(crate) fn active_pack_id(&self) -> String {
        self.state.borrow().active_pack_id.clone()
    }

    /// The sticker that is being sent right now.
    pub(crate) fn sending(&self) -> String {
        self.state.borrow().sending.clone()
    }

    /// Draws the tray of the stickers.
    ///
    /// The tray asks for the library the first time it is drawn and the
    /// library is still empty, the way the origin asks for it when the tray
    /// opens.
    pub(crate) fn render(&self, app: &Whatfast, cx: &mut Context<Whatfast>) -> gpui::Div {
        let scale = app.ui_scale;
        self.ensure_loaded(app, cx);
        self.ensure_search_input(cx);

        let packs = app.store.sticker_packs();
        let active = {
            let state = self.state.borrow();
            active_pack(packs, &state.active_pack_id)
        };
        let sending = self.state.borrow().sending.clone();
        let query = self.state.borrow().query.clone();

        // The panel floats over the composer: a box with rounded corners, a
        // border and a shadow, as wide as the conversation, with a small gap
        // over the composer. The tray of the pictures of the composer lifts
        // it, so the box never covers them.
        v_flex()
            .absolute()
            .left_0()
            .right_0()
            .bottom(
                px((COMPOSER_HEIGHT + COMPOSER_GAP) * scale)
                    + app.pending_images.tray_height(scale),
            )
            .mx(px(8. * scale))
            .h(px(TRAY_HEIGHT * scale))
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(cx.theme().border)
            .shadow_lg()
            .bg(cx.theme().popover)
            .overflow_hidden()
            .child(self.render_tabs(app, packs, cx))
            // El buscador, con la lupa dentro, como en la referencia.
            .child(self.render_search(app, cx))
            .child(self.render_body(app, active, &sending, &query, cx))
            // Los tabs del pie, los mismos del panel de los emojis.
            .child(app.render_panel_tabs(cx))
    }

    /// The strip of the tabs of the packs.
    ///
    /// The reference draws the categories of the stickers with a drawing each
    /// one and the active one with a green line under it. The app has the
    /// packs of the library instead of the categories: they are the tabs of
    /// the row, with the short name of the pack, the active one marked with
    /// the accent of the theme and the rest dimmed.
    fn render_tabs(
        &self,
        app: &Whatfast,
        packs: &[StickerPack],
        cx: &mut Context<Whatfast>,
    ) -> gpui::Stateful<gpui::Div> {
        let scale = app.ui_scale;
        if packs.is_empty() {
            return div().id("sticker-tabs-empty");
        }
        let active = {
            let state = self.state.borrow();
            active_pack(packs, &state.active_pack_id).map(|pack| pack.id.clone())
        };
        let hover = cx.theme().muted;
        h_flex()
            .id("sticker-tab-strip")
            .flex_none()
            .gap(px(2.))
            .px(px(8. * scale))
            .py(px(6. * scale))
            .border_b_1()
            .border_color(cx.theme().border)
            .overflow_x_scroll()
            .children(packs.iter().enumerate().map(|(index, pack)| {
                let selected = active.as_deref() == Some(pack.id.as_str());
                let pack_id = pack.id.clone();
                let state = self.state.clone();
                div()
                    .id(ElementId::named_usize("sticker-pack-tab", index))
                    .size(px(TAB_SIZE * scale))
                    .flex_none()
                    .rounded_sm()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_xs()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(if selected {
                        cx.theme().foreground
                    } else {
                        cx.theme().muted_foreground
                    })
                    .cursor_pointer()
                    // La activa lleva el fondo verde suave y la linea verde
                    // debajo, como la categoria activa de la referencia.
                    .when(selected, |tab| {
                        tab.bg(cx.theme().accent.opacity(0.16))
                            .border_b_2()
                            .border_color(cx.theme().accent)
                    })
                    .hover(move |style| style.bg(hover))
                    .child(pack_label(&pack.name))
                    .on_click(cx.listener(move |_app, _, _, cx| {
                        state.borrow_mut().active_pack_id = pack_id.clone();
                        cx.notify();
                    }))
            }))
    }

    /// The field of the search.
    ///
    /// The reference draws a field with the green border of the accent and
    /// the magnifier inside. The field of text is born with the first
    /// drawing of the tray: until then the row shows the same field, but
    /// without the cursor.
    fn render_search(&self, app: &Whatfast, cx: &mut Context<Whatfast>) -> gpui::Div {
        let scale = app.ui_scale;
        let input = self.state.borrow().search_input.clone();
        let magnifier = || {
            Icon::new(IconName::Search)
                .with_size(px(16. * scale))
                .text_color(cx.theme().muted_foreground)
        };
        h_flex()
            .flex_none()
            .px_3()
            .pt_2()
            .pb_2()
            .gap_2()
            .child(match input {
                Some(input) => Input::new(&input)
                    .w_full()
                    .h(px(36. * scale))
                    .rounded_full()
                    .bg(cx.theme().input)
                    .border_1()
                    .border_color(cx.theme().accent)
                    .prefix(magnifier())
                    .into_any_element(),
                None => div()
                    .w_full()
                    .h(px(36. * scale))
                    .rounded_full()
                    .bg(cx.theme().input)
                    .border_1()
                    .border_color(cx.theme().accent)
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(magnifier())
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(SEARCH_PLACEHOLDER),
                    )
                    .into_any_element(),
            })
    }

    /// The grid of the stickers of the open pack.
    fn render_body(
        &self,
        app: &Whatfast,
        active: Option<&StickerPack>,
        sending: &str,
        query: &str,
        cx: &mut Context<Whatfast>,
    ) -> gpui::Stateful<gpui::Div> {
        let scale = app.ui_scale;
        let Some(pack) = active else {
            return self.render_empty(cx);
        };
        if pack.stickers.is_empty() {
            return self.render_empty(cx);
        }
        let stickers = pack
            .stickers
            .iter()
            .filter(|sticker| sticker_matches(sticker, query))
            .collect::<Vec<_>>();
        h_flex()
            .id("sticker-grid")
            .flex_1()
            .min_h_0()
            .flex_wrap()
            // Las celdas arrancan arriba: la rejilla no se estira en el alto
            // que sobra.
            .items_start()
            .content_start()
            .gap(px(CELL_GAP * scale))
            .p(px(10. * scale))
            .overflow_y_scroll()
            // La primera celda es el boton de crear un sticker.
            .child(self.render_create(app, cx))
            .children(stickers.iter().enumerate().map(|(index, sticker)| {
                let sticker_id = sticker.id.clone();
                let path = sticker_local_path(pack, &sticker.id).map(PathBuf::from);
                let busy = sending == sticker.id;
                let state = self.state.clone();
                div()
                    .id(ElementId::named_usize("sticker-cell", index))
                    .size(px(CELL_SIZE * scale))
                    .flex_none()
                    .rounded_sm()
                    .p(px(CELL_PADDING * scale))
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(|style| style.bg(cx.theme().muted))
                    .when(busy, |cell| cell.opacity(0.5))
                    .child(match path {
                        Some(path) => div().child(
                            img(path)
                                .size(px((CELL_SIZE - 2. * CELL_PADDING) * scale))
                                .object_fit(ObjectFit::Contain),
                        ),
                        None => div()
                            .size(px((CELL_SIZE - 2. * CELL_PADDING) * scale))
                            .rounded_sm()
                            .bg(cx.theme().muted)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                Icon::new(IconName::GalleryVerticalEnd)
                                    .with_size(px(20. * scale))
                                    .text_color(cx.theme().muted_foreground),
                            ),
                    })
                    .on_click(cx.listener(move |app, _, _, cx| {
                        if !state.borrow().sending.is_empty() {
                            return;
                        }
                        state.borrow_mut().sending = sticker_id.clone();
                        app.send_sticker_from_library(sticker_id.clone());
                        // port: the origin waits for the answer of the backend
                        // before it lets the user send another sticker. The
                        // request of the client does not wait, so the cell
                        // is released at the end of the turn.
                        let released = state.clone();
                        cx.defer(move |_cx| {
                            released.borrow_mut().sending.clear();
                        });
                        cx.notify();
                    }))
            }))
            .when(stickers.is_empty(), |grid| {
                grid.child(
                    div()
                        .w_full()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(i18n::t("No stickers found.")),
                )
            })
    }

    /// The first cell of the grid: the button that turns a picture of the
    /// computer into a sticker.
    fn render_create(
        &self,
        app: &Whatfast,
        cx: &mut Context<Whatfast>,
    ) -> gpui::Stateful<gpui::Div> {
        let scale = app.ui_scale;
        div()
            .id("sticker-create")
            .size(px(CELL_SIZE * scale))
            .flex_none()
            .rounded_sm()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_1()
            .cursor_pointer()
            .hover(|style| style.bg(cx.theme().muted))
            .child(
                Icon::new(IconName::Plus)
                    .with_size(px(24. * scale))
                    .text_color(cx.theme().accent),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().foreground)
                    .child(i18n::t("Create")),
            )
            .on_click(cx.listener(|app, _, window, cx| {
                app.choose_sticker(window, cx);
            }))
    }

    /// The message of the tray when there is nothing to show.
    fn render_empty(&self, cx: &mut Context<Whatfast>) -> gpui::Stateful<gpui::Div> {
        let state = self.state.clone();
        v_flex()
            .id("sticker-empty")
            .flex_1()
            .min_h_0()
            .items_center()
            .justify_center()
            .gap_2()
            .p(px(20.))
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(i18n::t("No stickers yet."))
            .child(
                "Stickers you have sent or received appear here, along with anything you have favourited."
            )
            .child(
                Button::new("sticker-retry")
                    .secondary()
                    .small()
                    .label(i18n::t("Retry"))
                    .on_click(cx.listener(move |app, _, _, cx| {
                        state.borrow_mut().requested = false;
                        app.load_stickers();
                        cx.notify();
                    })),
            )
    }

    /// Asks for the library when it is not there yet.
    ///
    /// The question cannot be asked while the application draws: the
    /// application is borrowed by then, so the request waits for the next turn
    /// of the loop.
    fn ensure_loaded(&self, app: &Whatfast, cx: &mut Context<Whatfast>) {
        if !app.store.sticker_packs().is_empty() || app.sticker_library_loading {
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
                app.load_stickers();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Creates the field of the search the first time the tray is drawn.
    ///
    /// The state of a field of text is born with a window, and the tray is
    /// drawn while the window is busy with the drawing: the field waits for
    /// the end of the cycle of the effects, where the window is free again.
    fn ensure_search_input(&self, cx: &mut Context<Whatfast>) {
        {
            let mut state = self.state.borrow_mut();
            if state.search_input.is_some() || state.search_pending {
                return;
            }
            state.search_pending = true;
        }
        let state = self.state.clone();
        let weak = cx.weak_entity();
        let entity_id = cx.entity_id();
        cx.defer(move |app| {
            let created = app.with_window(entity_id, |window, cx| {
                let input =
                    cx.new(|cx| InputState::new(window, cx).placeholder(SEARCH_PLACEHOLDER));
                // Los cambios del campo son los de la busqueda.
                weak.update(cx, |_this, cx| {
                    let subscription = cx.observe(&input, {
                        let state = state.clone();
                        move |_this, input, cx| {
                            let query = input.read(cx).value().to_string();
                            state.borrow_mut().query = query;
                            cx.notify();
                        }
                    });
                    state.borrow_mut().search_subscription = Some(subscription);
                })
                .ok();
                input
            });
            match created {
                Some(input) => {
                    state.borrow_mut().search_input = Some(input);
                    state.borrow_mut().search_pending = false;
                    // El campo recien nacido tiene que aparecer.
                    weak.update(app, |_this, cx| cx.notify()).ok();
                }
                None => {
                    // La ventana no estaba lista: el proximo dibujo lo
                    // vuelve a intentar.
                    state.borrow_mut().search_pending = false;
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sticker(id: &str, local_path: &str) -> whatfast_protocol::Sticker {
        whatfast_protocol::Sticker {
            id: id.to_string(),
            local_path: local_path.to_string(),
            ..Default::default()
        }
    }

    fn pack(id: &str, name: &str, stickers: Vec<whatfast_protocol::Sticker>) -> StickerPack {
        StickerPack {
            id: id.to_string(),
            name: name.to_string(),
            stickers,
        }
    }

    #[test]
    fn the_active_pack_is_the_one_the_user_chose() {
        let packs = vec![
            pack("favorites", "Favourites", Vec::new()),
            pack("recent", "Recently used", Vec::new()),
        ];
        assert_eq!(
            active_pack(&packs, "recent").map(|pack| pack.id.as_str()),
            Some("recent")
        );
    }

    #[test]
    fn the_active_pack_falls_back_to_the_first_one() {
        let packs = vec![
            pack("favorites", "Favourites", Vec::new()),
            pack("recent", "Recently used", Vec::new()),
        ];
        assert_eq!(
            active_pack(&packs, "gone").map(|pack| pack.id.as_str()),
            Some("favorites")
        );
    }

    #[test]
    fn there_is_no_active_pack_without_packs() {
        assert!(active_pack(&[], "favorites").is_none());
    }

    #[test]
    fn the_label_of_a_pack_is_its_first_two_letters() {
        assert_eq!(pack_label("Favourites"), "FA");
        assert_eq!(pack_label("Recently used"), "RE");
        assert_eq!(pack_label(""), "");
        assert_eq!(pack_label("a"), "A");
    }

    #[test]
    fn the_path_of_a_sticker_is_the_one_of_its_pack() {
        let packs = [pack(
            "favorites",
            "Favourites",
            vec![sticker("fav-1", "/tmp/fav-1.webp"), sticker("fav-2", "")],
        )];
        assert_eq!(
            sticker_local_path(&packs[0], "fav-1"),
            Some("/tmp/fav-1.webp")
        );
    }

    #[test]
    fn a_sticker_that_was_never_downloaded_has_no_path() {
        let packs = [pack(
            "favorites",
            "Favourites",
            vec![sticker("fav-1", "/tmp/fav-1.webp"), sticker("fav-2", "")],
        )];
        assert_eq!(sticker_local_path(&packs[0], "fav-2"), None);
        assert_eq!(sticker_local_path(&packs[0], "gone"), None);
    }

    #[test]
    fn an_empty_search_lets_every_sticker_through() {
        assert!(sticker_matches(&sticker("fav-1", ""), ""));
        assert!(sticker_matches(&sticker("fav-1", ""), "   "));
    }

    #[test]
    fn the_search_looks_at_the_id_of_the_sticker() {
        assert!(sticker_matches(&sticker("fav-1", ""), "fav"));
        assert!(sticker_matches(&sticker("fav-1", ""), "FAV-1"));
        assert!(!sticker_matches(&sticker("fav-1", ""), "recent"));
    }
}
