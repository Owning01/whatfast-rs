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
//! - `attachments`: the card of a file that comes with a message
//! - `composer_attachments`: the pictures that wait for the send button
//! - `emoji_recent`: the emojis that the user used lately
//! - `helpers`: the free functions used by the drawing
//! - `icons`: the icon set of the web client
//! - `instance`: the lock that keeps a single instance of the app
//! - `local_time`: the offset of the local time zone, asked to the system
//! - `media_editor`: the crop and the rotation of a picture
//! - `members`: the panel of the members of a group
//! - `message_actions`: the menu of a message, the forward, the poll and
//!   the clip of the composer
//! - `navrail`: the column of icons at the left
//! - `newsletters`: the panel of the channels
//! - `quick_ask`: the fast mode of the chat, a question to an agent
//! - `automations_panel`: the tasks of the timer, under the drafts
//! - `recording`: the capture of the voice notes of the composer
//! - `statuses`: the strip of the states of the contacts
//! - `stickers`: the tray of the stickers
//! - `tabs`: the strip of open chats
//! - `text`: the pure helpers of the text of a message
//! - `tokens`: the theme tokens and the scale of the interface
//! - `ubicaciones`: the card of a place and the card of a contact

// Whatfast is a window, not a console program: with the `windows` subsystem
// the executable does not open a black terminal behind the interface when it
// is started from the desktop, from the Start menu or from the installer.
#![cfg_attr(windows, windows_subsystem = "windows")]

use gpui::{AppContext as _, KeyBinding, WindowBounds, WindowOptions, px, size};
use gpui_component::Root;

mod agents_panel;
mod app;
mod attachments;
mod audio;
mod automations_panel;
mod call_audio;
mod chat_drafts;
mod composer_attachments;
mod emoji_recent;
mod helpers;
mod icons;
mod instance;
mod local_time;
mod media_editor;
mod members;
mod message_actions;
mod navrail;
mod newsletters;
mod notifications;
mod quick_ask;
mod recording;
mod render;
mod rpc;
mod statuses;
mod stickers;
mod tabs;
mod text;
mod tokens;
mod ubicaciones;
mod video;

use app::Whatfast;

fn main() {
    // The same executable is the MCP server of the agents: `whatfast mcp
    // --agent <id>` runs before any window exists and before the instance
    // lock, because an agent works while the application is open.
    let mut arguments = std::env::args().skip(1);
    if arguments.next().as_deref() == Some("mcp") {
        std::process::exit(whatfast_mcp::run(arguments.collect::<Vec<_>>()));
    }
    // One instance only: two sidecars with the same device session would
    // confuse the server and could invalidate the session of the phone.
    let Some(_lock) = instance::InstanceLock::acquire(&whatfast_model::paths::data_dir()) else {
        warn("Whatfast is already running.");
        std::process::exit(1);
    };
    // The source of the assets is the one of `gpui_component` plus the icons
    // of Whatfast: without it the icons of the app are not painted.
    gpui_platform::application()
        .with_assets(crate::icons::WhatfastAssets::new())
        .run(|cx| {
            gpui_component::init(cx);
            cx.bind_keys([
                KeyBinding::new("ctrl-tab", app::CycleRecentChat, None),
                KeyBinding::new("ctrl-shift-tab", app::CycleRecentChatReverse, None),
                // Las tres vistas que no son conversaciones: los estados, los
                // canales y los archivados.
                KeyBinding::new("ctrl-shift-s", app::ToggleStates, None),
                KeyBinding::new("ctrl-shift-n", app::ToggleChannels, None),
                KeyBinding::new("ctrl-shift-a", app::ToggleAgents, None),
                KeyBinding::new("ctrl-shift-h", app::ToggleArchived, None),
                // Accesibilidad de teclado y atajos de navegacion:
                KeyBinding::new("ctrl-f", app::FocusSearch, None),
                KeyBinding::new("cmd-f", app::FocusSearch, None),
                // El modo rapido del chat: una pregunta sobre la conversacion
                // abierta, con la respuesta en el cuadro de escribir.
                KeyBinding::new("ctrl-shift-q", app::OpenQuickAsk, None),
                KeyBinding::new("escape", app::EscapeAction, None),
                KeyBinding::new("ctrl-down", app::ScrollToBottomAction, None),
                KeyBinding::new("end", app::ScrollToBottomAction, None),
                KeyBinding::new("ctrl-up", app::ScrollToTopAction, None),
                KeyBinding::new("home", app::ScrollToTopAction, None),
                KeyBinding::new("pageup", app::PageUpMessages, None),
                KeyBinding::new("pagedown", app::PageDownMessages, None),
                KeyBinding::new("alt-up", app::SelectPreviousChat, None),
                KeyBinding::new("alt-down", app::SelectNextChat, None),
                KeyBinding::new("ctrl-,", app::OpenSettingsAction, None),
                KeyBinding::new("cmd-,", app::OpenSettingsAction, None),
            ]);
            // The theme of Whatfast goes over the one of `gpui_component`: the
            // mode comes first, because `Theme::change` rewrites every color from
            // the configuration of the mode. The profile chooses the theme and
            // the language before the window exists, so the first frame is
            // already in the language and the colors of the user.
            let settings = whatfast_model::settings::load();
            let scale = tokens::normalized_ui_scale(settings.ui_scale);
            let custom: Vec<whatfast_core::theme::Theme> = settings
                .custom_themes
                .iter()
                .filter_map(whatfast_core::theme::normalize_theme)
                .collect();
            let theme = tokens::theme_by_id(&settings.theme_id, &custom);
            gpui_component::Theme::change(
                if theme.appearance == whatfast_core::theme::ThemeAppearance::Light {
                    gpui_component::ThemeMode::Light
                } else {
                    gpui_component::ThemeMode::Dark
                },
                None,
                cx,
            );
            tokens::apply_theme(&theme, scale, cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1180.), px(760.)), cx)),
                window_min_size: Some(size(px(480.), px(560.))),
                ..Default::default()
            };
            cx.spawn(async move |cx| {
                let opened = cx.open_window(options, |window, cx| {
                    // The title is what the taskbar and Alt-Tab show: without
                    // it the window of the app has no name in the system.
                    window.set_window_title("Whatfast");
                    let app = cx.new(|cx| Whatfast::new(window, cx));
                    // Fallback for when focus is lost or points at an unmounted
                    // element: in that state the root div is off the dispatch path
                    // and its on_action handlers never run, so also handle the
                    // switcher actions globally. When the root handlers do run
                    // they stop propagation, which keeps these from firing twice.
                    let window_handle = window.window_handle();
                    let weak = app.downgrade();
                    cx.on_action(move |_: &app::CycleRecentChat, cx| {
                        Whatfast::cycle_recent_chat_from_global(&weak, window_handle, false, cx);
                    });
                    let weak = app.downgrade();
                    cx.on_action(move |_: &app::CycleRecentChatReverse, cx| {
                        Whatfast::cycle_recent_chat_from_global(&weak, window_handle, true, cx);
                    });
                    cx.new(|cx| Root::new(app, window, cx))
                });
                if opened.is_err() {
                    // There is no window to show the error in, so the process has
                    // to say it in the only place that is left.
                    warn("Whatfast could not open its window.");
                    std::process::exit(1);
                }
            })
            .detach();
        });
}

/// Says a message that happens before there is a window to show it in.
///
/// The executable of Windows has no console (`windows_subsystem`), so the
/// message goes in a dialog of the system.
fn warn(message: &str) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONWARNING, MB_OK, MessageBoxW};
        let titulo: Vec<u16> = "Whatfast".encode_utf16().chain(Some(0)).collect();
        let texto: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
        // SAFETY: the two strings are vectors of `u16` that end in a zero, so
        // the call reads them as the `PCWSTR` that Windows expects.
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                texto.as_ptr(),
                titulo.as_ptr(),
                MB_OK | MB_ICONWARNING,
            );
        }
    }
    #[cfg(not(windows))]
    {
        eprintln!("{message}");
    }
}
