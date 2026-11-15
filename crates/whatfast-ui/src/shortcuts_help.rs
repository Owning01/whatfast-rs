//! Keyboard shortcuts & accessibility cheat sheet modal (`Ctrl+/` or `F1`).

use gpui::{
    Context, InteractiveElement as _, ParentElement as _, StatefulInteractiveElement as _,
    Styled as _, div, px, rgba,
};
use gpui_component::{ActiveTheme as _, Icon, IconName, h_flex, v_flex};
use whatfast_i18n as i18n;

use crate::app::Whatfast;

impl Whatfast {
    pub(crate) fn render_shortcuts_help_modal(&self, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let theme = cx.theme();
        let scale = self.ui_scale;

        let groups: [(&str, &[(&str, &str)]); 4] = [
            (
                "Navigation & Panes",
                &[
                    ("F6 / Shift+F6", "Cycle focus between panes"),
                    ("Ctrl+K", "Open Command Palette"),
                    ("Ctrl+F / /", "Search messages"),
                    ("Alt+Up / Alt+Down", "Previous / Next chat"),
                    ("Ctrl+Tab", "Recent chats switcher"),
                    ("Ctrl+E", "Focus message composer"),
                ],
            ),
            (
                "Message List (when focused)",
                &[
                    ("Up / Down (or K / J)", "Move between messages"),
                    ("Enter / Shift+F10", "Open message actions menu"),
                    ("R / E / S", "Reply / Edit / Star message"),
                    ("Ctrl+C / Delete", "Copy / Delete for me"),
                    ("PageUp / PageDown", "Scroll messages"),
                    ("Home / End", "Top / Bottom of chat"),
                ],
            ),
            (
                "Views & Tools",
                &[
                    ("Ctrl+Shift+S / N / A / H", "States / Channels / Agents / Archived"),
                    ("Ctrl+Shift+Q", "Quick Ask AI"),
                    ("Ctrl+,", "Settings"),
                    ("Ctrl+L", "Lock app"),
                ],
            ),
            (
                "Accessibility & Zoom",
                &[
                    ("Ctrl++ / Ctrl+- / Ctrl+0", "Zoom in / out / reset UI scale"),
                    ("Ctrl+/ / F1", "Show or hide this shortcuts guide"),
                    ("Esc", "Close active menu, modal, or viewer"),
                ],
            ),
        ];

        div()
            .id("shortcuts-help-backdrop")
            .absolute()
            .inset_0()
            .bg(rgba(0x00000088))
            .flex()
            .items_center()
            .justify_center()
            .p_6()
            .on_click(cx.listener(|this, _, _, cx| {
                this.show_shortcuts_help = false;
                cx.notify();
            }))
            .child(
                v_flex()
                    .id("shortcuts-help-modal")
                    .w(px(640. * scale))
                    .max_w_full()
                    .max_h(px(640. * scale))
                    .rounded_xl()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .shadow_lg()
                    .overflow_hidden()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        h_flex()
                            .px_5()
                            .py_3()
                            .justify_between()
                            .items_center()
                            .border_b_1()
                            .border_color(theme.border)
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(theme.foreground)
                                    .child(i18n::t("Keyboard shortcuts")),
                            )
                            .child(
                                div()
                                    .id("shortcuts-help-close")
                                    .p_1()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(theme.secondary))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.show_shortcuts_help = false;
                                        cx.notify();
                                    }))
                                    .child(
                                        Icon::new(IconName::Close)
                                            .size(px(16. * scale))
                                            .text_color(theme.muted_foreground),
                                    ),
                            ),
                    )
                    .child(
                        v_flex()
                            .id("shortcuts-help-body")
                            .p_5()
                            .gap_4()
                            .overflow_y_scroll()
                            .children(groups.into_iter().map(|(heading, rows)| {
                                v_flex()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(theme.primary)
                                            .child(i18n::t(heading)),
                                    )
                                    .children(rows.iter().map(|(keys, desc)| {
                                        h_flex()
                                            .justify_between()
                                            .items_center()
                                            .py_1()
                                            .px_2()
                                            .rounded_md()
                                            .bg(theme.secondary.opacity(0.4))
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(theme.foreground)
                                                    .child(i18n::t(desc)),
                                            )
                                            .child(
                                                div()
                                                    .px_2()
                                                    .py_0p5()
                                                    .rounded_sm()
                                                    .border_1()
                                                    .border_color(theme.border)
                                                    .bg(theme.background)
                                                    .text_xs()
                                                    .text_color(theme.muted_foreground)
                                                    .child(*keys),
                                            )
                                    }))
                            })),
                    ),
            )
    }
}
