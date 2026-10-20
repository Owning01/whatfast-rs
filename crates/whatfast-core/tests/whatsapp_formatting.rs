//! Port of `tauri/tests/whatsapp-formatting.test.ts` and
//! `tauri/tests/whatsapp-formatting-spoiler.test.ts` (the frozen oracle).
//!
//! Every case of the original suites is here, one to one, with the same
//! names translated to snake_case and the same semantic assertions.

use whatfast_core::whatsapp_formatting::{
    WhatsAppInlineSegment, WhatsAppInlineStyle, WhatsAppParseOptions, WhatsAppTextBlock,
    WhatsAppTextBlockKind, parse_whatsapp_text,
};

const BOLD: WhatsAppInlineStyle = WhatsAppInlineStyle::Bold;
const ITALIC: WhatsAppInlineStyle = WhatsAppInlineStyle::Italic;
const STRIKETHROUGH: WhatsAppInlineStyle = WhatsAppInlineStyle::Strikethrough;
const MONOSPACE: WhatsAppInlineStyle = WhatsAppInlineStyle::Monospace;
const INLINE_CODE: WhatsAppInlineStyle = WhatsAppInlineStyle::InlineCode;
const SPOILER: WhatsAppInlineStyle = WhatsAppInlineStyle::Spoiler;

fn text(value: &str, styles: &[WhatsAppInlineStyle]) -> WhatsAppInlineSegment {
    WhatsAppInlineSegment::Text {
        text: value.to_string(),
        styles: styles.to_vec(),
    }
}

fn link(value: &str, href: &str, styles: &[WhatsAppInlineStyle]) -> WhatsAppInlineSegment {
    WhatsAppInlineSegment::Link {
        text: value.to_string(),
        href: href.to_string(),
        styles: styles.to_vec(),
    }
}

fn mention(value: &str, name: &str, styles: &[WhatsAppInlineStyle]) -> WhatsAppInlineSegment {
    WhatsAppInlineSegment::Mention {
        text: value.to_string(),
        mention: name.to_string(),
        styles: styles.to_vec(),
    }
}

fn parse(source: &str) -> Vec<WhatsAppTextBlock> {
    parse_whatsapp_text(source, &WhatsAppParseOptions::default())
}

fn parse_with_mentions(source: &str, mentions: &[&str]) -> Vec<WhatsAppTextBlock> {
    parse_whatsapp_text(
        source,
        &WhatsAppParseOptions {
            mentions: Some(mentions.iter().map(|name| (*name).to_string()).collect()),
        },
    )
}

/// port: `blocks[2]?.kind === "numbered-list-item" ? blocks[2].number : -1`.
fn number_of(block: &WhatsAppTextBlock) -> Option<u32> {
    match block {
        WhatsAppTextBlock::NumberedListItem { number, .. } => Some(*number),
        _ => None,
    }
}

#[test]
fn parses_whatsapp_inline_emphasis_and_nested_formatting() {
    let blocks = parse("*bold* _italic_ ~gone~ ```mono``` `inline` *_both_*");

    assert_eq!(
        blocks[0].segments(),
        vec![
            text("bold", &[BOLD]),
            text(" ", &[]),
            text("italic", &[ITALIC]),
            text(" ", &[]),
            text("gone", &[STRIKETHROUGH]),
            text(" ", &[]),
            text("mono", &[MONOSPACE]),
            text(" ", &[]),
            text("inline", &[INLINE_CODE]),
            text(" ", &[]),
            text("both", &[BOLD, ITALIC]),
        ]
    );
}

#[test]
fn parses_quotes_bullet_items_and_one_or_two_digit_numbered_items() {
    let blocks = parse("> *Important*\n- first\n1. one\n12. twelve\n123. not a list");

    assert_eq!(
        blocks
            .iter()
            .map(|block| block.kind())
            .collect::<Vec<WhatsAppTextBlockKind>>(),
        vec![
            WhatsAppTextBlockKind::Quote,
            WhatsAppTextBlockKind::BulletListItem,
            WhatsAppTextBlockKind::NumberedListItem,
            WhatsAppTextBlockKind::NumberedListItem,
            WhatsAppTextBlockKind::Paragraph,
        ]
    );
    assert_eq!(
        blocks[0],
        WhatsAppTextBlock::Quote {
            segments: vec![text("Important", &[BOLD])],
        }
    );
    assert_eq!(number_of(&blocks[2]), Some(1));
    assert_eq!(number_of(&blocks[3]), Some(12));
}

#[test]
fn parses_a_multiline_triple_backtick_region_as_a_code_block() {
    assert_eq!(
        parse("before\n```\nconst x = '<b>safe</b>';\n```\nafter"),
        vec![
            WhatsAppTextBlock::Paragraph {
                segments: vec![text("before", &[])],
            },
            WhatsAppTextBlock::CodeBlock {
                text: "const x = '<b>safe</b>';".to_string(),
            },
            WhatsAppTextBlock::Paragraph {
                segments: vec![text("after", &[])],
            },
        ]
    );
}

#[test]
fn keeps_malformed_and_in_word_delimiters_as_literal_text() {
    let blocks = parse("*open _ spaced _ foo_bar_baz mid*word*here ~open");

    assert_eq!(
        blocks[0].segments(),
        vec![text(
            "*open _ spaced _ foo_bar_baz mid*word*here ~open",
            &[]
        )]
    );
}

#[test]
fn preserves_url_syntax_and_trims_sentence_punctuation_from_the_link() {
    let blocks = parse("*See https://example.com/a_path?q=~raw~*, then https://example.com/a_(b).");

    assert_eq!(
        blocks[0].segments(),
        vec![
            text("See ", &[BOLD]),
            link(
                "https://example.com/a_path?q=~raw~",
                "https://example.com/a_path?q=~raw~",
                &[BOLD],
            ),
            text(", then ", &[]),
            link(
                "https://example.com/a_(b)",
                "https://example.com/a_(b)",
                &[],
            ),
            text(".", &[]),
        ]
    );
}

#[test]
fn mentions_are_atomic_can_inherit_formatting_and_do_not_match_inside_code_or_email() {
    let blocks = parse_with_mentions("*@Ann Lee* @Annabelle `@Ann` x@Ann", &["Ann", "@Ann Lee"]);

    assert_eq!(
        blocks[0].segments(),
        vec![
            mention("@Ann Lee", "Ann Lee", &[BOLD]),
            text(" @Annabelle ", &[]),
            text("@Ann", &[INLINE_CODE]),
            text(" x@Ann", &[]),
        ]
    );
}

#[test]
fn html_looking_input_is_returned_only_as_inert_text_tokens() {
    assert_eq!(
        parse("<img src=x onerror=alert(1)> *<script>x</script>*"),
        vec![WhatsAppTextBlock::Paragraph {
            segments: vec![
                text("<img src=x onerror=alert(1)> ", &[]),
                text("<script>x</script>", &[BOLD]),
            ],
        }]
    );
}

#[test]
fn parses_a_spoiler_delimited_by_double_pipes() {
    let blocks = parse("||hidden||");

    assert_eq!(blocks[0].segments(), vec![text("hidden", &[SPOILER])]);
}

#[test]
fn keeps_the_text_around_a_spoiler_unstyled() {
    let blocks = parse("before ||hidden|| after");

    assert_eq!(
        blocks[0].segments(),
        vec![
            text("before ", &[]),
            text("hidden", &[SPOILER]),
            text(" after", &[]),
        ]
    );
}

#[test]
fn parses_several_spoilers_in_the_same_message() {
    let blocks = parse("||one|| and ||two||");

    assert_eq!(
        blocks[0].segments(),
        vec![
            text("one", &[SPOILER]),
            text(" and ", &[]),
            text("two", &[SPOILER]),
        ]
    );
}

#[test]
fn a_spoiler_inherits_and_composes_with_the_other_inline_styles() {
    let bold = parse("*||hidden||*");
    let spoiler = parse("||*hidden*||");

    assert_eq!(bold[0].segments(), vec![text("hidden", &[BOLD, SPOILER])]);
    assert_eq!(
        spoiler[0].segments(),
        vec![text("hidden", &[SPOILER, BOLD])]
    );
}

#[test]
fn a_spoiler_keeps_links_and_mentions_atomic() {
    let blocks = parse_with_mentions("||https://example.com/a @Ann||", &["Ann"]);

    assert_eq!(
        blocks[0].segments(),
        vec![
            link("https://example.com/a", "https://example.com/a", &[SPOILER],),
            text(" ", &[SPOILER]),
            mention("@Ann", "Ann", &[SPOILER]),
        ]
    );
}

#[test]
fn keeps_malformed_spoilers_as_literal_text() {
    let unclosed = parse("||hidden");
    let spaced = parse("|| hidden||");
    let trailing_space = parse("||hidden ||");
    let in_word = parse("mid||dle||");

    assert_eq!(unclosed[0].segments(), vec![text("||hidden", &[])]);
    assert_eq!(spaced[0].segments(), vec![text("|| hidden||", &[])]);
    assert_eq!(trailing_space[0].segments(), vec![text("||hidden ||", &[])]);
    assert_eq!(in_word[0].segments(), vec![text("mid||dle||", &[])]);
}

#[test]
fn a_spoiler_does_not_leak_into_the_next_line() {
    let blocks = parse("||hidden||\nplain");

    assert_eq!(
        blocks
            .iter()
            .map(|block| block.kind())
            .collect::<Vec<WhatsAppTextBlockKind>>(),
        vec![
            WhatsAppTextBlockKind::Paragraph,
            WhatsAppTextBlockKind::Paragraph
        ]
    );
    assert_eq!(blocks[0].segments(), vec![text("hidden", &[SPOILER])]);
    assert_eq!(blocks[1].segments(), vec![text("plain", &[])]);
}
