//! The toasts of the system for the messages that arrive in the background.
//!
//! port: the origin asked the webview for a `Notification`
//! (`tauri/src/lib/notification-policy.ts` decides *when*; the browser drew
//! it). The native app has no browser: the drawing is a call to the
//! notification center of the operating system, and the decision stays in
//! `whatfast_core::notification_policy`.
//!
//! The call can block while the system shows the toast, so it runs in its own
//! thread and the interface only leaves a note in the queue. A failure to
//! show a toast is silent: the message is already in the chat, and a broken
//! notification server is not a reason to interrupt the user.

use std::sync::mpsc::{self, Sender};

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
                for note in receiver {
                    show(&note);
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
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn show(note: &Note) {
    use notify_rust::{Notification, Timeout};

    let _ = Notification::new()
        .summary(&note.title)
        .body(&note.body)
        .appname("Whatfast")
        // The toast goes away on its own: a message notification that stays
        // until the user dismisses it is noise.
        .timeout(Timeout::Milliseconds(6000))
        .show();
}

/// The platforms without a notification center in this build: macOS is not
/// packaged (the release skips it) and any other target keeps the app usable
/// without toasts.
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn show(_note: &Note) {}

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
    }

    #[test]
    fn a_note_carries_the_title_and_the_body() {
        let note = Note {
            title: "Fast friend".into(),
            body: "Hola".into(),
        };
        assert_eq!(note.title, "Fast friend");
        assert_eq!(note.body, "Hola");
    }
}
