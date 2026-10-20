//! A small, deliberately non-HTML WhatsApp text-formatting parser.
//!
//! Port of `tauri/src/lib/whatsapp-formatting.ts` (the frozen oracle of the
//! migration). Consumers should render these tokens as plain text; the parser
//! never produces markup, every string it returns is inert text.
//!
//! Port notes:
//!
//! - The original indexes JavaScript strings by UTF-16 code unit; this port
//!   indexes by Unicode scalar value (`char`). The only observable difference
//!   is for astral characters next to a delimiter: JavaScript sees a lone
//!   surrogate there, which matches neither `\p{L}` nor `\p{N}`, so an astral
//!   character is never a word character in this port.
//! - The original leans on regular expressions (`/\s/u`, `/[\p{L}\p{N}]/u`,
//!   `/^https?:\/\/[^\s]+/iu`). The `regex` crate is not allowed in this
//!   crate, so every match is implemented by hand.
//! - `new URL(raw)` is implemented by hand as well (`parse_url`), following the
//!   WHATWG URL serialization rules that Node.js applies.
//!
//! Known deviations, all of them inside the IDNA processing of a host name:
//!
//! - The mapping of UTS#46 is approximated with the Unicode lowercase
//!   mapping: the compatibility decomposition (the full width forms, the
//!   ligatures) is not reproduced.
//! - The validity criteria that need the Unicode tables are not reproduced
//!   either: the NFC check, the Bidi rule and the unassigned code points. A
//!   domain that only fails one of them is a link here, while the original
//!   rejects the whole URL.

/// The inline styles WhatsApp supports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WhatsAppInlineStyle {
    Bold,
    Italic,
    Strikethrough,
    Monospace,
    InlineCode,
    Spoiler,
}

impl WhatsAppInlineStyle {
    /// The name of the style as the original spelled it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bold => "bold",
            Self::Italic => "italic",
            Self::Strikethrough => "strikethrough",
            Self::Monospace => "monospace",
            Self::InlineCode => "inline-code",
            Self::Spoiler => "spoiler",
        }
    }
}

/// The kind of an inline segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WhatsAppInlineSegmentKind {
    Text,
    Link,
    Mention,
}

/// One inline run of a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WhatsAppInlineSegment {
    Text {
        text: String,
        styles: Vec<WhatsAppInlineStyle>,
    },
    Link {
        text: String,
        href: String,
        styles: Vec<WhatsAppInlineStyle>,
    },
    Mention {
        text: String,
        /// The mention label without its leading `@`.
        mention: String,
        styles: Vec<WhatsAppInlineStyle>,
    },
}

impl WhatsAppInlineSegment {
    /// The kind of the segment.
    pub fn kind(&self) -> WhatsAppInlineSegmentKind {
        match self {
            Self::Text { .. } => WhatsAppInlineSegmentKind::Text,
            Self::Link { .. } => WhatsAppInlineSegmentKind::Link,
            Self::Mention { .. } => WhatsAppInlineSegmentKind::Mention,
        }
    }

    /// The plain text of the segment.
    pub fn text(&self) -> &str {
        match self {
            Self::Text { text, .. } | Self::Link { text, .. } | Self::Mention { text, .. } => text,
        }
    }

    /// The inline styles applied to the segment.
    pub fn styles(&self) -> &[WhatsAppInlineStyle] {
        match self {
            Self::Text { styles, .. }
            | Self::Link { styles, .. }
            | Self::Mention { styles, .. } => styles,
        }
    }
}

/// The kind of a block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WhatsAppTextBlockKind {
    Paragraph,
    Quote,
    BulletListItem,
    NumberedListItem,
    CodeBlock,
    BlankLine,
}

/// One block of a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WhatsAppTextBlock {
    Paragraph {
        segments: Vec<WhatsAppInlineSegment>,
    },
    Quote {
        segments: Vec<WhatsAppInlineSegment>,
    },
    BulletListItem {
        segments: Vec<WhatsAppInlineSegment>,
    },
    NumberedListItem {
        number: u32,
        segments: Vec<WhatsAppInlineSegment>,
    },
    CodeBlock {
        text: String,
    },
    BlankLine,
}

impl WhatsAppTextBlock {
    /// The kind of the block.
    pub fn kind(&self) -> WhatsAppTextBlockKind {
        match self {
            Self::Paragraph { .. } => WhatsAppTextBlockKind::Paragraph,
            Self::Quote { .. } => WhatsAppTextBlockKind::Quote,
            Self::BulletListItem { .. } => WhatsAppTextBlockKind::BulletListItem,
            Self::NumberedListItem { .. } => WhatsAppTextBlockKind::NumberedListItem,
            Self::CodeBlock { .. } => WhatsAppTextBlockKind::CodeBlock,
            Self::BlankLine => WhatsAppTextBlockKind::BlankLine,
        }
    }

    /// The inline segments of the block. Code blocks and blank lines have
    /// none.
    pub fn segments(&self) -> &[WhatsAppInlineSegment] {
        match self {
            Self::Paragraph { segments }
            | Self::Quote { segments }
            | Self::BulletListItem { segments }
            | Self::NumberedListItem { segments, .. } => segments,
            Self::CodeBlock { .. } | Self::BlankLine => &[],
        }
    }
}

/// Options of [`parse_whatsapp_text`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WhatsAppParseOptions {
    /// Display labels supplied by WhatsApp message metadata. Labels may be
    /// passed with or without `@`; longest labels win when names overlap.
    pub mentions: Option<Vec<String>>,
}

struct ParserContext {
    mentions: Vec<String>,
}

/// Parses WhatsApp's plain-text formatting into render-safe blocks and inline
/// segments. No source string is ever treated as markup or executable HTML.
pub fn parse_whatsapp_text(source: &str, options: &WhatsAppParseOptions) -> Vec<WhatsAppTextBlock> {
    let context = ParserContext {
        mentions: normalize_mentions(options.mentions.as_deref().unwrap_or(&[])),
    };
    let source: Vec<char> = source.chars().collect();
    let mut blocks: Vec<WhatsAppTextBlock> = Vec::new();
    let mut normal_start: usize = 0;
    let mut search_from: usize = 0;

    while search_from < source.len() {
        let Some(opening) = index_of(&source, "```", search_from) else {
            break;
        };
        let Some(closing) = index_of(&source, "```", opening + 3) else {
            break;
        };
        let content: String = source[opening + 3..closing].iter().collect();

        if content.contains('\n')
            && is_block_boundary_before(&source, opening)
            && is_block_boundary_after(&source, closing + 3)
        {
            append_line_blocks(
                &mut blocks,
                &source[normal_start..opening],
                &context,
                normal_start > 0,
            );
            blocks.push(WhatsAppTextBlock::CodeBlock {
                text: strip_fence_newlines(&content),
            });
            normal_start = closing + 3;
            search_from = normal_start;
            continue;
        }

        search_from = closing + 3;
    }

    append_line_blocks(
        &mut blocks,
        &source[normal_start..],
        &context,
        normal_start > 0,
    );
    blocks
}

fn append_line_blocks(
    output: &mut Vec<WhatsAppTextBlock>,
    source: &[char],
    context: &ParserContext,
    chunk_starts_after_code_block: bool,
) {
    if source.is_empty() {
        return;
    }

    let mut lines = split_lines(source);
    if chunk_starts_after_code_block && lines.first().is_some_and(|line| line.is_empty()) {
        lines.remove(0);
    }
    if lines.last().is_some_and(|line| line.is_empty()) && source.last() == Some(&'\n') {
        lines.pop();
    }

    for line in lines {
        if line.is_empty() {
            output.push(WhatsAppTextBlock::BlankLine);
            continue;
        }

        if line.starts_with(&['>', ' ']) {
            output.push(WhatsAppTextBlock::Quote {
                segments: parse_inline(&line[2..], context, &[]),
            });
            continue;
        }

        if line.starts_with(&['-', ' ']) {
            output.push(WhatsAppTextBlock::BulletListItem {
                segments: parse_inline(&line[2..], context, &[]),
            });
            continue;
        }

        if let Some((number, rest)) = match_numbered_list_item(line) {
            output.push(WhatsAppTextBlock::NumberedListItem {
                number,
                segments: parse_inline(rest, context, &[]),
            });
            continue;
        }

        output.push(WhatsAppTextBlock::Paragraph {
            segments: parse_inline(line, context, &[]),
        });
    }
}

fn parse_inline(
    source: &[char],
    context: &ParserContext,
    styles: &[WhatsAppInlineStyle],
) -> Vec<WhatsAppInlineSegment> {
    let mut output: Vec<WhatsAppInlineSegment> = Vec::new();
    let mut plain_start: usize = 0;
    let mut index: usize = 0;

    while index < source.len() {
        if starts_with_at(source, "http://", index) || starts_with_at(source, "https://", index) {
            if let Some(link) = read_http_url(source, index) {
                flush_plain(&mut output, &source[plain_start..index], styles, context);
                push_segment(
                    &mut output,
                    WhatsAppInlineSegment::Link {
                        text: link.text,
                        href: link.href,
                        styles: styles.to_vec(),
                    },
                );
                index = link.end;
                plain_start = index;
                continue;
            }
        }

        let code_delimiter = if starts_with_at(source, "```", index) {
            "```"
        } else if source[index] == '`' {
            "`"
        } else {
            ""
        };
        let code_delimiter_length = code_delimiter.chars().count();
        if code_delimiter_length > 0 && is_opening_delimiter(source, index, code_delimiter_length) {
            if let Some(closing) = find_closing_multi_char_delimiter(
                source,
                index + code_delimiter_length,
                code_delimiter,
            ) {
                flush_plain(&mut output, &source[plain_start..index], styles, context);
                let text: String = source[index + code_delimiter_length..closing]
                    .iter()
                    .collect();
                let mut nested_styles = styles.to_vec();
                nested_styles.push(if code_delimiter == "```" {
                    WhatsAppInlineStyle::Monospace
                } else {
                    WhatsAppInlineStyle::InlineCode
                });
                push_segment(
                    &mut output,
                    WhatsAppInlineSegment::Text {
                        text,
                        styles: nested_styles,
                    },
                );
                index = closing + code_delimiter_length;
                plain_start = index;
                continue;
            }
        }

        // `||spoiler||` hides its content until the reader reveals it. It is a
        // two-character delimiter, so it follows the same opening/closing rules
        // as the code delimiters.
        if starts_with_at(source, "||", index) && is_opening_delimiter(source, index, 2) {
            if let Some(closing) = find_closing_multi_char_delimiter(source, index + 2, "||") {
                flush_plain(&mut output, &source[plain_start..index], styles, context);
                let inner = &source[index + 2..closing];
                let nested_styles = push_style(styles, WhatsAppInlineStyle::Spoiler);
                for segment in parse_inline(inner, context, &nested_styles) {
                    push_segment(&mut output, segment);
                }
                index = closing + 2;
                plain_start = index;
                continue;
            }
        }

        let delimiter = source[index];
        if let Some(style) = delimiter_styles(delimiter) {
            if is_opening_delimiter(source, index, 1) {
                if let Some(closing) = find_closing_style_delimiter(source, index + 1, delimiter) {
                    flush_plain(&mut output, &source[plain_start..index], styles, context);
                    let inner = &source[index + 1..closing];
                    let nested_styles = push_style(styles, style);
                    for segment in parse_inline(inner, context, &nested_styles) {
                        push_segment(&mut output, segment);
                    }
                    index = closing + 1;
                    plain_start = index;
                    continue;
                }
            }
        }

        index += 1;
    }

    flush_plain(&mut output, &source[plain_start..], styles, context);
    output
}

fn append_lexical_segments(
    output: &mut Vec<WhatsAppInlineSegment>,
    source: &[char],
    styles: &[WhatsAppInlineStyle],
    context: &ParserContext,
) {
    let mut plain_start: usize = 0;
    let mut index: usize = 0;

    while index < source.len() {
        if starts_with_at(source, "http://", index) || starts_with_at(source, "https://", index) {
            if let Some(link) = read_http_url(source, index) {
                flush_text(output, &source[plain_start..index], styles);
                push_segment(
                    output,
                    WhatsAppInlineSegment::Link {
                        text: link.text,
                        href: link.href,
                        styles: styles.to_vec(),
                    },
                );
                index = link.end;
                plain_start = index;
                continue;
            }
        }

        if let Some(mention) = read_mention(source, index, &context.mentions) {
            flush_text(output, &source[plain_start..index], styles);
            push_segment(
                output,
                WhatsAppInlineSegment::Mention {
                    text: mention.text,
                    mention: mention.name,
                    styles: styles.to_vec(),
                },
            );
            index = mention.end;
            plain_start = index;
            continue;
        }

        index += 1;
    }

    flush_text(output, &source[plain_start..], styles);
}

fn find_closing_style_delimiter(source: &[char], start: usize, delimiter: char) -> Option<usize> {
    let mut index = start;
    while index < source.len() {
        if starts_with_at(source, "http://", index) || starts_with_at(source, "https://", index) {
            if let Some(link) = read_http_url(source, index) {
                let possible_closing = link.end.saturating_sub(1);
                if link.text.ends_with(delimiter)
                    && is_closing_delimiter(source, possible_closing, 1)
                {
                    return Some(possible_closing);
                }
                index = link.end;
                continue;
            }
        }
        if source[index] == delimiter && is_closing_delimiter(source, index, 1) {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn find_closing_multi_char_delimiter(
    source: &[char],
    start: usize,
    delimiter: &str,
) -> Option<usize> {
    let delimiter_length = delimiter.chars().count();
    let mut index = index_of(source, delimiter, start);
    while let Some(position) = index {
        if is_closing_delimiter(source, position, delimiter_length) {
            return Some(position);
        }
        index = index_of(source, delimiter, position + delimiter_length);
    }
    None
}

fn is_opening_delimiter(source: &[char], index: usize, length: usize) -> bool {
    let before = character_at(source, index.wrapping_sub(1));
    let after = source.get(index + length).copied();
    after.is_some_and(|value| !is_whitespace(value))
        && before.is_none_or(|value| !is_word_character(value))
}

fn is_closing_delimiter(source: &[char], index: usize, length: usize) -> bool {
    let before = character_at(source, index.wrapping_sub(1));
    let after = source.get(index + length).copied();
    before.is_some_and(|value| !is_whitespace(value))
        && after.is_none_or(|value| !is_word_character(value))
}

/// port: `/\s/u`. It is not the same set as Rust's `char::is_whitespace`:
/// JavaScript has U+FEFF and no U+0085.
fn is_whitespace(value: char) -> bool {
    matches!(
        value,
        '\u{9}'..='\u{d}'
            | '\u{20}'
            | '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
            | '\u{feff}'
    )
}

/// port: `/[\p{L}\p{N}]/u`. Two deviations, both documented in the module
/// docs: Rust's `Alphabetic` property also covers a few combining marks that
/// `\p{L}` does not, and an astral character is seen by JavaScript as a lone
/// surrogate, which is neither a letter nor a number.
fn is_word_character(value: char) -> bool {
    (value as u32) <= 0xffff && (value.is_alphabetic() || value.is_numeric())
}

struct LinkMatch {
    text: String,
    href: String,
    end: usize,
}

fn read_http_url(source: &[char], index: usize) -> Option<LinkMatch> {
    let before = character_at(source, index.wrapping_sub(1));
    if let Some(value) = before {
        if is_word_character(value) || value == '@' || value == '_' {
            return None;
        }
    }

    let matched = match_http_url(&source[index..])?;
    let raw = trim_url_punctuation(&matched);
    if raw.is_empty() {
        return None;
    }

    let url = parse_url(&raw)?;
    if url.protocol != "http:" && url.protocol != "https:" {
        return None;
    }

    Some(LinkMatch {
        text: raw.clone(),
        href: url.href,
        end: index + raw.chars().count(),
    })
}

/// port: `/^https?:\/\/[^\s]+/iu`.
fn match_http_url(source: &[char]) -> Option<String> {
    let scheme_length = if starts_with_at_ignore_case(source, "http://", 0) {
        "http://".chars().count()
    } else if starts_with_at_ignore_case(source, "https://", 0) {
        "https://".chars().count()
    } else {
        return None;
    };

    let mut end = scheme_length;
    while end < source.len() && !is_whitespace(source[end]) {
        end += 1;
    }
    if end == scheme_length {
        return None;
    }
    Some(source[..end].iter().collect())
}

fn trim_url_punctuation(value: &str) -> String {
    // port: /[.,!?;:]+$/u
    let mut result = value
        .trim_end_matches(['.', ',', '!', '?', ';', ':'])
        .to_string();

    let pairs = [('(', ')'), ('[', ']'), ('{', '}')];
    let mut changed = true;
    while changed {
        changed = false;
        for (opening, closing) in pairs {
            if result.ends_with(closing)
                && count_character(&result, closing) > count_character(&result, opening)
            {
                result.pop();
                changed = true;
            }
        }
    }
    result
}

fn count_character(value: &str, character: char) -> usize {
    value
        .chars()
        .filter(|current| *current == character)
        .count()
}

struct MentionMatch {
    text: String,
    name: String,
    end: usize,
}

fn read_mention(source: &[char], index: usize, mentions: &[String]) -> Option<MentionMatch> {
    if source.get(index) != Some(&'@') {
        return None;
    }
    let before = character_at(source, index.wrapping_sub(1));
    if before.is_some_and(is_word_character) {
        return None;
    }

    for name in mentions {
        let text = format!("@{name}");
        if !starts_with_at(source, &text, index) {
            continue;
        }
        let length = text.chars().count();
        let after = source.get(index + length).copied();
        if after.is_some_and(is_word_character) {
            continue;
        }
        return Some(MentionMatch {
            text,
            name: name.clone(),
            end: index + length,
        });
    }
    None
}

fn normalize_mentions(mentions: &[String]) -> Vec<String> {
    let mut normalized: Vec<String> = Vec::new();
    for mention in mentions {
        let trimmed = js_trim(mention);
        // port: `.replace(/^@/u, "")` removes a single leading `@`.
        let stripped = trimmed.strip_prefix('@').unwrap_or(trimmed);
        if stripped.is_empty() {
            continue;
        }
        if !normalized.iter().any(|existing| existing == stripped) {
            normalized.push(stripped.to_string());
        }
    }

    // port: `.sort((left, right) => right.length - left.length)`. JavaScript
    // lengths are UTF-16 code units, so an astral character counts as two.
    // The sort is stable, like `Array.prototype.sort` since ES2019.
    normalized.sort_by_key(|left| std::cmp::Reverse(utf16_length(left)));
    normalized
}

/// port: `String.prototype.trim`, which uses the same set as `/\s/u`.
fn js_trim(value: &str) -> &str {
    value.trim_matches(is_whitespace)
}

fn utf16_length(value: &str) -> usize {
    value.chars().map(char::len_utf16).sum()
}

fn is_block_boundary_before(source: &[char], index: usize) -> bool {
    index == 0 || source[index - 1] == '\n'
}

fn is_block_boundary_after(source: &[char], index: usize) -> bool {
    index == source.len() || source[index] == '\n'
}

fn strip_fence_newlines(value: &str) -> String {
    let without_leading = value.strip_prefix('\n').unwrap_or(value);
    without_leading
        .strip_suffix('\n')
        .unwrap_or(without_leading)
        .to_string()
}

fn push_segment(output: &mut Vec<WhatsAppInlineSegment>, segment: WhatsAppInlineSegment) {
    if segment.text().is_empty() {
        return;
    }
    let merge = match (output.last(), &segment) {
        (
            Some(WhatsAppInlineSegment::Text {
                styles: previous, ..
            }),
            WhatsAppInlineSegment::Text {
                styles: current, ..
            },
        ) => previous == current,
        _ => false,
    };
    if merge {
        if let (
            Some(WhatsAppInlineSegment::Text { text: previous, .. }),
            WhatsAppInlineSegment::Text { text: current, .. },
        ) = (output.last_mut(), &segment)
        {
            previous.push_str(current);
            return;
        }
    }
    output.push(segment);
}

fn push_style(
    styles: &[WhatsAppInlineStyle],
    style: WhatsAppInlineStyle,
) -> Vec<WhatsAppInlineStyle> {
    // port: `styles.includes(style) ? styles : [...styles, style]`.
    if styles.contains(&style) {
        styles.to_vec()
    } else {
        let mut nested = styles.to_vec();
        nested.push(style);
        nested
    }
}

fn delimiter_styles(delimiter: char) -> Option<WhatsAppInlineStyle> {
    match delimiter {
        '*' => Some(WhatsAppInlineStyle::Bold),
        '_' => Some(WhatsAppInlineStyle::Italic),
        '~' => Some(WhatsAppInlineStyle::Strikethrough),
        _ => None,
    }
}

fn flush_plain(
    output: &mut Vec<WhatsAppInlineSegment>,
    source: &[char],
    styles: &[WhatsAppInlineStyle],
    context: &ParserContext,
) {
    if source.is_empty() {
        return;
    }
    append_lexical_segments(output, source, styles, context);
}

fn flush_text(
    output: &mut Vec<WhatsAppInlineSegment>,
    source: &[char],
    styles: &[WhatsAppInlineStyle],
) {
    if source.is_empty() {
        return;
    }
    push_segment(
        output,
        WhatsAppInlineSegment::Text {
            text: source.iter().collect(),
            styles: styles.to_vec(),
        },
    );
}

fn split_lines(source: &[char]) -> Vec<&[char]> {
    let mut lines: Vec<&[char]> = Vec::new();
    let mut start: usize = 0;
    for (index, character) in source.iter().enumerate() {
        if *character == '\n' {
            lines.push(&source[start..index]);
            start = index + 1;
        }
    }
    lines.push(&source[start..]);
    lines
}

/// port: `/^(\d{1,2})\. (.*)$/u`. The dot of the original does not match line
/// terminators, so a line holding one is not a numbered item.
fn match_numbered_list_item(line: &[char]) -> Option<(u32, &[char])> {
    let digit_count = line
        .iter()
        .take(2)
        .take_while(|character| character.is_ascii_digit())
        .count();
    if digit_count == 0 || line.len() < digit_count + 2 {
        return None;
    }
    if line[digit_count] != '.' || line[digit_count + 1] != ' ' {
        return None;
    }
    let rest = &line[digit_count + 2..];
    if rest
        .iter()
        .any(|character| matches!(character, '\n' | '\r' | '\u{2028}' | '\u{2029}'))
    {
        return None;
    }
    let digits: String = line[..digit_count].iter().collect();
    let number = digits.parse::<u32>().ok()?;
    Some((number, rest))
}

fn character_at(source: &[char], index: usize) -> Option<char> {
    source.get(index).copied()
}

fn starts_with_at(source: &[char], needle: &str, index: usize) -> bool {
    let mut position = index;
    for character in needle.chars() {
        match source.get(position) {
            Some(current) if *current == character => position += 1,
            _ => return false,
        }
    }
    true
}

fn starts_with_at_ignore_case(source: &[char], needle: &str, index: usize) -> bool {
    let mut position = index;
    for character in needle.chars() {
        match source.get(position) {
            Some(current) if current.eq_ignore_ascii_case(&character) => position += 1,
            _ => return false,
        }
    }
    true
}

fn index_of(source: &[char], needle: &str, from: usize) -> Option<usize> {
    let needle: Vec<char> = needle.chars().collect();
    if needle.is_empty() {
        return Some(from);
    }
    if source.len() < needle.len() {
        return None;
    }
    (from..=source.len() - needle.len())
        .find(|position| source[*position..*position + needle.len()] == needle[..])
}

// ---------------------------------------------------------------------------
// URL parsing
//
// port: `new URL(raw)` for the `http`/`https` schemes, including the
// serialization rules of the WHATWG URL Standard that Node.js implements.
// ---------------------------------------------------------------------------

const PUNYCODE_BASE: u32 = 36;
const PUNYCODE_TMIN: u32 = 1;
const PUNYCODE_TMAX: u32 = 26;
const PUNYCODE_SKEW: u32 = 38;
const PUNYCODE_DAMP: u32 = 700;
const PUNYCODE_INITIAL_BIAS: u32 = 72;
const PUNYCODE_INITIAL_N: u32 = 128;

struct ParsedUrl {
    protocol: String,
    href: String,
}

fn parse_url(raw: &str) -> Option<ParsedUrl> {
    // The URL parser removes every ASCII tab or newline, then strips the
    // leading and trailing C0 controls and spaces.
    let without_newlines: String = raw
        .chars()
        .filter(|character| !matches!(character, '\t' | '\n' | '\r'))
        .collect();
    let input = without_newlines.trim_matches(|character: char| character <= '\u{20}');

    let (scheme, rest) = split_scheme(input)?;
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let default_port = if scheme == "http" { 80 } else { 443 };

    // port: the "special authority ignore slashes state" skips every slash.
    let rest = rest.trim_start_matches(['/', '\\']);

    let authority_end = rest.find(['/', '\\', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let remainder = &rest[authority_end..];

    let (userinfo, host_port) = match authority.rfind('@') {
        Some(position) => (Some(&authority[..position]), &authority[position + 1..]),
        None => (None, authority),
    };

    let (host, port) = split_host_and_port(host_port)?;
    let host = parse_host(&host)?;
    let port = match port {
        Some("") => None,
        Some(text) => {
            if !text.chars().all(|character| character.is_ascii_digit()) {
                return None;
            }
            let value = text.parse::<u32>().ok()?;
            if value > 65535 {
                return None;
            }
            if value == default_port {
                None
            } else {
                Some(value)
            }
        }
        None => None,
    };

    let (path, query, fragment) = split_path_query_fragment(remainder);
    let encoded_path = serialize_path(parse_path(path));

    let mut href = String::new();
    href.push_str(&scheme);
    href.push_str("://");
    if let Some(userinfo) = userinfo {
        let (user, password) = match userinfo.find(':') {
            Some(position) => (&userinfo[..position], Some(&userinfo[position + 1..])),
            None => (userinfo, None),
        };
        // port: the credentials are written only when the user name or the
        // password is not empty.
        if !user.is_empty() || password.is_some_and(|password| !password.is_empty()) {
            href.push_str(&encode_userinfo(user));
            if let Some(password) = password {
                if !password.is_empty() {
                    href.push(':');
                    href.push_str(&encode_userinfo(password));
                }
            }
            href.push('@');
        }
    }
    href.push_str(&host);
    if let Some(port) = port {
        href.push(':');
        href.push_str(&port.to_string());
    }
    href.push_str(&encoded_path);
    if let Some(query) = query {
        href.push('?');
        href.push_str(&encode_query(query));
    }
    if let Some(fragment) = fragment {
        href.push('#');
        href.push_str(&encode_fragment(fragment));
    }

    Some(ParsedUrl {
        protocol: format!("{scheme}:"),
        href,
    })
}

fn split_scheme(input: &str) -> Option<(String, &str)> {
    let colon = input.find(':')?;
    let scheme = &input[..colon];
    let mut characters = scheme.chars();
    let first = characters.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    if !characters
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.'))
    {
        return None;
    }
    Some((scheme.to_ascii_lowercase(), &input[colon + 1..]))
}

fn split_host_and_port(host_port: &str) -> Option<(String, Option<&str>)> {
    if let Some(rest) = host_port.strip_prefix('[') {
        let end = rest.find(']')?;
        let host = format!("[{}]", &rest[..end]);
        let after = &rest[end + 1..];
        let port = match after.strip_prefix(':') {
            Some(port) => Some(port),
            None => {
                if !after.is_empty() {
                    return None;
                }
                None
            }
        };
        return Some((host, port));
    }

    match host_port.find(':') {
        Some(position) => Some((
            host_port[..position].to_string(),
            Some(&host_port[position + 1..]),
        )),
        None => Some((host_port.to_string(), None)),
    }
}

/// port: the path state of the URL parser. The single dot and the double dot
/// segments are dropped, as the standard does.
fn parse_path(path: &str) -> Vec<String> {
    let mut segments: Vec<String> = Vec::new();
    let mut buffer = String::new();

    // The path start state consumes the leading separator of a special URL.
    let rest = match path.strip_prefix('/') {
        Some(rest) => rest,
        None => match path.strip_prefix('\\') {
            Some(rest) => rest,
            None => {
                // The path is empty: the path state appends the empty segment.
                segments.push(String::new());
                return segments;
            }
        },
    };

    for character in rest.chars() {
        if character == '/' || character == '\\' {
            // A backslash is a separator for a special scheme.
            flush_path_segment(&mut segments, &buffer, true);
            buffer.clear();
            continue;
        }
        if in_path_set(character) {
            push_percent_encoded(&mut buffer, character);
        } else {
            buffer.push(character);
        }
    }
    flush_path_segment(&mut segments, &buffer, false);
    segments
}

fn flush_path_segment(segments: &mut Vec<String>, buffer: &str, terminated: bool) {
    if is_double_dot_path_segment(buffer) {
        segments.pop();
        if !terminated {
            segments.push(String::new());
        }
        return;
    }
    if is_single_dot_path_segment(buffer) {
        if !terminated {
            segments.push(String::new());
        }
        return;
    }
    segments.push(buffer.to_string());
}

/// port: a single dot URL path segment is "." or an ASCII case-insensitive
/// match for "%2e".
fn is_single_dot_path_segment(segment: &str) -> bool {
    segment == "." || segment.eq_ignore_ascii_case("%2e")
}

/// port: a double dot URL path segment is ".." or an ASCII case-insensitive
/// match for ".%2e", "%2e." or "%2e%2e".
fn is_double_dot_path_segment(segment: &str) -> bool {
    segment == ".."
        || segment.eq_ignore_ascii_case(".%2e")
        || segment.eq_ignore_ascii_case("%2e.")
        || segment.eq_ignore_ascii_case("%2e%2e")
}

fn serialize_path(segments: Vec<String>) -> String {
    if segments.is_empty() {
        return String::new();
    }
    let mut path = String::from("/");
    path.push_str(&segments.join("/"));
    path
}

fn split_path_query_fragment(remainder: &str) -> (&str, Option<&str>, Option<&str>) {
    let (path, rest) = match remainder.find(['?', '#']) {
        Some(position) => (&remainder[..position], &remainder[position..]),
        None => (remainder, ""),
    };

    if let Some(stripped) = rest.strip_prefix('?') {
        return match stripped.find('#') {
            Some(position) => (
                path,
                Some(&stripped[..position]),
                Some(&stripped[position + 1..]),
            ),
            None => (path, Some(stripped), None),
        };
    }
    if let Some(stripped) = rest.strip_prefix('#') {
        return (path, None, Some(stripped));
    }
    (path, None, None)
}

/// port: the host parser of the WHATWG URL Standard.
fn parse_host(input: &str) -> Option<String> {
    if input.starts_with('[') {
        // An IPv6 host keeps its brackets.
        let inner = input.strip_prefix('[')?.strip_suffix(']')?;
        let address = parse_ipv6(inner)?;
        return Some(format!("[{address}]"));
    }
    if input.is_empty() {
        return None;
    }

    // port: the host is percent-decoded before the domain parsing.
    let decoded = percent_decode(input)?;
    let ascii_domain = domain_to_ascii(&decoded)?;
    if ends_in_a_number(&ascii_domain) {
        return parse_ipv4(&ascii_domain);
    }
    Some(ascii_domain)
}

/// port: the domain parser with `beStrict` set to false. An ASCII domain is
/// lowercased whatever the outcome of the IDNA processing is.
fn domain_to_ascii(domain: &str) -> Option<String> {
    let result = if domain.is_ascii() {
        domain.to_ascii_lowercase()
    } else {
        let mut labels: Vec<String> = Vec::new();
        for label in domain.split('.') {
            // port: the code points that UTS#46 ignores are removed from the
            // label before the punycode encoding.
            let cleaned: String = label
                .chars()
                .filter(|character| !is_ignored_in_domain(*character))
                .collect();
            if cleaned.chars().any(is_disallowed_in_domain) {
                // The IDNA processing fails, so the whole URL is invalid.
                return None;
            }
            labels.push(domain_label_to_ascii(&cleaned));
        }
        labels.join(".")
    };

    if result.is_empty() {
        return None;
    }
    if result.chars().any(is_forbidden_domain_code_point) {
        return None;
    }
    Some(result)
}

/// port: the code points that UTS#46 maps to nothing.
fn is_ignored_in_domain(character: char) -> bool {
    matches!(
        character,
        '\u{ad}' | '\u{180b}' | '\u{180c}' | '\u{180d}' | '\u{200b}' | '\u{feff}'
    )
}

/// port: the code points that make the IDNA processing of UTS#46 fail. The
/// unassigned code points are not reproduced: the standard library does not
/// expose the Unicode categories.
fn is_disallowed_in_domain(character: char) -> bool {
    if character.is_control() {
        return true;
    }
    matches!(
        character,
        '\u{2028}' | '\u{2029}' | '\u{fffd}'
            | '\u{200c}'
            | '\u{200d}'
            | '\u{600}'..='\u{605}'
            | '\u{61c}'
            | '\u{6dd}'
            | '\u{70f}'
            | '\u{890}'..='\u{891}'
            | '\u{8e2}'
            | '\u{180e}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206f}'
            | '\u{fff9}'..='\u{fffb}'
            | '\u{e0001}'
            | '\u{e0020}'..='\u{e007f}'
    )
}

/// port: a forbidden domain code point is a forbidden host code point, a C0
/// control, U+0025 (%) or U+007F DELETE.
fn is_forbidden_domain_code_point(character: char) -> bool {
    matches!(
        character,
        '\0' | '\t'
            | '\n'
            | '\r'
            | ' '
            | '#'
            | '/'
            | ':'
            | '<'
            | '>'
            | '?'
            | '@'
            | '['
            | '\\'
            | ']'
            | '^'
            | '|'
            | '%'
            | '\u{7f}'
    ) || (character as u32) <= 0x1f
}

/// port: the percent-decoding of a byte sequence.
fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut decoded: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte != b'%' || index + 2 >= bytes.len() {
            decoded.push(byte);
            index += 1;
            continue;
        }
        let high = (bytes[index + 1] as char).to_digit(16);
        let low = (bytes[index + 2] as char).to_digit(16);
        match (high, low) {
            (Some(high), Some(low)) => {
                decoded.push((high * 16 + low) as u8);
                index += 3;
            }
            _ => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(decoded).ok()
}

/// port: the IPv6 parser of the WHATWG URL Standard.
fn parse_ipv6(input: &str) -> Option<String> {
    let characters: Vec<char> = input.chars().collect();
    let mut address: [u16; 8] = [0; 8];
    let mut piece_index: usize = 0;
    let mut compress: Option<usize> = None;
    let mut pointer: usize = 0;

    if characters.first() == Some(&':') {
        if characters.get(1) != Some(&':') {
            return None;
        }
        pointer = 2;
        piece_index = 1;
        compress = Some(piece_index);
    }

    while pointer < characters.len() {
        if piece_index == 8 {
            return None;
        }
        if characters[pointer] == ':' {
            if compress.is_some() {
                return None;
            }
            pointer += 1;
            piece_index += 1;
            compress = Some(piece_index);
            continue;
        }

        let mut value: u16 = 0;
        let mut length: usize = 0;
        while length < 4 {
            let Some(digit) = characters.get(pointer).and_then(|c| c.to_digit(16)) else {
                break;
            };
            value = value * 16 + digit as u16;
            pointer += 1;
            length += 1;
        }

        if characters.get(pointer) == Some(&'.') {
            // port: an IPv4 address at the end of an IPv6 address.
            if length == 0 {
                return None;
            }
            pointer -= length;
            if piece_index > 6 {
                return None;
            }
            let mut numbers_seen: usize = 0;
            while pointer < characters.len() {
                let mut ipv4_piece: Option<u16> = None;
                if numbers_seen > 0 {
                    if characters[pointer] == '.' && numbers_seen < 4 {
                        pointer += 1;
                    } else {
                        return None;
                    }
                }
                if !characters
                    .get(pointer)
                    .is_some_and(|character| character.is_ascii_digit())
                {
                    return None;
                }
                while let Some(digit) = characters
                    .get(pointer)
                    .filter(|character| character.is_ascii_digit())
                    .and_then(|character| character.to_digit(10))
                {
                    match ipv4_piece {
                        None => ipv4_piece = Some(digit as u16),
                        Some(0) => return None,
                        Some(current) => {
                            let next = current * 10 + digit as u16;
                            if next > 255 {
                                return None;
                            }
                            ipv4_piece = Some(next);
                        }
                    }
                    pointer += 1;
                }
                let piece = ipv4_piece?;
                address[piece_index] = address[piece_index] * 256 + piece;
                numbers_seen += 1;
                if numbers_seen == 2 || numbers_seen == 4 {
                    piece_index += 1;
                }
            }
            if numbers_seen != 4 {
                return None;
            }
            break;
        }

        if characters.get(pointer) == Some(&':') {
            pointer += 1;
            if pointer >= characters.len() {
                return None;
            }
        } else if pointer < characters.len() {
            return None;
        }

        address[piece_index] = value;
        piece_index += 1;
    }

    if let Some(compress) = compress {
        let mut swaps = piece_index - compress;
        piece_index = 7;
        while piece_index != 0 && swaps > 0 {
            address.swap(piece_index, compress + swaps - 1);
            piece_index -= 1;
            swaps -= 1;
        }
    } else if piece_index != 8 {
        return None;
    }

    Some(serialize_ipv6(&address))
}

/// port: the serialization of an IPv6 address.
fn serialize_ipv6(address: &[u16; 8]) -> String {
    let mut compress: Option<usize> = None;
    let mut longest = 0usize;
    let mut current = 0usize;
    let mut start = 0usize;
    for (index, piece) in address.iter().enumerate() {
        if *piece == 0 {
            if current == 0 {
                start = index;
            }
            current += 1;
            if current > longest {
                longest = current;
                compress = Some(start);
            }
        } else {
            current = 0;
        }
    }
    // port: a single zero piece is not compressed.
    if longest < 2 {
        compress = None;
    }

    let mut output = String::new();
    let mut ignore_zero = false;
    for (index, piece) in address.iter().enumerate() {
        if ignore_zero && *piece == 0 {
            continue;
        }
        if ignore_zero {
            ignore_zero = false;
        }
        if compress == Some(index) {
            output.push_str(if index == 0 { "::" } else { ":" });
            ignore_zero = true;
            continue;
        }
        output.push_str(&format!("{piece:x}"));
        if index != 7 {
            output.push(':');
        }
    }
    output
}

/// port: the "ends in a number" checker.
fn ends_in_a_number(input: &str) -> bool {
    let mut parts: Vec<&str> = input.split('.').collect();
    if parts.last() == Some(&"") {
        parts.pop();
    }
    let Some(last) = parts.last() else {
        return false;
    };
    if !last.is_empty() && last.chars().all(|character| character.is_ascii_digit()) {
        return true;
    }
    parse_ipv4_number(last).is_some()
}

/// port: the IPv4 parser of the WHATWG URL Standard.
fn parse_ipv4(input: &str) -> Option<String> {
    let mut parts: Vec<&str> = input.split('.').collect();
    if parts.last() == Some(&"") && parts.len() > 1 {
        parts.pop();
    }
    if parts.len() > 4 {
        return None;
    }

    let mut numbers: Vec<u64> = Vec::new();
    for part in &parts {
        let (value, _validation_error) = parse_ipv4_number(part)?;
        numbers.push(value);
    }
    let (last, rest) = numbers.split_last()?;
    if rest.iter().any(|number| *number > 255) {
        return None;
    }
    if *last >= 256u64.pow(5 - numbers.len() as u32) {
        return None;
    }

    let mut ipv4 = *last;
    for (counter, number) in (0_u32..).zip(rest.iter()) {
        ipv4 += number * 256u64.pow(3 - counter);
    }

    Some(format!(
        "{}.{}.{}.{}",
        (ipv4 >> 24) & 0xff,
        (ipv4 >> 16) & 0xff,
        (ipv4 >> 8) & 0xff,
        ipv4 & 0xff
    ))
}

/// port: the IPv4 number parser. It returns the number and whether the
/// original was written in a hexadecimal or octal notation.
fn parse_ipv4_number(input: &str) -> Option<(u64, bool)> {
    if input.is_empty() {
        return None;
    }

    let mut validation_error = false;
    let mut radix = 10u32;
    let mut rest = input;
    if input.len() >= 2 && (input.starts_with("0x") || input.starts_with("0X")) {
        validation_error = true;
        radix = 16;
        rest = &input[2..];
    } else if let Some(stripped) = input.strip_prefix('0') {
        validation_error = true;
        radix = 8;
        rest = stripped;
    }
    if rest.is_empty() {
        return Some((0, validation_error));
    }

    let mut value: u64 = 0;
    for character in rest.chars() {
        let digit = character.to_digit(radix)?;
        value = value.checked_mul(radix as u64)?.checked_add(digit as u64)?;
    }
    Some((value, validation_error))
}

fn domain_label_to_ascii(label: &str) -> String {
    if label.is_ascii() {
        return label.to_ascii_lowercase();
    }
    // port: the mapping step of UTS#46 is approximated with the Unicode
    // lowercase mapping. The compatibility decomposition of the standard (the
    // full width forms, the ligatures) is not reproduced.
    let mapped: String = label.chars().flat_map(char::to_lowercase).collect();
    match punycode_encode(&mapped) {
        Some(encoded) => encoded,
        None => label.to_ascii_lowercase(),
    }
}

/// port: the Punycode encoding of RFC 3492, used for the non ASCII labels of a
/// domain. The Unicode mapping of UTS#46 is not applied.
fn punycode_encode(label: &str) -> Option<String> {
    let code_points: Vec<u32> = label.chars().map(u32::from).collect();
    let mut output = String::from("xn--");
    let mut basic: Vec<u32> = Vec::new();
    for code_point in &code_points {
        if *code_point < PUNYCODE_INITIAL_N {
            // port: the mapping of UTS#46 lowercases the ASCII letters of the
            // label, even when the label holds other code points.
            let character = char::from_u32(*code_point).unwrap_or('\u{0}');
            basic.push(u32::from(character.to_ascii_lowercase()));
        }
    }
    let basic_count = basic.len();
    for code_point in &basic {
        output.push(char::from_u32(*code_point)?);
    }
    if basic_count > 0 {
        output.push('-');
    }

    let mut handled = basic_count;
    let mut n = PUNYCODE_INITIAL_N;
    let mut delta: u32 = 0;
    let mut bias = PUNYCODE_INITIAL_BIAS;

    while handled < code_points.len() {
        // port: `m` is the minimum code point of the whole input that is
        // greater than or equal to `n`.
        let m = *code_points
            .iter()
            .filter(|code_point| **code_point >= n)
            .min()?;
        delta = delta.checked_add((m - n).checked_mul((handled + 1) as u32)?)?;
        n = m;

        for code_point in &code_points {
            if *code_point < n {
                delta = delta.checked_add(1)?;
            }
            if *code_point != n {
                continue;
            }
            let mut q = delta;
            let mut k = PUNYCODE_BASE;
            loop {
                let t = if k <= bias {
                    PUNYCODE_TMIN
                } else if k >= bias + PUNYCODE_TMAX {
                    PUNYCODE_TMAX
                } else {
                    k - bias
                };
                if q < t {
                    break;
                }
                output.push(punycode_digit(t + (q - t) % (PUNYCODE_BASE - t)));
                q = (q - t) / (PUNYCODE_BASE - t);
                k += PUNYCODE_BASE;
            }
            output.push(punycode_digit(q));
            bias = punycode_adapt(delta, (handled + 1) as u32, handled == basic_count);
            delta = 0;
            handled += 1;
        }

        delta = delta.checked_add(1)?;
        n = n.checked_add(1)?;
    }

    Some(output)
}

fn punycode_digit(value: u32) -> char {
    if value < 26 {
        char::from(b'a' + value as u8)
    } else {
        char::from(b'0' + (value - 26) as u8)
    }
}

fn punycode_adapt(delta: u32, number_of_points: u32, first_time: bool) -> u32 {
    let mut delta = if first_time {
        delta / PUNYCODE_DAMP
    } else {
        delta / 2
    };
    delta += delta / number_of_points.max(1);
    let mut k = 0;
    while delta > ((PUNYCODE_BASE - PUNYCODE_TMIN) * PUNYCODE_TMAX) / 2 {
        delta /= PUNYCODE_BASE - PUNYCODE_TMIN;
        k += PUNYCODE_BASE;
    }
    k + (((PUNYCODE_BASE - PUNYCODE_TMIN + 1) * delta) / (delta + PUNYCODE_SKEW))
}

fn is_c0_control_percent_encoded(character: char) -> bool {
    (character as u32) <= 0x1f || (character as u32) > 0x7e
}

fn in_path_set(character: char) -> bool {
    is_c0_control_percent_encoded(character)
        || matches!(character, ' ' | '"' | '<' | '>' | '^' | '`' | '{' | '}')
}

fn in_query_set(character: char) -> bool {
    is_c0_control_percent_encoded(character) || matches!(character, ' ' | '"' | '\'' | '<' | '>')
}

fn in_fragment_set(character: char) -> bool {
    is_c0_control_percent_encoded(character) || matches!(character, ' ' | '"' | '<' | '>' | '`')
}

fn in_userinfo_set(character: char) -> bool {
    is_c0_control_percent_encoded(character)
        || matches!(
            character,
            ' ' | '"'
                | '<'
                | '>'
                | '^'
                | '`'
                | '{'
                | '}'
                | '/'
                | ':'
                | ';'
                | '='
                | '@'
                | '['
                | '\\'
                | ']'
                | '|'
        )
}

fn encode_query(query: &str) -> String {
    let mut result = String::new();
    for character in query.chars() {
        if in_query_set(character) {
            push_percent_encoded(&mut result, character);
        } else {
            result.push(character);
        }
    }
    result
}

fn encode_fragment(fragment: &str) -> String {
    let mut result = String::new();
    for character in fragment.chars() {
        if in_fragment_set(character) {
            push_percent_encoded(&mut result, character);
        } else {
            result.push(character);
        }
    }
    result
}

fn encode_userinfo(userinfo: &str) -> String {
    let mut result = String::new();
    for character in userinfo.chars() {
        if in_userinfo_set(character) {
            push_percent_encoded(&mut result, character);
        } else {
            result.push(character);
        }
    }
    result
}

fn push_percent_encoded(output: &mut String, character: char) {
    let mut buffer = [0u8; 4];
    for byte in character.encode_utf8(&mut buffer).as_bytes() {
        output.push('%');
        output.push_str(&format!("{byte:02X}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The expectations of this module were taken from the `URL` class of
    /// Node.js, which is the runtime of the frozen oracle.
    fn href(raw: &str) -> Option<String> {
        parse_url(raw).map(|url| url.href)
    }

    #[test]
    fn serializes_the_urls_the_way_node_does() {
        assert_eq!(
            href("https://example.com/a_path?q=~raw~").as_deref(),
            Some("https://example.com/a_path?q=~raw~")
        );
        assert_eq!(
            href("https://example.com").as_deref(),
            Some("https://example.com/")
        );
        assert_eq!(
            href("HTTPS://Example.COM/Path").as_deref(),
            Some("https://example.com/Path")
        );
        assert_eq!(
            href("https://example.com:443/x").as_deref(),
            Some("https://example.com/x")
        );
        assert_eq!(
            href("http://example.com:80/x").as_deref(),
            Some("http://example.com/x")
        );
        assert_eq!(
            href("https://user:pass@example.com/x").as_deref(),
            Some("https://user:pass@example.com/x")
        );
        assert_eq!(
            href("https://example.com/a?b=c#d").as_deref(),
            Some("https://example.com/a?b=c#d")
        );
        assert_eq!(
            href("https://example.com/a b").as_deref(),
            Some("https://example.com/a%20b")
        );
        assert_eq!(
            href("https://example.com/ä").as_deref(),
            Some("https://example.com/%C3%A4")
        );
        assert_eq!(
            href("https://example.com/a_(b)").as_deref(),
            Some("https://example.com/a_(b)")
        );
        assert_eq!(
            href("https://example.com/a%2zb").as_deref(),
            Some("https://example.com/a%2zb")
        );
        assert_eq!(
            href("http://example.com\\a").as_deref(),
            Some("http://example.com/a")
        );
    }

    #[test]
    fn drops_the_dot_segments_of_the_path() {
        assert_eq!(
            href("https://example.com/a/../b").as_deref(),
            Some("https://example.com/b")
        );
        assert_eq!(
            href("https://example.com/.").as_deref(),
            Some("https://example.com/")
        );
        assert_eq!(
            href("https://example.com/a/./b").as_deref(),
            Some("https://example.com/a/b")
        );
    }

    #[test]
    fn writes_the_credentials_only_when_there_are_some() {
        assert_eq!(href("http://@Ann").as_deref(), Some("http://ann/"));
        assert_eq!(href("http://:x@Ann").as_deref(), Some("http://:x@ann/"));
        assert_eq!(
            href("http://user:@Ann").as_deref(),
            Some("http://user@ann/")
        );
    }

    #[test]
    fn parses_the_ipv4_and_the_ipv6_hosts() {
        assert_eq!(
            href("https://192.168.1.1/x").as_deref(),
            Some("https://192.168.1.1/x")
        );
        assert_eq!(href("https://1.2.3.4.5/x"), None);
        assert_eq!(href("https://example.com:99999/x"), None);
        assert_eq!(href("https://[::1]/x").as_deref(), Some("https://[::1]/x"));
        assert_eq!(
            href("https://[0:0::1]/x").as_deref(),
            Some("https://[::1]/x")
        );
        assert_eq!(
            href("https://[::ffff:192.168.1.1]/x").as_deref(),
            Some("https://[::ffff:c0a8:101]/x")
        );
    }

    #[test]
    fn encodes_the_internationalized_hosts() {
        assert_eq!(
            href("https://exämple.com/").as_deref(),
            Some("https://xn--exmple-cua.com/")
        );
        assert_eq!(
            punycode_encode("exämple").as_deref(),
            Some("xn--exmple-cua")
        );
        assert_eq!(punycode_encode("bücher").as_deref(), Some("xn--bcher-kva"));
    }

    #[test]
    fn knows_the_whitespace_and_the_word_characters_of_javascript() {
        assert!(is_whitespace(' '));
        assert!(is_whitespace('\u{feff}'));
        assert!(!is_whitespace('\u{85}'));
        assert!(is_word_character('a'));
        assert!(is_word_character('7'));
        assert!(!is_word_character('_'));
        assert!(!is_word_character('😀'));
    }
}
