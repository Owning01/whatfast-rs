//! The native GPUI user interface of Whatfast.
//!
//! This crate replaces the SolidJS webview of the previous client. It is
//! the destination of the batches B-U1..B-U8 of the migration plan
//! (`PLAN.md`).
//!
//! Layer rules (see `RULEBOOK.md`):
//! - no webview, no DOM, no browser engine: GPUI paints directly
//! - the domain rules live in `whatfast-core`, the wire contract in
//!   `whatfast-protocol`: this crate only renders and dispatches
//! - the linked-device credentials never enter this process: they live in
//!   the Go sidecar and only the RPC results reach here

use gpui::{Context, Render, Window, WindowBounds, WindowOptions, div, px, size};
use gpui_component::{Theme, ThemeMode};
use gpui_component_assets::Assets;

/// The root view of the application.
///
/// The real layout (three panes, tab strip, title bar, the chat list) arrives
/// with the batches B-U1 (tokens) and B-U2 (shell).
struct Whatfast;

impl Render for Whatfast {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl gpui::IntoElement {
        div()
            .flex_1()
            .flex_col()
            .items_center()
            .justify_center()
            .child(div().text_2xl().child("Whatfast"))
    }
}

fn main() {
    gpui_platform::application().with_assets(Assets).run(|cx| {
        gpui_component::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1180.), px(760.)), cx)),
            window_min_size: Some(size(px(480.), px(560.))),
            ..Default::default()
        };
        cx.spawn(async move |cx| {
            cx.open_window(options, |_window, cx| cx.new(|_cx| Whatfast))
                .expect("open the Whatfast window");
        })
        .detach();
    });
}
