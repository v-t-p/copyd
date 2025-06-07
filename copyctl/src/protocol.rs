// Include the generated protocol buffer code
include!(concat!(env!("OUT_DIR"), "/copyd.rs"));

use prost::Message;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use anyhow::{Result, Context};

/// Length-prefixed message frame format:
/// [4 bytes length][message bytes]
pub struct MessageFramer;

impl MessageFramer {
    pub async fn send_message<T: Message>(
        writer: &mut (dyn AsyncWriteExt + Unpin),
        message: &T,
    ) -> Result<()> {
        let mut buf = Vec::new();
        message.encode(&mut buf).context("Failed to encode message")?;
        
        let len = buf.len() as u32;
        writer.write_all(&len.to_le_bytes()).await.context("Failed to write length")?;
        writer.write_all(&buf).await.context("Failed to write message")?;
        writer.flush().await.context("Failed to flush writer")?;
        
        Ok(())
    }

    pub async fn receive_message<T: Message + Default>(
        reader: &mut (dyn AsyncReadExt + Unpin),
    ) -> Result<T> {
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
pub async fn send_request(
    writer: &mut (dyn AsyncWriteExt + Unpin),
    request: &Request,
) -> Result<()> {
    MessageFramer::send_message(writer, request).await
}

pub async fn receive_request(
    reader: &mut (dyn AsyncReadExt + Unpin),
) -> Result<Request> {
    MessageFramer::receive_message(reader).await
}

pub async fn send_response(
    writer: &mut (dyn AsyncWriteExt + Unpin),
    response: &Response,
) -> Result<()> {
    MessageFramer::send_message(writer, response).await
}

pub async fn receive_response(
    reader: &mut (dyn AsyncReadExt + Unpin),
) -> Result<Response> {
    MessageFramer::receive_message(reader).await
} 