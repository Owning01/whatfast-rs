//! The drafts of the agents and the decision of the user about each one.
//!
//! A draft is a message an agent wrote and the user has not sent: it waits in
//! `agents.db` with the why and the confidence of the agent. The tray of the
//! desktop lists them; approving one is what moves it towards the wire, and
//! discarding one throws it away. The three requests are answered here and not
//! in the `Server`: the table lives next to the session, but it is not the
//! session.

use whatfast_auto::{Draft, DraftState, Scope};
use whatfast_protocol::{
    Draft as WireDraft, DraftDecideRequest, DraftDecideResponse, DraftFinishRequest,
    DraftFinishResponse, DraftListRequest, DraftListResponse, DraftProposeRequest,
    DraftProposeResponse, RpcResponse, rpc_response,
};

use crate::{
    db::Store,
    guard::{Denied, Guard, Who},
};

/// The states a list returns when it does not name one: everything the user
/// may still act on.
const OPEN_STATES: [DraftState; 3] = [
    DraftState::Proposed,
    DraftState::Approved,
    DraftState::Failed,
];

/// Writes one draft and answers its row.
pub fn propose(
    guard: &Guard,
    store: &Store,
    who: &Who,
    request: &DraftProposeRequest,
    now_ms: i64,
) -> Result<RpcResponse, Denied> {
    let agent_id = agent_of(who, &request.agent_id)?;
    let is_group = crate::guard::is_group(&request.chat_id, store);
    let grant = grant_of(who);
    let allowed = match grant {
        Some(grant) => grant.allows(&request.chat_id, is_group, Scope::Draft),
        // The user writes drafts of their own with the full rights of the
        // profile; a local client without an agent keeps them too.
        None => true,
    };
    if !allowed {
        return Err(Denied {
            code: "forbidden",
            message: format!(
                "the grant of this agent does not allow drafts in {}",
                request.chat_id
            ),
        });
    }
    let text = request.text.trim();
    if text.is_empty() {
        return Err(Denied {
            code: "invalid_argument",
            message: "a draft needs a text".into(),
        });
    }
    let draft = Draft {
        id: 0,
        agent_id,
        chat_id: request.chat_id.clone(),
        reply_to: Some(request.reply_to_message_id.clone()).filter(|id| !id.is_empty()),
        text: text.to_owned(),
        reason: Some(request.reason.clone()).filter(|reason| !reason.is_empty()),
        confidence: Some(request.confidence.clamp(0.0, 1.0)),
        state: DraftState::Proposed,
        at_ms: now_ms,
    };
    let id = guard.db().add_draft(&draft).map_err(internal)?;
    Ok(answer(rpc_response::Result::DraftPropose(
        DraftProposeResponse {
            draft_id: id,
            state: DraftState::Proposed.as_str().to_owned(),
        },
    )))
}

/// Lists the drafts the caller may see.
pub fn list(
    guard: &Guard,
    store: &Store,
    who: &Who,
    request: &DraftListRequest,
) -> Result<RpcResponse, Denied> {
    let limit = request.limit.clamp(1, 200);
    let states: Vec<DraftState> = match DraftState::parse(request.state.trim()) {
        Some(state) => vec![state],
        None => OPEN_STATES.to_vec(),
    };
    let me = agent_of_optional(who);
    let mut drafts = Vec::new();
    for state in states {
        let rows = guard.db().drafts(state, limit).map_err(internal)?;
        for row in rows {
            if let Some(agent) = &me
                && &row.agent_id != agent
            {
                continue;
            }
            drafts.push(wire_draft(store, &row));
        }
    }
    drafts.sort_by_key(|draft| std::cmp::Reverse(draft.id));
    drafts.truncate(limit as usize);
    Ok(answer(rpc_response::Result::DraftList(DraftListResponse {
        drafts,
    })))
}

/// Applies the decision of the user: approve it for the next turn, or throw it
/// away. Sending happens where the session is, not here.
pub fn decide(
    guard: &Guard,
    store: &Store,
    who: &Who,
    request: &DraftDecideRequest,
) -> Result<RpcResponse, Denied> {
    let draft = guard
        .db()
        .draft(request.draft_id)
        .map_err(internal)?
        .ok_or_else(|| Denied {
            code: "not_found",
            message: format!("the draft {} does not exist", request.draft_id),
        })?;
    if let Some(me) = agent_of_optional(who)
        && draft.agent_id != me
    {
        return Err(Denied {
            code: "forbidden",
            message: "this draft belongs to another agent".into(),
        });
    }
    let next = if request.send {
        DraftState::Approved
    } else {
        DraftState::Discarded
    };
    guard
        .db()
        .set_draft_state(draft.id, next)
        .map_err(internal)?;
    let mut draft = draft;
    draft.state = next;
    Ok(answer(rpc_response::Result::DraftDecide(
        DraftDecideResponse {
            draft: Some(wire_draft(store, &draft)),
            error: String::new(),
        },
    )))
}

/// The answer of the desktop about an approved draft: it went out, or it did
/// not and says why.
///
/// Sending needs the session of WhatsApp, and the session lives in the window.
/// The backend keeps the state of the draft; the window is the one that puts
/// the text on the wire.
pub fn finish(
    guard: &Guard,
    store: &Store,
    who: &Who,
    request: &DraftFinishRequest,
) -> Result<RpcResponse, Denied> {
    let draft = guard
        .db()
        .draft(request.draft_id)
        .map_err(internal)?
        .ok_or_else(|| Denied {
            code: "not_found",
            message: format!("the draft {} does not exist", request.draft_id),
        })?;
    if let Some(me) = agent_of_optional(who)
        && draft.agent_id != me
    {
        return Err(Denied {
            code: "forbidden",
            message: "this draft belongs to another agent".into(),
        });
    }
    if draft.state != DraftState::Approved && draft.state != DraftState::Sending {
        return Err(Denied {
            code: "failed_precondition",
            message: format!(
                "the draft {} is {}, not approved",
                draft.id,
                draft.state.as_str()
            ),
        });
    }
    let next = if request.sent {
        DraftState::Sent
    } else {
        DraftState::Failed
    };
    guard
        .db()
        .set_draft_state(draft.id, next)
        .map_err(internal)?;
    let mut draft = draft;
    draft.state = next;
    // The reason of a failure is the last thing the user read about it: it
    // replaces the why of the agent instead of piling up.
    if !request.sent && !request.error.trim().is_empty() {
        draft.reason = Some(one_line(&request.error));
    }
    Ok(answer(rpc_response::Result::DraftFinish(
        DraftFinishResponse {
            draft: Some(wire_draft(store, &draft)),
        },
    )))
}

/// The first line of a message, cut to a size a row can show.
fn one_line(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    line.chars().take(200).collect()
}

/// The agent of the connection, or the one the request names when the caller is
/// the user. An agent may only write drafts of its own.
fn agent_of(who: &Who, named: &str) -> Result<String, Denied> {
    match who {
        Who::Agent { id, .. } => {
            if named.is_empty() || named == id {
                Ok(id.clone())
            } else {
                Err(Denied {
                    code: "forbidden",
                    message: format!("{id} cannot write drafts of {named}"),
                })
            }
        }
        Who::User => {
            if named.trim().is_empty() {
                Ok("usuario".to_owned())
            } else {
                Ok(named.trim().to_owned())
            }
        }
        Who::Denied(reason) => Err(Denied {
            code: "unauthorized",
            message: reason.clone(),
        }),
    }
}

/// The agent of the connection, when there is one.
fn agent_of_optional(who: &Who) -> Option<String> {
    match who {
        Who::Agent { id, .. } => Some(id.clone()),
        _ => None,
    }
}

/// The grant behind the connection, when an agent is the caller.
fn grant_of(who: &Who) -> Option<whatfast_auto::Grant> {
    match who {
        Who::Agent { grant, .. } => Some(grant.clone()),
        _ => None,
    }
}

/// One row of the table as the tray draws it.
fn wire_draft(store: &Store, draft: &Draft) -> WireDraft {
    let chat_title = store
        .chat(&draft.chat_id)
        .ok()
        .flatten()
        .map(|row| row.name)
        .unwrap_or_default();
    WireDraft {
        id: draft.id,
        agent_id: draft.agent_id.clone(),
        chat_id: draft.chat_id.clone(),
        chat_title,
        reply_to_message_id: draft.reply_to.clone().unwrap_or_default(),
        text: draft.text.clone(),
        reason: draft.reason.clone().unwrap_or_default(),
        confidence: draft.confidence.unwrap_or_default(),
        state: draft.state.as_str().to_owned(),
        at_ms: draft.at_ms,
    }
}

/// The answer of one of the three requests.
fn answer(result: rpc_response::Result) -> RpcResponse {
    RpcResponse {
        result: Some(result),
    }
}

/// A failure of the database, as the agent reads it.
fn internal(error: rusqlite::Error) -> Denied {
    Denied {
        code: "internal",
        message: format!("agents.db: {error}"),
    }
}
