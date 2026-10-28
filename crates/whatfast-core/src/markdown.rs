//! The blocks of a message, above the inline format of WhatsApp.
//!
//! [`crate::whatsapp_formatting`] reads the marks a person types in WhatsApp —
//! bold, italic, strike, mono and links— inside one paragraph. This module adds
//! the blocks a message may carry around them: headings, lists, quotes, code
//! fences, tables and rules, so the window can draw a document inside a bubble
//! without a second parser for the text and without interpreting any markup.
//!
//! The parser is deliberately forgiving and never fails: a message is whatever
//! the person who wrote it wanted, and a malformed fence is a paragraph that
//! starts with three backticks. Every string it returns is inert text.

/// One block of a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    /// Text without a mark of its own: the lines of one paragraph.
    Paragraph(Vec<String>),
    /// A line that starts with one to six `#`.
    Heading {
        /// One for `#`, six for `######`.
        level: u8,
        /// The text after the marks.
        text: String,
    },
    /// A fence of three or more backticks, with the language it declares.
    Code {
        /// The word after the opening fence, empty when there is none.
        language: String,
        /// Every line between the fences, without the fences.
        text: String,
    },
    /// Consecutive lines that start with the same bullet or number.
    List {
        /// True for `1.` lists, false for `-`, `*` and `+`.
        ordered: bool,
        /// The first number of an ordered list, `1` when it does not say.
        start: u32,
        /// One line per item, with its own text.
        items: Vec<String>,
    },
    /// Consecutive lines that start with `>`.
    Quote(Vec<String>),
    /// A table: the header row, the rows and the alignment of each column.
    Table(Table),
    /// A line of three or more `-`, `*` or `_`.
    Rule,
}

/// A table of a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    /// The cells of the first row.
    pub header: Vec<String>,
    /// How each column lines up, from the row of dashes.
    pub align: Vec<Align>,
    /// The cells of every other row.
    pub rows: Vec<Vec<String>>,
}

/// How a column of a table lines up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    /// `:---`
    Left,
    /// `:---:`
    Center,
    /// `---:`
    Right,
    /// `---`, which renders as left.
    Default,
}

/// Reads a message as blocks.
///
/// The marks of WhatsApp stay inside the blocks: this parser does not touch
/// them, and the caller passes each block to
/// [`crate::whatsapp_formatting::parse_whatsapp_text`] when it draws it.
pub fn parse(text: &str) -> Vec<Block> {
    let lines: Vec<&str> = text.lines().collect();
    let mut blocks = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim();
        if trimmed.is_empty() {
            index += 1;
            continue;
        }
        if let Some(fence) = fence_of(trimmed) {
            let (block, next) = read_fence(&lines, index, fence);
            blocks.push(block);
            index = next;
            continue;
        }
        if let Some((level, heading)) = heading_of(trimmed) {
            blocks.push(Block::Heading {
                level,
                text: heading.trim().to_string(),
            });
            index += 1;
            continue;
        }
        if rule_of(trimmed) {
            blocks.push(Block::Rule);
            index += 1;
            continue;
        }
        if let Some(table) = table_at(&lines, index) {
            let (table, next) = table;
            blocks.push(Block::Table(table));
            index = next;
            continue;
        }
        if let Some((ordered, start, _)) = list_marker(trimmed) {
            let (block, next) = read_list(&lines, index, ordered, start);
            blocks.push(block);
            index = next;
            continue;
        }
        if quote_line(trimmed).is_some() {
            let (block, next) = read_quote(&lines, index);
            blocks.push(block);
            index = next;
            continue;
        }
        let (block, next) = read_paragraph(&lines, index);
        blocks.push(block);
        index = next;
    }
    blocks
}

/// Whether a message carries any block a person would call a document.
///
/// The window uses it to keep the plain bubbles plain: a message without a
/// heading, a fence, a list, a quote or a table is drawn exactly as before.
pub fn has_blocks(text: &str) -> bool {
    parse(text)
        .iter()
        .any(|block| !matches!(block, Block::Paragraph(_)))
}

/// The marks of a rule, drawn with box characters: the view of text of the
/// window has no line of its own, and a rule that is text is always visible.
const RULE: &str = "────────────────────────";

/// The HTML of a message, for the view of text of the window.
///
/// This is the only place that knows both parsers: the blocks of this module
/// and the inline marks of [`crate::whatsapp_formatting`]. Every string is
/// escaped, so a message can never inject markup.
///
/// It returns `None` when the message carries no block of a document, so the
/// caller keeps the plain bubble of a message with only text and marks.
pub fn to_html(
    text: &str,
    options: &crate::whatsapp_formatting::WhatsAppParseOptions,
) -> Option<String> {
    if !has_blocks(text) {
        return None;
    }
    let mut html = String::new();
    for block in parse(text) {
        push_block(&mut html, block, options);
    }
    Some(html)
}

/// One piece of the drawing of a message.
///
/// The window draws the blocks of text with its view of text, but a diagram
/// is boxes and arrows: the message is cut where a diagram starts.
#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    /// Blocks of text, as the HTML of [`to_html`].
    Html(String),
    /// A diagram of [`crate::diagrams`].
    Diagram(crate::diagrams::Diagram),
}

/// The pieces of a message, in the order they appear.
///
/// It returns `None` when the message carries no block of a document, so the
/// caller keeps the plain bubble. A fence of `mermaid` or `flow` that the
/// parser of diagrams does not understand stays a block of code: the window
/// shows what the person wrote, never an empty box.
pub fn parts(
    text: &str,
    options: &crate::whatsapp_formatting::WhatsAppParseOptions,
) -> Option<Vec<Part>> {
    if !has_blocks(text) {
        return None;
    }
    let mut parts: Vec<Part> = Vec::new();
    let mut html = String::new();
    let flush = |parts: &mut Vec<Part>, html: &mut String| {
        if !html.is_empty() {
            parts.push(Part::Html(std::mem::take(html)));
        }
    };
    for block in parse(text) {
        if let Block::Code { language, text } = &block
            && matches!(language.to_ascii_lowercase().as_str(), "mermaid" | "flow")
            && let Some(diagram) = crate::diagrams::parse(text)
        {
            flush(&mut parts, &mut html);
            parts.push(Part::Diagram(diagram));
            continue;
        }
        push_block(&mut html, block, options);
    }
    flush(&mut parts, &mut html);
    Some(parts)
}

/// Writes one block as HTML.
fn push_block(
    html: &mut String,
    block: Block,
    options: &crate::whatsapp_formatting::WhatsAppParseOptions,
) {
    match block {
        Block::Paragraph(lines) => {
            html.push_str("<p>");
            push_lines(html, &lines, options);
            html.push_str("</p>");
        }
        Block::Heading { level, text } => {
            let level = level.clamp(1, 6);
            html.push_str(&format!("<h{level}>"));
            push_inline(html, &text, options);
            html.push_str(&format!("</h{level}>"));
        }
        Block::Code { text, .. } => {
            html.push_str("<pre>");
            html.push_str(&escape_html(&text));
            html.push_str("</pre>");
        }
        Block::List {
            ordered,
            start,
            items,
        } => {
            if ordered {
                html.push_str(&format!("<ol start=\"{start}\">"));
            } else {
                html.push_str("<ul>");
            }
            for item in items {
                html.push_str("<li>");
                push_inline(html, &item, options);
                html.push_str("</li>");
            }
            html.push_str(if ordered { "</ol>" } else { "</ul>" });
        }
        Block::Quote(lines) => {
            html.push_str("<blockquote>");
            push_lines(html, &lines, options);
            html.push_str("</blockquote>");
        }
        Block::Table(table) => push_table(html, &table, options),
        Block::Rule => {
            html.push_str("<p>");
            html.push_str(RULE);
            html.push_str("</p>");
        }
    }
}

/// The width of every row of a table, in columns: a short row is filled with
/// empty cells so the columns of the grid stay where the header put them.
fn push_table(
    html: &mut String,
    table: &Table,
    options: &crate::whatsapp_formatting::WhatsAppParseOptions,
) {
    let columns = table
        .rows
        .iter()
        .map(Vec::len)
        .chain(std::iter::once(table.header.len()))
        .max()
        .unwrap_or(0);
    html.push_str("<table><thead><tr>");
    for column in 0..columns {
        html.push_str("<th>");
        push_cell(html, table.header.get(column), options);
        html.push_str("</th>");
    }
    html.push_str("</tr></thead><tbody>");
    for row in &table.rows {
        html.push_str("<tr>");
        for column in 0..columns {
            html.push_str("<td>");
            push_cell(html, row.get(column), options);
            html.push_str("</td>");
        }
        html.push_str("</tr>");
    }
    html.push_str("</tbody></table>");
}

/// One cell of a table. An empty cell carries a zero width space: the view of
/// text drops a cell with no children, and the column would lose its place.
fn push_cell(
    html: &mut String,
    cell: Option<&String>,
    options: &crate::whatsapp_formatting::WhatsAppParseOptions,
) {
    match cell.map(String::as_str) {
        Some(cell) if !cell.trim().is_empty() => push_inline(html, cell, options),
        _ => html.push_str("&#8203;"),
    }
}

/// The lines of a paragraph or of a quote, one break between them.
fn push_lines(
    html: &mut String,
    lines: &[String],
    options: &crate::whatsapp_formatting::WhatsAppParseOptions,
) {
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            html.push_str("<br>");
        }
        push_inline(html, line, options);
    }
}

/// One line of text with the inline marks of WhatsApp, as HTML.
fn push_inline(
    html: &mut String,
    text: &str,
    options: &crate::whatsapp_formatting::WhatsAppParseOptions,
) {
    let mut wrote_any = false;
    for block in crate::whatsapp_formatting::parse_whatsapp_text(text, options) {
        match block {
            crate::whatsapp_formatting::WhatsAppTextBlock::CodeBlock { text } => {
                html.push_str("<code>");
                html.push_str(&escape_html(&text));
                html.push_str("</code>");
                wrote_any = true;
            }
            crate::whatsapp_formatting::WhatsAppTextBlock::BlankLine => {}
            block => {
                for segment in block.segments() {
                    push_segment(html, segment);
                    wrote_any = true;
                }
            }
        }
    }
    if !wrote_any {
        html.push_str(&escape_html(text));
    }
}

/// One run of text with its styles, as HTML.
fn push_segment(html: &mut String, segment: &crate::whatsapp_formatting::WhatsAppInlineSegment) {
    use crate::whatsapp_formatting::{WhatsAppInlineSegment, WhatsAppInlineStyle};

    let body = match segment {
        WhatsAppInlineSegment::Link { text, href, .. } => {
            format!("<a href=\"{}\">{}</a>", escape_html(href), escape_html(text))
        }
        // A mention is not a link to anywhere: the label is what the person
        // reads, and the window does not open a chat from the bubble.
        WhatsAppInlineSegment::Mention { text, .. } => {
            format!("<a href=\"#\">{}</a>", escape_html(text))
        }
        WhatsAppInlineSegment::Text { text, .. } => escape_html(text),
    };
    let styles = segment.styles();
    let mut wrapped = body;
    // The order is the one of the reference: the bold outside, the code
    // inside, so the borders of the code stay inside the weight.
    if styles.contains(&WhatsAppInlineStyle::Monospace)
        || styles.contains(&WhatsAppInlineStyle::InlineCode)
    {
        wrapped = format!("<code>{wrapped}</code>");
    }
    if styles.contains(&WhatsAppInlineStyle::Strikethrough) {
        wrapped = format!("<del>{wrapped}</del>");
    }
    if styles.contains(&WhatsAppInlineStyle::Italic) {
        wrapped = format!("<em>{wrapped}</em>");
    }
    if styles.contains(&WhatsAppInlineStyle::Bold) {
        wrapped = format!("<strong>{wrapped}</strong>");
    }
    html.push_str(&wrapped);
}

/// The text a person typed, as inert HTML.
fn escape_html(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// The opening fence of a code block, when the line is one.
fn fence_of(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("```")?;
    Some(rest.trim())
}

/// Reads a fence and the lines that follow it, up to its closing pair.
fn read_fence(lines: &[&str], start: usize, language: &str) -> (Block, usize) {
    let mut text = Vec::new();
    let mut index = start + 1;
    while index < lines.len() {
        if lines[index].trim().starts_with("```") {
            index += 1;
            break;
        }
        text.push(lines[index]);
        index += 1;
    }
    (
        Block::Code {
            language: language.to_string(),
            text: text.join("\n"),
        },
        index,
    )
}

/// The level and the text of a heading, when the line is one.
fn heading_of(line: &str) -> Option<(u8, &str)> {
    let marks = line
        .chars()
        .take_while(|character| *character == '#')
        .count();
    if !(1..=6).contains(&marks) {
        return None;
    }
    let rest = line.get(marks..)?;
    // A heading needs the space; `#hola` is a paragraph that starts with one.
    let text = rest.strip_prefix(' ')?;
    Some((marks as u8, text))
}

/// Whether the line is a rule of three or more of the same character.
fn rule_of(line: &str) -> bool {
    let compact: String = line.chars().filter(|character| *character != ' ').collect();
    if compact.len() < 3 {
        return false;
    }
    let first = compact.chars().next().unwrap_or_default();
    matches!(first, '-' | '*' | '_') && compact.chars().all(|character| character == first)
}

/// The bullet or number of a list item, when the line is one.
fn list_marker(line: &str) -> Option<(bool, u32, &str)> {
    for bullet in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(bullet) {
            return Some((false, 1, rest));
        }
    }
    let digits: String = line.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    let rest = line.get(digits.len()..)?;
    let text = rest
        .strip_prefix(". ")
        .or_else(|| rest.strip_prefix(") "))?;
    Some((true, digits.parse().unwrap_or(1), text))
}

/// The text of a quote line, when the line is one.
fn quote_line(line: &str) -> Option<&str> {
    line.strip_prefix("> ").or_else(|| line.strip_prefix(">"))
}

/// Reads consecutive lines of the same list.
fn read_list(lines: &[&str], start: usize, ordered: bool, first: u32) -> (Block, usize) {
    let mut items = Vec::new();
    let mut index = start;
    while index < lines.len() {
        let Some((also_ordered, _, text)) = list_marker(lines[index].trim()) else {
            break;
        };
        if also_ordered != ordered {
            break;
        }
        items.push(text.to_string());
        index += 1;
    }
    (
        Block::List {
            ordered,
            start: first,
            items,
        },
        index,
    )
}

/// Reads consecutive lines of the same quote.
fn read_quote(lines: &[&str], start: usize) -> (Block, usize) {
    let mut quoted = Vec::new();
    let mut index = start;
    while index < lines.len() {
        let Some(text) = quote_line(lines[index].trim()) else {
            break;
        };
        quoted.push(text.trim_start().to_string());
        index += 1;
    }
    (Block::Quote(quoted), index)
}

/// Reads a paragraph: every line up to a blank one or up to a line that opens
/// a block of its own.
fn read_paragraph(lines: &[&str], start: usize) -> (Block, usize) {
    let mut text = Vec::new();
    let mut index = start;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if trimmed.is_empty() {
            break;
        }
        if index > start
            && (fence_of(trimmed).is_some()
                || heading_of(trimmed).is_some()
                || rule_of(trimmed)
                || list_marker(trimmed).is_some()
                || quote_line(trimmed).is_some()
                || table_at(lines, index).is_some())
        {
            break;
        }
        text.push(lines[index].to_string());
        index += 1;
    }
    (Block::Paragraph(text), index)
}

/// Reads a table, when the line opens one: a header row and a row of dashes.
fn table_at(lines: &[&str], start: usize) -> Option<(Table, usize)> {
    let header = cells_of(lines.get(start)?)?;
    let dashes = cells_of(lines.get(start + 1)?)?;
    if dashes.is_empty() || !dashes.iter().all(|cell| is_dash_cell(cell)) {
        return None;
    }
    let align = dashes.iter().map(|cell| align_of(cell)).collect();
    let mut rows = Vec::new();
    let mut index = start + 2;
    while index < lines.len() {
        let Some(cells) = cells_of(lines[index]) else {
            break;
        };
        if cells.is_empty() {
            break;
        }
        rows.push(cells);
        index += 1;
    }
    Some((
        Table {
            header,
            align,
            rows,
        },
        index,
    ))
}

/// The cells of a table line, when the line is one.
fn cells_of(line: &str) -> Option<Vec<String>> {
    let trimmed = line.trim();
    if !trimmed.contains('|') {
        return None;
    }
    let inner = trimmed.trim_start_matches('|').trim_end_matches('|');
    Some(
        inner
            .split('|')
            .map(|cell| cell.trim().to_string())
            .collect(),
    )
}

/// Whether a cell of the row of dashes is one.
fn is_dash_cell(cell: &str) -> bool {
    let trimmed = cell.trim();
    let trimmed = trimmed.trim_start_matches(':').trim_end_matches(':');
    !trimmed.is_empty() && trimmed.chars().all(|character| character == '-')
}

/// How a cell of the row of dashes lines its column up.
fn align_of(cell: &str) -> Align {
    let trimmed = cell.trim();
    match (trimmed.starts_with(':'), trimmed.ends_with(':')) {
        (true, true) => Align::Center,
        (true, false) => Align::Left,
        (false, true) => Align::Right,
        (false, false) => Align::Default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_message_is_one_paragraph() {
        let blocks = parse("hola, ¿cómo va?");
        assert_eq!(
            blocks,
            vec![Block::Paragraph(vec!["hola, ¿cómo va?".into()])]
        );
        assert!(!has_blocks("hola, ¿cómo va?"));
    }

    #[test]
    fn the_lines_of_a_paragraph_stay_together() {
        let blocks = parse("una línea\notra línea\n\nsegundo párrafo");
        assert_eq!(
            blocks,
            vec![
                Block::Paragraph(vec!["una línea".into(), "otra línea".into()]),
                Block::Paragraph(vec!["segundo párrafo".into()]),
            ]
        );
    }

    #[test]
    fn a_fence_keeps_its_language_and_its_lines() {
        let blocks = parse("antes\n```rust\nfn main() {}\n```\ndespués");
        assert_eq!(
            blocks,
            vec![
                Block::Paragraph(vec!["antes".into()]),
                Block::Code {
                    language: "rust".into(),
                    text: "fn main() {}".into(),
                },
                Block::Paragraph(vec!["después".into()]),
            ]
        );
        assert!(has_blocks("```rust\nfn main() {}\n```"));
    }

    #[test]
    fn a_fence_that_never_closes_ends_with_the_message() {
        let blocks = parse("```\nuno\ndos");
        assert_eq!(
            blocks,
            vec![Block::Code {
                language: String::new(),
                text: "uno\ndos".into(),
            }]
        );
    }

    #[test]
    fn headings_need_the_space_after_the_marks() {
        assert_eq!(
            parse("# Título"),
            vec![Block::Heading {
                level: 1,
                text: "Título".into(),
            }]
        );
        assert_eq!(
            parse("### Tres"),
            vec![Block::Heading {
                level: 3,
                text: "Tres".into(),
            }]
        );
        // Seven marks are not a heading, and neither is a hash without space.
        assert!(matches!(
            parse("####### siete").as_slice(),
            [Block::Paragraph(_)]
        ));
        assert!(matches!(parse("#hola").as_slice(), [Block::Paragraph(_)]));
    }

    #[test]
    fn a_list_groups_the_lines_of_the_same_kind() {
        let blocks = parse("- uno\n- dos\n- tres");
        assert_eq!(
            blocks,
            vec![Block::List {
                ordered: false,
                start: 1,
                items: vec!["uno".into(), "dos".into(), "tres".into()],
            }]
        );
    }

    #[test]
    fn an_ordered_list_keeps_its_first_number() {
        let blocks = parse("3. tres\n4. cuatro");
        assert_eq!(
            blocks,
            vec![Block::List {
                ordered: true,
                start: 3,
                items: vec!["tres".into(), "cuatro".into()],
            }]
        );
    }

    #[test]
    fn a_quote_groups_its_lines() {
        let blocks = parse("> dijo\n> que sí");
        assert_eq!(
            blocks,
            vec![Block::Quote(vec!["dijo".into(), "que sí".into()])]
        );
    }

    #[test]
    fn a_table_reads_its_header_its_alignment_and_its_rows() {
        let blocks = parse("| a | b |\n|:--|--:|\n| 1 | 2 |\n| 3 | 4 |");
        let Some(Block::Table(table)) = blocks.first() else {
            panic!("expected a table, got {blocks:?}");
        };
        assert_eq!(table.header, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(table.align, vec![Align::Left, Align::Right]);
        assert_eq!(
            table.rows,
            vec![
                vec!["1".to_string(), "2".to_string()],
                vec!["3".to_string(), "4".to_string()],
            ]
        );
    }

    #[test]
    fn a_pipe_line_without_the_row_of_dashes_is_a_paragraph() {
        let blocks = parse("| a | b |\n| 1 | 2 |");
        assert!(matches!(blocks.as_slice(), [Block::Paragraph(_)]));
    }

    #[test]
    fn a_rule_is_a_line_of_the_same_mark() {
        assert_eq!(parse("---"), vec![Block::Rule]);
        assert_eq!(parse("***"), vec![Block::Rule]);
        assert_eq!(parse("_ _ _"), vec![Block::Rule]);
        // Two dashes are not a rule.
        assert!(matches!(parse("--").as_slice(), [Block::Paragraph(_)]));
    }

    #[test]
    fn the_marks_of_whatsapp_are_left_for_the_other_parser() {
        let blocks = parse("*negrita* y _itálica_");
        assert_eq!(
            blocks,
            vec![Block::Paragraph(vec!["*negrita* y _itálica_".into()])]
        );
    }

    #[test]
    fn a_document_has_blocks() {
        assert!(has_blocks("# t\n\ntexto"));
        assert!(has_blocks("- uno\n- dos"));
        assert!(has_blocks("| a |\n|---|\n| 1 |"));
        assert!(!has_blocks("solo *negrita* y un link https://ejemplo.com"));
    }

    use crate::whatsapp_formatting::WhatsAppParseOptions;
    use crate::{diagrams::Diagram, markdown::Part, markdown::parts};

    fn html(text: &str) -> String {
        to_html(text, &WhatsAppParseOptions::default()).unwrap_or_default()
    }

    #[test]
    fn a_plain_message_has_no_html_of_blocks() {
        assert_eq!(to_html("hola", &WhatsAppParseOptions::default()), None);
    }

    #[test]
    fn every_block_becomes_its_tag() {
        assert!(html("# Título").starts_with("<h1>Título</h1>"));
        assert_eq!(html("## Dos"), "<h2>Dos</h2>");
        assert_eq!(html("- uno\n- dos"), "<ul><li>uno</li><li>dos</li></ul>");
        assert_eq!(html("3. tres"), "<ol start=\"3\"><li>tres</li></ol>");
        assert_eq!(html("> cita"), "<blockquote>cita</blockquote>");
        assert_eq!(html("```\nfn main() {}\n```"), "<pre>fn main() {}</pre>");
        assert!(html("---").starts_with("<p>──"));
    }

    #[test]
    fn the_inline_marks_of_whatsapp_are_html_now() {
        let rendered = html("# *negrita* y _itálica_");
        assert!(rendered.contains("<strong>negrita</strong>"), "{rendered}");
        assert!(rendered.contains("<em>itálica</em>"), "{rendered}");
    }

    #[test]
    fn a_table_keeps_its_columns_when_a_row_is_short() {
        let rendered = html("| a | b |\n|---|---|\n| 1 |\n| 2 | 3 |");
        assert!(rendered.starts_with("<table><thead><tr><th>a</th><th>b</th></tr></thead>"));
        // The empty cell of the second row is a zero width space, not a gap.
        assert!(rendered.contains("<tr><td>1</td><td>&#8203;</td></tr>"), "{rendered}");
        assert!(rendered.contains("<tr><td>2</td><td>3</td></tr>"), "{rendered}");
    }

    #[test]
    fn a_message_cannot_inject_markup() {
        let rendered = html("# <script>alert('x')</script>");
        assert!(!rendered.contains("<script>"), "{rendered}");
        assert!(rendered.contains("&lt;script&gt;"), "{rendered}");
    }

    #[test]
    fn a_message_with_a_diagram_is_cut_where_the_diagram_starts() {
        let parts = parts(
            "# Red\n```mermaid\nflowchart TD\n A --> B\n```\ndespués",
            &WhatsAppParseOptions::default(),
        )
        .unwrap();
        assert_eq!(parts.len(), 3, "{parts:?}");
        assert!(matches!(&parts[0], Part::Html(html) if html.starts_with("<h1>Red</h1>")));
        assert!(matches!(&parts[1], Part::Diagram(Diagram::Flow(chart)) if chart.nodes.len() == 2));
        assert!(matches!(&parts[2], Part::Html(html) if html.contains("después")));
    }

    #[test]
    fn a_diagram_the_parser_does_not_know_stays_a_block_of_code() {
        let parts = parts(
            "```mermaid\nnot a diagram\n```",
            &WhatsAppParseOptions::default(),
        )
        .unwrap();
        assert_eq!(parts.len(), 1);
        assert!(matches!(&parts[0], Part::Html(html) if html.contains("<pre>not a diagram</pre>")));
    }

    #[test]
    fn a_link_inside_a_block_stays_a_link() {
        let rendered = html("- https://ejemplo.com/x");
        assert!(
            rendered.contains("<a href=\"https://ejemplo.com/x\">"),
            "{rendered}"
        );
    }
}
