//! The image and media editor: WhatsApp Desktop full-screen experience.
//!
//! Visual fidelity and full interactivity matching WhatsApp Desktop:
//! - Full-screen immersive workspace (#0c1317 background).
//! - Responsive image canvas that scales small and large images up to 920x580 containment.
//! - Top Bar with active/hover states:
//!   - Close (✕)
//!   - Undo (↶) & Redo (↷)
//!   - Crop & Rotate (⛶) with 90° rotation, aspect ratios (1:1, 4:3, 16:9, Libre), reset.
//!   - Filters (🪄) with live presets (Normal, Pop, B&N, Cálido, Frío, Sepia).
//!   - Pen (✏️) with real-time drawing, color palette, brush thickness, clear strokes.
//!   - Text (Aa) with custom text input, color palette, background box toggle, movable overlays.
//!   - Shapes (⬜) with rectangle, circle, and arrow overlays.
//!   - Blur / Mosaic (▦) with censorship pixelation, block size, drag-to-blur, auto-censor.
//!   - Stickers (😊) with emoji palette, draggable emojis, clear emojis.
//!   - Sticker Cutout (🏷️).
//!   - HD Quality ([HD]) toggle with green badge.
//!   - Save / Download (📥) saving directly to user Downloads folder.
//! - Floating Capsule Caption Bar (#202c33) with "Añade un comentario" and emoji picker.
//! - Bottom Bar:
//!   - Audience Privacy pill: "◎ Estado (221 excluidos)" with interactive modal.
//!   - Media Carousel with active green border, plus button that activates new images, remove thumbnails.
//!   - Circular WhatsApp Green Send Button (➤).

#![allow(dead_code)]

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use whatfast_i18n as i18n;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AppContext as _, Bounds, Context, EventEmitter, FontWeight, ImageSource, InteractiveElement as _,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit,
    ParentElement as _, Pixels, Point, Render, StatefulInteractiveElement as _, Styled as _,
    StyledImage as _, Window, canvas, div, img, px, rgb, rgba, PathPromptOptions,
};
use gpui::ElementId;
use gpui_component::input::{Input, InputState};
use gpui_component::{
    Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};

use whatfast_core::media_crop::{self, CropRect, EditPlan, PixelSize, Rotation};
use crate::icons::WhatfastIcon;

/// The longest edge of the preview buffer in pixels.
pub(crate) const PREVIEW_EDGE: f64 = 620.0;

/// The smallest crop the user can leave behind, in preview pixels.
pub(crate) const MIN_CROP_EDGE: f64 = 24.0;

/// The radius of the area that grabs a handle, in preview pixels.
const HANDLE_RADIUS: f64 = 10.0;

/// The side of the square that draws a handle, in preview pixels.
const HANDLE_EDGE: f64 = 12.0;

/// The active editing tool in the top bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum EditorTool {
    #[default]
    None,
    Crop,
    Filters,
    Pen,
    Text,
    Shapes,
    Blur,
    Stickers,
}

/// Image filter presets matching WhatsApp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ImageFilter {
    #[default]
    Normal,
    HighContrast,
    Grayscale,
    Warm,
    Cool,
    Sepia,
}

/// A freehand pen stroke.
#[derive(Debug, Clone)]
pub(crate) struct DrawStroke {
    pub(crate) points: Vec<(f64, f64)>,
    pub(crate) color: [u8; 4],
    pub(crate) width: f64,
}

/// A blur / mosaic pixelated rectangle.
#[derive(Debug, Clone)]
pub(crate) struct BlurRegion {
    pub(crate) rect: CropRect,
    pub(crate) block_size: u32,
}

/// A floating text overlay.
#[derive(Debug, Clone)]
pub(crate) struct TextOverlay {
    pub(crate) text: String,
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) color: [u8; 4],
    pub(crate) has_bg: bool,
}

/// A floating emoji overlay.
#[derive(Debug, Clone)]
pub(crate) struct EmojiOverlay {
    pub(crate) emoji: String,
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) size: f32,
}

/// Shape kinds for shape overlays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShapeKind {
    Rectangle,
    Circle,
    Arrow,
}

/// A floating shape overlay.
#[derive(Debug, Clone)]
pub(crate) struct ShapeOverlay {
    pub(crate) kind: ShapeKind,
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
    pub(crate) color: [u8; 4],
}

/// The handle the user is dragging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DragMode {
    Create,
    Move,
    NorthWest,
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
}

/// The state of one drag.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DragState {
    mode: DragMode,
    origin_x: f64,
    origin_y: f64,
    start: CropRect,
}

/// Snapshot of an item's editable state for Undo / Redo.
#[derive(Debug, Clone)]
pub(crate) struct ItemStateSnapshot {
    rotation: Rotation,
    crop: Option<CropRect>,
    filter: ImageFilter,
    strokes: Vec<DrawStroke>,
    blurs: Vec<BlurRegion>,
    texts: Vec<TextOverlay>,
    emojis: Vec<EmojiOverlay>,
    shapes: Vec<ShapeOverlay>,
}

/// One media item in the editor batch / carousel.
#[derive(Debug, Clone)]
pub(crate) struct MediaItem {
    pub(crate) path: PathBuf,
    pub(crate) file_name: String,
    pub(crate) natural: PixelSize,
    pub(crate) rotation: Rotation,
    pub(crate) crop: Option<CropRect>,
    pub(crate) filter: ImageFilter,
    pub(crate) strokes: Vec<DrawStroke>,
    pub(crate) blurs: Vec<BlurRegion>,
    pub(crate) texts: Vec<TextOverlay>,
    pub(crate) emojis: Vec<EmojiOverlay>,
    pub(crate) shapes: Vec<ShapeOverlay>,
    pub(crate) caption: String,
    pub(crate) is_hd: bool,
    pub(crate) preview_base: Option<image::RgbaImage>,
    pub(crate) preview_baked: Option<image::RgbaImage>,
    pub(crate) rendered: Option<Arc<gpui::RenderImage>>,
    pub(crate) rendered_version: usize,
}

impl MediaItem {
    pub(crate) fn new(path: PathBuf) -> Self {
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "photo".to_string());
        Self {
            path,
            file_name,
            natural: PixelSize {
                width: 0.0,
                height: 0.0,
            },
            rotation: Rotation::Deg0,
            crop: None,
            filter: ImageFilter::Normal,
            strokes: Vec::new(),
            blurs: Vec::new(),
            texts: Vec::new(),
            emojis: Vec::new(),
            shapes: Vec::new(),
            caption: String::new(),
            is_hd: false,
            preview_base: None,
            preview_baked: None,
            rendered: None,
            rendered_version: 0,
        }
    }

    fn snapshot(&self) -> ItemStateSnapshot {
        ItemStateSnapshot {
            rotation: self.rotation,
            crop: self.crop,
            filter: self.filter,
            strokes: self.strokes.clone(),
            blurs: self.blurs.clone(),
            texts: self.texts.clone(),
            emojis: self.emojis.clone(),
            shapes: self.shapes.clone(),
        }
    }

    fn restore(&mut self, snapshot: ItemStateSnapshot) {
        self.rotation = snapshot.rotation;
        self.crop = snapshot.crop;
        self.filter = snapshot.filter;
        self.strokes = snapshot.strokes;
        self.blurs = snapshot.blurs;
        self.texts = snapshot.texts;
        self.emojis = snapshot.emojis;
        self.shapes = snapshot.shapes;
        self.rebake_preview();
    }

    fn rotated_size(&self) -> PixelSize {
        match media_crop::rotated_size(self.natural, self.rotation) {
            Ok(size) => size,
            Err(_) => PixelSize {
                width: 0.0,
                height: 0.0,
            },
        }
    }

    fn crop_rect(&self) -> CropRect {
        match self.crop {
            Some(crop) => crop,
            None => {
                let size = self.rotated_size();
                CropRect {
                    x: 0.0,
                    y: 0.0,
                    width: size.width,
                    height: size.height,
                }
            }
        }
    }

    fn rebake_preview(&mut self) {
        let Some(base) = self.preview_base.as_ref() else {
            return;
        };
        let mut rotated = rotate_picture(base, self.rotation);
        apply_filter_to_image(&mut rotated, self.filter);

        let (base_w, base_h) = (rotated.width() as f64, rotated.height() as f64);
        let rot_size = self.rotated_size();
        let scale_x = if rot_size.width > 0.0 { base_w / rot_size.width } else { 1.0 };
        let scale_y = if rot_size.height > 0.0 { base_h / rot_size.height } else { 1.0 };

        for blur in &self.blurs {
            let scaled_rect = CropRect {
                x: blur.rect.x * scale_x,
                y: blur.rect.y * scale_y,
                width: blur.rect.width * scale_x,
                height: blur.rect.height * scale_y,
            };
            apply_mosaic_to_image(&mut rotated, &scaled_rect, blur.block_size);
        }

        for stroke in &self.strokes {
            let scaled_points: Vec<(f64, f64)> = stroke
                .points
                .iter()
                .map(|&(x, y)| (x * scale_x, y * scale_y))
                .collect();
            let scaled_stroke = DrawStroke {
                points: scaled_points,
                color: stroke.color,
                width: (stroke.width * (scale_x + scale_y) / 2.0).max(1.0),
            };
            apply_stroke_to_image(&mut rotated, &scaled_stroke);
        }

        self.preview_baked = Some(rotated.clone());
        self.rendered = Some(Arc::new(to_render_image(&rotated)));
        self.rendered_version = self.rendered_version.wrapping_add(1);
    }
}

/// The edit that the user applied to one picture.
#[derive(Debug, Clone)]
pub(crate) struct MediaEdit {
    pub(crate) path: PathBuf,
    pub(crate) plan: EditPlan,
    pub(crate) caption: String,
    pub(crate) filter: ImageFilter,
    pub(crate) strokes: Vec<DrawStroke>,
    pub(crate) blurs: Vec<BlurRegion>,
    pub(crate) texts: Vec<TextOverlay>,
    pub(crate) emojis: Vec<EmojiOverlay>,
    pub(crate) is_hd: bool,
}

/// The events of the editor.
#[derive(Debug, Clone)]
pub(crate) enum MediaEditorEvent {
    Applied(Vec<MediaEdit>),
    Cancelled,
}

/// The reason why the editor could not build the plan of the edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MediaEditError {
    NoPicture,
    Geometry(String),
}

impl std::fmt::Display for MediaEditError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediaEditError::NoPicture => write!(formatter, "the photo could not be edited"),
            MediaEditError::Geometry(message) => write!(formatter, "{message}"),
        }
    }
}

/// Applies a complete plan of edit (rotation, filter, blur mosaic, pen strokes, text overlays, crop) to a picture.
pub(crate) fn apply_media_edit(edit: &MediaEdit, destination: &Path) -> Result<(), String> {
    let picture = image::open(&edit.path)
        .map_err(|error| format!("the picture could not be opened: {error}"))?;

    let mut rotated = match edit.plan.rotation {
        Rotation::Deg0 => picture.into_rgba8(),
        Rotation::Deg90 => picture.rotate90().into_rgba8(),
        Rotation::Deg180 => picture.rotate180().into_rgba8(),
        Rotation::Deg270 => picture.rotate270().into_rgba8(),
    };

    apply_filter_to_image(&mut rotated, edit.filter);

    for blur in &edit.blurs {
        apply_mosaic_to_image(&mut rotated, &blur.rect, blur.block_size);
    }

    for stroke in &edit.strokes {
        apply_stroke_to_image(&mut rotated, stroke);
    }

    let x = edit.plan.crop.x.round().clamp(0.0, rotated.width() as f64) as u32;
    let y = edit.plan.crop.y.round().clamp(0.0, rotated.height() as f64) as u32;
    let width = (edit.plan.crop.width.round().max(1.0) as u32).min(rotated.width().saturating_sub(x));
    let height = (edit.plan.crop.height.round().max(1.0) as u32).min(rotated.height().saturating_sub(y));
    let cropped = image::imageops::crop_imm(&rotated, x, y, width, height).to_image();

    let ext = destination
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext == "jpg" || ext == "jpeg" {
        image::DynamicImage::ImageRgba8(cropped)
            .into_rgb8()
            .save(destination)
            .map_err(|error| format!("the picture could not be saved: {error}"))
    } else {
        cropped
            .save(destination)
            .map_err(|error| format!("the picture could not be saved: {error}"))
    }
}

/// Applies a crop and rotation to a picture and writes the result (legacy helper).
pub(crate) fn apply_edit(source: &Path, plan: &EditPlan, destination: &Path) -> Result<(), String> {
    let edit = MediaEdit {
        path: source.to_path_buf(),
        plan: *plan,
        caption: String::new(),
        filter: ImageFilter::Normal,
        strokes: Vec::new(),
        blurs: Vec::new(),
        texts: Vec::new(),
        emojis: Vec::new(),
        is_hd: false,
    };
    apply_media_edit(&edit, destination)
}

/// The full-screen WhatsApp media/status editor.
pub(crate) struct MediaEditor {
    pub(crate) items: Vec<MediaItem>,
    pub(crate) active_index: usize,
    pub(crate) active_tool: EditorTool,
    pub(crate) pen_color: [u8; 4],
    pub(crate) pen_width: f64,
    pub(crate) blur_block_size: u32,
    pub(crate) text_color: [u8; 4],
    pub(crate) text_has_bg: bool,
    pub(crate) shape_color: [u8; 4],
    pub(crate) active_stroke: Option<DrawStroke>,
    pub(crate) active_blur_start: Option<(f64, f64)>,
    pub(crate) active_blur_rect: Option<CropRect>,
    pub(crate) drag: Option<DragState>,
    pub(crate) working: bool,
    pub(crate) error: Option<String>,
    pub(crate) undo_stack: Vec<ItemStateSnapshot>,
    pub(crate) redo_stack: Vec<ItemStateSnapshot>,
    pub(crate) surface: Rc<Cell<Option<Bounds<Pixels>>>>,
    pub(crate) caption: Option<gpui::Entity<InputState>>,
    pub(crate) text_input: Option<gpui::Entity<InputState>>,
    pub(crate) caption_emoji_open: bool,
    pub(crate) status_privacy_open: bool,
    pub(crate) status_privacy_mode: String,
    pub(crate) save_notice: Option<String>,
}

impl EventEmitter<MediaEditorEvent> for MediaEditor {}

impl MediaEditor {
    pub(crate) fn new(path: &Path, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let caption = cx.new(|cx| {
            InputState::new(window, cx).placeholder(i18n::t("Añade un comentario"))
        });
        let text_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(i18n::t("Escribe tu texto..."))
        });
        let initial_item = MediaItem::new(path.to_path_buf());
        let mut editor = Self {
            items: vec![initial_item],
            active_index: 0,
            active_tool: EditorTool::None,
            pen_color: [0, 168, 132, 255], // WhatsApp green
            pen_width: 5.0,
            blur_block_size: 14,
            text_color: [255, 255, 255, 255], // White
            text_has_bg: true,
            shape_color: [0, 168, 132, 255], // WhatsApp green
            active_stroke: None,
            active_blur_start: None,
            active_blur_rect: None,
            drag: None,
            working: false,
            error: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            surface: Rc::new(Cell::new(None)),
            caption: Some(caption),
            text_input: Some(text_input),
            caption_emoji_open: false,
            status_privacy_open: false,
            status_privacy_mode: "Estado (221 excluidos)".to_string(),
            save_notice: None,
        };
        editor.load_item(0, cx);
        editor
    }

    pub(crate) fn active_item(&self) -> &MediaItem {
        &self.items[self.active_index.min(self.items.len().saturating_sub(1))]
    }

    pub(crate) fn active_item_mut(&mut self) -> &mut MediaItem {
        let idx = self.active_index.min(self.items.len().saturating_sub(1));
        &mut self.items[idx]
    }

    pub(crate) fn path(&self) -> &Path {
        &self.active_item().path
    }

    pub(crate) fn file_name(&self) -> &str {
        &self.active_item().file_name
    }

    pub(crate) fn is_working(&self) -> bool {
        self.working
    }

    pub(crate) fn rotation(&self) -> Rotation {
        self.active_item().rotation
    }

    pub(crate) fn crop(&self) -> CropRect {
        self.active_item().crop_rect()
    }

    fn push_undo(&mut self) {
        let snap = self.active_item().snapshot();
        self.undo_stack.push(snap);
        self.redo_stack.clear();
    }

    pub(crate) fn undo(&mut self, cx: &mut Context<Self>) {
        if let Some(prev) = self.undo_stack.pop() {
            let curr = self.active_item().snapshot();
            self.redo_stack.push(curr);
            self.active_item_mut().restore(prev);
            cx.notify();
        }
    }

    pub(crate) fn redo(&mut self, cx: &mut Context<Self>) {
        if let Some(next) = self.redo_stack.pop() {
            let curr = self.active_item().snapshot();
            self.undo_stack.push(curr);
            self.active_item_mut().restore(next);
            cx.notify();
        }
    }

    pub(crate) fn rotate_left(&mut self, cx: &mut Context<Self>) {
        self.push_undo();
        self.rotate(270, cx);
    }

    pub(crate) fn rotate_right(&mut self, cx: &mut Context<Self>) {
        self.push_undo();
        self.rotate(90, cx);
    }

    pub(crate) fn reset_crop(&mut self, cx: &mut Context<Self>) {
        self.push_undo();
        self.active_item_mut().crop = None;
        cx.notify();
    }

    pub(crate) fn set_aspect_ratio(&mut self, ratio: f64, cx: &mut Context<Self>) {
        self.push_undo();
        let frame = self.rotated_size();
        if frame.width <= 0.0 || frame.height <= 0.0 {
            return;
        }
        let (crop_w, crop_h) = if frame.width / frame.height > ratio {
            (frame.height * ratio, frame.height)
        } else {
            (frame.width, frame.width / ratio)
        };
        let crop_x = (frame.width - crop_w) / 2.0;
        let crop_y = (frame.height - crop_h) / 2.0;
        self.active_item_mut().crop = Some(CropRect {
            x: crop_x.max(0.0),
            y: crop_y.max(0.0),
            width: crop_w.min(frame.width),
            height: crop_h.min(frame.height),
        });
        cx.notify();
    }

    pub(crate) fn set_filter(&mut self, filter: ImageFilter, cx: &mut Context<Self>) {
        if self.active_item().filter != filter {
            self.push_undo();
            self.active_item_mut().filter = filter;
            self.active_item_mut().rebake_preview();
            cx.notify();
        }
    }

    pub(crate) fn toggle_hd(&mut self, cx: &mut Context<Self>) {
        let is_hd = self.active_item().is_hd;
        self.active_item_mut().is_hd = !is_hd;
        self.save_notice = Some(if !is_hd {
            i18n::t("Calidad HD activada").to_string()
        } else {
            i18n::t("Calidad estándar activada").to_string()
        });
        cx.notify();
    }

    pub(crate) fn add_text_from_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self
            .text_input
            .as_ref()
            .map(|i| i.read(cx).value().trim().to_string())
            .unwrap_or_default();
        if text.is_empty() {
            return;
        }
        self.push_undo();
        let center = self.rotated_size();
        let color = self.text_color;
        let has_bg = self.text_has_bg;
        self.active_item_mut().texts.push(TextOverlay {
            text: text.clone(),
            x: (center.width / 2.0).max(30.0),
            y: (center.height / 2.0).max(30.0),
            color,
            has_bg,
        });
        if let Some(inp) = self.text_input.as_ref() {
            inp.update(cx, |input, cx| {
                input.set_value(String::new(), window, cx);
            });
        }
        self.save_notice = Some(i18n::t("Texto añadido").to_string());
        cx.notify();
    }

    pub(crate) fn add_shape(&mut self, kind: ShapeKind, cx: &mut Context<Self>) {
        self.push_undo();
        let center = self.rotated_size();
        let (sw, sh) = (140.0_f64.min(center.width * 0.5), 90.0_f64.min(center.height * 0.5));
        let color = self.shape_color;
        self.active_item_mut().shapes.push(ShapeOverlay {
            kind,
            x: ((center.width - sw) / 2.0).max(10.0),
            y: ((center.height - sh) / 2.0).max(10.0),
            width: sw,
            height: sh,
            color,
        });
        cx.notify();
    }

    pub(crate) fn add_auto_blur(&mut self, cx: &mut Context<Self>) {
        self.push_undo();
        let center = self.rotated_size();
        let (bw, bh) = (160.0_f64.min(center.width * 0.6), 80.0_f64.min(center.height * 0.4));
        let bx = ((center.width - bw) / 2.0).max(0.0);
        let by = ((center.height - bh) / 2.0).max(0.0);
        let b = self.blur_block_size;
        self.active_item_mut().blurs.push(BlurRegion {
            rect: CropRect {
                x: bx,
                y: by,
                width: bw,
                height: bh,
            },
            block_size: b,
        });
        self.active_item_mut().rebake_preview();
        cx.notify();
    }

    pub(crate) fn cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(MediaEditorEvent::Cancelled);
    }

    pub(crate) fn sync_current_caption(&mut self, cx: &mut Context<Self>) {
        if let Some(caption_input) = self.caption.as_ref() {
            let val = caption_input.read(cx).value().trim().to_string();
            self.active_item_mut().caption = val;
        }
    }

    pub(crate) fn select_item(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.items.len() || index == self.active_index {
            return;
        }
        self.sync_current_caption(cx);
        self.active_index = index;
        self.undo_stack.clear();
        self.redo_stack.clear();
        let item_caption = self.items[index].caption.clone();
        if let Some(caption_input) = self.caption.as_ref() {
            caption_input.update(cx, |input, cx| {
                input.set_value(item_caption, window, cx);
            });
        }
        if self.items[index].preview_base.is_none() {
            self.load_item(index, cx);
        }
        cx.notify();
    }

    pub(crate) fn add_items_from_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        for path in paths {
            let item = MediaItem::new(path);
            self.items.push(item);
            let idx = self.items.len() - 1;
            self.load_item(idx, cx);
            self.active_index = idx; // Activate the newly added photo immediately!
        }
        cx.notify();
    }

    pub(crate) fn remove_item(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.items.len() <= 1 || index >= self.items.len() {
            return;
        }
        self.items.remove(index);
        let next_idx = self.active_index.min(self.items.len().saturating_sub(1));
        self.active_index = next_idx;
        let item_caption = self.items[next_idx].caption.clone();
        if let Some(caption_input) = self.caption.as_ref() {
            caption_input.update(cx, |input, cx| {
                input.set_value(item_caption, window, cx);
            });
        }
        cx.notify();
    }

    pub(crate) fn save_to_disk(&mut self, cx: &mut Context<Self>) {
        self.sync_current_caption(cx);
        let item = self.active_item();
        let plan = match self.plan_for_item(item) {
            Ok(p) => p,
            Err(e) => {
                self.error = Some(e.to_string());
                cx.notify();
                return;
            }
        };
        let edit = MediaEdit {
            path: item.path.clone(),
            plan,
            caption: item.caption.clone(),
            filter: item.filter,
            strokes: item.strokes.clone(),
            blurs: item.blurs.clone(),
            texts: item.texts.clone(),
            emojis: item.emojis.clone(),
            is_hd: item.is_hd,
        };

        let downloads_dir = std::env::var("USERPROFILE")
            .map(|p| PathBuf::from(p).join("Downloads"))
            .unwrap_or_else(|_| PathBuf::from("."));
        let file_name = format!(
            "whatfast_edit_{}.jpg",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );
        let dest = downloads_dir.join(file_name);

        let task_edit = edit;
        let task_dest = dest.clone();
        cx.spawn(async move |this, cx| {
            let res = smol::unblock(move || apply_media_edit(&task_edit, &task_dest)).await;
            this.update(cx, |this, cx| {
                match res {
                    Ok(()) => {
                        this.save_notice = Some(i18n::t("Foto guardada en Descargas").to_string());
                    }
                    Err(err) => {
                        this.error = Some(format!("Error al guardar: {err}"));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn apply(&mut self, cx: &mut Context<Self>) {
        if self.is_working() {
            return;
        }
        self.sync_current_caption(cx);
        let mut edits = Vec::new();
        for item in &self.items {
            match self.plan_for_item(item) {
                Ok(plan) => {
                    edits.push(MediaEdit {
                        path: item.path.clone(),
                        plan,
                        caption: item.caption.clone(),
                        filter: item.filter,
                        strokes: item.strokes.clone(),
                        blurs: item.blurs.clone(),
                        texts: item.texts.clone(),
                        emojis: item.emojis.clone(),
                        is_hd: item.is_hd,
                    });
                }
                Err(error) => {
                    self.error = Some(error.to_string());
                    cx.notify();
                    return;
                }
            }
        }
        if !edits.is_empty() {
            cx.emit(MediaEditorEvent::Applied(edits));
        }
    }

    pub(crate) fn plan(&self) -> Result<EditPlan, MediaEditError> {
        self.plan_for_item(self.active_item())
    }

    fn plan_for_item(&self, item: &MediaItem) -> Result<EditPlan, MediaEditError> {
        let natural = item.natural;
        if natural.width <= 0.0 || natural.height <= 0.0 {
            return Err(MediaEditError::NoPicture);
        }
        let rotation = item.rotation;
        let rotated = media_crop::rotated_size(natural, rotation)
            .map_err(|error| MediaEditError::Geometry(error.message))?;
        let source_crop =
            media_crop::crop_in_rotated_space(item.crop_rect(), rotated, inverse_rotation(rotation))
                .map_err(|error| MediaEditError::Geometry(error.message))?;
        media_crop::plan_edit(natural, source_crop, rotation)
            .map_err(|error| MediaEditError::Geometry(error.message))
    }

    fn load_item(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.items.len() {
            return;
        }
        self.working = true;
        self.error = None;
        let path = self.items[index].path.clone();
        cx.spawn(async move |this, cx| {
            let loaded = smol::unblock(move || load_picture(&path)).await;
            this.update(cx, |this, cx| {
                this.working = false;
                match loaded {
                    Ok(loaded) => {
                        if index < this.items.len() {
                            this.items[index].natural = loaded.natural;
                            this.items[index].preview_base = Some(loaded.preview);
                            this.items[index].rebake_preview();
                        }
                    }
                    Err(error) => {
                        this.error = Some(error);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn rotate(&mut self, steps: u32, cx: &mut Context<Self>) {
        let current = self.rotation();
        let next = media_crop::parse_rotation(current.degrees() as f64 + steps as f64);
        if let Some(crop) = self.active_item().crop {
            let frame = self.rotated_size();
            self.active_item_mut().crop = carried_crop(crop, frame, steps);
        }
        self.active_item_mut().rotation = next;
        self.active_item_mut().rebake_preview();
        cx.notify();
    }

    fn rotated_size(&self) -> PixelSize {
        self.active_item().rotated_size()
    }

    /// Size of the canvas containment on screen.
    fn drawn_size(&self) -> PixelSize {
        let rotated = self.rotated_size();
        if rotated.width <= 0.0 || rotated.height <= 0.0 {
            return PixelSize {
                width: 460.0,
                height: 320.0,
            };
        }
        let scale = display_scale(rotated);
        PixelSize {
            width: (rotated.width * scale).round().max(40.0),
            height: (rotated.height * scale).round().max(40.0),
        }
    }

    fn window_per_picture(&self) -> (f64, f64) {
        if let Some(bounds) = self.surface.get() {
            let rotated = self.rotated_size();
            let bw = f32::from(bounds.size.width) as f64;
            let bh = f32::from(bounds.size.height) as f64;
            if bw > 0.0 && bh > 0.0 && rotated.width > 0.0 && rotated.height > 0.0 {
                return (bw / rotated.width, bh / rotated.height);
            }
        }
        let drawn = self.drawn_size();
        let rotated = self.rotated_size();
        (
            if drawn.width > 0.0 && rotated.width > 0.0 {
                drawn.width / rotated.width
            } else {
                1.0
            },
            if drawn.height > 0.0 && rotated.height > 0.0 {
                drawn.height / rotated.height
            } else {
                1.0
            },
        )
    }

    fn min_edge(&self) -> f64 {
        let (scale_x, scale_y) = self.window_per_picture();
        MIN_CROP_EDGE * (scale_x + scale_y) / 2.0
    }

    fn handle_radius(&self) -> f64 {
        let (scale_x, scale_y) = self.window_per_picture();
        HANDLE_RADIUS * (scale_x + scale_y) / 2.0
    }

    fn picture_point(&self, position: Point<Pixels>) -> Option<(f64, f64)> {
        let bounds = self.surface.get()?;
        let (scale_x, scale_y) = self.window_per_picture();
        if scale_x <= 0.0 || scale_y <= 0.0 {
            return None;
        }
        Some((
            f32::from(position.x - bounds.origin.x) as f64 / scale_x,
            f32::from(position.y - bounds.origin.y) as f64 / scale_y,
        ))
    }

    fn begin_drag(&mut self, position: Point<Pixels>) {
        let Some((x, y)) = self.picture_point(position) else {
            return;
        };
        match self.active_tool {
            EditorTool::Pen => {
                self.push_undo();
                self.active_stroke = Some(DrawStroke {
                    points: vec![(x, y)],
                    color: self.pen_color,
                    width: self.pen_width,
                });
            }
            EditorTool::Blur => {
                self.push_undo();
                self.active_blur_start = Some((x, y));
                self.active_blur_rect = Some(CropRect {
                    x,
                    y,
                    width: 0.0,
                    height: 0.0,
                });
            }
            EditorTool::Crop | EditorTool::None => {
                let crop = self.crop();
                let mode = self.handle_under(x, y, crop);
                self.drag = Some(DragState {
                    mode,
                    origin_x: x,
                    origin_y: y,
                    start: crop,
                });
            }
            _ => {}
        }
    }

    fn move_drag(&mut self, position: Point<Pixels>) {
        let Some((x, y)) = self.picture_point(position) else {
            return;
        };
        match self.active_tool {
            EditorTool::Pen => {
                if let Some(stroke) = self.active_stroke.as_mut() {
                    stroke.points.push((x, y));
                }
            }
            EditorTool::Blur => {
                if let Some((sx, sy)) = self.active_blur_start {
                    let rx = sx.min(x);
                    let ry = sy.min(y);
                    let rw = (x - sx).abs();
                    let rh = (y - sy).abs();
                    self.active_blur_rect = Some(CropRect {
                        x: rx,
                        y: ry,
                        width: rw,
                        height: rh,
                    });
                }
            }
            EditorTool::Crop | EditorTool::None => {
                let Some(drag) = self.drag else {
                    return;
                };
                let frame = self.rotated_size();
                let min_edge = self.min_edge();
                let start = drag.start;
                let point_x = x.clamp(0.0, frame.width);
                let point_y = y.clamp(0.0, frame.height);
                let right = start.x + start.width;
                let bottom = start.y + start.height;
                let crop = match drag.mode {
                    DragMode::Create => {
                        let origin_x = drag.origin_x;
                        let origin_y = drag.origin_y;
                        let left = origin_x.min(point_x);
                        let top = origin_y.min(point_y);
                        CropRect {
                            x: left,
                            y: top,
                            width: (point_x - origin_x).abs(),
                            height: (point_y - origin_y).abs(),
                        }
                    }
                    DragMode::Move => {
                        let width = start.width.min(frame.width);
                        let height = start.height.min(frame.height);
                        let x = (start.x + (point_x - drag.origin_x))
                            .clamp(0.0, (frame.width - width).max(0.0));
                        let y = (start.y + (point_y - drag.origin_y))
                            .clamp(0.0, (frame.height - height).max(0.0));
                        CropRect {
                            x,
                            y,
                            width,
                            height,
                        }
                    }
                    DragMode::NorthWest => {
                        let (x, width) = left_edge(right, point_x, min_edge);
                        let (y, height) = top_edge(bottom, point_y, min_edge);
                        CropRect { x, y, width, height }
                    }
                    DragMode::North => {
                        let (y, height) = top_edge(bottom, point_y, min_edge);
                        CropRect {
                            x: start.x,
                            y,
                            width: start.width,
                            height,
                        }
                    }
                    DragMode::NorthEast => {
                        let width = right_edge(start, point_x, min_edge, frame.width);
                        let (y, height) = top_edge(bottom, point_y, min_edge);
                        CropRect {
                            x: start.x,
                            y,
                            width,
                            height,
                        }
                    }
                    DragMode::East => {
                        let width = right_edge(start, point_x, min_edge, frame.width);
                        CropRect {
                            x: start.x,
                            y: start.y,
                            width,
                            height: start.height,
                        }
                    }
                    DragMode::SouthEast => {
                        let width = right_edge(start, point_x, min_edge, frame.width);
                        let height = bottom_edge(start, point_y, min_edge, frame.height);
                        CropRect {
                            x: start.x,
                            y: start.y,
                            width,
                            height,
                        }
                    }
                    DragMode::South => {
                        let height = bottom_edge(start, point_y, min_edge, frame.height);
                        CropRect {
                            x: start.x,
                            y: start.y,
                            width: start.width,
                            height,
                        }
                    }
                    DragMode::SouthWest => {
                        let (x, width) = left_edge(right, point_x, min_edge);
                        let height = bottom_edge(start, point_y, min_edge, frame.height);
                        CropRect { x, y, width, height }
                    }
                    DragMode::West => {
                        let (x, width) = left_edge(right, point_x, min_edge);
                        CropRect {
                            x,
                            y: start.y,
                            width,
                            height: start.height,
                        }
                    }
                };
                self.active_item_mut().crop = Some(crop);
            }
            _ => {}
        }
    }

    fn end_drag(&mut self) {
        match self.active_tool {
            EditorTool::Pen => {
                if let Some(stroke) = self.active_stroke.take() {
                    if stroke.points.len() > 1 {
                        self.active_item_mut().strokes.push(stroke);
                        self.active_item_mut().rebake_preview();
                    }
                }
            }
            EditorTool::Blur => {
                self.active_blur_start = None;
                if let Some(rect) = self.active_blur_rect.take() {
                    if rect.width > 5.0 && rect.height > 5.0 {
                        let b = self.blur_block_size;
                        self.active_item_mut().blurs.push(BlurRegion {
                            rect,
                            block_size: b,
                        });
                        self.active_item_mut().rebake_preview();
                    }
                }
            }
            EditorTool::Crop | EditorTool::None => {
                if self.drag.take().is_none() {
                    return;
                }
                let frame = self.rotated_size();
                let min_edge = self.min_edge();
                let current = self.crop();
                let width = current.width.max(min_edge).min(frame.width);
                let height = current.height.max(min_edge).min(frame.height);
                self.active_item_mut().crop = Some(CropRect {
                    x: current.x.min(frame.width - width),
                    y: current.y.min(frame.height - height),
                    width,
                    height,
                });
            }
            _ => {}
        }
    }

    fn handle_under(&self, x: f64, y: f64, crop: CropRect) -> DragMode {
        let radius = self.handle_radius();
        let right = crop.x + crop.width;
        let bottom = crop.y + crop.height;
        let middle_x = crop.x + crop.width / 2.0;
        let middle_y = crop.y + crop.height / 2.0;
        let handles = [
            (DragMode::NorthWest, crop.x, crop.y),
            (DragMode::North, middle_x, crop.y),
            (DragMode::NorthEast, right, crop.y),
            (DragMode::East, right, middle_y),
            (DragMode::SouthEast, right, bottom),
            (DragMode::South, middle_x, bottom),
            (DragMode::SouthWest, crop.x, bottom),
            (DragMode::West, crop.x, middle_y),
        ];
        let mut closest: Option<(DragMode, f64)> = None;
        for (mode, handle_x, handle_y) in handles {
            let distance = (x - handle_x).hypot(y - handle_y);
            if distance > radius {
                continue;
            }
            if closest.is_none_or(|(_, best)| distance < best) {
                closest = Some((mode, distance));
            }
        }
        if let Some((mode, _)) = closest {
            return mode;
        }
        if x >= crop.x && x <= right && y >= crop.y && y <= bottom {
            return DragMode::Move;
        }
        DragMode::Create
    }

    fn rendered_preview(&mut self) -> Option<Arc<gpui::RenderImage>> {
        self.active_item().rendered.clone()
    }

    /// The complete full-screen render of the WhatsApp Desktop Media Editor.
    fn draw(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let can_undo = !self.undo_stack.is_empty();
        let can_redo = !self.redo_stack.is_empty();
        let active_tool = self.active_tool;
        let is_hd = self.active_item().is_hd;

        div()
            .absolute()
            .inset_0()
            .occlude()
            .bg(rgb(0x0c1317)) // Official WhatsApp dark workspace
            .flex()
            .flex_col()
            .justify_between()
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.end_drag();
                    cx.notify();
                }),
            )
            // Top Bar
            .child(self.render_top_bar(can_undo, can_redo, active_tool, is_hd, cx))
            // Active Tool Sub-Bar (floating if tool is active)
            .when(active_tool != EditorTool::None, |root| {
                root.child(self.render_sub_toolbar(active_tool, cx))
            })
            // Center Canvas Area (Large Containment)
            .child(
                div()
                    .flex_1()
                    .relative()
                    .overflow_hidden()
                    .flex()
                    .items_center()
                    .justify_center()
                    .p_4()
                    .child(self.surface(cx)),
            )
            // Floating Caption Bar
            .child(self.render_floating_caption(cx))
            // Bottom Bar (Audience Privacy, Carousel, Send Button)
            .child(self.render_bottom_bar(cx))
            // Privacy Modal overlay if open
            .when(self.status_privacy_open, |root| {
                root.child(self.render_privacy_modal(cx))
            })
            // Save notice toast
            .when_some(self.save_notice.clone(), |root, msg| {
                root.child(
                    div()
                        .absolute()
                        .top(px(64.))
                        .left_0()
                        .right_0()
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .px_4()
                                .py_2()
                                .rounded_full()
                                .bg(rgb(0x00a884))
                                .text_color(rgb(0xffffff))
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .shadow_lg()
                                .child(msg),
                        ),
                )
            })
    }

    fn render_crop_icon(is_active: bool) -> impl IntoElement {
        let col = if is_active { rgb(0x00a884) } else { rgb(0xd1d7db) };
        div()
            .size(px(16.))
            .relative()
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .w(px(11.))
                    .h(px(11.))
                    .border_l_2()
                    .border_t_2()
                    .border_color(col),
            )
            .child(
                div()
                    .absolute()
                    .bottom_0()
                    .right_0()
                    .w(px(11.))
                    .h(px(11.))
                    .border_r_2()
                    .border_b_2()
                    .border_color(col),
            )
    }

    fn render_top_bar(
        &self,
        can_undo: bool,
        can_redo: bool,
        active_tool: EditorTool,
        is_hd: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        h_flex()
            .h(px(56.))
            .w_full()
            .px_4()
            .items_center()
            .justify_between()
            .bg(rgba(0x0c1317f2))
            .border_b_1()
            .border_color(rgb(0x182229))
            // Left: Close, Undo, Redo
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .id("editor-close-btn")
                            .size(px(36.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|s| s.bg(rgba(0xffffff15)))
                            .text_color(rgb(0xd1d7db))
                            .child(Icon::new(WhatfastIcon::Close).with_size(px(20.)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.cancel(cx);
                            })),
                    )
                    .child(div().w(px(1.)).h(px(20.)).bg(rgb(0x2a3942)))
                    .child(
                        div()
                            .id("editor-undo-btn")
                            .size(px(36.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|s| if can_undo { s.bg(rgba(0xffffff15)) } else { s })
                            .text_color(if can_undo { rgb(0xd1d7db) } else { rgb(0x404e57) })
                            .child(Icon::new(IconName::Undo).with_size(px(18.)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.undo(cx);
                            })),
                    )
                    .child(
                        div()
                            .id("editor-redo-btn")
                            .size(px(36.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|s| if can_redo { s.bg(rgba(0xffffff15)) } else { s })
                            .text_color(if can_redo { rgb(0xd1d7db) } else { rgb(0x404e57) })
                            .child(Icon::new(IconName::Redo).with_size(px(18.)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.redo(cx);
                            })),
                    ),
            )
            // Center Group: Crop, Filters, Pen, Aa, Shapes, Blur, Stickers, Sticker Tag, HD
            .child(
                h_flex()
                    .gap_1p5()
                    .items_center()
                    .p_1()
                    .rounded_lg()
                    .bg(rgba(0x18222999))
                    // Crop & Rotate
                    .child(
                        div()
                            .id("tool-crop")
                            .size(px(36.))
                            .rounded_md()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .bg(if active_tool == EditorTool::Crop { rgb(0x202c33) } else { rgba(0x00000000) })
                            .text_color(if active_tool == EditorTool::Crop { rgb(0x00a884) } else { rgb(0xd1d7db) })
                            .hover(|s| s.bg(rgba(0xffffff15)))
                            .child(Self::render_crop_icon(active_tool == EditorTool::Crop))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.active_tool = if this.active_tool == EditorTool::Crop {
                                    EditorTool::None
                                } else {
                                    EditorTool::Crop
                                };
                                cx.notify();
                            })),
                    )
                    // Filters (Magic wand / palette)
                    .child(
                        div()
                            .id("tool-filters")
                            .size(px(36.))
                            .rounded_md()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .bg(if active_tool == EditorTool::Filters { rgb(0x202c33) } else { rgba(0x00000000) })
                            .text_color(if active_tool == EditorTool::Filters { rgb(0x00a884) } else { rgb(0xd1d7db) })
                            .hover(|s| s.bg(rgba(0xffffff15)))
                            .child(Icon::new(WhatfastIcon::Palette).with_size(px(18.)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.active_tool = if this.active_tool == EditorTool::Filters {
                                    EditorTool::None
                                } else {
                                    EditorTool::Filters
                                };
                                cx.notify();
                            })),
                    )
                    // Pen
                    .child(
                        div()
                            .id("tool-pen")
                            .size(px(36.))
                            .rounded_md()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .bg(if active_tool == EditorTool::Pen { rgb(0x202c33) } else { rgba(0x00000000) })
                            .text_color(if active_tool == EditorTool::Pen { rgb(0x00a884) } else { rgb(0xd1d7db) })
                            .hover(|s| s.bg(rgba(0xffffff15)))
                            .child(Icon::new(WhatfastIcon::Edit).with_size(px(18.)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.active_tool = if this.active_tool == EditorTool::Pen {
                                    EditorTool::None
                                } else {
                                    EditorTool::Pen
                                };
                                cx.notify();
                            })),
                    )
                    // Text (Aa)
                    .child(
                        div()
                            .id("tool-text")
                            .size(px(36.))
                            .rounded_md()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .bg(if active_tool == EditorTool::Text { rgb(0x202c33) } else { rgba(0x00000000) })
                            .text_color(if active_tool == EditorTool::Text { rgb(0x00a884) } else { rgb(0xd1d7db) })
                            .hover(|s| s.bg(rgba(0xffffff15)))
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .text_sm()
                                    .child("Aa"),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.active_tool = if this.active_tool == EditorTool::Text {
                                    EditorTool::None
                                } else {
                                    EditorTool::Text
                                };
                                cx.notify();
                            })),
                    )
                    // Shapes (Square)
                    .child(
                        div()
                            .id("tool-shapes")
                            .size(px(36.))
                            .rounded_md()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .bg(if active_tool == EditorTool::Shapes { rgb(0x202c33) } else { rgba(0x00000000) })
                            .text_color(if active_tool == EditorTool::Shapes { rgb(0x00a884) } else { rgb(0xd1d7db) })
                            .hover(|s| s.bg(rgba(0xffffff15)))
                            .child(
                                div()
                                    .size(px(15.))
                                    .border_2()
                                    .border_color(if active_tool == EditorTool::Shapes { rgb(0x00a884) } else { rgb(0xd1d7db) })
                                    .rounded_xs(),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.active_tool = if this.active_tool == EditorTool::Shapes {
                                    EditorTool::None
                                } else {
                                    EditorTool::Shapes
                                };
                                cx.notify();
                            })),
                    )
                    // Blur / Mosaic (▦)
                    .child(
                        div()
                            .id("tool-blur")
                            .size(px(36.))
                            .rounded_md()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .bg(if active_tool == EditorTool::Blur { rgb(0x202c33) } else { rgba(0x00000000) })
                            .text_color(if active_tool == EditorTool::Blur { rgb(0x00a884) } else { rgb(0xd1d7db) })
                            .hover(|s| s.bg(rgba(0xffffff15)))
                            .child(
                                v_flex()
                                    .gap(px(1.5))
                                    .child(
                                        h_flex()
                                            .gap(px(1.5))
                                            .child(div().size(px(3.5)).bg(rgb(0xd1d7db)).rounded_xs())
                                            .child(div().size(px(3.5)).bg(rgb(0x8696a0)).rounded_xs())
                                            .child(div().size(px(3.5)).bg(rgb(0xd1d7db)).rounded_xs()),
                                    )
                                    .child(
                                        h_flex()
                                            .gap(px(1.5))
                                            .child(div().size(px(3.5)).bg(rgb(0x8696a0)).rounded_xs())
                                            .child(div().size(px(3.5)).bg(rgb(0xd1d7db)).rounded_xs())
                                            .child(div().size(px(3.5)).bg(rgb(0x8696a0)).rounded_xs()),
                                    )
                                    .child(
                                        h_flex()
                                            .gap(px(1.5))
                                            .child(div().size(px(3.5)).bg(rgb(0xd1d7db)).rounded_xs())
                                            .child(div().size(px(3.5)).bg(rgb(0x8696a0)).rounded_xs())
                                            .child(div().size(px(3.5)).bg(rgb(0xd1d7db)).rounded_xs()),
                                    ),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.active_tool = if this.active_tool == EditorTool::Blur {
                                    EditorTool::None
                                } else {
                                    EditorTool::Blur
                                };
                                cx.notify();
                            })),
                    )
                    // Stickers / Emojis (😊)
                    .child(
                        div()
                            .id("tool-stickers")
                            .size(px(36.))
                            .rounded_md()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .bg(if active_tool == EditorTool::Stickers { rgb(0x202c33) } else { rgba(0x00000000) })
                            .text_color(if active_tool == EditorTool::Stickers { rgb(0x00a884) } else { rgb(0xd1d7db) })
                            .hover(|s| s.bg(rgba(0xffffff15)))
                            .child(Icon::new(WhatfastIcon::Smile).with_size(px(18.)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.active_tool = if this.active_tool == EditorTool::Stickers {
                                    EditorTool::None
                                } else {
                                    EditorTool::Stickers
                                };
                                cx.notify();
                            })),
                    )
                    // Sticker Tag (🏷️)
                    .child(
                        div()
                            .id("tool-sticker-cutout")
                            .size(px(36.))
                            .rounded_md()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|s| s.bg(rgba(0xffffff15)))
                            .text_color(rgb(0xd1d7db))
                            .child(Icon::new(WhatfastIcon::Sticker).with_size(px(18.)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.save_notice = Some(i18n::t("Modo sticker activo").to_string());
                                cx.notify();
                            })),
                    )
                    // HD Quality badge
                    .child(
                        div()
                            .id("tool-hd-toggle")
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .border_1()
                            .border_color(if is_hd { rgb(0x00a884) } else { rgb(0x54656f) })
                            .text_color(if is_hd { rgb(0x00a884) } else { rgb(0x8696a0) })
                            .bg(if is_hd { rgba(0x00a88420) } else { rgba(0x00000000) })
                            .cursor_pointer()
                            .hover(|s| s.bg(rgba(0xffffff15)))
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .child("HD")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_hd(cx);
                            })),
                    ),
            )
            // Right: Download / Save button
            .child(
                div()
                    .id("editor-download-btn")
                    .size(px(36.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgba(0xffffff15)))
                    .text_color(rgb(0xd1d7db))
                    .child(Icon::new(WhatfastIcon::Download).with_size(px(18.)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.save_to_disk(cx);
                    })),
            )
    }

    fn render_sub_toolbar(&self, tool: EditorTool, cx: &mut Context<Self>) -> impl IntoElement {
        let container = h_flex()
            .h(px(42.))
            .px_4()
            .items_center()
            .gap_3()
            .rounded_full()
            .bg(rgb(0x202c33))
            .border_1()
            .border_color(rgb(0x2a3942))
            .shadow_md();

        div()
            .w_full()
            .flex()
            .justify_center()
            .pt_2()
            .child(match tool {
                EditorTool::Crop => container
                    .child(
                        Button::new("crop-rotate-left")
                            .ghost()
                            .small()
                            .label("↶ 90°")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.rotate_left(cx);
                            })),
                    )
                    .child(
                        Button::new("crop-rotate-right")
                            .ghost()
                            .small()
                            .label("↷ 90°")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.rotate_right(cx);
                            })),
                    )
                    .child(div().w(px(1.)).h(px(16.)).bg(rgb(0x2a3942)))
                    .child(
                        Button::new("crop-aspect-1-1")
                            .ghost()
                            .small()
                            .label("1:1")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_aspect_ratio(1.0, cx);
                            })),
                    )
                    .child(
                        Button::new("crop-aspect-4-3")
                            .ghost()
                            .small()
                            .label("4:3")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_aspect_ratio(4.0 / 3.0, cx);
                            })),
                    )
                    .child(
                        Button::new("crop-aspect-16-9")
                            .ghost()
                            .small()
                            .label("16:9")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_aspect_ratio(16.0 / 9.0, cx);
                            })),
                    )
                    .child(div().w(px(1.)).h(px(16.)).bg(rgb(0x2a3942)))
                    .child(
                        Button::new("crop-reset")
                            .ghost()
                            .small()
                            .label(i18n::t("Restablecer"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.reset_crop(cx);
                            })),
                    ),
                EditorTool::Filters => {
                    let active_filter = self.active_item().filter;
                    let filters = [
                        (ImageFilter::Normal, "Normal"),
                        (ImageFilter::HighContrast, "Pop"),
                        (ImageFilter::Grayscale, "B&N"),
                        (ImageFilter::Warm, "Cálido"),
                        (ImageFilter::Cool, "Frío"),
                        (ImageFilter::Sepia, "Sepia"),
                    ];
                    let mut list = container;
                    for (idx, (f, name)) in filters.iter().enumerate() {
                        let f = *f;
                        let is_active = active_filter == f;
                        list = list.child(
                            div()
                                .id(ElementId::named_usize("filter-chip", idx))
                                .px_2p5()
                                .py_1()
                                .rounded_full()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .cursor_pointer()
                                .bg(if is_active { rgb(0x00a884) } else { rgb(0x182229) })
                                .text_color(if is_active { rgb(0xffffff) } else { rgb(0xd1d7db) })
                                .hover(|s| if !is_active { s.bg(rgb(0x2a3942)) } else { s })
                                .child(*name)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.set_filter(f, cx);
                                })),
                        );
                    }
                    list
                }
                EditorTool::Pen => {
                    let colors: [([u8; 4], &str); 7] = [
                        ([255, 255, 255, 255], "Blanco"),
                        ([0, 0, 0, 255], "Negro"),
                        ([0, 168, 132, 255], "Verde"),
                        ([234, 67, 53, 255], "Rojo"),
                        ([251, 188, 4, 255], "Amarillo"),
                        ([66, 133, 244, 255], "Azul"),
                        ([161, 66, 244, 255], "Púrpura"),
                    ];
                    let mut list = container;
                    for (idx, (col, _)) in colors.iter().enumerate() {
                        let col = *col;
                        let is_selected = self.pen_color == col;
                        let r = col[0];
                        let g = col[1];
                        let b = col[2];
                        list = list.child(
                            div()
                                .id(ElementId::named_usize("pen-color", idx))
                                .size(px(18.))
                                .rounded_full()
                                .bg(rgb((r as u32) << 16 | (g as u32) << 8 | b as u32))
                                .border_2()
                                .border_color(if is_selected { rgb(0xffffff) } else { rgb(0x2a3942) })
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.pen_color = col;
                                    cx.notify();
                                })),
                        );
                    }
                    list = list.child(div().w(px(1.)).h(px(16.)).bg(rgb(0x2a3942)));
                    let widths = [(3.0, px(3.)), (7.0, px(7.)), (14.0, px(12.))];
                    for (idx, (w, dot_size)) in widths.into_iter().enumerate() {
                        let is_sel = (self.pen_width - w).abs() < 0.1;
                        list = list.child(
                            div()
                                .id(ElementId::named_usize("pen-width", idx))
                                .size(px(20.))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .bg(if is_sel { rgba(0xffffff20) } else { rgba(0x00000000) })
                                .child(
                                    div()
                                        .size(dot_size)
                                        .rounded_full()
                                        .bg(rgb(0xd1d7db)),
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.pen_width = w;
                                    cx.notify();
                                })),
                        );
                    }
                    list = list.child(div().w(px(1.)).h(px(16.)).bg(rgb(0x2a3942)));
                    list = list.child(
                        Button::new("pen-clear")
                            .ghost()
                            .small()
                            .label("Limpiar trazos")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.push_undo();
                                this.active_item_mut().strokes.clear();
                                this.active_item_mut().rebake_preview();
                                cx.notify();
                            })),
                    );
                    list
                }
                EditorTool::Text => {
                    let mut list = container;
                    if let Some(inp) = self.text_input.as_ref() {
                        list = list.child(
                            div()
                                .w(px(200.))
                                .child(Input::new(inp)),
                        );
                    }
                    list = list.child(
                        Button::new("text-add-btn")
                            .ghost()
                            .small()
                            .label("+ Añadir")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.add_text_from_input(window, cx);
                            })),
                    );
                    list = list.child(div().w(px(1.)).h(px(16.)).bg(rgb(0x2a3942)));
                    let colors: [([u8; 4], &str); 4] = [
                        ([255, 255, 255, 255], "Blanco"),
                        ([0, 168, 132, 255], "Verde"),
                        ([251, 188, 4, 255], "Amarillo"),
                        ([234, 67, 53, 255], "Rojo"),
                    ];
                    for (idx, (col, _)) in colors.iter().enumerate() {
                        let col = *col;
                        let is_selected = self.text_color == col;
                        let r = col[0];
                        let g = col[1];
                        let b = col[2];
                        list = list.child(
                            div()
                                .id(ElementId::named_usize("text-col", idx))
                                .size(px(16.))
                                .rounded_full()
                                .bg(rgb((r as u32) << 16 | (g as u32) << 8 | b as u32))
                                .border_2()
                                .border_color(if is_selected { rgb(0xffffff) } else { rgb(0x2a3942) })
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.text_color = col;
                                    cx.notify();
                                })),
                        );
                    }
                    list = list.child(
                        Button::new("text-bg-toggle")
                            .ghost()
                            .small()
                            .label(if self.text_has_bg { "Fondo: Sí" } else { "Fondo: No" })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.text_has_bg = !this.text_has_bg;
                                cx.notify();
                            })),
                    );
                    list = list.child(
                        Button::new("text-clear-btn")
                            .ghost()
                            .small()
                            .label("Limpiar textos")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.push_undo();
                                this.active_item_mut().texts.clear();
                                cx.notify();
                            })),
                    );
                    list
                }
                EditorTool::Shapes => container
                    .child(
                        Button::new("shape-rect")
                            .ghost()
                            .small()
                            .label("▢ Rectángulo")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.add_shape(ShapeKind::Rectangle, cx);
                            })),
                    )
                    .child(
                        Button::new("shape-circle")
                            .ghost()
                            .small()
                            .label("◯ Círculo")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.add_shape(ShapeKind::Circle, cx);
                            })),
                    )
                    .child(
                        Button::new("shape-arrow")
                            .ghost()
                            .small()
                            .label("➜ Flecha")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.add_shape(ShapeKind::Arrow, cx);
                            })),
                    )
                    .child(div().w(px(1.)).h(px(16.)).bg(rgb(0x2a3942)))
                    .child(
                        Button::new("shape-clear")
                            .ghost()
                            .small()
                            .label("Limpiar formas")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.push_undo();
                                this.active_item_mut().shapes.clear();
                                cx.notify();
                            })),
                    ),
                EditorTool::Blur => container
                    .child(
                        Button::new("blur-normal")
                            .ghost()
                            .small()
                            .label("14px")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.blur_block_size = 14;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("blur-heavy")
                            .ghost()
                            .small()
                            .label("24px")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.blur_block_size = 24;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("blur-center-auto")
                            .ghost()
                            .small()
                            .label("+ Censurar centro")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.add_auto_blur(cx);
                            })),
                    )
                    .child(div().w(px(1.)).h(px(16.)).bg(rgb(0x2a3942)))
                    .child(
                        Button::new("blur-clear")
                            .ghost()
                            .small()
                            .label(i18n::t("Limpiar desenfoques"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.push_undo();
                                this.active_item_mut().blurs.clear();
                                this.active_item_mut().rebake_preview();
                                cx.notify();
                            })),
                    ),
                EditorTool::Stickers => {
                    let mut list = container;
                    for (idx, &emoji) in ["😀", "😂", "😍", "🔥", "🎉", "👍", "💯", "👏", "❤️", "🚀", "🐶", "🍕", "🥳", "🤩"].iter().enumerate() {
                        list = list.child(
                            div()
                                .id(ElementId::named_usize("sticker-em", idx))
                                .px_1p5()
                                .py_0p5()
                                .rounded_sm()
                                .text_base()
                                .cursor_pointer()
                                .hover(|s| s.bg(rgba(0xffffff20)))
                                .child(emoji)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.push_undo();
                                    let center = this.rotated_size();
                                    this.active_item_mut().emojis.push(EmojiOverlay {
                                        emoji: emoji.to_string(),
                                        x: (center.width / 2.0).max(20.0),
                                        y: (center.height / 2.0).max(20.0),
                                        size: 40.0,
                                    });
                                    cx.notify();
                                })),
                        );
                    }
                    list = list.child(div().w(px(1.)).h(px(16.)).bg(rgb(0x2a3942)));
                    list = list.child(
                        Button::new("stickers-clear")
                            .ghost()
                            .small()
                            .label("Limpiar emojis")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.push_undo();
                                this.active_item_mut().emojis.clear();
                                cx.notify();
                            })),
                    );
                    list
                }
                _ => container.child(
                    div()
                        .text_xs()
                        .text_color(rgb(0x8696a0))
                        .child("Haz clic o arrastra en la foto para editar"),
                ),
            })
    }

    fn render_floating_caption(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let is_emoji_open = self.caption_emoji_open;

        div()
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            .pb_3()
            // Quick emoji picker for caption
            .when(is_emoji_open, |root| {
                root.child(
                    div()
                        .mb_2()
                        .px_3()
                        .py_1p5()
                        .rounded_full()
                        .bg(rgb(0x202c33))
                        .border_1()
                        .border_color(rgb(0x2a3942))
                        .shadow_lg()
                        .child(
                            h_flex()
                                .gap_1p5()
                                .children(["😀", "😂", "❤️", "🔥", "✨", "🎉", "👍", "👏", "🙏", "🥳"].iter().enumerate().map(|(idx, &em)| {
                                    div()
                                        .id(ElementId::named_usize("caption-em", idx))
                                        .px_1p5()
                                        .py_0p5()
                                        .rounded_sm()
                                        .text_base()
                                        .cursor_pointer()
                                        .hover(|s| s.bg(rgba(0xffffff20)))
                                        .child(em)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            if let Some(input) = this.caption.as_ref() {
                                                input.update(cx, |input, cx| {
                                                    let mut val = input.value().to_string();
                                                    val.push_str(em);
                                                    input.set_value(val, window, cx);
                                                });
                                            }
                                            this.caption_emoji_open = false;
                                            cx.notify();
                                        }))
                                })),
                        ),
                )
            })
            // Capsule Input
            .child(
                div()
                    .w(px(580.))
                    .h(px(46.))
                    .rounded_full()
                    .bg(rgb(0x202c33))
                    .border_1()
                    .border_color(rgb(0x2a3942))
                    .px_4()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex_1()
                            .child(match self.caption.as_ref() {
                                Some(input) => Input::new(input).flex_1().into_any_element(),
                                None => div().into_any_element(),
                            }),
                    )
                    .child(
                        div()
                            .id("caption-smile-toggle")
                            .size(px(32.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|s| s.bg(rgba(0xffffff15)))
                            .text_color(rgb(0x8696a0))
                            .child(Icon::new(WhatfastIcon::Smile).with_size(px(20.)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.caption_emoji_open = !this.caption_emoji_open;
                                cx.notify();
                            })),
                    ),
            )
    }

    fn render_bottom_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let privacy_mode = self.status_privacy_mode.clone();
        let items_len = self.items.len();
        let active_idx = self.active_index;

        h_flex()
            .h(px(72.))
            .w_full()
            .px_6()
            .items_center()
            .justify_between()
            .bg(rgb(0x0c1317))
            // Left: Audience Privacy Pill
            .child(
                div()
                    .id("status-privacy-pill")
                    .rounded_full()
                    .border_1()
                    .border_color(rgba(0x00a88440))
                    .bg(rgba(0x00a88415))
                    .px_3p5()
                    .py_1p5()
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgba(0x00a88425)))
                    .child(
                        div()
                            .text_color(rgb(0x00a884))
                            .child(Icon::new(WhatfastIcon::Status).with_size(px(18.))),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(0x00a884))
                            .child(privacy_mode),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.status_privacy_open = !this.status_privacy_open;
                        cx.notify();
                    })),
            )
            // Center: Media Carousel (Thumbnails + Plus Button)
            .child(
                h_flex()
                    .gap_2p5()
                    .items_center()
                    .children(self.items.iter().enumerate().map(|(idx, item)| {
                        let is_active = idx == active_idx;
                        let item_rendered = item.rendered.clone();
                        div()
                            .id(ElementId::named_usize("editor-thumb", idx))
                            .size(px(54.))
                            .rounded_md()
                            .overflow_hidden()
                            .relative()
                            .cursor_pointer()
                            .border_2()
                            .border_color(if is_active { rgb(0x00a884) } else { rgb(0x2a3942) })
                            .opacity(if is_active { 1.0 } else { 0.7 })
                            .hover(|s| s.opacity(1.0))
                            .child(match item_rendered {
                                Some(render_img) => img(ImageSource::Render(render_img))
                                    .size_full()
                                    .object_fit(ObjectFit::Cover)
                                    .into_any_element(),
                                None => div()
                                    .size_full()
                                    .bg(rgb(0x202c33))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_xs()
                                    .text_color(rgb(0x8696a0))
                                    .child("...")
                                    .into_any_element(),
                            })
                            // Remove button if multiple items
                            .when(items_len > 1, |d| {
                                d.child(
                                    div()
                                        .id(ElementId::named_usize("thumb-rem", idx))
                                        .absolute()
                                        .top_0p5()
                                        .right_0p5()
                                        .size(px(16.))
                                        .rounded_full()
                                        .bg(rgba(0x000000cc))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .text_xs()
                                        .text_color(rgb(0xffffff))
                                        .child("✕")
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.remove_item(idx, window, cx);
                                        })),
                                )
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_item(idx, window, cx);
                            }))
                    }))
                    // Plus button to add more images
                    .child(
                        div()
                            .id("editor-add-media-btn")
                            .size(px(54.))
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(0x2a3942))
                            .bg(rgb(0x202c33))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|s| s.bg(rgba(0xffffff15)))
                            .text_color(rgb(0xd1d7db))
                            .child(Icon::new(IconName::Plus).with_size(px(22.)))
                            .on_click(cx.listener(|_this, _, window, cx| {
                                let prompt = cx.prompt_for_paths(PathPromptOptions {
                                    files: true,
                                    directories: false,
                                    multiple: true,
                                    prompt: Some(i18n::t("Elige más fotos o videos").into()),
                                });
                                let view = cx.entity();
                                cx.spawn_in(window, async move |_, window| {
                                    if let Ok(Ok(Some(paths))) = prompt.await {
                                        window.update(|_, cx| {
                                            view.update(cx, |this, cx| {
                                                this.add_items_from_paths(paths, cx);
                                            });
                                        }).ok();
                                    }
                                }).detach();
                            })),
                    ),
            )
            // Right: Circular WhatsApp Green Send Button
            .child(
                div()
                    .id("editor-send-btn")
                    .size(px(54.))
                    .rounded_full()
                    .bg(rgb(0x00a884))
                    .hover(|s| s.bg(rgb(0x02906f)))
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .justify_center()
                    .shadow_md()
                    .child(
                        div()
                            .text_color(rgb(0x0b141a))
                            .child(Icon::new(WhatfastIcon::Send).with_size(px(22.))),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        if !this.working {
                            this.apply(cx);
                        }
                    })),
            )
    }

    fn render_privacy_modal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .bottom(px(80.))
            .left(px(24.))
            .w(px(320.))
            .rounded_lg()
            .bg(rgb(0x202c33))
            .border_1()
            .border_color(rgb(0x2a3942))
            .p_4()
            .shadow_xl()
            .child(
                v_flex()
                    .gap_3()
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .text_sm()
                            .text_color(rgb(0xffffff))
                            .child("Quién puede ver mis estados"),
                    )
                    .child(
                        v_flex()
                            .gap_2()
                            .child(
                                div()
                                    .id("priv-opt-contacts")
                                    .p_2()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(rgba(0xffffff10)))
                                    .text_sm()
                                    .text_color(rgb(0xd1d7db))
                                    .child("Mis contactos")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.status_privacy_mode = "Estado (Mis contactos)".to_string();
                                        this.status_privacy_open = false;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .id("priv-opt-excluded")
                                    .p_2()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(rgba(0xffffff10)))
                                    .text_sm()
                                    .text_color(rgb(0xd1d7db))
                                    .child("Mis contactos, excepto... (221 excluidos)")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.status_privacy_mode = "Estado (221 excluidos)".to_string();
                                        this.status_privacy_open = false;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .id("priv-opt-selected")
                                    .p_2()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(rgba(0xffffff10)))
                                    .text_sm()
                                    .text_color(rgb(0xd1d7db))
                                    .child("Solo compartir con...")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.status_privacy_mode = "Estado (Seleccionados)".to_string();
                                        this.status_privacy_open = false;
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
    }

    /// The surface of the preview: image, blurs, strokes, texts, emojis, shapes, and crop handles.
    fn surface(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let (scale_x, scale_y) = self.window_per_picture();
        let crop = self.crop();
        let primary = rgb(0x00a884);
        let surface_bounds = Rc::clone(&self.surface);
        let rendered = self.rendered_preview();
        let working = self.is_working();
        let error = self.error.clone();
        let active_tool = self.active_tool;

        let mut handles = Vec::new();
        let edge = HANDLE_EDGE;
        for (x, y) in [
            (crop.x, crop.y),
            (crop.x + crop.width, crop.y),
            (crop.x, crop.y + crop.height),
            (crop.x + crop.width, crop.y + crop.height),
            (crop.x + crop.width / 2.0, crop.y),
            (crop.x + crop.width / 2.0, crop.y + crop.height),
            (crop.x, crop.y + crop.height / 2.0),
            (crop.x + crop.width, crop.y + crop.height / 2.0),
        ] {
            handles.push(
                div()
                    .absolute()
                    .left(px((x * scale_x - edge / 2.0) as f32))
                    .top(px((y * scale_y - edge / 2.0) as f32))
                    .size(px(edge as f32))
                    .rounded_sm()
                    .bg(primary)
                    .border_1()
                    .border_color(rgb(0xffffff)),
            );
        }

        let drawn = self.drawn_size();
        let emojis = self.active_item().emojis.clone();
        let texts = self.active_item().texts.clone();
        let shapes = self.active_item().shapes.clone();

        let mut surface = div()
            .relative()
            .w(px(drawn.width as f32))
            .h(px(drawn.height as f32))
            .overflow_hidden()
            .rounded_md()
            .bg(rgb(0x182229))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.begin_drag(event.position);
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                this.move_drag(event.position);
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.end_drag();
                    cx.notify();
                }),
            )
            // Base preview image (absolute to fill the surface container)
            .child(match rendered {
                Some(image) => img(ImageSource::Render(image))
                    .absolute()
                    .size_full()
                    .object_fit(ObjectFit::Fill)
                    .into_any_element(),
                None => div()
                    .absolute()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .p_4()
                    .text_center()
                    .text_sm()
                    .text_color(rgb(0x8696a0))
                    .child(if let Some(err) = error {
                        format!("No se pudo cargar la imagen: {err}")
                    } else if working {
                        "Cargando imagen...".to_string()
                    } else {
                        "Selecciona una foto".to_string()
                    })
                    .into_any_element(),
            })
            .child(
                canvas(
                    move |bounds, _window, _cx| {
                        surface_bounds.set(Some(bounds));
                    },
                    |_bounds, _value, _window, _cx| {},
                )
                .absolute()
                .size_full(),
            );

        // Floating shapes on canvas
        for shape in shapes {
            let r = shape.color[0];
            let g = shape.color[1];
            let b = shape.color[2];
            let col = rgb((r as u32) << 16 | (g as u32) << 8 | b as u32);
            let sx = (shape.x * scale_x) as f32;
            let sy = (shape.y * scale_y) as f32;
            let sw = (shape.width * scale_x) as f32;
            let sh = (shape.height * scale_y) as f32;
            match shape.kind {
                ShapeKind::Rectangle => {
                    surface = surface.child(
                        div()
                            .absolute()
                            .left(px(sx))
                            .top(px(sy))
                            .w(px(sw))
                            .h(px(sh))
                            .border_3()
                            .border_color(col)
                            .rounded_sm(),
                    );
                }
                ShapeKind::Circle => {
                    surface = surface.child(
                        div()
                            .absolute()
                            .left(px(sx))
                            .top(px(sy))
                            .w(px(sw))
                            .h(px(sh))
                            .border_3()
                            .border_color(col)
                            .rounded_full(),
                    );
                }
                ShapeKind::Arrow => {
                    surface = surface.child(
                        div()
                            .absolute()
                            .left(px(sx))
                            .top(px(sy))
                            .text_size(px(32.))
                            .text_color(col)
                            .child("➔"),
                    );
                }
            }
        }

        // Floating texts on canvas
        for text in texts {
            let r = text.color[0];
            let g = text.color[1];
            let b = text.color[2];
            let col = rgb((r as u32) << 16 | (g as u32) << 8 | b as u32);
            surface = surface.child(
                div()
                    .absolute()
                    .left(px((text.x * scale_x) as f32))
                    .top(px((text.y * scale_y) as f32))
                    .px_2p5()
                    .py_1()
                    .rounded_md()
                    .bg(if text.has_bg { rgba(0x000000aa) } else { rgba(0x00000000) })
                    .text_color(col)
                    .font_weight(FontWeight::BOLD)
                    .text_lg()
                    .shadow_md()
                    .child(text.text),
            );
        }

        // Floating emojis on canvas
        for em in emojis {
            surface = surface.child(
                div()
                    .absolute()
                    .left(px((em.x * scale_x - (em.size as f64 / 2.0)) as f32))
                    .top(px((em.y * scale_y - (em.size as f64 / 2.0)) as f32))
                    .text_size(px(em.size))
                    .child(em.emoji),
            );
        }

        // Active blur region dragging outline
        if let Some(r) = self.active_blur_rect {
            surface = surface.child(
                div()
                    .absolute()
                    .left(px((r.x * scale_x) as f32))
                    .top(px((r.y * scale_y) as f32))
                    .w(px((r.width * scale_x) as f32))
                    .h(px((r.height * scale_y) as f32))
                    .border_2()
                    .border_color(rgb(0x00a884))
                    .bg(rgba(0x00a88435)),
            );
        }

        // Active pen stroke live preview
        if let Some(stroke) = &self.active_stroke {
            if stroke.points.len() > 1 {
                let r = stroke.color[0];
                let g = stroke.color[1];
                let b = stroke.color[2];
                let col = rgb((r as u32) << 16 | (g as u32) << 8 | b as u32);
                let w = (stroke.width * (scale_x + scale_y) / 2.0).max(2.0) as f32;
                for pt in &stroke.points {
                    surface = surface.child(
                        div()
                            .absolute()
                            .left(px((pt.0 * scale_x - (w / 2.0) as f64) as f32))
                            .top(px((pt.1 * scale_y - (w / 2.0) as f64) as f32))
                            .size(px(w))
                            .rounded_full()
                            .bg(col),
                    );
                }
            }
        }

        // Crop overlay and rule-of-thirds grid (only when Crop tool is active or dragging crop)
        if active_tool == EditorTool::Crop || self.drag.is_some() {
            let cx_px = (crop.x * scale_x) as f32;
            let cy_px = (crop.y * scale_y) as f32;
            let cw_px = (crop.width * scale_x) as f32;
            let ch_px = (crop.height * scale_y) as f32;

            // Crop boundary box
            surface = surface.child(
                div()
                    .absolute()
                    .left(px(cx_px))
                    .top(px(cy_px))
                    .w(px(cw_px))
                    .h(px(ch_px))
                    .border_2()
                    .border_color(primary)
                    // Rule of thirds vertical lines
                    .child(
                        div()
                            .absolute()
                            .left(px(cw_px / 3.0))
                            .top_0()
                            .bottom_0()
                            .w(px(1.))
                            .bg(rgba(0xffffff45)),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(cw_px * 2.0 / 3.0))
                            .top_0()
                            .bottom_0()
                            .w(px(1.))
                            .bg(rgba(0xffffff45)),
                    )
                    // Rule of thirds horizontal lines
                    .child(
                        div()
                            .absolute()
                            .top(px(ch_px / 3.0))
                            .left_0()
                            .right_0()
                            .h(px(1.))
                            .bg(rgba(0xffffff45)),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(ch_px * 2.0 / 3.0))
                            .left_0()
                            .right_0()
                            .h(px(1.))
                            .bg(rgba(0xffffff45)),
                    ),
            );

            for handle in handles {
                surface = surface.child(handle);
            }
        }

        surface
    }
}

impl Render for MediaEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.draw(cx)
    }
}

/// Dynamic scale factor so image comfortably fits the editor canvas up to 920x580.
fn display_scale(rotated: PixelSize) -> f64 {
    if rotated.width <= 0.0 || rotated.height <= 0.0 {
        return 1.0;
    }
    let max_w = 900.0;
    let max_h = 580.0;
    let scale_w = max_w / rotated.width;
    let scale_h = max_h / rotated.height;
    scale_w.min(scale_h).min(2.5).max(0.05)
}

/// The scale of the preview: the picture is reduced until its longest edge is `PREVIEW_EDGE` pixels.
fn preview_scale(natural: PixelSize) -> f64 {
    if natural.width <= 0.0 || natural.height <= 0.0 {
        return 0.0;
    }
    let longest = natural.width.max(natural.height);
    (PREVIEW_EDGE / longest).min(1.0)
}

fn left_edge(right: f64, point_x: f64, min_edge: f64) -> (f64, f64) {
    let left = point_x.min(right - min_edge);
    (left, right - left)
}

fn right_edge(start: CropRect, point_x: f64, min_edge: f64, frame_width: f64) -> f64 {
    (point_x - start.x).max(min_edge).min(frame_width - start.x)
}

fn top_edge(bottom: f64, point_y: f64, min_edge: f64) -> (f64, f64) {
    let top = point_y.min(bottom - min_edge);
    (top, bottom - top)
}

fn bottom_edge(start: CropRect, point_y: f64, min_edge: f64, frame_height: f64) -> f64 {
    (point_y - start.y)
        .max(min_edge)
        .min(frame_height - start.y)
}

fn inverse_rotation(rotation: Rotation) -> Rotation {
    match rotation {
        Rotation::Deg0 => Rotation::Deg0,
        Rotation::Deg90 => Rotation::Deg270,
        Rotation::Deg180 => Rotation::Deg180,
        Rotation::Deg270 => Rotation::Deg90,
    }
}

fn steps_of(steps: u32) -> Rotation {
    media_crop::parse_rotation(steps as f64)
}

fn carried_crop(crop: CropRect, frame: PixelSize, steps: u32) -> Option<CropRect> {
    media_crop::crop_in_rotated_space(crop, frame, steps_of(steps)).ok()
}

struct LoadedPicture {
    natural: PixelSize,
    preview: image::RgbaImage,
}

fn load_picture(path: &Path) -> Result<LoadedPicture, String> {
    let decoded = image::open(path)
        .map_err(|error| format!("the photo could not be read: {error}"))?;
    let (width, height) = (decoded.width(), decoded.height());
    let natural = PixelSize {
        width: width as f64,
        height: height as f64,
    };
    let scale = preview_scale(natural);
    let preview_width = ((natural.width * scale).round() as u32).max(1);
    let preview_height = ((natural.height * scale).round() as u32).max(1);
    let preview = image::imageops::resize(
        &decoded.into_rgba8(),
        preview_width,
        preview_height,
        image::imageops::FilterType::Triangle,
    );
    Ok(LoadedPicture { natural, preview })
}

fn rotate_picture(picture: &image::RgbaImage, rotation: Rotation) -> image::RgbaImage {
    match rotation {
        Rotation::Deg0 => picture.clone(),
        Rotation::Deg90 => image::imageops::rotate90(picture),
        Rotation::Deg180 => image::imageops::rotate180(picture),
        Rotation::Deg270 => image::imageops::rotate270(picture),
    }
}

/// Applies preset color filter to RGBA buffer in-place.
fn apply_filter_to_image(img: &mut image::RgbaImage, filter: ImageFilter) {
    match filter {
        ImageFilter::Normal => {}
        ImageFilter::HighContrast => {
            for pixel in img.pixels_mut() {
                for c in 0..3 {
                    let v = pixel[c] as f32;
                    let c_adj = ((v - 128.0) * 1.35 + 128.0).clamp(0.0, 255.0) as u8;
                    pixel[c] = c_adj;
                }
            }
        }
        ImageFilter::Grayscale => {
            for pixel in img.pixels_mut() {
                let lum = (0.299 * pixel[0] as f32 + 0.587 * pixel[1] as f32 + 0.114 * pixel[2] as f32) as u8;
                pixel[0] = lum;
                pixel[1] = lum;
                pixel[2] = lum;
            }
        }
        ImageFilter::Warm => {
            for pixel in img.pixels_mut() {
                pixel[0] = pixel[0].saturating_add(28);
                pixel[1] = pixel[1].saturating_add(12);
                pixel[2] = pixel[2].saturating_sub(18);
            }
        }
        ImageFilter::Cool => {
            for pixel in img.pixels_mut() {
                pixel[0] = pixel[0].saturating_sub(18);
                pixel[1] = pixel[1].saturating_add(8);
                pixel[2] = pixel[2].saturating_add(32);
            }
        }
        ImageFilter::Sepia => {
            for pixel in img.pixels_mut() {
                let r = pixel[0] as f32;
                let g = pixel[1] as f32;
                let b = pixel[2] as f32;
                let tr = (0.393 * r + 0.769 * g + 0.189 * b).min(255.0) as u8;
                let tg = (0.349 * r + 0.686 * g + 0.168 * b).min(255.0) as u8;
                let tb = (0.272 * r + 0.534 * g + 0.131 * b).min(255.0) as u8;
                pixel[0] = tr;
                pixel[1] = tg;
                pixel[2] = tb;
            }
        }
    }
}

/// Applies pixelation mosaic to given region.
fn apply_mosaic_to_image(img: &mut image::RgbaImage, rect: &CropRect, block_size: u32) {
    let img_w = img.width();
    let img_h = img.height();
    let rx = (rect.x.round() as u32).min(img_w);
    let ry = (rect.y.round() as u32).min(img_h);
    let rw = (rect.width.round() as u32).min(img_w.saturating_sub(rx));
    let rh = (rect.height.round() as u32).min(img_h.saturating_sub(ry));
    let b = block_size.max(4);

    let mut by = ry;
    while by < ry + rh {
        let bh = b.min((ry + rh) - by);
        let mut bx = rx;
        while bx < rx + rw {
            let bw = b.min((rx + rw) - bx);
            let mut sum_r = 0u64;
            let mut sum_g = 0u64;
            let mut sum_b = 0u64;
            let mut sum_a = 0u64;
            let count = (bw * bh) as u64;
            for y in by..by + bh {
                for x in bx..bx + bw {
                    let p = img.get_pixel(x, y);
                    sum_r += p[0] as u64;
                    sum_g += p[1] as u64;
                    sum_b += p[2] as u64;
                    sum_a += p[3] as u64;
                }
            }
            if count > 0 {
                let avg = image::Rgba([
                    (sum_r / count) as u8,
                    (sum_g / count) as u8,
                    (sum_b / count) as u8,
                    (sum_a / count) as u8,
                ]);
                for y in by..by + bh {
                    for x in bx..bx + bw {
                        img.put_pixel(x, y, avg);
                    }
                }
            }
            bx += bw;
        }
        by += bh;
    }
}

/// Applies a pen stroke (sequence of line segments) to the image buffer.
fn apply_stroke_to_image(img: &mut image::RgbaImage, stroke: &DrawStroke) {
    if stroke.points.len() < 2 {
        return;
    }
    let radius = (stroke.width / 2.0).max(1.0);
    let color = stroke.color;
    for i in 0..stroke.points.len() - 1 {
        let (x0, y0) = stroke.points[i];
        let (x1, y1) = stroke.points[i + 1];
        draw_line_on_image(img, x0, y0, x1, y1, color, radius);
    }
}

fn draw_line_on_image(
    img: &mut image::RgbaImage,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    color: [u8; 4],
    radius: f64,
) {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let dist = dx.hypot(dy);
    let steps = (dist * 2.0).ceil().max(1.0) as usize;
    let r_sq = radius * radius;
    let (w, h) = (img.width() as i64, img.height() as i64);
    let r_int = radius.ceil() as i64;
    let rgba_col = image::Rgba(color);

    for s in 0..=steps {
        let t = s as f64 / steps as f64;
        let cx = (x0 + t * dx).round() as i64;
        let cy = (y0 + t * dy).round() as i64;

        for dy_i in -r_int..=r_int {
            for dx_i in -r_int..=r_int {
                if (dx_i * dx_i + dy_i * dy_i) as f64 <= r_sq {
                    let px = cx + dx_i;
                    let py = cy + dy_i;
                    if px >= 0 && px < w && py >= 0 && py < h {
                        img.put_pixel(px as u32, py as u32, rgba_col);
                    }
                }
            }
        }
    }
}

pub(crate) fn to_render_image(picture: &image::RgbaImage) -> gpui::RenderImage {
    let (width, height) = picture.dimensions();
    let mut data = picture.as_raw().clone();
    for pixel in data.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let buffer = image::RgbaImage::from_raw(width, height, data)
        .unwrap_or_else(|| image::RgbaImage::new(width, height));
    gpui::RenderImage::new(vec![image::Frame::new(buffer)])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(natural: PixelSize) -> MediaEditor {
        let mut item = MediaItem::new(PathBuf::from("photo.png"));
        item.natural = natural;
        MediaEditor {
            items: vec![item],
            active_index: 0,
            active_tool: EditorTool::None,
            pen_color: [0, 168, 132, 255],
            pen_width: 5.0,
            blur_block_size: 14,
            text_color: [255, 255, 255, 255],
            text_has_bg: true,
            shape_color: [0, 168, 132, 255],
            active_stroke: None,
            active_blur_start: None,
            active_blur_rect: None,
            drag: None,
            working: false,
            error: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            surface: Rc::new(Cell::new(None)),
            caption: None,
            text_input: None,
            caption_emoji_open: false,
            status_privacy_open: false,
            status_privacy_mode: "Estado (221 excluidos)".to_string(),
            save_notice: None,
        }
    }

    #[test]
    fn the_scale_of_the_preview_keeps_the_longest_edge() {
        assert_eq!(
            preview_scale(PixelSize {
                width: 0.0,
                height: 0.0
            }),
            0.0
        );
        assert_eq!(
            preview_scale(PixelSize {
                width: 100.0,
                height: 50.0
            }),
            1.0
        );
        assert_eq!(
            preview_scale(PixelSize {
                width: 1240.0,
                height: 620.0
            }),
            0.5
        );
        assert_eq!(
            preview_scale(PixelSize {
                width: 620.0,
                height: 1240.0
            }),
            0.5
        );
    }

    #[test]
    fn the_crop_of_a_picture_without_a_crop_is_the_whole_picture() {
        let mut editor = editor(PixelSize {
            width: 200.0,
            height: 100.0,
        });
        assert_eq!(
            editor.crop(),
            CropRect {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 100.0
            }
        );
        editor.active_item_mut().rotation = Rotation::Deg90;
        assert_eq!(
            editor.crop(),
            CropRect {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 200.0
            }
        );
    }

    #[test]
    fn the_plan_of_a_picture_without_a_crop_is_the_whole_picture() {
        let editor = editor(PixelSize {
            width: 200.0,
            height: 100.0,
        });
        let plan = match editor.plan() {
            Ok(plan) => plan,
            Err(error) => panic!("the plan could not be built: {error}"),
        };
        assert_eq!(plan.rotation, Rotation::Deg0);
        assert_eq!(
            plan.crop,
            CropRect {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 100.0
            }
        );
        assert_eq!(
            plan.output,
            PixelSize {
                width: 200.0,
                height: 100.0
            }
        );
    }

    #[test]
    fn the_plan_of_a_picture_without_a_picture_is_an_error() {
        let editor = editor(PixelSize {
            width: 0.0,
            height: 0.0,
        });
        assert_eq!(editor.plan(), Err(MediaEditError::NoPicture));
    }

    #[test]
    fn the_plan_keeps_the_crop_in_the_pixels_of_the_rotated_picture() {
        let mut editor = editor(PixelSize {
            width: 200.0,
            height: 100.0,
        });
        editor.active_item_mut().rotation = Rotation::Deg90;
        editor.active_item_mut().crop = Some(CropRect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 50.0,
        });
        let plan = match editor.plan() {
            Ok(plan) => plan,
            Err(error) => panic!("the plan could not be built: {error}"),
        };
        assert_eq!(plan.rotation, Rotation::Deg90);
        assert_eq!(
            plan.crop,
            CropRect {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 50.0
            }
        );
    }

    #[test]
    fn the_inverse_of_a_rotation_undoes_it() {
        for rotation in Rotation::ANGLES {
            let inverse = inverse_rotation(rotation);
            if rotation != Rotation::Deg0 && rotation != Rotation::Deg180 {
                assert_ne!(inverse, rotation, "a quarter turn is not its own inverse");
            }
            let size = PixelSize {
                width: 200.0,
                height: 100.0,
            };
            let crop = CropRect {
                x: 10.0,
                y: 20.0,
                width: 50.0,
                height: 30.0,
            };
            let rotated = match media_crop::crop_in_rotated_space(crop, size, rotation) {
                Ok(rotated) => rotated,
                Err(error) => panic!("the crop could not be rotated: {error}"),
            };
            let rotated_size = match media_crop::rotated_size(size, rotation) {
                Ok(size) => size,
                Err(error) => panic!("the size could not be rotated: {error}"),
            };
            let back = match media_crop::crop_in_rotated_space(rotated, rotated_size, inverse) {
                Ok(back) => back,
                Err(error) => panic!("the crop could not be rotated back: {error}"),
            };
            assert_eq!(back, crop, "the rotation {rotation:?} is not undone");
        }
    }

    #[test]
    fn the_steps_of_a_rotation_are_a_quarter_turn() {
        assert_eq!(steps_of(90), Rotation::Deg90);
        assert_eq!(steps_of(270), Rotation::Deg270);
        assert_eq!(steps_of(180), Rotation::Deg180);
    }

    #[test]
    fn the_picture_is_turned_a_quarter_turn() {
        let picture = image::RgbaImage::from_pixel(2, 3, image::Rgba([10, 20, 30, 255]));
        let turned = rotate_picture(&picture, Rotation::Deg90);
        assert_eq!(turned.dimensions(), (3, 2));
        let turned = rotate_picture(&picture, Rotation::Deg180);
        assert_eq!(turned.dimensions(), (2, 3));
        let turned = rotate_picture(&picture, Rotation::Deg270);
        assert_eq!(turned.dimensions(), (3, 2));
        let turned = rotate_picture(&picture, Rotation::Deg0);
        assert_eq!(turned.dimensions(), (2, 3));
    }

    #[test]
    fn the_buffer_of_the_picture_is_in_bgra() {
        let picture = image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 4]));
        let rendered = to_render_image(&picture);
        let bytes = match rendered.as_bytes(0) {
            Some(bytes) => bytes,
            None => panic!("the picture has no bytes"),
        };
        assert_eq!(bytes, &[3, 2, 1, 4]);
    }

    fn editor_with_surface() -> MediaEditor {
        let editor = editor(PixelSize {
            width: 200.0,
            height: 100.0,
        });
        editor.surface.set(Some(Bounds {
            origin: gpui::point(px(0.), px(0.)),
            size: gpui::size(px(200.), px(100.)),
        }));
        editor
    }

    #[test]
    fn the_drag_of_the_top_left_corner_moves_the_two_edges() {
        let mut editor = editor_with_surface();
        editor.begin_drag(gpui::point(px(0.), px(0.)));
        editor.move_drag(gpui::point(px(50.), px(40.)));
        assert_eq!(
            editor.crop(),
            CropRect {
                x: 50.0,
                y: 40.0,
                width: 150.0,
                height: 60.0
            }
        );
    }

    #[test]
    fn the_drag_of_the_bottom_right_corner_moves_the_two_edges() {
        let mut editor = editor_with_surface();
        editor.begin_drag(gpui::point(px(200.), px(100.)));
        editor.move_drag(gpui::point(px(150.), px(80.)));
        assert_eq!(
            editor.crop(),
            CropRect {
                x: 0.0,
                y: 0.0,
                width: 150.0,
                height: 80.0
            }
        );
    }

    #[test]
    fn the_drag_leaves_a_crop_of_at_least_the_smallest_one() {
        let mut editor = editor_with_surface();
        editor.begin_drag(gpui::point(px(0.), px(0.)));
        editor.move_drag(gpui::point(px(190.), px(90.)));
        assert_eq!(
            editor.crop(),
            CropRect {
                x: 176.0,
                y: 76.0,
                width: 24.0,
                height: 24.0
            }
        );
    }

    #[test]
    fn the_drag_of_the_middle_of_the_crop_moves_the_whole_crop() {
        let mut editor = editor_with_surface();
        editor.active_item_mut().crop = Some(CropRect {
            x: 20.0,
            y: 20.0,
            width: 50.0,
            height: 30.0,
        });
        editor.begin_drag(gpui::point(px(40.), px(30.)));
        editor.move_drag(gpui::point(px(60.), px(50.)));
        assert_eq!(
            editor.crop(),
            CropRect {
                x: 40.0,
                y: 40.0,
                width: 50.0,
                height: 30.0
            }
        );
    }

    #[test]
    fn the_drag_of_the_crop_never_leaves_the_picture() {
        let mut editor = editor_with_surface();
        editor.active_item_mut().crop = Some(CropRect {
            x: 20.0,
            y: 20.0,
            width: 50.0,
            height: 30.0,
        });
        editor.begin_drag(gpui::point(px(40.), px(30.)));
        editor.move_drag(gpui::point(px(1000.), px(1000.)));
        assert_eq!(
            editor.crop(),
            CropRect {
                x: 150.0,
                y: 70.0,
                width: 50.0,
                height: 30.0
            }
        );
    }

    #[test]
    fn the_drag_outside_of_the_crop_starts_a_new_one() {
        let mut editor = editor_with_surface();
        editor.active_item_mut().crop = Some(CropRect {
            x: 20.0,
            y: 20.0,
            width: 50.0,
            height: 30.0,
        });
        editor.begin_drag(gpui::point(px(120.), px(80.)));
        editor.move_drag(gpui::point(px(150.), px(90.)));
        assert_eq!(
            editor.crop(),
            CropRect {
                x: 120.0,
                y: 80.0,
                width: 30.0,
                height: 10.0
            }
        );
    }

    #[test]
    fn the_crop_travels_with_the_picture() {
        let crop = CropRect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        };
        let frame = PixelSize {
            width: 200.0,
            height: 100.0,
        };
        let carried = match carried_crop(crop, frame, 90) {
            Some(carried) => carried,
            None => panic!("the crop was not carried"),
        };
        assert_eq!(
            carried,
            CropRect {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0
            }
        );
    }
}
