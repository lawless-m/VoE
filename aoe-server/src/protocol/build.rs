//! AoE response frame building
//!
//! Builds response frames from request headers and response data.

use super::ata::AtaResponse;
use super::types::*;

/// Build an AoE response frame
pub fn build_response(request: &AoeFrame, response: ResponseData) -> Vec<u8> {
    match response {
        ResponseData::Ata(ata_response) => build_ata_response(request, ata_response),
        ResponseData::Config(config) => build_config_response(request, config),
        ResponseData::Error { code } => build_error_response(request, code),
    }
}

/// Response data variants
pub enum ResponseData {
    /// ATA command response
    Ata(AtaResponse),
    /// Config command response
    Config(ConfigResponse),
    /// Error response
    Error { code: u8 },
}

/// Config command response data
pub struct ConfigResponse {
    pub buffer_count: u16,
    pub firmware_version: u16,
    pub sector_count: u8,
    pub config_string: Vec<u8>,
}

/// Build an ATA response frame
fn build_ata_response(request: &AoeFrame, response: AtaResponse) -> Vec<u8> {
    let data_len = response.data.as_ref().map(|d| d.len()).unwrap_or(0);
    let mut frame = Vec::with_capacity(AoeHeader::SIZE + AtaHeader::SIZE + data_len);

    // Ethernet header - swap src/dst MACs
    frame.extend_from_slice(&request.header.src_mac); // dst = original src
    frame.extend_from_slice(&request.header.dst_mac); // src = original dst
    frame.extend_from_slice(&AOE_ETHERTYPE.to_be_bytes());

    // AoE header with response flag set
    let mut flags = request.header.flags;
    flags.response = true;
    flags.error = false;
    frame.push(flags.to_byte(AOE_VERSION));
    frame.push(0); // no error
    frame.extend_from_slice(&request.header.shelf.to_be_bytes());
    frame.push(request.header.slot);
    frame.push(AoeCommand::Ata as u8);
    frame.extend_from_slice(&request.header.tag.to_be_bytes());

    // ATA header
    let ata_flags = if let AoePayload::Ata { header, .. } = &request.payload {
        header.flags
    } else {
        AtaFlags::default()
    };
    frame.push(ata_flags.to_byte());
    frame.push(response.error);
    frame.push(response.sector_count);
    frame.push(response.status);

    // LBA (6 bytes) - echo back from request
    let lba = if let AoePayload::Ata { header, .. } = &request.payload {
        header.lba
    } else {
        0
    };
    frame.push((lba & 0xFF) as u8);
    frame.push(((lba >> 8) & 0xFF) as u8);
    frame.push(((lba >> 16) & 0xFF) as u8);
    frame.push(((lba >> 24) & 0xFF) as u8);
    frame.push(((lba >> 32) & 0xFF) as u8);
    frame.push(((lba >> 40) & 0xFF) as u8);

    // Reserved (2 bytes)
    frame.extend_from_slice(&[0, 0]);

    // Data payload (for read responses)
    if let Some(data) = response.data {
        frame.extend_from_slice(&data);
    }

    frame
}

/// Build a config response frame
fn build_config_response(request: &AoeFrame, response: ConfigResponse) -> Vec<u8> {
    let config_len = response.config_string.len();
    let mut frame = Vec::with_capacity(AoeHeader::SIZE + ConfigHeader::MIN_SIZE + config_len);

    // Ethernet header - swap src/dst MACs
    frame.extend_from_slice(&request.header.src_mac);
    frame.extend_from_slice(&request.header.dst_mac);
    frame.extend_from_slice(&AOE_ETHERTYPE.to_be_bytes());

    // AoE header with response flag set
    let mut flags = request.header.flags;
    flags.response = true;
    flags.error = false;
    frame.push(flags.to_byte(AOE_VERSION));
    frame.push(0); // no error
    frame.extend_from_slice(&request.header.shelf.to_be_bytes());
    frame.push(request.header.slot);
    frame.push(AoeCommand::Config as u8);
    frame.extend_from_slice(&request.header.tag.to_be_bytes());

    // Config header
    frame.extend_from_slice(&response.buffer_count.to_be_bytes());
    frame.extend_from_slice(&response.firmware_version.to_be_bytes());
    frame.push(response.sector_count);
    frame.push((AOE_VERSION << 4) | 0); // AoE version in high nibble, ccmd=0 in response
    frame.extend_from_slice(&(config_len as u16).to_be_bytes());
    frame.extend_from_slice(&response.config_string);

    frame
}

/// Build an error response frame
fn build_error_response(request: &AoeFrame, error_code: u8) -> Vec<u8> {
    let mut frame = Vec::with_capacity(AoeHeader::SIZE);

    // Ethernet header - swap src/dst MACs
    frame.extend_from_slice(&request.header.src_mac);
    frame.extend_from_slice(&request.header.dst_mac);
    frame.extend_from_slice(&AOE_ETHERTYPE.to_be_bytes());

    // AoE header with response and error flags set
    let mut flags = request.header.flags;
    flags.response = true;
    flags.error = true;
    frame.push(flags.to_byte(AOE_VERSION));
    frame.push(error_code);
    frame.extend_from_slice(&request.header.shelf.to_be_bytes());
    frame.push(request.header.slot);
    frame.push(request.header.command as u8);
    frame.extend_from_slice(&request.header.tag.to_be_bytes());

    // For ATA error responses, include minimal ATA header
    if request.header.command == AoeCommand::Ata {
        // ATA flags
        let ata_flags = if let AoePayload::Ata { header, .. } = &request.payload {
            header.flags.to_byte()
        } else {
            0
        };
        frame.push(ata_flags);
        frame.push(ata_error::ABRT); // error register: aborted
        frame.push(0); // sector count
        frame.push(ata_status::ERR | ata_status::DRDY); // status: error + ready

        // LBA (6 bytes)
        let lba = if let AoePayload::Ata { header, .. } = &request.payload {
            header.lba
        } else {
            0
        };
        frame.push((lba & 0xFF) as u8);
        frame.push(((lba >> 8) & 0xFF) as u8);
        frame.push(((lba >> 16) & 0xFF) as u8);
        frame.push(((lba >> 24) & 0xFF) as u8);
        frame.push(((lba >> 32) & 0xFF) as u8);
        frame.push(((lba >> 40) & 0xFF) as u8);

        // Reserved
        frame.extend_from_slice(&[0, 0]);
    }

    frame
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::parse::parse_frame;

    fn make_test_request() -> AoeFrame {
        let mut frame = vec![0u8; AoeHeader::SIZE + AtaHeader::SIZE];

        // Ethernet header
        frame[0..6].copy_from_slice(&[0x00, 0x11, 0x22, 0x33, 0x44, 0x55]);
        frame[6..12].copy_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
        frame[12..14].copy_from_slice(&AOE_ETHERTYPE.to_be_bytes());

        // AoE header (version in low nibble, flags in high nibble)
        frame[14] = 0x01; // version 1, no flags
        frame[15] = 0;
        frame[16..18].copy_from_slice(&1u16.to_be_bytes());
        frame[18] = 0;
        frame[19] = 0; // ATA
        frame[20..24].copy_from_slice(&0x12345678u32.to_be_bytes());

        // ATA header
        frame[24] = 0x40;
        frame[25] = 0;
        frame[26] = 1;
        frame[27] = 0x24;

        parse_frame(&frame).unwrap()
    }

    #[test]
    fn test_build_ata_response() {
        let request = make_test_request();
        let response = AtaResponse {
            status: ata_status::DRDY,
            error: 0,
            sector_count: 1,
            data: Some(vec![0xAA; 512]),
        };

        let frame = build_ata_response(&request, response);

        // Check MACs are swapped
        assert_eq!(&frame[0..6], &[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
        assert_eq!(&frame[6..12], &[0x00, 0x11, 0x22, 0x33, 0x44, 0x55]);

        // Check response flag is set
        assert_eq!(frame[14] & 0x80, 0x80);

        // Check tag is preserved
        let tag = u32::from_be_bytes([frame[20], frame[21], frame[22], frame[23]]);
        assert_eq!(tag, 0x12345678);

        // Check data is included
        assert_eq!(frame.len(), AoeHeader::SIZE + AtaHeader::SIZE + 512);
    }

    #[test]
    fn test_build_error_response() {
        let request = make_test_request();
        let frame = build_error_response(&request, 3); // device unavailable

        // Check error flag is set
        assert_eq!(frame[14] & 0x40, 0x40);

        // Check error code
        assert_eq!(frame[15], 3);
    }
}
