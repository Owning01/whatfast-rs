//! Port of `tauri/tests/unsupported.test.ts`.
//!
//! Every case of the oracle suite has its own test here, with the same name
//! translated to snake_case.

use whatfast_core::unsupported::{PollPresentation, parse_poll_fallback};

#[test]
fn poll_fallback_preserves_title_and_every_option() {
    assert_eq!(
        parse_poll_fallback("\u{1F4CA} Poll: Lunch\n\u{2022} Pizza\n\u{2022} Sushi"),
        PollPresentation {
            title: "Lunch".to_string(),
            options: vec!["Pizza".to_string(), "Sushi".to_string()],
            results: false,
        },
    );
}

#[test]
fn poll_result_snapshots_remain_distinguishable() {
    assert_eq!(
        parse_poll_fallback("\u{1F4CA} Poll results: Lunch"),
        PollPresentation {
            title: "Lunch".to_string(),
            options: Vec::new(),
            results: true,
        },
    );
}
