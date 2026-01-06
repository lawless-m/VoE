//! SCSI command handling
//!
//! Implements essential SCSI commands for block device operations

use std::io;

/// SCSI opcodes
pub mod opcodes {
    pub const TEST_UNIT_READY: u8 = 0x00;
    pub const INQUIRY: u8 = 0x12;
    pub const MODE_SENSE_6: u8 = 0x1a;
    pub const MODE_SENSE_10: u8 = 0x5a;
    pub const READ_CAPACITY_10: u8 = 0x25;
    pub const READ_CAPACITY_16: u8 = 0x9e; // Service action 0x10
    pub const READ_10: u8 = 0x28;
    pub const READ_16: u8 = 0x88;
    pub const WRITE_10: u8 = 0x2a;
    pub const WRITE_16: u8 = 0x8a;
    pub const REPORT_LUNS: u8 = 0xa0;
}

/// Generate SCSI INQUIRY response
pub fn handle_inquiry(evpd: bool, page_code: u8) -> Vec<u8> {
    if evpd {
        // Vital Product Data pages
        match page_code {
            0x00 => {
                // Supported VPD pages
                vec![
                    0x00, // Peripheral qualifier, device type (direct access)
                    0x00, // Page code
                    0x00, 0x03, // Page length
                    0x00, 0x80, 0x83, // Supported pages
                ]
            }
            0x80 => {
                // Unit serial number
                let serial = b"VoE-CAS-001";
                let mut response = vec![
                    0x00, // Device type
                    0x80, // Page code
                    0x00,
                    serial.len() as u8, // Page length
                ];
                response.extend_from_slice(serial);
                response
            }
            0x83 => {
                // Device identification
                vec![
                    0x00, // Device type
                    0x83, // Page code
                    0x00, 0x00, // Page length (TODO: implement properly)
                ]
            }
            _ => {
                // Unsupported page
                vec![]
            }
        }
    } else {
        // Standard INQUIRY
        let mut response = vec![
            0x00, // Peripheral device type: Direct access block device
            0x00, // Removable: no
            0x05, // Version: SPC-3
            0x02, // Response data format: 2
            0x5b, // Additional length (91 bytes total)
            0x00, // SCCS: no
            0x00, // ACC: no
            0x00, // TPGS: no
        ];

        // Vendor identification (8 bytes)
        response.extend_from_slice(b"VoE     ");

        // Product identification (16 bytes)
        response.extend_from_slice(b"CAS Block Device");

        // Product revision (4 bytes)
        response.extend_from_slice(b"1.0 ");

        // Pad to 96 bytes
        response.resize(96, 0);

        response
    }
}

/// Generate SCSI READ CAPACITY (10) response
pub fn handle_read_capacity_10(total_sectors: u64) -> Vec<u8> {
    let max_lba = if total_sectors > 0xffffffff {
        0xffffffff_u32
    } else {
        (total_sectors - 1) as u32
    };

    let block_length = 512_u32;

    let mut response = Vec::with_capacity(8);
    response.extend_from_slice(&max_lba.to_be_bytes());
    response.extend_from_slice(&block_length.to_be_bytes());

    response
}

/// Generate SCSI READ CAPACITY (16) response
pub fn handle_read_capacity_16(total_sectors: u64) -> Vec<u8> {
    let max_lba = if total_sectors > 0 {
        total_sectors - 1
    } else {
        0
    };

    let block_length = 512_u32;

    let mut response = Vec::with_capacity(32);
    response.extend_from_slice(&max_lba.to_be_bytes());
    response.extend_from_slice(&block_length.to_be_bytes());

    // Pad to 32 bytes
    response.resize(32, 0);

    response
}

/// Generate SCSI MODE SENSE response
pub fn handle_mode_sense() -> Vec<u8> {
    vec![
        0x00, 0x06, 0x00, 0x00, // Mode parameter header
        0x00, 0x00, 0x00, 0x00, // Block descriptor (empty)
    ]
}

/// Generate SCSI REPORT LUNS response
pub fn handle_report_luns() -> Vec<u8> {
    // Report single LUN 0
    vec![
        0x00, 0x00, 0x00, 0x08, // LUN list length (8 bytes)
        0x00, 0x00, 0x00, 0x00, // Reserved
        0x00, 0x00, 0x00, 0x00, // LUN 0
        0x00, 0x00, 0x00, 0x00, // (8 bytes total)
    ]
}

/// Parse SCSI CDB and extract LBA and transfer length
pub fn parse_read_write_cdb(cdb: &[u8]) -> io::Result<(u64, u32)> {
    if cdb.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "empty CDB"));
    }

    match cdb[0] {
        opcodes::READ_10 | opcodes::WRITE_10 => {
            if cdb.len() < 10 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "CDB too short"));
            }
            let lba = u32::from_be_bytes([cdb[2], cdb[3], cdb[4], cdb[5]]) as u64;
            let transfer_length = u16::from_be_bytes([cdb[7], cdb[8]]) as u32;
            Ok((lba, transfer_length))
        }
        opcodes::READ_16 | opcodes::WRITE_16 => {
            if cdb.len() < 16 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "CDB too short"));
            }
            let lba = u64::from_be_bytes([
                cdb[2], cdb[3], cdb[4], cdb[5], cdb[6], cdb[7], cdb[8], cdb[9],
            ]);
            let transfer_length = u32::from_be_bytes([cdb[10], cdb[11], cdb[12], cdb[13]]);
            Ok((lba, transfer_length))
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported opcode: 0x{:02x}", cdb[0]),
        )),
    }
}
