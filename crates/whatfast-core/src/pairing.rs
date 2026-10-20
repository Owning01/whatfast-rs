//! Port of `tauri/src/state/pairing.ts`.
//!
//! The original decides the startup screen and turns the backend QR payload
//! into the phase the UI shows.

/// The screen the app opens on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairingScreen {
    Pairing,
    Chats,
}

impl PairingScreen {
    /// The string the original returns in the `screen` field.
    pub fn as_str(self) -> &'static str {
        match self {
            PairingScreen::Pairing => "pairing",
            PairingScreen::Chats => "chats",
        }
    }
}

/// Whether the startup flow has to drive the pairing screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PairingStartupDecision {
    pub screen: PairingScreen,
    pub start_pairing: bool,
}

/// The auth state the startup decision reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PairingAuthSnapshot {
    pub paired: bool,
    pub logged_in: bool,
}

/// The life cycle phase of the pairing QR code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QrPhase {
    Waiting,
    Active,
    Expired,
}

impl QrPhase {
    /// The string the original returns in the `phase` field.
    pub fn as_str(self) -> &'static str {
        match self {
            QrPhase::Waiting => "waiting",
            QrPhase::Active => "active",
            QrPhase::Expired => "expired",
        }
    }
}

/// What the QR panel shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QrPresentation {
    pub phase: QrPhase,
    pub seconds_remaining: u32,
}

/// Decides the screen the app starts on.
///
/// port: the original only looks at `auth.paired`; `loggedIn` is carried over
/// untouched.
pub fn pairing_startup_decision(auth: &PairingAuthSnapshot) -> PairingStartupDecision {
    if auth.paired {
        return PairingStartupDecision {
            screen: PairingScreen::Chats,
            start_pairing: false,
        };
    }
    PairingStartupDecision {
        screen: PairingScreen::Pairing,
        start_pairing: true,
    }
}

/// Turns the backend QR payload into the phase and the countdown the UI shows.
///
/// port: the original reads the clock itself (`Date.now()` is not used here,
/// `nowMs` is a parameter already) and the caller injects the instant, so the
/// port stays testable without a real clock.
pub fn qr_presentation(code: &str, expires_at_ms: i64, now_ms: i64) -> QrPresentation {
    // port: `!code` in JS is "the string is empty"; a non empty code is always
    // truthy.
    if code.is_empty() {
        return QrPresentation {
            phase: QrPhase::Waiting,
            seconds_remaining: 0,
        };
    }
    if expires_at_ms > 0 && now_ms >= expires_at_ms {
        return QrPresentation {
            phase: QrPhase::Expired,
            seconds_remaining: 0,
        };
    }
    let seconds_remaining = if expires_at_ms > 0 {
        let remaining = (expires_at_ms - now_ms) as f64 / 1_000.0;
        let ceiling = remaining.ceil();
        // port: `Math.max(1, Math.ceil(...))`, the countdown never shows zero
        // while the code is alive.
        if ceiling < 1.0 { 1 } else { ceiling as u32 }
    } else {
        0
    };
    QrPresentation {
        phase: QrPhase::Active,
        seconds_remaining,
    }
}
