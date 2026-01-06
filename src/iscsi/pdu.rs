//! iSCSI Protocol Data Unit (PDU) definitions
//!
//! Based on RFC 3720

use std::io::{self, Read, Write};
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};

/// iSCSI Opcode
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opcode {
    // Initiator opcodes
    Nop = 0x00,
    ScsiCommand = 0x01,
    ScsiTaskManagement = 0x02,
    LoginRequest = 0x03,
    TextRequest = 0x04,
    ScsiDataOut = 0x05,
    LogoutRequest = 0x06,

    // Target opcodes
    NopIn = 0x20,
    ScsiResponse = 0x21,
    ScsiTaskManagementResponse = 0x22,
    LoginResponse = 0x23,
    TextResponse = 0x24,
    ScsiDataIn = 0x25,
    LogoutResponse = 0x26,
    R2T = 0x31, // Ready To Transfer
    AsyncMessage = 0x32,
    Reject = 0x3f,
}

impl Opcode {
    pub fn from_byte(byte: u8) -> Result<Self, io::Error> {
        match byte & 0x3f {
            0x00 => Ok(Opcode::Nop),
            0x01 => Ok(Opcode::ScsiCommand),
            0x02 => Ok(Opcode::ScsiTaskManagement),
            0x03 => Ok(Opcode::LoginRequest),
            0x04 => Ok(Opcode::TextRequest),
            0x05 => Ok(Opcode::ScsiDataOut),
            0x06 => Ok(Opcode::LogoutRequest),
            0x20 => Ok(Opcode::NopIn),
            0x21 => Ok(Opcode::ScsiResponse),
            0x22 => Ok(Opcode::ScsiTaskManagementResponse),
            0x23 => Ok(Opcode::LoginResponse),
            0x24 => Ok(Opcode::TextResponse),
            0x25 => Ok(Opcode::ScsiDataIn),
            0x26 => Ok(Opcode::LogoutResponse),
            0x31 => Ok(Opcode::R2T),
            0x32 => Ok(Opcode::AsyncMessage),
            0x3f => Ok(Opcode::Reject),
            _ => Err(io::Error::new(io::ErrorKind::InvalidData, "invalid opcode")),
        }
    }
}

/// Basic Header Segment (BHS) - 48 bytes
#[derive(Debug, Clone)]
pub struct BasicHeaderSegment {
    pub opcode: u8,
    pub flags: u8,
    pub total_ahs_length: u8, // in 4-byte words
    pub data_segment_length: u32, // in bytes (24-bit field)
    pub lun: u64,
    pub initiator_task_tag: u32,
    pub target_transfer_tag: u32,
    pub cmd_sn: u32,
    pub exp_stat_sn: u32,
    pub max_cmd_sn: u32,
    pub specific: [u8; 12], // Opcode-specific fields
}

impl BasicHeaderSegment {
    pub fn new(opcode: Opcode) -> Self {
        Self {
            opcode: opcode as u8,
            flags: 0,
            total_ahs_length: 0,
            data_segment_length: 0,
            lun: 0,
            initiator_task_tag: 0xffffffff,
            target_transfer_tag: 0xffffffff,
            cmd_sn: 0,
            exp_stat_sn: 0,
            max_cmd_sn: 0,
            specific: [0; 12],
        }
    }

    pub fn read<R: Read>(reader: &mut R) -> io::Result<Self> {
        let opcode = reader.read_u8()?;
        let flags = reader.read_u8()?;

        // Read opcode-specific 2 bytes (varies by opcode)
        let spec_byte1 = reader.read_u8()?;
        let spec_byte2 = reader.read_u8()?;

        // Data segment length is 24-bit
        let dsl_high = reader.read_u8()?;
        let dsl_mid = reader.read_u8()?;
        let dsl_low = reader.read_u8()?;
        let data_segment_length = ((dsl_high as u32) << 16) | ((dsl_mid as u32) << 8) | (dsl_low as u32);

        let total_ahs_length = reader.read_u8()?;
        let lun = reader.read_u64::<BigEndian>()?;
        let initiator_task_tag = reader.read_u32::<BigEndian>()?;
        let target_transfer_tag = reader.read_u32::<BigEndian>()?;
        let cmd_sn = reader.read_u32::<BigEndian>()?;
        let exp_stat_sn = reader.read_u32::<BigEndian>()?;
        let max_cmd_sn = reader.read_u32::<BigEndian>()?;

        let mut specific = [0u8; 12];
        specific[0] = spec_byte1;
        specific[1] = spec_byte2;
        reader.read_exact(&mut specific[2..])?;

        Ok(Self {
            opcode,
            flags,
            total_ahs_length,
            data_segment_length,
            lun,
            initiator_task_tag,
            target_transfer_tag,
            cmd_sn,
            exp_stat_sn,
            max_cmd_sn,
            specific,
        })
    }

    pub fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_u8(self.opcode)?;
        writer.write_u8(self.flags)?;
        writer.write_u8(self.specific[0])?;
        writer.write_u8(self.specific[1])?;

        // Data segment length (24-bit big-endian)
        writer.write_u8(((self.data_segment_length >> 16) & 0xff) as u8)?;
        writer.write_u8(((self.data_segment_length >> 8) & 0xff) as u8)?;
        writer.write_u8((self.data_segment_length & 0xff) as u8)?;

        writer.write_u8(self.total_ahs_length)?;
        writer.write_u64::<BigEndian>(self.lun)?;
        writer.write_u32::<BigEndian>(self.initiator_task_tag)?;
        writer.write_u32::<BigEndian>(self.target_transfer_tag)?;
        writer.write_u32::<BigEndian>(self.cmd_sn)?;
        writer.write_u32::<BigEndian>(self.exp_stat_sn)?;
        writer.write_u32::<BigEndian>(self.max_cmd_sn)?;
        writer.write_all(&self.specific[2..])?;

        Ok(())
    }
}

/// iSCSI PDU with header and data
#[derive(Debug, Clone)]
pub struct Pdu {
    pub bhs: BasicHeaderSegment,
    pub ahs: Vec<u8>, // Additional Header Segment
    pub data: Vec<u8>,
}

impl Pdu {
    pub fn new(opcode: Opcode) -> Self {
        Self {
            bhs: BasicHeaderSegment::new(opcode),
            ahs: Vec::new(),
            data: Vec::new(),
        }
    }

    pub fn read<R: Read>(reader: &mut R) -> io::Result<Self> {
        let bhs = BasicHeaderSegment::read(reader)?;

        // Read AHS if present
        let ahs_len = bhs.total_ahs_length as usize * 4;
        let mut ahs = vec![0u8; ahs_len];
        if ahs_len > 0 {
            reader.read_exact(&mut ahs)?;
        }

        // Read data segment with padding
        let data_len = bhs.data_segment_length as usize;
        let padded_len = (data_len + 3) & !3; // Round up to 4-byte boundary
        let mut data = vec![0u8; padded_len];
        if padded_len > 0 {
            reader.read_exact(&mut data)?;
        }
        data.truncate(data_len); // Remove padding

        Ok(Self { bhs, ahs, data })
    }

    pub fn write<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        self.bhs.write(writer)?;

        // Write AHS if present
        if !self.ahs.is_empty() {
            writer.write_all(&self.ahs)?;
        }

        // Write data segment with padding
        if !self.data.is_empty() {
            writer.write_all(&self.data)?;

            // Add padding to 4-byte boundary
            let padding = (4 - (self.data.len() % 4)) % 4;
            for _ in 0..padding {
                writer.write_u8(0)?;
            }
        }

        Ok(())
    }

    pub fn opcode(&self) -> Result<Opcode, io::Error> {
        Opcode::from_byte(self.bhs.opcode)
    }
}

/// Login stages
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginStage {
    SecurityNegotiation = 0,
    LoginOperationalNegotiation = 1,
    FullFeaturePhase = 3,
}

/// SCSI status codes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScsiStatus {
    Good = 0x00,
    CheckCondition = 0x02,
    ConditionMet = 0x04,
    Busy = 0x08,
    ReservationConflict = 0x18,
    TaskSetFull = 0x28,
    AcaActive = 0x30,
    TaskAborted = 0x40,
}
