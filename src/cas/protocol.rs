//! CAS TCP protocol implementation
//!
//! Simple binary protocol:
//! [1 byte: command] [4 bytes: length] [data...]

use std::io::{self, Read, Write};
use super::Hash;

/// CAS protocol commands
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CasCommand {
    /// Write data, returns hash
    Write = 0x01,
    /// Read data by hash
    Read = 0x02,
    /// Check if hash exists
    Exists = 0x03,
    /// Ping/keepalive
    Ping = 0x04,
}

impl TryFrom<u8> for CasCommand {
    type Error = io::Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(CasCommand::Write),
            0x02 => Ok(CasCommand::Read),
            0x03 => Ok(CasCommand::Exists),
            0x04 => Ok(CasCommand::Ping),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown command: {}", value),
            )),
        }
    }
}

/// CAS protocol responses
#[derive(Debug, Clone)]
pub enum CasResponse {
    /// Hash of written data
    Hash(Hash),
    /// Data block
    Data(Vec<u8>),
    /// Existence check result
    Exists(bool),
    /// Pong response
    Pong,
    /// Error response
    Error(String),
}

/// Read a frame from the stream
pub fn read_frame<R: Read>(reader: &mut R) -> io::Result<(CasCommand, Vec<u8>)> {
    // Read command byte
    let mut cmd_buf = [0u8; 1];
    reader.read_exact(&mut cmd_buf)?;
    let command = CasCommand::try_from(cmd_buf[0])?;

    // Read length (4 bytes, little-endian)
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf)?;
    let length = u32::from_le_bytes(len_buf) as usize;

    // Read data
    let mut data = vec![0u8; length];
    if length > 0 {
        reader.read_exact(&mut data)?;
    }

    Ok((command, data))
}

/// Write a frame to the stream
pub fn write_frame<W: Write>(
    writer: &mut W,
    command: CasCommand,
    data: &[u8],
) -> io::Result<()> {
    // Write command byte
    writer.write_all(&[command as u8])?;

    // Write length (4 bytes, little-endian)
    let length = data.len() as u32;
    writer.write_all(&length.to_le_bytes())?;

    // Write data
    if !data.is_empty() {
        writer.write_all(data)?;
    }

    writer.flush()
}

/// Write a response to the stream
pub fn write_response<W: Write>(writer: &mut W, response: &CasResponse) -> io::Result<()> {
    match response {
        CasResponse::Hash(hash) => {
            // Command 0x01 (write response), length 32, hash bytes
            write_frame(writer, CasCommand::Write, hash)?;
        }
        CasResponse::Data(data) => {
            // Command 0x02 (read response), length, data bytes
            write_frame(writer, CasCommand::Read, data)?;
        }
        CasResponse::Exists(exists) => {
            // Command 0x03 (exists response), length 1, boolean byte
            write_frame(writer, CasCommand::Exists, &[*exists as u8])?;
        }
        CasResponse::Pong => {
            // Command 0x04 (pong), length 0
            write_frame(writer, CasCommand::Ping, &[])?;
        }
        CasResponse::Error(msg) => {
            // Command 0xFF (error), length, error message bytes
            writer.write_all(&[0xFF])?;
            let msg_bytes = msg.as_bytes();
            let length = msg_bytes.len() as u32;
            writer.write_all(&length.to_le_bytes())?;
            writer.write_all(msg_bytes)?;
            writer.flush()?;
        }
    }
    Ok(())
}
