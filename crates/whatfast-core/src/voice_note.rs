//! Port of `tauri/src/lib/voice-note.ts`.
//!
//! WhatsApp strictly requires Ogg Opus containers (OggS with OpusHead)
//! for push-to-talk voice notes. Chromium and WebView2 on Windows record
//! into `audio/webm` (EBML container) with Opus frames.
//!
//! This module converts WebM Opus recordings into standard Ogg Opus
//! bitstreams so WhatsApp treats them as genuine push-to-talk voice notes.
//!
//! port: the original runs in the webview and touches no browser API beyond
//! `TextEncoder`; the port builds the very same bytes with plain Rust, so
//! the module stays pure and dependency free.

/// The MIME prefix a recording must have to be sent as a push-to-talk note.
const PUSH_TO_TALK_PREFIX: &str = "audio/ogg";

/// The serial number the original stamps on every generated page: 'RMEW'.
const STREAM_SERIAL: u32 = 0x524D_4557;

/// A 20 ms Opus frame at 48 kHz carries 960 samples.
const SAMPLES_PER_FRAME: i64 = 960;

/// True only when the recorded container can be sent as a voice note.
pub fn is_push_to_talk_container(mime_type: &str) -> bool {
    mime_type.starts_with(PUSH_TO_TALK_PREFIX)
}

/// Standard Ogg CRC-32 lookup table (polynomial 0x04C11DB7).
///
/// port: the original fills the table at module load with a JS loop; the port
/// builds it at compile time. The polynomial is the *non reflected* one of the
/// Ogg specification: this is NOT the CRC-32 of zlib.
const fn build_ogg_crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut index = 0usize;
    while index < 256 {
        let mut remainder = (index as u32) << 24;
        let mut bit = 0;
        while bit < 8 {
            remainder = if remainder & 0x8000_0000 != 0 {
                (remainder << 1) ^ 0x04C1_1DB7
            } else {
                remainder << 1
            };
            bit += 1;
        }
        table[index] = remainder;
        index += 1;
    }
    table
}

static OGG_CRC_TABLE: [u32; 256] = build_ogg_crc_table();

/// The checksum of an Ogg page, computed with the CRC field read as zero.
fn ogg_crc(data: &[u8]) -> u32 {
    let mut crc: u32 = 0;
    for &byte in data {
        let index = ((crc >> 24) ^ u32::from(byte)) & 0xFF;
        crc = (crc << 8) ^ OGG_CRC_TABLE[index as usize];
    }
    crc
}

/// Creates an Ogg page header and data buffer with valid checksum.
fn create_ogg_page(
    header_type: u8,
    granule_pos: i64,
    serial: u32,
    seq: u32,
    packets: &[&[u8]],
) -> Vec<u8> {
    // Build segment table
    let mut segment_table: Vec<u8> = Vec::new();
    for packet in packets {
        let mut len = packet.len();
        while len >= 255 {
            segment_table.push(255);
            len -= 255;
        }
        segment_table.push(len as u8);
    }

    let header_len = 27 + segment_table.len();
    let payload_len: usize = packets.iter().map(|packet| packet.len()).sum();

    let mut page = vec![0u8; header_len + payload_len];

    // 'OggS'
    page[0] = 0x4F;
    page[1] = 0x67;
    page[2] = 0x67;
    page[3] = 0x53;
    page[4] = 0; // version
    page[5] = header_type;
    // port: every field of an Ogg page is little endian, and the original
    // writes the granule position with `setBigInt64(6, granulePos, true)`.
    page[6..14].copy_from_slice(&granule_pos.to_le_bytes());
    page[14..18].copy_from_slice(&serial.to_le_bytes());
    page[18..22].copy_from_slice(&seq.to_le_bytes());
    page[22..26].copy_from_slice(&0u32.to_le_bytes()); // CRC zeroed during calculation
    // port: the original assigns the segment count to a `Uint8Array` cell, so
    // the count wraps at 256.
    page[26] = segment_table.len() as u8;

    for (index, segment) in segment_table.iter().enumerate() {
        page[27 + index] = *segment;
    }

    let mut offset = header_len;
    for packet in packets {
        page[offset..offset + packet.len()].copy_from_slice(packet);
        offset += packet.len();
    }

    // Calculate CRC
    let checksum = ogg_crc(&page);
    page[22..26].copy_from_slice(&checksum.to_le_bytes());

    page
}

/// Build standard OpusHead packet (RFC 7845).
fn create_opus_head(channels: u8, sample_rate: u32) -> Vec<u8> {
    let mut head = vec![0u8; 19];
    // 'OpusHead'
    head[0..8].copy_from_slice(b"OpusHead");
    head[8] = 1; // version
    head[9] = channels; // channels
    head[10..12].copy_from_slice(&384u16.to_le_bytes()); // pre-skip
    head[12..16].copy_from_slice(&sample_rate.to_le_bytes()); // original sample rate
    head[16..18].copy_from_slice(&0i16.to_le_bytes()); // gain
    head[18] = 0; // channel mapping family
    head
}

/// Build standard OpusTags packet (RFC 7845).
fn create_opus_tags() -> Vec<u8> {
    let vendor = b"whatfast";
    let mut tags = vec![0u8; 8 + 4 + vendor.len() + 4];
    // 'OpusTags'
    tags[0..8].copy_from_slice(b"OpusTags");
    tags[8..12].copy_from_slice(&(vendor.len() as u32).to_le_bytes());
    tags[12..12 + vendor.len()].copy_from_slice(vendor);
    tags[12 + vendor.len()..16 + vendor.len()].copy_from_slice(&0u32.to_le_bytes()); // 0 user comments
    tags
}

/// A decoded EBML variable-size integer.
struct EbmlVarint {
    /// The value the varint carries, truncated to 32 bits.
    value: u32,
    /// How many bytes the varint occupies.
    size: usize,
}

/// Read EBML variable-size integer from buffer at offset.
fn read_ebml_varint(buffer: &[u8], offset: usize) -> Option<EbmlVarint> {
    if offset >= buffer.len() {
        return None;
    }
    let first = buffer[offset];
    if first == 0 {
        return None;
    }

    let mut mask: u32 = 0x80;
    let mut size: usize = 1;
    while (u32::from(first) & mask) == 0 && size <= 8 {
        mask >>= 1;
        size += 1;
    }
    if size > 8 || offset + size > buffer.len() {
        return None;
    }

    // port: the original shifts with the JS `<<` operator, which keeps only
    // the 32 bit pattern of the value.
    let mut value = u32::from(first) & (mask - 1);
    for index in 1..size {
        value = (value << 8) | u32::from(buffer[offset + index]);
    }
    Some(EbmlVarint { value, size })
}

/// Extract raw Opus frames from WebM/Matroska container bytes.
pub fn extract_opus_frames_from_webm(webm: &[u8]) -> Vec<Vec<u8>> {
    let mut frames: Vec<Vec<u8>> = Vec::new();
    let mut pos = 0usize;

    while pos < webm.len().saturating_sub(4) {
        // Look for SimpleBlock (ID 0xA3) or Block (ID 0xA1)
        if webm[pos] == 0xA3 || webm[pos] == 0xA1 {
            pos += 1; // skip ID
            let Some(size_varint) = read_ebml_varint(webm, pos) else {
                continue;
            };
            pos += size_varint.size;

            let block_size = size_varint.value as u64;
            if pos as u64 + block_size > webm.len() as u64 {
                break;
            }

            // Inside SimpleBlock/Block:
            // Track number (EBML varint)
            let Some(track_varint) = read_ebml_varint(webm, pos) else {
                pos = pos.saturating_add(block_size as usize);
                continue;
            };
            let header_skip = track_varint.size + 2 + 1; // track + timecode (2) + flags (1)
            if (header_skip as u64) < block_size {
                let frame_data = &webm[pos + header_skip..pos + block_size as usize];
                if !frame_data.is_empty() {
                    frames.push(frame_data.to_vec());
                }
            }
            pos = pos.saturating_add(block_size as usize);
        } else {
            pos += 1;
        }
    }

    frames
}

/// Remuxes WebM Opus audio into a valid Ogg Opus container.
/// If the input is already Ogg Opus, returns it unmodified.
///
/// port: the original returns the very same `Uint8Array` it received when the
/// input is already OggS or when no frame can be extracted; the port returns
/// a copy of those bytes.
pub fn webm_opus_to_ogg_opus(data: &[u8]) -> Vec<u8> {
    // If already OggS, return directly
    if data.len() >= 4 && data[0] == 0x4F && data[1] == 0x67 && data[2] == 0x67 && data[3] == 0x53 {
        return data.to_vec();
    }

    let frames = extract_opus_frames_from_webm(data);
    if frames.is_empty() {
        // If no frames extracted, return original data
        return data.to_vec();
    }

    let serial = STREAM_SERIAL; // 'RMEW'
    let mut pages: Vec<Vec<u8>> = Vec::new();

    // Page 0: OpusHead (header type 0x02 = beginning of stream)
    let head_packet = create_opus_head(1, 48000);
    pages.push(create_ogg_page(
        0x02,
        0,
        serial,
        0,
        &[head_packet.as_slice()],
    ));

    // Page 1: OpusTags
    let tags_packet = create_opus_tags();
    pages.push(create_ogg_page(
        0x00,
        0,
        serial,
        1,
        &[tags_packet.as_slice()],
    ));

    // Subsequent pages: Opus audio frames (e.g. 1 frame per page or small chunks)
    // Standard 20ms Opus frame has 960 samples at 48 kHz
    let mut current_granule: i64 = 0;
    let last = frames.len() - 1;

    for (index, frame) in frames.iter().enumerate() {
        current_granule += SAMPLES_PER_FRAME;
        let is_last = index == last;
        let header_type = if is_last { 0x04 } else { 0x00 }; // 0x04 = end of stream
        // port: the original keeps a counter that starts at 2 and grows with
        // every frame; the port takes the sequence number from the index.
        let seq = 2 + index as u32;
        pages.push(create_ogg_page(
            header_type,
            current_granule,
            serial,
            seq,
            &[frame.as_slice()],
        ));
    }

    // Combine pages
    let mut ogg = Vec::new();
    for page in &pages {
        ogg.extend_from_slice(page);
    }

    ogg
}
