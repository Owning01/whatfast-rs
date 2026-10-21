//! The drawing of the Whatfast client.
//!
//! port: taken from the origin's `desktop/src/main.rs`. Every `render_*`
//! method of the GPUI client lives here, with the same colours, sizes,
//! spacing, texts and behaviour: the origin is the visual and functional
//! reference, this file is its translation.
//!
//! The state lives in `app.rs` (the other half of the origin's `main.rs`),
//! the free helpers of the drawing in `helpers.rs`.

use std::collections::HashSet;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Context, Focusable as _, FontWeight, InteractiveElement as _, IntoElement, KeyDownEvent,
    ModifiersChangedEvent, ObjectFit, ParentElement as _, Pixels, Render, ScrollStrategy,
    SharedString, StatefulInteractiveElement as _, Styled as _, StyledImage as _, Window, div, img,
    point, px, rgb, rgba, size, uniform_list,
};
use gpui_component::{
    ActiveTheme as _, Colorize as _, Disableable as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    scroll::{ScrollableElement as _, ScrollbarAxis},
    v_flex, v_virtual_list,
};
use qrcode::QrCode;
// The origin's `crate::proto` is now the protocol crate: the same types, with
// the same names, at the root of the crate.
use whatfast_protocol as proto;

use whatfast_model::emoji_picker::EmojiCategory;
use whatfast_model::geom;
use whatfast_model::model::{
    ChatView, Screen, image_row_path, image_viewer_path, message_text, reaction_counts,
    reaction_sender_detail, reaction_sender_name,
};

use crate::app::{
    CycleRecentChat, CycleRecentChatReverse, DARK_GREEN, EMOJI_COLUMNS, EmojiTarget, MAX_UI_SCALE,
    MIN_UI_SCALE, PANEL, ScrollSurface, SearchResultRow, UI_SCALE_STEP, Whatfast,
};
use crate::helpers::{
    avatar_element, connection_label, format_disappearing_timer, format_epoch_date, identity_hue,
    image_render_size, message_preview, message_status, one_line, qr_canvas,
};
use crate::rpc::PendingRequest;
use crate::text::{is_safe_web_url, link_preview_host, mention_display_name, split_url_token};

impl Whatfast {
    fn render_pairing(&self, cx: &mut Context<Self>) -> gpui::Div {
        let qr = self
            .store
            .qr_code
            .as_deref()
            .and_then(|code| QrCode::new(code.as_bytes()).ok());
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_4()
            .child(
                div()
                    .text_2xl()
                    .font_weight(FontWeight::BOLD)
                    .child("Link Whatfast"),
            )
            .child("WhatsApp → Linked devices → Link a device")
            .child(match qr {
                Some(qr) => qr_canvas(qr),
                None => div()
                    .size(px(240.))
                    .rounded_lg()
                    .bg(if cx.theme().is_dark() {
                        cx.theme().secondary
                    } else {
                        rgb(PANEL).into()
                    })
                    .flex()
                    .items_center()
                    .justify_center()
                    .child("Waiting for QR…"),
            })
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("The code refreshes automatically."),
            )
            .child(
                Button::new("retry-pairing")
                    .label("Refresh pairing code")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.begin_pairing();
                        cx.notify();
                    })),
            )
    }

    fn render_center(
        &self,
        title: impl Into<SharedString>,
        detail: impl Into<SharedString>,
    ) -> gpui::Div {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::BOLD)
                    .child(title.into()),
            )
            .child(div().text_sm().child(detail.into()))
    }

    fn render_chats(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui::Div {
        let selected = self.store.selected_chat_id.clone();
        let compact = window.viewport_size().width < px(800. * self.ui_scale);
        let show_sidebar = !compact || selected.is_none();
        let show_conversation = !compact || selected.is_some();
        h_flex()
            .size_full()
            // The column of icons is the left edge of the window: it is the
            // navigation of the web client, the same way.
            .child(self.nav_rail.clone())
            .when(show_sidebar, |root| root.child(self.render_sidebar(cx)))
            .when(show_conversation, |root| {
                root.child(self.render_conversation(compact, window, cx))
            })
    }

    fn render_sidebar(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let ui_scale = self.ui_scale;
        let total = self.store.total_chats;
        let logout_pending = self.rpc.logout_pending();
        let dark = cx.theme().is_dark();
        let chat_view = self.chat_view;
        let visible_chat_ids = self.store.chat_ids(chat_view);
        let search_active = self.search_query.trim().chars().count() >= 2;
        let visible_count = visible_chat_ids.len();
        let has_more = !self.store.next_chat_cursor.is_empty();
        let virtual_count = visible_count + usize::from(has_more);
        let list_id = if chat_view == ChatView::Archived {
            "archived-chat-list"
        } else {
            "inbox-chat-list"
        };
        let view_title = if chat_view == ChatView::Archived {
            "Archived"
        } else {
            "Whatfast"
        };
        let local_status = if self.store.sync_active {
            format!(
                "{} chats · scanned {} chats · {} messages",
                total, self.store.sync_chats, self.store.sync_messages
            )
        } else if chat_view == ChatView::Archived {
            format!(
                "{}{} archived · {} chats",
                visible_count,
                if has_more { "+" } else { "" },
                total
            )
        } else {
            format!("{} chats", total)
        };
        let selected_background = if dark { rgb(0x173a35) } else { rgb(0xe7f5ef) };
        let hover_background = if dark { rgb(0x152f2c) } else { rgb(0xeff5f2) };
        let visible_for_list = visible_chat_ids.clone();
        v_flex()
            .h_full()
            .w(px(340. * ui_scale))
            .min_w(px(280. * ui_scale))
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                v_flex()
                    .h(px(132. * ui_scale))
                    .px_3()
                    .py_2()
                    .gap_1()
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_lg()
                                    .font_weight(FontWeight::BOLD)
                                    .child(view_title),
                            )
                            .child(
                                h_flex()
                                    .gap_1()
                                    .child(
                                        Button::new("theme")
                                            .small()
                                            .icon(if dark {
                                                IconName::Sun
                                            } else {
                                                IconName::Moon
                                            })
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.toggle_theme(window, cx);
                                            })),
                                    )
                                    .child(
                                        Button::new("archived")
                                            .small()
                                            .icon(if chat_view == ChatView::Archived {
                                                IconName::Inbox
                                            } else {
                                                IconName::FolderClosed
                                            })
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.toggle_chat_view(window, cx);
                                                cx.notify();
                                            })),
                                    )
                                    .child(Button::new("settings").small().icon(IconName::Settings).on_click(
                                        cx.listener(|this, _, _, cx| {
                                            this.settings_open = !this.settings_open;
                                            this.reaction_details = None;
                                            this.image_viewer = None;
                                            cx.notify();
                                        }),
                                    ))
                                    .child(
                                        Button::new("logout")
                                            .small()
                                            .label(if logout_pending {
                                                "Clearing…"
                                            } else if self.confirming_logout {
                                                "Confirm"
                                            } else {
                                                "Log out"
                                            })
                                            .disabled(logout_pending)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                if this.confirming_logout {
                                                    this.request_logout();
                                                } else {
                                                    this.confirming_logout = true;
                                                }
                                                cx.notify();
                                            })),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(local_status),
                    )
                    .child(
                        div()
                            .w_full()
                            .h(px(3.))
                            .rounded_full()
                            .overflow_hidden()
                            .bg(cx.theme().secondary)
                            .when(self.store.sync_active, |track| {
                                track.flex().justify_center().child(
                                    div().h_full().w(px(72.)).rounded_full().bg(rgb(0x25d366)),
                                )
                            })
                            .when(self.store.sync_complete, |track| {
                                track
                                    .child(div().h_full().w_full().rounded_full().bg(rgb(0x25d366)))
                            }),
                    )
                    .child(div().pt_1().child(Input::new(&self.search_input).w_full())),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .when(!search_active, |list| {
                        list.child(
                            uniform_list(
                                list_id,
                                virtual_count,
                                cx.processor(move |this, range: Range<usize>, _, cx| {
                                    let loaded = visible_for_list.len();
                                    if range.end >= loaded
                                        && !this.store.next_chat_cursor.is_empty()
                                    {
                                        this.load_chats(this.store.next_chat_cursor.clone());
                                    }
                                    if this.store.connection == proto::ConnectionState::Connected
                                        && this.rpc.pending_media_count() < 4
                                        && let Some(chat_id) = range.clone().find_map(|index| {
                                            visible_for_list.get(index).and_then(|chat_id| {
                                                this.store.chat(chat_id).and_then(|chat| {
                                                    (chat.avatar_path.is_empty()
                                                        && chat.kind() != proto::ChatKind::Other
                                                        && !this
                                                            .avatar_attempted
                                                            .contains(&chat.id))
                                                    .then(|| chat.id.clone())
                                                })
                                            })
                                        })
                                    {
                                        this.load_avatar(chat_id);
                                    }
                                    range
                                        .map(|index| {
                                            let Some(chat) = visible_for_list
                                                .get(index)
                                                .and_then(|id| this.store.chat(id))
                                                .cloned()
                                            else {
                                                return div()
                                                    .id(("loading-chat", index))
                                                    .h(px(72.))
                                                    .px_4()
                                                    .flex()
                                                    .items_center()
                                                    .text_sm()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child("Loading more chats…");
                                            };
                                            let selected = this.store.selected_chat_id.as_deref()
                                                == Some(chat.id.as_str());
                                            let avatar = avatar_element(
                                                &chat.title,
                                                &chat.avatar_path,
                                                px(42. * ui_scale),
                                                cx,
                                            );
                                            let typing_preview = this.typing_label(
                                                &chat.id,
                                                chat.kind() == proto::ChatKind::Group,
                                            );
                                            let row_typing = typing_preview.is_some();
                                            let preview = typing_preview.unwrap_or_else(|| {
                                                if chat.phone_number.is_empty() {
                                                    chat.last_message_preview.clone()
                                                } else if chat.last_message_preview.is_empty() {
                                                    chat.phone_number.clone()
                                                } else {
                                                    format!(
                                                        "{} · {}",
                                                        chat.phone_number,
                                                        chat.last_message_preview
                                                    )
                                                }
                                            });
                                            div()
                                                .id(("chat", index))
                                                .h(px(72. * ui_scale))
                                                .w_full()
                                                .px_3()
                                                .flex()
                                                .items_center()
                                                .gap_3()
                                                .overflow_hidden()
                                                .when(selected, |row| row.bg(selected_background))
                                                .hover(move |style| style.bg(hover_background))
                                                .on_click(cx.listener(move |this, _, window, cx| {
                                                    this.open_chat(chat.id.clone(), window, cx);
                                                    cx.notify();
                                                }))
                                                .child(avatar)
                                                .child(
                                                    v_flex()
                                                        .flex_1()
                                                        .min_w_0()
                                                        .gap_1()
                                                        .child(
                                                            h_flex()
                                                                .w_full()
                                                                .items_center()
                                                                .gap_2()
                                                                .child(
                                                                    div()
                                                                        .flex_1()
                                                                        .min_w_0()
                                                                        .truncate()
                                                                        .font_weight(
                                                                            FontWeight::SEMIBOLD,
                                                                        )
                                                                        .child(one_line(&chat.title)),
                                                                )
                                                                .when(chat.unread_count > 0, |line| {
                                                                    line.child(
                                                                        div()
                                                                            .flex_shrink_0()
                                                                            .min_w(px(18.))
                                                                            .h(px(18.))
                                                                            .px_1()
                                                                            .rounded_full()
                                                                            .flex()
                                                                            .items_center()
                                                                            .justify_center()
                                                                            .bg(rgb(0x25d366))
                                                                            .text_xs()
                                                                            .font_weight(
                                                                                FontWeight::SEMIBOLD,
                                                                            )
                                                                            .text_color(rgb(0x0b141a))
                                                                            .child(
                                                                                chat.unread_count
                                                                                    .to_string(),
                                                                            ),
                                                                    )
                                                                }),
                                                        )
                                                        .child(
                                                            div()
                                                                .w_full()
                                                                .min_w_0()
                                                                .truncate()
                                                                .text_sm()
                                                                .text_color(if row_typing {
                                                                    if dark {
                                                                        gpui::Hsla::from(rgb(0x25d366))
                                                                    } else {
                                                                        gpui::Hsla::from(rgb(
                                                                            DARK_GREEN,
                                                                        ))
                                                                    }
                                                                } else {
                                                                    cx.theme().muted_foreground
                                                                })
                                                                .child(one_line(&preview)),
                                                        ),
                                                )
                                        })
                                        .collect::<Vec<_>>()
                                }),
                            )
                            .h_full()
                            .track_scroll(&self.chat_scroll)
                            .on_scroll_wheel(cx.listener(
                                |this, event, window, cx| {
                                    this.handle_smooth_scroll_input(
                                        ScrollSurface::ChatList,
                                        event,
                                        window,
                                        cx,
                                    );
                                },
                            )),
                        )
                    })
                    .when(!search_active && visible_count == 0 && !has_more, |list| {
                        list.child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .px_4()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(if chat_view == ChatView::Archived {
                                    "No archived chats"
                                } else {
                                    "No chats yet"
                                }),
                        )
                    })
                    .when(search_active, |list| {
                        list.child(self.render_search_results(cx))
                    }),
            )
    }

    fn render_search_results(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let pending = self.rpc.pending_requests().any(|pending| {
            matches!(pending, PendingRequest::Search { generation, .. } if *generation == self.search_generation)
        });
        let total = self.search_results.len();
        let query_len = self.search_query.trim().chars().count();
        let mut content = v_flex().absolute().inset_0().bg(cx.theme().background);
        if pending && total == 0 {
            return content.child(
                div()
                    .p_4()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Searching local data…"),
            );
        }
        if let Some(error) = self.search_error.clone() {
            return content.child(div().p_4().text_sm().text_color(rgb(0xb91c1c)).child(error));
        }
        if total == 0 {
            return content.child(
                div()
                    .p_4()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(if query_len < 3 {
                        "No contacts or groups found · type 3 characters to search messages"
                    } else {
                        "No local results"
                    }),
            );
        }

        let rows = Rc::new(self.search_results.rows());
        let row_sizes = Rc::new(
            rows.iter()
                .map(|row| {
                    let height = match row {
                        SearchResultRow::Header { .. } => 30.,
                        SearchResultRow::Contact { .. } | SearchResultRow::Group { .. } => 58.,
                        SearchResultRow::Message { .. } => 68.,
                    };
                    size(px(1.), px(height * self.ui_scale))
                })
                .collect::<Vec<_>>(),
        );
        let rows_for_list = rows.clone();
        content = content.child(
            v_flex()
                .relative()
                .flex_1()
                .min_h_0()
                .on_scroll_wheel(cx.listener(|this, event, window, cx| {
                    this.handle_smooth_scroll_input(ScrollSurface::Search, event, window, cx);
                }))
                .child(
                    v_virtual_list(
                        cx.entity().clone(),
                        "search-result-list",
                        row_sizes,
                        move |this, visible_range, _, cx| {
                            if this.store.connection == proto::ConnectionState::Connected
                                && this.rpc.pending_media_count() < 4
                            {
                                for row_index in visible_range.clone() {
                                    match rows_for_list.get(row_index) {
                                        Some(SearchResultRow::Contact { source_index, .. }) => {
                                            if let Some(contact) =
                                                this.search_results.contacts.get(*source_index)
                                            {
                                                if !contact.chat_id.is_empty() {
                                                    let has_avatar = this
                                                        .store
                                                        .chat(&contact.chat_id)
                                                        .is_some_and(|c| !c.avatar_path.is_empty());
                                                    if !has_avatar
                                                        && !this
                                                            .avatar_attempted
                                                            .contains(&contact.chat_id)
                                                    {
                                                        this.load_avatar(contact.chat_id.clone());
                                                    }
                                                } else if !contact.contact_jid.is_empty() {
                                                    let has_avatar = this
                                                        .store
                                                        .participant_avatar_path(
                                                            &contact.contact_jid,
                                                        )
                                                        .is_some();
                                                    if !has_avatar
                                                        && !this
                                                            .participant_avatar_attempted
                                                            .contains(&contact.contact_jid)
                                                    {
                                                        this.load_participant_avatar(
                                                            contact.contact_jid.clone(),
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                        Some(SearchResultRow::Group { source_index, .. }) => {
                                            if let Some(chat) =
                                                this.search_results.groups.get(*source_index)
                                            {
                                                let has_avatar = this
                                                    .store
                                                    .chat(&chat.id)
                                                    .is_some_and(|c| !c.avatar_path.is_empty())
                                                    || !chat.avatar_path.is_empty();
                                                if !has_avatar
                                                    && !this.avatar_attempted.contains(&chat.id)
                                                {
                                                    this.load_avatar(chat.id.clone());
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            visible_range
                                .filter_map(|row_index| rows_for_list.get(row_index).cloned())
                                .filter_map(|row| this.render_search_result_row(row, cx))
                                .collect::<Vec<_>>()
                        },
                    )
                    .track_scroll(&self.search_scroll),
                )
                .scrollbar(&self.search_scroll, ScrollbarAxis::Vertical),
        );
        if query_len < 3 {
            content = content.child(
                div()
                    .flex_none()
                    .px_3()
                    .py_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Type 3 characters to include messages"),
            );
        }
        content
    }

    fn render_search_result_row(
        &self,
        row: SearchResultRow,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let selected_background = if cx.theme().is_dark() {
            rgb(0x173f35)
        } else {
            rgb(0xd9fdd3)
        };
        let hover_background = if cx.theme().is_dark() {
            rgb(0x202c33)
        } else {
            rgb(0xf0f2f5)
        };
        match row {
            SearchResultRow::Header { title, count } => Some(
                h_flex()
                    .h(px(30. * self.ui_scale))
                    .px_3()
                    .items_center()
                    .justify_between()
                    .bg(cx.theme().secondary)
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title)
                    .child(count.to_string())
                    .into_any_element(),
            ),
            SearchResultRow::Contact {
                result_index,
                source_index,
            } => {
                let contact = self.search_results.contacts.get(source_index)?.clone();
                let subtitle = match (
                    contact.secondary_name.is_empty(),
                    contact.phone_number.is_empty(),
                ) {
                    (false, false) => {
                        format!("{} · {}", contact.secondary_name, contact.phone_number)
                    }
                    (false, true) => contact.secondary_name.clone(),
                    (true, false) => contact.phone_number.clone(),
                    (true, true) => "WhatsApp contact".into(),
                };
                let cached_avatar = if !contact.chat_id.is_empty() {
                    self.store
                        .chat(&contact.chat_id)
                        .map(|c| c.avatar_path.as_str())
                } else {
                    None
                };
                let avatar_path = cached_avatar
                    .or_else(|| self.store.participant_avatar_path(&contact.contact_jid))
                    .unwrap_or("");
                Some(
                    h_flex()
                        .id(("search-contact", result_index))
                        .h(px(58. * self.ui_scale))
                        .overflow_hidden()
                        .px_3()
                        .gap_3()
                        .items_center()
                        .cursor_pointer()
                        .when(result_index == self.search_highlighted, |row| {
                            row.bg(selected_background)
                        })
                        .hover(move |style| style.bg(hover_background))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.search_highlighted = result_index;
                            this.activate_search_result(window, cx);
                        }))
                        .child(avatar_element(
                            &contact.display_name,
                            avatar_path,
                            px(38.),
                            cx,
                        ))
                        .child(
                            v_flex()
                                .min_w_0()
                                .flex_1()
                                .child(
                                    div()
                                        .w_full()
                                        .min_w_0()
                                        .truncate()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(one_line(&contact.display_name)),
                                )
                                .child(
                                    div()
                                        .w_full()
                                        .min_w_0()
                                        .truncate()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(one_line(&subtitle)),
                                ),
                        )
                        .into_any_element(),
                )
            }
            SearchResultRow::Group {
                result_index,
                source_index,
            } => {
                let chat = self.search_results.groups.get(source_index)?.clone();
                let preview = if chat.last_message_preview.is_empty() {
                    "No messages yet".to_string()
                } else {
                    chat.last_message_preview.clone()
                };
                let cached_avatar = self.store.chat(&chat.id).map(|c| c.avatar_path.as_str());
                let avatar_path = cached_avatar.unwrap_or(chat.avatar_path.as_str());
                Some(
                    h_flex()
                        .id(("search-group", result_index))
                        .h(px(58. * self.ui_scale))
                        .overflow_hidden()
                        .px_3()
                        .gap_3()
                        .items_center()
                        .cursor_pointer()
                        .when(result_index == self.search_highlighted, |row| {
                            row.bg(selected_background)
                        })
                        .hover(move |style| style.bg(hover_background))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.search_highlighted = result_index;
                            this.activate_search_result(window, cx);
                        }))
                        .child(avatar_element(&chat.title, avatar_path, px(38.), cx))
                        .child(
                            v_flex()
                                .min_w_0()
                                .flex_1()
                                .child(
                                    div()
                                        .w_full()
                                        .min_w_0()
                                        .truncate()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(one_line(&chat.title)),
                                )
                                .child(
                                    div()
                                        .w_full()
                                        .min_w_0()
                                        .truncate()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(one_line(&preview)),
                                ),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(if chat.archived { "Archived" } else { "Group" }),
                        )
                        .into_any_element(),
                )
            }
            SearchResultRow::Message {
                result_index,
                source_index,
            } => {
                let message = self.search_results.messages.get(source_index)?.clone();
                let meta = if message.archived {
                    format!("{} · Archived", message.sender_name)
                } else {
                    message.sender_name.clone()
                };
                Some(
                    v_flex()
                        .id(("search-message", result_index))
                        .h(px(68. * self.ui_scale))
                        .overflow_hidden()
                        .px_3()
                        .py_2()
                        .cursor_pointer()
                        .when(result_index == self.search_highlighted, |row| {
                            row.bg(selected_background)
                        })
                        .hover(move |style| style.bg(hover_background))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.search_highlighted = result_index;
                            this.activate_search_result(window, cx);
                        }))
                        .child(
                            h_flex()
                                .justify_between()
                                .gap_2()
                                .child(
                                    div()
                                        .min_w_0()
                                        .flex_1()
                                        .truncate()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(one_line(&message.chat_title)),
                                )
                                .child(
                                    div()
                                        .flex_none()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(one_line(&meta)),
                                ),
                        )
                        .child(
                            div()
                                .w_full()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(one_line(&message.snippet)),
                        )
                        .into_any_element(),
                )
            }
        }
    }

    fn render_conversation(
        &mut self,
        compact: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let Some(chat) = self.store.selected_chat().cloned() else {
            return self
                .render_center("Whatfast", "Choose a conversation to start messaging")
                .flex_1();
        };
        if chat.avatar_path.is_empty()
            && chat.kind() != proto::ChatKind::Other
            && self.store.connection == proto::ConnectionState::Connected
        {
            self.load_avatar(chat.id.clone());
        }
        let available = window.viewport_size().width - if compact { px(56.) } else { px(396.) };
        let wrap_width = if available < px(160.) {
            px(160.)
        } else if available > px(536.) {
            px(536.)
        } else {
            available
        };
        self.measure_message_heights(wrap_width, cx.theme().font_size, window);
        let sizes = self.store.message_sizes();
        if let Some(generation) = self.scroll_to_bottom_generation.take() {
            self.stop_smooth_scroll(ScrollSurface::Messages);
            let item_count = sizes.len();
            cx.on_next_frame(window, move |this, _, cx| {
                if item_count > 0
                    && this.message_generation == generation
                    && this.store.messages.len() == item_count
                {
                    this.message_scroll
                        .scroll_to_item(item_count.saturating_sub(1), ScrollStrategy::Bottom);
                    cx.notify();
                }
            });
        }
        if let Some((anchor_id, old_offset)) = self.pending_prepend_anchor.take()
            && let Some(anchor_index) = self
                .store
                .messages
                .iter()
                .position(|message| message.id == anchor_id)
        {
            self.stop_smooth_scroll(ScrollSurface::Messages);
            let inserted_height = sizes[..anchor_index]
                .iter()
                .fold(px(0.), |height, size| height + px(size.height) + px(8.));
            self.message_scroll
                .set_offset(point(old_offset.x, old_offset.y - inserted_height));
        }
        if let Some(target_id) = self.pending_search_scroll_id.take()
            && let Some(index) = self
                .store
                .messages
                .iter()
                .position(|message| message.id == target_id)
        {
            self.stop_smooth_scroll(ScrollSurface::Messages);
            self.message_scroll
                .scroll_to_item(index, ScrollStrategy::Center);
        }
        if let Some(target_id) = self.pending_top_scroll_id.take()
            && let Some(index) = self
                .store
                .messages
                .iter()
                .position(|message| message.id == target_id)
        {
            self.stop_smooth_scroll(ScrollSurface::Messages);
            self.message_scroll
                .scroll_to_item(index, ScrollStrategy::Top);
        }
        let connected = self.store.connection == proto::ConnectionState::Connected;
        let has_older = self.store.has_older_messages;
        let has_newer = self.store.has_newer_messages || self.store.newer_activity;
        let dark = cx.theme().is_dark();
        let conversation_background = if dark { rgb(0x0d1715) } else { rgb(0xf4f0e8) };
        let connection_detail = if self.store.connection_detail.is_empty() {
            connection_label(self.store.connection).to_string()
        } else {
            self.store.connection_detail.clone()
        };
        let mut identity_parts = Vec::new();
        if !chat.phone_number.is_empty() {
            identity_parts.push(chat.phone_number.clone());
        }
        if !chat.business_name.is_empty() && chat.business_name != chat.title {
            identity_parts.push(chat.business_name.clone());
        }
        if !chat.push_name.is_empty()
            && chat.push_name != chat.title
            && chat.push_name != chat.business_name
        {
            identity_parts.push(chat.push_name.clone());
        }
        identity_parts.push(connection_detail);
        let typing_line = self.typing_label(&chat.id, chat.kind() == proto::ChatKind::Group);
        let header_typing = typing_line.is_some();
        let composer_typing_line = typing_line.clone();
        let identity_line = typing_line.unwrap_or_else(|| identity_parts.join(" · "));
        let reply_context = self.replying_to_message_id.as_deref().map(|message_id| {
            self.store.message(message_id).map_or_else(
                || {
                    (
                        "Original message".to_string(),
                        "Message unavailable".to_string(),
                    )
                },
                |message| {
                    let sender = if message.from_me {
                        "You".to_string()
                    } else if message.sender_name.trim().is_empty() {
                        "Unknown contact".to_string()
                    } else {
                        message.sender_name.trim().to_string()
                    };
                    (sender, message_preview(message))
                },
            )
        });
        let header_avatar =
            avatar_element(&chat.title, &chat.avatar_path, px(38. * self.ui_scale), cx);
        let list_sizes = Rc::new(
            sizes
                .iter()
                .map(|entry| size(px(entry.width), px(entry.height)))
                .collect::<Vec<_>>(),
        );
        v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            // The strip of the open chats sits above the header of the
            // conversation, the same way the web client draws it.
            .child(self.tab_strip.clone())
            .child(
                h_flex()
                    .h(px(64. * self.ui_scale))
                    .px_4()
                    .gap_3()
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .when(compact, |header| {
                        header.child(Button::new("back").label("‹").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.store.selected_chat_id = None;
                                this.reaction_details = None;
                                this.cancel_reply();
                                cx.notify();
                            },
                        )))
                    })
                    .child(
                        h_flex()
                            .id("chat-info-open")
                            .flex_1()
                            .min_w_0()
                            .gap_3()
                            .items_center()
                            .cursor_pointer()
                            .on_click(cx.listener({
                                let chat_id = chat.id.clone();
                                move |this, _, _, cx| {
                                    this.open_chat_info(chat_id.clone());
                                    cx.notify();
                                }
                            }))
                            .child(header_avatar)
                            .child(
                                v_flex()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .truncate()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(chat.title),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .text_xs()
                                            .text_color(if header_typing {
                                                if dark {
                                                    gpui::Hsla::from(rgb(0x25d366))
                                                } else {
                                                    gpui::Hsla::from(rgb(DARK_GREEN))
                                                }
                                            } else {
                                                cx.theme().muted_foreground
                                            })
                                            .child(identity_line),
                                    ),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .bg(conversation_background)
                    .on_scroll_wheel(cx.listener(|this, event, window, cx| {
                        this.handle_smooth_scroll_input(ScrollSurface::Messages, event, window, cx);
                    }))
                    .child(
                        v_virtual_list(
                            cx.entity().clone(),
                            "messages",
                            list_sizes,
                            |this, range: Range<usize>, window, cx| {
                                let visible_images = range
                                    .clone()
                                    .filter_map(|index| this.store.messages.get(index))
                                    .filter_map(|message| match message.content.as_ref() {
                                        Some(proto::message::Content::Image(image))
                                            if image.thumbnail_path.is_empty()
                                                && image.downloadable =>
                                        {
                                            Some((message.chat_id.clone(), message.id.clone()))
                                        }
                                        _ => None,
                                    })
                                    .collect::<Vec<_>>();
                                for (chat_id, message_id) in visible_images {
                                    this.load_message_image(chat_id, message_id);
                                }
                                if this
                                    .store
                                    .selected_chat()
                                    .is_some_and(|chat| chat.kind() == proto::ChatKind::Group)
                                {
                                    let participants = range
                                        .clone()
                                        .filter_map(|index| this.store.messages.get(index))
                                        .filter(|message| {
                                            !message.from_me
                                                && message.sender_avatar_path.is_empty()
                                                && this
                                                    .store
                                                    .participant_avatar_path(&message.sender_id)
                                                    .is_none()
                                                && !message.sender_id.is_empty()
                                        })
                                        .map(|message| message.sender_id.clone())
                                        .collect::<HashSet<_>>();
                                    for participant_id in participants {
                                        this.load_participant_avatar(participant_id);
                                    }
                                }
                                if range.end == this.store.messages.len()
                                    && (this.store.has_newer_messages || this.store.newer_activity)
                                {
                                    this.load_newer_messages();
                                } else if window.is_window_active()
                                    && range.end == this.store.messages.len()
                                {
                                    this.mark_read();
                                }
                                range.map(|index| this.render_message(index, cx)).collect()
                            },
                        )
                        .track_scroll(&self.message_scroll)
                        .p_4()
                        .gap_2(),
                    ),
            )
            .when(has_older, |column| {
                column.child(Button::new("older").label("Load older messages").on_click(
                    cx.listener(|this, _, _, cx| {
                        this.load_older_messages();
                        cx.notify();
                    }),
                ))
            })
            .when(has_newer, |column| {
                column.child(
                    Button::new("jump-latest")
                        .primary()
                        .label("Jump to latest messages")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.refresh_latest();
                            cx.notify();
                        })),
                )
            })
            .when(!connected, |column| {
                column.child(
                    div()
                        .px_4()
                        .py_2()
                        .bg(if dark { rgb(0x594918) } else { rgb(0xffe9b5) })
                        .child("Offline — messages will be available after reconnection"),
                )
            })
            .when(self.emoji_target.is_some(), |column| {
                column.child(self.render_emoji_picker(cx))
            })
            .when(self.mention_popup_visible(), |column| {
                column.child(self.render_mention_picker(cx))
            })
            .child(
                v_flex()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .when_some(composer_typing_line, |composer, typing| {
                        composer.child(
                            div()
                                .h(px(24. * self.ui_scale))
                                .px_4()
                                .pt_1()
                                .text_xs()
                                .text_color(if dark {
                                    gpui::Hsla::from(rgb(0x25d366))
                                } else {
                                    gpui::Hsla::from(rgb(DARK_GREEN))
                                })
                                .child(typing),
                        )
                    })
                    .when_some(reply_context, |composer, (sender, preview)| {
                        composer.child(
                            h_flex()
                                .mx_3()
                                .mt_2()
                                .px_3()
                                .py_2()
                                .gap_3()
                                .rounded_md()
                                .bg(cx.theme().secondary)
                                .child(
                                    v_flex()
                                        .min_w_0()
                                        .flex_1()
                                        .child(
                                            div()
                                                .text_xs()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(rgb(0x25d366))
                                                .child(format!("Replying to {sender}")),
                                        )
                                        .child(
                                            div()
                                                .truncate()
                                                .text_sm()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(preview),
                                        ),
                                )
                                .child(Button::new("cancel-reply").small().label("×").on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.cancel_reply();
                                        cx.notify();
                                    }),
                                )),
                        )
                    })
                    .child(
                        h_flex()
                            .p_3()
                            .gap_2()
                            .child(
                                Button::new("emoji-picker")
                                    .label("😀")
                                    .disabled(!connected)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_composer_emoji();
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("send-image")
                                    .label("📷")
                                    .disabled(!connected)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.choose_image(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("send-sticker")
                                    .small()
                                    .label(if self.sticker_preparing {
                                        "Preparing…"
                                    } else {
                                        "Sticker"
                                    })
                                    .disabled(!connected || self.sticker_preparing)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.choose_sticker(window, cx)
                                    })),
                            )
                            .child(Input::new(&self.composer).disabled(!connected).flex_1())
                            .child(
                                Button::new("send")
                                    .primary()
                                    .label("Send")
                                    .disabled(!connected)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.send_text(window, cx)
                                    })),
                            ),
                    ),
            )
    }

    fn render_mention_picker(&self, cx: &mut Context<Self>) -> gpui::Div {
        let matches = self.mention_matches();
        let highlighted = self
            .mention_picker
            .as_ref()
            .map_or(0, |picker| picker.highlighted);
        let selected_background = cx.theme().secondary;
        let dark = cx.theme().mode.is_dark();
        let hover_background = if dark { rgb(0x2a3942) } else { rgb(0xf0f2f5) };
        v_flex()
            .mx_3()
            .mt_2()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .when(matches.is_empty(), |column| {
                column.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("Loading group members…"),
                )
            })
            .children(matches.into_iter().enumerate().map(|(index, participant)| {
                let name = mention_display_name(&participant);
                let phone = participant.phone_number.clone();
                h_flex()
                    .id(("mention-row", index))
                    .px_3()
                    .py_2()
                    .gap_3()
                    .items_center()
                    .cursor_pointer()
                    .when(index == highlighted, |row| row.bg(selected_background))
                    .hover(move |style| style.bg(hover_background))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.apply_mention(index, window, cx);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(name),
                    )
                    .when(!phone.is_empty(), |row| {
                        row.child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(phone),
                        )
                    })
            }))
    }

    fn render_emoji_picker(&self, cx: &mut Context<Self>) -> gpui::Div {
        let row_count = self.emoji_results.len().div_ceil(EMOJI_COLUMNS);
        let selected = self.emoji_category;
        let reaction_mode = matches!(self.emoji_target, Some(EmojiTarget::Reaction { .. }));
        let has_own_reaction = match &self.emoji_target {
            Some(EmojiTarget::Reaction { message_id, .. }) => self
                .store
                .messages
                .iter()
                .find(|message| &message.id == message_id)
                .is_some_and(|message| message.reactions.iter().any(|reaction| reaction.from_me)),
            _ => false,
        };
        v_flex()
            .h(px(300.))
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .child(Input::new(&self.emoji_search).flex_1())
                    .when(has_own_reaction, |row| {
                        row.child(
                            Button::new("remove-reaction")
                                .label("Remove mine")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.remove_reaction();
                                    cx.notify();
                                })),
                        )
                    })
                    .child(Button::new("close-emoji").label("×").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.emoji_target = None;
                            cx.notify();
                        },
                    ))),
            )
            .child(
                h_flex()
                    .px_3()
                    .gap_1()
                    .children(EmojiCategory::ALL.into_iter().enumerate().map(
                        |(index, category)| {
                            let button =
                                Button::new(("emoji-category", index)).label(category.label());
                            let button = if category == selected {
                                button.primary()
                            } else {
                                button
                            };
                            button.on_click(cx.listener(move |this, _, _, cx| {
                                this.set_emoji_category(category);
                                cx.notify();
                            }))
                        },
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(
                        uniform_list(
                            "emoji-results",
                            row_count,
                            cx.processor(|this, rows: Range<usize>, _, cx| {
                                rows.map(|row| {
                                    h_flex().h(px(42.)).px_3().gap_1().children(
                                        (0..EMOJI_COLUMNS).filter_map(|column| {
                                            let index = row * EMOJI_COLUMNS + column;
                                            let emoji = this.emoji_results.get(index)?;
                                            let value = emoji.as_str().to_string();
                                            Some(
                                                Button::new(("emoji", index))
                                                    .label(value.clone())
                                                    .on_click(cx.listener(
                                                        move |this, _, window, cx| {
                                                            this.choose_emoji(&value, window, cx);
                                                            cx.notify();
                                                        },
                                                    )),
                                            )
                                        }),
                                    )
                                })
                                .collect::<Vec<_>>()
                            }),
                        )
                        .h_full(),
                    )
                    .when(row_count == 0, |panel| {
                        panel.child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_color(cx.theme().muted_foreground)
                                .child("No emoji found"),
                        )
                    }),
            )
            .when(reaction_mode, |panel| {
                panel.child(
                    div()
                        .px_3()
                        .pb_1()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Choose one emoji to react"),
                )
            })
    }

    fn render_reaction_details(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let Some(details) = self.reaction_details.as_ref() else {
            return div();
        };
        let emoji = details.emoji.clone();
        let count = details.reactions.len();
        let list_height = px((count.clamp(1, 7) as f32) * 58.);
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .bg(rgba(0x00000088))
            .child(
                v_flex()
                    .w(px(420.))
                    .max_w_full()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .child(
                        h_flex()
                            .h(px(58.))
                            .px_4()
                            .items_center()
                            .justify_between()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(
                                v_flex()
                                    .child(
                                        div()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(format!("{emoji} Reactions")),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(format!(
                                                "{count} {}",
                                                if count == 1 { "person" } else { "people" }
                                            )),
                                    ),
                            )
                            .child(Button::new("close-reaction-details").label("×").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.reaction_details = None;
                                    cx.notify();
                                }),
                            )),
                    )
                    .child(
                        uniform_list(
                            "reaction-details-reactors",
                            count,
                            cx.processor(|this, range: Range<usize>, _, cx| {
                                let reactions = this
                                    .reaction_details
                                    .as_ref()
                                    .map(|details| details.reactions.clone())
                                    .unwrap_or_default();
                                let visible = range
                                    .filter_map(|index| {
                                        reactions
                                            .get(index)
                                            .cloned()
                                            .map(|reaction| (index, reaction))
                                    })
                                    .collect::<Vec<_>>();
                                for (_, reaction) in &visible {
                                    if reaction.sender_avatar_path.is_empty()
                                        && this
                                            .store
                                            .participant_avatar_path(&reaction.sender_id)
                                            .is_none()
                                    {
                                        this.load_participant_avatar(reaction.sender_id.clone());
                                    }
                                }
                                visible
                                    .into_iter()
                                    .map(|(index, reaction)| {
                                        let name = reaction_sender_name(&reaction).to_string();
                                        let detail = reaction_sender_detail(&reaction).to_string();
                                        let cached_avatar = this
                                            .store
                                            .participant_avatar_path(&reaction.sender_id)
                                            .unwrap_or_default();
                                        let avatar_path = if reaction.sender_avatar_path.is_empty()
                                        {
                                            cached_avatar
                                        } else {
                                            reaction.sender_avatar_path.as_str()
                                        };
                                        let avatar =
                                            avatar_element(&name, avatar_path, px(38.), cx);
                                        h_flex()
                                            .id(("reaction-detail-row", index))
                                            .h(px(58.))
                                            .px_4()
                                            .gap_3()
                                            .items_center()
                                            .child(avatar)
                                            .child(
                                                v_flex()
                                                    .min_w_0()
                                                    .child(
                                                        div()
                                                            .truncate()
                                                            .font_weight(if reaction.from_me {
                                                                FontWeight::SEMIBOLD
                                                            } else {
                                                                FontWeight::NORMAL
                                                            })
                                                            .child(name),
                                                    )
                                                    .child(
                                                        div()
                                                            .truncate()
                                                            .text_xs()
                                                            .text_color(cx.theme().muted_foreground)
                                                            .child(detail),
                                                    ),
                                            )
                                    })
                                    .collect::<Vec<_>>()
                            }),
                        )
                        .h(list_height),
                    ),
            )
    }

    fn render_settings(&self, cx: &mut Context<Self>) -> gpui::Div {
        let scale_percent = (self.ui_scale * 100.0).round() as u32;
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .bg(rgba(0x00000088))
            .child(
                v_flex()
                    .w(px(420.))
                    .max_w_full()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .child(
                        h_flex()
                            .h(px(58.))
                            .px_4()
                            .items_center()
                            .justify_between()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(div().font_weight(FontWeight::SEMIBOLD).child("Settings"))
                            .child(
                                Button::new("close-settings")
                                    .label("×")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.settings_open = false;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        v_flex()
                            .p_4()
                            .gap_3()
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(
                                        div().font_weight(FontWeight::SEMIBOLD).child("UI scale"),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(
                                                "Increase text, controls, and interface spacing.",
                                            ),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        Button::new("decrease-ui-scale")
                                            .label("−")
                                            .disabled(self.ui_scale <= MIN_UI_SCALE)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.adjust_ui_scale(-UI_SCALE_STEP, window, cx);
                                            })),
                                    )
                                    .child(
                                        div()
                                            .w(px(84.))
                                            .text_center()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(format!("{scale_percent}%")),
                                    )
                                    .child(
                                        Button::new("increase-ui-scale")
                                            .label("+")
                                            .disabled(self.ui_scale >= MAX_UI_SCALE)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.adjust_ui_scale(UI_SCALE_STEP, window, cx);
                                            })),
                                    )
                                    .child(
                                        Button::new("reset-ui-scale")
                                            .small()
                                            .label("Reset")
                                            .disabled(self.ui_scale == 1.0)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.set_ui_scale(1.0, window, cx);
                                            })),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Saved automatically for future launches."),
                            ),
                    ),
            )
    }

    fn render_chat_info(&mut self, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let Some(state) = self.chat_info.as_ref() else {
            return div().id("chat-info-backdrop");
        };
        let chat_id = state.chat_id.clone();
        let loading = state.loading;
        let error = state.error.clone();
        let info = state.info.clone();
        let chat = self.store.chat(&chat_id).cloned();
        let is_group = chat
            .as_ref()
            .is_some_and(|chat| chat.kind() == proto::ChatKind::Group);
        let title = chat
            .as_ref()
            .map(|chat| chat.title.clone())
            .unwrap_or_else(|| "Conversation".into());
        let avatar_path = chat
            .as_ref()
            .map(|chat| chat.avatar_path.clone())
            .unwrap_or_default();

        // Warm the avatars for the members that are about to be shown; the
        // media budget in the RPC layer keeps this from flooding the backend.
        if let Some(info) = info.as_ref() {
            let pending: Vec<String> = info
                .participants
                .iter()
                .take(24)
                .filter(|participant| {
                    self.store
                        .participant_avatar_path(&participant.participant_id)
                        .is_none()
                })
                .map(|participant| participant.participant_id.clone())
                .collect();
            for participant_id in pending {
                self.load_participant_avatar(participant_id);
            }
        }

        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let hover_background = cx.theme().muted;
        let section = |label: &'static str, value: String| {
            v_flex()
                .gap_1()
                .child(div().text_xs().text_color(muted).child(label))
                .child(div().text_sm().child(value))
        };

        let subtitle = if let Some(info) = info.as_ref().filter(|_| is_group) {
            format!("Group · {} members", info.participant_count)
        } else if is_group {
            "Group".to_string()
        } else {
            chat.as_ref()
                .map(|chat| chat.phone_number.clone())
                .unwrap_or_default()
        };
        let identity_avatar = avatar_element(&title, &avatar_path, px(96.), cx);

        let mut body = v_flex()
            .id("chat-info-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.chat_info_scroll)
            .on_scroll_wheel(cx.listener(|this, event, window, cx| {
                this.handle_smooth_scroll_input(ScrollSurface::ChatInfo, event, window, cx);
            }))
            .p_4()
            .gap_4()
            .child(
                v_flex()
                    .items_center()
                    .gap_2()
                    .child(identity_avatar)
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_center()
                            .child(title.clone()),
                    )
                    .when(!subtitle.is_empty(), |identity| {
                        identity.child(
                            div()
                                .text_sm()
                                .text_color(muted)
                                .text_center()
                                .child(subtitle),
                        )
                    }),
            );
        if loading {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(muted)
                    .text_center()
                    .child("Loading chat details…"),
            );
        }
        if let Some(error) = error {
            body = body.child(
                v_flex()
                    .items_center()
                    .gap_2()
                    .child(div().text_sm().text_color(rgb(0xef4444)).child(error))
                    .child(
                        Button::new("chat-info-retry")
                            .small()
                            .label("Retry")
                            .on_click(cx.listener({
                                let chat_id = chat_id.clone();
                                move |this, _, _, cx| {
                                    this.open_chat_info(chat_id.clone());
                                    cx.notify();
                                }
                            })),
                    ),
            );
        }
        if let Some(info) = info {
            if !is_group {
                if !info.about.is_empty() {
                    body = body.child(section("About", info.about.clone()));
                }
                if let Some(chat) = chat.as_ref() {
                    if !chat.phone_number.is_empty() {
                        body = body.child(section("Phone", chat.phone_number.clone()));
                    }
                    if !info.verified_name.is_empty() {
                        body = body.child(section("Verified business", info.verified_name.clone()));
                    }
                    if !chat.business_name.is_empty() && chat.business_name != chat.title {
                        body = body.child(section("Business name", chat.business_name.clone()));
                    }
                    if !chat.push_name.is_empty() && chat.push_name != chat.title {
                        body = body.child(section("Profile name", format!("~{}", chat.push_name)));
                    }
                }
            } else {
                let description = if info.description.is_empty() {
                    "No description".to_string()
                } else {
                    info.description.clone()
                };
                body = body.child(section("Description", description));
                if info.created_at_ms > 0 {
                    let mut created = format!("Created {}", format_epoch_date(info.created_at_ms));
                    if !info.created_by.is_empty() {
                        created.push_str(&format!(" by {}", info.created_by));
                    }
                    body = body.child(div().text_xs().text_color(muted).child(created));
                }
                if info.disappearing_timer_seconds > 0 {
                    body = body.child(section(
                        "Disappearing messages",
                        format_disappearing_timer(info.disappearing_timer_seconds),
                    ));
                }
                let mut notes: Vec<&'static str> = Vec::new();
                if info.is_community {
                    notes.push("Community group");
                }
                if info.announce_only {
                    notes.push("Only admins can send messages");
                }
                if info.locked {
                    notes.push("Only admins can edit group info");
                }
                if info.join_approval_required {
                    notes.push("Admins approve new members");
                }
                if !notes.is_empty() {
                    body =
                        body.child(v_flex().gap_1().children(notes.into_iter().map(|note| {
                            div().text_xs().text_color(muted).child(format!("• {note}"))
                        })));
                }
                if !info.participants.is_empty() {
                    const MAX_MEMBER_ROWS: usize = 200;
                    let mut members = v_flex().gap_1().child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(format!("{} members", info.participant_count)),
                    );
                    for (index, participant) in
                        info.participants.iter().take(MAX_MEMBER_ROWS).enumerate()
                    {
                        let display_name = if participant.is_me {
                            "You".to_string()
                        } else if participant.display_name.is_empty() {
                            participant.phone_number.clone()
                        } else {
                            participant.display_name.clone()
                        };
                        let member_avatar_path = self
                            .store
                            .participant_avatar_path(&participant.participant_id)
                            .filter(|path| !path.is_empty())
                            .unwrap_or_default();
                        let member_avatar =
                            avatar_element(&display_name, member_avatar_path, px(32.), cx);
                        let badge = if participant.is_super_admin {
                            Some("Owner")
                        } else if participant.is_admin {
                            Some("Admin")
                        } else {
                            None
                        };
                        let clickable =
                            !participant.is_me && !participant.participant_id.is_empty();
                        let mut row = h_flex()
                            .id(index)
                            .px_2()
                            .py_1()
                            .gap_2()
                            .items_center()
                            .rounded_md()
                            .child(member_avatar)
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(div().text_sm().truncate().child(display_name))
                                    .when(
                                        !participant.phone_number.is_empty() && !participant.is_me,
                                        |details| {
                                            details.child(
                                                div()
                                                    .text_xs()
                                                    .text_color(muted)
                                                    .truncate()
                                                    .child(participant.phone_number.clone()),
                                            )
                                        },
                                    ),
                            )
                            .when_some(badge, |row, badge| {
                                row.child(
                                    div()
                                        .px_2()
                                        .py_0p5()
                                        .rounded_sm()
                                        .border_1()
                                        .border_color(border)
                                        .text_xs()
                                        .text_color(muted)
                                        .child(badge),
                                )
                            });
                        if clickable {
                            row = row
                                .cursor_pointer()
                                .hover(move |style| style.bg(hover_background))
                                .on_click(cx.listener({
                                    let contact_jid = participant.participant_id.clone();
                                    move |this, _, _, cx| {
                                        this.chat_info = None;
                                        this.request(
                                            proto::rpc_request::Request::OpenContact(
                                                proto::OpenContactRequest {
                                                    contact_jid: contact_jid.clone(),
                                                },
                                            ),
                                            PendingRequest::OpenContact,
                                        );
                                        cx.notify();
                                    }
                                }));
                        }
                        members = members.child(row);
                    }
                    if info.participants.len() > MAX_MEMBER_ROWS {
                        members = members.child(div().text_xs().text_color(muted).child(format!(
                            "…and {} more",
                            info.participants.len() - MAX_MEMBER_ROWS
                        )));
                    }
                    body = body.child(members);
                }
            }
            let mut flags = Vec::new();
            if let Some(chat) = chat.as_ref() {
                if chat.pinned {
                    flags.push("Pinned");
                }
                if chat.muted {
                    flags.push("Muted");
                }
                if chat.archived {
                    flags.push("Archived");
                }
            }
            if !flags.is_empty() {
                body = body.child(h_flex().gap_2().children(flags.into_iter().map(|flag| {
                    div()
                        .px_2()
                        .py_0p5()
                        .rounded_sm()
                        .border_1()
                        .border_color(border)
                        .text_xs()
                        .text_color(muted)
                        .child(flag)
                })));
            }
            if !info.address.is_empty() {
                body = body.child(section("WhatsApp address", info.address.clone()));
            }
        }

        div()
            .id("chat-info-backdrop")
            .absolute()
            .inset_0()
            // Without occlusion, clicks fall through to the conversation
            // header below, whose click target immediately reopens the pane.
            .occlude()
            .bg(rgba(0x00000066))
            .flex()
            .justify_end()
            .on_click(cx.listener(|this, _, _, cx| {
                this.chat_info = None;
                cx.notify();
            }))
            .child(
                v_flex()
                    .w(px(400.))
                    .max_w_full()
                    .h_full()
                    // Occlude the backdrop so clicks inside the pane don't hit
                    // its close-on-click handler.
                    .occlude()
                    .bg(cx.theme().background)
                    .border_l_1()
                    .border_color(border)
                    .child(
                        h_flex()
                            .h(px(58.))
                            .px_4()
                            .items_center()
                            .justify_between()
                            .border_b_1()
                            .border_color(border)
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(if is_group {
                                "Group info"
                            } else {
                                "Contact info"
                            }))
                            .child(Button::new("close-chat-info").label("×").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.chat_info = None;
                                    cx.notify();
                                }),
                            )),
                    )
                    .child(body),
            )
    }

    fn render_chat_switcher(&self, cx: &mut Context<Self>) -> gpui::Div {
        let Some(switcher) = self.chat_switcher.as_ref() else {
            return div();
        };
        let highlighted = switcher.highlighted;
        let chats = switcher
            .chat_ids
            .iter()
            .filter_map(|chat_id| self.store.chat(chat_id).cloned())
            .collect::<Vec<_>>();
        let selected_background = if cx.theme().is_dark() {
            rgb(0x173f35)
        } else {
            rgb(0xd9fdd3)
        };
        let hover_background = if cx.theme().is_dark() {
            rgb(0x202c33)
        } else {
            rgb(0xf0f2f5)
        };
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .bg(rgba(0x00000088))
            .child(
                v_flex()
                    .w(px(500. * self.ui_scale))
                    .max_w_full()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .overflow_hidden()
                    .child(
                        h_flex()
                            .h(px(52. * self.ui_scale))
                            .px_4()
                            .items_center()
                            .justify_between()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Recent chats"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!("{} of {}", highlighted + 1, chats.len())),
                            ),
                    )
                    .children(chats.into_iter().enumerate().map(|(index, chat)| {
                        let chat_id = chat.id.clone();
                        let preview = if chat.last_message_preview.trim().is_empty() {
                            if chat.phone_number.trim().is_empty() {
                                "No messages yet".to_string()
                            } else {
                                chat.phone_number.clone()
                            }
                        } else {
                            chat.last_message_preview.clone()
                        };
                        let avatar = avatar_element(
                            &chat.title,
                            &chat.avatar_path,
                            px(40. * self.ui_scale),
                            cx,
                        );
                        h_flex()
                            .id(("recent-chat", index))
                            .h(px(64. * self.ui_scale))
                            .px_4()
                            .gap_3()
                            .items_center()
                            .overflow_hidden()
                            .cursor_pointer()
                            .when(index == highlighted, |row| row.bg(selected_background))
                            .hover(move |style| style.bg(hover_background))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.commit_chat_switcher_to(chat_id.clone(), window, cx);
                                cx.notify();
                            }))
                            .child(avatar)
                            .child(
                                v_flex()
                                    .min_w_0()
                                    .flex_1()
                                    .child(
                                        div()
                                            .w_full()
                                            .min_w_0()
                                            .truncate()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(one_line(&chat.title)),
                                    )
                                    .child(
                                        div()
                                            .w_full()
                                            .min_w_0()
                                            .truncate()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(one_line(&preview)),
                                    ),
                            )
                            .when(chat.archived, |row| {
                                row.child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("Archived"),
                                )
                            })
                    }))
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .border_t_1()
                            .border_color(cx.theme().border)
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Release Ctrl to switch · Esc to cancel"),
                    ),
            )
    }

    fn render_image_viewer(&self, cx: &mut Context<Self>) -> gpui::Div {
        let Some(viewer) = self.image_viewer.as_ref() else {
            return div();
        };
        let chat_id = viewer.chat_id.clone();
        let message_id = viewer.message_id.clone();
        let path = viewer.path.clone();
        let caption = viewer.caption.trim().to_string();
        let media_name = if viewer.sticker { "Sticker" } else { "Photo" };
        let opening_label = if viewer.sticker {
            "Opening sticker…"
        } else {
            "Opening photo…"
        };
        let fallback_label = if viewer.sticker {
            "This sticker couldn't be decoded. Try Reload."
        } else {
            "This photo couldn't be decoded. Try Reload."
        };
        div()
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .flex_col()
            .p_4()
            .bg(rgba(0x050807f2))
            .text_color(rgb(0xffffff))
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(media_name))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Button::new("reload-viewed-image").label("Reload").on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.retry_message_image(chat_id.clone(), message_id.clone());
                                    cx.notify();
                                }),
                            ))
                            .child(Button::new("close-image-viewer").label("×").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.image_viewer = None;
                                    cx.notify();
                                }),
                            )),
                    ),
            )
            .child(
                div().flex_1().min_h_0().min_w_0().p_4().child(
                    img(PathBuf::from(path))
                        .size_full()
                        .object_fit(ObjectFit::Contain)
                        .with_loading(move || {
                            div()
                                .size_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(opening_label)
                                .into_any_element()
                        })
                        .with_fallback(move || {
                            div()
                                .size_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(fallback_label)
                                .into_any_element()
                        }),
                ),
            )
            .when(!caption.is_empty(), |viewer| {
                viewer.child(
                    div()
                        .w_full()
                        .max_h(px(96.))
                        .overflow_hidden()
                        .px_4()
                        .py_2()
                        .text_sm()
                        .child(caption),
                )
            })
    }

    fn render_linkified_text(
        &self,
        message_id: &str,
        text: &str,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let mut lines = Vec::new();
        for (line_index, line) in text.split('\n').enumerate() {
            let mut row = h_flex().flex_wrap();
            if line.is_empty() {
                row = row.child(div().child(" "));
            }
            for (token_index, token) in line.split_whitespace().enumerate() {
                if let Some((prefix, url, suffix)) = split_url_token(token) {
                    if !prefix.is_empty() {
                        row = row.child(div().child(prefix.to_string()));
                    }
                    let target = url.to_string();
                    row = row.child(
                        div()
                            .id(format!(
                                "message-link-{message_id}-{line_index}-{token_index}"
                            ))
                            .cursor_pointer()
                            .text_color(rgb(0x0b57d0))
                            .underline()
                            .child(url.to_string())
                            .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&target))),
                    );
                    row = row.child(div().child(format!("{suffix} ")));
                } else {
                    row = row.child(div().child(format!("{token} ")));
                }
            }
            lines.push(row.into_any_element());
        }
        div().child(v_flex().children(lines))
    }

    fn render_link_preview(
        &self,
        index: usize,
        preview: proto::LinkPreview,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let target = is_safe_web_url(&preview.url).then(|| preview.url.clone());
        let host = link_preview_host(&preview.url).to_string();
        let title = if preview.title.trim().is_empty() {
            host.clone()
        } else {
            preview.title.clone()
        };
        let thumbnail = (!preview.jpeg_thumbnail.is_empty()).then(|| {
            Arc::new(gpui::Image::from_bytes(
                gpui::ImageFormat::Jpeg,
                preview.jpeg_thumbnail,
            ))
        });
        v_flex()
            .id(("link-preview", index))
            .w(px(320.))
            .max_w_full()
            .overflow_hidden()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().secondary)
            .when_some(thumbnail, |card, thumbnail| {
                card.child(
                    img(thumbnail)
                        .w_full()
                        .h(px(144.))
                        .object_fit(ObjectFit::Cover),
                )
            })
            .child(
                v_flex()
                    .p_2()
                    .gap_1()
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .when(!preview.description.trim().is_empty(), |details| {
                        details.child(
                            div()
                                .truncate()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(preview.description),
                        )
                    })
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(host),
                    ),
            )
            .when_some(target, |card, target| {
                card.cursor_pointer()
                    .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&target)))
            })
    }

    fn render_message(&self, index: usize, cx: &mut Context<Self>) -> gpui::Div {
        let message = &self.store.messages[index];
        let text_content = match message.content.as_ref() {
            Some(proto::message::Content::Text(text)) => Some(text.clone()),
            _ => None,
        };
        let image_content = match message.content.as_ref() {
            Some(proto::message::Content::Image(image)) => Some(image.clone()),
            _ => None,
        };
        let sticker = image_content.as_ref().is_some_and(|image| image.sticker);
        let text = if sticker {
            String::new()
        } else {
            message_text(message).to_string()
        };
        let reaction_chips = reaction_counts(message);
        let reply_target_id = message.reply_to_message_id.clone();
        let reply_target = self.store.message(&reply_target_id).cloned();
        let reply_count = self.store.reply_count(&message.id);
        let first_reply_id = self.store.first_reply_id(&message.id).map(str::to_owned);
        let chat_id = message.chat_id.clone();
        let message_id = message.id.clone();
        let body_message_id = message.id.clone();
        let search_target = self.search_target_message_id.as_deref() == Some(message.id.as_str());
        let reply_action_message_id = message_id.clone();
        let message_image_chat_id = chat_id.clone();
        let message_image_id = message_id.clone();
        let image_failure = self
            .image_failures
            .get(&(chat_id.clone(), message_id.clone()))
            .cloned();
        let reaction_chat_id = chat_id.clone();
        let reaction_message_id = message_id.clone();
        let own_reaction_background: gpui::Hsla = if cx.theme().is_dark() {
            rgb(0x164e3d).into()
        } else {
            rgb(0xd9fdd3).into()
        };
        let reaction_hover = if cx.theme().is_dark() {
            rgb(0x245c4e)
        } else {
            rgb(0xc8f4c1)
        };
        let group_incoming = !message.from_me
            && self
                .store
                .selected_chat()
                .is_some_and(|chat| chat.kind() == proto::ChatKind::Group);
        let meta_text = {
            let mut segments: Vec<&str> = Vec::new();
            if message.edited {
                segments.push("edited");
            }
            if message.from_me {
                let status = message_status(message.status());
                if !status.is_empty() {
                    segments.push(status);
                }
            }
            segments.join(" · ")
        };
        h_flex()
            .w_full()
            .items_start()
            .gap_2()
            .when(message.from_me, |row| row.justify_end())
            .when(group_incoming, |row| {
                let avatar_path = if message.sender_avatar_path.is_empty() {
                    self.store
                        .participant_avatar_path(&message.sender_id)
                        .unwrap_or_default()
                } else {
                    message.sender_avatar_path.as_str()
                };
                row.child(avatar_element(
                    &message.sender_name,
                    avatar_path,
                    px(34.),
                    cx,
                ))
            })
            .child(
                v_flex()
                    .max_w(px(560.))
                    .when(!sticker, |bubble| bubble.px_3().py_2())
                    .rounded_lg()
                    .when(search_target, |bubble| {
                        bubble.border_2().border_color(rgb(0x25d366))
                    })
                    .gap_1()
                    .bg(if sticker {
                        gpui::transparent_black()
                    } else if message.from_me {
                        if cx.theme().is_dark() {
                            rgb(0x173f35).into()
                        } else {
                            gpui::hsla(0.31, 0.87, 0.91, 1.0)
                        }
                    } else {
                        cx.theme().background
                    })
                    .when(group_incoming, |bubble| {
                        let name_color = cx.theme().blue.hue(identity_hue(&message.sender_name));
                        let show_number = !message.sender_phone_number.is_empty()
                            && message.sender_phone_number != message.sender_name;
                        bubble.child(
                            h_flex()
                                .items_baseline()
                                .gap_2()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(name_color)
                                        .child(message.sender_name.clone()),
                                )
                                .when(show_number, |header| {
                                    header.child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(message.sender_phone_number.clone()),
                                    )
                                }),
                        )
                    })
                    .when(!reply_target_id.is_empty(), |bubble| {
                        let target_id = reply_target_id.clone();
                        let (sender, preview) = reply_target.as_ref().map_or_else(
                            || {
                                (
                                    "Original message".to_string(),
                                    "Message unavailable".to_string(),
                                )
                            },
                            |target| {
                                let sender = if target.from_me {
                                    "You".to_string()
                                } else if !target.sender_name.trim().is_empty() {
                                    target.sender_name.trim().to_string()
                                } else {
                                    "Unknown contact".to_string()
                                };
                                (sender, message_preview(target))
                            },
                        );
                        bubble.child(
                            v_flex()
                                .id(("quoted-message", index))
                                .w_full()
                                .min_w(px(180.))
                                .px_3()
                                .py_2()
                                .rounded_md()
                                .border_l_2()
                                .border_color(rgb(0x25d366))
                                .bg(cx.theme().secondary)
                                .cursor_pointer()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(0x25d366))
                                        .child(sender),
                                )
                                .child(
                                    div()
                                        .truncate()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(preview),
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.scroll_to_message(&target_id);
                                    cx.notify();
                                })),
                        )
                    })
                    .when_some(image_content, |bubble, image| {
                        let media_name = if image.sticker { "sticker" } else { "photo" };
                        let (image_width, image_height) = image_render_size(&image);
                        let image_path = image_row_path(&image).to_string();
                        let viewer_path = image_viewer_path(&image).to_string();
                        let image_is_cached = !image_path.is_empty()
                            && !viewer_path.is_empty()
                            && PathBuf::from(&image_path).is_file()
                            && PathBuf::from(&viewer_path).is_file();
                        let cached_path_missing =
                            (!image_path.is_empty() || !viewer_path.is_empty()) && !image_is_cached;
                        let retryable =
                            image.downloadable && (image_failure.is_some() || cached_path_missing);
                        let placeholder = if cached_path_missing {
                            format!("Cached {media_name} missing · Click to reload")
                        } else if let Some(failure) = image_failure.clone() {
                            failure
                        } else if image.downloadable {
                            format!("Loading {media_name}…")
                        } else if image.sticker {
                            "Sticker unavailable".to_string()
                        } else {
                            "Photo unavailable".to_string()
                        };
                        let viewer_chat_id = message_image_chat_id.clone();
                        let viewer_message_id = message_image_id.clone();
                        let viewer_caption = image.caption.clone();
                        let viewer_sticker = image.sticker;
                        let retry_chat_id = message_image_chat_id.clone();
                        let retry_message_id = message_image_id.clone();
                        let loading_color = cx.theme().muted_foreground;
                        let fallback_color = cx.theme().muted_foreground;
                        let decoding_label = format!("Decoding {media_name}…");
                        let decode_failure_label =
                            format!("Couldn't display {media_name} · Open to retry");
                        bubble.child(
                            div()
                                .id(("message-image", index))
                                .w(image_width)
                                .h(image_height)
                                .max_w_full()
                                .overflow_hidden()
                                .rounded_md()
                                .when(!image.sticker, |container| {
                                    container
                                        .border_1()
                                        .border_color(cx.theme().border)
                                        .bg(cx.theme().secondary)
                                })
                                .when(image_is_cached, |container| {
                                    container.child(
                                        img(PathBuf::from(image_path.clone()))
                                            .size_full()
                                            .object_fit(ObjectFit::Contain)
                                            .with_loading(move || {
                                                div()
                                                    .size_full()
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .text_sm()
                                                    .text_color(loading_color)
                                                    .child(decoding_label.clone())
                                                    .into_any_element()
                                            })
                                            .with_fallback(move || {
                                                div()
                                                    .size_full()
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .px_3()
                                                    .text_sm()
                                                    .text_color(fallback_color)
                                                    .child(decode_failure_label.clone())
                                                    .into_any_element()
                                            }),
                                    )
                                })
                                .when(image_is_cached, |container| {
                                    container.cursor_pointer().on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.open_image_viewer(
                                                viewer_chat_id.clone(),
                                                viewer_message_id.clone(),
                                                viewer_path.clone(),
                                                viewer_caption.clone(),
                                                viewer_sticker,
                                            );
                                            cx.notify();
                                        },
                                    ))
                                })
                                .when(!image_is_cached, |container| {
                                    container.child(
                                        div()
                                            .size_full()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .px_3()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(placeholder),
                                    )
                                })
                                .when(retryable, |container| {
                                    container.cursor_pointer().on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.retry_message_image(
                                                retry_chat_id.clone(),
                                                retry_message_id.clone(),
                                            );
                                            cx.notify();
                                        },
                                    ))
                                }),
                        )
                    })
                    .when_some(
                        text_content.and_then(|text| text.link_preview),
                        |bubble, preview| {
                            bubble.child(self.render_link_preview(index, preview, cx))
                        },
                    )
                    .when(!text.is_empty(), |bubble| {
                        bubble.child(self.render_linkified_text(&body_message_id, &text, cx))
                    })
                    .when(!reaction_chips.is_empty(), |bubble| {
                        bubble.child(
                            h_flex()
                                .gap_1()
                                .children(reaction_chips.into_iter().enumerate().map(
                                    |(chip_index, reaction)| {
                                        let emoji = reaction.emoji.to_string();
                                        let clicked_emoji = emoji.clone();
                                        let clicked_chat_id = reaction_chat_id.clone();
                                        let clicked_message_id = reaction_message_id.clone();
                                        div()
                                            .id(format!("reaction-chip-{index}-{chip_index}"))
                                            .px_2()
                                            .py_1()
                                            .rounded_lg()
                                            .border_1()
                                            .border_color(if reaction.from_me {
                                                rgb(0x25d366).into()
                                            } else {
                                                cx.theme().border
                                            })
                                            .bg(if reaction.from_me {
                                                own_reaction_background
                                            } else {
                                                cx.theme().secondary
                                            })
                                            .hover(move |style| style.bg(reaction_hover))
                                            .cursor_pointer()
                                            .text_xs()
                                            .child(format!("{emoji} {}", reaction.count))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.open_reaction_details(
                                                    clicked_chat_id.clone(),
                                                    clicked_message_id.clone(),
                                                    clicked_emoji.clone(),
                                                );
                                                cx.notify();
                                            }))
                                    },
                                )),
                        )
                    })
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1()
                            .when(!meta_text.is_empty(), |footer| {
                                footer.child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(meta_text.clone()),
                                )
                            })
                            .when(reply_count > 0, |footer| {
                                let reply_id = first_reply_id.clone().unwrap_or_default();
                                footer.child(
                                    Button::new(("view-replies", index))
                                        .ghost()
                                        .small()
                                        .label(if reply_count == 1 {
                                            "1 reply".to_string()
                                        } else {
                                            format!("{reply_count} replies")
                                        })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.scroll_to_message(&reply_id);
                                            cx.notify();
                                        })),
                                )
                            })
                            .child(
                                Button::new(("reply-message", index))
                                    .ghost()
                                    .small()
                                    .icon(IconName::Undo)
                                    .tooltip("Reply")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.start_reply(reply_action_message_id.clone());
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new(("react-message", index))
                                    .ghost()
                                    .small()
                                    .icon(IconName::Heart)
                                    .tooltip("React")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.open_reaction_picker(
                                            chat_id.clone(),
                                            message_id.clone(),
                                        );
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
    }

    /// The height of every message, measured with the text system of the
    /// window and cached by the width bucket of the bubble.
    fn measure_message_heights(
        &mut self,
        wrap_width: Pixels,
        font_size: Pixels,
        window: &mut Window,
    ) {
        let width_bucket = (wrap_width / px(8.)).round() as i32;
        if !self.store.message_sizes_need_measurement(width_bucket) {
            return;
        }
        let line_height = window.line_height();
        let mut sizes = Vec::with_capacity(self.store.messages.len());
        for index in 0..self.store.messages.len() {
            let height = if let Some(height) = {
                let message = &self.store.messages[index];
                self.store.cached_message_height(&message.id, width_bucket)
            } {
                height
            } else {
                let (id, text) = {
                    let message = &self.store.messages[index];
                    (message.id.clone(), message_text(message).to_string())
                };
                let text_height = if text.is_empty() {
                    px(0.)
                } else {
                    let text: SharedString = text.into();
                    let run = window.text_style().to_run(text.len());
                    window
                        .text_system()
                        .shape_text(text, font_size, &[run], Some(wrap_width), None)
                        .map(|lines| {
                            lines.iter().fold(px(0.), |height, line| {
                                height + line.size(line_height).height
                            })
                        })
                        .unwrap_or(line_height)
                };
                let image_height = match self.store.messages[index].content.as_ref() {
                    Some(proto::message::Content::Image(image)) => {
                        image_render_size(image).1
                            + if text_height > px(0.) { px(4.) } else { px(0.) }
                    }
                    _ => px(0.),
                };
                let link_preview_height = match self.store.messages[index].content.as_ref() {
                    Some(proto::message::Content::Text(text)) if text.link_preview.is_some() => {
                        let thumbnail = text
                            .link_preview
                            .as_ref()
                            .is_some_and(|preview| !preview.jpeg_thumbnail.is_empty());
                        px(if thumbnail { 210. } else { 70. } * self.ui_scale)
                    }
                    _ => px(0.),
                };
                let reaction_height = if reaction_counts(&self.store.messages[index]).is_empty() {
                    px(0.)
                } else {
                    px(30. * self.ui_scale)
                };
                let reply_height = if self.store.messages[index].reply_to_message_id.is_empty() {
                    px(0.)
                } else {
                    px(54. * self.ui_scale)
                };
                // Group messages carry a sender name/number line above the body.
                let sender_header_height = if !self.store.messages[index].from_me
                    && self
                        .store
                        .selected_chat()
                        .is_some_and(|chat| chat.kind() == proto::ChatKind::Group)
                {
                    px(22. * self.ui_scale)
                } else {
                    px(0.)
                };
                let height = text_height
                    + image_height
                    + link_preview_height
                    + px(40. * self.ui_scale)
                    + reaction_height
                    + reply_height
                    + sender_header_height;
                // The model stores the heights as plain numbers: GPUI's
                // `Pixels` is a newtype over the same `f32`.
                let height = f32::from(height);
                self.store.cache_message_height(id, width_bucket, height);
                height
            };
            sizes.push(geom::size(geom::px(1.), height));
        }
        self.store.set_measured_message_sizes(width_bucket, sizes);
    }
}

impl Render for Whatfast {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match self.store.screen {
            Screen::Starting => self.render_center(
                "Starting Whatfast",
                "Connecting to the local WhatsApp backend",
            ),
            Screen::Pairing => self.render_pairing(cx),
            Screen::Syncing => self.render_center(
                "Syncing your chats",
                format!(
                    "{} chats · {} messages",
                    self.store.sync_chats, self.store.sync_messages
                ),
            ),
            Screen::Chats => self.render_chats(window, cx),
            Screen::Fatal => self.render_center(
                "Whatfast needs attention",
                self.store
                    .fatal_error
                    .clone()
                    .unwrap_or_else(|| "Unknown backend error".into()),
            ),
        };
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &CycleRecentChat, window, cx| {
                this.cycle_recent_chat(false, window, cx);
                cx.stop_propagation();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CycleRecentChatReverse, window, cx| {
                this.cycle_recent_chat(true, window, cx);
                cx.stop_propagation();
                cx.notify();
            }))
            .on_modifiers_changed(
                cx.listener(|this, event: &ModifiersChangedEvent, window, cx| {
                    if this.chat_switcher.is_some() && !event.modifiers.control {
                        this.commit_chat_switcher(window, cx);
                        cx.notify();
                    }
                }),
            )
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.modifiers.secondary() && event.keystroke.key == "k" {
                    this.focus_search(window, cx);
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                let composer_focused = this.composer.read(cx).focus_handle(cx).is_focused(window);
                let emoji_focused = this
                    .emoji_search
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window);
                let search_focused = this
                    .search_input
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window);
                if event.keystroke.key == "/"
                    && !event.keystroke.modifiers.modified()
                    && !composer_focused
                    && !emoji_focused
                    && !search_focused
                {
                    this.focus_search(window, cx);
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                if composer_focused && this.mention_popup_visible() {
                    let matches_len = this.mention_matches().len();
                    let key = event.keystroke.key.as_str();
                    let handled = match key {
                        "down" | "up" if matches_len > 0 => {
                            if let Some(picker) = this.mention_picker.as_mut() {
                                picker.highlighted = if key == "down" {
                                    (picker.highlighted + 1) % matches_len
                                } else {
                                    (picker.highlighted + matches_len - 1) % matches_len
                                };
                            }
                            true
                        }
                        "enter" | "tab" if matches_len > 0 => {
                            let index = this
                                .mention_picker
                                .as_ref()
                                .map_or(0, |picker| picker.highlighted.min(matches_len - 1));
                            this.apply_mention(index, window, cx);
                            true
                        }
                        "escape" => {
                            this.mention_picker = None;
                            true
                        }
                        _ => false,
                    };
                    if handled {
                        cx.stop_propagation();
                        cx.notify();
                        return;
                    }
                }
                if search_focused && this.search_query.trim().chars().count() >= 2 {
                    match event.keystroke.key.as_str() {
                        "down" => this.move_search_selection(false),
                        "up" => this.move_search_selection(true),
                        "enter" => this.activate_search_result(window, cx),
                        "escape" => this.clear_search(window, cx),
                        _ => return,
                    }
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                if this.chat_info.is_some() && event.keystroke.key == "escape" {
                    this.chat_info = None;
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                if this.chat_switcher.is_some() && event.keystroke.key == "escape" {
                    this.cancel_chat_switcher(window, cx);
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .child(content)
            .when_some(self.store.toast_error.clone(), |root, error| {
                root.child(
                    div()
                        .absolute()
                        .bottom_4()
                        .right_4()
                        .max_w(px(420.))
                        .p_3()
                        .rounded_lg()
                        .bg(rgb(0x7f1d1d))
                        .text_color(rgb(0xffffff))
                        .child(error),
                )
            })
            .when(self.reaction_details.is_some(), |root| {
                root.child(self.render_reaction_details(cx))
            })
            .when(self.image_viewer.is_some(), |root| {
                root.child(self.render_image_viewer(cx))
            })
            .when(self.settings_open, |root| {
                root.child(self.render_settings(cx))
            })
            .when(self.chat_info.is_some(), |root| {
                root.child(self.render_chat_info(cx))
            })
            .when(self.chat_switcher.is_some(), |root| {
                root.child(self.render_chat_switcher(cx))
            })
            .when_some(self.media_editor.clone(), |root, editor| {
                root.child(editor)
            })
    }
}
