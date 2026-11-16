//! Global Command Palette (`Ctrl+K` / `Cmd+K`) for instant navigation and quick actions.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div, px, rgba,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use whatfast_i18n as i18n;

use crate::app::Whatfast;
use crate::navrail::SidebarView;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum QuickCommand {
    OpenSettings,
    OpenDevStats,
    ToggleStates,
    ToggleChannels,
    ToggleAgents,
    ToggleArchived,
    FocusSearch,
    ToggleDoNotDisturb,
    LockApp,
    ExportActiveChat,
}

impl QuickCommand {
    fn label(self) -> &'static str {
        match self {
            Self::OpenSettings => "Open Settings",
            Self::OpenDevStats => "Open DevStats Inspector (Ctrl+Shift+D)",
            Self::ToggleStates => "Open Statuses",
            Self::ToggleChannels => "Open Channels",
            Self::ToggleAgents => "Open Agents",
            Self::ToggleArchived => "Open Archived Chats",
            Self::FocusSearch => "Search Messages",
            Self::ToggleDoNotDisturb => "Toggle Do Not Disturb",
            Self::LockApp => "Lock App (Ctrl+L)",
            Self::ExportActiveChat => "Export Current Chat",
        }
    }

    const ALL: [Self; 10] = [
        Self::OpenSettings,
        Self::OpenDevStats,
        Self::ToggleStates,
        Self::ToggleChannels,
        Self::ToggleAgents,
        Self::ToggleArchived,
        Self::FocusSearch,
        Self::ToggleDoNotDisturb,
        Self::LockApp,
        Self::ExportActiveChat,
    ];
}

#[derive(Default)]
pub(crate) struct CommandPaletteState {
    pub open: bool,
    pub query: String,
    pub selected_index: usize,
    pub input: Option<Entity<InputState>>,
    pub _sub: Option<gpui::Subscription>,
}

impl Whatfast {
    pub(crate) fn toggle_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.command_palette.open {
            self.command_palette.open = false;
            self.focus_handle.focus(window, cx);
            cx.notify();
            return;
        }
        if self.command_palette.input.is_none() {
            let input = cx.new(|cx| {
                InputState::new(window, cx).placeholder(i18n::t("Jump to chat or run a command…"))
            });
            let sub = cx.subscribe(&input, |this, input, event: &InputEvent, cx| {
                match event {
                    InputEvent::Change => {
                        this.command_palette.query = input.read(cx).value().to_string();
                        this.command_palette.selected_index = 0;
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => {
                        this.execute_selected_palette_match(cx);
                    }
                    _ => {}
                }
            });
            self.command_palette._sub = Some(sub);
            self.command_palette.input = Some(input);
        }
        self.command_palette.query.clear();
        self.command_palette.selected_index = 0;
        if let Some(input) = self.command_palette.input.clone() {
            input.update(cx, |inp, cx| {
                inp.set_value("", window, cx);
                inp.focus(window, cx);
            });
        }
        self.command_palette.open = true;
        cx.notify();
    }

    fn palette_matches(&self) -> (Vec<QuickCommand>, Vec<(String, String, u32)>) {
        let q = self.command_palette.query.trim().to_lowercase();
        let matched_cmds: Vec<QuickCommand> = QuickCommand::ALL
            .iter()
            .copied()
            .filter(|cmd| q.is_empty() || i18n::t(cmd.label()).to_lowercase().contains(&q))
            .take(5)
            .collect();
        let matched_chats: Vec<(String, String, u32)> = self
            .store
            .chats
            .iter()
            .filter(|c| {
                q.is_empty()
                    || c.title.to_lowercase().contains(&q)
                    || c.id.to_lowercase().contains(&q)
            })
            .take(8)
            .map(|c| (c.id.clone(), c.title.clone(), c.unread_count))
            .collect();
        (matched_cmds, matched_chats)
    }

    pub(crate) fn move_palette_selection(&mut self, forward: bool) {
        let (cmds, chats) = self.palette_matches();
        let total = cmds.len() + chats.len();
        if total == 0 {
            self.command_palette.selected_index = 0;
            return;
        }
        let current = self.command_palette.selected_index.min(total - 1);
        self.command_palette.selected_index = if forward {
            (current + 1) % total
        } else {
            (current + total - 1) % total
        };
    }

    pub(crate) fn execute_selected_palette_match(&mut self, cx: &mut Context<Self>) {
        let (cmds, chats) = self.palette_matches();
        let total = cmds.len() + chats.len();
        if total == 0 {
            return;
        }
        let idx = self.command_palette.selected_index.min(total - 1);
        if idx < cmds.len() {
            self.run_quick_command(cmds[idx], cx);
        } else if let Some((chat_id, _, _)) = chats.get(idx - cmds.len()) {
            let chat_id = chat_id.clone();
            self.command_palette.open = false;
            self.store.select_chat(chat_id.clone());
            self.load_selected_chat(chat_id);
            cx.notify();
        }
    }

    pub(crate) fn run_quick_command(&mut self, cmd: QuickCommand, cx: &mut Context<Self>) {
        self.command_palette.open = false;
        match cmd {
            QuickCommand::OpenSettings => {
                self.sidebar_view = SidebarView::Settings;
            }
            QuickCommand::OpenDevStats => {
                self.toggle_dev_stats(cx);
                return;
            }
            QuickCommand::ToggleStates => {
                self.sidebar_view = SidebarView::States;
            }
            QuickCommand::ToggleChannels => {
                self.sidebar_view = SidebarView::Channels;
            }
            QuickCommand::ToggleAgents => {
                self.sidebar_view = SidebarView::Agents;
            }
            QuickCommand::ToggleArchived => {
                self.sidebar_view = SidebarView::Archived;
            }
            QuickCommand::FocusSearch => {
                self.store.toast_info = Some(i18n::t("Press Ctrl+F to search inside messages").into());
            }
            QuickCommand::ToggleDoNotDisturb => {
                self.do_not_disturb = !self.do_not_disturb;
                self.save_settings();
            }
            QuickCommand::LockApp => {
                self.lock_now(cx);
                return;
            }
            QuickCommand::ExportActiveChat => {
                self.export_active_chat_txt(cx);
                return;
            }
        }
        // El comando cambia la vista: el riel tiene que reflejarla ahora, no
        // en el proximo gesto que llame a `sync_shell`.
        self.sync_shell(cx);
        cx.notify();
    }

    pub(crate) fn render_command_palette(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let scale = self.ui_scale;
        let (matched_cmds, matched_chats) = self.palette_matches();
        let cmd_count = matched_cmds.len();
        let total_matches = cmd_count + matched_chats.len();
        let active_idx = if total_matches > 0 {
            Some(self.command_palette.selected_index.min(total_matches - 1))
        } else {
            None
        };

        div()
            .id("command-palette-backdrop")
            .absolute()
            .inset_0()
            .bg(rgba(0x00000080))
            .flex()
            .items_start()
            .justify_center()
            .pt(px(80. * scale))
            .on_click(cx.listener(|this, _, _, cx| {
                this.command_palette.open = false;
                cx.notify();
            }))
            .child(
                v_flex()
                    .id("command-palette-modal")
                    .w(px(540. * scale))
                    .max_w_full()
                    .rounded_xl()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .shadow_lg()
                    .overflow_hidden()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        h_flex()
                            .p_3()
                            .gap_2()
                            .items_center()
                            .border_b_1()
                            .border_color(theme.border)
                            .child(
                                Icon::new(IconName::Search)
                                    .size(px(16. * scale))
                                    .text_color(theme.muted_foreground),
                            )
                            .when_some(self.command_palette.input.clone(), |el, input| {
                                el.child(div().flex_1().child(Input::new(&input).cleanable(true)))
                            }),
                    )
                    .child(
                        v_flex()
                            .id("command-palette-results")
                            .max_h(px(380. * scale))
                            .overflow_y_scroll()
                            .p_2()
                            .gap_1()
                            .when(!matched_cmds.is_empty(), |el| {
                                el.child(
                                    div()
                                        .px_2()
                                        .py_1()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(i18n::t("Commands")),
                                )
                                .children(matched_cmds.into_iter().enumerate().map(|(i, cmd)| {
                                    let selected = active_idx == Some(i);
                                    div()
                                        .id(("palette-cmd", i))
                                        .px_3()
                                        .py_2()
                                        .rounded_md()
                                        .cursor_pointer()
                                        .border_1()
                                        .border_color(if selected {
                                            theme.ring
                                        } else {
                                            gpui::transparent_black()
                                        })
                                        .when(selected, |s| s.bg(theme.accent.opacity(0.16)))
                                        .hover(|s| s.bg(theme.secondary))
                                        .text_sm()
                                        .text_color(theme.foreground)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.run_quick_command(cmd, cx);
                                        }))
                                        .child(i18n::t(cmd.label()))
                                }))
                            })
                            .when(!matched_chats.is_empty(), |el| {
                                el.child(
                                    div()
                                        .px_2()
                                        .py_1()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(i18n::t("Chats")),
                                )
                                .children(matched_chats.into_iter().enumerate().map(
                                    |(i, (id, title, unread))| {
                                        let selected = active_idx == Some(cmd_count + i);
                                        h_flex()
                                            .id(("palette-chat", i))
                                            .px_3()
                                            .py_2()
                                            .rounded_md()
                                            .cursor_pointer()
                                            .border_1()
                                            .border_color(if selected {
                                                theme.ring
                                            } else {
                                                gpui::transparent_black()
                                            })
                                            .when(selected, |s| s.bg(theme.accent.opacity(0.16)))
                                            .justify_between()
                                            .hover(|s| s.bg(theme.secondary))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.command_palette.open = false;
                                                this.store.select_chat(id.clone());
                                                this.load_selected_chat(id.clone());
                                                cx.notify();
                                            }))
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(theme.foreground)
                                                    .truncate()
                                                    .child(title),
                                            )
                                            .when(unread > 0, |row| {
                                                row.child(
                                                    div()
                                                        .px_2()
                                                        .py(px(1.))
                                                        .rounded_full()
                                                        .bg(theme.accent)
                                                        .text_xs()
                                                        .text_color(theme.accent_foreground)
                                                        .child(unread.to_string()),
                                                )
                                            })
                                    },
                                ))
                            }),
                    ),
            )
    }
}
