//! The automations: what comes due, runs, and delivers its answer.
//!
//! The table of `schedules` keeps the tasks, the cron keeps their turns and the
//! window keeps them inside the hours of the user. This module is the part that
//! actually moves: it looks for the tasks that came due, writes their next
//! turn, and asks the harness of the task for an answer without a window in
//! front of it.
//!
//! What happens with the answer is the `output` of the task. `read` only
//! consults and leaves nothing; `draft` leaves the text for the user to
//! approve; `send` hands it to the WhatsApp session. A send that cannot go
//! out -the desk is offline, or WhatsApp refuses it- leaves the text as a
//! draft instead, so the answer of the harness is never lost.

use std::sync::Arc;

use async_channel::Sender;
use whatfast_auto::{
    Draft, DraftState, Grant, Output, Schedule,
    runner::{self, RunError},
};
use whatfast_protocol::{
    BackendEvent, ConnectionState, DraftProposeRequest, ListMessagesRequest, SendTextRequest,
    rpc_request::Request as RpcRequest,
};

use crate::{
    db::Store,
    guard::{Guard, Who},
    session::Session,
};

/// How long a claimed turn stays out of `due` while its harness runs. A
/// backend that dies mid-run leaves the lease; the next one picks the task up
/// again after it expires.
const RUN_LEASE_MS: i64 = 15 * 60 * 1000;

/// Turns a schedule is carried out with: `once` has no next turn.
fn next_turn(guard: &Guard, task: &Schedule, now_ms: i64) -> Option<i64> {
    match task.kind {
        whatfast_auto::Kind::Once => None,
        whatfast_auto::Kind::Cron => {
            let cron = task.cron.as_deref()?;
            // The turn of a task is read from the instant it came due, not from
            // the moment the loop happened to look at it, so a backend that
            // started late does not skip an hour.
            guard.next_turn(
                cron,
                now_ms.max(task.next_at_ms.unwrap_or(now_ms)),
                task.window,
            )
        }
    }
}

/// What the answer of a task does once the harness wrote it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Delivery {
    /// Nothing: the task only read the chat.
    Nothing,
    /// A draft the user reads and approves.
    Draft,
    /// The message goes out now.
    Send,
}

/// The delivery of one answer, from what the task asks and whether WhatsApp is
/// up. A send without a connection becomes a draft: the text waits for the
/// user instead of dying in a failed attempt.
fn delivery(output: Output, online: bool) -> Delivery {
    match output {
        Output::Read => Delivery::Nothing,
        Output::Draft => Delivery::Draft,
        Output::Send if online => Delivery::Send,
        Output::Send => Delivery::Draft,
    }
}

/// Runs every task that came due, without holding the serve loop.
///
/// It returns how many it started. Each one runs on its own task: a harness is
/// another program with its own network, and the backend has a desktop to
/// answer meanwhile.
pub fn dispatch(
    guard: &Arc<Guard>,
    store: &Arc<Store>,
    session: Option<Arc<Session>>,
    events: &Sender<BackendEvent>,
    now_ms: i64,
) -> usize {
    let due = match guard.db().due(now_ms) {
        Ok(due) => due,
        Err(error) => {
            tracing::warn!("the schedules were not read: {error}");
            return 0;
        }
    };
    let mut started = 0;
    for task in due {
        let previous_next = task.next_at_ms.unwrap_or(now_ms);
        // The lease is written only if the row still has the turn that `due`
        // read. Two backends looking at the same table cannot both claim the
        // same turn. The real next turn is written when the run ends: a
        // `once` task is closed only after it actually ran.
        let lease = now_ms + RUN_LEASE_MS;
        if let Err(error) = guard.db().claim_schedule(task.id, previous_next, Some(lease)) {
            tracing::warn!("the task {} could not be claimed: {error}", task.id);
            continue;
        }
        let next = next_turn(guard, &task, now_ms);
        let guard = Arc::clone(guard);
        let store = Arc::clone(store);
        let session = session.clone();
        let events = events.clone();
        tokio::spawn(async move {
            run_one(&guard, &store, session, &events, &task, lease, next).await;
        });
        started += 1;
    }
    started
}

/// One task: it runs, then its turn is closed. A `once` task that never got
/// to run stays due, so the single shot is not lost to a harness that is
/// missing or a pause that arrived after the dispatch.
async fn run_one(
    guard: &Guard,
    store: &Store,
    session: Option<Arc<Session>>,
    events: &Sender<BackendEvent>,
    task: &Schedule,
    lease: i64,
    next: Option<i64>,
) {
    let ran = run_task(guard, store, session, events, task).await;
    let final_next = if task.kind == whatfast_auto::Kind::Once && !ran {
        task.next_at_ms
    } else {
        next
    };
    // The lease is only replaced if it is still ours: an edit of the task
    // while it ran wins, and its own turn is left alone.
    match guard.db().claim_schedule(task.id, lease, final_next) {
        Ok(true) => {}
        Ok(false) => tracing::info!("the turn of the task {} changed while it ran", task.id),
        Err(error) => tracing::warn!("the turn of the task {} was not closed: {error}", task.id),
    }
}

/// The body of one task. `false` means the harness never ran or its answer
/// could not be kept: the task is still due and the next tick retries it.
async fn run_task(
    guard: &Guard,
    store: &Store,
    session: Option<Arc<Session>>,
    events: &Sender<BackendEvent>,
    task: &Schedule,
) -> bool {
    let started = std::time::Instant::now();
    let agent_id = task.agent_id.clone();
    let chat_id = task.chat_id.clone();
    let log = |ok: bool, detail: Option<String>| {
        let call = whatfast_auto::Call {
            agent_id: agent_id.clone(),
            tool: "schedule".to_owned(),
            chat_id: Some(chat_id.clone()),
            ok,
            ms: started.elapsed().as_millis() as i64,
            bytes: 0,
            detail,
        };
        guard.log(
            &crate::guard::Who::Agent {
                id: agent_id.clone(),
                // The log does not read the grant; an empty one is enough here.
                grant: whatfast_auto::Grant::default(),
            },
            &call,
            crate::db::now_ms(),
        );
    };
    // A pause, a revocation or a change of grant after the dispatch stops the
    // run here. The same check runs again just before the send.
    if let Err(reason) = permitted(guard, store, task, false) {
        log(false, Some(format!("la automatización no se ejecutó: {reason}")));
        return false;
    }
    let Some(spec) = whatfast_agents::by_id(&task.agent_id) else {
        log(
            false,
            Some(format!("the harness {} is not known", task.agent_id)),
        );
        return false;
    };
    let Some(run) = spec.run else {
        log(false, Some(format!("{} has no headless mode", spec.name)));
        return false;
    };
    let prompt = build_prompt(task);
    // The harness blocks on its own child; the run waits for it on the pool so
    // the send that follows can await the session.
    let answer = match tokio::task::spawn_blocking(move || {
        runner::run(&run, &prompt, runner::DEFAULT_TIMEOUT)
    })
    .await
    {
        Ok(Ok(answer)) => answer,
        Ok(Err(error)) => {
            tracing::info!("the task {} did not answer: {error}", task.id);
            log(false, Some(describe(&error)));
            return false;
        }
        Err(error) => {
            log(false, Some(format!("the harness task did not finish: {error}")));
            return false;
        }
    };

    let online = session
        .as_ref()
        .is_some_and(|session| session.state.connection() == ConnectionState::Connected);
    match delivery(task.output, online) {
        Delivery::Nothing => {
            tracing::info!("the task {} only read the chat", task.id);
            // The summary has no screen of its own yet: the first line goes to
            // the log, so the user can still see that the task ran and what it
            // found.
            log(true, Some(format!("solo lectura: {}", first_line(&answer.text))));
            true
        }
        Delivery::Draft => leave_draft(
            guard,
            task,
            &answer.text,
            format!("automatización: {}", first_line(&task.prompt)),
            DraftState::Proposed,
            &log,
        ),
        Delivery::Send => {
            let Some(session) = session.as_ref() else {
                // `delivery` never says send without a session; keep the text
                // anyway if that ever changes.
                leave_draft(
                    guard,
                    task,
                    &answer.text,
                    "la automatización no encontró la sesión: el mensaje quedó como borrador"
                        .to_owned(),
                    DraftState::Proposed,
                    &log,
                );
                return false;
            };
            if let Err(reason) = permitted(guard, store, task, true) {
                tracing::info!("the task {} may not send now: {reason}", task.id);
                return leave_draft(
                    guard,
                    task,
                    &answer.text,
                    format!("la automatización no pudo enviar: {reason}"),
                    DraftState::Failed,
                    &log,
                );
            }
            let request = SendTextRequest {
                client_message_id: crate::send::uuid4(),
                chat_id: task.chat_id.clone(),
                text: answer.text.clone(),
                ..SendTextRequest::default()
            };
            match crate::send::send_text(session, store, &request).await {
                Ok(row) => {
                    // The same two events the answer of a `SendText` pushes:
                    // the bubble shows up in the chat and the list moves it up.
                    let _ = events.send(crate::ingest::message_upserted(&row)).await;
                    match crate::send::refresh_chat_pointer(store, &row.chat_jid) {
                        Ok(chat) => {
                            let _ = events.send(crate::ingest::chat_upserted(chat)).await;
                        }
                        Err(error) => {
                            tracing::warn!("the chat pointer was not refreshed: {error}");
                        }
                    }
                    tracing::info!("the task {} sent a message to {}", task.id, task.chat_id);
                    // A message of an automation uses the ceiling of its
                    // agent, just like a message sent through the endpoint.
                    guard.log(
                        &Who::Agent {
                            id: agent_id.clone(),
                            grant: Grant::default(),
                        },
                        &whatfast_auto::Call {
                            agent_id: agent_id.clone(),
                            tool: "send_text".to_owned(),
                            chat_id: Some(chat_id.clone()),
                            ok: true,
                            ms: started.elapsed().as_millis() as i64,
                            bytes: 0,
                            detail: Some(format!("automatización {}", task.id)),
                        },
                        crate::db::now_ms(),
                    );
                    log(true, None);
                    true
                }
                Err((code, message, _)) => {
                    tracing::warn!("the task {} could not send ({code}): {message}", task.id);
                    leave_draft(
                        guard,
                        task,
                        &answer.text,
                        format!("la automatización no pudo enviar ({code}): {message}"),
                        DraftState::Failed,
                        &log,
                    )
                }
            }
        }
    }
}

/// Re-reads the row and the grant before a run or a send: a task paused or an
/// agent revoked after the dispatch does not act for the account. The same
/// `check` of the local endpoint enforces the scope, the hours and the
/// ceiling of messages; a send request also counts against that ceiling.
fn permitted(guard: &Guard, store: &Store, task: &Schedule, sending: bool) -> Result<(), String> {
    let current = guard
        .db()
        .schedule(task.id)
        .map_err(|error| format!("agents.db: {error}"))?
        .ok_or_else(|| format!("the task {} was removed", task.id))?;
    if current.paused {
        return Err("the task was paused".to_owned());
    }
    // A missing or revoked agent closes the door even for a read.
    guard
        .current_grant(&task.agent_id)
        .map_err(|denied| denied.message)?;
    let who = Who::Agent {
        id: task.agent_id.clone(),
        grant: Grant::default(),
    };
    let request = if sending || task.output == Output::Send {
        RpcRequest::SendText(SendTextRequest {
            chat_id: task.chat_id.clone(),
            ..SendTextRequest::default()
        })
    } else if task.output == Output::Draft {
        RpcRequest::DraftPropose(DraftProposeRequest {
            chat_id: task.chat_id.clone(),
            agent_id: task.agent_id.clone(),
            ..DraftProposeRequest::default()
        })
    } else {
        RpcRequest::ListMessages(ListMessagesRequest {
            chat_id: task.chat_id.clone(),
            ..ListMessagesRequest::default()
        })
    };
    guard.check(&who, &request, store).map_err(|denied| denied.message)
}

/// Saves the answer as a draft and logs the result.
fn leave_draft(
    guard: &Guard,
    task: &Schedule,
    text: &str,
    reason: String,
    state: DraftState,
    log: &impl Fn(bool, Option<String>),
) -> bool {
    let draft = Draft {
        id: 0,
        agent_id: task.agent_id.clone(),
        chat_id: task.chat_id.clone(),
        reply_to: None,
        text: text.to_owned(),
        reason: Some(reason),
        confidence: None,
        state,
        at_ms: crate::db::now_ms(),
    };
    match guard.db().add_draft(&draft) {
        Ok(id) => {
            tracing::info!("the task {} left the draft {id}", task.id);
            log(true, None);
            true
        }
        Err(error) => {
            log(false, Some(format!("the draft was not saved: {error}")));
            false
        }
    }
}

/// What the harness is asked: the prompt of the task, its chat and the rule of
/// the house, which depends on what the task does with the answer.
fn build_prompt(task: &Schedule) -> String {
    let tail = match task.output {
        Output::Read => {
            "Esta tarea solo consulta: leé ese chat y no escribas un mensaje para mandar."
        }
        Output::Draft => {
            "Escribí el mensaje que hay que mandar a ese chat, y nada más: sin comillas, sin explicación y sin encabezado. Lo que escribas queda como borrador para que el usuario lo apruebe."
        }
        Output::Send => {
            "Escribí el mensaje que hay que mandar a ese chat, y nada más: sin comillas, sin explicación y sin encabezado. Se manda tal cual, así que no agregues notas ni comentarios."
        }
    };
    format!("{}\n\nChat: {}\n{tail}", task.prompt.trim(), task.chat_id)
}

/// The line of the prompt a schedule shows in the tray.
fn first_line(prompt: &str) -> String {
    let line = prompt.lines().map(str::trim).find(|line| !line.is_empty());
    line.unwrap_or_default().chars().take(80).collect()
}

/// The failure of a harness, in one line for the log and the screen.
fn describe(error: &RunError) -> String {
    match error {
        RunError::NoHeadless => "el harness no tiene modo headless".to_owned(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use whatfast_auto::{Kind, Output};

    fn task(kind: Kind, cron: Option<&str>, next_at_ms: Option<i64>) -> Schedule {
        Schedule {
            id: 1,
            agent_id: "codex".into(),
            chat_id: "5491100000000@s.whatsapp.net".into(),
            kind,
            at_ms: next_at_ms,
            cron: cron.map(str::to_owned),
            prompt: "resumí el día".into(),
            output: Output::Draft,
            window: None,
            next_at_ms,
            paused: false,
        }
    }

    #[test]
    fn a_task_that_runs_once_has_no_next_turn() {
        let (_profile, guard) = guard();
        let once = task(Kind::Once, None, Some(1_000));
        assert!(next_turn(&guard, &once, 2_000).is_none());
        // A cron without an expression has no turn either.
        let broken = task(Kind::Cron, None, Some(1_000));
        assert!(next_turn(&guard, &broken, 2_000).is_none());
    }

    #[test]
    fn a_cron_task_moves_to_its_next_turn() {
        let (_profile, guard) = guard();
        let cron = task(Kind::Cron, Some("0 20 * * *"), Some(1_793_016_000_000));
        let next = next_turn(&guard, &cron, 1_793_016_000_000).expect("next");
        // 20:00 in UTC-3 is 23:00 UTC of the same day.
        assert_eq!(next, 1_793_016_000_000 + 11 * 3_600_000);
    }

    #[test]
    fn a_send_task_sends_only_with_a_connection() {
        assert_eq!(delivery(Output::Send, true), Delivery::Send);
        // Offline, the answer waits as a draft instead of failing the send.
        assert_eq!(delivery(Output::Send, false), Delivery::Draft);
        assert_eq!(delivery(Output::Draft, true), Delivery::Draft);
        assert_eq!(delivery(Output::Draft, false), Delivery::Draft);
        // A read leaves nothing, with or without a connection.
        assert_eq!(delivery(Output::Read, true), Delivery::Nothing);
        assert_eq!(delivery(Output::Read, false), Delivery::Nothing);
    }

    #[test]
    fn the_prompt_says_which_chat_and_what_to_answer() {
        let prompt = build_prompt(&task(Kind::Once, None, Some(1)));
        // The harness needs the chat it answers and the exact instruction:
        // without them the draft lands in the wrong place or answers the
        // wrong thing.
        assert!(prompt.contains("5491100000000@s.whatsapp.net"));
        assert!(prompt.contains("resumí el día"));
        assert!(prompt.contains("sin comillas, sin explicación"));
        assert!(prompt.contains("borrador"));
    }

    #[test]
    fn the_prompt_promises_only_what_the_task_does() {
        let mut send = task(Kind::Once, None, Some(1));
        send.output = Output::Send;
        let prompt = build_prompt(&send);
        assert!(!prompt.contains("borrador"));
        assert!(prompt.contains("Se manda tal cual"));

        let mut read = task(Kind::Once, None, Some(1));
        read.output = Output::Read;
        let prompt = build_prompt(&read);
        assert!(!prompt.contains("borrador"));
        assert!(prompt.contains("no escribas un mensaje"));
    }

    #[test]
    fn the_reason_is_the_first_line() {
        assert_eq!(first_line("  hola\nmundo"), "hola");
        assert_eq!(first_line("   "), "");
    }

    /// A profile of its own, removed when the test ends.
    fn guard() -> (std::path::PathBuf, Arc<Guard>) {
        let dir = std::env::temp_dir().join(format!(
            "whatfast-scheduler-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("profile");
        let guard = Guard::open(&dir, -180).expect("agents.db");
        (dir, Arc::new(guard))
    }
}
