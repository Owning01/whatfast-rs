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

    use gpui::{App, Window};
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
        pub(crate) fn install(window: &Window, cx: &App) -> Option<Self> {
            if let Ok(mut labels) = LABELS.lock() {
                *labels = Some(Labels {
                    open: wide(&i18n::t("Open Whatfast")),
                    quit: wide(&i18n::t("Quit")),
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
            // tray, and only the menu of the icon closes it for real.
            window.on_window_should_close(cx, |_window, _cx| {
                if QUITTING.load(Ordering::SeqCst) {
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

#[cfg(not(windows))]
pub(crate) struct Tray;

#[cfg(not(windows))]
impl Tray {
    /// There is no tray outside Windows: the window closes the app.
    pub(crate) fn install(_window: &gpui::Window, _cx: &gpui::App) -> Option<Self> {
        None
    }
}
