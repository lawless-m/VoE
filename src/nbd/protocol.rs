//! NBD protocol implementation
//!
//! Based on the NBD protocol specification:
//! https://github.com/NetworkBlockDevice/nbd/blob/master/doc/proto.md

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use std::io::{self, Read, Write};

/// NBD magic numbers
pub const NBD_MAGIC: u64 = 0x4e42444d41474943; // "NBDMAGIC"
pub const NBD_OPTS_MAGIC: u64 = 0x49484156454F5054; // "IHAVEOPT"
pub const NBD_REPLY_MAGIC: u64 = 0x3e889045565a9; // Reply magic
pub const NBD_REQUEST_MAGIC: u32 = 0x25609513;
pub const NBD_SIMPLE_REPLY_MAGIC: u32 = 0x67446698;
pub const NBD_OPT_REPLY_MAGIC: u64 = 0x3e889045565a9; // Option reply magic

/// NBD handshake flags
pub const NBD_FLAG_FIXED_NEWSTYLE: u16 = (1 << 0);
pub const NBD_FLAG_NO_ZEROES: u16 = (1 << 1);

/// NBD client flags
pub const NBD_FLAG_C_FIXED_NEWSTYLE: u32 = (1 << 0);
pub const NBD_FLAG_C_NO_ZEROES: u32 = (1 << 1);

/// NBD options
pub const NBD_OPT_EXPORT_NAME: u32 = 1;
pub const NBD_OPT_ABORT: u32 = 2;
pub const NBD_OPT_LIST: u32 = 3;

/// NBD option replies
pub const NBD_REP_ACK: u32 = 1;
pub const NBD_REP_SERVER: u32 = 2;
pub const NBD_REP_ERR_UNSUP: u32 = (1 << 31) | 1;

/// NBD commands
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NbdCommand {
    Read = 0,
    Write = 1,
    Disc = 2, // Disconnect
    Flush = 3,
    Trim = 4,
    WriteZeroes = 6,
}

impl NbdCommand {
    pub fn from_u32(val: u32) -> Option<Self> {
        match val {
            0 => Some(NbdCommand::Read),
            1 => Some(NbdCommand::Write),
            2 => Some(NbdCommand::Disc),
            3 => Some(NbdCommand::Flush),
            4 => Some(NbdCommand::Trim),
            6 => Some(NbdCommand::WriteZeroes),
            _ => None,
        }
    }
}

/// NBD transmission flags
pub const NBD_FLAG_HAS_FLAGS: u16 = (1 << 0);
pub const NBD_FLAG_READ_ONLY: u16 = (1 << 1);
pub const NBD_FLAG_SEND_FLUSH: u16 = (1 << 2);
pub const NBD_FLAG_SEND_TRIM: u16 = (1 << 5);
pub const NBD_FLAG_SEND_WRITE_ZEROES: u16 = (1 << 6);

/// NBD request
#[derive(Debug)]
pub struct NbdRequest {
    pub magic: u32,
    pub command: u32,
    pub handle: u64,
    pub offset: u64,
    pub length: u32,
}

impl NbdRequest {
    pub fn read<R: Read>(reader: &mut R) -> io::Result<Self> {
        let magic = reader.read_u32::<BigEndian>()?;
        if magic != NBD_REQUEST_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid request magic: 0x{:08x}", magic),
            ));
        }

        let command = reader.read_u32::<BigEndian>()?;
        let handle = reader.read_u64::<BigEndian>()?;
        let offset = reader.read_u64::<BigEndian>()?;
        let length = reader.read_u32::<BigEndian>()?;

        Ok(Self {
            magic,
            command,
            handle,
            offset,
            length,
        })
    }

    pub fn command_type(&self) -> Option<NbdCommand> {
        NbdCommand::from_u32(self.command & 0xffff)
    }
}

/// NBD simple reply
pub struct NbdReply {
    pub magic: u32,
    pub error: u32,
    pub handle: u64,
}

impl NbdReply {
    pub fn new(handle: u64, error: u32) -> Self {
        Self {
            magic: NBD_SIMPLE_REPLY_MAGIC,
            error,
            handle,
        }
    }

    pub fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_u32::<BigEndian>(self.magic)?;
        writer.write_u32::<BigEndian>(self.error)?;
        writer.write_u64::<BigEndian>(self.handle)?;
        Ok(())
    }
}

/// Send NBD handshake (oldstyle) - DEPRECATED
#[allow(dead_code)]
pub fn send_handshake_oldstyle<W: Write>(writer: &mut W, size: u64, flags: u16) -> io::Result<()> {
    // Old-style handshake
    writer.write_u64::<BigEndian>(NBD_MAGIC)?;
    writer.write_u64::<BigEndian>(0x00420281861253)?; // Old magic
    writer.write_u64::<BigEndian>(size)?;
    writer.write_u16::<BigEndian>(flags)?;
    writer.write_all(&[0u8; 124])?; // Padding
    writer.flush()?;
    Ok(())
}

/// Send NBD newstyle handshake and handle option negotiation
pub fn send_newstyle_handshake<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    size: u64,
    trans_flags: u16,
) -> io::Result<()> {
    // Send initial greeting
    writer.write_u64::<BigEndian>(NBD_MAGIC)?;
    writer.write_u64::<BigEndian>(NBD_OPTS_MAGIC)?;

    // Server flags: support fixed newstyle
    let handshake_flags = NBD_FLAG_FIXED_NEWSTYLE | NBD_FLAG_NO_ZEROES;
    writer.write_u16::<BigEndian>(handshake_flags)?;
    writer.flush()?;

    // Read client flags
    let client_flags = reader.read_u32::<BigEndian>()?;

    log::debug!("Client flags: 0x{:08x}", client_flags);

    // Negotiate options
    loop {
        // Read option header
        let opts_magic = reader.read_u64::<BigEndian>()?;
        if opts_magic != NBD_OPTS_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid option magic: 0x{:016x}", opts_magic),
            ));
        }

        let option = reader.read_u32::<BigEndian>()?;
        let option_len = reader.read_u32::<BigEndian>()?;

        log::debug!("Option: {}, length: {}", option, option_len);

        match option {
            NBD_OPT_EXPORT_NAME => {
                // Read export name (we ignore it for now)
                let mut export_name = vec![0u8; option_len as usize];
                reader.read_exact(&mut export_name)?;

                log::debug!("Export name: {:?}", String::from_utf8_lossy(&export_name));

                // Send export info (no option reply for EXPORT_NAME)
                writer.write_u64::<BigEndian>(size)?;
                writer.write_u16::<BigEndian>(trans_flags)?;

                // If client supports NO_ZEROES, don't send padding
                if (client_flags & NBD_FLAG_C_NO_ZEROES) == 0 {
                    writer.write_all(&[0u8; 124])?; // Padding
                }

                writer.flush()?;

                // EXPORT_NAME ends negotiation
                return Ok(());
            }

            NBD_OPT_ABORT => {
                // Client wants to abort
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "client aborted connection",
                ));
            }

            _ => {
                // Unsupported option - skip data and send error
                let mut option_data = vec![0u8; option_len as usize];
                reader.read_exact(&mut option_data)?;

                // Send unsupported reply
                writer.write_u64::<BigEndian>(NBD_OPT_REPLY_MAGIC)?;
                writer.write_u32::<BigEndian>(option)?;
                writer.write_u32::<BigEndian>(NBD_REP_ERR_UNSUP)?;
                writer.write_u32::<BigEndian>(0)?; // No reply data
                writer.flush()?;
            }
        }
    }
}
