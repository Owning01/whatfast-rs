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

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(count: usize) -> Vec<String> {
        (0..count)
            .map(|index| format!("persona {index}: mensaje {index}"))
            .collect()
    }

    #[test]
    fn the_prompt_carries_the_question_and_the_chat() {
        let prompt = question_prompt("5491100000000@s.whatsapp.net", "¿cuánto sale?", &lines(3));
        assert!(prompt.contains("5491100000000@s.whatsapp.net"));
        assert!(prompt.contains("¿cuánto sale?"));
        assert!(prompt.contains("persona 2"));
        assert!(prompt.contains("no se manda solo"));
    }

    #[test]
    fn an_empty_chat_still_makes_a_prompt() {
        let prompt = question_prompt("a@s.whatsapp.net", "hola", &[]);
        assert!(!prompt.contains("Mensajes recientes"));
        assert!(prompt.contains("hola"));
    }

    #[test]
    fn the_newest_messages_are_kept_when_the_chat_is_long() {
        let many = lines(400);
        let prompt = question_prompt("a@s.whatsapp.net", "resumí", &many);
        assert!(prompt.contains("persona 399"));
        assert!(!prompt.contains("persona 0:"), "the oldest lines are cut");
        assert!(prompt.len() < MAX_CONTEXT + 2_000);
    }

    #[test]
    fn the_question_is_trimmed() {
        let prompt = question_prompt("a@s.whatsapp.net", "  resumí  ", &[]);
        assert!(prompt.contains("Pedido del usuario: resumí"));
    }

    #[test]
    fn an_unknown_agent_has_no_headless_mode() {
        let error = ask("no-existe", "hola", QUESTION_TIMEOUT).expect_err("no headless");
        assert_eq!(error, RunError::NoHeadless);
    }

    #[test]
    fn a_harness_without_a_run_spec_is_refused() {
        // Cursor is detected and registered but has no print mode.
        let error = ask("cursor", "hola", QUESTION_TIMEOUT).expect_err("no headless");
        assert_eq!(error, RunError::NoHeadless);
    }
}
