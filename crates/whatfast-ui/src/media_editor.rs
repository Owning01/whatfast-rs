//! The image editor: a free crop with handles and a quarter turn rotation.
//!
//! port: taken from the origin's `tauri/src/components/MediaEditor.tsx`. The
//! geometry of the crop is not here: it comes from
//! `whatfast_core::media_crop`, the port of `tauri/src/lib/media-crop.ts`.
//!
//! The picture is shown already rotated and the rectangle of the crop lives
//! in the pixels of the rotated picture, which is exactly the space the core
//! crops in.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Bounds, Context, EventEmitter, FontWeight, ImageSource, InteractiveElement as _, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, ParentElement as _,
    Pixels, Point, Render, Styled as _, StyledImage as _, Window, canvas, div, img, px, rgba,
};
use gpui_component::{
    ActiveTheme as _, Sizable as _, button::{Button, ButtonVariants as _}, h_flex, v_flex,
};

use whatfast_core::media_crop::{self, CropRect, EditPlan, PixelSize, Rotation};

use crate::icons::WhatfastIcon;

/// The longest edge of the preview, in pixels.
pub(crate) const PREVIEW_EDGE: f64 = 460.0;

/// The smallest crop the user can leave behind, in preview pixels.
pub(crate) const MIN_CROP_EDGE: f64 = 24.0;

/// The radius of the area that grabs a handle, in preview pixels.
const HANDLE_RADIUS: f64 = 10.0;

/// The side of the square that draws a handle, in preview pixels.
const HANDLE_EDGE: f64 = 12.0;

/// The handle the user is dragging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DragMode {
    /// A new crop, drawn from the point of the press.
    Create,
    /// The whole rectangle travels with the mouse.
    Move,
    /// The top left corner.
    NorthWest,
    /// The top edge.
    North,
    /// The top right corner.
    NorthEast,
    /// The right edge.
    East,
    /// The bottom right corner.
    SouthEast,
    /// The bottom edge.
    South,
    /// The bottom left corner.
    SouthWest,
    /// The left edge.
    West,
}

/// The state of one drag.
#[derive(Debug, Clone, Copy)]
struct DragState {
    mode: DragMode,
    /// The point of the press, in the pixels of the rotated picture.
    origin_x: f64,
    origin_y: f64,
    /// The crop when the drag started.
    start: CropRect,
}

/// The edit that the user applied.
#[derive(Debug, Clone)]
pub(crate) struct MediaEdit {
    /// The picture that was edited.
    pub(crate) path: PathBuf,
    /// The rotation and the crop. The crop is in the pixels of the rotated
    /// picture: that is the space the core crops in.
    pub(crate) plan: EditPlan,
}

/// The events of the editor.
#[derive(Debug, Clone)]
pub(crate) enum MediaEditorEvent {
    /// The user applied the edit.
    Applied(MediaEdit),
    /// The user closed the editor without applying anything.
    Cancelled,
}

/// The reason why the editor could not build the plan of the edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MediaEditError {
    /// The picture is not loaded.
    NoPicture,
    /// The geometry of the picture is not valid.
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

/// The editor of one picture.
pub(crate) struct MediaEditor {
    path: PathBuf,
    file_name: String,
    /// The size of the picture, in pixels.
    natural: PixelSize,
    /// The rotation of the picture.
    rotation: Rotation,
    /// The crop, in the pixels of the rotated picture. `None` is the whole
    /// picture.
    crop: Option<CropRect>,
    /// The handle that is being dragged.
    drag: Option<DragState>,
    /// Whether the editor is busy.
    working: bool,
    /// The error of the last load.
    error: Option<String>,
    /// The picture, already reduced to the size of the preview.
    preview: Option<image::RgbaImage>,
    /// The preview, already rotated, ready to be painted.
    rendered: Option<Arc<gpui::RenderImage>>,
    /// The rotation of `rendered`.
    rendered_rotation: Option<Rotation>,
    /// The bounds of the surface of the preview, in the coordinates of the
    /// window. The canvas of the surface fills them on every frame, so the
    /// mouse can be turned into a point of the picture.
    surface: Rc<Cell<Option<Bounds<Pixels>>>>,
}

impl EventEmitter<MediaEditorEvent> for MediaEditor {}

impl MediaEditor {
    /// Builds the editor of the picture and starts to load it.
    pub(crate) fn new(path: &Path, cx: &mut Context<Self>) -> Self {
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "photo".to_string());
        let editor = Self {
            path: path.to_path_buf(),
            file_name,
            natural: PixelSize {
                width: 0.0,
                height: 0.0,
            },
            rotation: Rotation::Deg0,
            crop: None,
            drag: None,
            working: false,
            error: None,
            preview: None,
            rendered: None,
            rendered_rotation: None,
            surface: Rc::new(Cell::new(None)),
        };
        let mut editor = editor;
        editor.load(cx);
        editor
    }

    /// The picture that is being edited.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// The name of the picture, the way the user knows it.
    pub(crate) fn file_name(&self) -> &str {
        &self.file_name
    }

    /// Whether the editor is busy.
    pub(crate) fn is_working(&self) -> bool {
        self.working
    }

    /// The rotation of the picture.
    pub(crate) fn rotation(&self) -> Rotation {
        self.rotation
    }

    /// The crop, in the pixels of the rotated picture.
    pub(crate) fn crop(&self) -> CropRect {
        self.crop_rect()
    }

    /// Turns the picture a quarter turn to the left.
    pub(crate) fn rotate_left(&mut self, cx: &mut Context<Self>) {
        self.rotate(270, cx);
    }

    /// Turns the picture a quarter turn to the right.
    pub(crate) fn rotate_right(&mut self, cx: &mut Context<Self>) {
        self.rotate(90, cx);
    }

    /// Drops the crop and goes back to the whole picture.
    pub(crate) fn reset_crop(&mut self, cx: &mut Context<Self>) {
        self.crop = None;
        cx.notify();
    }

    /// Closes the editor without applying anything.
    pub(crate) fn cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(MediaEditorEvent::Cancelled);
    }

    /// Applies the edit and hands the plan to the one that opened the editor.
    pub(crate) fn apply(&mut self, cx: &mut Context<Self>) {
        if self.working {
            return;
        }
        match self.plan() {
            Ok(plan) => {
                let edit = MediaEdit {
                    path: self.path.clone(),
                    plan,
                };
                cx.emit(MediaEditorEvent::Applied(edit));
            }
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
            }
        }
    }

    /// The plan of the edit: the rotation and the crop, in the pixels of the
    /// rotated picture.
    pub(crate) fn plan(&self) -> Result<EditPlan, MediaEditError> {
        let natural = self.natural;
        if natural.width <= 0.0 || natural.height <= 0.0 {
            return Err(MediaEditError::NoPicture);
        }
        let rotated = media_crop::rotated_size(natural, self.rotation)
            .map_err(|error| MediaEditError::Geometry(error.message))?;
        // The editor keeps the crop in the pixels of the rotated picture, the
        // space the core crops in, and `plan_edit` takes the crop in the
        // pixels of the original picture: the rectangle travels back through
        // the inverse of the rotation.
        let source_crop = media_crop::crop_in_rotated_space(
            self.crop_rect(),
            rotated,
            inverse_rotation(self.rotation),
        )
        .map_err(|error| MediaEditError::Geometry(error.message))?;
        media_crop::plan_edit(natural, source_crop, self.rotation)
            .map_err(|error| MediaEditError::Geometry(error.message))
    }

    /// The drawing of the editor.
    pub(crate) fn render(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        self.draw(cx)
    }

    /// Loads the picture in the background.
    fn load(&mut self, cx: &mut Context<Self>) {
        self.working = true;
        self.error = None;
        let path = self.path.clone();
        cx.spawn(async move |this, cx| {
            let loaded = smol::unblock(move || load_picture(&path)).await;
            this.update(cx, |this, cx| {
                this.working = false;
                match loaded {
                    Ok(loaded) => {
                        this.natural = loaded.natural;
                        this.preview = Some(loaded.preview);
                        this.rendered = None;
                        this.rendered_rotation = None;
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Turns the picture a quarter turn and carries the crop with it.
    fn rotate(&mut self, steps: u32, cx: &mut Context<Self>) {
        let current = self.rotation;
        let next = media_crop::parse_rotation(current.degrees() as f64 + steps as f64);
        if let Some(crop) = self.crop {
            let frame = self.rotated_size();
            // The crop travels with the picture: a quarter turn maps the
            // rectangle of the current frame onto the rectangle of the next
            // one.
            self.crop = media_crop::crop_in_rotated_space(crop, frame, steps_of(steps))
                .ok();
        }
        self.rotation = next;
        cx.notify();
    }

    /// The size of the picture after the rotation.
    fn rotated_size(&self) -> PixelSize {
        match media_crop::rotated_size(self.natural, self.rotation) {
            Ok(size) => size,
            Err(_) => PixelSize {
                width: 0.0,
                height: 0.0,
            },
        }
    }

    /// The size of the preview of the picture, in the pixels of the window.
    fn drawn_size(&self) -> PixelSize {
        let scale = preview_scale(self.natural);
        PixelSize {
            width: (self.natural.width * scale).round(),
            height: (self.natural.height * scale).round(),
        }
    }

    /// How many pixels of the window are one pixel of the picture.
    fn window_per_picture(&self) -> (f64, f64) {
        let drawn = self.drawn_size();
        (
            if drawn.width > 0.0 {
                drawn.width / self.natural.width
            } else {
                0.0
            },
            if drawn.height > 0.0 {
                drawn.height / self.natural.height
            } else {
                0.0
            },
        )
    }

    /// The crop in the pixels of the rotated picture.
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

    /// The smallest crop the user can leave behind, in the pixels of the
    /// rotated picture.
    fn min_edge(&self) -> f64 {
        let (scale_x, scale_y) = self.window_per_picture();
        MIN_CROP_EDGE * (scale_x + scale_y) / 2.0
    }

    /// The radius of the area that grabs a handle, in the pixels of the
    /// rotated picture.
    fn handle_radius(&self) -> f64 {
        let (scale_x, scale_y) = self.window_per_picture();
        HANDLE_RADIUS * (scale_x + scale_y) / 2.0
    }

    /// The point of the mouse, in the pixels of the rotated picture.
    fn picture_point(&self, position: Point<Pixels>) -> Option<(f64, f64)> {
        let bounds = self.surface.get()?;
        let (scale_x, scale_y) = self.window_per_picture();
        if scale_x <= 0.0 || scale_y <= 0.0 {
            return None;
        }
        Some((
            (position.x - bounds.origin.x).as_f32() as f64 / scale_x,
            (position.y - bounds.origin.y).as_f32() as f64 / scale_y,
        ))
    }

    /// Starts a drag: the handle under the mouse, or a new crop.
    fn begin_drag(&mut self, position: Point<Pixels>) {
        let Some((x, y)) = self.picture_point(position) else {
            return;
        };
        let crop = self.crop_rect();
        let mode = self.handle_under(x, y, crop);
        self.drag = Some(DragState {
            mode,
            origin_x: x,
            origin_y: y,
            start: crop,
        });
    }

    /// Moves the handle that is being dragged.
    fn move_drag(&mut self, position: Point<Pixels>) {
        let Some(drag) = self.drag else {
            return;
        };
        let Some((x, y)) = self.picture_point(position) else {
            return;
        };
        let frame = self.rotated_size();
        let min_edge = self.min_edge();
        let start = drag.start;
        // port: the origin keeps the point of the mouse inside the picture.
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
                let x = (start.x + (point_x - drag.origin_x)).clamp(0.0, (frame.width - width).max(0.0));
                let y =
                    (start.y + (point_y - drag.origin_y)).clamp(0.0, (frame.height - height).max(0.0));
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
                CropRect {
                    x,
                    y,
                    width,
                    height,
                }
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
                CropRect {
                    x,
                    y: start.y,
                    width,
                    height,
                }
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
        self.crop = Some(crop);
    }

    /// Ends the drag and leaves a crop that fits the picture.
    fn end_drag(&mut self) {
        if self.drag.take().is_none() {
            return;
        }
        let frame = self.rotated_size();
        let min_edge = self.min_edge();
        let current = self.crop_rect();
        let width = current.width.max(min_edge).min(frame.width);
        let height = current.height.max(min_edge).min(frame.height);
        self.crop = Some(CropRect {
            x: current.x.min(frame.width - width),
            y: current.y.min(frame.height - height),
            width,
            height,
        });
    }

    /// The handle that is under the point, if there is one.
    fn handle_under(&self, x: f64, y: f64, crop: CropRect) -> DragMode {
        let radius = self.handle_radius();
        let right = crop.x + crop.width;
        let bottom = crop.y + crop.height;
        let middle_x = crop.x + crop.width / 2.0;
        let middle_y = crop.y + crop.height / 2.0;
        let near = |value: f64, target: f64| (value - target).abs() <= radius;
        if near(x, crop.x) && near(y, crop.y) {
            return DragMode::NorthWest;
        }
        if near(x, right) && near(y, crop.y) {
            return DragMode::NorthEast;
        }
        if near(x, crop.x) && near(y, bottom) {
            return DragMode::SouthWest;
        }
        if near(x, right) && near(y, bottom) {
            return DragMode::SouthEast;
        }
        if near(x, middle_x) && near(y, crop.y) {
            return DragMode::North;
        }
        if near(x, middle_x) && near(y, bottom) {
            return DragMode::South;
        }
        if near(x, crop.x) && near(y, middle_y) {
            return DragMode::West;
        }
        if near(x, right) && near(y, middle_y) {
            return DragMode::East;
        }
        if x >= crop.x && x <= right && y >= crop.y && y <= bottom {
            return DragMode::Move;
        }
        DragMode::Create
    }

    /// The preview, already rotated, ready to be painted.
    fn rendered_preview(&mut self) -> Option<Arc<gpui::RenderImage>> {
        if self.rendered_rotation == Some(self.rotation) {
            return self.rendered.clone();
        }
        let preview = self.preview.as_ref()?;
        let rotated = rotate_picture(preview, self.rotation);
        let rendered = Arc::new(to_render_image(&rotated));
        self.rendered = Some(rendered.clone());
        self.rendered_rotation = Some(self.rotation);
        Some(rendered)
    }

    /// The drawing of the dialog.
    fn draw(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let background = cx.theme().background;
        let border = cx.theme().border;
        let foreground = cx.theme().foreground;
        let muted = cx.theme().muted_foreground;
        let file_name = self.file_name.clone();
        let working = self.working;
        let error = self.error.clone();

        div()
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x00000066))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.end_drag();
                    cx.notify();
                }),
            )
            .child(
                v_flex()
                    .w(px(520.))
                    .gap_3()
                    .p_4()
                    .rounded_lg()
                    .bg(background)
                    .border_1()
                    .border_color(border)
                    .text_color(foreground)
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .child("Edit photo"),
                            )
                            .child(
                                Button::new("media-editor-close")
                                    .ghost()
                                    .small()
                                    .icon(WhatfastIcon::Close)
                                    .tooltip("Cancel")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.cancel(cx);
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .truncate()
                            .child(file_name.clone()),
                    )
                    .child(self.surface(cx))
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child("Drag on the picture to crop it. The preview shows exactly what will be sent."),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("media-editor-rotate-left")
                                    .label("Rotate left")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.rotate_left(cx);
                                    })),
                            )
                            .child(
                                Button::new("media-editor-rotate-right")
                                    .label("Rotate right")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.rotate_right(cx);
                                    })),
                            )
                            .child(
                                Button::new("media-editor-reset")
                                    .label("Reset crop")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.reset_crop(cx);
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .justify_end()
                            .child(
                                Button::new("media-editor-cancel")
                                    .label("Cancel")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.cancel(cx);
                                    })),
                            )
                            .child(
                                Button::new("media-editor-send")
                                    .label(if working { "Preparing." } else { "Send" })
                                    .loading(working)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if !this.working {
                                            this.apply(cx);
                                        }
                                    })),
                            ),
                    )
                    .when(error.is_some(), |dialog| {
                        dialog.child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().danger)
                                .child(error.clone().unwrap_or_default()),
                        )
                    }),
            )
    }

    /// The surface of the preview: the picture, already rotated, and the
    /// rectangle of the crop.
    fn surface(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let frame = self.rotated_size();
        let (scale_x, scale_y) = self.window_per_picture();
        let crop = self.crop_rect();
        let primary = cx.theme().primary;
        let surface = Rc::clone(&self.surface);
        let rendered = self.rendered_preview();
        let working = self.working;
        let error = self.error.is_some();

        let mut handles = Vec::new();
        let edge = HANDLE_EDGE * (scale_x + scale_y) / 2.0;
        for (x, y) in [
            (crop.x, crop.y),
            (crop.x + crop.width, crop.y),
            (crop.x, crop.y + crop.height),
            (crop.x + crop.width, crop.y + crop.height),
        ] {
            handles.push(
                div()
                    .absolute()
                    .left(px((x * scale_x - edge / 2.0) as f32))
                    .top(px((y * scale_y - edge / 2.0) as f32))
                    .size(px(edge as f32))
                    .rounded_sm()
                    .bg(primary)
                    .into_any_element(),
            );
        }

        div()
            .relative()
            .w(px(frame.width as f32))
            .h(px(frame.height as f32))
            .overflow_hidden()
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
            .child(
                canvas(
                    move |bounds, _window, _cx| {
                        surface.set(Some(bounds));
                    },
                    |_bounds, _value, _window, _cx| {},
                )
                .size_full(),
            )
            .child(match rendered {
                Some(image) => img(ImageSource::Render(image))
                    .size_full()
                    .object_fit(ObjectFit::Fill)
                    .into_any_element(),
                None => div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .child(if error {
                        "The photo could not be edited"
                    } else if working {
                        "Preparing."
                    } else {
                        ""
                    })
                    .into_any_element(),
            })
            .child(
                div()
                    .absolute()
                    .left(px((crop.x * scale_x) as f32))
                    .top(px((crop.y * scale_y) as f32))
                    .w(px((crop.width * scale_x) as f32))
                    .h(px((crop.height * scale_y) as f32))
                    .border_2()
                    .border_color(primary),
            )
            .children(handles)
    }
}

impl Render for MediaEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.draw(cx)
    }
}

/// The scale of the preview: the picture is reduced until its longest edge is
/// `PREVIEW_EDGE` pixels.
fn preview_scale(natural: PixelSize) -> f64 {
    if natural.width <= 0.0 || natural.height <= 0.0 {
        return 0.0;
    }
    let longest = natural.width.max(natural.height);
    (PREVIEW_EDGE / longest).min(1.0)
}

/// The left edge of the crop, moved to the point of the mouse.
///
/// The edge never gets closer to the right edge than the smallest crop.
fn left_edge(right: f64, point_x: f64, min_edge: f64) -> (f64, f64) {
    let left = point_x.min(right - min_edge);
    (left, right - left)
}

/// The right edge of the crop, moved to the point of the mouse.
fn right_edge(start: CropRect, point_x: f64, min_edge: f64, frame_width: f64) -> f64 {
    (point_x - start.x).max(min_edge).min(frame_width - start.x)
}

/// The top edge of the crop, moved to the point of the mouse.
fn top_edge(bottom: f64, point_y: f64, min_edge: f64) -> (f64, f64) {
    let top = point_y.min(bottom - min_edge);
    (top, bottom - top)
}

/// The bottom edge of the crop, moved to the point of the mouse.
fn bottom_edge(start: CropRect, point_y: f64, min_edge: f64, frame_height: f64) -> f64 {
    (point_y - start.y).max(min_edge).min(frame_height - start.y)
}

/// The quarter turn that undoes a rotation.
fn inverse_rotation(rotation: Rotation) -> Rotation {
    match rotation {
        Rotation::Deg0 => Rotation::Deg0,
        Rotation::Deg90 => Rotation::Deg270,
        Rotation::Deg180 => Rotation::Deg180,
        Rotation::Deg270 => Rotation::Deg90,
    }
}

/// The quarter turn of a number of degrees.
fn steps_of(steps: u32) -> Rotation {
    media_crop::parse_rotation(steps as f64)
}

/// The picture, loaded and reduced to the size of the preview.
struct LoadedPicture {
    natural: PixelSize,
    preview: image::RgbaImage,
}

/// Reads the picture and reduces it to the size of the preview.
fn load_picture(path: &Path) -> Result<LoadedPicture, String> {
    let decoded = image::ImageReader::open(path)
        .map_err(|error| format!("the photo could not be read: {error}"))?
        .with_guessed_format()
        .map_err(|error| format!("the photo could not be read: {error}"))?
        .decode()
        .map_err(|error| format!("the photo could not be decoded: {error}"))?;
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

/// The picture, turned a quarter turn clockwise.
fn rotate_picture(picture: &image::RgbaImage, rotation: Rotation) -> image::RgbaImage {
    match rotation {
        Rotation::Deg0 => picture.clone(),
        Rotation::Deg90 => image::imageops::rotate90(picture),
        Rotation::Deg180 => image::imageops::rotate180(picture),
        Rotation::Deg270 => image::imageops::rotate270(picture),
    }
}

/// The picture, as the buffer that GPUI paints.
fn to_render_image(picture: &image::RgbaImage) -> gpui::RenderImage {
    let (width, height) = picture.dimensions();
    let mut data = picture.as_raw().clone();
    // port: the renderer of GPUI expects the pixels in BGRA.
    for pixel in data.chunks_exact_mut(4) {
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
        MediaEditor {
            path: PathBuf::from("photo.png"),
            file_name: "photo.png".to_string(),
            natural,
            rotation: Rotation::Deg0,
            crop: None,
            drag: None,
            working: false,
            error: None,
            preview: None,
            rendered: None,
            rendered_rotation: None,
            surface: Rc::new(Cell::new(None)),
        }
    }

    #[test]
    fn the_scale_of_the_preview_keeps_the_longest_edge() {
        assert_eq!(preview_scale(PixelSize { width: 0.0, height: 0.0 }), 0.0);
        assert_eq!(
            preview_scale(PixelSize {
                width: 100.0,
                height: 50.0
            }),
            1.0
        );
        assert_eq!(
            preview_scale(PixelSize {
                width: 920.0,
                height: 460.0
            }),
            0.5
        );
        assert_eq!(
            preview_scale(PixelSize {
                width: 460.0,
                height: 920.0
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
            editor.crop_rect(),
            CropRect {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 100.0
            }
        );
        editor.rotation = Rotation::Deg90;
        assert_eq!(
            editor.crop_rect(),
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
        // A quarter turn: the picture is 100x200 and the crop is the top
        // half of it.
        editor.rotation = Rotation::Deg90;
        editor.crop = Some(CropRect {
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
            assert_ne!(inverse, rotation, "a quarter turn is not its own inverse");
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
}
