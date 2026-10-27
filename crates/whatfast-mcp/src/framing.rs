//! The framed envelopes of the local bridge, over async I/O.
//!
//! `whatfast_protocol::transport` holds the blocking half the desktop uses;
//! this is the mirror the agent server speaks. The rules come from
//! `proto/bridge.proto`: a 4-byte big-endian length followed by one encoded
//! `Envelope`.

use std::io;

use prost::Message as _;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use whatfast_protocol::{Envelope, envelope, transport::MAX_FRAME_BYTES};

/// Writes one length-prefixed envelope and flushes it.
pub async fn write_frame<W>(writer: &mut W, envelope: &Envelope) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let payload = envelope.encode_to_vec();
    if payload.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "outgoing frame is too big",
        ));
    }
    writer
        .write_all(&(payload.len() as u32).to_be_bytes())
        .await?;
    writer.write_all(&payload).await?;
    writer.flush().await
}

/// Reads one length-prefixed envelope the server sent back.
///
/// The server answers with responses only; a request frame here means the
/// stream is no longer the one this side opened.
pub async fn read_answer<R>(reader: &mut R) -> io::Result<Option<Envelope>>
where
    R: AsyncRead + Unpin,
{
    let mut header = [0u8; 4];
    let mut filled = 0;
    while filled < header.len() {
        let read = reader.read(&mut header[filled..]).await?;
        if read == 0 {
            return match filled {
                0 => Ok(None),
                _ => Err(truncated()),
            };
        }
        filled += read;
    }
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid frame length {length}"),
        ));
    }
    let mut payload = vec![0u8; length];
    reader.read_exact(&mut payload).await?;
    let envelope = Envelope::decode(payload.as_slice())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if !matches!(envelope.body, Some(envelope::Body::Response(_))) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the answer is not a response",
        ));
    }
    Ok(Some(envelope))
}

fn truncated() -> io::Error {
    io::Error::new(io::ErrorKind::UnexpectedEof, "truncated frame")
}
