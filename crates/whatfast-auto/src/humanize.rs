//! The rhythm of a person writing, not the rhythm of a program.
//!
//! A model answers in one instant. A message that leaves right after it arrives
//! looks automated to the person on the other side, and the platform watches
//! the same tell. This module turns "send this text" into a plan: the parts it
//! is written in and how long to wait before each one.
//!
//! The profile is the prudent one the user picked: a short wait, a maximum so a
//! long text never turns into minutes, and gaps that change every time so two
//! sends never line up. The random source is seeded, so a test can expect an
//! exact plan.

/// How long a person takes to write, and how a long text is broken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pace {
    /// Milliseconds of writing per character.
    pub per_char_ms: u64,
    /// The shortest wait, for a message of two words.
    pub min_ms: u64,
    /// The longest wait, whatever the length.
    pub max_ms: u64,
    /// The shortest gap between two parts.
    pub gap_min_ms: u64,
    /// The longest gap between two parts.
    pub gap_max_ms: u64,
    /// The length a part is split at.
    pub chunk_chars: usize,
}

impl Pace {
    /// The prudent profile: 1,5 s at the shortest, 6 s at the longest.
    pub fn prudent() -> Self {
        Self {
            per_char_ms: 45,
            min_ms: 1_500,
            max_ms: 6_000,
            gap_min_ms: 800,
            gap_max_ms: 1_800,
            chunk_chars: 700,
        }
    }

    /// No wait at all, for the tests and for a user who turns it off.
    pub fn instant() -> Self {
        Self {
            per_char_ms: 0,
            min_ms: 0,
            max_ms: 0,
            gap_min_ms: 0,
            gap_max_ms: 0,
            chunk_chars: 700,
        }
    }
}

impl Default for Pace {
    fn default() -> Self {
        Self::prudent()
    }
}

/// The wait for a text of this many characters, before the random gap.
pub fn typing_ms(chars: usize, pace: &Pace) -> u64 {
    let written = pace.per_char_ms.saturating_mul(chars as u64);
    written.clamp(pace.min_ms, pace.max_ms.max(pace.min_ms))
}

/// What to send, and how long to wait before each part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The text cut into the parts a person would write.
    pub parts: Vec<String>,
    /// The wait before each part, in the same order.
    pub waits_ms: Vec<u64>,
}

impl Plan {
    /// The whole plan for a text that is already fast.
    pub fn single(text: &str) -> Self {
        Self {
            parts: vec![text.to_string()],
            waits_ms: vec![0],
        }
    }

    /// Whether there is nothing to send.
    pub fn is_empty(&self) -> bool {
        self.parts.iter().all(|part| part.trim().is_empty())
    }
}

/// A small deterministic source of change, so two sends never look alike and a
/// test still knows the answer.
#[derive(Clone, Debug)]
pub struct Jitter {
    state: u64,
}

impl Jitter {
    /// Seeds the source. Zero is replaced: xorshift never leaves it.
    pub fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0x9e37_79b9_7f4a_7c15
            } else {
                seed
            },
        }
    }

    /// The next number of the sequence.
    pub fn next_u64(&mut self) -> u64 {
        let mut state = self.state;
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        self.state = state;
        state
    }

    /// A number between the two bounds, both included.
    pub fn between(&mut self, low: u64, high: u64) -> u64 {
        if high <= low {
            return low;
        }
        low + self.next_u64() % (high - low + 1)
    }
}

/// Plans one text: the parts it goes out in and the wait before each one.
pub fn plan(text: &str, pace: &Pace, seed: u64) -> Plan {
    let parts = split(text, pace.chunk_chars);
    let mut jitter = Jitter::new(seed);
    let mut waits = Vec::with_capacity(parts.len());
    for (index, part) in parts.iter().enumerate() {
        let gap = if index == 0 {
            0
        } else {
            jitter.between(pace.gap_min_ms, pace.gap_max_ms)
        };
        let typing = if pace.per_char_ms == 0 {
            0
        } else {
            let written = typing_ms(part.chars().count(), pace);
            // A person writing does not take the same time twice.
            written + jitter.between(0, written / 5 + 1)
        };
        waits.push(gap + typing);
    }
    Plan {
        parts,
        waits_ms: waits,
    }
}

/// Cuts a text into the parts a person would send, at the punctuation when it
/// is close enough.
pub fn split(text: &str, max_chars: usize) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let limit = max_chars.max(80);
    let mut parts = Vec::new();
    let mut rest = trimmed;
    loop {
        let chars = rest.chars().count();
        if chars <= limit {
            parts.push(rest.trim().to_string());
            break;
        }
        let cut = breakpoint(rest, limit);
        let (head, tail) = rest.split_at(cut);
        let head = head.trim();
        if !head.is_empty() {
            parts.push(head.to_string());
        }
        rest = tail.trim_start();
        if rest.is_empty() {
            break;
        }
    }
    parts
}

/// The byte offset of a byte-safe cut of at most `limit` characters: the last
/// sentence end, else the last space, else the character itself.
fn breakpoint(text: &str, limit: usize) -> usize {
    let hard = limit_chars(text, limit);
    let head = &text[..hard];
    // A sentence end is the cut a person makes, but never one so early that the
    // first part becomes a stub.
    if let Some(offset) = last_sentence_end(head)
        && offset * 2 >= hard
    {
        return offset;
    }
    match head.rfind(' ') {
        Some(offset) if offset > 0 => offset,
        _ => hard,
    }
}

/// The byte offset just past the last `.`, `!`, `?` or newline of the text.
fn last_sentence_end(head: &str) -> Option<usize> {
    let mut found = None;
    for (offset, character) in head.char_indices() {
        if matches!(character, '.' | '!' | '?' | '\n') {
            found = Some(offset + character.len_utf8());
        }
    }
    found
}

/// The number of bytes the first `limit` characters of the text take.
fn limit_chars(text: &str, limit: usize) -> usize {
    text.char_indices()
        .nth(limit)
        .map(|(offset, _)| offset)
        .unwrap_or(text.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_text_is_one_part() {
        let plan = plan("hola, ¿cómo va?", &Pace::prudent(), 7);
        assert_eq!(plan.parts, vec!["hola, ¿cómo va?".to_string()]);
        assert_eq!(plan.waits_ms.len(), 1);
    }

    #[test]
    fn the_wait_has_a_floor_and_a_ceiling() {
        let pace = Pace::prudent();
        assert_eq!(typing_ms(1, &pace), pace.min_ms);
        assert_eq!(typing_ms(10_000, &pace), pace.max_ms);
        assert_eq!(typing_ms(40, &pace), 1_800);
    }

    #[test]
    fn an_instant_pace_waits_nothing() {
        let plan = plan(&"a".repeat(900), &Pace::instant(), 1);
        assert!(plan.waits_ms.iter().all(|wait| *wait == 0));
    }

    #[test]
    fn a_long_text_is_cut_at_the_punctuation() {
        let text = format!("{}. {}", "a".repeat(400), "b".repeat(400));
        let parts = split(&text, 700);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], format!("{}.", "a".repeat(400)));
        assert_eq!(parts[1], "b".repeat(400));
    }

    #[test]
    fn a_text_without_punctuation_is_cut_at_a_space() {
        let text = "palabra ".repeat(120);
        let parts = split(&text, 200);
        assert!(parts.len() > 1);
        for part in &parts {
            assert!(part.chars().count() <= 200);
            assert!(!part.starts_with(' '));
        }
        let rejoined = parts.join(" ");
        assert_eq!(rejoined, text.trim());
    }

    #[test]
    fn a_cut_never_splits_a_character() {
        let text = "ñ".repeat(500);
        let parts = split(&text, 100);
        assert_eq!(parts.concat(), text);
        for part in &parts {
            assert!(part.chars().all(|character| character == 'ñ'));
        }
    }

    #[test]
    fn an_empty_text_makes_no_part() {
        assert!(split("   \n ", 700).is_empty());
        assert!(plan("  ", &Pace::prudent(), 1).is_empty());
    }

    #[test]
    fn two_sends_do_not_wait_the_same() {
        let first = plan(&"x".repeat(120), &Pace::default(), 11);
        let second = plan(&"x".repeat(120), &Pace::default(), 12);
        assert_ne!(first.waits_ms, second.waits_ms);
    }

    #[test]
    fn the_same_seed_gives_the_same_plan() {
        let first = plan(&"hola".repeat(300), &Pace::default(), 99);
        let second = plan(&"hola".repeat(300), &Pace::default(), 99);
        assert_eq!(first, second);
    }

    #[test]
    fn the_wait_grows_with_the_text() {
        let pace = Pace::prudent();
        let short = plan("hola", &pace, 3).waits_ms[0];
        let long = plan(&"hola ".repeat(40), &pace, 3).waits_ms[0];
        assert!(long > short);
        assert!(long <= pace.max_ms * 2);
    }
}
