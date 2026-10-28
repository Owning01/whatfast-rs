//! The logos of the agent harnesses, as the assets of the window serve them.
//!
//! The marks are the official ones of each project, taken from the public
//! catalogues of their brands: `@lobehub/icons-static-svg` for the agents and
//! `simple-icons` for the editor. Every document is one line of SVG with a box
//! of 24 by 24 and `currentColor`, so the logo follows the color of the text of
//! its card in the light and in the dark theme, and the card decides the size.
//!
//! A harness without a mark of its own —Hermes, today— falls back to the robot
//! of `gpui_component`: the screen never draws an empty square.

use gpui::SharedString;
use gpui_component::IconNamed;

/// Claude Code.
const CLAUDE_CODE: &str = r#"<svg fill="currentColor" fill-rule="evenodd" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><title>Claude</title><path d="M4.709 15.955l4.72-2.647.08-.23-.08-.128H9.2l-.79-.048-2.698-.073-2.339-.097-2.266-.122-.571-.121L0 11.784l.055-.352.48-.321.686.06 1.52.103 2.278.158 1.652.097 2.449.255h.389l.055-.157-.134-.098-.103-.097-2.358-1.596-2.552-1.688-1.336-.972-.724-.491-.364-.462-.158-1.008.656-.722.881.06.225.061.893.686 1.908 1.476 2.491 1.833.365.304.145-.103.019-.073-.164-.274-1.355-2.446-1.446-2.49-.644-1.032-.17-.619a2.97 2.97 0 01-.104-.729L6.283.134 6.696 0l.996.134.42.364.62 1.414 1.002 2.229 1.555 3.03.456.898.243.832.091.255h.158V9.01l.128-1.706.237-2.095.23-2.695.08-.76.376-.91.747-.492.584.28.48.685-.067.444-.286 1.851-.559 2.903-.364 1.942h.212l.243-.242.985-1.306 1.652-2.064.73-.82.85-.904.547-.431h1.033l.76 1.129-.34 1.166-1.064 1.347-.881 1.142-1.264 1.7-.79 1.36.073.11.188-.02 2.856-.606 1.543-.28 1.841-.315.833.388.091.395-.328.807-1.969.486-2.309.462-3.439.813-.042.03.049.061 1.549.146.662.036h1.622l3.02.225.79.522.474.638-.079.485-1.215.62-1.64-.389-3.829-.91-1.312-.329h-.182v.11l1.093 1.068 2.006 1.81 2.509 2.33.127.578-.322.455-.34-.049-2.205-1.657-.851-.747-1.926-1.62h-.128v.17l.444.649 2.345 3.521.122 1.08-.17.353-.608.213-.668-.122-1.374-1.925-1.415-2.167-1.143-1.943-.14.08-.674 7.254-.316.37-.729.28-.607-.461-.322-.747.322-1.476.389-1.924.315-1.53.286-1.9.17-.632-.012-.042-.14.018-1.434 1.967-2.18 2.945-1.726 1.845-.414.164-.717-.37.067-.662.401-.589 2.388-3.036 1.44-1.882.93-1.086-.006-.158h-.055L4.132 18.56l-1.13.146-.487-.456.061-.746.231-.243 1.908-1.312-.006.006z"></path></svg>"#;

/// Claude Desktop.
const CLAUDE_DESKTOP: &str = r#"<svg fill="currentColor" role="img" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><title>Anthropic</title><path d="M17.3041 3.541h-3.6718l6.696 16.918H24Zm-10.6082 0L0 20.459h3.7442l1.3693-3.5527h7.0052l1.3693 3.5528h3.7442L10.5363 3.5409Zm-.3712 10.2232 2.2914-5.9456 2.2914 5.9456Z"/></svg>"#;

/// Codex.
const CODEX: &str = r#"<svg fill="currentColor" fill-rule="evenodd" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><title>OpenAI</title><path d="M9.205 8.658v-2.26c0-.19.072-.333.238-.428l4.543-2.616c.619-.357 1.356-.523 2.117-.523 2.854 0 4.662 2.212 4.662 4.566 0 .167 0 .357-.024.547l-4.71-2.759a.797.797 0 00-.856 0l-5.97 3.473zm10.609 8.8V12.06c0-.333-.143-.57-.429-.737l-5.97-3.473 1.95-1.118a.433.433 0 01.476 0l4.543 2.617c1.309.76 2.189 2.378 2.189 3.948 0 1.808-1.07 3.473-2.76 4.163zM7.802 12.703l-1.95-1.142c-.167-.095-.239-.238-.239-.428V5.899c0-2.545 1.95-4.472 4.591-4.472 1 0 1.927.333 2.712.928L8.23 5.067c-.285.166-.428.404-.428.737v6.898zM12 15.128l-2.795-1.57v-3.33L12 8.658l2.795 1.57v3.33L12 15.128zm1.796 7.23c-1 0-1.927-.332-2.712-.927l4.686-2.712c.285-.166.428-.404.428-.737v-6.898l1.974 1.142c.167.095.238.238.238.428v5.233c0 2.545-1.974 4.472-4.614 4.472zm-5.637-5.303l-4.544-2.617c-1.308-.761-2.188-2.378-2.188-3.948A4.482 4.482 0 014.21 6.327v5.423c0 .333.143.571.428.738l5.947 3.449-1.95 1.118a.432.432 0 01-.476 0zm-.262 3.9c-2.688 0-4.662-2.021-4.662-4.519 0-.19.024-.38.047-.57l4.686 2.71c.286.167.571.167.856 0l5.97-3.448v2.26c0 .19-.07.333-.237.428l-4.543 2.616c-.619.357-1.356.523-2.117.523zm5.899 2.83a5.947 5.947 0 005.827-4.756C22.287 18.339 24 15.84 24 13.296c0-1.665-.713-3.282-1.998-4.448.119-.5.19-.999.19-1.498 0-3.401-2.759-5.947-5.946-5.947-.642 0-1.26.095-1.88.31A5.962 5.962 0 0010.205 0a5.947 5.947 0 00-5.827 4.757C1.713 5.447 0 7.945 0 10.49c0 1.666.713 3.283 1.998 4.448-.119.5-.19 1-.19 1.499 0 3.401 2.759 5.946 5.946 5.946.642 0 1.26-.095 1.88-.309a5.96 5.96 0 004.162 1.713z"></path></svg>"#;

/// Cursor.
const CURSOR: &str = r#"<svg fill="currentColor" fill-rule="evenodd" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><title>Cursor</title><path d="M22.106 5.68L12.5.135a.998.998 0 00-.998 0L1.893 5.68a.84.84 0 00-.419.726v11.186c0 .3.16.577.42.727l9.607 5.547a.999.999 0 00.998 0l9.608-5.547a.84.84 0 00.42-.727V6.407a.84.84 0 00-.42-.726zm-.603 1.176L12.228 22.92c-.063.108-.228.064-.228-.061V12.34a.59.59 0 00-.295-.51l-9.11-5.26c-.107-.062-.063-.228.062-.228h18.55c.264 0 .428.286.296.514z"></path></svg>"#;

/// OpenCode.
const OPENCODE: &str = r#"<svg fill="currentColor" fill-rule="evenodd" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><title>opencode</title><path d="M16 6H8v12h8V6zm4 16H4V2h16v20z"></path></svg>"#;

/// Pi.
const PI: &str = r#"<svg fill="currentColor" fill-rule="evenodd" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><title>Pi</title><path clip-rule="evenodd" d="M1 1h16.5v11H12v5.5H6.5V23H1V1zm5.5 5.5V12H12V6.5H6.5z"></path><path d="M17.5 12H23v11h-5.5V12z"></path></svg>"#;

/// Antigravity.
const ANTIGRAVITY: &str = r#"<svg fill="currentColor" fill-rule="evenodd" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><title>Antigravity</title><path d="M21.751 22.607c1.34 1.005 3.35.335 1.508-1.508C17.73 15.74 18.904 1 12.037 1 5.17 1 6.342 15.74.815 21.1c-2.01 2.009.167 2.511 1.507 1.506 5.192-3.517 4.857-9.714 9.715-9.714 4.857 0 4.522 6.197 9.714 9.715z"></path></svg>"#;

/// VS Code.
const VSCODE: &str = r#"<svg fill="currentColor" role="img" viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg"><title>Visual Studio Code</title><path d="M23.15 2.587L18.21.21a1.494 1.494 0 0 0-1.705.29l-9.46 8.63-4.12-3.128a.999.999 0 0 0-1.276.057L.327 7.261A1 1 0 0 0 .326 8.74L3.899 12 .326 15.26a1 1 0 0 0 .001 1.479L1.65 17.94a.999.999 0 0 0 1.276.057l4.12-3.128 9.46 8.63a1.492 1.492 0 0 0 1.704.29l4.942-2.377A1.5 1.5 0 0 0 24 20.06V3.939a1.5 1.5 0 0 0-.85-1.352zm-5.146 14.861L10.826 12l7.178-5.448v10.896z"/></svg>"#;

/// The document of SVG of the logo of one harness, when the project has one.
pub(crate) fn svg(id: &str) -> Option<&'static str> {
    match id {
        "claude-code" => Some(CLAUDE_CODE),
        "claude-desktop" => Some(CLAUDE_DESKTOP),
        "codex" => Some(CODEX),
        "cursor" => Some(CURSOR),
        "opencode" => Some(OPENCODE),
        "pi" => Some(PI),
        "antigravity" => Some(ANTIGRAVITY),
        "vscode" => Some(VSCODE),
        _ => None,
    }
}

/// The logo of one harness, for `Icon::new`.
#[derive(Clone, Copy)]
pub(crate) struct HarnessLogo {
    /// The id of the harness, the same one of its spec.
    pub(crate) id: &'static str,
}

impl HarnessLogo {
    /// The path this logo answers to in the assets of the window.
    ///
    /// A harness without a mark answers to the path of the robot, so the card
    /// always has something to draw.
    fn path_data(self) -> &'static str {
        match self.id {
            "claude-code" => "harness/claude-code",
            "claude-desktop" => "harness/claude-desktop",
            "codex" => "harness/codex",
            "cursor" => "harness/cursor",
            "opencode" => "harness/opencode",
            "pi" => "harness/pi",
            "antigravity" => "harness/antigravity",
            "vscode" => "harness/vscode",
            _ => "harness/none",
        }
    }
}

impl IconNamed for HarnessLogo {
    fn path(self) -> SharedString {
        SharedString::new_static(self.path_data())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_logo_is_a_document_of_svg() {
        for id in [
            "claude-code",
            "claude-desktop",
            "codex",
            "cursor",
            "opencode",
            "pi",
            "antigravity",
            "vscode",
        ] {
            let svg = svg(id).unwrap_or_else(|| panic!("{id} has no logo"));
            assert!(svg.starts_with("<svg"), "{id} does not start with svg");
            assert!(svg.ends_with("</svg>"), "{id} does not close");
            assert!(svg.contains("viewBox=\"0 0 24 24\""), "{id} has no box");
            assert!(svg.contains("currentColor"), "{id} does not follow the theme");
            assert!(!svg.contains("width="), "{id} has a fixed width");
        }
    }

    #[test]
    fn a_harness_without_a_logo_asks_for_the_robot() {
        assert!(svg("hermes").is_none());
        assert_eq!(HarnessLogo { id: "hermes" }.path_data(), "harness/none");
        assert_eq!(HarnessLogo { id: "cursor" }.path_data(), "harness/cursor");
    }

    #[test]
    fn every_harness_of_the_catalog_has_its_mark() {
        // El unico harness sin marca propia es Hermes: los demas proyectos
        // publican su logo y la pantalla lo dibuja tal cual.
        for spec in whatfast_agents::spec::all() {
            let tiene = svg(spec.id).is_some();
            assert_eq!(
                tiene,
                spec.id != "hermes",
                "{}: logo ausente o de mas",
                spec.id
            );
        }
    }
}
