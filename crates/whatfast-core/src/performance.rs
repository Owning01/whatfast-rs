//! The roster of a group: the row model and the section builder the UI mounts.
//!
//! port: `tauri/src/lib/participant-roster.ts` (SolidJS frontend, frozen
//! oracle). The rest of the original `performance.ts` helpers (the bounded set,
//! the bounded window and the reply index) were never wired into the GPUI app.

/// The height of a row of the roster, in pixels.
pub const PARTICIPANT_ROW_HEIGHT: u32 = 54;

/// The height of the header of a section of the roster, in pixels.
pub const PARTICIPANT_HEADER_HEIGHT: u32 = 30;

/// A member of the roster of a group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatParticipant {
    pub participant_id: String,
    pub display_name: String,
    pub phone_number: String,
    pub is_admin: bool,
    pub is_super_admin: bool,
    pub is_me: bool,
}

/// A row of the roster of a group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParticipantRosterRow {
    /// The header of a section of the roster.
    Header { key: String, label: String },
    /// A member of the roster.
    Participant {
        key: String,
        participant: ChatParticipant,
        position: u32,
    },
}

/// The rows of the roster of a group, with the sections when asked for.
pub fn participant_roster_rows(
    participants: &[ChatParticipant],
    sectioned: bool,
) -> Vec<ParticipantRosterRow> {
    if !sectioned {
        return participants
            .iter()
            .enumerate()
            .map(|(index, participant)| ParticipantRosterRow::Participant {
                key: participant.participant_id.clone(),
                participant: participant.clone(),
                position: index as u32 + 1,
            })
            .collect();
    }
    let admins: Vec<&ChatParticipant> = participants
        .iter()
        .filter(|participant| participant.is_admin || participant.is_super_admin)
        .collect();
    let members: Vec<&ChatParticipant> = participants
        .iter()
        .filter(|participant| !participant.is_admin && !participant.is_super_admin)
        .collect();
    let mut result: Vec<ParticipantRosterRow> = Vec::new();
    let mut position: u32 = 0;
    if !admins.is_empty() {
        result.push(ParticipantRosterRow::Header {
            key: "admins".to_string(),
            label: format!("ADMINS \u{2014} {}", admins.len()),
        });
        for participant in admins {
            position += 1;
            result.push(ParticipantRosterRow::Participant {
                key: participant.participant_id.clone(),
                participant: participant.clone(),
                position,
            });
        }
    }
    if !members.is_empty() {
        result.push(ParticipantRosterRow::Header {
            key: "members".to_string(),
            label: format!("MEMBERS \u{2014} {}", members.len()),
        });
        for participant in members {
            position += 1;
            result.push(ParticipantRosterRow::Participant {
                key: participant.participant_id.clone(),
                participant: participant.clone(),
                position,
            });
        }
    }
    result
}
