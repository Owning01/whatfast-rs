//! The toasts of the system for the messages that arrive in the background.
//!
//! port: the origin asked the webview for a `Notification`
//! (`tauri/src/lib/notification-policy.ts` decided *when*; the browser drew
//! it). The native app has no browser: the drawing is a call to the
//! notification center of the operating system, and the app crate decides
//! when.
//!
//! The call can block while the system shows the toast, so it runs in its own
//! thread and the interface only leaves a note in the queue. A failure to
//! show a toast is silent: the message is already in the chat, and a broken
//! notification server is not a reason to interrupt the user.
//!
//! The toast never plays its own sound: the app has the sounds of the
//! messages (`crate::sounds`), and two sounds for one message is noise.

use std::sync::mpsc::{self, Sender};
use std::time::Duration;

/// One toast: the title of the conversation and the line of the message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Note {
    pub(crate) title: String,
    pub(crate) body: String,
}

/// The queue of the toasts.
pub(crate) struct Notifier {
    sender: Option<Sender<Note>>,
}

impl Notifier {
    /// Starts the thread that shows the toasts.
    pub(crate) fn new() -> Self {
        let (sender, receiver) = mpsc::channel::<Note>();
        let started = std::thread::Builder::new()
            .name("whatfast-notifications".to_string())
            .spawn(move || {
                // The thread of the toast owns the apartment of the platform:
                // Windows needs it before the first WinRT call, and the
                // shortcut of the app is registered in the same turn.
                #[cfg(windows)]
                toast::prepare();
                loop {
                    match receiver.recv_timeout(Duration::from_millis(50)) {
                        Ok(note) => show(&note),
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                    // WinRT paints the toast with the messages of this thread.
                    // Without a pump Windows files it in the center but never
                    // lifts it over the desk.
                    #[cfg(windows)]
                    toast::pump_messages();
                }
            });
        Self {
            // A thread that cannot start leaves the app without toasts, which
            // is better than a panic in the middle of a message.
            sender: started.ok().map(|_| sender),
        }
    }

    /// Queues a toast.
    pub(crate) fn show(&self, note: Note) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(note);
        }
    }
}

impl Default for Notifier {
    fn default() -> Self {
        Self::new()
    }
}

/// Shows one toast in the notification center of the system.
#[cfg(windows)]
fn show(note: &Note) {
    toast::show(note);
}

/// Shows one toast in the notification center of the system.
///
/// On Linux the daemon of the desktop draws it (GNOME Shell, KDE Plasma,
/// dunst). The app name is the one the daemon shows and the timeout is the
/// one of a message: it goes away alone.
#[cfg(target_os = "linux")]
fn show(note: &Note) {
    use notify_rust::{Notification, Timeout};

    let _ = Notification::new()
        .summary(&note.title)
        .body(&note.body)
        .appname("Whatfast")
        .timeout(Timeout::Milliseconds(6000))
        .show();
}

/// The platforms without a notification center in this build: macOS is not
/// packaged (the release skips it) and any other target keeps the app usable
/// without toasts.
#[cfg(not(any(windows, target_os = "linux")))]
fn show(_note: &Note) {}

/// The toast of Windows, with the AppUserModelID of Whatfast.
///
/// The system only shows the toast of an unpackaged app when the id is the one
/// of a shortcut of the Start menu: the first turn of the thread puts the id on
/// the shortcut of the installer, and the Show falls back to the id of the
/// PowerShell, which is always registered, when Whatfast runs from a build.
#[cfg(windows)]
mod toast {
    use std::path::PathBuf;

    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
    use windows::Win32::Foundation::PROPERTYKEY;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, IPersistFile,
        STGM_READWRITE, StructuredStorage::PROPVARIANT,
    };
    use windows::Win32::UI::Shell::{
        IShellLinkW, PropertiesSystem::IPropertyStore, SetCurrentProcessExplicitAppUserModelID,
        ShellLink,
    };
    use windows::core::{GUID, HSTRING, Interface as _, Result as WinResult};

    use super::Note;

    /// The AppUserModelID of the app: the one the installer writes on the
    /// shortcut of the Start menu.
    pub(super) const APP_ID: &str = "Owning01.Whatfast";

    /// The id of the PowerShell: a toast with it always has a shortcut to
    /// belong to, so it is the last resort.
    const POWERSHELL_APP_ID: &str =
        "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe";

    /// `PKEY_AppUserModel_ID`: the property of the shortcut that names the
    /// app for the notification center.
    const PKEY_APP_USER_MODEL_ID: PROPERTYKEY = PROPERTYKEY {
        fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
        pid: 5,
    };

    /// Prepares the thread: the apartment and the id of the app.
    pub(super) fn prepare() {
        // WinRT runs on the apartment of this thread; the interface has its
        // own and never waits for a toast.
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        // An unpackaged app must name itself before Windows accepts its
        // toasts. Writing the id on the shortcut is not enough: the process
        // that calls the notifier has to carry the same id.
        if let Err(error) =
            unsafe { SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(APP_ID)) }
        {
            eprintln!("notifications: the process id was not set: {error}");
        }
        if let Err(error) = register_app_id() {
            eprintln!("notifications: the id of the app was not registered: {error}");
        }
        if let Err(error) = enable_setting() {
            eprintln!("notifications: the setting of the app was not enabled: {error}");
        }
    }

    /// Processes the messages of this thread so the toast can paint.
    ///
    /// A WinRT toast needs a pump on the thread that shows it. With only a
    /// blocking wait on the queue, Windows files the notice in the center but
    /// never lifts it over the desk.
    pub(super) fn pump_messages() {
        use windows::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
        };
        unsafe {
            let mut message = MSG::default();
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&message);
                let _ = DispatchMessageW(&message);
            }
        }
    }

    /// Shows the toast, with the id of the app or with the one of the
    /// PowerShell when the app is not installed.
    pub(super) fn show(note: &Note) {
        // The message text is XML: it cannot carry its own tags into the
        // toast. The audio is silent because the sound of the message is the
        // one of the app.
        let xml = format!(
            "<toast duration=\"short\"><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual><audio silent=\"true\"/></toast>",
            escape(&note.title),
            escape(&note.body)
        );
        // The first notifier uses the id the process declared for itself;
        // the other two are the fallback of an installed app and of a build
        // that runs from the tree.
        if show_with(None, &xml).is_err()
            && show_with(Some(APP_ID), &xml).is_err()
            && let Err(error) = show_with(Some(POWERSHELL_APP_ID), &xml)
        {
            eprintln!("notifications: the toast was not shown: {error}");
        }
    }

    /// Shows one toast. Without an id, Windows uses the one of the process.
    fn show_with(app_id: Option<&str>, xml: &str) -> WinResult<()> {
        let document = XmlDocument::new()?;
        document.LoadXml(&HSTRING::from(xml))?;
        let toast = ToastNotification::CreateToastNotification(&document)?;
        let notifier = match app_id {
            Some(id) => ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(id))?,
            None => ToastNotificationManager::CreateToastNotifier()?,
        };
        notifier.Show(&toast)
    }

    /// The path of the shortcut of the Start menu that the installer wrote.
    fn shortcut_path() -> Option<PathBuf> {
        let appdata = std::env::var_os("APPDATA")?;
        Some(
            PathBuf::from(appdata)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs")
                .join("Whatfast.lnk"),
        )
    }

    /// Writes the id of the app on the shortcut of the Start menu.
    ///
    /// Without a shortcut there is nothing to write: a build from the tree
    /// keeps the fallback of the PowerShell. The shortcut of an installed app
    /// is the one the installer made, so the id is set on it instead of
    /// adding a second entry to the menu.
    pub(super) fn register_app_id() -> WinResult<()> {
        let Some(path) = shortcut_path() else {
            return Ok(());
        };
        if !path.is_file() {
            return Ok(());
        }
        let wide = HSTRING::from(path.as_os_str());
        unsafe {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            let file: IPersistFile = link.cast()?;
            file.Load(&wide, STGM_READWRITE)?;
            let store: IPropertyStore = link.cast()?;
            let value = PROPVARIANT::from(APP_ID);
            store.SetValue(&PKEY_APP_USER_MODEL_ID, &value)?;
            store.Commit()?;
            file.Save(&wide, true)?;
        }
        Ok(())
    }

    /// Writes `Enabled = 1` in the setting of the app.
    ///
    /// A fresh install creates the key without that value. Windows then treats
    /// the app as one the user never allowed, and the toast goes straight to
    /// the center without ever appearing on the desk.
    fn enable_setting() -> WinResult<()> {
        use windows::core::HRESULT;
        use windows_sys::Win32::Foundation::ERROR_SUCCESS;
        use windows_sys::Win32::System::Registry::{
            HKEY_CURRENT_USER, KEY_SET_VALUE, REG_DWORD, RegCloseKey, RegCreateKeyExW,
            RegSetValueExW,
        };

        let subkey: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Notifications\\Settings\\Owning01.Whatfast"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let value: Vec<u16> = "Enabled"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut key: windows_sys::Win32::System::Registry::HKEY = std::ptr::null_mut();
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                0,
                std::ptr::null(),
                0,
                KEY_SET_VALUE,
                std::ptr::null(),
                &mut key,
                std::ptr::null_mut(),
            )
        };
        if status != ERROR_SUCCESS {
            return Err(windows::core::Error::from_hresult(HRESULT(status as i32)));
        }
        let enabled: u32 = 1;
        let status = unsafe {
            RegSetValueExW(
                key,
                value.as_ptr(),
                0,
                REG_DWORD,
                enabled.to_ne_bytes().as_ptr(),
                std::mem::size_of::<u32>() as u32,
            )
        };
        unsafe {
            RegCloseKey(key);
        }
        if status != ERROR_SUCCESS {
            return Err(windows::core::Error::from_hresult(HRESULT(status as i32)));
        }
        Ok(())
    }

    /// The text of a message as the content of an XML element.
    fn escape(text: &str) -> String {
        let mut escaped = String::with_capacity(text.len());
        for character in text.chars() {
            match character {
                '&' => escaped.push_str("&amp;"),
                '<' => escaped.push_str("&lt;"),
                '>' => escaped.push_str("&gt;"),
                '"' => escaped.push_str("&quot;"),
                '\'' => escaped.push_str("&apos;"),
                other => escaped.push(other),
            }
        }
        escaped
    }

    #[cfg(test)]
    mod tests {
        use super::escape;

        #[test]
        fn the_text_of_a_message_cannot_break_the_toast() {
            assert_eq!(
                escape("Ana & Luis <b> \"hola\" 'chau'"),
                "Ana &amp; Luis &lt;b&gt; &quot;hola&quot; &apos;chau&apos;"
            );
            assert_eq!(escape("sin nada raro"), "sin nada raro");
        }

        /// Checks the registration of the id on the real shortcut of the
        /// Start menu. It is ignored because it writes to the profile of the
        /// machine: `cargo test -p whatfast-ui the_id_of_the_app -- --ignored`.
        #[cfg(windows)]
        #[test]
        #[ignore]
        fn the_id_of_the_app_is_registered_in_the_shortcut() {
            use windows::Win32::System::Com::{
                CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile, STGM_READWRITE,
            };
            use windows::Win32::UI::Shell::{
                IShellLinkW, PropertiesSystem::IPropertyStore, ShellLink,
            };
            use windows::core::{HSTRING, Interface as _};

            let _ = unsafe {
                windows::Win32::System::Com::CoInitializeEx(
                    None,
                    windows::Win32::System::Com::COINIT_MULTITHREADED,
                )
            };
            super::register_app_id().expect("the shortcut could not take the id");
            let path = super::shortcut_path().expect("the profile has a Start menu");
            assert!(path.is_file(), "the shortcut of the app is missing");
            let wide = HSTRING::from(path.as_os_str());
            unsafe {
                let link: IShellLinkW =
                    CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).unwrap();
                let file: IPersistFile = link.cast().unwrap();
                file.Load(&wide, STGM_READWRITE).unwrap();
                let store: IPropertyStore = link.cast().unwrap();
                let value = store.GetValue(&super::PKEY_APP_USER_MODEL_ID).unwrap();
                // The store gives the id as a string, whatever the flavour of
                // the variant: it is the text what matters.
                assert_eq!(value.to_string(), super::APP_ID);
            }
            // The system has the last word: this line tells whether the toasts
            // of the app would appear at all.
            // The system has the last word: this line tells whether the toasts
            // of the app would appear at all.
            let notifier =
                windows::UI::Notifications::ToastNotificationManager::CreateToastNotifierWithId(
                    &HSTRING::from(super::APP_ID),
                )
                .unwrap();
            eprintln!("toast setting: {:?}", notifier.Setting().unwrap());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::{Note, Notifier};

    #[test]
    fn a_closed_queue_does_not_break_the_interface() {
        // The thread of the toasts can die (the notification server went
        // away); sending a note then must be a no-op, not a panic.
        let (sender, receiver) = mpsc::channel();
        drop(receiver);
        let notifier = Notifier {
            sender: Some(sender),
        };
        notifier.show(Note {
            title: "Fast friend".into(),
            body: "Hola".into(),
        });
        assert!(
            notifier.sender.is_some(),
            "a closed queue must not disable the notifier"
        );
    }

    /// Shows a real toast on the desk. It is ignored because it needs the
    /// notification center of the machine:
    /// `cargo test -p whatfast-ui a_real_toast -- --ignored`.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn a_real_toast_appears() {
        let notifier = Notifier::new();
        notifier.show(Note {
            title: "Whatfast".into(),
            body: "Prueba de notificación nativa".into(),
        });
        // The thread shows it after the test gives it the turn.
        std::thread::sleep(std::time::Duration::from_millis(2000));
    }
}
