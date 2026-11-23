//! Frames: a 4-byte little-endian length, then that many bytes of postcard.
//!
//! The length is checked against the caller's limit before anything is
//! allocated, and a frame must decode exactly, with no bytes left over.

use std::io;

use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

pub async fn write_frame<W: AsyncWrite + Unpin + ?Sized>(w: &mut W, body: &[u8], max: usize) -> io::Result<()> {
    if body.is_empty() || body.len() > max {
        return Err(invalid(format!("frame of {} bytes is outside the limit of {max}", body.len())));
    }
    let mut buf = Vec::with_capacity(4 + body.len());
    buf.extend_from_slice(&(body.len() as u32).to_le_bytes());
    buf.extend_from_slice(body);
    w.write_all(&buf).await?;
    w.flush().await
}

pub async fn read_frame<R: AsyncRead + Unpin + ?Sized>(r: &mut R, max: usize) -> io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).await?;
    let len = u32::from_le_bytes(len) as usize;
    if len == 0 || len > max {
        return Err(invalid(format!("frame of {len} bytes is outside the limit of {max}")));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).await?;
    Ok(body)
}

/// Decode exactly one value, refusing trailing bytes.
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> io::Result<T> {
    match postcard::take_from_bytes(bytes) {
        Ok((value, [])) => Ok(value),
        Ok(_) => Err(invalid("trailing bytes after the message")),
        Err(e) => Err(invalid(format!("malformed message: {e}"))),
    }
}

pub async fn send<T: Serialize, W: AsyncWrite + Unpin + ?Sized>(w: &mut W, msg: &T, max: usize) -> io::Result<()> {
    let body = postcard::to_stdvec(msg).map_err(|e| invalid(format!("could not encode: {e}")))?;
    write_frame(w, &body, max).await
}

pub async fn recv<T: DeserializeOwned, R: AsyncRead + Unpin + ?Sized>(r: &mut R, max: usize) -> io::Result<T> {
    decode(&read_frame(r, max).await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::Request;

    #[tokio::test]
    async fn frames_round_trip() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        send(&mut a, &Request::Status, 64).await.unwrap();
        assert_eq!(recv::<Request, _>(&mut b, 64).await.unwrap(), Request::Status);
    }

    #[tokio::test]
    async fn an_oversized_length_is_refused_before_allocating() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        a.write_all(&u32::MAX.to_le_bytes()).await.unwrap();
        let e = read_frame(&mut b, 1024).await.unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn empty_frames_and_oversized_sends_are_refused() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        a.write_all(&0u32.to_le_bytes()).await.unwrap();
        assert!(read_frame(&mut b, 1024).await.is_err());
        assert!(write_frame(&mut a, &[0u8; 65], 64).await.is_err());
    }

    #[test]
    fn trailing_bytes_and_garbage_are_refused() {
        let mut bytes = postcard::to_stdvec(&Request::Status).unwrap();
        bytes.push(0);
        assert!(decode::<Request>(&bytes).is_err());
        assert!(decode::<Request>(&[0xff, 0xff, 0xff]).is_err());
    }
}
