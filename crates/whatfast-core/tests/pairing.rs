//! Port of `tauri/tests/pairing.test.ts`.
//!
//! Every case of the oracle suite has its own test here, with the same name
//! translated to snake_case.

use whatfast_core::pairing::{
    PairingAuthSnapshot, PairingScreen, QrPhase, QrPresentation, pairing_startup_decision,
    qr_presentation,
};

#[test]
fn only_an_unpaired_startup_begins_pairing() {
    assert_eq!(
        pairing_startup_decision(&PairingAuthSnapshot {
            paired: false,
            logged_in: false,
        }),
        whatfast_core::pairing::PairingStartupDecision {
            screen: PairingScreen::Pairing,
            start_pairing: true,
        },
    );
    assert_eq!(
        pairing_startup_decision(&PairingAuthSnapshot {
            paired: true,
            logged_in: false,
        }),
        whatfast_core::pairing::PairingStartupDecision {
            screen: PairingScreen::Chats,
            start_pairing: false,
        },
    );
    assert_eq!(
        pairing_startup_decision(&PairingAuthSnapshot {
            paired: true,
            logged_in: true,
        }),
        whatfast_core::pairing::PairingStartupDecision {
            screen: PairingScreen::Chats,
            start_pairing: false,
        },
    );
}

#[test]
fn qr_lifecycle_distinguishes_waiting_active_and_expired_codes() {
    assert_eq!(
        qr_presentation("", 20_000, 10_000),
        QrPresentation {
            phase: QrPhase::Waiting,
            seconds_remaining: 0,
        },
    );
    assert_eq!(
        qr_presentation("code", 12_001, 10_000),
        QrPresentation {
            phase: QrPhase::Active,
            seconds_remaining: 3,
        },
    );
    assert_eq!(
        qr_presentation("code", 10_000, 10_000),
        QrPresentation {
            phase: QrPhase::Expired,
            seconds_remaining: 0,
        },
    );
}

#[test]
fn qr_codes_without_a_backend_expiry_remain_usable() {
    assert_eq!(
        qr_presentation("code", 0, 10_000),
        QrPresentation {
            phase: QrPhase::Active,
            seconds_remaining: 0,
        },
    );
}
