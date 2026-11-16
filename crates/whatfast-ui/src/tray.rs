//! The icon of Whatfast in the tray of the system.
//!
//! Closing the window does not close the application: the window hides and
//! the icon stays in the tray, so the sidecar keeps the session of the phone
//! and the messages keep arriving. The menu of the icon brings the window
//! back or quits for real.
//!
//! GPUI has no tray of its own, so this module talks to the shell of Windows:
//! a hidden window owns the icon (`Shell_NotifyIconW`) and its message loop
//! runs on a thread of its own. The callbacks of the icon only touch Windows:
//! showing the main window and posting the close that quits the app do not
//! need the UI thread.

#[cfg(windows)]
mod windows_tray {
    use std::mem::size_of;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

    use gpui::{App, WeakEntity, Window};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use whatfast_i18n as i18n;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{
        ExtractIconExW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
        Shell_NotifyIconW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
        DispatchMessageW, GetCursorPos, GetMessageW, IDI_APPLICATION, LoadIconW, MF_STRING, MSG,
        PostMessageW, PostQuitMessage, RegisterClassW, SW_HIDE, SW_RESTORE, SW_SHOW,
        SetForegroundWindow, ShowWindow, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WM_APP,
        WM_CLOSE, WM_COMMAND, WM_DESTROY, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSW,
    };

    use crate::app::Whatfast;

    /// The identifier of the icon inside the profile of the shell.
    const ICON_ID: u32 = 1;
    /// The message the icon sends to its window.
    const WM_TRAY: u32 = WM_APP + 1;
    /// The commands of the menu of the icon.
    const CMD_OPEN: usize = 1;
    const CMD_QUIT: usize = 2;

    /// The window of the application, the one that the icon shows and hides.
    static MAIN_WINDOW: AtomicIsize = AtomicIsize::new(0);
    /// The hidden window that owns the icon, so a quit can remove it.
    static TRAY_WINDOW: AtomicIsize = AtomicIsize::new(0);
    /// Whether the user asked to quit from the menu: the close is real.
    static QUITTING: AtomicBool = AtomicBool::new(false);
    /// The labels of the menu, in the language of the profile.
    static LABELS: Mutex<Option<Labels>> = Mutex::new(None);

    struct Labels {
        open: Vec<u16>,
        quit: Vec<u16>,
    }

    /// The handle of the tray window, so the app can ask it to remove the
    /// icon when it goes away.
    pub(crate) struct Tray {
        window: isize,
    }

    impl Tray {
        /// Puts the icon in the tray and wires the close of the window to
        /// hide it instead of quitting.
        ///
        /// Returns `None` when the shell does not take the icon: the app then
        /// closes as it always did, with no hidden window that nobody can
        /// bring back.
        pub(crate) fn install(
            window: &Window,
            cx: &App,
            app: WeakEntity<Whatfast>,
        ) -> Option<Self> {
            if let Ok(mut labels) = LABELS.lock() {
                *labels = Some(Labels {
                    open: wide(i18n::t("Open Whatfast")),
                    quit: wide(i18n::t("Quit")),
                });
            }
            // The thread registers its class and creates its window before
            // it answers: a shell that does not take the icon leaves the
            // close of the window as it always was. The answer has a deadline
            // because this runs on the UI thread: a shell that never answers
            // must not freeze the window.
            let (sender, receiver) = std::sync::mpsc::channel();
            std::thread::Builder::new()
                .name("whatfast-tray".into())
                .spawn(move || {
                    // The message loop of `run_tray` ends when the window is
                    // destroyed, which is when the app drops this value.
                    unsafe { run_tray(sender) };
                })
                .ok()?;
            let tray_window = receiver
                .recv_timeout(std::time::Duration::from_secs(2))
                .ok()
                .flatten()?;
            let Some(main) = main_handle(window) else {
                // The icon has no window to show: take it away instead of
                // leaving an icon that does nothing.
                unsafe {
                    let _ = PostMessageW(tray_window as HWND, WM_CLOSE, 0, 0);
                }
                return None;
            };
            MAIN_WINDOW.store(main, Ordering::SeqCst);
            // Closing the window hides it: the application stays alive in the
            // tray, and only the menu of the icon closes it for real. The real
            // close writes the text that the debounce has not saved yet.
            window.on_window_should_close(cx, move |_window, cx| {
                if QUITTING.load(Ordering::SeqCst) {
                    let _ = app.update(cx, |this, _| this.save_drafts_now());
                    return true;
                }
                hide_main();
                false
            });
            Some(Self {
                window: tray_window,
            })
        }
    }

    impl Drop for Tray {
        fn drop(&mut self) {
            let window = self.window as HWND;
            if !window.is_null() {
                unsafe {
                    let _ = PostMessageW(window, WM_CLOSE, 0, 0);
                }
            }
        }
    }

    /// The handle of the window of GPUI: the one the icon shows and hides.
    fn main_handle(window: &Window) -> Option<isize> {
        // `Window` also has a `window_handle` of its own that answers the
        // handle of GPUI, so the call names the trait.
        match HasWindowHandle::window_handle(window).ok()?.as_raw() {
            RawWindowHandle::Win32(window) => Some(window.hwnd.get()),
            _ => None,
        }
    }

    /// Hides the window of the application without closing it.
    fn hide_main() {
        let window = MAIN_WINDOW.load(Ordering::SeqCst) as HWND;
        if !window.is_null() {
            unsafe {
                let _ = ShowWindow(window, SW_HIDE);
            }
        }
    }

    /// Brings the window back to the front.
    fn show_main() {
        let window = MAIN_WINDOW.load(Ordering::SeqCst) as HWND;
        if window.is_null() {
            return;
        }
        unsafe {
            let _ = ShowWindow(window, SW_SHOW);
            let _ = ShowWindow(window, SW_RESTORE);
            let _ = SetForegroundWindow(window);
        }
    }

    /// Quits the application for real: the close goes through the window of
    /// GPUI, which is the one that owns the sidecar and the lock.
    fn quit_app() {
        QUITTING.store(true, Ordering::SeqCst);
        // The icon goes away before the process ends: the shell would keep
        // drawing it until the user moves the mouse over it.
        let tray = TRAY_WINDOW.load(Ordering::SeqCst) as HWND;
        if !tray.is_null() {
            unsafe {
                let _ = PostMessageW(tray, WM_CLOSE, 0, 0);
            }
        }
        let window = MAIN_WINDOW.load(Ordering::SeqCst) as HWND;
        if window.is_null() {
            std::process::exit(0);
        }
        unsafe {
            if PostMessageW(window, WM_CLOSE, 0, 0) == 0 {
                std::process::exit(0);
            }
        }
    }

    /// Creates the hidden window, puts the icon in the tray and runs the
    /// message loop until the window is destroyed.
    ///
    /// The handle of the window goes back to the caller as soon as the icon
    /// is in the shell: from then on this thread only answers the messages of
    /// the icon.
    unsafe fn run_tray(sender: std::sync::mpsc::Sender<Option<isize>>) {
        let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
        let class_name = wide("WhatfastTrayWindow");
        // The window of the tray does not carry the title of the app: the
        // second instance finds the window to show by that title, and a
        // hidden window of the tray would answer first.
        let window_name = wide("Whatfast tray");
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class_name.as_ptr(),
            ..Default::default()
        };
        // The class is registered once per process; a second call answers 0
        // with ERROR_CLASS_ALREADY_EXISTS, which is not a failure.
        unsafe {
            RegisterClassW(&class);
        }
        let window = unsafe {
            CreateWindowExW(
                0,
                class_name.as_ptr(),
                window_name.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            )
        };
        if window.is_null() {
            let _ = sender.send(None);
            return;
        }
        if !add_icon(window) {
            unsafe {
                let _ = DestroyWindow(window);
            }
            let _ = sender.send(None);
            return;
        }
        TRAY_WINDOW.store(window as isize, Ordering::SeqCst);
        let _ = sender.send(Some(window as isize));
        let mut message = MSG::default();
        loop {
            let result = unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) };
            if result <= 0 {
                break;
            }
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }

    /// Puts the icon of Whatfast in the tray, with the icon of the program.
    fn add_icon(window: HWND) -> bool {
        let mut icon = std::ptr::null_mut();
        if let Ok(path) = std::env::current_exe() {
            let path = wide(&path.to_string_lossy());
            unsafe {
                ExtractIconExW(path.as_ptr(), 0, std::ptr::null_mut(), &mut icon, 1);
            }
        }
        if icon.is_null() {
            icon = unsafe { LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) };
        }
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: window,
            uID: ICON_ID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: WM_TRAY,
            hIcon: icon,
            ..Default::default()
        };
        let tooltip = wide("Whatfast");
        let length = tooltip.len().min(data.szTip.len());
        data.szTip[..length].copy_from_slice(&tooltip[..length]);
        unsafe { Shell_NotifyIconW(NIM_ADD, &data) != 0 }
    }

    /// Removes the icon from the tray.
    fn remove_icon(window: HWND) {
        let data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: window,
            uID: ICON_ID,
            ..Default::default()
        };
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
        }
    }

    /// The menu of the icon: open the window or quit.
    fn show_menu(window: HWND) {
        let labels = LABELS.lock().ok();
        let Some(labels) = labels.as_ref().and_then(|labels| labels.as_ref()) else {
            return;
        };
        let menu = unsafe { CreatePopupMenu() };
        if menu.is_null() {
            return;
        }
        unsafe {
            let _ = AppendMenuW(menu, MF_STRING, CMD_OPEN, labels.open.as_ptr());
            let _ = AppendMenuW(menu, MF_STRING, CMD_QUIT, labels.quit.as_ptr());
            let mut point = POINT::default();
            let _ = GetCursorPos(&mut point);
            // The menu only closes when the click goes outside if the window
            // that owns it is in front.
            let _ = SetForegroundWindow(window);
            let _ = TrackPopupMenu(
                menu,
                TPM_RIGHTBUTTON,
                point.x,
                point.y,
                0,
                window,
                std::ptr::null(),
            );
            let _ = DestroyMenu(menu);
        }
    }

    /// The messages of the icon and of its menu.
    unsafe extern "system" fn window_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_TRAY => {
                match lparam as u32 {
                    WM_LBUTTONUP | WM_LBUTTONDBLCLK => show_main(),
                    WM_RBUTTONUP => show_menu(window),
                    _ => {}
                }
                0
            }
            WM_COMMAND => {
                match wparam & 0xffff {
                    CMD_OPEN => show_main(),
                    CMD_QUIT => quit_app(),
                    _ => {}
                }
                0
            }
            WM_CLOSE => {
                unsafe {
                    let _ = DestroyWindow(window);
                }
                0
            }
            WM_DESTROY => {
                remove_icon(window);
                unsafe {
                    PostQuitMessage(0);
                }
                0
            }
            _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
        }
    }

    /// A string of Rust as the wide string that Windows reads.
    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }
}

#[cfg(windows)]
pub(crate) use windows_tray::Tray;

#[cfg(target_os = "linux")]
mod linux_tray {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use gpui::{App, AppContext as _, WeakEntity, Window};
    use whatfast_i18n as i18n;
    use zbus::blocking::Connection;
    use zbus::interface;
    use zbus::zvariant::{OwnedValue, Value};

    use crate::app::Whatfast;

    enum TrayEvent {
        Open,
        Quit,
    }

    pub(crate) struct Tray {
        _connection: Connection,
        _quitting: Arc<AtomicBool>,
    }

    struct StatusNotifierItem {
        events: async_channel::Sender<TrayEvent>,
    }

    #[interface(name = "org.kde.StatusNotifierItem")]
    impl StatusNotifierItem {
        #[zbus(property)]
        fn category(&self) -> &str {
            "Communications"
        }

        #[zbus(property)]
        fn id(&self) -> &str {
            "whatfast"
        }

        #[zbus(property)]
        fn title(&self) -> &str {
            "Whatfast"
        }

        #[zbus(property)]
        fn status(&self) -> &str {
            "Active"
        }

        #[zbus(property)]
        fn icon_name(&self) -> &str {
            "whatfast"
        }

        #[zbus(property)]
        fn menu(&self) -> zbus::zvariant::ObjectPath<'_> {
            zbus::zvariant::ObjectPath::from_static_str("/MenuBar").expect("valid path")
        }

        #[zbus(property)]
        fn item_is_menu(&self) -> bool {
            false
        }

        fn activate(&self, _x: i32, _y: i32) {
            let _ = self.events.try_send(TrayEvent::Open);
        }

        fn secondary_activate(&self, _x: i32, _y: i32) {
            let _ = self.events.try_send(TrayEvent::Open);
        }
    }

    struct DbusMenu {
        open_label: String,
        quit_label: String,
        events: async_channel::Sender<TrayEvent>,
    }

    #[interface(name = "com.canonical.dbusmenu")]
    impl DbusMenu {
        #[zbus(property)]
        fn version(&self) -> u32 {
            3
        }

        #[zbus(property)]
        fn status(&self) -> &str {
            "normal"
        }

        fn get_layout(
            &self,
            _parent_id: i32,
            _recursion_depth: i32,
            _property_names: Vec<String>,
        ) -> (u32, (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>)) {
            let open_item = make_menu_item(1, &self.open_label);
            let quit_item = make_menu_item(2, &self.quit_label);
            let mut root_props = HashMap::new();
            root_props.insert(
                "children-display".to_string(),
                OwnedValue::from(Value::from("submenu")),
            );
            (1, (0, root_props, vec![open_item, quit_item]))
        }

        fn event(&self, id: i32, event_id: &str, _data: Value<'_>, _timestamp: u32) {
            if event_id != "clicked" {
                return;
            }
            match id {
                1 => {
                    let _ = self.events.try_send(TrayEvent::Open);
                }
                2 => {
                    let _ = self.events.try_send(TrayEvent::Quit);
                }
                _ => {}
            }
        }
    }

    fn make_menu_item(id: i32, label: &str) -> OwnedValue {
        let mut props: HashMap<String, Value<'_>> = HashMap::new();
        props.insert("label".to_string(), Value::from(label.to_string()));
        props.insert("enabled".to_string(), Value::from(true));
        props.insert("visible".to_string(), Value::from(true));
        let children: Vec<Value<'_>> = Vec::new();
        OwnedValue::try_from(Value::from((id, props, children))).expect("menu item")
    }

    impl Tray {
        pub(crate) fn install(
            window: &Window,
            cx: &App,
            app: WeakEntity<Whatfast>,
        ) -> Option<Self> {
            let (tx, rx) = async_channel::bounded::<TrayEvent>(8);
            let sni = StatusNotifierItem { events: tx.clone() };
            let menu = DbusMenu {
                open_label: i18n::t("Open Whatfast").to_string(),
                quit_label: i18n::t("Quit").to_string(),
                events: tx,
            };
            let connection = Connection::session().ok()?;
            connection
                .object_server()
                .at("/StatusNotifierItem", sni)
                .ok()?;
            connection.object_server().at("/MenuBar", menu).ok()?;
            let service = format!("org.kde.StatusNotifierItem-{}-1", std::process::id());
            connection.request_name(service.as_str()).ok()?;
            connection
                .call_method(
                    Some("org.kde.StatusNotifierWatcher"),
                    "/StatusNotifierWatcher",
                    Some("org.kde.StatusNotifierWatcher"),
                    "RegisterStatusNotifierItem",
                    &(service.as_str(),),
                )
                .ok()?;

            let quitting = Arc::new(AtomicBool::new(false));
            let quitting_close = Arc::clone(&quitting);
            window.on_window_should_close(cx, move |window, _cx| {
                if quitting_close.load(Ordering::SeqCst) {
                    return true;
                }
                window.minimize_window();
                false
            });

            let handle = window.window_handle();
            let quitting_task = Arc::clone(&quitting);
            cx.spawn(async move |cx| {
                while let Ok(event) = rx.recv().await {
                    match event {
                        TrayEvent::Open => {
                            let _ = handle.update(cx, |_, window, _| {
                                window.activate_window();
                            });
                        }
                        TrayEvent::Quit => {
                            quitting_task.store(true, Ordering::SeqCst);
                            // The text that the debounce has not saved yet
                            // lands before the process leaves.
                            let _ = app.update(cx, |this, _| this.save_drafts_now());
                            let _ = cx.update(|cx| cx.quit());
                            break;
                        }
                    }
                }
            })
            .detach();

            Some(Self {
                _connection: connection,
                _quitting: quitting,
            })
        }
    }
}

#[cfg(target_os = "linux")]
pub(crate) use linux_tray::Tray;

#[cfg(not(any(windows, target_os = "linux")))]
pub(crate) struct Tray;

#[cfg(not(any(windows, target_os = "linux")))]
impl Tray {
    /// There is no tray on other targets: closing the window quits the app.
    pub(crate) fn install(
        _window: &gpui::Window,
        _cx: &gpui::App,
        _app: gpui::WeakEntity<crate::app::Whatfast>,
    ) -> Option<Self> {
        None
    }
}

/// Enables or disables starting Whatfast automatically on OS login.
pub(crate) fn set_autostart(enabled: bool) {
    #[cfg(windows)]
    {
        if let Ok(exe) = std::env::current_exe() {
            let exe_str = exe.to_string_lossy().into_owned();
            if enabled {
                let _ = std::process::Command::new("reg")
                    .args([
                        "add",
                        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                        "/v",
                        "Whatfast",
                        "/t",
                        "REG_SZ",
                        "/d",
                        &format!("\"{exe_str}\""),
                        "/f",
                    ])
                    .output();
            } else {
                let _ = std::process::Command::new("reg")
                    .args([
                        "delete",
                        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                        "/v",
                        "Whatfast",
                        "/f",
                    ])
                    .output();
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            let dir = std::path::PathBuf::from(home).join(".config/autostart");
            let file = dir.join("whatfast.desktop");
            if enabled {
                if std::fs::create_dir_all(&dir).is_ok() {
                    let exe = std::env::current_exe()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_else(|_| "whatfast".to_string());
                    let content = format!(
                        "[Desktop Entry]\nType=Application\nName=Whatfast\nExec={exe}\nX-GNOME-Autostart-enabled=true\n"
                    );
                    let _ = std::fs::write(file, content);
                }
            } else {
                let _ = std::fs::remove_file(file);
            }
        }
    }
}

/// Updates the taskbar/dock unread badge count where supported.
#[allow(dead_code)]
pub(crate) fn update_unread_badge(count: u32) {
    #[cfg(target_os = "linux")]
    {
        let visible = if count > 0 { "true" } else { "false" };
        let _ = std::process::Command::new("dbus-send")
            .args([
                "--session",
                "--type=signal",
                "/com/whatfast/launcher",
                "com.canonical.Unity.LauncherEntry.Update",
                "string:application://whatfast.desktop",
                &format!("dict:string:variant:count,int64:{count},count-visible,boolean:{visible}"),
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
    let _ = count;
}
