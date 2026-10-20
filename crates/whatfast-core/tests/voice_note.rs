//! Port of `tauri/tests/voice-note.test.ts`.
//!
//! Every case of the oracle suite has its own test here, with the same name
//! translated to snake_case.

use whatfast_core::voice_note::{
    extract_opus_frames_from_webm, is_push_to_talk_container, webm_opus_to_ogg_opus,
};

#[test]
fn only_ogg_opus_recordings_are_push_to_talk_voice_notes() {
    assert!(is_push_to_talk_container("audio/ogg;codecs=opus"));
    assert!(is_push_to_talk_container("audio/ogg"));
    assert!(!is_push_to_talk_container("audio/webm;codecs=opus"));
    assert!(!is_push_to_talk_container("audio/webm"));
    assert!(!is_push_to_talk_container("audio/mp4"));
    assert!(!is_push_to_talk_container(""));
}

#[test]
fn webm_opus_to_ogg_opus_passes_through_existing_oggs_data() {
    let ogg_header = [0x4F, 0x67, 0x67, 0x53, 0x00, 0x02];
    let result = webm_opus_to_ogg_opus(&ogg_header);
    assert_eq!(result, ogg_header);
}

#[test]
fn webm_opus_to_ogg_opus_converts_webm_simpleblocks_into_valid_ogg_opus_container() {
    // Construct a minimal synthetic WebM with 1 SimpleBlock (0xA3)
    // EBML ID: 0xA3, size: 0x86 (6 bytes)
    // Track 1: 0x81 (1 byte), timecode: 0x00 0x00 (2 bytes), flags: 0x80 (1 byte), Opus payload: [0xFC, 0x01] (2 bytes)
    let synthetic_webm = [
        0x1A, 0x45, 0xDF, 0xA3, // EBML header
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, // EBML size
        0xA3, 0x86, // SimpleBlock ID + length (6 bytes)
        0x81, // Track 1
        0x00, 0x00, // timecode 0
        0x80, // flags keyframe
        0xFC, 0x01, // Opus packet data
    ];

    let converted = webm_opus_to_ogg_opus(&synthetic_webm);
    assert!(converted.len() > synthetic_webm.len());

    // Starts with 'OggS'
    assert_eq!(converted[0], 0x4F);
    assert_eq!(converted[1], 0x67);
    assert_eq!(converted[2], 0x67);
    assert_eq!(converted[3], 0x53);

    // Contains OpusHead
    // port: the oracle decodes the container with an ASCII `TextDecoder` and
    // looks for the two magic strings; the port looks for the same bytes.
    assert!(converted.windows(8).any(|window| window == b"OpusHead"));
    assert!(converted.windows(8).any(|window| window == b"OpusTags"));

    // The container really is an Ogg stream: every page checksums and the
    // pages demux back into the packets the container was built from.
    let pages = demux_ogg(&converted);
    assert_eq!(pages.len(), 3);

    assert_eq!(pages[0].header_type, 0x02); // beginning of stream
    assert_eq!(pages[0].sequence, 0);
    assert_eq!(pages[0].granule, 0);
    assert_eq!(pages[0].payload.len(), 19);
    assert_eq!(&pages[0].payload[0..8], b"OpusHead");

    assert_eq!(pages[1].header_type, 0x00); // a page in the middle
    assert_eq!(pages[1].sequence, 1);
    assert_eq!(pages[1].granule, 0);
    assert_eq!(&pages[1].payload[0..8], b"OpusTags");
    assert_eq!(&pages[1].payload[12..20], b"whatfast");

    assert_eq!(pages[2].header_type, 0x04); // end of stream
    assert_eq!(pages[2].sequence, 2);
    assert_eq!(pages[2].granule, 960); // one 20 ms frame at 48 kHz
    // port: the scanner of the original is naive: it finds the `0xA3` that
    // closes the EBML magic (`0x1A 0x45 0xDF 0xA3`), reads the next eight
    // bytes as the size of the block (the `0x01` of the EBML size is a varint
    // with the marker in its lowest bit) and keeps the four bytes that come
    // after the track number and the first timecode byte: the second timecode
    // byte, the flags and the two payload bytes. The port keeps those four
    // bytes too: the oracle is the judge.
    assert_eq!(pages[2].payload, vec![0x00, 0x80, 0xFC, 0x01]);

    // The three pages belong to the same logical stream.
    assert_eq!(pages[0].serial, pages[1].serial);
    assert_eq!(pages[1].serial, pages[2].serial);

    // The frame the remuxer took out of the WebM is the one it packed.
    assert_eq!(
        extract_opus_frames_from_webm(&synthetic_webm),
        vec![vec![0x00, 0x80, 0xFC, 0x01]]
    );
}

/// One Ogg page read back from a generated container.
struct OggPage {
    header_type: u8,
    granule: i64,
    serial: u32,
    sequence: u32,
    payload: Vec<u8>,
}

/// Reads the pages of an Ogg stream, checking the checksum of every one.
///
/// The checksum is recomputed bit by bit, with a different algorithm than the
/// table of the module, so the check does not repeat the same mistake twice.
fn demux_ogg(ogg: &[u8]) -> Vec<OggPage> {
    let mut pages = Vec::new();
    let mut offset = 0usize;

    while offset < ogg.len() {
        assert_eq!(&ogg[offset..offset + 4], b"OggS");
        let header_type = ogg[offset + 5];
        let granule = read_i64(&ogg[offset + 6..offset + 14]);
        let serial = read_u32(&ogg[offset + 14..offset + 18]);
        let sequence = read_u32(&ogg[offset + 18..offset + 22]);
        let checksum = read_u32(&ogg[offset + 22..offset + 26]);
        let segment_count = ogg[offset + 26] as usize;

        let table_end = offset + 27 + segment_count;
        let payload_len: usize = ogg[offset + 27..table_end]
            .iter()
            .map(|segment| *segment as usize)
            .sum();
        let page_end = table_end + payload_len;

        // The checksum covers the whole page with the checksum field zeroed.
        let mut page = ogg[offset..page_end].to_vec();
        page[22..26].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(ogg_crc_bitwise(&page), checksum);

        pages.push(OggPage {
            header_type,
            granule,
            serial,
            sequence,
            payload: ogg[table_end..page_end].to_vec(),
        });

        offset = page_end;
    }

    pages
}

/// The Ogg CRC-32 (polynomial 0x04C11DB7, not reflected) computed bit by bit.
fn ogg_crc_bitwise(data: &[u8]) -> u32 {
    let mut crc: u32 = 0;
    for &byte in data {
        crc ^= u32::from(byte) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04C1_1DB7
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn read_u32(bytes: &[u8]) -> u32 {
    let mut buffer = [0u8; 4];
    buffer.copy_from_slice(bytes);
    u32::from_le_bytes(buffer)
}

fn read_i64(bytes: &[u8]) -> i64 {
    let mut buffer = [0u8; 8];
    buffer.copy_from_slice(bytes);
    i64::from_le_bytes(buffer)
}
