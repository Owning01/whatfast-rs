//! The tray of the pictures the user stages before sending them.
//!
//! port: the reference (WhatsApp Web) does not send a picture the moment it
//! arrives: the pasted or picked pictures wait over the field of the message
//! as thumbnails, with the button that adds more and the cross that drops
//! one, and they go out when the user presses send. The native client used to
//! send a pasted picture right away; this module is that missing step.
//!
//! The tray is state here and pixels in `crate::render` and `crate::app`: the
//! module owns both halves, the same way `crate::attachments` owns the card
//! of a file that comes with a message.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Context, Hsla, ImageSource, InteractiveElement as _, IntoElement, ObjectFit,
    ParentElement as _, Pixels, StatefulInteractiveElement as _, Styled as _, StyledImage as _,
    div, img, px, rgb, rgba,
};
use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex};
use whatfast_i18n as i18n;

use crate::app::Whatfast;
use crate::icons::WhatfastIcon;
use crate::media_editor::to_render_image;
use crate::text::validate_image_message;

/// How many pictures the tray holds at most.
///
/// port: the compose box of the reference accepts up to thirty pictures; the
/// native client keeps a smaller number, because every picture holds a
/// thumbnail in memory.
pub(crate) const MAX_PENDING_IMAGES: usize = 8;

/// The side of one thumbnail, before the scale.
const THUMB: f32 = 62.;
/// The gap between two thumbnails, before the scale.
const THUMB_GAP: f32 = 8.;
/// The padding of the tray, before the scale.
const TRAY_PADDING: f32 = 8.;
/// The radius of a thumbnail, before the scale.
const THUMB_RADIUS: f32 = 8.;
/// The side of the badge of a thumbnail, before the scale.
const BADGE: f32 = 20.;
/// The edge of the buffer of a thumbnail: twice its box, so the picture stays
/// sharp on the screens of double density.
const THUMB_BUFFER: u32 = 124;

/// The green of the reference: the border of the picture that is selected and
/// the colour of the interface that the rest of the app already uses.
const GREEN: u32 = 0x25d366;

/// One picture that is waiting for the send button.
#[derive(Clone)]
pub(crate) struct PendingImage {
    /// The number that names the thumbnail in the tree of the interface.
    pub(crate) id: u64,
    /// The file that goes to the backend when the user sends it.
    pub(crate) path: PathBuf,
    /// The picture, already squared and reduced to the size of the box.
    ///
    /// It is `None` when the decoder could not read the file: the thumbnail
    /// then shows the icon of a file and the picture still may be sent.
    pub(crate) thumbnail: Option<Arc<gpui::RenderImage>>,
}

/// The pictures that wait for the send button, in the order the user picked
/// them.
#[derive(Default)]
pub(crate) struct PendingImages {
    images: Vec<PendingImage>,
    /// The picture whose options are shown: the one with the green border.
    selected: usize,
    next_id: u64,
}

impl PendingImages {
    /// The pictures of the tray, in the order the user picked them.
    pub(crate) fn images(&self) -> &[PendingImage] {
        &self.images
    }

    /// One picture of the tray.
    pub(crate) fn get(&self, index: usize) -> Option<&PendingImage> {
        self.images.get(index)
    }

    /// How many pictures the tray holds.
    pub(crate) fn len(&self) -> usize {
        self.images.len()
    }

    /// Whether the tray is empty, which is when the composer sends its text.
    pub(crate) fn is_empty(&self) -> bool {
        self.images.is_empty()
    }

    /// Whether one more picture fits.
    pub(crate) fn can_add_more(&self) -> bool {
        self.images.len() < MAX_PENDING_IMAGES
    }

    /// The picture whose options are shown.
    pub(crate) fn selected_index(&self) -> usize {
        self.selected
    }

    /// Puts the options of one picture in front.
    pub(crate) fn select(&mut self, index: usize) {
        if index < self.images.len() {
            self.selected = index;
        }
    }

    /// Adds one picture, after checking that it may be sent.
    ///
    /// port: the checks are the ones of the send button of the reference: a
    /// picture that is too big or has a format that WhatsApp does not take
    /// never reaches the tray.
    pub(crate) fn stage(&mut self, path: PathBuf) -> Result<(), String> {
        if !self.can_add_more() {
            return Err(format!(
                "You can send up to {MAX_PENDING_IMAGES} photos at once"
            ));
        }
        // The caption is checked again when the pictures go out: here the
        // picture is the only thing that matters.
        validate_image_message(&path, "")?;
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.images.push(pending_image(path, id));
        self.selected = self.images.len() - 1;
        Ok(())
    }

    /// Replaces the picture of one file with the one the editor wrote.
    ///
    /// Returns `false` when the file was not in the tray, so the caller can
    /// stage the edited picture as a new one.
    pub(crate) fn replace(&mut self, source: &Path, path: PathBuf) -> bool {
        let Some(index) = self
            .images
            .iter()
            .position(|image| image.path.as_path() == source)
        else {
            return false;
        };
        let id = self.images[index].id;
        self.images[index] = pending_image(path, id);
        self.selected = index;
        true
    }

    /// Drops one picture of the tray.
    pub(crate) fn remove(&mut self, index: usize) {
        if index >= self.images.len() {
            return;
        }
        self.images.remove(index);
        if self.selected > index {
            self.selected -= 1;
        }
        if self.selected >= self.images.len() {
            self.selected = self.images.len().saturating_sub(1);
        }
    }

    /// Empties the tray and returns what it held, for the send.
    pub(crate) fn take_all(&mut self) -> Vec<PendingImage> {
        self.selected = 0;
        std::mem::take(&mut self.images)
    }

    /// Empties the tray.
    pub(crate) fn clear(&mut self) {
        self.selected = 0;
        self.images.clear();
    }

    /// The height the tray adds to the footer, for the boxes that float over
    /// the composer and have to clear it.
    pub(crate) fn tray_height(&self, scale: f32) -> Pixels {
        if self.images.is_empty() {
            return px(0.);
        }
        // The border of the top of the tray is the extra pixel.
        px((THUMB + TRAY_PADDING * 2.) * scale + 1.)
    }
}

/// One picture, ready for the tray, with its thumbnail.
fn pending_image(path: PathBuf, id: u64) -> PendingImage {
    let thumbnail = load_thumbnail(&path);
    PendingImage {
        id,
        path,
        thumbnail,
    }
}

/// The picture of one file, squared and reduced to the size of the box.
///
/// A picture that cannot be decoded leaves the thumbnail empty, and the tray
/// paints the icon of a file instead: the file may still be a picture that
/// WhatsApp takes.
fn load_thumbnail(path: &Path) -> Option<Arc<gpui::RenderImage>> {
    let decoded = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?
        .into_rgba8();
    let (width, height) = decoded.dimensions();
    let side = width.min(height);
    if side == 0 {
        return None;
    }
    // The square of the middle: the box of the thumbnail paints the whole
    // picture, so the sides are cut instead of leaving an empty band.
    let square = image::imageops::crop_imm(
        &decoded,
        (width - side) / 2,
        (height - side) / 2,
        side,
        side,
    )
    .to_image();
    let reduced = image::imageops::resize(
        &square,
        THUMB_BUFFER,
        THUMB_BUFFER,
        image::imageops::FilterType::Triangle,
    );
    Some(Arc::new(to_render_image(&reduced)))
}

/// The number of the pictures of a send, as the origin writes it on the
/// button of the send: `Send`, `Send 2 photos`.
pub(crate) fn send_label(count: usize) -> String {
    match count {
        0 | 1 => i18n::t("Send").to_string(),
        count => i18n::t_args("Send {count} photos", &[("count", &count.to_string())]),
    }
}

impl Whatfast {
    /// The tray of the pictures that wait for the send button.
    ///
    /// It sits between the list of the messages and the field of the message,
    /// so the field never covers it, and it grows the footer instead of
    /// floating over the conversation.
    pub(crate) fn render_pending_images(&self, cx: &mut Context<Self>) -> gpui::Div {
        let scale = self.ui_scale;
        // The colours of the theme are copied before the listeners borrow the
        // window: the theme lives in the context the listeners take.
        let theme = cx.theme();
        let accent = Hsla::from(rgb(GREEN));
        let border = theme.border;
        let muted = theme.muted;
        let muted_foreground = theme.muted_foreground;
        let surface = theme.secondary;
        let foreground = theme.foreground;
        let background = theme.background;
        let selected_index = self.pending_images.selected_index();

        h_flex()
            .flex_shrink_0()
            .items_center()
            .gap(px(THUMB_GAP * scale))
            .px(px(TRAY_PADDING * scale))
            .py(px(TRAY_PADDING * scale))
            .border_t_1()
            .border_color(border)
            .bg(background)
            .overflow_hidden()
            .children(
                self.pending_images
                    .images()
                    .iter()
                    .enumerate()
                    .map(|(index, image)| {
                        let selected = index == selected_index;
                        let id = image.id;
                        let picture = image.thumbnail.clone();
                        let body = match picture {
                            Some(picture) => img(ImageSource::Render(picture))
                                .size_full()
                                .object_fit(ObjectFit::Cover)
                                .into_any_element(),
                            None => div()
                                .size_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    Icon::new(WhatfastIcon::Attach)
                                        .with_size(px(24. * scale))
                                        .text_color(muted_foreground),
                                )
                                .into_any_element(),
                        };
                        div()
                            .id(("pending-image", id))
                            .relative()
                            .flex_none()
                            .size(px(THUMB * scale))
                            .rounded(px(THUMB_RADIUS * scale))
                            .overflow_hidden()
                            .border_2()
                            .border_color(if selected { accent } else { border })
                            .bg(muted)
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_pending_image(index, cx);
                            }))
                            .child(body)
                            // The cross that drops the picture, at the top
                            // right corner, like the reference.
                            .child(
                                div()
                                    .id(("remove-pending-image", id))
                                    .absolute()
                                    .top(px(2. * scale))
                                    .right(px(2. * scale))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(px(BADGE * scale))
                                    .rounded_full()
                                    .bg(rgba(0x000000b8))
                                    .hover(|style| style.bg(rgba(0x000000e6)))
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.remove_pending_image(index, window, cx);
                                        cx.notify();
                                    }))
                                    .child(
                                        Icon::new(WhatfastIcon::Close)
                                            .with_size(px(12. * scale))
                                            .text_color(rgb(0xffffff)),
                                    ),
                            )
                            // The pencil of the editor, on the picture whose
                            // options are in front.
                            .when(selected, |thumb| {
                                thumb.child(
                                    div()
                                        .id(("edit-pending-image", id))
                                        .absolute()
                                        .bottom(px(2. * scale))
                                        .right(px(2. * scale))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .size(px(BADGE * scale))
                                        .rounded_full()
                                        .bg(rgba(0x000000b8))
                                        .hover(|style| style.bg(rgba(0x000000e6)))
                                        .cursor_pointer()
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            cx.stop_propagation();
                                            this.edit_pending_image(index, window, cx);
                                            cx.notify();
                                        }))
                                        .child(
                                            Icon::new(WhatfastIcon::Edit)
                                                .with_size(px(12. * scale))
                                                .text_color(rgb(0xffffff)),
                                        ),
                                )
                            })
                    }),
            )
            // The square that adds more pictures, while there is room.
            .when(self.pending_images.can_add_more(), |row| {
                row.child(
                    div()
                        .id("add-pending-image")
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(THUMB * scale))
                        .rounded(px(THUMB_RADIUS * scale))
                        .border_1()
                        .border_color(border)
                        .bg(surface)
                        .cursor_pointer()
                        .hover(move |style| style.bg(muted))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.choose_image(window, cx);
                        }))
                        .child(
                            Icon::new(IconName::Plus)
                                .with_size(px(24. * scale))
                                .text_color(foreground),
                        ),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_PENDING_IMAGES, PendingImage, PendingImages, send_label};
    use std::path::PathBuf;

    /// One picture of the tray, without touching the disk.
    fn picture(id: u64) -> PendingImage {
        PendingImage {
            id,
            path: PathBuf::from(format!("photo-{id}.png")),
            thumbnail: None,
        }
    }

    /// A tray with `count` pictures, as if the user had picked them.
    fn tray(count: usize) -> PendingImages {
        let mut tray = PendingImages::default();
        for index in 0..count {
            tray.stage_for_test(picture(index as u64));
        }
        tray
    }

    impl PendingImages {
        /// The bookkeeping of `stage`, without the file of the backend.
        fn stage_for_test(&mut self, image: PendingImage) {
            self.images.push(PendingImage {
                id: self.next_id,
                ..image
            });
            self.next_id += 1;
            self.selected = self.images.len() - 1;
        }
    }

    #[test]
    fn a_new_picture_is_the_one_with_the_options_in_front() {
        let mut tray = PendingImages::default();
        tray.stage_for_test(picture(0));
        assert_eq!(tray.selected_index(), 0);
        tray.stage_for_test(picture(1));
        assert_eq!(tray.len(), 2);
        assert_eq!(tray.selected_index(), 1);
    }

    #[test]
    fn the_options_follow_the_picture_that_is_dropped() {
        let mut tray = tray(3);
        // The last one is in front, and dropping it leaves the one before.
        tray.remove(2);
        assert_eq!(tray.len(), 2);
        assert_eq!(tray.selected_index(), 1);

        // Dropping a picture before the one in front moves the options with
        // it: the same picture stays selected.
        tray.select(1);
        tray.remove(0);
        assert_eq!(tray.len(), 1);
        assert_eq!(tray.selected_index(), 0);

        // Dropping the last picture leaves no picture in front, and no
        // panic.
        tray.remove(0);
        assert!(tray.is_empty());
        assert_eq!(tray.selected_index(), 0);
        assert_eq!(tray.tray_height(1.0), gpui::px(0.0));
    }

    #[test]
    fn the_tray_holds_no_more_than_its_limit() {
        let full = tray(MAX_PENDING_IMAGES);
        assert!(!full.can_add_more());
        assert_eq!(full.len(), MAX_PENDING_IMAGES);
        // The last picture keeps the options: the seventh is not stolen.
        assert_eq!(full.selected_index(), MAX_PENDING_IMAGES - 1);
    }

    #[test]
    fn the_edited_picture_takes_the_place_of_the_one_it_came_from() {
        let mut tray = tray(2);
        let source = PathBuf::from("photo-1.png");
        assert!(tray.replace(&source, PathBuf::from("edited.png")));
        assert_eq!(tray.len(), 2);
        // The edited one keeps the number of the thumbnail and takes the
        // options, so the picture the user was working on stays in front.
        assert_eq!(tray.selected_index(), 1);
        assert_eq!(tray.images()[1].path, PathBuf::from("edited.png"));

        // A picture that is not in the tray is not replaced: the caller
        // stages it as a new one.
        assert!(!tray.replace(&PathBuf::from("otra.png"), PathBuf::from("edited.png")));
    }

    #[test]
    fn the_send_empties_the_tray() {
        let mut tray = tray(2);
        let taken = tray.take_all();
        assert_eq!(taken.len(), 2);
        assert!(tray.is_empty());
        assert_eq!(tray.selected_index(), 0);
        assert!(tray.take_all().is_empty());
    }

    #[test]
    fn the_tray_is_as_tall_as_its_thumbnail() {
        let tray = tray(1);
        // The thumbnail, the padding of the two sides and the border.
        assert_eq!(tray.tray_height(1.0), gpui::px(79.0));
        assert_eq!(tray.tray_height(2.0), gpui::px(157.0));
    }

    #[test]
    fn the_button_of_the_send_counts_the_pictures() {
        assert_eq!(send_label(0), "Send");
        assert_eq!(send_label(1), "Send");
        assert_eq!(send_label(2), "Send 2 photos");
        assert_eq!(send_label(8), "Send 8 photos");
    }
}
