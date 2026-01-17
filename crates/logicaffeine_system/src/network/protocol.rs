//! Logos mesh protocol codec for libp2p request-response.
//!
//! Defines the protocol identifier and codec for reading/writing
//! length-prefixed binary frames over libp2p streams.

use async_trait::async_trait;
use futures::prelude::*;
use libp2p::request_response::Codec;
use libp2p::StreamProtocol;
use std::io;

/// The Logos mesh protocol identifier.
pub const LOGOS_PROTOCOL: StreamProtocol = StreamProtocol::new("/logos/mesh/1.0.0");

/// Maximum message size (16 MB).
pub const MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024;

/// Codec for encoding/decoding Logos mesh messages.
///
/// Uses length-prefixed framing: 4-byte big-endian length followed by payload.
#[derive(Debug, Clone, Default)]
pub struct LogosCodec;

/// A request on the Logos mesh (bincode-encoded bytes).
#[derive(Debug, Clone)]
pub struct LogosRequest(pub Vec<u8>);

/// A response on the Logos mesh (bincode-encoded bytes).
#[derive(Debug, Clone)]
pub struct LogosResponse(pub Vec<u8>);

#[async_trait]
impl Codec for LogosCodec {
    type Protocol = StreamProtocol;
    type Request = LogosRequest;
    type Response = LogosResponse;

    async fn read_request<T>(&mut self, _: &Self::Protocol, io: &mut T) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        let bytes = read_length_prefixed(io, MAX_MESSAGE_SIZE).await?;
        Ok(LogosRequest(bytes))
    }

    async fn read_response<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
    ) -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send,
    {
        let bytes = read_length_prefixed(io, MAX_MESSAGE_SIZE).await?;
        Ok(LogosResponse(bytes))
    }

    async fn write_request<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
        req: Self::Request,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        write_length_prefixed(io, &req.0).await
    }

    async fn write_response<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
        res: Self::Response,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        write_length_prefixed(io, &res.0).await
    }
}

/// Read a length-prefixed message from the stream.
async fn read_length_prefixed<T>(io: &mut T, max_size: usize) -> io::Result<Vec<u8>>
where
    T: AsyncRead + Unpin,
{
    let mut len_buf = [0u8; 4];
    io.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;

    if len > max_size {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Message too large: {} > {}", len, max_size),
        ));
    }

    let mut buf = vec![0u8; len];
    io.read_exact(&mut buf).await?;
    Ok(buf)
}

/// Write a length-prefixed message to the stream.
async fn write_length_prefixed<T>(io: &mut T, data: &[u8]) -> io::Result<()>
where
    T: AsyncWrite + Unpin,
{
    let len = data.len() as u32;
    io.write_all(&len.to_be_bytes()).await?;
    io.write_all(data).await?;
    io.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_protocol_name() {
        assert_eq!(LOGOS_PROTOCOL.as_ref(), "/logos/mesh/1.0.0");
    }
}
