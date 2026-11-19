//! The fast mode of the chat: ask the connected agent one question.
//!
//! The user is reading a conversation and wants a hand with it —a summary, a
//! price, a translation—. The question goes to the harness that is already
//! connected, with the last messages of the chat as context, and its answer
//! comes back to the composer. Nothing is sent: writing the message stays a
//! human decision.

use std::time::Duration;

use whatfast_agents::by_id;

use crate::runner::{self, Answer, RunError};

/// The most of the conversation that travels with the question, in characters.
///
/// A chat of a phone is long and the harness pays for every character twice; a
/// few recent messages are what the question is about.
pub const MAX_CONTEXT: usize = 6_000;

/// The prompt of a question, with the tail of the conversation.
///
/// The messages arrive oldest first, already written as `who: text`, and the
/// oldest ones are cut first: the question is about what was just said.
pub fn question_prompt(chat_id: &str, question: &str, lines: &[String]) -> String {
    let mut context = String::new();
    for line in lines.iter().rev() {
        if context.len() + line.len() > MAX_CONTEXT {
            break;
        }
        context.insert_str(0, &format!("{line}\n"));
    }
    let context = context.trim();
    let mut prompt = String::new();
    if !context.is_empty() {
        prompt.push_str("Mensajes recientes del chat:\n");
        prompt.push_str(context);
        prompt.push_str("\n\n");
    }
    prompt.push_str(&format!("Chat: {chat_id}\n"));
    prompt.push_str(&format!("Pedido del usuario: {}\n\n", question.trim()));
    prompt.push_str(
        "Respondé corto y en el idioma del chat. Lo que escribas va al cuadro de texto del usuario, no se manda solo.",
    );
    prompt
}

/// Asks one harness a prompt and waits for the answer.
pub fn ask(agent: &str, prompt: &str, timeout: Duration) -> Result<Answer, RunError> {
    let spec = by_id(agent).ok_or(RunError::NoHeadless)?;
    match spec.run {
        Some(run) => runner::run(&run, prompt, timeout),
        None => Err(RunError::NoHeadless),
    }
}

/// The deadline of a question typed by a person, who is waiting for it.
pub const QUESTION_TIMEOUT: Duration = Duration::from_secs(90);

