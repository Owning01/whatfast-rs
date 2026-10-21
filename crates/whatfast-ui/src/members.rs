//! The panel of the members of a group, and the roster of its participants.
//!
//! port: taken from the origin's `tauri/src/components/MemberPanel.tsx`,
//! `ParticipantList.tsx` and `ParticipantRow.tsx`. The lengths come from the
//! stylesheet of the origin (`tauri/src/styles.css`, the classes
//! `member-panel`, `member-panel-header`, `member-section-label` and
//! `participant-row`).
//!
//! The rows of the roster are not written here: the core already has the
//! port of `tauri/src/lib/participant-roster.ts`
//! (`whatfast_core::performance::participant_roster_rows`), and this module
//! only draws them.
//!
//! The data is not here either: the participants of a group arrive with the
//! directory of the mentions (`app.mention_directories`), which is filled by
//! `ensure_mention_directory()`.

#![allow(dead_code)]

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    Context, ElementId, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, div, list, px,
};
use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};

use whatfast_core::performance::{
    PARTICIPANT_HEADER_HEIGHT, PARTICIPANT_ROW_HEIGHT, ParticipantRosterRow,
    participant_roster_rows,
};
use whatfast_core::theme::default_theme;
use whatfast_protocol::ChatParticipant;

use crate::app::Whatfast;
use crate::helpers::avatar_element;
use crate::tokens::Metrics;

/// The width of the panel, in pixels, before the scale.
pub(crate) const PANEL_WIDTH: f32 = 240.0;
/// The smallest width of the panel, in pixels, before the scale.
pub(crate) const PANEL_MIN_WIDTH: f32 = 200.0;
/// The side of the avatar of a member, in pixels, before the scale.
pub(crate) const AVATAR_SIZE: f32 = 39.0;
/// How many avatars of the roster are asked for at once.
pub(crate) const AVATAR_WARMUP: usize = 24;

/// The state of the panel of the members.
pub(crate) struct MemberPanelState {
    /// Whether the panel is docked next to the conversation.
    pub(crate) open: bool,
    /// The chat whose roster was already asked for.
    pub(crate) requested: Option<String>,
    /// The scroll of the roster.
    list: gpui::ListState,
    /// How many rows the roster had the last time it was drawn.
    rows: usize,
    /// The rows of the roster, the group they belong to and the number of
    /// members they had when they were built.
    rows_cache: Option<(String, usize, Rc<Vec<ParticipantRosterRow>>)>,
    /// Whether the avatars of the roster were already asked for.
    avatars_warmed: bool,
}

impl Default for MemberPanelState {
    fn default() -> Self {
        Self {
            open: false,
            requested: None,
            list: gpui::ListState::new(0, gpui::ListAlignment::Top, px(0.)),
            rows: 0,
            rows_cache: None,
            avatars_warmed: false,
        }
    }
}

/// The number of members that the header of the panel shows.
///
/// port: `memberCountLabel` of the origin: the count of the chat info when
/// it is there, and the number of participants that arrived otherwise.
pub(crate) fn member_count(participant_count: u32, participants: usize) -> u32 {
    if participant_count > 0 {
        participant_count
    } else {
        u32::try_from(participants).unwrap_or(u32::MAX)
    }
}

/// The label of the header of the panel.
///
/// port: `memberCountLabel` of the origin: "12 members", or "Members" when
/// the count is not known yet.
pub(crate) fn member_count_label(count: u32) -> String {
    if count > 0 {
        format!("{count} members")
    } else {
        "Members".to_string()
    }
}

/// The name that stands for a member of the roster.
///
/// port: the origin writes `participant.displayName || participant.phoneNumber`.
pub(crate) fn participant_name(
    participant: &whatfast_core::performance::ChatParticipant,
) -> String {
    if !participant.display_name.trim().is_empty() {
        participant.display_name.clone()
    } else {
        participant.phone_number.clone()
    }
}

/// The role of a member of the roster.
///
/// port: the origin writes "Owner" for the super admin and "Admin" for the
/// other admins.
pub(crate) fn participant_role(
    participant: &whatfast_core::performance::ChatParticipant,
) -> Option<&'static str> {
    if participant.is_super_admin {
        Some("Owner")
    } else if participant.is_admin {
        Some("Admin")
    } else {
        None
    }
}

/// Converts the participants of the bridge into the ones of the core.
///
/// The core has no dependency on the protocol, so the roster of the core is
/// built with its own type.
pub(crate) fn core_participants(
    participants: &[ChatParticipant],
) -> Vec<whatfast_core::performance::ChatParticipant> {
    participants
        .iter()
        .map(|participant| whatfast_core::performance::ChatParticipant {
            participant_id: participant.participant_id.clone(),
            display_name: participant.display_name.clone(),
            phone_number: participant.phone_number.clone(),
            is_admin: participant.is_admin,
            is_super_admin: participant.is_super_admin,
            is_me: participant.is_me,
        })
        .collect()
}

/// The panel of the members of a group.
#[derive(Default)]
pub(crate) struct MemberPanel {
    /// The state of the panel.
    state: Rc<RefCell<MemberPanelState>>,
}

impl MemberPanel {
    /// Whether the panel is docked next to the conversation.
    pub(crate) fn is_open(&self) -> bool {
        self.state.borrow().open
    }

    /// Docks the panel next to the conversation, or undocks it.
    pub(crate) fn set_open(&self, open: bool) {
        self.state.borrow_mut().open = open;
    }

    /// Draws the panel of the members.
    ///
    /// The panel is empty when it is closed or when the open conversation is
    /// not a group: the origin hides it in both cases.
    pub(crate) fn render(&self, app: &Whatfast, cx: &mut Context<Whatfast>) -> gpui::Div {
        let scale = app.ui_scale;
        let metrics = Metrics::new(scale, default_theme().visual_style);
        let Some(chat) = app.store.selected_chat() else {
            return div();
        };
        if !self.is_open() || chat.kind() != whatfast_protocol::ChatKind::Group {
            return div();
        }
        let chat_id = chat.id.clone();
        self.ensure_roster(&chat_id, cx);

        let participants = app.mention_directories.get(&chat_id);
        let ready = participants.is_some();
        let participants = participants.map(Vec::as_slice).unwrap_or(&[]);
        let participant_count = app
            .chat_info
            .as_ref()
            .filter(|info| info.chat_id == chat_id)
            .and_then(|info| info.info.as_ref())
            .map(|info| info.participant_count)
            .unwrap_or(0);
        let count = member_count(participant_count, participants.len());
        if ready {
            self.warm_avatars(participants, cx);
        }

        let body: gpui::AnyElement = if ready {
            self.render_roster(app, &chat_id, participants, cx)
                .into_any_element()
        } else {
            v_flex()
                .flex_1()
                .min_h_0()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("Loading membersÔÇª"),
                )
                .into_any_element()
        };

        v_flex()
            .w(px(PANEL_WIDTH * scale))
            .min_w(px(PANEL_MIN_WIDTH * scale))
            .flex_none()
            .border_l_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar)
            .child(
                h_flex()
                    .flex_none()
                    .h(metrics.header_height)
                    .min_h(metrics.header_height)
                    .gap(px(10.))
                    .px(px(14.))
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        Icon::new(IconName::User)
                            .with_size(px(17.))
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(member_count_label(count)),
                    ),
            )
            .child(body)
    }

    /// Draws the roster of the members.
    fn render_roster(
        &self,
        app: &Whatfast,
        chat_id: &str,
        participants: &[ChatParticipant],
        cx: &mut Context<Whatfast>,
    ) -> gpui::List {
        let scale = app.ui_scale;
        let rows = self.roster_rows(chat_id, participants);
        {
            let mut state = self.state.borrow_mut();
            if state.rows != rows.len() {
                state.rows = rows.len();
                state.list.reset(rows.len());
            }
        }
        let list_state = self.state.borrow().list.clone();
        let weak = cx.entity().downgrade();
        let row_scale = scale;

        list(list_state, move |index, _window, cx| {
            let Some(row) = rows.get(index) else {
                return div().into_any_element();
            };
            match row {
                ParticipantRosterRow::Header { label, .. } => div()
                    .h(px(PARTICIPANT_HEADER_HEIGHT as f32 * row_scale))
                    .px(px(14.))
                    .pt(px(14. * row_scale))
                    .pb(px(4. * row_scale))
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(cx.theme().muted_foreground)
                    .child(label.clone())
                    .into_any_element(),
                ParticipantRosterRow::Participant { participant, .. } => {
                    render_participant(&weak, index, participant, row_scale, cx).into_any_element()
                }
            }
        })
    }

    /// The rows of the roster, built again only when the group changes.
    ///
    /// port: the origin keeps the rows of the roster in a memo, because a
    /// group can have thousands of members and the panel is drawn on every
    /// frame. The cache here is keyed by the group and by the number of
    /// members, the same signal the origin uses to know that the roster
    /// changed.
    fn roster_rows(
        &self,
        chat_id: &str,
        participants: &[ChatParticipant],
    ) -> Rc<Vec<ParticipantRosterRow>> {
        let mut state = self.state.borrow_mut();
        let cached = state
            .rows_cache
            .as_ref()
            .filter(|(cached_chat, cached_count, _)| {
                cached_chat == chat_id && *cached_count == participants.len()
            })
            .map(|(_, _, rows)| rows.clone());
        if let Some(rows) = cached {
            return rows;
        }
        let rows = Rc::new(participant_roster_rows(
            &core_participants(participants),
            true,
        ));
        state.rows_cache = Some((chat_id.to_string(), participants.len(), rows.clone()));
        rows
    }

    /// Asks for the roster of the group when it is not there yet.
    fn ensure_roster(&self, chat_id: &str, cx: &mut Context<Whatfast>) {
        {
            let mut state = self.state.borrow_mut();
            if state.requested.as_deref() == Some(chat_id) {
                return;
            }
            state.requested = Some(chat_id.to_string());
        }
        let chat_id = chat_id.to_string();
        cx.spawn(async move |weak, cx| {
            weak.update(cx, |app, cx| {
                app.ensure_mention_directory(chat_id.clone());
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Asks for the avatars of the first members of the roster.
    ///
    /// port: the origin asks for the avatar of every row that it mounts, and
    /// its queue keeps the number of downloads that are in flight small. The
    /// client of the native app has the same budget in its RPC layer, so the
    /// panel asks for the first members of the roster and lets the queue of
    /// the application do the rest.
    fn warm_avatars(&self, participants: &[ChatParticipant], cx: &mut Context<Whatfast>) {
        {
            let mut state = self.state.borrow_mut();
            if state.avatars_warmed {
                return;
            }
            state.avatars_warmed = true;
        }
        let pending: Vec<String> = participants
            .iter()
            .take(AVATAR_WARMUP)
            .map(|participant| participant.participant_id.clone())
            .collect();
        cx.spawn(async move |weak, cx| {
            weak.update(cx, |app, cx| {
                for participant_id in pending {
                    if app.store.participant_avatar_path(&participant_id).is_none() {
                        app.load_participant_avatar(participant_id);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

/// Draws one member of the roster.
///
/// port: `ParticipantRow` of the origin: the avatar, the name, the phone
/// number and the badge of the role. The row of the user cannot be pressed.
fn render_participant(
    weak: &WeakEntity<Whatfast>,
    index: usize,
    participant: &whatfast_core::performance::ChatParticipant,
    scale: f32,
    cx: &mut gpui::App,
) -> gpui::Stateful<gpui::Div> {
    let name = participant_name(participant);
    let avatar_path = weak
        .read_with(cx, |app, _| {
            app.store
                .participant_avatar_path(&participant.participant_id)
                .map(str::to_string)
        })
        .ok()
        .flatten()
        .unwrap_or_default();
    let hover = cx.theme().muted;
    let muted = cx.theme().muted_foreground;
    let accent = cx.theme().accent;
    let accent_soft = cx.theme().accent.opacity(0.12);
    let participant_id = participant.participant_id.clone();
    let weak = weak.clone();

    let row = h_flex()
        .id(ElementId::named_usize("participant-row", index))
        .w_full()
        .min_h(px(PARTICIPANT_ROW_HEIGHT as f32 * scale))
        .gap(px(10.))
        .px(px(14.))
        .py(px(7.))
        .items_center()
        .hover(move |style| style.bg(hover))
        .child(avatar_element(
            &name,
            &avatar_path,
            px(AVATAR_SIZE * scale),
            cx,
        ))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .truncate()
                        .child(if participant.is_me {
                            "You".to_string()
                        } else {
                            name.clone()
                        }),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .truncate()
                        .child(participant.phone_number.clone()),
                ),
        );

    let role = participant_role(participant);
    let row = match role {
        Some(role) => row.child(
            div()
                .px(px(6.))
                .py(px(3.))
                .rounded_sm()
                .bg(accent_soft)
                .text_color(accent)
                .text_xs()
                .child(role),
        ),
        None => row,
    };

    if participant.is_me {
        return row;
    }
    row.on_click(move |_, window, cx| {
        let participant_id = participant_id.clone();
        weak.update(cx, |app, cx| {
            let chat_id = app
                .store
                .chats
                .iter()
                .find(|chat| chat.transport_jid == participant_id)
                .map(|chat| chat.id.clone());
            match chat_id {
                Some(chat_id) => app.open_chat(chat_id, window, cx),
                None => {
                    app.store.toast_error =
                        Some("This member is not in your conversations yet".into());
                }
            }
        })
        .ok();
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn participant(
        id: &str,
        name: &str,
        phone: &str,
        admin: bool,
        super_admin: bool,
        me: bool,
    ) -> whatfast_core::performance::ChatParticipant {
        whatfast_core::performance::ChatParticipant {
            participant_id: id.to_string(),
            display_name: name.to_string(),
            phone_number: phone.to_string(),
            is_admin: admin,
            is_super_admin: super_admin,
            is_me: me,
        }
    }

    fn bridge_participant(
        id: &str,
        name: &str,
        phone: &str,
        admin: bool,
        super_admin: bool,
        me: bool,
    ) -> ChatParticipant {
        ChatParticipant {
            participant_id: id.to_string(),
            display_name: name.to_string(),
            phone_number: phone.to_string(),
            is_admin: admin,
            is_super_admin: super_admin,
            is_me: me,
        }
    }

    #[test]
    fn the_count_of_the_header_is_the_one_of_the_chat_info() {
        assert_eq!(member_count(12, 3), 12);
    }

    #[test]
    fn the_count_of_the_header_falls_back_to_the_roster() {
        assert_eq!(member_count(0, 3), 3);
    }

    #[test]
    fn the_header_says_how_many_members_there_are() {
        assert_eq!(member_count_label(12), "12 members");
        assert_eq!(member_count_label(1), "1 members");
    }

    #[test]
    fn the_header_says_members_when_the_count_is_not_known() {
        assert_eq!(member_count_label(0), "Members");
    }

    #[test]
    fn the_name_of_a_member_falls_back_to_the_phone_number() {
        let member = participant("1", "", "+5493811234567", false, false, false);
        assert_eq!(participant_name(&member), "+5493811234567");
        let member = participant("1", "Ana", "+5493811234567", false, false, false);
        assert_eq!(participant_name(&member), "Ana");
    }

    #[test]
    fn the_owner_of_the_group_has_the_badge_of_the_owner() {
        let member = participant("1", "Ana", "+549", true, true, false);
        assert_eq!(participant_role(&member), Some("Owner"));
    }

    #[test]
    fn an_admin_has_the_badge_of_the_admin() {
        let member = participant("1", "Ana", "+549", true, false, false);
        assert_eq!(participant_role(&member), Some("Admin"));
    }

    #[test]
    fn a_member_without_a_role_has_no_badge() {
        let member = participant("1", "Ana", "+549", false, false, false);
        assert_eq!(participant_role(&member), None);
    }

    #[test]
    fn the_participants_of_the_bridge_become_the_ones_of_the_core() {
        let members = vec![bridge_participant("1", "Ana", "+549", true, true, false)];
        let converted = core_participants(&members);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].participant_id, "1");
        assert_eq!(converted[0].display_name, "Ana");
        assert!(converted[0].is_admin);
        assert!(converted[0].is_super_admin);
        assert!(!converted[0].is_me);
    }

    #[test]
    fn the_roster_has_a_section_of_the_admins_and_one_of_the_members() {
        let members = vec![
            participant("1", "Ana", "+1", true, true, false),
            participant("2", "Beto", "+2", false, false, false),
        ];
        let rows = participant_roster_rows(&members, true);
        assert_eq!(rows.len(), 4);
        assert!(matches!(rows[0], ParticipantRosterRow::Header { .. }));
        assert!(matches!(rows[1], ParticipantRosterRow::Participant { .. }));
        assert!(matches!(rows[2], ParticipantRosterRow::Header { .. }));
        assert!(matches!(rows[3], ParticipantRosterRow::Participant { .. }));
    }
}
