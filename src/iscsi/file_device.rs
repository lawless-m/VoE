//! File-backed SCSI block device
//!
//! Simple backend that reads/writes a flat file or block device directly.

use std::fs::{File, OpenOptions};
use std::os::unix::fs::FileExt;
use std::path::Path;

use iscsi_target::{IscsiError, ScsiBlockDevice, ScsiResult};

const BLOCK_SIZE: u32 = 512;

/// File-backed SCSI block device
pub struct FileScsiDevice {
    file: File,
    capacity_blocks: u64,
    vendor_id: String,
    product_id: String,
    product_rev: String,
}

impl FileScsiDevice {
    /// Open an existing file/device as a SCSI block device
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        let size = file.metadata()?.len();
        let capacity_blocks = size / BLOCK_SIZE as u64;

        log::info!("Opened file device: {:?} ({} MB, {} blocks)",
            path, size / (1024 * 1024), capacity_blocks);

        Ok(Self {
            file,
            capacity_blocks,
            vendor_id: "VoE     ".to_string(),
            product_id: "File Disk       ".to_string(),
            product_rev: "1.0 ".to_string(),
        })
    }

    /// Create a new sparse file of the given size in MB
    pub fn create(path: &Path, size_mb: u64) -> std::io::Result<Self> {
        let size = size_mb * 1024 * 1024;
        let file = OpenOptions::new()
            .read(true).write(true).create(true).truncate(true)
            .open(path)?;
        file.set_len(size)?;
        let capacity_blocks = size / BLOCK_SIZE as u64;

        log::info!("Created file device: {:?} ({} MB, {} blocks)",
            path, size_mb, capacity_blocks);

        Ok(Self {
            file,
            capacity_blocks,
            vendor_id: "VoE     ".to_string(),
            product_id: format!("File Disk {:>5}MB", size_mb),
            product_rev: "1.0 ".to_string(),
        })
    }
}

impl ScsiBlockDevice for FileScsiDevice {
    fn read(&self, lba: u64, blocks: u32, block_size: u32) -> ScsiResult<Vec<u8>> {
        let offset = lba * block_size as u64;
        let len = blocks as usize * block_size as usize;
        let mut buf = vec![0u8; len];
        self.file.read_at(&mut buf, offset)
            .map_err(IscsiError::Io)?;
        Ok(buf)
    }

    fn write(&mut self, lba: u64, data: &[u8], block_size: u32) -> ScsiResult<()> {
        let offset = lba * block_size as u64;
        self.file.write_at(data, offset)
            .map_err(IscsiError::Io)?;
        Ok(())
    }

    fn capacity(&self) -> u64 {
        self.capacity_blocks
    }

    fn block_size(&self) -> u32 {
        BLOCK_SIZE
    }

    fn flush(&mut self) -> ScsiResult<()> {
        self.file.sync_data().map_err(IscsiError::Io)?;
        Ok(())
    }

    fn vendor_id(&self) -> &str {
        &self.vendor_id
    }

    fn product_id(&self) -> &str {
        &self.product_id
    }

    fn product_rev(&self) -> &str {
        &self.product_rev
    }
}
