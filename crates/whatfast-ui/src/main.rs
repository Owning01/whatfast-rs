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

use gpui::{App, Window, WindowOptions};

/// The shell of the application. The real layout (three panes, tab strip,
/// title bar) arrives with the batches B-U1 and B-U2.
fn main() {
    gpui::Application::new().run(|app: &mut App| {
        let _ = app.open_window(WindowOptions {
            title: "Whatfast".into(),
            ..Default::default()
        }, |window: &mut Window, _| {
            window.render(|_frame, _cx| {});
        });
    });
}
