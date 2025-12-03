//! AoE frame parsing
//!
//! Parses raw Ethernet frames into structured AoE frames.

use super::types::*;
use thiserror::Error;

/// Parsing errors
#[derive(Debug, Error)]
pub enum ParseError {
    #[error("frame too short: expected at least {expected} bytes, got {actual}")]
    TooShort { expected: usize, actual: usize },

    #[error("invalid EtherType: expected 0x{:04X}, got 0x{actual:04X}", AOE_ETHERTYPE)]
    InvalidEtherType { actual: u16 },

    #[error("unsupported version: {0}")]
    UnsupportedVersion(u8),

    #[error("unknown command: {0}")]
    UnknownCommand(u8),

    #[error("invalid ATA header")]
    InvalidAtaHeader,

    #[error("invalid config header")]
    InvalidConfigHeader,
}

/// Parse a raw Ethernet frame into an AoE frame
pub fn parse_frame(data: &[u8]) -> Result<AoeFrame, ParseError> {
    // Minimum size: Ethernet header (14) + AoE header (10) = 24 bytes
    if data.len() < AoeHeader::SIZE {
        return Err(ParseError::TooShort {
            expected: AoeHeader::SIZE,
            actual: data.len(),
        });
    }

    // Parse Ethernet header
    let dst_mac: [u8; 6] = data[0..6].try_into().unwrap();
    let src_mac: [u8; 6] = data[6..12].try_into().unwrap();
    let ethertype = u16::from_be_bytes([data[12], data[13]]);

    if ethertype != AOE_ETHERTYPE {
        return Err(ParseError::InvalidEtherType { actual: ethertype });
    }

    // Parse AoE common header
    let ver_flags = data[14];
    let version = ver_flags & 0x0F;
    let flags = AoeFlags::from_byte(ver_flags >> 4);

    if version != AOE_VERSION {
        return Err(ParseError::UnsupportedVersion(version));
    }

    let error = data[15];
    let shelf = u16::from_be_bytes([data[16], data[17]]);
    let slot = data[18];
    let command_byte = data[19];
    let tag = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);

    let command = AoeCommand::try_from(command_byte)
        .map_err(|_| ParseError::UnknownCommand(command_byte))?;

    let header = AoeHeader {
        dst_mac,
        src_mac,
        version,
        flags,
        error,
        shelf,
        slot,
        command,
        tag,
    };

    // Parse command-specific payload
    let payload = match command {
        AoeCommand::Ata => parse_ata_payload(&data[AoeHeader::SIZE..])?,
        AoeCommand::Config => parse_config_payload(&data[AoeHeader::SIZE..])?,
    };

    Ok(AoeFrame { header, payload })
}

/// Parse ATA command payload
fn parse_ata_payload(data: &[u8]) -> Result<AoePayload, ParseError> {
    if data.len() < AtaHeader::SIZE {
        return Err(ParseError::TooShort {
            expected: AoeHeader::SIZE + AtaHeader::SIZE,
            actual: AoeHeader::SIZE + data.len(),
        });
    }

    let flags = AtaFlags::from_byte(data[0]);
    let err_feature = data[1];
    let sector_count = data[2];
    let cmd_status = data[3];

    // LBA is stored in 6 bytes (LBA0-LBA5)
    let lba = u64::from(data[4])
        | (u64::from(data[5]) << 8)
        | (u64::from(data[6]) << 16)
        | (u64::from(data[7]) << 24)
        | (u64::from(data[8]) << 32)
        | (u64::from(data[9]) << 40);

    // Bytes 10-11 are reserved

    let ata_header = AtaHeader {
        flags,
        err_feature,
        sector_count,
        cmd_status,
        lba,
    };

    // Data follows the ATA header (if write command)
    let payload_data = data[AtaHeader::SIZE..].to_vec();

    Ok(AoePayload::Ata {
        header: ata_header,
        data: payload_data,
    })
}

/// Parse Config command payload
fn parse_config_payload(data: &[u8]) -> Result<AoePayload, ParseError> {
    if data.len() < ConfigHeader::MIN_SIZE {
        return Err(ParseError::TooShort {
            expected: AoeHeader::SIZE + ConfigHeader::MIN_SIZE,
            actual: AoeHeader::SIZE + data.len(),
        });
    }

    let buffer_count = u16::from_be_bytes([data[0], data[1]]);
    let firmware_version = u16::from_be_bytes([data[2], data[3]]);
    let sector_count = data[4];
    let aoe_ccmd = data[5];
    let config_len = u16::from_be_bytes([data[6], data[7]]);

    let config_string = if config_len > 0 {
        let start = ConfigHeader::MIN_SIZE;
        let end = start + config_len as usize;
        if data.len() < end {
            return Err(ParseError::InvalidConfigHeader);
        }
        data[start..end].to_vec()
    } else {
        Vec::new()
    };

    Ok(AoePayload::Config(ConfigHeader {
        buffer_count,
        firmware_version,
        sector_count,
        aoe_ccmd,
        config_len,
        config_string,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_ata_frame() {
        // Construct a minimal ATA read request
        let mut frame = vec![0u8; AoeHeader::SIZE + AtaHeader::SIZE];

        // Ethernet header
        frame[0..6].copy_from_slice(&[0x00, 0x11, 0x22, 0x33, 0x44, 0x55]); // dst
        frame[6..12].copy_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]); // src
        frame[12..14].copy_from_slice(&AOE_ETHERTYPE.to_be_bytes()); // ethertype

        // AoE header (version in low nibble, flags in high nibble)
        frame[14] = 0x01; // version 1, no flags
        frame[15] = 0; // no error
        frame[16..18].copy_from_slice(&1u16.to_be_bytes()); // shelf 1
        frame[18] = 0; // slot 0
        frame[19] = 0; // ATA command
        frame[20..24].copy_from_slice(&0x12345678u32.to_be_bytes()); // tag

        // ATA header
        frame[24] = 0x40; // extended flag
        frame[25] = 0; // feature
        frame[26] = 1; // sector count
        frame[27] = 0x24; // READ SECTORS EXT
        // LBA = 0
        // Reserved bytes already 0

        let result = parse_frame(&frame).unwrap();
        assert_eq!(result.header.shelf, 1);
        assert_eq!(result.header.slot, 0);
        assert_eq!(result.header.tag, 0x12345678);

        if let AoePayload::Ata { header, .. } = result.payload {
            assert!(header.flags.extended);
            assert_eq!(header.sector_count, 1);
            assert_eq!(header.cmd_status, 0x24);
        } else {
            panic!("Expected ATA payload");
        }
    }

    #[test]
    fn test_parse_too_short() {
        let frame = vec![0u8; 10];
        assert!(matches!(
            parse_frame(&frame),
            Err(ParseError::TooShort { .. })
        ));
    }

    #[test]
    fn test_parse_invalid_ethertype() {
        let mut frame = vec![0u8; AoeHeader::SIZE + AtaHeader::SIZE];
        frame[12..14].copy_from_slice(&0x0800u16.to_be_bytes()); // IPv4

        assert!(matches!(
            parse_frame(&frame),
            Err(ParseError::InvalidEtherType { .. })
        ));
    }
}
