//! Port of `tauri/src/lib/polls.ts`.
//!
//! The original is the optimistic reducer of the poll votes.
//!
//! port: the message model the original imports from `tauri/src/lib/types.ts`
//! lives in `crate::types`, the single home of the shared DTOs of the app.
//! The reducer reads the poll of a message, so it works with the projection
//! [`MessageSummary`], the same one the pure formatters use.

use std::collections::HashSet;

use crate::types::{MessageContent, MessageSummary, PollContent, PollOption, PollVoter};

/// Snapshot a poll without structuredClone. Solid store values are proxies,
/// which the browser structured-clone algorithm rejects with DataCloneError.
pub fn clone_poll_content(poll: &PollContent) -> PollContent {
    PollContent {
        question: poll.question.clone(),
        options: poll
            .options
            .iter()
            .map(|option| PollOption {
                name: option.name.clone(),
                vote_count: option.vote_count,
                selected_by_me: option.selected_by_me,
                voters: option
                    .voters
                    .iter()
                    .map(|voter| PollVoter {
                        user_id: voter.user_id.clone(),
                        display_name: voter.display_name.clone(),
                        avatar_path: voter.avatar_path.clone(),
                        from_me: voter.from_me,
                    })
                    .collect(),
            })
            .collect(),
        selectable_options_count: poll.selectable_options_count,
        total_voters: poll.total_voters,
    }
}

/// Apply one complete local vote intent. The caller owns request sequencing;
/// this pure reducer makes optimistic updates and rollback deterministic.
pub fn optimistic_poll_vote(poll: &PollContent, selected_options: &[String]) -> PollContent {
    let previous: HashSet<&str> = poll
        .options
        .iter()
        .filter(|option| option.selected_by_me)
        .map(|option| option.name.as_str())
        .collect();
    let next: HashSet<&str> = selected_options.iter().map(String::as_str).collect();

    let total_voters = if previous.is_empty() && !next.is_empty() {
        poll.total_voters.saturating_add(1)
    } else if !previous.is_empty() && next.is_empty() {
        poll.total_voters.saturating_sub(1)
    } else {
        poll.total_voters
    };

    PollContent {
        question: poll.question.clone(),
        options: poll
            .options
            .iter()
            .map(|option| {
                let name = option.name.as_str();
                let was_selected = previous.contains(name);
                let is_selected = next.contains(name);
                let vote_count = if was_selected && !is_selected {
                    option.vote_count.saturating_sub(1)
                } else if !was_selected && is_selected {
                    option.vote_count.saturating_add(1)
                } else {
                    option.vote_count
                };
                PollOption {
                    name: option.name.clone(),
                    vote_count,
                    selected_by_me: is_selected,
                    voters: option.voters.clone(),
                }
            })
            .collect(),
        selectable_options_count: poll.selectable_options_count,
        total_voters,
    }
}

/// Ignore an unsolicited/stale poll snapshot while a newer local full-state
/// vote is pending. Non-poll metadata still converges immediately.
pub fn preserve_pending_poll_intent(
    current: &MessageSummary,
    incoming: &MessageSummary,
    pending: bool,
) -> MessageSummary {
    // port: `!current.content` in JS is "the content is null" and
    // `"poll" in current.content` is "the content is the poll variant".
    if !pending
        || !matches!(current.content, Some(MessageContent::Poll(_)))
        || !matches!(incoming.content, Some(MessageContent::Poll(_)))
    {
        return incoming.clone();
    }
    let mut merged = incoming.clone();
    merged.content = current.content.clone();
    merged
}
