//! The tray of the library of stickers.
//!
//! port: taken from the origin's `tauri/src/components/StickerTray.tsx`. The
//! lengths come from the stylesheet of the origin
//! (`tauri/src/styles.css`, the classes `sticker-tray`, `sticker-tab-strip`,
//! `sticker-pack-tab`, `sticker-grid` and `sticker-cell`).
//!
//! The data is not here: the packs live in the store of the application
//! (`store.sticker_packs()`), and the panel only draws them. The two actions
//! it needs already exist in `app.rs`: `load_stickers()` and
//! `send_sticker_from_library()`.

#![allow(dead_code)]

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Context, ElementId, InteractiveElement as _, ObjectFit, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, StyledImage as _, div, img, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};

use whatfast_protocol::StickerPack;

use crate::app::Whatfast;

/// The width of the tray, in pixels, before the scale.
pub(crate) const TRAY_WIDTH: f32 = 360.0;
/// The height of the tray, in pixels, before the scale.
pub(crate) const TRAY_HEIGHT: f32 = 420.0;
/// The side of one cell of the grid of stickers, in pixels, before the scale.
pub(crate) const CELL_SIZE: f32 = 72.0;
/// The side of one tab of the strip of packs, in pixels, before the scale.
pub(crate) const TAB_SIZE: f32 = 34.0;
/// The gap between two cells of the grid, in pixels, before the scale.
pub(crate) const CELL_GAP: f32 = 6.0;

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

        let packs = app.store.sticker_packs();
        let active = {
            let state = self.state.borrow();
            active_pack(packs, &state.active_pack_id)
        };
        let sending = self.state.borrow().sending.clone();

        v_flex()
            .w(px(TRAY_WIDTH * scale))
            .h(px(TRAY_HEIGHT * scale))
            .flex_shrink_0()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .overflow_hidden()
            .child(self.render_tabs(app, packs, cx))
            .child(self.render_body(app, active, &sending, cx))
    }

    /// The strip of the tabs of the packs.
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
                    .border_1()
                    .border_color(if selected {
                        cx.theme().primary
                    } else {
                        gpui::transparent_black()
                    })
                    .bg(if selected {
                        cx.theme().accent.opacity(0.12)
                    } else {
                        gpui::transparent_black()
                    })
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_xs()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(pack_label(&pack.name))
                    .on_click(cx.listener(move |_app, _, _, cx| {
                        state.borrow_mut().active_pack_id = pack_id.clone();
                        cx.notify();
                    }))
            }))
    }

    /// The grid of the stickers of the open pack.
    fn render_body(
        &self,
        app: &Whatfast,
        active: Option<&StickerPack>,
        sending: &str,
        cx: &mut Context<Whatfast>,
    ) -> gpui::Stateful<gpui::Div> {
        let scale = app.ui_scale;
        let Some(pack) = active else {
            return self.render_empty(cx);
        };
        if pack.stickers.is_empty() {
            return self.render_empty(cx);
        }
        h_flex()
            .id("sticker-grid")
            .flex_1()
            .min_h_0()
            .flex_wrap()
            .gap(px(CELL_GAP * scale))
            .p(px(10. * scale))
            .overflow_y_scroll()
            .children(pack.stickers.iter().enumerate().map(|(index, sticker)| {
                let sticker_id = sticker.id.clone();
                let path = sticker_local_path(pack, &sticker.id).map(PathBuf::from);
                let busy = sending == sticker.id;
                let state = self.state.clone();
                div()
                    .id(ElementId::named_usize("sticker-cell", index))
                    .size(px(CELL_SIZE * scale))
                    .flex_none()
                    .rounded_sm()
                    .p(px(4. * scale))
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(|style| style.bg(cx.theme().muted))
                    .when(busy, |cell| cell.opacity(0.5))
                    .child(match path {
                        Some(path) => div().child(
                            img(path)
                                .size(px(CELL_SIZE * scale))
                                .object_fit(ObjectFit::Contain),
                        ),
                        None => div()
                            .size(px(CELL_SIZE * scale))
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
                        state.borrow_mut().sending.clear();
                        cx.notify();
                    }))
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
            .child("No stickers yet.")
            .child(
                "Stickers you have sent or received appear here, along with anything you have favourited."
            )
            .child(
                Button::new("sticker-retry")
                    .secondary()
                    .small()
                    .label("Retry")
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
}
