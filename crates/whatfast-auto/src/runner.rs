//! Running a harness without a window.
//!
//! The fast mode of the chat and the automations both need the same thing: ask
//! a harness a question and read its answer, with nobody in front of it. Every
//! harness that has a headless mode says so in its [`RunSpec`]: the program and
//! the arguments that put it in print mode, with the prompt on standard input.
//!
//! The call is bounded on purpose. A harness is another program with its own
//! network, its own login and its own bugs: it gets a deadline, and past it the
//! child is killed instead of holding the backend.

use std::{
    io::Write as _,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use whatfast_agents::RunSpec;

/// The longest a harness may take before it is killed.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// What a harness answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    /// The text of standard output, trimmed.
    pub text: String,
    /// The exit code, when the system reported one.
    pub code: Option<i32>,
    /// How long it took.
    pub ms: i64,
}

/// Why a harness did not answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunError {
    /// This harness has no headless mode: it cannot be asked anything.
    NoHeadless,
    /// The program could not start at all.
    Start(String),
    /// It ran past the deadline and was killed.
    Timeout(Duration),
    /// It ended with a failure, and said why on standard error.
    Failed {
        /// The exit code, when the system reported one.
        code: Option<i32>,
        /// Standard error, trimmed and cut to a line.
        detail: String,
    },
    /// It ended well but wrote nothing the user could read.
    Empty,
}

impl std::fmt::Display for RunError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoHeadless => {
                write!(
                    formatter,
                    "this harness has no headless mode to ask it anything"
                )
            }
            Self::Start(detail) => write!(formatter, "the harness did not start: {detail}"),
            Self::Timeout(limit) => write!(
                formatter,
                "the harness ran past {} s and was killed",
                limit.as_secs()
            ),
            Self::Failed { code, detail } => match code {
                Some(code) if detail.is_empty() => {
                    write!(formatter, "the harness ended with code {code}")
                }
                Some(code) => write!(formatter, "the harness ended with code {code}: {detail}"),
                None => write!(formatter, "the harness ended without a code: {detail}"),
            },
            Self::Empty => write!(formatter, "the harness answered nothing"),
        }
    }
}

/// Asks one harness a prompt and waits for its answer.
///
/// The prompt goes to standard input, which is how every print mode reads it:
/// an argument would show the text in the process list of the machine.
pub fn run(spec: &RunSpec, prompt: &str, timeout: Duration) -> Result<Answer, RunError> {
    let mut command = Command::new(spec.bin);
    command.args(spec.print_args);
    run_command(&mut command, prompt, timeout)
}

/// The same, over a command the caller already built.
///
/// It is public so a test —and the future "ask the agent" of the chat— can hand
/// in a command of its own without going through a harness spec.
pub fn run_command(
    command: &mut Command,
    prompt: &str,
    timeout: Duration,
) -> Result<Answer, RunError> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        // The backend has no console: a harness that opened one would flash a
        // window on the desk of the user.
        command.creation_flags(0x08000000);
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let started = Instant::now();
    let mut child = command
        .spawn()
        .map_err(|error| RunError::Start(error.to_string()))?;
    // The prompt is written and the pipe closed: the print modes read until
    // the end of standard input. A prompt bigger than the pipe buffer waits for
    // the child to read, and the readers are already up, so it cannot deadlock.
    let mut stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let payload = prompt.as_bytes().to_vec();
    let writer = thread::spawn(move || {
        if let Some(stdin) = stdin.as_mut() {
            let _ = stdin.write_all(&payload);
            let _ = stdin.flush();
        }
        drop(stdin.take());
    });
    let reader_out = thread::spawn(move || drain(stdout));
    let reader_err = thread::spawn(move || drain(stderr));

    let deadline = started + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => thread::sleep(Duration::from_millis(25)),
            Err(error) => return Err(RunError::Start(error.to_string())),
        }
    };
    let _ = writer.join();
    let output = reader_out.join().unwrap_or_default();
    let failure = reader_err.join().unwrap_or_default();
    let ms = started.elapsed().as_millis() as i64;
    let Some(status) = status else {
        return Err(RunError::Timeout(timeout));
    };
    let text = String::from_utf8_lossy(&output).trim().to_owned();
    if !status.success() {
        return Err(RunError::Failed {
            code: status.code(),
            detail: one_line(&String::from_utf8_lossy(&failure)),
        });
    }
    if text.is_empty() {
        return Err(RunError::Empty);
    }
    Ok(Answer {
        text,
        code: status.code(),
        ms,
    })
}

/// Reads a pipe to its end; a pipe that cannot be read counts as empty.
fn drain<R: std::io::Read>(pipe: Option<R>) -> Vec<u8> {
    let Some(mut pipe) = pipe else {
        return Vec::new();
    };
    let mut buffer = Vec::new();
    let _ = pipe.read_to_end(&mut buffer);
    buffer
}

/// The first meaningful line of a message, cut to a size a screen can show.
fn one_line(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    line.chars().take(300).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A command of the system that copies standard input to standard output.
    #[cfg(windows)]
    fn echo_command() -> Command {
        let mut command = Command::new("cmd");
        command.args(["/c", "more"]);
        command
    }

    /// The same on the systems that have `cat`.
    #[cfg(not(windows))]
    fn echo_command() -> Command {
        Command::new("cat")
    }

    /// A command that never ends on its own, for the deadline.
    #[cfg(windows)]
    fn sleeper() -> Command {
        let mut command = Command::new("ping");
        command.args(["-n", "30", "127.0.0.1"]);
        command
    }

    /// The same on the systems that have `sleep`.
    #[cfg(not(windows))]
    fn sleeper() -> Command {
        let mut command = Command::new("sleep");
        command.arg("30");
        command
    }

    /// A command that ends well and writes nothing.
    #[cfg(windows)]
    fn silent() -> Command {
        let mut command = Command::new("cmd");
        command.args(["/c", "exit", "0"]);
        command
    }

    /// The same on the systems that have `true`.
    #[cfg(not(windows))]
    fn silent() -> Command {
        Command::new("true")
    }

    /// A command that fails with a code.
    #[cfg(windows)]
    fn failing() -> Command {
        let mut command = Command::new("cmd");
        command.args(["/c", "exit", "3"]);
        command
    }

    /// The same on the systems that have `false`.
    #[cfg(not(windows))]
    fn failing() -> Command {
        Command::new("false")
    }

    #[test]
    fn the_answer_is_the_output_of_the_harness() {
        let answer = run_command(&mut echo_command(), "hola\n", DEFAULT_TIMEOUT).expect("answer");
        assert_eq!(answer.text, "hola");
        assert_eq!(answer.code, Some(0));
    }

    #[test]
    fn a_harness_that_runs_forever_is_killed() {
        let limit = Duration::from_millis(400);
        let error = run_command(&mut sleeper(), "hola", limit).expect_err("timeout");
        assert_eq!(error, RunError::Timeout(limit));
        assert!(error.to_string().contains("killed"));
    }

    #[test]
    fn an_empty_answer_is_not_an_answer() {
        let error = run_command(&mut silent(), "hola", DEFAULT_TIMEOUT).expect_err("empty");
        assert_eq!(error, RunError::Empty);
    }

    #[test]
    fn a_failure_carries_its_code() {
        let error = run_command(&mut failing(), "hola", DEFAULT_TIMEOUT).expect_err("failed");
        match error {
            RunError::Failed { code, .. } => assert_ne!(code, Some(0)),
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[test]
    fn a_program_that_does_not_exist_is_a_start_failure() {
        let mut command = Command::new("whatfast-no-existe-en-esta-maquina");
        let error = run_command(&mut command, "hola", DEFAULT_TIMEOUT).expect_err("start");
        assert!(matches!(error, RunError::Start(_)), "{error:?}");
    }

    #[test]
    fn the_failure_of_a_harness_without_headless_says_so() {
        assert!(RunError::NoHeadless.to_string().contains("headless"));
    }

    #[test]
    fn a_long_prompt_does_not_deadlock_the_pipe() {
        let prompt = "x".repeat(200_000);
        let answer = run_command(&mut echo_command(), &prompt, DEFAULT_TIMEOUT).expect("answer");
        // `more` may add its own line breaks; the prompt has to come back whole.
        assert!(answer.text.len() >= prompt.len(), "{}", answer.text.len());
        assert!(answer.text.trim_end().ends_with(&prompt[..64]));
    }
}
