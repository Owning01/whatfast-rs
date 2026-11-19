//! The automations, as the screen sees them: list, create, pause, remove.
//!
//! A task lives in `agents.db` and the serve loop looks at the ones that came
//! due. This module is only the door the desktop uses to edit them: it checks
//! what the screen sent, works out the first turn, and answers the row.
//!
//! Only the user edits automations. An agent connected through the local
//! endpoint may read chats and send messages; it does not get to schedule what
//! the account does while nobody is in front of it.

use whatfast_auto::{Kind, Output, Schedule, Window};
use whatfast_protocol::{
    RpcResponse, Schedule as WireSchedule, ScheduleListRequest, ScheduleListResponse,
    SchedulePauseRequest, SchedulePauseResponse, SchedulePutRequest, SchedulePutResponse,
    ScheduleRemoveRequest, ScheduleRemoveResponse, rpc_response,
};

use crate::{
    db::Store,
    guard::{Denied, Guard, Who},
};

/// Lists every task of the profile, paused ones included.
pub fn list(
    guard: &Guard,
    store: &Store,
    who: &Who,
    _request: &ScheduleListRequest,
) -> Result<RpcResponse, Denied> {
    user_only(who)?;
    let rows = guard.db().schedules().map_err(internal)?;
    let schedules = rows.iter().map(|row| wire(store, row)).collect();
    Ok(answer(rpc_response::Result::ScheduleList(
        ScheduleListResponse { schedules },
    )))
}

/// Creates a task, or rewrites one the user already has.
///
/// The first turn is worked out here and not by the screen: the offset of the
/// zone belongs to the machine, and the screen is one more reader of the same
/// table.
pub fn put(
    guard: &Guard,
    store: &Store,
    who: &Who,
    request: &SchedulePutRequest,
    now_ms: i64,
) -> Result<RpcResponse, Denied> {
    user_only(who)?;
    let saved = build(guard, request, now_ms);
    let (schedule, error) = match saved {
        Ok(schedule) => {
            if schedule.id == 0 {
                let id = guard.db().add_schedule(&schedule).map_err(internal)?;
                (Schedule { id, ..schedule }, String::new())
            } else if guard.db().update_schedule(&schedule).map_err(internal)? {
                (schedule, String::new())
            } else {
                (schedule, format!("the task {} does not exist", request.id))
            }
        }
        Err(error) => (
            // A refused task still answers a row, so the screen can show the
            // values the user typed next to the reason.
            Schedule {
                id: request.id,
                agent_id: request.agent_id.clone(),
                chat_id: request.chat_id.clone(),
                kind: Kind::parse(request.kind.trim()).unwrap_or(Kind::Once),
                at_ms: Some(request.at_ms).filter(|at| *at > 0),
                cron: Some(request.cron.clone()).filter(|cron| !cron.is_empty()),
                prompt: request.prompt.clone(),
                output: Output::parse(request.output.trim()).unwrap_or(Output::Draft),
                window: Window::parse(request.window.trim()),
                next_at_ms: None,
                paused: false,
            },
            error.message,
        ),
    };
    Ok(answer(rpc_response::Result::SchedulePut(
        SchedulePutResponse {
            schedule: Some(wire(store, &schedule)),
            error,
        },
    )))
}

/// Pauses a task, or lets it run again with its next turn from now on.
pub fn pause(
    guard: &Guard,
    store: &Store,
    who: &Who,
    request: &SchedulePauseRequest,
    now_ms: i64,
) -> Result<RpcResponse, Denied> {
    user_only(who)?;
    let row = guard
        .db()
        .schedule(request.id)
        .map_err(internal)?
        .ok_or_else(|| not_found(request.id))?;
    guard
        .db()
        .pause_schedule(row.id, request.paused)
        .map_err(internal)?;
    // A task that comes back to life looks for a turn ahead: the old one is
    // behind us and firing it now would be a surprise.
    let mut row = row;
    row.paused = request.paused;
    if !request.paused {
        row.next_at_ms = turn(guard, &row, now_ms);
        guard
            .db()
            .set_schedule_next(row.id, row.next_at_ms)
            .map_err(internal)?;
    }
    Ok(answer(rpc_response::Result::SchedulePause(
        SchedulePauseResponse {
            schedule: Some(wire(store, &row)),
            error: String::new(),
        },
    )))
}

/// Removes a task for good.
pub fn remove(
    guard: &Guard,
    who: &Who,
    request: &ScheduleRemoveRequest,
) -> Result<RpcResponse, Denied> {
    user_only(who)?;
    let existed = guard.db().schedule(request.id).map_err(internal)?.is_some();
    if existed {
        guard.db().remove_schedule(request.id).map_err(internal)?;
    }
    Ok(answer(rpc_response::Result::ScheduleRemove(
        ScheduleRemoveResponse { removed: existed },
    )))
}

/// The task the screen asked for, with its first turn worked out.
fn build(guard: &Guard, request: &SchedulePutRequest, now_ms: i64) -> Result<Schedule, Denied> {
    let kind = Kind::parse(request.kind.trim()).ok_or_else(|| Denied {
        code: "invalid_argument",
        message: "a task is either `once` or `cron`".into(),
    })?;
    let output = Output::parse(request.output.trim()).ok_or_else(|| Denied {
        code: "invalid_argument",
        message: "a task answers with `read`, `draft` or `send`".into(),
    })?;
    let window = match request.window.trim() {
        "" => None,
        text => Some(Window::parse(text).ok_or_else(|| Denied {
            code: "invalid_argument",
            message: format!("`{text}` is not a window; it looks like 08:00-23:00"),
        })?),
    };
    let agent_id = request.agent_id.trim();
    if agent_id.is_empty() {
        return Err(Denied {
            code: "invalid_argument",
            message: "a task needs an agent".into(),
        });
    }
    if agent_id != "__direct__" {
        let spec = whatfast_agents::by_id(agent_id).ok_or_else(|| Denied {
            code: "invalid_argument",
            message: format!("`{agent_id}` is not a harness this build knows"),
        })?;
        if spec.run.is_none() {
            return Err(Denied {
                code: "failed_precondition",
                message: format!(
                    "{} has no headless mode, so it cannot run a task",
                    spec.name
                ),
            });
        }
    }
    let chat_id = request.chat_id.trim();
    if chat_id.is_empty() {
        return Err(Denied {
            code: "invalid_argument",
            message: "a task needs a chat".into(),
        });
    }
    let prompt = request.prompt.trim();
    if prompt.is_empty() {
        return Err(Denied {
            code: "invalid_argument",
            message: "a task needs a prompt".into(),
        });
    }
    // Editing a paused task keeps it paused: pausing is a decision of the
    // user, not a field of the form.
    let paused = if request.id == 0 {
        false
    } else {
        guard
            .db()
            .schedule(request.id)
            .ok()
            .flatten()
            .is_some_and(|row| row.paused)
    };
    let mut schedule = Schedule {
        id: request.id,
        agent_id: agent_id.to_owned(),
        chat_id: chat_id.to_owned(),
        kind,
        at_ms: None,
        cron: None,
        prompt: prompt.to_owned(),
        output,
        window,
        next_at_ms: None,
        paused,
    };
    match kind {
        Kind::Once => {
            let at_ms = request.at_ms;
            if at_ms <= now_ms {
                return Err(Denied {
                    code: "invalid_argument",
                    message: "a task that runs once needs an instant ahead".into(),
                });
            }
            schedule.at_ms = Some(at_ms);
        }
        Kind::Cron => {
            let expression = request.cron.trim();
            if whatfast_auto::Cron::parse(expression).is_none() {
                return Err(Denied {
                    code: "invalid_argument",
                    message: format!("`{expression}` is not a cron of five fields"),
                });
            }
            schedule.cron = Some(expression.to_owned());
        }
    }
    schedule.next_at_ms = turn(guard, &schedule, now_ms);
    if schedule.next_at_ms.is_none() {
        return Err(Denied {
            code: "invalid_argument",
            message: "the expression has no turn ahead".into(),
        });
    }
    Ok(schedule)
}

/// The instant a task fires next, inside its window.
///
/// The window is the will of the user about the hours the account speaks: it
/// holds every task, the one that runs once included. A task that comes due at
/// five in the morning waits for the window to open.
fn turn(guard: &Guard, schedule: &Schedule, now_ms: i64) -> Option<i64> {
    let offset = whatfast_auto::offset_now();
    match schedule.kind {
        Kind::Once => schedule
            .at_ms
            .filter(|at| *at > now_ms)
            .map(|at| whatfast_auto::defer_to_window(at, schedule.window, offset)),
        Kind::Cron => {
            let cron = schedule.cron.as_deref()?;
            guard.next_turn(cron, now_ms, schedule.window)
        }
    }
}

/// The row as the screen reads it.
fn wire(store: &Store, schedule: &Schedule) -> WireSchedule {
    let chat_title = store
        .chat(&schedule.chat_id)
        .ok()
        .flatten()
        .map(|row| row.name)
        .unwrap_or_default();
    WireSchedule {
        id: schedule.id,
        agent_id: schedule.agent_id.clone(),
        chat_id: schedule.chat_id.clone(),
        chat_title,
        kind: schedule.kind.as_str().to_owned(),
        at_ms: schedule.at_ms.unwrap_or_default(),
        cron: schedule.cron.clone().unwrap_or_default(),
        prompt: schedule.prompt.clone(),
        output: schedule.output.as_str().to_owned(),
        window: schedule.window.map(window_text).unwrap_or_default(),
        next_at_ms: schedule.next_at_ms.unwrap_or_default(),
        paused: schedule.paused,
    }
}

/// The text of a window, the same one `Window::parse` reads.
fn window_text(window: Window) -> String {
    window.to_string()
}

/// Automations are the user's, not the agents'.
fn user_only(who: &Who) -> Result<(), Denied> {
    match who {
        Who::User => Ok(()),
        Who::Agent { id, .. } => Err(Denied {
            code: "forbidden",
            message: format!("{id} cannot edit the automations of the profile"),
        }),
        Who::Denied(reason) => Err(Denied {
            code: "unauthorized",
            message: reason.clone(),
        }),
    }
}

/// The task a request names does not exist.
fn not_found(id: i64) -> Denied {
    Denied {
        code: "not_found",
        message: format!("the task {id} does not exist"),
    }
}

/// The failure of the table, as the answer of a request.
fn internal(error: rusqlite::Error) -> Denied {
    Denied {
        code: "internal",
        message: format!("agents.db: {error}"),
    }
}

/// The answer of one of the four requests.
fn answer(result: rpc_response::Result) -> RpcResponse {
    RpcResponse {
        result: Some(result),
    }
}

