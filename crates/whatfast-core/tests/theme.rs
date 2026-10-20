//! Port of the frozen suite `tauri/tests/theme.test.ts`.
//!
//! The oracle applies the theme on a fake DOM element; the port decides the
//! values of that application ([`theme_root_declaration`]) and the UI crate
//! writes them, so the case checks the same values.

use whatfast_core::theme::{
    ThemeAppearance, ThemeVisualStyle, builtin_themes, clone_theme, default_theme, export_theme,
    normalize_theme, theme_root_declaration,
};

#[test]
fn ships_dark_and_light_skeuomorphic_themes_without_changing_the_default() {
    assert_eq!(default_theme().id, "vercel-dark");
    let themes = builtin_themes();
    assert_eq!(
        themes.first().map(|theme| theme.id.as_str()),
        Some("throwback-graphite")
    );
    assert_eq!(
        themes.first().map(|theme| theme.visual_style),
        Some(ThemeVisualStyle::Skeuomorphic)
    );
    assert_eq!(
        themes.first().map(|theme| theme.appearance),
        Some(ThemeAppearance::Dark)
    );
    assert_eq!(
        themes.get(1).map(|theme| theme.id.as_str()),
        Some("classic-messages")
    );
    assert_eq!(
        themes.get(1).map(|theme| theme.appearance),
        Some(ThemeAppearance::Light)
    );
}

#[test]
fn legacy_and_invalid_theme_styles_normalize_to_modern() {
    let legacy = normalize_theme(&serde_json::json!({
        "id": "legacy",
        "name": "Legacy",
        "appearance": "light",
        "tokens": {},
    }));
    let invalid = normalize_theme(&serde_json::json!({
        "id": "invalid",
        "name": "Invalid",
        "visualStyle": "url(https://example.com/theme.css)",
        "tokens": {},
    }));

    assert_eq!(
        legacy.as_ref().map(|theme| theme.visual_style),
        Some(ThemeVisualStyle::Modern)
    );
    assert_eq!(
        invalid.as_ref().map(|theme| theme.visual_style),
        Some(ThemeVisualStyle::Modern)
    );
}

#[test]
fn clone_and_export_retain_the_trusted_structural_style() {
    let source = builtin_themes()
        .first()
        .cloned()
        .unwrap_or_else(default_theme);
    let copy = clone_theme(&source, "Graphite copy");
    let exported: serde_json::Value =
        serde_json::from_str(&export_theme(&copy)).unwrap_or(serde_json::Value::Null);

    assert_eq!(copy.visual_style, ThemeVisualStyle::Skeuomorphic);
    assert_eq!(
        exported
            .get("visualStyle")
            .and_then(serde_json::Value::as_str),
        Some("skeuomorphic")
    );
}

#[test]
fn theme_application_exposes_appearance_and_structural_style_on_the_root() {
    let source = builtin_themes()
        .first()
        .cloned()
        .unwrap_or_else(default_theme);
    let declaration = theme_root_declaration(&source);

    assert_eq!(declaration.appearance, "dark");
    assert_eq!(declaration.theme_style, "skeuomorphic");
    assert_eq!(declaration.color_scheme, "dark");
    assert_eq!(
        declaration
            .properties
            .iter()
            .find(|(name, _)| name == "--accent")
            .map(|(_, value)| value.as_str()),
        Some("#a2d422")
    );
}
