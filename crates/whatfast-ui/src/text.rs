//! The pure text helpers of the client.
//!
//! port: taken from the origin's `desktop/src/main.rs` (the block of free
//! functions at lines 143-311 and the typing label at lines 392-434). Nothing
//! here touches the GPUI toolkit: these are the functions that decide whether
//! a URL is safe to click, where a mention token starts, how the mentions are
//! rewritten for the wire, whether a message may be sent and what the
//! "typing…" line says.
//!
//! The types the helpers need (`MentionEntry`, `TypingIndicator`) live in
//! `crate::app`, next to the state they belong to.

// Some of these helpers are the consumer of the drawing half of this batch
// (`crate::render` paints the link previews and the linkified text).
// TODO(port): drop this allow once `render.rs` is in the same crate.
#![allow(dead_code)]

use std::{collections::HashMap, fs, io::Read as _, path::Path, time::Instant};

use whatfast_protocol::ChatParticipant;

use crate::app::{
    MAX_CAPTION_BYTES, MAX_IMAGE_BYTES, MAX_TEXT_BYTES, MAX_UI_SCALE, MIN_UI_SCALE, MentionEntry,
    TypingIndicator,
};

/// Whether a URL may be turned into a clickable link.
///
/// port: the origin only accepted `http`/`https` with a non-empty host and
/// without whitespace, so a `javascript:` or `file:` URL never became a link.
pub(crate) fn is_safe_web_url(url: &str) -> bool {
    let remainder = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"));
    remainder.is_some_and(|value| {
        !value.is_empty() && !value.starts_with('/') && !value.chars().any(char::is_whitespace)
    })
}

/// Splits one whitespace-delimited token around its first HTTP(S) URL. Common
/// sentence punctuation is kept outside the clickable target.
pub(crate) fn split_url_token(token: &str) -> Option<(&str, &str, &str)> {
    let start = match (token.find("https://"), token.find("http://")) {
        (Some(https), Some(http)) => https.min(http),
        (Some(https), None) => https,
        (None, Some(http)) => http,
        (None, None) => return None,
    };
    let candidate = &token[start..];
    let url =
        candidate.trim_end_matches(['.', ',', '!', '?', ';', ':', ')', ']', '}', '>', '"', '\'']);
    if !is_safe_web_url(url) {
        return None;
    }
    Some((&token[..start], url, &candidate[url.len()..]))
}

/// The host of a URL, without its scheme and without the path.
pub(crate) fn link_preview_host(url: &str) -> &str {
    url.strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or(url)
}

/// Returns the byte offset of the `@` opening an in-progress mention token
/// that ends at `cursor`. The `@` must start the text or follow whitespace,
/// and the partial query may not contain whitespace or another `@`.
pub(crate) fn mention_token_start(text: &str, cursor: usize) -> Option<usize> {
    if !text.is_char_boundary(cursor) {
        return None;
    }
    let before = &text[..cursor];
    for (offset, character) in before.char_indices().rev() {
        if character == '@' {
            let preceded_ok = before[..offset]
                .chars()
                .next_back()
                .is_none_or(char::is_whitespace);
            return preceded_ok.then_some(offset);
        }
        if character.is_whitespace() {
            return None;
        }
    }
    None
}

/// The name a participant is shown with, and the name a mention token uses.
pub(crate) fn mention_display_name(participant: &ChatParticipant) -> String {
    if !participant.display_name.is_empty() {
        participant.display_name.clone()
    } else if !participant.phone_number.is_empty() {
        participant.phone_number.clone()
    } else {
        participant
            .participant_id
            .split('@')
            .next()
            .unwrap_or_default()
            .to_string()
    }
}

/// Rewrites `@Name` tokens to the `@<jid-user>` wire form WhatsApp clients
/// expect, returning the rewritten text and the JIDs it still references.
/// Mentions whose token the user deleted are dropped.
pub(crate) fn encode_mentions(text: &str, mentions: &[MentionEntry]) -> (String, Vec<String>) {
    let mut wire = text.to_string();
    let mut jids = Vec::new();
    let mut ordered: Vec<&MentionEntry> = mentions.iter().collect();
    // Longest names first so "@Ann" can never clobber part of "@Anna Lee".
    ordered.sort_by_key(|entry| std::cmp::Reverse(entry.display_name.len()));
    for entry in ordered {
        if entry.display_name.is_empty() {
            continue;
        }
        let Some(user) = entry.jid.split('@').next().filter(|user| !user.is_empty()) else {
            continue;
        };
        let token = format!("@{}", entry.display_name);
        if wire.contains(&token) {
            wire = wire.replace(&token, &format!("@{user}"));
            if !jids.contains(&entry.jid) {
                jids.push(entry.jid.clone());
            }
        }
    }
    (wire, jids)
}

/// Whether a text message may be sent, and why not when it may not.
pub(crate) fn validate_text_message(text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("Type a message before sending".into());
    }
    if text.len() > MAX_TEXT_BYTES {
        return Err(format!(
            "Message is too long ({} of {MAX_TEXT_BYTES} bytes)",
            text.len()
        ));
    }
    Ok(())
}

/// Whether an image may be sent, and why not when it may not.
///
/// port: the origin sniffed the first twelve bytes of the file instead of
/// trusting the extension, and it accepted exactly JPEG, PNG, GIF and WebP.
pub(crate) fn validate_image_message(path: &Path, caption: &str) -> Result<(), String> {
    if caption.len() > MAX_CAPTION_BYTES {
        return Err(format!(
            "Caption is too long ({} of {MAX_CAPTION_BYTES} bytes)",
            caption.len()
        ));
    }
    let metadata =
        fs::metadata(path).map_err(|_| "The selected image cannot be read".to_string())?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err("The selected image is not a non-empty file".into());
    }
    if metadata.len() > MAX_IMAGE_BYTES {
        return Err("Image is larger than the 32 MiB limit".into());
    }
    let mut file =
        fs::File::open(path).map_err(|_| "The selected image cannot be opened".to_string())?;
    let mut header = [0_u8; 12];
    let read = file
        .read(&mut header)
        .map_err(|_| "The selected image cannot be read".to_string())?;
    let supported = (read >= 3 && header[..3] == [0xff, 0xd8, 0xff])
        || (read >= 8 && header[..8] == [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a])
        || (read >= 6 && matches!(&header[..6], b"GIF87a" | b"GIF89a"))
        || (read >= 12 && &header[..4] == b"RIFF" && &header[8..12] == b"WEBP");
    if !supported {
        return Err("Choose a JPEG, PNG, GIF, or WebP image".into());
    }
    Ok(())
}

// The normalization of the scale belongs to the tokens of the theme: it is
// re-exported here so that the rest of the crate keeps reading it from this
// module, and there is only one definition.
pub(crate) use crate::tokens::normalized_ui_scale;

/// The "typing…" line of a chat, or `None` when nobody is typing there.
///
/// `group` picks the phrasing that names the member.
pub(crate) fn format_typing_label(
    indicators: &HashMap<String, TypingIndicator>,
    group: bool,
    now: Instant,
) -> Option<String> {
    let mut indicators = indicators
        .values()
        .filter(|indicator| indicator.expires_at > now)
        .collect::<Vec<_>>();
    if indicators.is_empty() {
        return None;
    }
    indicators.sort_by_key(|indicator| indicator.sender_name.to_lowercase());
    let action = if indicators.iter().all(|indicator| indicator.recording) {
        "recording audio…"
    } else {
        "typing…"
    };
    if !group {
        return Some(action.to_string());
    }
    let names = indicators
        .iter()
        .map(|indicator| {
            let name = indicator.sender_name.trim();
            if name.is_empty() { "Someone" } else { name }
        })
        .collect::<Vec<_>>();
    match names.as_slice() {
        [name] => Some(format!("{name} is {action}")),
        [first, second] => Some(format!("{first} and {second} are {action}")),
        [first, second, rest @ ..] => {
            let others = if rest.len() == 1 { "other" } else { "others" };
            Some(format!(
                "{first}, {second} and {} {others} are {action}",
                rest.len()
            ))
        }
        [] => None,
    }
}
