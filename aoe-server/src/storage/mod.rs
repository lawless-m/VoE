//! Storage backends
//!
//! This module defines the BlockStorage trait and various implementations.

pub mod cas;
pub mod file;

use thiserror::Error;

/// Storage errors
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("LBA out of range: {lba} (max: {max})")]
    OutOfRange { lba: u64, max: u64 },

    #[error("invalid sector count: {0}")]
    InvalidSectorCount(u8),

    #[error("backend error: {0}")]
    Backend(String),

    #[error("device is read-only")]
    ReadOnly,

    #[error("data corruption detected")]
    Corrupted,
}

/// Result type for storage operations
pub type StorageResult<T> = Result<T, StorageError>;

/// Device information for IDENTIFY DEVICE
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    /// Model name (max 40 chars)
    pub model: String,
    /// Serial number (max 20 chars)
    pub serial: String,
    /// Firmware revision (max 8 chars)
    pub firmware: String,
    /// Total sectors (LBA48 max)
    pub total_sectors: u64,
    /// Sector size in bytes (512 or 4096)
    pub sector_size: u32,
    /// LBA48 support
    pub lba48: bool,
}

impl Default for DeviceInfo {
    fn default() -> Self {
        Self {
            model: "AoE Virtual Disk".to_string(),
            serial: "0000000000".to_string(),
            firmware: "1.0".to_string(),
            total_sectors: 0,
            sector_size: 512,
            lba48: true,
        }
    }
}

/// Block storage trait - the core abstraction for storage backends
pub trait BlockStorage: Send + Sync {
    /// Read sectors starting at LBA.
    /// Returns exactly count * sector_size bytes.
    fn read(&self, lba: u64, count: u8) -> StorageResult<Vec<u8>>;

    /// Write sectors starting at LBA.
    /// Data length must equal count * sector_size.
    fn write(&mut self, lba: u64, data: &[u8]) -> StorageResult<()>;

    /// Flush pending writes to stable storage.
    fn flush(&mut self) -> StorageResult<()>;

    /// Device information (size, model, serial, etc.)
    fn info(&self) -> &DeviceInfo;

    /// Validate that a range is within bounds
    fn validate_range(&self, lba: u64, count: u8) -> StorageResult<()> {
        let info = self.info();
        let end_lba = lba + count as u64;
        if end_lba > info.total_sectors {
            return Err(StorageError::OutOfRange {
                lba,
                max: info.total_sectors,
            });
        }
        Ok(())
    }
}

/// Snapshot information
#[derive(Debug, Clone)]
pub struct SnapshotInfo {
    /// Snapshot identifier (usually root hash)
    pub id: String,
    /// Creation timestamp (Unix seconds)
    pub timestamp: u64,
    /// Optional description
    pub description: Option<String>,
}

/// Extended trait for archival storage (CAS backend)
pub trait ArchivalStorage: BlockStorage {
    /// Create snapshot, return identifier (root hash).
    fn snapshot(&mut self, description: Option<&str>) -> StorageResult<String>;

    /// List available snapshots.
    fn list_snapshots(&self) -> StorageResult<Vec<SnapshotInfo>>;

    /// Restore to a snapshot (reads will see that version).
    fn restore(&mut self, snapshot_id: &str) -> StorageResult<()>;
}

// Re-export backends
pub use cas::CasBackend;
pub use file::FileBackend;
