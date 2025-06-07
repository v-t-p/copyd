// Include the generated protocol buffer code
include!(concat!(env!("OUT_DIR"), "/copyd.rs"));

use prost::Message;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use anyhow::{Result, Context};
use std::str::FromStr;
use std::fmt;

/// Length-prefixed message frame format:
/// [4 bytes length][message bytes]
pub struct MessageFramer;

impl MessageFramer {
    pub async fn send_message<W, T>(
        writer: &mut W,
        message: &T,
    ) -> Result<()>
    where
        W: AsyncWriteExt + Unpin,
        T: Message,
    {
        let mut buf = Vec::new();
        message.encode(&mut buf).context("Failed to encode message")?;
        
        let len = buf.len() as u32;
        writer.write_all(&len.to_le_bytes()).await.context("Failed to write length")?;
        writer.write_all(&buf).await.context("Failed to write message")?;
        writer.flush().await.context("Failed to flush writer")?;
        
        Ok(())
    }

    pub async fn receive_message<R, T>(
        reader: &mut R,
    ) -> Result<T>
    where
        R: AsyncReadExt + Unpin,
        T: Message + Default,
    {
        // Read the length prefix
        let mut len_buf = [0u8; 4];
        reader.read_exact(&mut len_buf).await.context("Failed to read length")?;
        let len = u32::from_le_bytes(len_buf) as usize;
        
        // Sanity check: limit message size to 16MB
        if len > 16 * 1024 * 1024 {
            anyhow::bail!("Message too large: {} bytes", len);
        }
        
        // Read the message
        let mut buf = vec![0u8; len];
        reader.read_exact(&mut buf).await.context("Failed to read message")?;
        
        T::decode(buf.as_slice()).context("Failed to decode message")
    }
}

// Convenience functions for common message types
pub async fn send_request<W>(
    writer: &mut W,
    request: &Request,
) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    MessageFramer::send_message(writer, request).await
}

pub async fn receive_request<R>(
    reader: &mut R,
) -> Result<Request>
where
    R: AsyncReadExt + Unpin,
{
    MessageFramer::receive_message(reader).await
}

pub async fn send_response<W>(
    writer: &mut W,
    response: &Response,
) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    MessageFramer::send_message(writer, response).await
}

pub async fn receive_response<R>(
    reader: &mut R,
) -> Result<Response>
where
    R: AsyncReadExt + Unpin,
{
    MessageFramer::receive_message(reader).await
}

impl fmt::Display for VerifyMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl FromStr for VerifyMode {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "none" => Ok(VerifyMode::None),
            "size" => Ok(VerifyMode::Size),
            "md5" => Ok(VerifyMode::Md5),
            "sha256" => Ok(VerifyMode::Sha256),
            _ => Err(anyhow::anyhow!("Invalid verify mode: {}", s)),
        }
    }
}

impl fmt::Display for ExistsAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl FromStr for ExistsAction {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "overwrite" => Ok(ExistsAction::Overwrite),
            "skip" => Ok(ExistsAction::Skip),
            "serial" => Ok(ExistsAction::Serial),
            _ => Err(anyhow::anyhow!("Invalid exists action: {}", s)),
        }
    }
}

impl fmt::Display for CopyEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl FromStr for CopyEngine {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "auto" => Ok(CopyEngine::Auto),
            "io_uring" => Ok(CopyEngine::IoUring),
            "copyfilerange" => Ok(CopyEngine::CopyFileRange),
            "sendfile" => Ok(CopyEngine::Sendfile),
            "reflink" => Ok(CopyEngine::Reflink),
            "readwrite" => Ok(CopyEngine::ReadWrite),
            _ => Err(anyhow::anyhow!("Invalid copy engine: {}", s)),
        }
    }
}
