//! Technical diagnostic inspector (`Ctrl+Shift+D`).
//!
//! Collects cold-startup timings, `BoundedImageCache` memory/hit metrics,
//! SQLite WAL/DB disk usage, subsystem status (`ffmpeg`, `tray`, `video`, `audio`),
//! and recent log tail across Windows and Linux.

use std::{
    path::PathBuf,
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, StatefulInteractiveElement as _,
    Styled as _, div, px, rgba,
};
use gpui_component::{ActiveTheme as _, Icon, IconName, button::{Button, ButtonVariants as _}, h_flex, v_flex};
use whatfast_i18n as i18n;

use crate::app::Whatfast;
use crate::image_cache;

static BOOT_START: OnceLock<Instant> = OnceLock::new();
static EMOJI_FONT_MS: AtomicU64 = AtomicU64::new(0);
static WINDOW_OPEN_MS: AtomicU64 = AtomicU64::new(0);
static TOTAL_STARTUP_MS: AtomicU64 = AtomicU64::new(0);
static LAST_RPC_RTT_MS: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct StartupTimings {
    pub emoji_font_ms: u64,
    pub window_open_ms: u64,
    pub total_startup_ms: u64,
}

pub(crate) fn record_boot_start() {
    let _ = BOOT_START.set(Instant::now());
}

pub(crate) fn record_emoji_font_ready() {
    if let Some(t0) = BOOT_START.get() {
        EMOJI_FONT_MS.store(t0.elapsed().as_millis() as u64, Ordering::Relaxed);
    }
}

pub(crate) fn record_window_ready() {
    if let Some(t0) = BOOT_START.get() {
        let total = t0.elapsed().as_millis() as u64;
        let emoji = EMOJI_FONT_MS.load(Ordering::Relaxed);
        WINDOW_OPEN_MS.store(total.saturating_sub(emoji), Ordering::Relaxed);
        TOTAL_STARTUP_MS.store(total, Ordering::Relaxed);
    }
}

pub(crate) fn record_rpc_rtt_ms(ms: u64) {
    LAST_RPC_RTT_MS.store(ms, Ordering::Relaxed);
}

pub(crate) fn startup_timings() -> StartupTimings {
    StartupTimings {
        emoji_font_ms: EMOJI_FONT_MS.load(Ordering::Relaxed),
        window_open_ms: WINDOW_OPEN_MS.load(Ordering::Relaxed),
        total_startup_ms: TOTAL_STARTUP_MS.load(Ordering::Relaxed),
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DevStatsSnapshot {
    pub startup: StartupTimings,
    pub cache: image_cache::ImageCacheStats,
    pub db_bytes: u64,
    pub wal_bytes: u64,
    pub media_files: usize,
    pub media_bytes: u64,
    pub last_rpc_rtt_ms: u64,
    pub platform: &'static str,
    pub ffmpeg_found: bool,
    pub tray_active: bool,
    pub video_running: bool,
    pub audio_playing: bool,
    pub log_tail: Vec<String>,
}

impl DevStatsSnapshot {
    pub(crate) fn collect(app: &Whatfast) -> Self {
        let data_dir = whatfast_model::paths::data_dir();
        let db_path = data_dir.join("chats.db");
        let wal_path = data_dir.join("chats.db-wal");
        let db_bytes = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);
        let wal_bytes = std::fs::metadata(&wal_path).map(|m| m.len()).unwrap_or(0);

        let (media_files, media_bytes) = dir_usage(&data_dir.join("media"));
        let log_tail = read_log_tail(&data_dir, 30);

        let platform = if cfg!(windows) {
            "Windows"
        } else if cfg!(target_os = "linux") {
            if std::env::var_os("WAYLAND_DISPLAY").is_some() {
                "Linux (Wayland)"
            } else {
                "Linux (X11)"
            }
        } else {
            std::env::consts::OS
        };

        Self {
            startup: startup_timings(),
            cache: image_cache::stats_snapshot(),
            db_bytes,
            wal_bytes,
            media_files,
            media_bytes,
            last_rpc_rtt_ms: LAST_RPC_RTT_MS.load(Ordering::Relaxed),
            platform,
            ffmpeg_found: crate::call_video::ffmpeg().is_some(),
            tray_active: app.tray.is_some(),
            video_running: app.video.is_running(),
            audio_playing: app.voice.key.is_some() && !app.voice.paused,
            log_tail,
        }
    }
}

fn dir_usage(dir: &PathBuf) -> (usize, u64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (0, 0);
    };
    let mut count = 0usize;
    let mut bytes = 0u64;
    for entry in entries.flatten() {
        if let Ok(meta) = entry.metadata()
            && meta.is_file()
        {
            count += 1;
            bytes = bytes.saturating_add(meta.len());
        }
    }
    (count, bytes)
}

fn read_log_tail(data_dir: &std::path::Path, max_lines: usize) -> Vec<String> {
    use std::io::{Read as _, Seek as _, SeekFrom};

    const MAX_TAIL_BYTES: u64 = 64 * 1024;
    let candidates = [
        data_dir.join("backend.log"),
        data_dir.join("whatfast.log"),
        data_dir.join("logs").join("whatfast.log"),
        data_dir.join("sidecar.log"),
    ];
    for path in candidates {
        let Ok(mut file) = std::fs::File::open(&path) else {
            continue;
        };
        let len = file.metadata().map(|m| m.len()).unwrap_or(0);
        let skip_first = len > MAX_TAIL_BYTES;
        if skip_first && file.seek(SeekFrom::End(-(MAX_TAIL_BYTES as i64))).is_err() {
            continue;
        }
        let mut buf = Vec::with_capacity(len.min(MAX_TAIL_BYTES) as usize);
        if file.read_to_end(&mut buf).is_err() {
            continue;
        }
        let text = String::from_utf8_lossy(&buf);
        let mut lines: Vec<&str> = text.lines().collect();
        if skip_first && !lines.is_empty() {
            lines.remove(0);
        }
        let start = lines.len().saturating_sub(max_lines);
        return lines[start..].iter().map(|s| (*s).to_string()).collect();
    }
    Vec::new()
}

pub(crate) fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * 1024 * 1024;
    if bytes >= GIB {
        format!("{:.2} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

impl Whatfast {
    pub(crate) fn toggle_dev_stats(&mut self, cx: &mut Context<Self>) {
        if self.dev_stats_open {
            self.dev_stats_open = false;
        } else {
            self.dev_stats_snapshot = Some(DevStatsSnapshot::collect(self));
            self.dev_stats_open = true;
        }
        cx.notify();
    }

    pub(crate) fn render_dev_stats_modal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let scale = self.ui_scale;
        let snap = self
            .dev_stats_snapshot
            .clone()
            .unwrap_or_else(|| DevStatsSnapshot::collect(self));

        let metric_card = |label: &str, value: String, detail: String| {
            v_flex()
                .flex_1()
                .min_w(px(170. * scale))
                .p_3()
                .rounded_lg()
                .bg(theme.secondary)
                .border_1()
                .border_color(theme.border)
                .gap_1()
                .child(
                    div()
                        .text_size(px(11. * scale))
                        .text_color(theme.muted_foreground)
                        .child(label.to_string()),
                )
                .child(
                    div()
                        .text_size(px(16. * scale))
                        .text_color(theme.foreground)
                        .child(value),
                )
                .child(
                    div()
                        .text_size(px(11. * scale))
                        .text_color(theme.muted_foreground)
                        .child(detail),
                )
        };

        div()
            .id("dev-stats-backdrop")
            .absolute()
            .inset_0()
            .bg(rgba(0x00000088))
            .flex()
            .items_center()
            .justify_center()
            .on_click(cx.listener(|this, _, _, cx| {
                this.dev_stats_open = false;
                cx.notify();
            }))
            .child(
                v_flex()
                    .id("dev-stats-modal")
                    .w(px(680. * scale))
                    .max_w_full()
                    .max_h(px(620. * scale))
                    .p_4()
                    .gap_3()
                    .rounded_xl()
                    .bg(theme.background)
                    .border_1()
                    .border_color(theme.border)
                    .shadow_lg()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_2()
                                    .child(Icon::new(IconName::Settings).size(px(18. * scale)))
                                    .child(
                                        div()
                                            .text_size(px(16. * scale))
                                            .text_color(theme.foreground)
                                            .child(format!(
                                                "{} ({})",
                                                i18n::t("DevStats Inspector"),
                                                snap.platform
                                            )),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        Button::new("dev-stats-refresh")
                                            .ghost()
                                            .label(i18n::t("Refresh"))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.dev_stats_snapshot =
                                                    Some(DevStatsSnapshot::collect(this));
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        Button::new("dev-stats-close")
                                            .ghost()
                                            .icon(IconName::Close)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.dev_stats_open = false;
                                                cx.notify();
                                            })),
                                    ),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .child(metric_card(
                                i18n::t("Cold Startup"),
                                format!("{} ms", snap.startup.total_startup_ms),
                                format!(
                                    "fonts {} ms · window {} ms",
                                    snap.startup.emoji_font_ms, snap.startup.window_open_ms
                                ),
                            ))
                            .child(metric_card(
                                i18n::t("Image Cache"),
                                format!(
                                    "{} / {}",
                                    format_bytes(snap.cache.bytes as u64),
                                    format_bytes(snap.cache.max_bytes as u64)
                                ),
                                format!(
                                    "{}/{} items · {} in-flight · hits {} / misses {} · evictions {}{}",
                                    snap.cache.entries,
                                    snap.cache.max_entries,
                                    snap.cache.in_flight,
                                    snap.cache.hits,
                                    snap.cache.misses,
                                    snap.cache.evictions,
                                    if snap.cache.battery_saver {
                                        " (eco)"
                                    } else {
                                        ""
                                    }
                                ),
                            ))
                            .child(metric_card(
                                i18n::t("SQLite & Media"),
                                format_bytes(snap.db_bytes + snap.wal_bytes),
                                format!(
                                    "db {} · wal {} · {} media ({})",
                                    format_bytes(snap.db_bytes),
                                    format_bytes(snap.wal_bytes),
                                    snap.media_files,
                                    format_bytes(snap.media_bytes)
                                ),
                            )),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .child(metric_card(
                                i18n::t("Subsystems"),
                                format!(
                                    "ffmpeg: {} · tray: {}",
                                    if snap.ffmpeg_found { "ok" } else { "missing" },
                                    if snap.tray_active { "ok" } else { "off" }
                                ),
                                format!(
                                    "video: {} · audio: {} · RPC RTT: {} ms",
                                    if snap.video_running { "playing" } else { "idle" },
                                    if snap.audio_playing { "playing" } else { "idle" },
                                    snap.last_rpc_rtt_ms
                                ),
                            )),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(12. * scale))
                                    .text_color(theme.muted_foreground)
                                    .child(i18n::t("Recent Log Tail")),
                            )
                            .child(
                                v_flex()
                                    .id("dev-stats-logs")
                                    .h(px(220. * scale))
                                    .overflow_y_scroll()
                                    .p_2()
                                    .rounded_lg()
                                    .bg(theme.secondary)
                                    .border_1()
                                    .border_color(theme.border)
                                    .gap_1()
                                    .when(snap.log_tail.is_empty(), |el| {
                                        el.child(
                                            div()
                                                .text_size(px(11. * scale))
                                                .text_color(theme.muted_foreground)
                                                .child(i18n::t("No log file entries found.")),
                                        )
                                    })
                                    .children(snap.log_tail.into_iter().map(|line| {
                                        div()
                                            .text_size(px(11. * scale))
                                            .text_color(theme.foreground)
                                            .child(line)
                                    })),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::format_bytes;

    #[test]
    fn formats_byte_units_cleanly() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2.0 KiB");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5.0 MiB");
    }
}
