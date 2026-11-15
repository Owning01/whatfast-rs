//! Rust port of the roster cases of `tauri/tests/performance.test.ts` (frozen oracle).
//!
//! Only the roster helpers the GPUI app mounts are kept in
//! `whatfast_core::performance`. The bounded set, the bounded window and the
//! reply index of the original module were never wired into the app.

use whatfast_core::performance::{ChatParticipant, ParticipantRosterRow, participant_roster_rows};

fn participant(index: u32) -> ChatParticipant {
    ChatParticipant {
        participant_id: format!("member-{index}"),
        display_name: format!("Member {index}"),
        phone_number: index.to_string(),
        is_admin: index < 5,
        is_super_admin: index == 0,
        is_me: false,
    }
}

#[test]
fn the_1_000_member_roster_separates_admins_from_members() {
    let participants: Vec<ChatParticipant> = (0..1_000).map(participant).collect();
    let rows = participant_roster_rows(&participants, true);

    // 5 admins and 995 members, each section with its header.
    assert_eq!(rows.len(), 1_002);
    assert_eq!(
        rows[0],
        ParticipantRosterRow::Header {
            key: "admins".to_string(),
            label: "ADMINS \u{2014} 5".to_string(),
        }
    );
    for (offset, index) in (0..5).enumerate() {
        assert_eq!(
            rows[offset + 1],
            ParticipantRosterRow::Participant {
                key: format!("member-{index}"),
                participant: participant(index),
                position: offset as u32 + 1,
            }
        );
    }
    assert_eq!(
        rows[6],
        ParticipantRosterRow::Header {
            key: "members".to_string(),
            label: "MEMBERS \u{2014} 995".to_string(),
        }
    );
    for (offset, index) in (5..1_000).enumerate() {
        assert_eq!(
            rows[offset + 7],
            ParticipantRosterRow::Participant {
                key: format!("member-{index}"),
                participant: participant(index),
                position: offset as u32 + 6,
            }
        );
    }
}

#[test]
fn the_roster_without_sections_has_no_headers_and_starts_at_position_one() {
    let participants: Vec<ChatParticipant> = (0..1_000).map(participant).collect();
    let rows = participant_roster_rows(&participants, false);

    assert_eq!(rows.len(), 1_000);
    assert!(rows
        .iter()
        .all(|row| matches!(row, ParticipantRosterRow::Participant { .. })));
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(
            *row,
            ParticipantRosterRow::Participant {
                key: format!("member-{index}"),
                participant: participant(index as u32),
                position: index as u32 + 1,
            }
        );
    }
}
