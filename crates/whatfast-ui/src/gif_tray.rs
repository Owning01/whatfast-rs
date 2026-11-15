//! The GIF search and send tray above the composer (`PanelTab::Gif`).
//!
//! Uses `ureq` for debounced search + trending GIFs, caches preview files in
//! `<data_dir>/cache/gifs/`, and sends the chosen GIF to the active chat.

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AppContext as _, Context, Entity, InteractiveElement as _, ObjectFit, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, StyledImage as _, div, img, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use whatfast_i18n as i18n;
use whatfast_protocol::AttachmentKind;

use crate::app::{PanelTab, Whatfast};

pub(crate) const TRAY_HEIGHT: f32 = 420.0;

#[derive(Debug, Clone)]
pub(crate) struct GifItem {
    pub id: String,
    pub title: String,
    pub media_url: String,
    pub local_path: Option<PathBuf>,
}

#[derive(Default)]
struct GifTrayState {
    search_input: Option<Entity<InputState>>,
    _search_sub: Option<gpui::Subscription>,
    query: String,
    items: Vec<GifItem>,
    loading: bool,
    error: Option<String>,
    initialized: bool,
    search_seq: u64,
}

#[derive(Clone, Default)]
pub(crate) struct GifTray {
    state: Rc<RefCell<GifTrayState>>,
}

impl GifTray {
    pub(crate) fn cache_dir() -> PathBuf {
        whatfast_model::paths::data_dir().join("cache").join("gifs")
    }

    pub(crate) fn ensure_loaded(&self, cx: &mut Context<Whatfast>) {
        let should_fetch = {
            let mut st = self.state.borrow_mut();
            if st.initialized {
                false
            } else {
                st.initialized = true;
                true
            }
        };
        if should_fetch {
            self.fetch_gifs(String::new(), cx);
        }
    }

    fn fetch_gifs(&self, query: String, cx: &mut Context<Whatfast>) {
        let seq = {
            let mut st = self.state.borrow_mut();
            st.search_seq = st.search_seq.wrapping_add(1);
            st.loading = true;
            st.error = None;
            st.search_seq
        };
        let state = self.state.clone();
        cx.spawn(async move |view, cx| {
            let trimmed = query.trim().to_string();
            let result = cx
                .background_executor()
                .spawn(async move { fetch_and_cache_gifs(&trimmed) })
                .await;
            let _ = view.update(cx, |_, cx| {
                let mut st = state.borrow_mut();
                if st.search_seq != seq {
                    return;
                }
                st.loading = false;
                match result {
                    Ok(items) => st.items = items,
                    Err(err) => {
                        st.error = Some(err);
                        if st.items.is_empty() {
                            st.items = load_cached_gifs_fallback();
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn render(
        &self,
        app: &Whatfast,
        window: &mut gpui::Window,
        cx: &mut Context<Whatfast>,
    ) -> gpui::Stateful<gpui::Div> {
        let scale = app.ui_scale;
        let theme = cx.theme();
        let border = theme.border;
        let bg = theme.background;
        let secondary = theme.secondary;
        let muted = theme.muted_foreground;

        let search_entity = {
            let mut st = self.state.borrow_mut();
            if st.search_input.is_none() {
                let input = cx.new(|cx| {
                    InputState::new(window, cx).placeholder(i18n::t("Search GIFs"))
                });
                let state_ref = self.state.clone();
                let tray = self.clone();
                let sub = cx.subscribe(&input, move |_, input, event: &InputEvent, cx| {
                    if let InputEvent::Change = event {
                        let q = input.read(cx).value().to_string();
                        let seq = {
                            let mut st = state_ref.borrow_mut();
                            st.query = q.clone();
                            st.search_seq = st.search_seq.wrapping_add(1);
                            st.search_seq
                        };
                        let tray_debounced = tray.clone();
                        let state_debounced = state_ref.clone();
                        cx.spawn(async move |view, cx| {
                            cx.background_executor()
                                .timer(Duration::from_millis(320))
                                .await;
                            let _ = view.update(cx, |_, cx| {
                                if state_debounced.borrow().search_seq == seq {
                                    tray_debounced.fetch_gifs(q, cx);
                                }
                            });
                        })
                        .detach();
                    }
                });
                st._search_sub = Some(sub);
                st.search_input = Some(input);
            }
            st.search_input.clone()
        };

        let (items, loading, error) = {
            let st = self.state.borrow();
            (st.items.clone(), st.loading, st.error.clone())
        };

        v_flex()
            .id("gif-tray")
            .h(px(TRAY_HEIGHT * scale))
            .w(px(360. * scale))
            .max_w_full()
            .rounded_xl()
            .border_1()
            .border_color(border)
            .bg(bg)
            .shadow_lg()
            .overflow_hidden()
            .child(
                h_flex()
                    .p_2()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(border)
                    .child(
                        Icon::new(IconName::Search)
                            .size(px(16. * scale))
                            .text_color(muted),
                    )
                    .when_some(search_entity, |el, input| {
                        el.child(div().flex_1().child(Input::new(&input).cleanable(true)))
                    }),
            )
            .child(
                v_flex()
                    .id("gif-grid-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_2()
                    .gap_2()
                    .when(loading && items.is_empty(), |el| {
                        el.child(
                            div()
                                .py_8()
                                .text_center()
                                .text_xs()
                                .text_color(muted)
                                .child(i18n::t("Loading GIFs…")),
                        )
                    })
                    .when_some(error, |el, err| {
                        el.child(
                            div()
                                .px_2()
                                .py_1()
                                .rounded_md()
                                .bg(secondary)
                                .text_xs()
                                .text_color(muted)
                                .child(err),
                        )
                    })
                    .when(!loading && items.is_empty(), |el| {
                        el.child(
                            div()
                                .py_8()
                                .text_center()
                                .text_xs()
                                .text_color(muted)
                                .child(i18n::t("No GIFs found")),
                        )
                    })
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap_2()
                            .children(items.into_iter().enumerate().map(|(idx, item)| {
                                let gif_id = item.id.clone();
                                let media_url = item.media_url.clone();
                                let local = item.local_path.clone();
                                let label = if item.title.is_empty() {
                                    "GIF".to_string()
                                } else {
                                    item.title.clone()
                                };
                                div()
                                    .id(("gif-cell", idx))
                                    .w(px(104. * scale))
                                    .h(px(88. * scale))
                                    .rounded_md()
                                    .overflow_hidden()
                                    .bg(secondary)
                                    .cursor_pointer()
                                    .hover(|s| s.opacity(0.85))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.send_gif_item(gif_id.clone(), media_url.clone(), local.clone(), cx);
                                    }))
                                    .map(|cell| {
                                        if let Some(path) = item.local_path {
                                            cell.child(
                                                img(path)
                                                    .size_full()
                                                    .object_fit(ObjectFit::Cover),
                                            )
                                        } else {
                                            cell.flex()
                                                .items_center()
                                                .justify_center()
                                                .text_xs()
                                                .text_color(muted)
                                                .child(label)
                                        }
                                    })
                            })),
                    ),
            )
            .child(app.render_panel_tabs(cx))
    }
}

fn url_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn fetch_and_cache_gifs(query: &str) -> Result<Vec<GifItem>, String> {
    let url = if query.is_empty() {
        "https://g.tenor.com/v1/trending?key=LIVDSRZULELA&limit=18&media_filter=minimal".to_string()
    } else {
        format!(
            "https://g.tenor.com/v1/search?q={}&key=LIVDSRZULELA&limit=18&media_filter=minimal",
            url_encode(query)
        )
    };
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(6)))
        .build()
        .new_agent();
    let raw = agent
        .get(&url)
        .call()
        .map_err(|e| format!("GIF search unavailable: {e}"))?
        .into_body()
        .read_to_vec()
        .map_err(|e| format!("Failed to read GIF response: {e}"))?;
    let body: serde_json::Value =
        serde_json::from_slice(&raw).map_err(|e| format!("Invalid GIF response: {e}"))?;

    let cache_dir = GifTray::cache_dir();
    let _ = std::fs::create_dir_all(&cache_dir);

    let mut items = Vec::new();
    if let Some(results) = body.get("results").and_then(|v| v.as_array()) {
        for entry in results {
            let id = entry
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            if id.is_empty() {
                continue;
            }
            let title = entry
                .get("content_description")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let media_obj = entry
                .get("media")
                .and_then(|m| m.as_array())
                .and_then(|arr| arr.first());
            let preview_url = media_obj
                .and_then(|m| m.get("tinygif").or_else(|| m.get("nanogif")))
                .and_then(|g| g.get("url"))
                .and_then(|u| u.as_str())
                .unwrap_or_default()
                .to_string();
            let media_url = media_obj
                .and_then(|m| m.get("mp4").or_else(|| m.get("gif")))
                .and_then(|g| g.get("url"))
                .and_then(|u| u.as_str())
                .unwrap_or(&preview_url)
                .to_string();
            if preview_url.is_empty() {
                continue;
            }
            let safe_id: String = id
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            let local = cache_dir.join(format!("{safe_id}.gif"));
            if !local.exists()
                && let Ok(resp) = agent.get(&preview_url).call()
                && let Ok(bytes) = resp.into_body().read_to_vec()
            {
                let _ = std::fs::write(&local, &bytes);
            }
            items.push(GifItem {
                id,
                title,
                media_url,
                local_path: local.exists().then_some(local),
            });
        }
    }
    Ok(items)
}

fn load_cached_gifs_fallback() -> Vec<GifItem> {
    let dir = GifTray::cache_dir();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) == Some("gif") {
                let id = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("gif")
                    .to_string();
                Some(GifItem {
                    id,
                    title: "Cached GIF".to_string(),
                    media_url: String::new(),
                    local_path: Some(path),
                })
            } else {
                None
            }
        })
        .take(18)
        .collect()
}

impl Whatfast {
    pub(crate) fn send_gif_item(
        &mut self,
        id: String,
        media_url: String,
        local_preview: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        let Some(chat_id) = self.store.selected_chat_id.clone() else {
            self.store.toast_error = Some(i18n::t("Choose a conversation before sending").into());
            cx.notify();
            return;
        };
        self.panel_tab = PanelTab::Emoji;
        self.gif_tray_open = false;
        self.emoji_target = None;
        cx.notify();

        cx.spawn(async move |view, cx| {
            let path = cx
                .background_executor()
                .spawn(async move { download_sendable_gif(&id, &media_url, local_preview.as_deref()) })
                .await;
            let _ = view.update(cx, |this, cx| {
                match path {
                    Ok((send_path, kind)) => {
                        this.send_attachment_to(chat_id, send_path, kind, String::new(), false);
                    }
                    Err(err) => {
                        this.store.toast_error = Some(err);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

fn download_sendable_gif(
    id: &str,
    media_url: &str,
    local_preview: Option<&Path>,
) -> Result<(PathBuf, AttachmentKind), String> {
    let dir = GifTray::cache_dir();
    let _ = std::fs::create_dir_all(&dir);
    let safe_id: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();

    if !media_url.is_empty() {
        let is_mp4 = media_url.contains(".mp4");
        let ext = if is_mp4 { "mp4" } else { "gif" };
        let target = dir.join(format!("{safe_id}-full.{ext}"));
        if !target.exists() {
            let agent = ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(12)))
                .build()
                .new_agent();
            if let Ok(resp) = agent.get(media_url).call()
                && let Ok(bytes) = resp.into_body().read_to_vec()
            {
                let _ = std::fs::write(&target, &bytes);
            }
        }
        if target.exists() {
            let kind = if is_mp4 {
                AttachmentKind::Video
            } else {
                AttachmentKind::Document
            };
            return Ok((target, kind));
        }
    }

    if let Some(preview) = local_preview
        && preview.exists()
    {
        return Ok((preview.to_path_buf(), AttachmentKind::Document));
    }

    Err("Could not download GIF".to_string())
}

#[cfg(test)]
mod tests {
    use super::url_encode;

    #[test]
    fn url_encodes_spaces_and_unicode() {
        assert_eq!(url_encode("happy cat"), "happy%20cat");
        assert_eq!(url_encode("café"), "caf%C3%A9");
    }
}
