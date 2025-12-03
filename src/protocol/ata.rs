//! ATA command handling
//!
//! Dispatches ATA commands to storage backends and builds responses.

use super::types::*;
use crate::storage::{BlockStorage, DeviceInfo};

/// ATA command response
#[derive(Debug)]
pub struct AtaResponse {
    /// Status register
    pub status: u8,
    /// Error register
    pub error: u8,
    /// Sector count (echoed or remaining)
    pub sector_count: u8,
    /// Data payload (for reads)
    pub data: Option<Vec<u8>>,
}

impl AtaResponse {
    /// Create a successful response with data
    pub fn success_with_data(data: Vec<u8>, sector_count: u8) -> Self {
        Self {
            status: ata_status::DRDY,
            error: 0,
            sector_count,
            data: Some(data),
        }
    }

    /// Create a successful response without data
    pub fn success() -> Self {
        Self {
            status: ata_status::DRDY,
            error: 0,
            sector_count: 0,
            data: None,
        }
    }

    /// Create an error response
    pub fn error(error_code: u8) -> Self {
        Self {
            status: ata_status::ERR | ata_status::DRDY,
            error: error_code,
            sector_count: 0,
            data: None,
        }
    }
}

/// Handle an ATA command
pub fn handle_ata_command(
    storage: &mut dyn BlockStorage,
    header: &AtaHeader,
    data: &[u8],
) -> AtaResponse {
    let cmd = match AtaCommand::try_from(header.cmd_status) {
        Ok(cmd) => cmd,
        Err(_) => {
            log::warn!("Unknown ATA command: 0x{:02X}", header.cmd_status);
            return AtaResponse::error(ata_error::ABRT);
        }
    };

    log::debug!(
        "ATA command: {} LBA={} count={}",
        cmd,
        header.lba,
        header.sector_count
    );

    match cmd {
        AtaCommand::ReadSectors | AtaCommand::ReadSectorsExt => {
            handle_read(storage, header)
        }
        AtaCommand::WriteSectors | AtaCommand::WriteSectorsExt => {
            handle_write(storage, header, data)
        }
        AtaCommand::IdentifyDevice => handle_identify(storage),
        AtaCommand::FlushCache | AtaCommand::FlushCacheExt => {
            handle_flush(storage)
        }
    }
}

/// Handle READ SECTORS command
fn handle_read(storage: &dyn BlockStorage, header: &AtaHeader) -> AtaResponse {
    let lba = if header.flags.extended {
        header.lba48()
    } else {
        header.lba28() as u64
    };

    let count = if header.sector_count == 0 {
        // 0 means 256 sectors for LBA28, or use extended count for LBA48
        if header.flags.extended {
            256
        } else {
            256
        }
    } else {
        header.sector_count as u16
    };

    // Validate range
    let info = storage.info();
    if lba + count as u64 > info.total_sectors {
        log::warn!(
            "Read beyond end: LBA {} + {} > {}",
            lba,
            count,
            info.total_sectors
        );
        return AtaResponse::error(ata_error::IDNF);
    }

    // Perform read
    match storage.read(lba, count as u8) {
        Ok(data) => AtaResponse::success_with_data(data, header.sector_count),
        Err(e) => {
            log::error!("Read error at LBA {}: {}", lba, e);
            AtaResponse::error(ata_error::UNC)
        }
    }
}

/// Handle WRITE SECTORS command
fn handle_write(
    storage: &mut dyn BlockStorage,
    header: &AtaHeader,
    data: &[u8],
) -> AtaResponse {
    let lba = if header.flags.extended {
        header.lba48()
    } else {
        header.lba28() as u64
    };

    let count = if header.sector_count == 0 { 256 } else { header.sector_count as u16 };
    let expected_len = count as usize * SECTOR_SIZE;

    if data.len() != expected_len {
        log::warn!(
            "Write data length mismatch: expected {}, got {}",
            expected_len,
            data.len()
        );
        return AtaResponse::error(ata_error::ABRT);
    }

    // Validate range
    let info = storage.info();
    if lba + count as u64 > info.total_sectors {
        log::warn!(
            "Write beyond end: LBA {} + {} > {}",
            lba,
            count,
            info.total_sectors
        );
        return AtaResponse::error(ata_error::IDNF);
    }

    // Perform write
    match storage.write(lba, data) {
        Ok(()) => AtaResponse::success(),
        Err(e) => {
            log::error!("Write error at LBA {}: {}", lba, e);
            AtaResponse::error(ata_error::UNC)
        }
    }
}

/// Handle IDENTIFY DEVICE command
fn handle_identify(storage: &dyn BlockStorage) -> AtaResponse {
    let info = storage.info();
    let data = build_identify_data(info);
    AtaResponse::success_with_data(data, 1)
}

/// Build 512-byte IDENTIFY DEVICE response
fn build_identify_data(info: &DeviceInfo) -> Vec<u8> {
    let mut data = vec![0u8; 512];

    // Word 0: General configuration
    // Bit 15: 0 = ATA device
    // Bit 6: Fixed device
    data[0] = 0x00;
    data[1] = 0x00;

    // Words 10-19: Serial number (20 ASCII chars, space-padded)
    let serial = format!("{:20}", &info.serial[..info.serial.len().min(20)]);
    copy_ata_string(&mut data[20..40], &serial);

    // Words 23-26: Firmware revision (8 ASCII chars)
    let firmware = format!("{:8}", &info.firmware[..info.firmware.len().min(8)]);
    copy_ata_string(&mut data[46..54], &firmware);

    // Words 27-46: Model number (40 ASCII chars)
    let model = format!("{:40}", &info.model[..info.model.len().min(40)]);
    copy_ata_string(&mut data[54..94], &model);

    // Word 47: Max sectors per interrupt (R/W multiple)
    data[94] = 0x00;
    data[95] = 0x01; // 1 sector

    // Word 49: Capabilities
    // Bit 9: LBA supported
    // Bit 8: DMA supported
    data[98] = 0x00;
    data[99] = 0x03; // LBA + DMA

    // Word 53: Field validity
    // Bit 1: Words 64-70 valid
    // Bit 2: Word 88 valid
    data[106] = 0x00;
    data[107] = 0x06;

    // Words 60-61: Total addressable sectors (LBA28)
    let lba28_sectors = info.total_sectors.min(0x0FFF_FFFF) as u32;
    data[120] = (lba28_sectors & 0xFF) as u8;
    data[121] = ((lba28_sectors >> 8) & 0xFF) as u8;
    data[122] = ((lba28_sectors >> 16) & 0xFF) as u8;
    data[123] = ((lba28_sectors >> 24) & 0xFF) as u8;

    // Word 83: Command set supported (2)
    // Bit 10: LBA48 supported
    data[166] = 0x00;
    data[167] = 0x04;

    // Word 86: Command set enabled (2)
    // Bit 10: LBA48 enabled
    data[172] = 0x00;
    data[173] = 0x04;

    // Words 100-103: Total addressable sectors (LBA48)
    if info.lba48 {
        let sectors = info.total_sectors;
        data[200] = (sectors & 0xFF) as u8;
        data[201] = ((sectors >> 8) & 0xFF) as u8;
        data[202] = ((sectors >> 16) & 0xFF) as u8;
        data[203] = ((sectors >> 24) & 0xFF) as u8;
        data[204] = ((sectors >> 32) & 0xFF) as u8;
        data[205] = ((sectors >> 40) & 0xFF) as u8;
        data[206] = 0;
        data[207] = 0;
    }

    // Word 106: Physical/Logical sector size
    // Bit 12: Device logical sector size > 256 words
    // Bits 3:0: 2^X logical sectors per physical sector
    if info.sector_size == 4096 {
        data[212] = 0x00;
        data[213] = 0x10; // 4K logical sectors
    }

    data
}

/// Copy a string to ATA format (word-swapped ASCII)
fn copy_ata_string(dest: &mut [u8], src: &str) {
    let bytes = src.as_bytes();
    for i in (0..dest.len()).step_by(2) {
        if i + 1 < bytes.len() {
            // ATA strings are byte-swapped within each word
            dest[i] = bytes[i + 1];
            dest[i + 1] = bytes[i];
        } else if i < bytes.len() {
            dest[i] = b' ';
            dest[i + 1] = bytes[i];
        } else {
            dest[i] = b' ';
            dest[i + 1] = b' ';
        }
    }
}

/// Handle FLUSH CACHE command
fn handle_flush(storage: &mut dyn BlockStorage) -> AtaResponse {
    match storage.flush() {
        Ok(()) => AtaResponse::success(),
        Err(e) => {
            log::error!("Flush error: {}", e);
            AtaResponse::error(ata_error::ABRT)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_copy_ata_string() {
        let mut dest = [0u8; 8];
        copy_ata_string(&mut dest, "TEST");

        // "TEST" should become "ETTS" (byte-swapped pairs)
        // T=0x54, E=0x45, S=0x53, T=0x54
        // Word 0: bytes[1], bytes[0] = E, T
        // Word 1: bytes[3], bytes[2] = T, S
        assert_eq!(dest[0], b'E');
        assert_eq!(dest[1], b'T');
        assert_eq!(dest[2], b'T');
        assert_eq!(dest[3], b'S');
    }

    #[test]
    fn test_ata_response_success() {
        let resp = AtaResponse::success();
        assert_eq!(resp.status, ata_status::DRDY);
        assert_eq!(resp.error, 0);
        assert!(resp.data.is_none());
    }

    #[test]
    fn test_ata_response_error() {
        let resp = AtaResponse::error(ata_error::ABRT);
        assert_eq!(resp.status, ata_status::ERR | ata_status::DRDY);
        assert_eq!(resp.error, ata_error::ABRT);
    }
}
