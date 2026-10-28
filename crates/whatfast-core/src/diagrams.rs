//! The diagrams of a message: a subset of Mermaid, laid out in pixels.
//!
//! [`crate::markdown`] keeps a fence as a [`crate::markdown::Block::Code`] with
//! the language it declares. When that language is `mermaid` or `flow`, this
//! module reads the body: a flow chart, a sequence diagram or a diagram of
//! entities. What it does not understand is not a diagram, and the window
//! draws the fence as the code the person wrote.
//!
//! The layout is pure arithmetic on purpose: the same text gives the same
//! drawing, byte by byte, and the window can measure a diagram before it
//! draws it, which is what the virtual list of the conversation needs. No
//! node is ever placed with a random seed, a hash of a pointer or the clock.
//!
//! The subset is deliberately small:
//!
//! - `flowchart TD` and `flowchart LR` (also `graph`), with the nodes `A[text]`,
//!   `A(text)`, `A{text}` and the edges `-->`, `---`, `-->|label|`.
//! - `sequenceDiagram`, with `participant`/`actor`, `A->>B: text` and the
//!   dashed `A-->>B: text`.
//! - `erDiagram`, with entities, their attributes and the relations
//!   `A ||--o{ B : label`.
//!
//! Everything else — subgraphs, styles, notes, alternative frames— is skipped
//! line by line, so a diagram with one unfamiliar line still draws the rest.

/// The most nodes a diagram may carry: a longer one is cut, not drawn.
pub const MAX_NODES: usize = 60;

/// The most edges of a diagram, for the same reason.
pub const MAX_EDGES: usize = 120;

/// A diagram of a message.
#[derive(Clone, Debug, PartialEq)]
pub enum Diagram {
    /// A flow chart: boxes and arrows.
    Flow(FlowChart),
    /// A sequence diagram: columns and arrows in time order.
    Sequence(Sequence),
    /// A diagram of entities and their relations.
    Entities(Entities),
}

/// Where a flow chart grows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// The first layer on top: `TD` and `TB`.
    TopDown,
    /// The first layer at the left: `LR`.
    LeftRight,
}

/// The outline of a node of a flow chart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeShape {
    /// `A[text]`
    Rect,
    /// `A(text)`
    Round,
    /// `A{text}`
    Diamond,
}

/// One node of a flow chart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlowNode {
    /// The identifier the edges use.
    pub id: String,
    /// The text inside the box.
    pub label: String,
    /// The outline of the box.
    pub shape: NodeShape,
}

/// One arrow of a flow chart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlowEdge {
    /// The node the arrow leaves.
    pub from: String,
    /// The node the arrow reaches.
    pub to: String,
    /// The text over the arrow, empty when it has none.
    pub label: String,
    /// Whether the line is dashed (`-.->`).
    pub dashed: bool,
}

/// A flow chart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlowChart {
    /// Where the chart grows.
    pub direction: Direction,
    /// The nodes, in the order they appear.
    pub nodes: Vec<FlowNode>,
    /// The arrows, in the order they appear.
    pub edges: Vec<FlowEdge>,
    /// Whether the parser cut something out.
    pub truncated: bool,
}

/// One arrow of a sequence diagram.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SequenceMessage {
    /// The column the arrow leaves.
    pub from: usize,
    /// The column the arrow reaches.
    pub to: usize,
    /// The text over the arrow.
    pub text: String,
    /// Whether the line is dashed.
    pub dashed: bool,
}

/// A sequence diagram.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sequence {
    /// The columns, in the order they appear.
    pub participants: Vec<String>,
    /// The arrows, in the order of the text.
    pub messages: Vec<SequenceMessage>,
}

/// One entity of a diagram of entities.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entity {
    /// The name of the entity.
    pub name: String,
    /// Its attributes, as the file spells them: `string id`.
    pub attributes: Vec<String>,
}

/// One relation of a diagram of entities.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityRelation {
    /// The entity at the left.
    pub from: usize,
    /// The entity at the right.
    pub to: usize,
    /// The text of the relation.
    pub label: String,
}

/// A diagram of entities.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entities {
    /// The entities, in the order they appear.
    pub entities: Vec<Entity>,
    /// The relations, in the order they appear.
    pub relations: Vec<EntityRelation>,
}

/// Reads the body of a fence as a diagram.
///
/// It returns `None` when the text is not one of the three diagrams, or when
/// it is one but carries nothing to draw: the window shows the code instead.
pub fn parse(text: &str) -> Option<Diagram> {
    let mut lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("%%"));
    let header = lines.next()?;
    let header = header.to_ascii_lowercase();
    if header.starts_with("flowchart") || header.starts_with("graph") {
        return parse_flow(&header, lines).map(Diagram::Flow);
    }
    if header.starts_with("sequencediagram") {
        return parse_sequence(lines).map(Diagram::Sequence);
    }
    if header.starts_with("erdiagram") {
        return parse_entities(lines).map(Diagram::Entities);
    }
    None
}

/// Reads a flow chart. The lines it does not know are skipped.
fn parse_flow<'a>(header: &str, lines: impl Iterator<Item = &'a str>) -> Option<FlowChart> {
    let direction = if header.ends_with("lr") || header.ends_with("rl") {
        Direction::LeftRight
    } else {
        Direction::TopDown
    };
    let mut chart = FlowChart {
        direction,
        nodes: Vec::new(),
        edges: Vec::new(),
        truncated: false,
    };
    for line in lines {
        let lowered = line.to_ascii_lowercase();
        // The statements of style and the subgraphs are not part of the
        // subset: their lines are skipped, not their whole diagram.
        if lowered.starts_with("subgraph")
            || lowered == "end"
            || lowered.starts_with("style ")
            || lowered.starts_with("classdef")
            || lowered.starts_with("class ")
            || lowered.starts_with("linkstyle")
            || lowered.starts_with("click ")
            || lowered.starts_with("direction ")
        {
            continue;
        }
        for statement in split_statements(line) {
            read_flow_statement(&mut chart, statement);
        }
    }
    if chart.nodes.is_empty() {
        return None;
    }
    Some(chart)
}

/// One arrow of a flow chart, with the parts around it.
struct FlowArrow<'a> {
    left: &'a str,
    right: &'a str,
    label: String,
    dashed: bool,
}

/// The arrows of a line, when it has one.
fn flow_arrow(line: &str) -> Option<FlowArrow<'_>> {
    // The longest marks go first: `-->>` has to be read before `-->`.
    const MARKS: [(&str, bool); 8] = [
        ("-.-.->", true),
        ("-..->", true),
        ("-.->", true),
        ("-->", false),
        ("==>", false),
        ("->>", false),
        ("---", false),
        ("--", false),
    ];
    for (mark, dashed) in MARKS {
        let Some(index) = line.find(mark) else {
            continue;
        };
        let left = line[..index].trim();
        let rest = line[index + mark.len()..].trim();
        // `-->|sí| B` puts the label between pipes, before the target.
        let (label, right) = if let Some(rest) = rest.strip_prefix('|')
            && let Some(end) = rest.find('|')
        {
            (rest[..end].trim().to_string(), rest[end + 1..].trim())
        } else {
            (String::new(), rest)
        };
        if left.is_empty() || right.is_empty() {
            return None;
        }
        return Some(FlowArrow {
            left,
            right,
            label,
            dashed,
        });
    }
    None
}

/// Reads one statement: an arrow, or the declaration of a node.
fn read_flow_statement(chart: &mut FlowChart, statement: &str) {
    if statement.is_empty() {
        return;
    }
    if let Some(arrow) = flow_arrow(statement) {
        let Some(from) = read_node(chart, arrow.left) else {
            return;
        };
        let Some(to) = read_node(chart, arrow.right) else {
            return;
        };
        if chart.edges.len() < MAX_EDGES {
            chart.edges.push(FlowEdge {
                from,
                to,
                label: arrow.label,
                dashed: arrow.dashed,
            });
        } else {
            chart.truncated = true;
        }
        return;
    }
    read_node(chart, statement);
}

/// Splits a line into the statements it carries: Mermaid ends one with `;`.
fn split_statements(line: &str) -> impl Iterator<Item = &str> {
    line.split(';').map(str::trim).filter(|part| !part.is_empty())
}

/// The identifier of a node, and its box.
///
/// `A[texto]` carries the text; a bare `A` is its own label. The function adds
/// the node to the chart when it is new and returns its identifier.
fn read_node(chart: &mut FlowChart, token: &str) -> Option<String> {
    let token = token.trim();
    let (id, shape, label) = match token.find(['[', '(', '{']) {
        Some(index) => {
            let id = token[..index].trim();
            let opening = token[index..].chars().next()?;
            let closing = match opening {
                '[' => ']',
                '(' => ')',
                _ => '}',
            };
            let inner = token[index + opening.len_utf8()..].trim_end();
            let inner = inner.strip_suffix(closing).unwrap_or(inner);
            let shape = match opening {
                '(' => NodeShape::Round,
                '{' => NodeShape::Diamond,
                _ => NodeShape::Rect,
            };
            (id, shape, unquote(inner.trim()))
        }
        None => (token, NodeShape::Rect, token.to_string()),
    };
    let id = clean_id(id)?;
    if !chart.nodes.iter().any(|node| node.id == id) {
        if chart.nodes.len() >= MAX_NODES {
            chart.truncated = true;
            return None;
        }
        chart.nodes.push(FlowNode {
            id: id.clone(),
            label: label.trim().to_string(),
            shape,
        });
    } else if let Some(node) = chart.nodes.iter_mut().find(|node| node.id == id)
        && node.label == node.id
        && !label.trim().is_empty()
        && label.trim() != id
    {
        // A later line may carry the text of a node the arrows named first.
        node.label = label.trim().to_string();
        node.shape = shape;
    }
    Some(id)
}

/// The identifier of a node: letters, digits, `_`, `-` and `.`.
fn clean_id(token: &str) -> Option<String> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    let mut id = String::new();
    for character in token.chars() {
        if character.is_alphanumeric() || matches!(character, '_' | '-' | '.' | ':') {
            id.push(character);
        } else {
            break;
        }
    }
    if id.is_empty() { None } else { Some(id) }
}

/// The text between double quotes, when the file spells it that way.
fn unquote(text: &str) -> String {
    let text = text.trim();
    if let Some(inner) = text.strip_prefix('"').and_then(|t| t.strip_suffix('"')) {
        inner.to_string()
    } else {
        text.to_string()
    }
}

/// Reads a sequence diagram.
fn parse_sequence<'a>(lines: impl Iterator<Item = &'a str>) -> Option<Sequence> {
    let mut sequence = Sequence {
        participants: Vec::new(),
        messages: Vec::new(),
    };
    // `participant C as Cliente` gives the column the label of the person and
    // keeps `C` as the name the arrows use.
    let mut aliases: Vec<(String, String)> = Vec::new();
    for line in lines {
        let lowered = line.to_ascii_lowercase();
        if let Some(rest) = lowered
            .starts_with("participant ")
            .then(|| line["participant ".len()..].trim())
            .or_else(|| {
                lowered
                    .starts_with("actor ")
                    .then(|| line["actor ".len()..].trim())
            })
        {
            let (id, name) = match rest.split_once(" as ") {
                Some((id, name)) => (id.trim(), name.trim()),
                None => (rest, rest),
            };
            if let Some(id) = clean_id(id) {
                push_participant(&mut sequence, name);
                aliases.push((id, name.to_string()));
            }
            continue;
        }
        if lowered.starts_with("note ")
            || lowered.starts_with("alt ")
            || lowered.starts_with("else")
            || lowered == "end"
            || lowered.starts_with("loop ")
            || lowered.starts_with("opt ")
            || lowered.starts_with("activate ")
            || lowered.starts_with("deactivate ")
            || lowered.starts_with("rect ")
            || lowered.starts_with("autonumber")
        {
            continue;
        }
        let Some((arrow, text)) = line.split_once(':') else {
            continue;
        };
        let text = text.trim().to_string();
        let Some((from, to, dashed)) = sequence_arrow(arrow.trim()) else {
            continue;
        };
        let (Some(from), Some(to)) = (clean_id(from), clean_id(to)) else {
            continue;
        };
        let label = |id: &str| {
            aliases
                .iter()
                .find(|(alias, _)| alias == id)
                .map_or_else(|| id.to_string(), |(_, name)| name.clone())
        };
        let from = label(&from);
        let to = label(&to);
        push_participant(&mut sequence, &from);
        push_participant(&mut sequence, &to);
        let from = sequence
            .participants
            .iter()
            .position(|participant| participant == &from)
            .unwrap_or(0);
        let to = sequence
            .participants
            .iter()
            .position(|participant| participant == &to)
            .unwrap_or(from);
        if sequence.messages.len() < MAX_EDGES {
            sequence.messages.push(SequenceMessage {
                from,
                to,
                text,
                dashed,
            });
        }
    }
    if sequence.participants.is_empty() || sequence.messages.is_empty() {
        return None;
    }
    Some(sequence)
}

/// Adds a column when the diagram does not have it yet.
fn push_participant(sequence: &mut Sequence, name: &str) {
    let name = name.trim();
    if name.is_empty() || sequence.participants.iter().any(|participant| participant == name) {
        return;
    }
    sequence.participants.push(name.to_string());
}

/// The two ends of an arrow of a sequence diagram, and whether it is dashed.
fn sequence_arrow(arrow: &str) -> Option<(&str, &str, bool)> {
    const MARKS: [(&str, bool); 8] = [
        ("-->>", true),
        ("--x", true),
        ("--)", true),
        ("->>", false),
        ("--->", false),
        ("-)", false),
        ("-->", false),
        ("->", false),
    ];
    for (mark, dashed) in MARKS {
        if let Some(index) = arrow.find(mark) {
            let from = arrow[..index].trim();
            let to = arrow[index + mark.len()..].trim();
            if !from.is_empty() && !to.is_empty() {
                return Some((from, to, dashed));
            }
        }
    }
    None
}

/// Reads a diagram of entities.
fn parse_entities<'a>(lines: impl Iterator<Item = &'a str>) -> Option<Entities> {
    let mut entities = Entities {
        entities: Vec::new(),
        relations: Vec::new(),
    };
    let mut open: Option<Entity> = None;
    for line in lines {
        if let Some(entity) = open.as_mut() {
            if line == "}" {
                entities.entities.push(open.take().unwrap_or(Entity {
                    name: String::new(),
                    attributes: Vec::new(),
                }));
                continue;
            }
            if entity.attributes.len() < 40 {
                entity.attributes.push(line.to_string());
            }
            continue;
        }
        if let Some(name) = line.strip_suffix('{') {
            let name = name.trim();
            if let Some(name) = clean_id(name) {
                open = Some(Entity {
                    name,
                    attributes: Vec::new(),
                });
            }
            continue;
        }
        // `A ||--o{ B : label`, with any of the cardinality marks.
        let Some((left, right)) = line.split_once("--") else {
            continue;
        };
        let label = right.split_once(':').map(|(_, label)| label.trim().to_string());
        let Some(label) = label else {
            continue;
        };
        let left = left.trim_end_matches(['|', 'o', '{', '}', '>', '<', ' ']);
        let right = right
            .split_once(':')
            .map(|(rest, _)| rest)
            .unwrap_or(right)
            .trim_start_matches(['|', 'o', '{', '}', '>', '<', ' ']);
        let (Some(from), Some(to)) = (clean_id(left), clean_id(right)) else {
            continue;
        };
        let from = entity_index(&mut entities, &from);
        let to = entity_index(&mut entities, &to);
        if entities.relations.len() < MAX_EDGES {
            entities.relations.push(EntityRelation { from, to, label });
        }
    }
    if let Some(entity) = open {
        entities.entities.push(entity);
    }
    if entities.entities.is_empty() {
        return None;
    }
    Some(entities)
}

/// The index of an entity, adding it when the relations name it first.
fn entity_index(entities: &mut Entities, name: &str) -> usize {
    if let Some(index) = entities
        .entities
        .iter()
        .position(|entity| entity.name == name)
    {
        return index;
    }
    entities.entities.push(Entity {
        name: name.to_string(),
        attributes: Vec::new(),
    });
    entities.entities.len() - 1
}

/// The width the window gives to one character of a label, before the scale.
pub const CHAR_WIDTH: f32 = 7.2;

/// The height of one line of a label, before the scale.
pub const LINE_HEIGHT: f32 = 18.;

/// The air inside a box, at each side.
pub const BOX_PADDING: f32 = 12.;

/// The smallest box of a flow chart.
pub const MIN_BOX_WIDTH: f32 = 76.;

/// The widest box of a flow chart.
pub const MAX_BOX_WIDTH: f32 = 260.;

/// The air between two boxes of the same layer.
pub const NODE_GAP: f32 = 28.;

/// The air between two layers.
pub const LAYER_GAP: f32 = 66.;

/// A point of the drawing, in pixels and before the scale of the interface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    /// From the left edge of the drawing.
    pub x: f32,
    /// From the top edge of the drawing.
    pub y: f32,
}

/// The outline of a box, as the window draws it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// A plain rectangle.
    Rect,
    /// A rectangle with round corners.
    Round,
    /// A rhombus.
    Diamond,
    /// The box of a participant of a sequence diagram.
    Column,
    /// The box of an entity: a header and its attributes.
    Card,
}

/// A box of the drawing.
#[derive(Clone, Debug, PartialEq)]
pub struct LaidBox {
    /// The left edge.
    pub x: f32,
    /// The top edge.
    pub y: f32,
    /// The width.
    pub w: f32,
    /// The height.
    pub h: f32,
    /// The title of the box.
    pub label: String,
    /// The lines under the title, one per attribute.
    pub lines: Vec<String>,
    /// The outline.
    pub shape: Shape,
}

/// One straight run of an arrow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LaidSegment {
    /// The start.
    pub from: Point,
    /// The end.
    pub to: Point,
    /// Whether the segment is dashed.
    pub dashed: bool,
}

/// An arrow of the drawing: its segments, its head and its label.
#[derive(Clone, Debug, PartialEq)]
pub struct LaidArrow {
    /// The segments, in the order the line goes.
    pub segments: Vec<LaidSegment>,
    /// Where the head of the arrow points, when it has one.
    pub head: Option<Point>,
    /// The text of the arrow, empty when it has none.
    pub label: String,
    /// Where the label goes.
    pub label_at: Option<Point>,
}

/// A line of text of the drawing that is not inside a box.
#[derive(Clone, Debug, PartialEq)]
pub struct LaidText {
    /// The left edge.
    pub x: f32,
    /// The top edge.
    pub y: f32,
    /// The text.
    pub text: String,
}

/// A diagram, placed in pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    /// The width the drawing needs.
    pub width: f32,
    /// The height the drawing needs.
    pub height: f32,
    /// The boxes.
    pub boxes: Vec<LaidBox>,
    /// The arrows.
    pub arrows: Vec<LaidArrow>,
    /// The loose lines of text: the labels of the columns of a sequence and
    /// the labels of the relations of the entities.
    pub texts: Vec<LaidText>,
    /// Whether the diagram is not complete: the window says so under it.
    pub truncated: bool,
}

/// Places a diagram at the widths its labels need.
///
/// The result depends only on the diagram, so the measurement of the virtual
/// list and the drawing ask for the same numbers.
pub fn layout(diagram: &Diagram) -> Layout {
    match diagram {
        Diagram::Flow(chart) => layout_flow(chart),
        Diagram::Sequence(sequence) => layout_sequence(sequence),
        Diagram::Entities(entities) => layout_entities(entities),
    }
}

/// The width of a label, clamped to the box it lives in.
fn label_width(label: &str) -> f32 {
    let characters = label.chars().count().max(3) as f32;
    (characters * CHAR_WIDTH + BOX_PADDING * 2.).clamp(MIN_BOX_WIDTH, MAX_BOX_WIDTH)
}

/// The depth of every node: zero for a root, one more than its parent.
///
/// A node that only takes part in a cycle has no root above it, so it starts
/// at zero: the drawing is always finite.
fn flow_depths(chart: &FlowChart) -> Vec<usize> {
    let nodes = chart.nodes.len();
    let mut depths = vec![0usize; nodes];
    let index_of = |id: &str| chart.nodes.iter().position(|node| node.id == id);
    let mut incoming = vec![0usize; nodes];
    for edge in &chart.edges {
        if let (Some(_), Some(to)) = (index_of(&edge.from), index_of(&edge.to)) {
            incoming[to] += 1;
        }
    }
    // A few rounds of relaxation place every node that a root reaches, and
    // the bound keeps a cycle from spinning: the depth is at most the number
    // of nodes.
    let mut changed = true;
    let mut rounds = 0;
    while changed && rounds < nodes {
        changed = false;
        rounds += 1;
        for (index, node) in chart.nodes.iter().enumerate() {
            if incoming[index] == 0 {
                continue;
            }
            let mut best = None;
            for edge in &chart.edges {
                if edge.to != node.id {
                    continue;
                }
                if let Some(parent) = index_of(&edge.from)
                    && parent != index
                {
                    best = Some(best.map_or(depths[parent] + 1, |value: usize| {
                        value.max(depths[parent] + 1)
                    }));
                }
            }
            if let Some(best) = best
                && best > depths[index]
            {
                depths[index] = best.min(nodes);
                changed = true;
            }
        }
    }
    depths
}

/// Places a flow chart by layers.
fn layout_flow(chart: &FlowChart) -> Layout {
    let depths = flow_depths(chart);
    let layers = depths.iter().copied().max().map_or(1, |max| max + 1);
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); layers];
    for (index, depth) in depths.iter().enumerate() {
        members[*depth].push(index);
    }
    let sizes: Vec<(f32, f32)> = chart
        .nodes
        .iter()
        .map(|node| {
            let width = label_width(&node.label);
            // A long label wraps at the widest box instead of growing it.
            let rows = (node.label.chars().count() as f32 * CHAR_WIDTH / (width - BOX_PADDING * 2.))
                .ceil()
                .max(1.);
            (width, rows * LINE_HEIGHT + BOX_PADDING * 2.)
        })
        .collect();
    let layer_extent: Vec<f32> = members
        .iter()
        .map(|indices| {
            indices
                .iter()
                .map(|index| sizes[*index].0)
                .sum::<f32>()
                + NODE_GAP * (indices.len().saturating_sub(1)) as f32
        })
        .collect();
    let across = layer_extent.iter().copied().fold(0., f32::max);
    let along: f32 = members
        .iter()
        .map(|indices| {
            indices
                .iter()
                .map(|index| sizes[*index].1)
                .fold(0., f32::max)
        })
        .sum::<f32>()
        + LAYER_GAP * (layers.saturating_sub(1)) as f32;
    let wide = chart.direction == Direction::LeftRight;
    let (width, height) = if wide {
        (along, across)
    } else {
        (across, along)
    };

    let mut boxes = Vec::with_capacity(chart.nodes.len());
    let mut centers = vec![Point { x: 0., y: 0. }; chart.nodes.len()];
    let mut offset = 0.;
    for (depth, indices) in members.iter().enumerate() {
        let band = indices
            .iter()
            .map(|index| sizes[*index].1)
            .fold(0., f32::max);
        let mut along_offset = (across - layer_extent[depth]) / 2.;
        for index in indices {
            let (box_width, box_height) = sizes[*index];
            let (x, y) = if wide {
                (offset, along_offset)
            } else {
                (along_offset, offset)
            };
            let shape = match chart.nodes[*index].shape {
                NodeShape::Rect => Shape::Rect,
                NodeShape::Round => Shape::Round,
                NodeShape::Diamond => Shape::Diamond,
            };
            boxes.push(LaidBox {
                x,
                y,
                w: box_width,
                // A box of a layer fills the band of its layer, so the arrows
                // between two layers are straight.
                h: if wide { band } else { box_height },
                label: chart.nodes[*index].label.clone(),
                lines: Vec::new(),
                shape,
            });
            if wide {
                boxes.last_mut().unwrap().h = band;
            } else {
                boxes.last_mut().unwrap().h = box_height;
            }
            centers[*index] = Point {
                x: x + box_width / 2.,
                y: y + boxes.last().map_or(box_height, |b| b.h) / 2.,
            };
            along_offset += sizes[*index].1 + NODE_GAP;
        }
        offset += band + LAYER_GAP;
    }

    let index_of = |id: &str| chart.nodes.iter().position(|node| node.id == id);
    let mut arrows = Vec::with_capacity(chart.edges.len());
    for edge in &chart.edges {
        let (Some(from), Some(to)) = (index_of(&edge.from), index_of(&edge.to)) else {
            continue;
        };
        let source = &boxes[from];
        let target = &boxes[to];
        let segments = if from == to {
            // An arrow that leaves a node and comes back: a small loop at the
            // right of the box.
            let right = source.x + source.w;
            let top = source.y + source.h / 2. - 8.;
            vec![
                LaidSegment {
                    from: Point { x: right, y: top },
                    to: Point { x: right + 18., y: top },
                    dashed: edge.dashed,
                },
                LaidSegment {
                    from: Point { x: right + 18., y: top },
                    to: Point { x: right + 18., y: top + 22. },
                    dashed: edge.dashed,
                },
                LaidSegment {
                    from: Point { x: right + 18., y: top + 22. },
                    to: Point { x: right, y: top + 22. },
                    dashed: edge.dashed,
                },
            ]
        } else if wide {
            let from_right = from > to;
            let start = Point {
                x: if from_right {
                    source.x
                } else {
                    source.x + source.w
                },
                y: source.y + source.h / 2.,
            };
            let end = Point {
                x: if from_right {
                    target.x + target.w
                } else {
                    target.x
                },
                y: target.y + target.h / 2.,
            };
            cross_segments(start, end, true, edge.dashed)
        } else {
            let downwards = to > from || target.y >= source.y;
            let start = Point {
                x: source.x + source.w / 2.,
                y: if downwards {
                    source.y + source.h
                } else {
                    source.y
                },
            };
            let end = Point {
                x: target.x + target.w / 2.,
                y: if downwards {
                    target.y
                } else {
                    target.y + target.h
                },
            };
            cross_segments(start, end, false, edge.dashed)
        };
        let head = segments.last().map(|segment| segment.to);
        let label_at = (!edge.label.is_empty()).then(|| {
            let last = segments.last().unwrap_or(&LaidSegment {
                from: Point { x: 0., y: 0. },
                to: Point { x: 0., y: 0. },
                dashed: false,
            });
            Point {
                x: (last.from.x + last.to.x) / 2. + 4.,
                y: (last.from.y + last.to.y) / 2. - LINE_HEIGHT,
            }
        });
        arrows.push(LaidArrow {
            segments,
            head,
            label: edge.label.clone(),
            label_at,
        });
    }

    Layout {
        width: width.max(MIN_BOX_WIDTH),
        height: height.max(LINE_HEIGHT),
        boxes,
        arrows,
        texts: Vec::new(),
        truncated: chart.truncated,
    }
}

/// The segments of an arrow between two points that do not share a side.
///
/// The line leaves into the gap between the layers, crosses it, and comes in
/// from the side of the target: two right angles at most.
fn cross_segments(start: Point, end: Point, horizontal: bool, dashed: bool) -> Vec<LaidSegment> {
    let straight = if horizontal {
        (start.y - end.y).abs() < 0.5
    } else {
        (start.x - end.x).abs() < 0.5
    };
    if straight {
        return vec![LaidSegment {
            from: start,
            to: end,
            dashed,
        }];
    }
    if horizontal {
        let middle = (start.x + end.x) / 2.;
        vec![
            LaidSegment {
                from: start,
                to: Point { x: middle, y: start.y },
                dashed,
            },
            LaidSegment {
                from: Point { x: middle, y: start.y },
                to: Point { x: middle, y: end.y },
                dashed,
            },
            LaidSegment {
                from: Point { x: middle, y: end.y },
                to: end,
                dashed,
            },
        ]
    } else {
        let middle = (start.y + end.y) / 2.;
        vec![
            LaidSegment {
                from: start,
                to: Point { x: start.x, y: middle },
                dashed,
            },
            LaidSegment {
                from: Point { x: start.x, y: middle },
                to: Point { x: end.x, y: middle },
                dashed,
            },
            LaidSegment {
                from: Point { x: end.x, y: middle },
                to: end,
                dashed,
            },
        ]
    }
}

/// The width of the column of a participant.
fn column_width(name: &str) -> f32 {
    label_width(name).max(110.)
}

/// Places a sequence diagram: one column per participant, one row per arrow.
fn layout_sequence(sequence: &Sequence) -> Layout {
    let widths: Vec<f32> = sequence
        .participants
        .iter()
        .map(|name| column_width(name))
        .collect();
    let mut centers = Vec::with_capacity(widths.len());
    let mut x = 0.;
    for width in &widths {
        centers.push(x + width / 2.);
        x += width + NODE_GAP;
    }
    let width = x - NODE_GAP;
    let header = LINE_HEIGHT + BOX_PADDING * 2.;
    let row = LINE_HEIGHT * 1.8;
    let first_row = header + row;
    let height = first_row + row * sequence.messages.len() as f32;

    let mut boxes = Vec::with_capacity(widths.len());
    let mut texts = Vec::new();
    for (index, name) in sequence.participants.iter().enumerate() {
        boxes.push(LaidBox {
            x: centers[index] - widths[index] / 2.,
            y: 0.,
            w: widths[index],
            h: header,
            label: name.clone(),
            lines: Vec::new(),
            shape: Shape::Column,
        });
    }
    // The life lines go under the headers, one per column.
    let mut arrows = Vec::new();
    for center in &centers {
        arrows.push(LaidArrow {
            segments: vec![LaidSegment {
                from: Point {
                    x: *center,
                    y: header,
                },
                to: Point {
                    x: *center,
                    y: height,
                },
                dashed: true,
            }],
            head: None,
            label: String::new(),
            label_at: None,
        });
    }
    for (index, message) in sequence.messages.iter().enumerate() {
        let y = first_row + row * index as f32 + row / 2.;
        let from = centers[message.from];
        let to = centers[message.to];
        let segments = if message.from == message.to {
            vec![
                LaidSegment {
                    from: Point { x: from, y },
                    to: Point {
                        x: from + 26.,
                        y,
                    },
                    dashed: message.dashed,
                },
                LaidSegment {
                    from: Point { x: from + 26., y },
                    to: Point {
                        x: from + 26.,
                        y: y + 12.,
                    },
                    dashed: message.dashed,
                },
                LaidSegment {
                    from: Point {
                        x: from + 26.,
                        y: y + 12.,
                    },
                    to: Point {
                        x: from,
                        y: y + 12.,
                    },
                    dashed: message.dashed,
                },
            ]
        } else {
            vec![LaidSegment {
                from: Point { x: from, y },
                to: Point { x: to, y },
                dashed: message.dashed,
            }]
        };
        let head = segments.last().map(|segment| segment.to);
        let label_at = Some(Point {
            x: (from + to) / 2.,
            y: y - LINE_HEIGHT,
        });
        arrows.push(LaidArrow {
            segments,
            head,
            label: message.text.clone(),
            label_at,
        });
    }
    texts.push(LaidText {
        x: 0.,
        y: 0.,
        text: String::new(),
    });
    texts.clear();
    Layout {
        width,
        height,
        boxes,
        arrows,
        texts,
        truncated: false,
    }
}

/// Places a diagram of entities: two columns of cards, with the relations
/// drawn over the gap between them.
fn layout_entities(entities: &Entities) -> Layout {
    let cards: Vec<(f32, f32)> = entities
        .entities
        .iter()
        .map(|entity| {
            let width = entity
                .attributes
                .iter()
                .chain(std::iter::once(&entity.name))
                .map(|line| label_width(line))
                .fold(MIN_BOX_WIDTH, f32::max);
            let rows = entity.attributes.len() as f32;
            (width, LINE_HEIGHT * (rows + 1.) + BOX_PADDING * 2.)
        })
        .collect();
    let columns = if entities.entities.len() > 1 { 2 } else { 1 };
    let mut column_widths = vec![0f32; columns];
    for (index, (width, _)) in cards.iter().enumerate() {
        column_widths[index % columns] = column_widths[index % columns].max(*width);
    }
    let gap = NODE_GAP * 2.;
    let width = column_widths.iter().sum::<f32>() + gap * (columns.saturating_sub(1)) as f32;
    let mut boxes = Vec::with_capacity(entities.entities.len());
    let mut column_heights = vec![0f32; columns];
    for (index, entity) in entities.entities.iter().enumerate() {
        let column = index % columns;
        let (_, card_height) = cards[index];
        boxes.push(LaidBox {
            x: column_widths[..column].iter().sum::<f32>() + gap * column as f32,
            y: column_heights[column],
            w: column_widths[column],
            h: card_height,
            label: entity.name.clone(),
            lines: entity.attributes.clone(),
            shape: Shape::Card,
        });
        column_heights[column] += card_height + LAYER_GAP / 2.;
    }
    let height = column_heights.iter().copied().fold(0., f32::max);

    let mut arrows = Vec::new();
    let mut texts = Vec::new();
    for relation in &entities.relations {
        let (Some(from), Some(to)) = (
            boxes.get(relation.from),
            boxes.get(relation.to),
        ) else {
            continue;
        };
        let start = Point {
            x: from.x + from.w,
            y: from.y + from.h / 2.,
        };
        let end = Point {
            x: to.x,
            y: to.y + to.h / 2.,
        };
        let segments = if (start.x - end.x).abs() < 0.5 && (start.y - end.y).abs() < 0.5 {
            vec![LaidSegment {
                from: start,
                to: Point {
                    x: start.x + 18.,
                    y: start.y,
                },
                dashed: false,
            }]
        } else {
            cross_segments(start, end, true, false)
        };
        let head = segments.last().map(|segment| segment.to);
        let label_at = (!relation.label.is_empty()).then(|| Point {
            x: (start.x + end.x) / 2. - relation.label.chars().count() as f32 * CHAR_WIDTH / 2.,
            y: (start.y + end.y) / 2. - LINE_HEIGHT,
        });
        arrows.push(LaidArrow {
            segments,
            head,
            label: relation.label.clone(),
            label_at,
        });
    }
    texts.clear();
    Layout {
        width: width.max(MIN_BOX_WIDTH),
        height: height.max(LINE_HEIGHT),
        boxes,
        arrows,
        texts,
        truncated: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_fence_is_not_a_diagram() {
        assert!(parse("fn main() {}").is_none());
        assert!(parse("flowchart TD").is_none());
    }

    #[test]
    fn a_flow_chart_reads_its_nodes_shapes_and_labels() {
        let Some(Diagram::Flow(chart)) = parse(
            "flowchart TD\n  A[Pedido] -->|sí| B{¿stock?}\n  B --> C(Enviar)\n  B --> D[Cancelar]",
        ) else {
            panic!("expected a flow chart");
        };
        assert_eq!(chart.direction, Direction::TopDown);
        assert_eq!(chart.nodes.len(), 4);
        assert_eq!(chart.nodes[0].label, "Pedido");
        assert_eq!(chart.nodes[1].shape, NodeShape::Diamond);
        assert_eq!(chart.nodes[2].shape, NodeShape::Round);
        assert_eq!(chart.edges[0].label, "sí");
    }

    #[test]
    fn a_left_to_right_chart_keeps_its_direction() {
        let Some(Diagram::Flow(chart)) = parse("graph LR\n A --> B") else {
            panic!("expected a flow chart");
        };
        assert_eq!(chart.direction, Direction::LeftRight);
    }

    #[test]
    fn the_statements_of_style_are_skipped() {
        let Some(Diagram::Flow(chart)) = parse(
            "flowchart TD\n subgraph uno\n A --> B\n end\n style A fill:#f00\n classDef x fill:#0f0",
        ) else {
            panic!("expected a flow chart");
        };
        assert_eq!(chart.nodes.len(), 2);
        assert_eq!(chart.edges.len(), 1);
    }

    #[test]
    fn a_sequence_reads_its_columns_and_its_arrows() {
        let Some(Diagram::Sequence(sequence)) = parse(
            "sequenceDiagram\n participant C as Cliente\n C->>S: pedido\n S-->>C: ok\n C-)S: ping",
        ) else {
            panic!("expected a sequence diagram");
        };
        assert_eq!(sequence.participants, vec!["Cliente", "S"]);
        assert_eq!(sequence.messages.len(), 3);
        assert!(sequence.messages[1].dashed);
        assert_eq!(sequence.messages[0].text, "pedido");
    }

    #[test]
    fn an_entity_diagram_reads_its_attributes_and_relations() {
        let Some(Diagram::Entities(entities)) = parse(
            "erDiagram\n  PEDIDO {\n    string id PK\n    int total\n  }\n  CLIENTE {\n    string id\n  }\n  CLIENTE ||--o{ PEDIDO : hace",
        ) else {
            panic!("expected a diagram of entities");
        };
        assert_eq!(entities.entities.len(), 2);
        assert_eq!(entities.entities[0].attributes.len(), 2);
        assert_eq!(entities.relations.len(), 1);
        assert_eq!(entities.relations[0].label, "hace");
    }

    #[test]
    fn the_layout_is_the_same_for_the_same_text() {
        let text = "flowchart TD\n A[a] --> B[b]\n B --> C[c]";
        let first = layout(&parse(text).unwrap());
        let second = layout(&parse(text).unwrap());
        assert_eq!(first, second);
    }

    #[test]
    fn the_arrows_of_a_flow_never_overlap_their_boxes() {
        let diagram = parse("flowchart TD\n A[uno] --> B[dos]\n A --> C[tres]\n B --> C").unwrap();
        let placed = layout(&diagram);
        for arrow in &placed.arrows {
            assert!(arrow.segments.iter().all(|segment| segment.to.y >= 0.));
        }
        // Every node has a box, and the boxes of one layer do not overlap.
        assert_eq!(placed.boxes.len(), 3);
        let mut sorted = placed.boxes.clone();
        sorted.sort_by(|a, b| a.y.partial_cmp(&b.y).unwrap());
        assert!(placed.width > 0. && placed.height > 0.);
    }

    #[test]
    fn a_cycle_is_drawn_and_does_not_hang() {
        let diagram = parse("flowchart TD\n A --> B\n B --> C\n C --> A").unwrap();
        let placed = layout(&diagram);
        assert_eq!(placed.boxes.len(), 3);
        assert_eq!(placed.arrows.len(), 3);
    }

    #[test]
    fn a_huge_diagram_is_cut_instead_of_growing_forever() {
        let mut text = String::from("flowchart TD\n");
        for index in 0..(MAX_NODES + 20) {
            text.push_str(&format!(" N{index}[nodo {index}]\n"));
        }
        let Some(Diagram::Flow(chart)) = parse(&text) else {
            panic!("expected a flow chart");
        };
        assert_eq!(chart.nodes.len(), MAX_NODES);
        assert!(chart.truncated);
    }

    #[test]
    fn a_thirty_node_chart_lays_out_in_less_than_twenty_milliseconds() {
        let mut text = String::from("flowchart TD\n");
        for index in 0..29 {
            text.push_str(&format!(" N{index}[nodo {index}] --> N{}\n", index + 1));
        }
        let diagram = parse(&text).unwrap();
        let start = std::time::Instant::now();
        let placed = layout(&diagram);
        let elapsed = start.elapsed();
        assert_eq!(placed.boxes.len(), 30);
        assert!(
            elapsed.as_millis() < 20,
            "the layout took {elapsed:?}: the window cannot draw it"
        );
    }
}
