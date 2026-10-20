//! The native GPUI user interface of Whatfast.
//!
//! This crate replaces the SolidJS webview of the previous client. It is
//! the destination of the batches B-U1..B-U8 of the migration plan
//! (`PLAN.md`).
//!
//! Layer rules (see `RULEBOOK.md`):
//! - no webview, no DOM, no browser engine: GPUI paints directly
//! - the domain rules live in `whatfast-core`, the wire contract in
//!   `whatfast-protocol` and the data model in `whatfast-model`: this crate
//!   only renders and dispatches
//! - the linked-device credentials never enter this process: they live in
//!   the Go sidecar and only the RPC results reach here
//!
//! The modules:
//! - `app`: the state of the application and the actions that change it
//! - `rpc`: the lifecycle of the requests towards the sidecar
//! - `render`: the drawing of every panel
//! - `helpers`: the free functions used by the drawing
//! - `text`: the pure helpers of the text of a message
//! - `tokens`: the theme tokens and the scale of the interface

use gpui::{AppContext as _, KeyBinding, WindowBounds, WindowOptions, px, size};
use gpui_component::{Root, Theme, ThemeMode};
use gpui_component_assets::Assets;

mod app;
mod helpers;
mod render;
mod rpc;
mod text;
mod tokens;

use app::Whatfast;

/// The actions that switch between the recent chats.
///
/// They are declared at the root of the crate so that the drawing module can
/// listen to them: `gpui::actions!` puts the structs in the scope where it
/// is called.
gpui::actions!(whatfast, [CycleRecentChat, CycleRecentChatReverse]);

fn main() {
    gpui_platform::application().with_assets(Assets).run(|cx| {
        gpui_component::init(cx);
        cx.bind_keys([
            KeyBinding::new("ctrl-tab", CycleRecentChat, None),
            KeyBinding::new("ctrl-shift-tab", CycleRecentChatReverse, None),
        ]);
        Theme::change(ThemeMode::Dark, None, cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1180.), px(760.)), cx)),
            window_min_size: Some(size(px(480.), px(560.))),
            ..Default::default()
        };
        cx.spawn(async move |cx| {
            let opened = cx.open_window(options, |window, cx| {
                let app = cx.new(|cx| Whatfast::new(window, cx));
                // Fallback for when focus is lost or points at an unmounted
                // element: in that state the root div is off the dispatch path
                // and its on_action handlers never run, so also handle the
                // switcher actions globally. When the root handlers do run
                // they stop propagation, which keeps these from firing twice.
                let window_handle = window.window_handle();
                let weak = app.downgrade();
                cx.on_action(move |_: &CycleRecentChat, cx| {
                    Whatfast::cycle_recent_chat_from_global(&weak, window_handle, false, cx);
                });
                let weak = app.downgrade();
                cx.on_action(move |_: &CycleRecentChatReverse, cx| {
                    Whatfast::cycle_recent_chat_from_global(&weak, window_handle, true, cx);
                });
                cx.new(|cx| Root::new(app, window, cx))
            });
            if opened.is_err() {
                // There is no window to show the error in, so the process has
                // to say it in the only place that is left.
                eprintln!("Whatfast: no se pudo abrir la ventana");
                std::process::exit(1);
            }
        })
        .detach();
    });
}
