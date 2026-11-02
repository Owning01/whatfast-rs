//! The card WhatsApp draws under a message that carries a link.
//!
//! WhatsApp previews the first link of a text: the client fetches the page
//! once and sends the title, the description and a small JPEG inside the
//! message, so the receivers never fetch the page themselves. When a text
//! carries several links the first one wins, the way the phone does it.
//!
//! The link is found with the same parser the interface underlines with, so
//! the card never points at a token that the bubble does not draw as a link.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};
use std::sync::LazyLock;
use std::time::Duration;

use url::{Host, Url};
use whatfast_core::whatsapp_formatting::{
    WhatsAppInlineSegment, WhatsAppParseOptions, parse_whatsapp_text,
};

/// A page is a text document, not a download: only the head matters. The cap
/// keeps a hostile host from spending the memory of the sidecar.
const MAX_HTML_BYTES: u64 = 512 * 1024;

/// The picture of the card is a thumbnail. The original may be a photograph
/// of ten megabytes, and the message only needs a few kilobytes.
const MAX_IMAGE_BYTES: u64 = 4 * 1024 * 1024;

/// A page whose picture is larger than this is not worth decoding: the card
/// is 320 pixels wide, and a giant image only costs memory and time.
const MAX_IMAGE_PIXELS: u64 = 40_000_000;

/// The card shows the picture at the width of a bubble, so a larger edge is
/// wasted bytes inside every copy of the message.
const THUMBNAIL_EDGE: u32 = 320;

/// The quality WhatsApp itself uses for the tiny previews of the history.
const THUMBNAIL_QUALITY: u8 = 70;

/// A page that does not answer in this time leaves the message without a
/// card: the link has to go out even when the site is down.
const PAGE_TIMEOUT: Duration = Duration::from_secs(3);
const IMAGE_TIMEOUT: Duration = Duration::from_secs(2);

/// The face WhatsApp Web shows when it asks for a page.
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
                          AppleWebKit/537.36 (KHTML, like Gecko) \
                          Chrome/124.0.0.0 Safari/537.36";

/// How many redirects a preview follows. Every hop passes the same
/// public-address check as the first link: a page that answers
/// `Location: http://192.168.1.1/` is not a door into the network of the
/// person using the application.
const MAX_REDIRECTS: usize = 3;

/// One client for every preview: one connection pool and one TLS session
/// cache, instead of a handshake per card.
static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
    ureq::config::Config::builder()
        // A 404 or a 302 is an answer: the caller reads the status.
        .http_status_as_error(false)
        // Redirects are followed by hand, one hop at a time, so each target
        // is checked before the next request.
        .max_redirects(0)
        .build()
        .into()
});

const MAX_TITLE_CHARS: usize = 200;
const MAX_DESCRIPTION_CHARS: usize = 400;

/// Everything the card needs, in the shape the WhatsApp message carries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LinkPreview {
    /// The link as it appears in the text: WhatsApp underlines this one.
    pub matched_text: String,
    pub title: String,
    pub description: String,
    /// A JPEG, because the field of the message is `jpegThumbnail`.
    pub thumbnail: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// The metadata of one page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct PageMeta {
    title: String,
    description: String,
    image: String,
}

/// The first link of the text, if it has one.
pub(crate) fn first_link(text: &str) -> Option<String> {
    let options = WhatsAppParseOptions::default();
    for block in parse_whatsapp_text(text, &options) {
        for segment in block.segments() {
            if let WhatsAppInlineSegment::Link { text, .. } = segment {
                return Some(text.clone());
            }
        }
    }
    None
}

/// The preview of the first link of one text, ready to ride on the message.
///
/// Runs on a blocking thread: the fetch is a synchronous HTTP call, and the
/// sidecar's loop must keep answering the desktop while the page lands.
pub(crate) async fn for_text(text: &str) -> Option<LinkPreview> {
    let text = text.to_owned();
    tokio::task::spawn_blocking(move || preview_of(&text))
        .await
        .ok()
        .flatten()
}

/// The blocking half of [`for_text`], kept apart so the tests can reach the
/// parser without a network.
fn preview_of(text: &str) -> Option<LinkPreview> {
    let matched_text = first_link(text)?;
    let (page, content_type) = fetch(&matched_text, MAX_HTML_BYTES, PAGE_TIMEOUT)?;
    if !content_type.is_empty() && !content_type.contains("html") {
        return None;
    }
    let html = String::from_utf8_lossy(&page);
    let meta = parse_page(&html, &matched_text);
    let title = compact(&meta.title, MAX_TITLE_CHARS);
    let description = compact(&meta.description, MAX_DESCRIPTION_CHARS);
    if title.is_empty() && description.is_empty() {
        return None;
    }
    let (thumbnail, width, height) = if meta.image.is_empty() {
        (Vec::new(), 0, 0)
    } else {
        thumbnail_of(&meta.image).unwrap_or_default()
    };
    Some(LinkPreview {
        matched_text,
        title,
        description,
        thumbnail,
        width,
        height,
    })
}

/// One GET with a hard deadline and a ceiling on the body.
///
/// Redirects are followed here, not by the client, because the destination
/// has to pass the same check as the link in the message.
fn fetch(url: &str, cap: u64, timeout: Duration) -> Option<(Vec<u8>, String)> {
    let mut target = url.to_owned();
    for _ in 0..=MAX_REDIRECTS {
        if !is_fetchable(&target) {
            return None;
        }
        let response = AGENT
            .get(&target)
            .config()
            .timeout_global(Some(timeout))
            .build()
            .header("User-Agent", USER_AGENT)
            .header(
                "Accept",
                "text/html,application/xhtml+xml,application/xml;q=0.9,image/*;q=0.8,*/*;q=0.5",
            )
            .call()
            .ok()?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get("location")
                .and_then(|value| value.to_str().ok())?;
            target = resolve_url(&target, location);
            continue;
        }
        if !response.status().is_success() {
            return None;
        }
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let body = response
            .into_body()
            .into_with_config()
            .limit(cap)
            .read_to_vec()
            .ok()?;
        return Some((body, content_type));
    }
    None
}

/// Whether a link may be fetched at all.
///
/// The preview is built from a link that anybody can send, so it never
/// reaches the machine itself or the network around it: loopback, the
/// private ranges, link-local (where the cloud metadata service lives),
/// carrier-grade NAT, multicast and the documentation ranges are refused.
/// A name is resolved and every address it answers must be public.
///
/// The name is resolved again by the client when it connects; a host that
/// changes its answer between the two lookups (DNS rebinding) is the one
/// gap this check does not close.
fn is_fetchable(url: &str) -> bool {
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return false;
    }
    match parsed.host() {
        Some(Host::Ipv4(address)) => is_public_v4(address),
        Some(Host::Ipv6(address)) => is_public_v6(address),
        Some(Host::Domain(name)) => {
            if name.eq_ignore_ascii_case("localhost") {
                return false;
            }
            let Some(port) = parsed.port_or_known_default() else {
                return false;
            };
            let Ok(addresses) = (name, port).to_socket_addrs() else {
                return false;
            };
            let mut answered = false;
            for address in addresses {
                answered = true;
                if !is_public_address(address.ip()) {
                    return false;
                }
            }
            // A name that does not resolve has nothing to fetch.
            answered
        }
        None => false,
    }
}

fn is_public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_public_v4(address),
        IpAddr::V6(address) => is_public_v6(address),
    }
}

fn is_public_v4(address: Ipv4Addr) -> bool {
    let [first, second, ..] = address.octets();
    !(first == 0
        || first == 10
        || first == 127
        || (first == 100 && (64..128).contains(&second))
        || (first == 169 && second == 254)
        || (first == 172 && (16..32).contains(&second))
        || (first == 192 && second == 168)
        || (first == 192 && second == 0)
        || (first == 198 && (second == 18 || second == 19))
        || (first == 198 && second == 51)
        || (first == 203 && second == 0)
        || first >= 224)
}

fn is_public_v6(address: Ipv6Addr) -> bool {
    if address.is_loopback() || address.is_unspecified() || address.is_multicast() {
        return false;
    }
    let segments = address.segments();
    // Unique local (fc00::/7) and link-local (fe80::/10).
    if (segments[0] & 0xfe00) == 0xfc00 || (segments[0] & 0xffc0) == 0xfe80 {
        return false;
    }
    // Documentation (2001:db8::/32).
    if segments[0] == 0x2001 && segments[1] == 0x0db8 {
        return false;
    }
    // A v4 address carried inside a v6 one is checked as the v4 address it
    // is: ::ffff:127.0.0.1 is loopback, not a public v6 host.
    match embedded_v4(&segments) {
        Some(address) => is_public_v4(address),
        None => true,
    }
}

/// The v4 address inside an IPv4-mapped, IPv4-compatible or NAT64 address.
fn embedded_v4(segments: &[u16; 8]) -> Option<Ipv4Addr> {
    let mapped = segments[..5] == [0, 0, 0, 0, 0] && segments[5] == 0xffff;
    let compatible =
        segments[..6] == [0, 0, 0, 0, 0, 0] && (segments[6] != 0 || segments[7] > 1);
    let nat64 =
        segments[0] == 0x0064 && segments[1] == 0xff9b && segments[2..6] == [0, 0, 0, 0];
    if !(mapped || compatible || nat64) {
        return None;
    }
    Some(Ipv4Addr::new(
        (segments[6] >> 8) as u8,
        segments[6] as u8,
        (segments[7] >> 8) as u8,
        segments[7] as u8,
    ))
}

/// The head of one page: Open Graph first, the classic tags after it.
fn parse_page(html: &str, page_url: &str) -> PageMeta {
    let lower = html.to_ascii_lowercase();
    let mut meta = PageMeta::default();
    let mut start = 0;
    while let Some(found) = lower[start..].find("<meta") {
        let open = start + found;
        let Some(close) = lower[open..].find('>').map(|index| open + index) else {
            break;
        };
        let attributes = attributes_of(&html[open + 5..close]);
        let key = attributes
            .iter()
            .find(|(name, _)| name == "property" || name == "name")
            .map(|(_, value)| value.to_ascii_lowercase())
            .unwrap_or_default();
        let content = attributes
            .iter()
            .find(|(name, _)| name == "content")
            .map(|(_, value)| decode_entities(value))
            .unwrap_or_default();
        if !content.is_empty() {
            match key.as_str() {
                "og:title" | "twitter:title" => fill(&mut meta.title, &content),
                "og:description" | "twitter:description" | "description" => {
                    fill(&mut meta.description, &content);
                }
                "og:image" | "og:image:url" | "og:image:secure_url" | "twitter:image"
                | "twitter:image:src" => {
                    if meta.image.is_empty() {
                        meta.image = resolve_url(page_url, &content);
                    }
                }
                _ => {}
            }
        }
        start = close + 1;
    }
    if meta.title.is_empty() {
        meta.title = page_title(&lower, html);
    }
    meta
}

/// The `<title>` of the page, the fallback WhatsApp uses when the page has
/// no Open Graph title.
fn page_title(lower: &str, html: &str) -> String {
    let Some(open) = lower.find("<title") else {
        return String::new();
    };
    let Some(body) = lower[open..].find('>').map(|index| open + index + 1) else {
        return String::new();
    };
    let Some(close) = lower[body..].find("</title").map(|index| body + index) else {
        return String::new();
    };
    decode_entities(html[body..close].trim())
}

/// The attributes of one tag, lowercased names, decoded values.
fn attributes_of(tag: &str) -> Vec<(String, String)> {
    let chars: Vec<char> = tag.chars().collect();
    let mut out = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        while index < chars.len()
            && !chars[index].is_alphanumeric()
            && chars[index] != '_'
            && chars[index] != ':'
            && chars[index] != '-'
        {
            index += 1;
        }
        let name_start = index;
        while index < chars.len()
            && (chars[index].is_alphanumeric()
                || chars[index] == '_'
                || chars[index] == ':'
                || chars[index] == '-')
        {
            index += 1;
        }
        if name_start == index {
            break;
        }
        let name: String = chars[name_start..index]
            .iter()
            .collect::<String>()
            .to_ascii_lowercase();
        while index < chars.len() && chars[index].is_whitespace() {
            index += 1;
        }
        if index >= chars.len() || chars[index] != '=' {
            continue;
        }
        index += 1;
        while index < chars.len() && chars[index].is_whitespace() {
            index += 1;
        }
        if index < chars.len() && (chars[index] == '"' || chars[index] == '\'') {
            let quote = chars[index];
            index += 1;
            let value_start = index;
            while index < chars.len() && chars[index] != quote {
                index += 1;
            }
            let value: String = chars[value_start..index].iter().collect();
            out.push((name, value));
            if index < chars.len() {
                index += 1;
            }
        } else {
            let value_start = index;
            while index < chars.len() && !chars[index].is_whitespace() {
                index += 1;
            }
            let value: String = chars[value_start..index].iter().collect();
            out.push((name, value));
        }
    }
    out
}

/// The five entities a title usually carries, plus the numeric forms.
fn decode_entities(value: &str) -> String {
    if !value.contains('&') {
        return value.to_owned();
    }
    let chars: Vec<char> = value.chars().collect();
    let mut out = String::with_capacity(value.len());
    let mut index = 0;
    while index < chars.len() {
        if chars[index] != '&' {
            out.push(chars[index]);
            index += 1;
            continue;
        }
        let Some(end) = chars[index..]
            .iter()
            .take(12)
            .position(|value| *value == ';')
            .map(|offset| index + offset)
        else {
            out.push('&');
            index += 1;
            continue;
        };
        let entity: String = chars[index + 1..end].iter().collect();
        let decoded = match entity.as_str() {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            "hellip" => Some('…'),
            "mdash" => Some('—'),
            "ndash" => Some('–'),
            "rsquo" => Some('’'),
            "lsquo" => Some('‘'),
            "ldquo" => Some('“'),
            "rdquo" => Some('”'),
            _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                u32::from_str_radix(&entity[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
            }
            _ if entity.starts_with('#') => entity[1..]
                .parse::<u32>()
                .ok()
                .and_then(char::from_u32),
            _ => None,
        };
        match decoded {
            Some(value) => {
                out.push(value);
                index = end + 1;
            }
            None => {
                out.push('&');
                index += 1;
            }
        }
    }
    out
}

/// An absolute picture, resolved against the page that names it.
fn resolve_url(page_url: &str, source: &str) -> String {
    let source = source.trim();
    if source.is_empty() {
        return String::new();
    }
    if source.starts_with("http://") || source.starts_with("https://") {
        return source.to_owned();
    }
    let (scheme, host, path) = split_url(page_url);
    if let Some(rest) = source.strip_prefix("//") {
        return format!("{scheme}://{rest}");
    }
    if source.starts_with('/') {
        return format!("{scheme}://{host}{source}");
    }
    let directory = match path.rfind('/') {
        Some(index) => &path[..=index],
        None => "/",
    };
    format!("{scheme}://{host}{directory}{source}")
}

fn split_url(url: &str) -> (&str, &str, &str) {
    let (scheme, rest) = url.split_once("://").unwrap_or(("https", url));
    match rest.find('/') {
        Some(index) => (scheme, &rest[..index], &rest[index..]),
        None => (scheme, rest, "/"),
    }
}

/// The picture of the card: decoded, shrunk, and encoded as the JPEG that
/// the message carries.
fn thumbnail_of(url: &str) -> Option<(Vec<u8>, u32, u32)> {
    let (bytes, _) = fetch(url, MAX_IMAGE_BYTES, IMAGE_TIMEOUT)?;
    let reader = image::ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()
        .ok()?;
    let (width, height) = reader.into_dimensions().ok()?;
    if u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
        return None;
    }
    let picture = image::load_from_memory(&bytes).ok()?;
    let thumbnail = picture.thumbnail(THUMBNAIL_EDGE, THUMBNAIL_EDGE);
    let mut out = Vec::new();
    let mut encoder =
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, THUMBNAIL_QUALITY);
    encoder.encode_image(&thumbnail).ok()?;
    Some((out, thumbnail.width(), thumbnail.height()))
}

/// Writes the value only when the field is still empty: the first tag wins,
/// the way the phone reads them.
fn fill(field: &mut String, value: &str) {
    if field.is_empty() {
        field.push_str(value);
    }
}

/// One line, without the surrounding noise, cut at the limit WhatsApp shows.
fn compact(value: &str, max_chars: usize) -> String {
    let joined = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.chars().count() <= max_chars {
        return joined;
    }
    joined.chars().take(max_chars).collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_link_of_a_text_wins() {
        let text = "mira https://a.example/uno y despues https://b.example/dos";
        assert_eq!(
            first_link(text).as_deref(),
            Some("https://a.example/uno")
        );
    }

    #[test]
    fn a_text_without_a_link_has_no_preview() {
        assert_eq!(first_link("hola, sin enlaces"), None);
    }

    #[test]
    fn a_preview_never_reaches_the_machine_or_its_network() {
        for url in [
            "http://127.0.0.1:8080/",
            "http://10.1.2.3/",
            "http://172.16.0.9/",
            "http://192.168.1.1/",
            "http://169.254.169.254/latest/meta-data/",
            "http://[::1]/",
            "http://[fe80::1]/",
            "http://[fd00::1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://localhost/",
            "file:///C:/Windows/win.ini",
            "ftp://example.com/file",
        ] {
            assert!(!is_fetchable(url), "{url} should be refused");
        }
    }

    #[test]
    fn a_public_address_is_allowed() {
        assert!(is_fetchable("https://93.184.216.34/"));
        assert!(is_fetchable("http://1.1.1.1/imagen.png"));
    }

    #[test]
    fn the_v4_address_inside_a_v6_one_is_checked_too() {
        assert!(!is_public_address("::ffff:192.168.0.1".parse().unwrap()));
        assert!(!is_public_address("64:ff9b::7f00:1".parse().unwrap()));
        assert!(is_public_address("::ffff:93.184.216.34".parse().unwrap()));
    }

    #[test]
    fn the_metadata_comes_from_the_open_graph_tags() {
        let html = r#"
            <html><head>
            <meta property="og:title" content="Una pagina">
            <meta property="og:description" content="Lo que cuenta">
            <meta property="og:image" content="https://cdn.example/p.png">
            </head></html>
        "#;
        let meta = parse_page(html, "https://example.com/nota");
        assert_eq!(meta.title, "Una pagina");
        assert_eq!(meta.description, "Lo que cuenta");
        assert_eq!(meta.image, "https://cdn.example/p.png");
    }

    #[test]
    fn the_title_of_the_page_is_the_fallback() {
        let html = "<html><head><title>Sin Open Graph</title></head></html>";
        let meta = parse_page(html, "https://example.com");
        assert_eq!(meta.title, "Sin Open Graph");
        assert!(meta.description.is_empty());
    }

    #[test]
    fn the_entities_of_the_metadata_are_decoded() {
        let html = r#"<meta property="og:title" content="Perez &amp; C&#237;a &#x2014; home">"#;
        let meta = parse_page(html, "https://example.com");
        assert_eq!(meta.title, "Perez & Cía — home");
    }

    #[test]
    fn a_relative_picture_is_resolved_against_the_page() {
        let root = parse_page(
            r#"<meta property="og:image" content="/img/portada.png">"#,
            "https://example.com/nota/hoy",
        );
        assert_eq!(root.image, "https://example.com/img/portada.png");
        let sibling = parse_page(
            r#"<meta property="og:image" content="portada.png">"#,
            "https://example.com/nota/hoy",
        );
        assert_eq!(sibling.image, "https://example.com/nota/portada.png");
        let protocol = parse_page(
            r#"<meta property="og:image" content="//cdn.example/p.webp">"#,
            "https://example.com/nota",
        );
        assert_eq!(protocol.image, "https://cdn.example/p.webp");
    }

    #[test]
    fn the_first_picture_of_the_page_wins() {
        let html = r#"
            <meta name="twitter:image" content="https://cdn.example/primera.png">
            <meta property="og:image" content="https://cdn.example/segunda.png">
        "#;
        let meta = parse_page(html, "https://example.com");
        assert_eq!(meta.image, "https://cdn.example/primera.png");
    }

    /// Manual check of the whole path against the real network: it is
    /// ignored because the suite must pass offline.
    #[test]
    #[ignore]
    fn manual_preview_of_a_real_page() {
        for url in ["https://www.rust-lang.org/", "https://example.com/"] {
            let preview = preview_of(&format!("mira {url}"));
            match preview {
                Some(preview) => println!(
                    "{url} -> title={:?} description={:?} thumb={} bytes {}x{} matched={:?}",
                    preview.title,
                    preview.description,
                    preview.thumbnail.len(),
                    preview.width,
                    preview.height,
                    preview.matched_text
                ),
                None => println!("{url} -> sin tarjeta"),
            }
        }
    }

    #[test]
    fn the_quotes_of_the_attributes_do_not_matter() {
        let html = "<meta property=og:title content='Con comillas simples'>";
        let meta = parse_page(html, "https://example.com");
        assert_eq!(meta.title, "Con comillas simples");
    }
}
