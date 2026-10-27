//! The automations: what comes due, runs, and leaves its answer as a draft.
//!
//! The table of `schedules` keeps the tasks, the cron keeps their turns and the
//! window keeps them inside the hours of the user. This module is the part that
//! actually moves: it looks for the tasks that came due, writes their next
//! turn, and asks the harness of the task for an answer without a window in
//! front of it.
//!
//! The answer never leaves on its own. A task leaves a **draft** with the text
//! the harness wrote: the user reads it and decides. Sending from a schedule is
//! the next step, and it needs the tray to exist first.

use std::sync::Arc;

use whatfast_auto::{
    Draft, DraftState, Schedule,
    runner::{self, RunError},
};

use crate::{db::Store, guard::Guard};

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

/// Runs every task that came due, without holding the serve loop.
///
/// It returns how many it started. Each one runs on the blocking pool: a
/// harness is another program with its own network, and the backend has a
/// desktop to answer meanwhile.
pub fn dispatch(guard: &Arc<Guard>, store: &Arc<Store>, now_ms: i64) -> usize {
    let due = match guard.db().due(now_ms) {
        Ok(due) => due,
        Err(error) => {
            tracing::warn!("the schedules were not read: {error}");
            return 0;
        }
    };
    let mut started = 0;
    for task in due {
        // The next turn is written before the run: a harness that hangs or a
        // backend that dies must not fire the same task twice.
        let next = next_turn(guard, &task, now_ms);
        if let Err(error) = guard.db().set_schedule_next(task.id, next) {
            tracing::warn!(
                "the next turn of the task {} was not written: {error}",
                task.id
            );
            continue;
        }
        let guard = Arc::clone(guard);
        let store = Arc::clone(store);
        tokio::task::spawn_blocking(move || run_one(&guard, &store, &task));
        started += 1;
    }
    started
}

/// One task: the harness, its answer, and the row in the log.
fn run_one(guard: &Guard, store: &Store, task: &Schedule) {
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
    let Some(spec) = whatfast_agents::by_id(&task.agent_id) else {
        log(
            false,
            Some(format!("the harness {} is not known", task.agent_id)),
        );
        return;
    };
    let Some(run) = spec.run else {
        log(false, Some(format!("{} has no headless mode", spec.name)));
        return;
    };
    let prompt = build_prompt(task);
    match runner::run(&run, &prompt, runner::DEFAULT_TIMEOUT) {
        Ok(answer) => {
            let draft = Draft {
                id: 0,
                agent_id: task.agent_id.clone(),
                chat_id: task.chat_id.clone(),
                reply_to: None,
                text: answer.text.clone(),
                reason: Some(format!("automatización: {}", first_line(&task.prompt))),
                confidence: None,
                state: DraftState::Proposed,
                at_ms: crate::db::now_ms(),
            };
            match guard.db().add_draft(&draft) {
                Ok(id) => {
                    tracing::info!("the task {} left the draft {id}", task.id);
                    log(true, None);
                }
                Err(error) => log(false, Some(format!("the draft was not saved: {error}"))),
            }
        }
        Err(error) => {
            tracing::info!("the task {} did not answer: {error}", task.id);
            log(false, Some(describe(&error)));
        }
    }
    let _ = store;
}

/// What the harness is asked: the prompt of the task, its chat and the rule of
/// the house.
fn build_prompt(task: &Schedule) -> String {
    format!(
        "{}\n\nChat: {}\nEscribí el mensaje que hay que mandar a ese chat, y nada más: sin comillas, sin explicación y sin encabezado. Lo que escribas queda como borrador para que el usuario lo apruebe.",
        task.prompt.trim(),
        task.chat_id
    )
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
    fn a_cron_task_without_an_expression_never_turns() {
        let (_profile, guard) = guard();
        let broken = task(Kind::Cron, None, Some(1_000));
        assert!(next_turn(&guard, &broken, 2_000).is_none());
    }

    #[test]
    fn the_prompt_says_which_chat_and_what_to_answer() {
        let prompt = build_prompt(&task(Kind::Once, None, Some(1)));
        assert!(prompt.contains("5491100000000@s.whatsapp.net"));
        assert!(prompt.contains("resumí el día"));
        assert!(prompt.contains("borrador"));
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
