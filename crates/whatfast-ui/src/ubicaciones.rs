//! The cards of the location and of the contacts of a message.
//!
//! port: taken from the origin's `tauri/src/components/MessageBubble.tsx`:
//! the cases `contacts` and `location` of `MessageContent`, and the classes
//! `contact-card`, `contact-icon`, `contact-meta`, `location-card`,
//! `location-icon` and `location-meta` of `tauri/src/styles.css`.
//!
//! The data comes from the content of the message (`whatfast_protocol`); this
//! module only draws the cards. The place of a location opens with the
//! program of the system, the same way the origin opens it with the browser
//! of the system.
//!
//! A message whose content is a location or a contact shows its card INSTEAD
//! of the line of text: the text of the message is the name of the place or
//! the name of the contact, and the card already carries it.

#![allow(dead_code)]

use gpui::{
    App, ElementId, FontWeight, Hsla, InteractiveElement as _, IntoElement as _,
    ParentElement as _, Rgba, StatefulInteractiveElement as _, Styled as _, div, px,
};
use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};

use whatfast_core::theme::ThemeAppearance;
use whatfast_protocol::{ContactContent, ContactsContent, LocationContent, message::Content};

use crate::helpers::one_line;
use crate::icons::WhatfastIcon;
use crate::text::is_safe_web_url;
use crate::tokens::{self, ThemeColors};

/// The width of a card, before the scale of the interface.
///
/// port: `width: min(calc(300px * var(--scale)), 55vw)` of the origin.
const CARD_WIDTH: f32 = 300.0;
/// The side of the circle of the icon of a card, before the scale.
///
/// port: the `width` and the `height` of `.location-icon` of the origin.
const BADGE_SIZE: f32 = 40.0;
/// The side of the icon of a card, before the scale.
///
/// port: the `size` of the `MapPin` and of the `ContactRound` of the origin.
const ICON_SIZE: f32 = 22.0;
/// The gap between the icon and the text of a card, before the scale.
///
/// port: the `gap` of `.location-card` of the origin.
const CARD_GAP: f32 = 10.0;
/// The padding of a card, before the scale.
///
/// port: the `padding` of `.location-card` of the origin.
const CARD_PADDING: f32 = 10.0;
/// The space that a card leaves below it, before the scale.
///
/// port: the `margin-bottom` of `.location-card` of the origin.
const CARD_MARGIN: f32 = 5.0;
/// The gap between the title and the subtitle of a card, before the scale.
///
/// port: the `margin-top` of `.location-meta span` of the origin.
const META_GAP: f32 = 3.0;

/// The colors of the theme of the app.
///
/// The tokens that `gpui_component` does not have (the soft color of the
/// accent, the subtle color of the text, the color of the hover and the color
/// of the active) come from the catalog of the core, with the same theme the
/// app wears: no color of a card is written by hand.
fn theme_colors(cx: &App) -> ThemeColors {
    let appearance = if cx.theme().is_dark() {
        ThemeAppearance::Dark
    } else {
        ThemeAppearance::Light
    };
    tokens::ThemeColors::resolve(&tokens::app_theme(appearance))
}

/// The title of the card of a location.
///
/// port: `location.name || (location.live ? "Live location" : "Location")` of
/// the origin.
pub(crate) fn location_title(location: &LocationContent) -> String {
    if !location.name.is_empty() {
        return location.name.clone();
    }
    if location.live {
        "Live location".to_string()
    } else {
        "Location".to_string()
    }
}

/// The subtitle of the card of a location: the address, or the coordinates.
///
/// port: `location.address || "lat, lon"` of the origin, with the five
/// decimals of `Number.prototype.toFixed`.
pub(crate) fn location_subtitle(location: &LocationContent) -> String {
    if !location.address.is_empty() {
        return location.address.clone();
    }
    format!(
        "{}, {}",
        fixed5(location.latitude),
        fixed5(location.longitude)
    )
}

/// The address that opens the place of a location.
///
/// port: `safeHttpUrl(location.url)` of the origin, and the link of
/// OpenStreetMap when the message does not bring one.
pub(crate) fn location_target(location: &LocationContent) -> String {
    if is_safe_web_url(&location.url) {
        return location.url.clone();
    }
    format!(
        "https://www.openstreetmap.org/?mlat={}&mlon={}",
        location.latitude, location.longitude
    )
}

/// The title of the card of a contact: its name, or "Contact".
///
/// port: `contact.displayName || "Contact"` of the origin.
pub(crate) fn contact_title(contact: &ContactContent) -> String {
    if contact.display_name.is_empty() {
        "Contact".to_string()
    } else {
        contact.display_name.clone()
    }
}

/// The subtitle of the card of a contact: the vcard of the contact.
///
/// The origin writes "Shared contact" here; this port writes the vcard, which
/// is the data that the message carries.
pub(crate) fn contact_subtitle(contact: &ContactContent) -> String {
    let vcard = one_line(&contact.vcard);
    if vcard.is_empty() {
        "Shared contact".to_string()
    } else {
        vcard
    }
}

/// A coordinate with five decimals, the way the oracle prints it.
///
/// port: `Number.prototype.toFixed(5)` of the origin rounds the halves away
/// from zero, while the formatter of Rust rounds them to the even one.
fn fixed5(value: f64) -> String {
    if !value.is_finite() {
        return format!("{value:.5}");
    }
    let sign = if value < 0.0 { "-" } else { "" };
    let scaled = value.abs() * 100_000.0;
    if scaled >= 1e15 {
        // A number this big is not a coordinate: the plain formatter is
        // better than a number that does not fit.
        return format!("{value:.5}");
    }
    let units = (scaled + 0.5).floor() as u64;
    format!("{sign}{}.{:05}", units / 100_000, units % 100_000)
}

/// The circle of the icon of a card.
///
/// port: `.location-icon` and `.contact-icon` of the origin: a circle with
/// the soft color of the accent and the icon in the color of the accent.
fn icon_badge(icon: Icon, scale: f32, accent: Hsla, accent_soft: Hsla) -> gpui::Div {
    div()
        .size(px(BADGE_SIZE * scale))
        .flex_none()
        .rounded_full()
        .bg(accent_soft)
        .flex()
        .items_center()
        .justify_center()
        .child(icon.with_size(px(ICON_SIZE * scale)).text_color(accent))
}

/// The two lines of the text of a card: the title in bold and the subtitle.
///
/// port: `.location-meta` and `.contact-meta` of the origin.
fn card_meta(title: String, subtitle: String, subtle: Rgba) -> gpui::Div {
    v_flex()
        .min_w_0()
        .flex_1()
        .gap(px(META_GAP))
        .child(
            div()
                .truncate()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(
            div()
                .truncate()
                .text_xs()
                .text_color(subtle)
                .child(subtitle),
        )
}

/// The card of a location.
///
/// port: the `location` case of `MessageContent` of the origin: a button
/// with the pin at the left, the name of the place in bold and the address
/// below it. Pressing it opens the place with the program of the system.
///
/// The `index` is the number of the message in the conversation: it names the
/// card, so that the state of the pointer belongs to this card and not to
/// another one.
pub(crate) fn render_location_card(
    location: &LocationContent,
    index: usize,
    scale: f32,
    cx: &mut App,
) -> gpui::Stateful<gpui::Div> {
    let colors = theme_colors(cx);
    let title = location_title(location);
    let subtitle = location_subtitle(location);
    let target = location_target(location);
    let radius = cx.theme().radius;
    let hover: Hsla = colors.bg_hover.into();
    let pressed: Hsla = colors.bg_active.into();
    let accent: Hsla = colors.accent.into();
    let accent_soft: Hsla = colors.accent_soft.into();

    h_flex()
        .id(ElementId::named_usize("location-card", index))
        .w(px(CARD_WIDTH * scale))
        .max_w_full()
        .items_center()
        .gap(px(CARD_GAP * scale))
        .p(px(CARD_PADDING * scale))
        .mb(px(CARD_MARGIN * scale))
        .rounded(radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.bg_elevated)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .active(move |style| style.bg(pressed))
        .on_click(move |_, _, cx| cx.open_url(&target))
        .child(icon_badge(
            Icon::new(WhatfastIcon::Pin),
            scale,
            accent,
            accent_soft,
        ))
        .child(card_meta(title, subtitle, colors.fg_subtle))
}

/// The card of a contact.
///
/// port: the `contacts` case of `MessageContent` of the origin: the icon of
/// the contact at the left, its name in bold and the vcard below it.
///
/// The `index` is the number of the message in the conversation and the
/// `position` is the number of the contact inside the message: the two name
/// the card.
pub(crate) fn render_contact_card(
    contact: &ContactContent,
    index: usize,
    position: usize,
    scale: f32,
    cx: &mut App,
) -> gpui::Stateful<gpui::Div> {
    let colors = theme_colors(cx);
    let title = contact_title(contact);
    let subtitle = contact_subtitle(contact);
    let radius = cx.theme().radius;
    let hover: Hsla = colors.bg_hover.into();
    let pressed: Hsla = colors.bg_active.into();
    let accent: Hsla = colors.accent.into();
    let accent_soft: Hsla = colors.accent_soft.into();
    // The card of a contact does not have a place to open: pressing it puts
    // the vcard in the clipboard, which is what the contact carries.
    let vcard = contact.vcard.clone();

    h_flex()
        .id(ElementId::from((
            ElementId::named_usize("contact-card", index),
            format!("contact-{position}"),
        )))
        .w(px(CARD_WIDTH * scale))
        .max_w_full()
        .items_center()
        .gap(px(CARD_GAP * scale))
        .p(px(CARD_PADDING * scale))
        .mb(px(CARD_MARGIN * scale))
        .rounded(radius)
        .border_1()
        .border_color(colors.border)
        .bg(colors.bg_elevated)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .active(move |style| style.bg(pressed))
        .on_click(move |_, _, cx| {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(vcard.clone()));
        })
        .child(icon_badge(
            Icon::new(IconName::User),
            scale,
            accent,
            accent_soft,
        ))
        .child(card_meta(title, subtitle, colors.fg_subtle))
}

/// The cards of the contacts of a message, one card per contact.
///
/// port: the origin draws one `contact-card` per contact of the message, in
/// the same list.
pub(crate) fn render_contacts_cards(
    contacts: &ContactsContent,
    index: usize,
    scale: f32,
    cx: &mut App,
) -> gpui::Div {
    v_flex().w_full().gap(px(CARD_MARGIN * scale)).children(
        contacts
            .contacts
            .iter()
            .enumerate()
            .map(|(position, contact)| {
                render_contact_card(contact, index, position, scale, cx).into_any_element()
            }),
    )
}

/// The card of a message whose content is a location or a contact.
///
/// Returns `None` when the content is something else, so the caller paints
/// the card or the text of the message, never the two.
pub(crate) fn render_content_card(
    content: &Content,
    index: usize,
    scale: f32,
    cx: &mut App,
) -> Option<gpui::AnyElement> {
    match content {
        Content::Location(location) => {
            Some(render_location_card(location, index, scale, cx).into_any_element())
        }
        Content::Contacts(contacts) => {
            Some(render_contacts_cards(contacts, index, scale, cx).into_any_element())
        }
        _ => None,
    }
}

/// Whether the content of a message is drawn as a card and not as a text.
///
/// A location and a contact carry their name in the card, so the line of
/// text of the message would repeat it.
pub(crate) fn is_card_content(content: &Content) -> bool {
    matches!(content, Content::Location(_) | Content::Contacts(_))
}

/// The height that the cards of a message add to its bubble, before the
/// scale.
///
/// The measure of the messages needs it: the card of a location (and the
/// cards of the contacts) is taller than the line of text that the message
/// would draw alone.
pub(crate) fn card_height(content: &Content, scale: f32) -> f32 {
    let cards = match content {
        Content::Location(_) => 1usize,
        Content::Contacts(contacts) => contacts.contacts.len(),
        _ => return 0.0,
    };
    if cards == 0 {
        return 0.0;
    }
    // The circle of the icon, the padding and the border of the card, the
    // space that the card leaves below it, and the gap between two cards of
    // contacts.
    let card = BADGE_SIZE + CARD_PADDING * 2.0 + 2.0 + CARD_MARGIN;
    let gaps = (cards - 1) as f32 * CARD_MARGIN;
    (cards as f32 * card + gaps) * scale
}

#[cfg(test)]
mod tests {
    use whatfast_protocol::{
        ContactContent, ContactsContent, LocationContent, TextContent, message::Content,
    };

    use super::{
        card_height, contact_subtitle, contact_title, fixed5, location_subtitle, location_target,
        location_title,
    };

    /// A location with the fields that the tests change.
    fn location() -> LocationContent {
        LocationContent {
            latitude: 12.4244,
            longitude: 75.7382,
            name: "Madikeri Homestay".into(),
            address: "Galibeedu Road, Madikeri".into(),
            url: "https://www.openstreetmap.org/?mlat=12.4244&mlon=75.7382".into(),
            live: false,
        }
    }

    /// A contact with the fields that the tests change.
    fn contact() -> ContactContent {
        ContactContent {
            display_name: "Homestay host".into(),
            vcard: "BEGIN:VCARD\nFN:Homestay host\nTEL:+919900001234\nEND:VCARD".into(),
        }
    }

    #[test]
    fn the_name_of_the_place_is_the_title_of_the_card() {
        assert_eq!(location_title(&location()), "Madikeri Homestay");
    }

    #[test]
    fn a_place_without_a_name_says_location() {
        let mut place = location();
        place.name = String::new();
        assert_eq!(location_title(&place), "Location");
    }

    #[test]
    fn a_place_without_a_name_that_moves_says_live_location() {
        let mut place = location();
        place.name = String::new();
        place.live = true;
        assert_eq!(location_title(&place), "Live location");
    }

    #[test]
    fn the_address_of_the_place_is_the_subtitle_of_the_card() {
        assert_eq!(location_subtitle(&location()), "Galibeedu Road, Madikeri");
    }

    #[test]
    fn a_place_without_an_address_shows_the_coordinates() {
        let mut place = location();
        place.address = String::new();
        assert_eq!(location_subtitle(&place), "12.42440, 75.73820");
    }

    #[test]
    fn the_link_of_the_message_opens_the_place() {
        assert_eq!(
            location_target(&location()),
            "https://www.openstreetmap.org/?mlat=12.4244&mlon=75.7382"
        );
    }

    #[test]
    fn a_place_without_a_link_opens_the_map_of_openstreetmap() {
        let mut place = location();
        place.url = String::new();
        assert_eq!(
            location_target(&place),
            "https://www.openstreetmap.org/?mlat=12.4244&mlon=75.7382"
        );
    }

    #[test]
    fn a_link_that_is_not_a_web_page_does_not_open() {
        let mut place = location();
        place.url = "javascript:alert(1)".into();
        assert_eq!(
            location_target(&place),
            "https://www.openstreetmap.org/?mlat=12.4244&mlon=75.7382"
        );
    }

    #[test]
    fn the_coordinates_keep_five_decimals() {
        assert_eq!(fixed5(12.4244), "12.42440");
        assert_eq!(fixed5(-0.5), "-0.50000");
        assert_eq!(fixed5(0.0), "0.00000");
        assert_eq!(fixed5(-0.0), "0.00000");
        assert_eq!(fixed5(12.424456), "12.42446");
    }

    #[test]
    fn the_name_of_the_contact_is_the_title_of_the_card() {
        assert_eq!(contact_title(&contact()), "Homestay host");
    }

    #[test]
    fn a_contact_without_a_name_says_contact() {
        let mut person = contact();
        person.display_name = String::new();
        assert_eq!(contact_title(&person), "Contact");
    }

    #[test]
    fn the_vcard_of_the_contact_is_the_subtitle_of_the_card() {
        assert_eq!(
            contact_subtitle(&contact()),
            "BEGIN:VCARD FN:Homestay host TEL:+919900001234 END:VCARD"
        );
    }

    #[test]
    fn a_contact_without_a_vcard_says_shared_contact() {
        let mut person = contact();
        person.vcard = String::new();
        assert_eq!(contact_subtitle(&person), "Shared contact");
    }

    #[test]
    fn the_card_of_a_place_is_taller_than_the_text() {
        let place = Content::Location(location());
        assert_eq!(card_height(&place, 1.0), 67.0);
    }

    #[test]
    fn the_cards_of_the_contacts_are_one_after_the_other() {
        let contacts = Content::Contacts(ContactsContent {
            contacts: vec![contact(), contact()],
        });
        assert_eq!(card_height(&contacts, 1.0), 139.0);
    }

    #[test]
    fn a_message_without_a_card_has_no_height_of_its_own() {
        assert_eq!(
            card_height(&Content::Text(TextContent::default()), 1.0),
            0.0
        );
    }
}
