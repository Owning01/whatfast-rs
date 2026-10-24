//! The server side of the framed bridge transport.
//!
//! `whatfast-protocol`'s transport owns the client half (it spawns this
//! process and drives requests); this module is the mirror a backend speaks:
//! frames in on stdin, framed responses and events out on stdout. The rules
//! come from the contract in `proto/bridge.proto`: a 4-byte big-endian length
//! followed by one encoded `Envelope`, and stdout never carrying anything but
//! framed bytes.

use std::io::{Read, Write};

use prost::Message as _;
use whatfast_protocol::{Envelope, envelope, transport::MAX_FRAME_BYTES};

/// Why a frame could not be read. Both ends of a framing failure are fatal
/// for the process: the desktop cannot resynchronize a half-read frame.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    /// The desktop closed its side. Not an error to log loudly: it is how
    /// the desktop ends the process between sessions.
    #[error("stdin closed")]
    Closed,
    #[error("invalid frame length {length}")]
    Length { length: u32 },
    #[error("decoding an envelope: {0}")]
    Decode(#[from] prost::DecodeError),
    /// The desktop sent a response or an event frame: the stream is out of
    /// sync and cannot be resynchronized.
    #[error("desktop frame is not a request")]
    NotARequest,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Reads one length-prefixed envelope. `Err(FrameError::Closed)` marks a
/// clean end of the stream.
pub fn read_frame(reader: &mut impl Read) -> Result<Envelope, FrameError> {
    let mut header = [0u8; 4];
    read_exact(reader, &mut header)?;
    let length = u32::from_be_bytes(header);
    if length == 0 || length as usize > MAX_FRAME_BYTES {
        return Err(FrameError::Length { length });
    }
    let mut payload = vec![0u8; length as usize];
    read_exact(reader, &mut payload)?;
    let envelope = Envelope::decode(payload.as_slice())?;
    // The backend only ever receives requests from the desktop; a response or
    // event frame here means the stream is out of sync.
    if !matches!(envelope.body, Some(envelope::Body::Request(_))) {
        return Err(FrameError::NotARequest);
    }
    Ok(envelope)
}

/// Writes one length-prefixed envelope and flushes it, so a response is on
/// the wire before the request handler moves on.
pub fn write_frame(writer: &mut impl Write, envelope: &Envelope) -> Result<(), FrameError> {
    let payload = envelope.encode_to_vec();
    if payload.len() > MAX_FRAME_BYTES {
        return Err(FrameError::Length {
            length: payload.len() as u32,
        });
    }
    writer.write_all(&(payload.len() as u32).to_be_bytes())?;
    writer.write_all(&payload)?;
    writer.flush()?;
    Ok(())
}

/// `Read::read_exact` reports EOF through an error whose kind is not distinct
/// from a broken pipe, so the length is read byte-tolerantly to tell a clean
/// close after a frame boundary from a truncated frame.
fn read_exact(reader: &mut impl Read, buffer: &mut [u8]) -> Result<(), FrameError> {
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..])? {
            0 if filled == 0 => return Err(FrameError::Closed),
            0 => {
                return Err(FrameError::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "truncated frame",
                )));
            }
            n => filled += n,
        }
    }
    Ok(())
}
