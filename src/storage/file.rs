//! File-based storage backend
//!
//! Simple implementation that stores data in a regular file.

use super::{BlockStorage, DeviceInfo, StorageResult};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Mutex;

/// File-based block storage
pub struct FileBackend {
    file: Mutex<File>,
    info: DeviceInfo,
}

impl FileBackend {
    /// Open an existing file as a block device
    pub fn open<P: AsRef<Path>>(path: P) -> StorageResult<Self> {
        Self::open_with_options(path, false)
    }

    /// Open or create a file with specified size
    pub fn open_or_create<P: AsRef<Path>>(path: P, size_bytes: u64) -> StorageResult<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path.as_ref())?;

        let metadata = file.metadata()?;
        let current_size = metadata.len();

        // Extend file if needed
        if current_size < size_bytes {
            file.set_len(size_bytes)?;
        }

        let total_sectors = size_bytes / 512;
        let serial = generate_serial(path.as_ref());

        let info = DeviceInfo {
            model: "AoE File Backend".to_string(),
            serial,
            firmware: env!("CARGO_PKG_VERSION").to_string(),
            total_sectors,
            sector_size: 512,
            lba48: true,
        };

        Ok(Self {
            file: Mutex::new(file),
            info,
        })
    }

    /// Open with explicit read-only option
    fn open_with_options<P: AsRef<Path>>(path: P, read_only: bool) -> StorageResult<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(!read_only)
            .open(path.as_ref())?;

        let metadata = file.metadata()?;
        let file_size = metadata.len();
        let total_sectors = file_size / 512;

        let serial = generate_serial(path.as_ref());

        let info = DeviceInfo {
            model: "AoE File Backend".to_string(),
            serial,
            firmware: env!("CARGO_PKG_VERSION").to_string(),
            total_sectors,
            sector_size: 512,
            lba48: true,
        };

        Ok(Self {
            file: Mutex::new(file),
            info,
        })
    }

    /// Open as read-only
    pub fn open_read_only<P: AsRef<Path>>(path: P) -> StorageResult<Self> {
        Self::open_with_options(path, true)
    }
}

impl BlockStorage for FileBackend {
    fn read(&self, lba: u64, count: u8) -> StorageResult<Vec<u8>> {
        self.validate_range(lba, count)?;

        let offset = lba * self.info.sector_size as u64;
        let length = count as usize * self.info.sector_size as usize;

        let mut file = self.file.lock().unwrap();
        file.seek(SeekFrom::Start(offset))?;

        let mut buffer = vec![0u8; length];
        file.read_exact(&mut buffer)?;

        Ok(buffer)
    }

    fn write(&mut self, lba: u64, data: &[u8]) -> StorageResult<()> {
        let count = (data.len() / self.info.sector_size as usize) as u8;
        self.validate_range(lba, count)?;

        let offset = lba * self.info.sector_size as u64;

        let mut file = self.file.lock().unwrap();
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(data)?;

        Ok(())
    }

    fn flush(&mut self) -> StorageResult<()> {
        let file = self.file.lock().unwrap();
        file.sync_all()?;
        Ok(())
    }

    fn info(&self) -> &DeviceInfo {
        &self.info
    }
}

/// Generate a serial number from file path
fn generate_serial(path: &Path) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    format!("{:016X}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageError;
    use tempfile::NamedTempFile;

    #[test]
    fn test_file_backend_create() {
        let temp = NamedTempFile::new().unwrap();
        let path = temp.path();

        let backend = FileBackend::open_or_create(path, 1024 * 1024).unwrap();
        assert_eq!(backend.info().total_sectors, 2048); // 1MB / 512
    }

    #[test]
    fn test_file_backend_read_write() {
        let temp = NamedTempFile::new().unwrap();
        let path = temp.path();

        let mut backend = FileBackend::open_or_create(path, 1024 * 1024).unwrap();

        // Write some data
        let write_data = vec![0xAA; 512];
        backend.write(0, &write_data).unwrap();

        // Read it back
        let read_data = backend.read(0, 1).unwrap();
        assert_eq!(read_data, write_data);
    }

    #[test]
    fn test_file_backend_multiple_sectors() {
        let temp = NamedTempFile::new().unwrap();
        let path = temp.path();

        let mut backend = FileBackend::open_or_create(path, 1024 * 1024).unwrap();

        // Write 4 sectors
        let mut write_data = Vec::new();
        for i in 0..4 {
            write_data.extend(vec![i as u8; 512]);
        }
        backend.write(10, &write_data).unwrap();

        // Read them back
        let read_data = backend.read(10, 4).unwrap();
        assert_eq!(read_data, write_data);
    }

    #[test]
    fn test_file_backend_out_of_range() {
        let temp = NamedTempFile::new().unwrap();
        let path = temp.path();

        let backend = FileBackend::open_or_create(path, 512 * 10).unwrap(); // 10 sectors

        // Try to read beyond end
        let result = backend.read(9, 2);
        assert!(matches!(result, Err(StorageError::OutOfRange { .. })));
    }
}
