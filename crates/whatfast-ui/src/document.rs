//! The document of a message: its blocks of text and its diagrams.
//!
//! [`whatfast_core::markdown`] decides what a message is — a heading, a list,
//! a table, a fence — and [`crate::render`] decides where the bubble goes.
//! This module draws one part at a time: the text with the view of text of
//! the window, which selects and copies the words, and a diagram with boxes
//! and arrows of its own, which the view of text cannot draw.
//!
//! The height of a document is measured here too, and with the same numbers
//! that draw it: the conversation is a virtual list, and a row whose measured
//! height differs from the drawn one makes the list jump.

use gpui::{
    Div, ElementId, FontWeight, Hsla, ParentElement as _, Pixels, SharedString, Styled as _,
    Window, div, px, rems,
};
use gpui_component::{
    highlighter::HighlightTheme,
    text::{TextView, TextViewStyle},
    v_flex,
};
use whatfast_core::diagrams::{self, Diagram, LaidArrow, LaidBox, Shape};
use whatfast_core::markdown::{self, Block, Part};
use whatfast_core::whatsapp_formatting::WhatsAppParseOptions;

/// The air between two blocks of a document, as a part of the text size.
pub(crate) const BLOCK_GAP: f32 = 0.45;

/// The longest message that is drawn as a document.
///
/// The blocks of a message are read on every frame, and a message of a
/// hundred thousand characters would drag the conversation: past this size
/// the bubble is the plain one, with the whole text to select and copy.
const MAX_DOCUMENT_CHARS: usize = 20_000;

/// The air inside the frame of a diagram, in pixels before the scale.
const DIAGRAM_PADDING: f32 = 8.;

/// The smallest height of a diagram, so an empty one is not a hole.
const DIAGRAM_MIN_HEIGHT: f32 = 24.;

/// The size of the label of an arrow of a diagram, as a part of the scale.
const DIAGRAM_LABEL_SIZE: f32 = 10.;

/// The size of the text inside a box of a diagram, as a part of the scale.
const DIAGRAM_BOX_SIZE: f32 = 12.;

/// The size of the attributes of an entity, as a part of the scale.
const DIAGRAM_ATTRIBUTE_SIZE: f32 = 10.;

/// The height of a heading, as a multiple of the text of the message.
fn heading_factor(level: u8) -> f32 {
    match level {
        1 => 1.45,
        2 => 1.28,
        3 => 1.14,
        4 => 1.06,
        5 => 1.0,
        _ => 0.94,
    }
}

/// The colors and the sizes of a document, as the bubble has them.
#[derive(Clone, Copy)]
pub(crate) struct DocumentStyle {
    /// The scale of the interface.
    pub scale: f32,
    /// The size of the text of the message.
    pub font_size: Pixels,
    /// The color of the text.
    pub text: Hsla,
    /// The color of the secondary text: the labels of a diagram.
    pub muted: Hsla,
    /// The color of the borders and the lines of a diagram.
    pub border: Hsla,
    /// The background of a block of code and of a box of a diagram.
    pub surface: Hsla,
    /// The color of the accent: the labels of the relations.
    pub accent: Hsla,
    /// Whether the theme is a dark one.
    pub dark: bool,
}

impl DocumentStyle {
    /// The style of the view of text of one block of text.
    fn text_view(&self) -> TextViewStyle {
        let base = f32::from(self.font_size);
        let highlight = if self.dark {
            HighlightTheme::default_dark()
        } else {
            HighlightTheme::default_light()
        };
        let mut style = TextViewStyle::default()
            // The blocks of a message are close: the air of a document, not
            // the air of a page.
            .paragraph_gap(rems(BLOCK_GAP))
            .heading_font_size(move |level, _base| px(base * heading_factor(level)));
        style.heading_base_font_size = self.font_size;
        // A table wider than the bubble scrolls inside the block: cutting a
        // cell would lose the value the person wrote.
        let mut table = gpui::StyleRefinement::default();
        table.overflow.x = Some(gpui::Overflow::Scroll);
        style.table = table;
        style.highlight_theme = highlight;
        style.is_dark = self.dark;
        style
    }
}

/// The parts of a message, with the labels of the mentions of its chat.
pub(crate) fn parts(text: &str, mentions: Option<Vec<String>>) -> Option<Vec<Part>> {
    if text.len() > MAX_DOCUMENT_CHARS {
        return None;
    }
    let options = WhatsAppParseOptions { mentions };
    markdown::parts(text, &options)
}

/// Draws the parts of a message, one under the other.
pub(crate) fn render_document(id: &str, parts: &[Part], style: &DocumentStyle) -> Div {
    let mut column = v_flex().w_full();
    for (index, part) in parts.iter().enumerate() {
        match part {
            Part::Html(html) => {
                column = column.child(
                    TextView::html(
                        ElementId::Name(format!("document-block-{id}-{index}").into()),
                        html.clone(),
                    )
                    .selectable(true)
                    .text_size(style.font_size)
                    .text_color(style.text)
                    .style(style.text_view()),
                );
            }
            Part::Diagram(diagram) => {
                column = column.child(render_diagram(diagram, style, index));
            }
        }
    }
    column
}

/// Draws a diagram: a box for every node and a line for every arrow.
///
/// The layout comes from the core, in pixels before the scale: the same text
/// gives the same drawing on every frame, and the height the caller measures
/// is the one this element uses.
pub(crate) fn render_diagram(diagram: &Diagram, style: &DocumentStyle, _index: usize) -> Div {
    let layout = diagrams::layout(diagram);
    let scale = style.scale;
    let width = px(layout.width * scale);
    let height = px((layout.height * scale).max(DIAGRAM_MIN_HEIGHT));
    let mut frame = div()
        .w_full()
        .overflow_hidden()
        .rounded_md()
        .bg(style.surface)
        .border_1()
        .border_color(style.border)
        .p(px(DIAGRAM_PADDING * scale))
        .child(
            div().relative().w(width).h(height).child(
                div()
                    .relative()
                    .size_full()
                    // The arrows go under the boxes: a line never crosses
                    // the text of a node.
                    .children(
                        layout
                            .arrows
                            .iter()
                            .enumerate()
                            .map(|(arrow_index, arrow)| render_arrow(arrow, style, arrow_index)),
                    )
                    .children(
                        layout.boxes.iter().map(|node| render_box(node, style)),
                    ),
            ),
        );
    if layout.truncated {
        frame = frame.child(
            div()
                .text_xs()
                .text_color(style.muted)
                .child("The diagram is too big: the rest is not drawn."),
        );
    }
    frame
}

/// One box of a diagram.
fn render_box(node: &LaidBox, style: &DocumentStyle) -> Div {
    let scale = style.scale;
    let mut card = div()
        .absolute()
        .left(px(node.x * scale))
        .top(px(node.y * scale))
        .w(px(node.w * scale))
        .min_h(px(node.h * scale))
        .px(px(8. * scale))
        .py(px(5. * scale))
        .border_1()
        .border_color(style.border)
        .bg(style.surface)
        .justify_center()
        .items_center()
        .child(
            div()
                .text_size(px(DIAGRAM_BOX_SIZE * scale))
                .font_weight(FontWeight::MEDIUM)
                .text_color(style.text)
                .child(node.label.clone()),
        );
    card = match node.shape {
        Shape::Rect => card,
        Shape::Round => card.rounded_full(),
        Shape::Diamond | Shape::Column | Shape::Card => card.rounded_md(),
    };
    if matches!(node.shape, Shape::Column | Shape::Card) {
        card = card.font_weight(FontWeight::SEMIBOLD);
    }
    if !node.lines.is_empty() {
        card = card.child(
            v_flex()
                .w_full()
                .children(node.lines.iter().map(|line| {
                    div()
                        .text_size(px(DIAGRAM_ATTRIBUTE_SIZE * scale))
                        .text_color(style.muted)
                        .child(line.clone())
                })),
        );
    }
    card
}

/// One arrow of a diagram: its runs of line, its head and its label.
fn render_arrow(arrow: &LaidArrow, style: &DocumentStyle, index: usize) -> Div {
    let scale = style.scale;
    let mut layer = div().absolute().inset_0();
    for (segment_index, segment) in arrow.segments.iter().enumerate() {
        layer = layer.children(dashes(
            segment.from.x * scale,
            segment.from.y * scale,
            segment.to.x * scale,
            segment.to.y * scale,
            segment.dashed,
            style.border,
            (index, segment_index),
        ));
    }
    if let Some(head) = arrow.head {
        // The head of an arrow is the glyph of its direction: the window has
        // no way to turn a box, and a triangle of borders is not a line.
        let glyph = match arrow.segments.last() {
            Some(segment)
                if (segment.to.x - segment.from.x).abs()
                    > (segment.to.y - segment.from.y).abs() =>
            {
                if segment.to.x >= segment.from.x {
                    "▶"
                } else {
                    "◀"
                }
            }
            Some(segment) if segment.to.y >= segment.from.y => "▼",
            _ => "▲",
        };
        layer = layer.child(
            div()
                .absolute()
                .left(px(head.x * scale - 4.))
                .top(px(head.y * scale - 5.))
                .text_size(px(9. * scale))
                .text_color(style.border)
                .child(glyph),
        );
    }
    if let Some(at) = arrow.label_at {
        layer = layer.child(
            div()
                .absolute()
                .left(px(at.x * scale))
                .top(px(at.y * scale))
                .text_size(px(DIAGRAM_LABEL_SIZE * scale))
                .text_color(style.accent)
                .child(arrow.label.clone()),
        );
    }
    layer
}

/// The runs of a line, solid or dashed.
///
/// A dashed line is a row of short runs: the window has no stroke pattern,
/// and the runs keep the line where the layout put it.
fn dashes(
    from_x: f32,
    from_y: f32,
    to_x: f32,
    to_y: f32,
    dashed: bool,
    color: Hsla,
    _key: (usize, usize),
) -> Vec<Div> {
    let horizontal = (to_y - from_y).abs() < 0.5;
    let vertical = (to_x - from_x).abs() < 0.5;
    if !horizontal && !vertical {
        // Two right angles at most, so a diagonal is a rounding error: there
        // is nothing to draw for this run.
        return Vec::new();
    }
    let length = if horizontal {
        (to_x - from_x).abs()
    } else {
        (to_y - from_y).abs()
    };
    let start = if horizontal {
        from_x.min(to_x)
    } else {
        from_y.min(to_y)
    };
    let fixed = if horizontal { from_y } else { from_x };
    let mut runs = Vec::new();
    let mut offset = 0f32;
    let mut run = 0usize;
    while offset < length && run < 64 {
        let span = if dashed { 5. } else { length - offset };
        let span = span.min(length - offset);
        let mut piece = div().absolute().bg(color);
        piece = if horizontal {
            piece
                .left(px(start + offset))
                .top(px(fixed))
                .w(px(span))
                .h(px(1.))
        } else {
            piece
                .left(px(fixed))
                .top(px(start + offset))
                .w(px(1.))
                .h(px(span))
        };
        runs.push(piece);
        run += 1;
        offset += if dashed { 10. } else { span.max(1.) };
    }
    runs
}

/// The height of the document of a message, measured with the text system.
///
/// It returns `None` when the message has no block of a document: the caller
/// keeps the height of the plain bubble.
pub(crate) fn document_height(
    window: &mut Window,
    text: &str,
    width: Pixels,
    font_size: Pixels,
    scale: f32,
) -> Option<Pixels> {
    if text.len() > MAX_DOCUMENT_CHARS || !markdown::has_blocks(text) {
        return None;
    }
    let line_height = window.line_height();
    let base = font_size;
    let gap = px(f32::from(base) * BLOCK_GAP * scale);
    let mut height = px(0.);
    for block in markdown::parse(text) {
        if let Block::Code { language, text } = &block
            && matches!(language.to_ascii_lowercase().as_str(), "mermaid" | "flow")
            && let Some(diagram) = diagrams::parse(text)
        {
            let layout = diagrams::layout(&diagram);
            height += px((layout.height * scale).max(DIAGRAM_MIN_HEIGHT)
                + 2. * DIAGRAM_PADDING * scale
                + 2.)
                + gap;
            continue;
        }
        height += block_height(window, &block, width, base, line_height, scale) + gap;
    }
    Some(height)
}

/// The height of one block of text, shaped with the same width it is drawn.
fn block_height(
    window: &mut Window,
    block: &Block,
    width: Pixels,
    font_size: Pixels,
    line_height: Pixels,
    scale: f32,
) -> Pixels {
    let shape = |window: &mut Window, text: &str, size: Pixels, width: Pixels| {
        if text.is_empty() {
            return px(0.);
        }
        let shared: SharedString = text.into();
        let run = window.text_style().to_run(text.len());
        window
            .text_system()
            .shape_text(shared, size, &[run], Some(width), None)
            .map(|lines| {
                lines
                    .iter()
                    .fold(px(0.), |height, line| height + line.size(line_height).height)
            })
            .unwrap_or(line_height)
    };
    match block {
        Block::Paragraph(lines) => shape(window, &lines.join(" "), font_size, width),
        Block::Heading { level, text } => shape(
            window,
            text,
            px(f32::from(font_size) * heading_factor(*level)),
            width,
        ),
        Block::Code { text, .. } => {
            let lines = text.lines().count().max(1) as f32;
            px(lines * f32::from(line_height) * 0.9 + 16. * scale)
        }
        Block::List { items, .. } => {
            let indent = px(18. * scale);
            items.iter().fold(px(0.), |height, item| {
                height + shape(window, item, font_size, width - indent)
            })
        }
        Block::Quote(lines) => shape(window, &lines.join(" "), font_size, width - px(12. * scale)),
        Block::Table(table) => {
            let rows = table.rows.len() as f32 + 1.;
            px(rows * f32::from(line_height) + 8. * scale)
        }
        Block::Rule => line_height,
    }
}
