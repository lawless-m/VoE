//! CAS storage engine
//!
//! Handles content-addressable storage with xxHash3-128 hashing.

use super::Hash;
use xxhash_rust::xxh3::xxh3_128;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// Content-addressable storage
pub struct CasStorage {
    base_path: PathBuf,
}

impl CasStorage {
    /// Create a new CAS storage at the specified path
    pub fn new<P: AsRef<Path>>(base_path: P) -> io::Result<Self> {
        let base_path = base_path.as_ref().to_path_buf();
        fs::create_dir_all(&base_path)?;
        Ok(Self { base_path })
    }

    /// Write data and return its hash
    pub fn write(&self, data: &[u8]) -> io::Result<Hash> {
        // Calculate hash using xxHash3-128
        let hash_u128 = xxh3_128(data);
        let hash: Hash = hash_u128.to_le_bytes();

        // Write to file (organized in subdirectories by first 2 hex chars)
        let path = self.hash_to_path(&hash);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Only write if doesn't exist (content-addressable = immutable)
        if !path.exists() {
            let mut file = File::create(&path)?;
            file.write_all(data)?;
            file.sync_all()?;
        }

        Ok(hash)
    }

    /// Read data by hash
    pub fn read(&self, hash: &Hash) -> io::Result<Vec<u8>> {
        let path = self.hash_to_path(hash);
        let mut file = File::open(&path)?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)?;
        Ok(data)
    }

    /// Check if hash exists
    pub fn exists(&self, hash: &Hash) -> bool {
        self.hash_to_path(hash).exists()
    }

    /// Delete data by hash
    /// Returns true if the file was deleted, false if it didn't exist
    pub fn delete(&self, hash: &Hash) -> io::Result<bool> {
        let path = self.hash_to_path(hash);
        if path.exists() {
            fs::remove_file(&path)?;
            log::debug!("Deleted CAS block: {}", hex::encode(hash));
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Convert hash to file path (organized as base/XX/YYYYYYYY...)
    fn hash_to_path(&self, hash: &Hash) -> PathBuf {
        let hex = hex::encode(hash);
        let (prefix, suffix) = hex.split_at(2);
        self.base_path.join(prefix).join(suffix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_write_and_read() {
        let temp_dir = TempDir::new().unwrap();
        let storage = CasStorage::new(temp_dir.path()).unwrap();

        let data = b"hello world";
        let hash = storage.write(data).unwrap();

        // Read it back
        let read_data = storage.read(&hash).unwrap();
        assert_eq!(read_data, data);
    }

    #[test]
    fn test_exists() {
        let temp_dir = TempDir::new().unwrap();
        let storage = CasStorage::new(temp_dir.path()).unwrap();

        let data = b"test data";
        let hash = storage.write(data).unwrap();

        assert!(storage.exists(&hash));

        // Non-existent hash
        let fake_hash = [0u8; 16];
        assert!(!storage.exists(&fake_hash));
    }

    #[test]
    fn test_duplicate_write() {
        let temp_dir = TempDir::new().unwrap();
        let storage = CasStorage::new(temp_dir.path()).unwrap();

        let data = b"duplicate test";
        let hash1 = storage.write(data).unwrap();
        let hash2 = storage.write(data).unwrap();

        // Same data = same hash
        assert_eq!(hash1, hash2);
    }
}
