//! Persistent floating top mini-player bar when audio or video plays in another chat.

use gpui::{
    Context, InteractiveElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px,
    prelude::FluentBuilder as _,
};
use gpui_component::{ActiveTheme as _, Icon, IconName, h_flex};
use whatfast_i18n as i18n;

use crate::app::Whatfast;

impl Whatfast {
    pub(crate) fn render_floating_mini_player(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<gpui::Stateful<gpui::Div>> {
        let selected = self.store.selected_chat_id.as_deref();
        let voice_other = self.voice.key.as_ref().and_then(|(chat_id, _msg_id)| {
            if Some(chat_id.as_str()) != selected {
                Some((chat_id.clone(), true, self.voice.paused, self.voice.speed))
            } else {
                None
            }
        });
        let video_other = if voice_other.is_none() {
            self.video.key.as_ref().and_then(|key| {
                if let Some(chat_id) = key.0.strip_prefix("chat:").or(Some(key.0.as_str()))
                    && Some(chat_id) != selected
                {
                    Some((chat_id.to_string(), false, !self.video.playing, 1.0f32))
                } else {
                    None
                }
            })
        } else {
            None
        };

        let (chat_id, is_voice, paused, speed) = voice_other.or(video_other)?;
        let progress = if is_voice && self.voice.duration > 0.0 {
            (self.voice.position() / self.voice.duration).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let title = self
            .store
            .chats
            .iter()
            .find(|c| c.id == chat_id)
            .map(|c| c.title.clone())
            .unwrap_or_else(|| chat_id.clone());
        let scale = self.ui_scale;
        let theme = cx.theme();
        let next_speed = if speed < 1.25 {
            1.5
        } else if speed < 1.75 {
            2.0
        } else {
            1.0
        };

        Some(
            h_flex()
                .id("floating-mini-player")
                .relative()
                .w_full()
                .px_3()
                .py_1p5()
                .gap_2()
                .items_center()
                .bg(theme.secondary)
                .when(is_voice && self.voice.duration > 0.0, |bar| {
                    bar.child(
                        div()
                            .absolute()
                            .left_0()
                            .bottom_0()
                            .h(px(2.))
                            .w(gpui::relative(progress))
                            .bg(theme.accent),
                    )
                })
                .border_b_1()
                .border_color(theme.border)
                .child(
                    Icon::new(if is_voice {
                        IconName::Bell
                    } else {
                        IconName::Eye
                    })
                    .size(px(14. * scale))
                    .text_color(theme.primary),
                )
                .child(
                    div()
                        .id("mini-player-title")
                        .flex_1()
                        .truncate()
                        .text_xs()
                        .cursor_pointer()
                        .text_color(theme.foreground)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.store.select_chat(chat_id.clone());
                            this.load_selected_chat(chat_id.clone());
                            cx.notify();
                        }))
                        .child(format!(
                            "{}: {}",
                            if is_voice {
                                i18n::t("Voice note")
                            } else {
                                i18n::t("Video")
                            },
                            title
                        )),
                )
                .child(
                    div()
                        .id("mini-player-speed")
                        .px_2()
                        .py_0p5()
                        .rounded_sm()
                        .text_xs()
                        .cursor_pointer()
                        .bg(theme.background)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if is_voice {
                                this.voice.speed = next_speed;
                                this.voice_audio.speed(next_speed);
                            } else {
                                this.video_engine.set_speed(next_speed);
                            }
                            cx.notify();
                        }))
                        .child(format!("{speed:.1}x")),
                )
                .child(
                    div()
                        .id("mini-player-toggle")
                        .px_2()
                        .py_0p5()
                        .rounded_sm()
                        .text_xs()
                        .cursor_pointer()
                        .bg(theme.background)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if is_voice {
                                if this.voice.paused {
                                    this.voice.resume();
                                    let at = this.voice.position();
                                    let speed = this.voice.speed;
                                    let session = this.voice.session;
                                    let path = this.voice.path.clone();
                                    this.voice_audio.play(
                                        std::path::Path::new(&path),
                                        at,
                                        speed,
                                        session,
                                    );
                                } else {
                                    this.voice.pause();
                                    this.voice_audio.pause();
                                }
                            } else if !this.video.playing {
                                this.video_engine.resume();
                                this.video.playing = true;
                            } else {
                                this.video_engine.pause();
                                this.video.playing = false;
                            }
                            cx.notify();
                        }))
                        .child(if paused { "▶" } else { "⏸" }),
                )
                .child(
                    div()
                        .id("mini-player-stop")
                        .px_1p5()
                        .py_0p5()
                        .rounded_sm()
                        .text_xs()
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if is_voice {
                                this.voice_audio.stop();
                                this.voice.key = None;
                            } else {
                                // El corte del reproductor apaga tambien el
                                // reloj de 16 ms y el pedido de descarga.
                                this.stop_video();
                            }
                            cx.notify();
                        }))
                        .child("✕"),
                ),
        )
    }
}
