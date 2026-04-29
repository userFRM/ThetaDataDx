//! PacketStream wire framing for the MDDS legacy port.
//!
//! Frame layout (all big-endian):
//!
//! ```text
//! offset 0  : u32 payload_size (NOT including the 14-byte header)
//! offset 4  : u16 message_code
//! offset 6  : i64 request_id  (-1 for connection-scoped frames)
//! offset 14 : N  bytes of payload
//! ```
//!
//! This module provides a minimal async reader/writer over `tokio::io`
//! traits. The writer flushes after each frame so the server sees discrete
//! requests rather than a coalesced batch (the vendor server is sensitive
//! to this; it does not handle two queued frames back-to-back without a
//! flush boundary).

use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::Error;

/// Maximum payload size we will accept from the server. Each chunk in a
/// FLAT_FILE response is observed at ~512 KiB; the server is configured to
/// allocate up to 32 MiB internally. We accept anything up to 64 MiB and
/// reject larger frames as protocol violations to avoid unbounded
/// allocation under malicious input.
const MAX_PAYLOAD: u32 = 64 * 1024 * 1024;

/// Wire codes for the messages this module produces or consumes. Defined as
/// `u16` constants rather than an enum because the wire byte values are
/// fixed by the protocol and the constants are matched against incoming
/// frame headers.
#[allow(dead_code)]
pub(crate) mod msg {
    pub const CREDENTIALS: u16 = 0;
    pub const SESSION_TOKEN: u16 = 1;
    pub const METADATA: u16 = 3;
    pub const CONNECTED: u16 = 4;
    pub const VERSION: u16 = 5;
    pub const PING: u16 = 100;
    pub const ERROR: u16 = 101;
    pub const DISCONNECTED: u16 = 102;
    pub const FLAT_FILE: u16 = 217;
    pub const FLAT_FILE_END: u16 = 218;
}

/// Single decoded frame.
pub(crate) struct Frame {
    pub msg: u16,
    pub id: i64,
    pub payload: Vec<u8>,
}

/// Encode and write a single frame to `out`, then flush.
pub(crate) async fn write_frame<W>(
    out: &mut W,
    msg: u16,
    id: i64,
    payload: &[u8],
) -> Result<(), Error>
where
    W: AsyncWrite + Unpin,
{
    let size = u32::try_from(payload.len()).map_err(|_| {
        Error::Config(format!(
            "flatfiles: payload {} bytes exceeds u32",
            payload.len()
        ))
    })?;
    let mut header = [0u8; 14];
    header[0..4].copy_from_slice(&size.to_be_bytes());
    header[4..6].copy_from_slice(&msg.to_be_bytes());
    header[6..14].copy_from_slice(&id.to_be_bytes());
    out.write_all(&header).await?;
    out.write_all(payload).await?;
    out.flush().await?;
    Ok(())
}

/// Read one frame from `src`. Errors propagate the underlying tokio error.
pub(crate) async fn read_frame<R>(src: &mut R) -> Result<Frame, Error>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut hdr = [0u8; 14];
    src.read_exact(&mut hdr).await?;
    let size = u32::from_be_bytes(hdr[0..4].try_into().unwrap());
    let msg = u16::from_be_bytes(hdr[4..6].try_into().unwrap());
    let id = i64::from_be_bytes(hdr[6..14].try_into().unwrap());
    if size > MAX_PAYLOAD {
        return Err(Error::Config(format!(
            "flatfiles: server frame size {size} exceeds {MAX_PAYLOAD}"
        )));
    }
    let mut payload = vec![0u8; size as usize];
    src.read_exact(&mut payload).await?;
    Ok(Frame { msg, id, payload })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn write_then_read_round_trip() {
        let payload = b"hello world".to_vec();
        let mut buf: Vec<u8> = Vec::new();
        write_frame(&mut buf, 217, 9001, &payload).await.unwrap();
        // header + payload = 14 + 11
        assert_eq!(buf.len(), 25);
        let mut cursor = std::io::Cursor::new(&buf);
        let frame = read_frame(&mut cursor).await.unwrap();
        assert_eq!(frame.msg, 217);
        assert_eq!(frame.id, 9001);
        assert_eq!(frame.payload, payload);
    }

    #[tokio::test]
    async fn rejects_oversize_frame() {
        let mut hdr = [0u8; 14];
        hdr[0..4].copy_from_slice(&(u32::MAX).to_be_bytes());
        let mut cursor = std::io::Cursor::new(&hdr[..]);
        let res = read_frame(&mut cursor).await;
        assert!(res.is_err());
    }
}
