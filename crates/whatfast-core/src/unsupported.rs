//! Port of `tauri/src/lib/unsupported.ts`.
//!
//! The original decodes the human readable poll fallback that the backend
//! emits when it cannot decode a poll message.
//!
//! port: the original uses three regular expressions. `regex` is not an
//! authorised dependency of this crate (RULEBOOK D1), so the three patterns
//! are matched by hand, with the same semantics.

/// What a poll fallback renders as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollPresentation {
    pub title: String,
    pub options: Vec<String>,
    pub results: bool,
}

/// The bullet characters the original strips from every option line.
const OPTION_BULLETS: [char; 3] = ['\u{2022}', '\u{25CF}', '\u{25CB}'];

/// Decode the stable human-readable poll fallback emitted by the backend.
pub fn parse_poll_fallback(value: &str) -> PollPresentation {
    let mut lines: Vec<&str> = Vec::new();
    for line in split_lines(value) {
        let trimmed = line.trim_matches(is_js_whitespace);
        if !trimmed.is_empty() {
            lines.push(trimmed);
        }
    }

    // port: `lines.shift() ?? "Poll"`.
    let heading = lines.first().copied().unwrap_or("Poll");
    let rest = &lines[1.min(lines.len())..];

    let results = is_poll_results_heading(heading);
    let stripped = strip_poll_heading(heading);
    let title = stripped.trim_matches(is_js_whitespace);
    let title = if title.is_empty() {
        "Poll".to_string()
    } else {
        title.to_string()
    };

    let options = rest
        .iter()
        .map(|line| {
            let without_bullet = strip_option_bullet(line);
            without_bullet.trim_matches(is_js_whitespace)
        })
        .filter(|line| !line.is_empty())
        .map(|line| line.to_string())
        .collect();

    PollPresentation {
        title,
        options,
        results,
    }
}

/// port: the original splits with `/\r?\n/`, so a lone `\r` is not a separator.
fn split_lines(value: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    for line in value.split('\n') {
        lines.push(line.strip_suffix('\r').unwrap_or(line));
    }
    lines
}

/// port: `/^📊\s*Poll results\b/i` — the heading announces the results.
fn is_poll_results_heading(heading: &str) -> bool {
    let Some(rest) = heading.strip_prefix('\u{1F4CA}') else {
        return false;
    };
    let rest = rest.trim_start_matches(is_js_whitespace);
    let Some(rest) = strip_prefix_ignore_case(rest, "Poll") else {
        return false;
    };
    let Some(rest) = rest.strip_prefix(' ') else {
        return false;
    };
    let Some(rest) = strip_prefix_ignore_case(rest, "results") else {
        return false;
    };
    // port: `\b` is a word boundary and a JS word character is ASCII only.
    match rest.chars().next() {
        None => true,
        Some(next) => !(next.is_ascii_alphanumeric() || next == '_'),
    }
}

/// port: `/^📊\s*Poll(?: results)?\s*:?\s*/i` — removes the heading prefix.
fn strip_poll_heading(heading: &str) -> &str {
    let Some(rest) = heading.strip_prefix('\u{1F4CA}') else {
        return heading;
    };
    let rest = rest.trim_start_matches(is_js_whitespace);
    let Some(mut rest) = strip_prefix_ignore_case(rest, "Poll") else {
        return heading;
    };
    // port: the `(?: results)?` group is optional and greedy.
    if let Some(after_space) = rest.strip_prefix(' ')
        && let Some(after_results) = strip_prefix_ignore_case(after_space, "results")
    {
        rest = after_results;
    }
    let rest = rest.trim_start_matches(is_js_whitespace);
    let rest = rest.strip_prefix(':').unwrap_or(rest);
    rest.trim_start_matches(is_js_whitespace)
}

/// port: `/^[•●○]\s*/` — removes the bullet of an option line.
fn strip_option_bullet(line: &str) -> &str {
    let mut chars = line.chars();
    match chars.next() {
        Some(first) if OPTION_BULLETS.contains(&first) => {
            chars.as_str().trim_start_matches(is_js_whitespace)
        }
        _ => line,
    }
}

/// port: the `i` flag of the regular expressions.
fn strip_prefix_ignore_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let head = value.get(..prefix.len())?;
    if head.eq_ignore_ascii_case(prefix) {
        Some(&value[prefix.len()..])
    } else {
        None
    }
}

/// port: the `\s` of the regular expressions is the JS white space set.
fn is_js_whitespace(value: char) -> bool {
    matches!(
        value,
        '\u{0009}'..='\u{000D}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
            | '\u{FEFF}'
    )
}
