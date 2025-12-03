//! AoE protocol types and constants
//!
//! Defines the wire format structures for AoE frames.

use std::fmt;

/// AoE EtherType
pub const AOE_ETHERTYPE: u16 = 0x88A2;

/// AoE protocol version
pub const AOE_VERSION: u8 = 1;

/// Broadcast MAC address
pub const BROADCAST_MAC: [u8; 6] = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];

/// Broadcast shelf address
pub const BROADCAST_SHELF: u16 = 0xFFFF;

/// Broadcast slot address
pub const BROADCAST_SLOT: u8 = 0xFF;

/// AoE command types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AoeCommand {
    Ata = 0,
    Config = 1,
}

impl TryFrom<u8> for AoeCommand {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(AoeCommand::Ata),
            1 => Ok(AoeCommand::Config),
            other => Err(other),
        }
    }
}

/// AoE flags (upper nibble of version/flags byte)
#[derive(Debug, Clone, Copy, Default)]
pub struct AoeFlags {
    /// Response flag (bit 3)
    pub response: bool,
    /// Error flag (bit 2)
    pub error: bool,
}

impl AoeFlags {
    pub fn from_byte(flags_nibble: u8) -> Self {
        // Input is already the upper nibble (shifted right by 4)
        Self {
            response: (flags_nibble & 0x08) != 0,
            error: (flags_nibble & 0x04) != 0,
        }
    }

    pub fn to_byte(&self, version: u8) -> u8 {
        let mut flags_nibble = 0u8;
        if self.response {
            flags_nibble |= 0x08; // Bit 3 of flags nibble
        }
        if self.error {
            flags_nibble |= 0x04; // Bit 2 of flags nibble
        }
        // Put flags in upper nibble, version in lower nibble
        (flags_nibble << 4) | (version & 0x0F)
    }
}

/// Common AoE header (after Ethernet header)
#[derive(Debug, Clone)]
pub struct AoeHeader {
    /// Destination MAC address
    pub dst_mac: [u8; 6],
    /// Source MAC address
    pub src_mac: [u8; 6],
    /// Protocol version (should be 1)
    pub version: u8,
    /// Flags
    pub flags: AoeFlags,
    /// Error code (valid when flags.error is true)
    pub error: u8,
    /// Shelf (major) address
    pub shelf: u16,
    /// Slot (minor) address
    pub slot: u8,
    /// Command type
    pub command: AoeCommand,
    /// Tag for request/response correlation
    pub tag: u32,
}

impl AoeHeader {
    /// Total size of Ethernet + AoE common header
    pub const SIZE: usize = 24;

    /// Check if this header addresses a specific target
    pub fn addresses_target(&self, shelf: u16, slot: u8) -> bool {
        let shelf_match = self.shelf == shelf || self.shelf == BROADCAST_SHELF;
        let slot_match = self.slot == slot || self.slot == BROADCAST_SLOT;
        shelf_match && slot_match
    }
}

/// ATA command flags
#[derive(Debug, Clone, Copy, Default)]
pub struct AtaFlags {
    /// Extended (LBA48) command (bit 6)
    pub extended: bool,
    /// Device/head register flag (bit 5, legacy)
    pub device: bool,
    /// Async write - don't wait for disk (bit 1)
    pub async_write: bool,
    /// Write command - data follows header (bit 0)
    pub write: bool,
}

impl AtaFlags {
    pub fn from_byte(byte: u8) -> Self {
        Self {
            extended: (byte & 0x40) != 0,
            device: (byte & 0x20) != 0,
            async_write: (byte & 0x02) != 0,
            write: (byte & 0x01) != 0,
        }
    }

    pub fn to_byte(&self) -> u8 {
        let mut flags = 0u8;
        if self.extended {
            flags |= 0x40;
        }
        if self.device {
            flags |= 0x20;
        }
        if self.async_write {
            flags |= 0x02;
        }
        if self.write {
            flags |= 0x01;
        }
        flags
    }
}

/// Common ATA commands
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AtaCommand {
    ReadSectors = 0x20,
    ReadSectorsExt = 0x24,
    WriteSectors = 0x30,
    WriteSectorsExt = 0x34,
    IdentifyDevice = 0xEC,
    FlushCache = 0xE7,
    FlushCacheExt = 0xEA,
}

impl TryFrom<u8> for AtaCommand {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x20 => Ok(AtaCommand::ReadSectors),
            0x24 => Ok(AtaCommand::ReadSectorsExt),
            0x30 => Ok(AtaCommand::WriteSectors),
            0x34 => Ok(AtaCommand::WriteSectorsExt),
            0xEC => Ok(AtaCommand::IdentifyDevice),
            0xE7 => Ok(AtaCommand::FlushCache),
            0xEA => Ok(AtaCommand::FlushCacheExt),
            other => Err(other),
        }
    }
}

impl fmt::Display for AtaCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AtaCommand::ReadSectors => write!(f, "READ SECTORS"),
            AtaCommand::ReadSectorsExt => write!(f, "READ SECTORS EXT"),
            AtaCommand::WriteSectors => write!(f, "WRITE SECTORS"),
            AtaCommand::WriteSectorsExt => write!(f, "WRITE SECTORS EXT"),
            AtaCommand::IdentifyDevice => write!(f, "IDENTIFY DEVICE"),
            AtaCommand::FlushCache => write!(f, "FLUSH CACHE"),
            AtaCommand::FlushCacheExt => write!(f, "FLUSH CACHE EXT"),
        }
    }
}

/// ATA header (12 bytes after common AoE header)
#[derive(Debug, Clone)]
pub struct AtaHeader {
    /// ATA flags
    pub flags: AtaFlags,
    /// Error (response) / Feature (request) register
    pub err_feature: u8,
    /// Sector count
    pub sector_count: u8,
    /// Command (request) / Status (response) register
    pub cmd_status: u8,
    /// 48-bit LBA address
    pub lba: u64,
}

impl AtaHeader {
    /// Size of ATA header
    pub const SIZE: usize = 12;

    /// Get LBA as 48-bit value
    pub fn lba48(&self) -> u64 {
        self.lba & 0x0000_FFFF_FFFF_FFFF
    }

    /// Get LBA as 28-bit value
    pub fn lba28(&self) -> u32 {
        (self.lba & 0x0FFF_FFFF) as u32
    }
}

/// Config command types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ConfigCommand {
    Read = 0,
    TestExact = 1,
    TestPrefix = 2,
    Set = 3,
    ForceSet = 4,
}

impl TryFrom<u8> for ConfigCommand {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(ConfigCommand::Read),
            1 => Ok(ConfigCommand::TestExact),
            2 => Ok(ConfigCommand::TestPrefix),
            3 => Ok(ConfigCommand::Set),
            4 => Ok(ConfigCommand::ForceSet),
            other => Err(other),
        }
    }
}

/// Config header (8 bytes after common AoE header)
#[derive(Debug, Clone)]
pub struct ConfigHeader {
    /// Buffer count (max data server can handle)
    pub buffer_count: u16,
    /// Firmware version
    pub firmware_version: u16,
    /// Max sectors per ATA command
    pub sector_count: u8,
    /// AoE version (high nibble) and config command (low nibble)
    pub aoe_ccmd: u8,
    /// Config string length
    pub config_len: u16,
    /// Config string data
    pub config_string: Vec<u8>,
}

impl ConfigHeader {
    /// Minimum size of config header (without string)
    pub const MIN_SIZE: usize = 8;

    /// Get config command from aoe_ccmd byte
    pub fn config_command(&self) -> Result<ConfigCommand, u8> {
        ConfigCommand::try_from(self.aoe_ccmd & 0x0F)
    }

    /// Get AoE version from aoe_ccmd byte
    pub fn aoe_version(&self) -> u8 {
        self.aoe_ccmd >> 4
    }
}

/// Parsed AoE frame
#[derive(Debug, Clone)]
pub struct AoeFrame {
    /// Common header
    pub header: AoeHeader,
    /// Command-specific payload
    pub payload: AoePayload,
}

/// Command-specific payload
#[derive(Debug, Clone)]
pub enum AoePayload {
    /// ATA command with optional data
    Ata {
        header: AtaHeader,
        data: Vec<u8>,
    },
    /// Config/Query command
    Config(ConfigHeader),
}

/// ATA status register bits
pub mod ata_status {
    pub const ERR: u8 = 0x01;  // Error
    pub const DRQ: u8 = 0x08;  // Data request
    pub const DF: u8 = 0x20;   // Device fault
    pub const DRDY: u8 = 0x40; // Device ready
    pub const BSY: u8 = 0x80;  // Busy
}

/// ATA error register bits
pub mod ata_error {
    pub const AMNF: u8 = 0x01;  // Address mark not found
    pub const TK0NF: u8 = 0x02; // Track 0 not found
    pub const ABRT: u8 = 0x04;  // Aborted command
    pub const MCR: u8 = 0x08;   // Media change request
    pub const IDNF: u8 = 0x10;  // ID not found
    pub const MC: u8 = 0x20;    // Media changed
    pub const UNC: u8 = 0x40;   // Uncorrectable data error
    pub const BBK: u8 = 0x80;   // Bad block detected
}

/// Maximum sectors per standard Ethernet frame (MTU 1500)
pub const MAX_SECTORS_STANDARD: u8 = 2;

/// Maximum sectors per jumbo frame (MTU 9000)
pub const MAX_SECTORS_JUMBO: u8 = 16;

/// Sector size in bytes
pub const SECTOR_SIZE: usize = 512;
