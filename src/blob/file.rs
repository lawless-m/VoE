//! File-based blob store
//!
//! Stores blobs as files in a directory structure.

use super::{BlobError, BlobResult, BlobStore, Hash};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

/// File-based blob store
///
/// Directory structure:
/// ```text
/// root/
///   ab/
///     ab3f7c9d...  (first 2 chars = subdirectory)
///   cd/
///     cd8e2a1b...
/// ```
pub struct FileBlobStore {
    root: PathBuf,
}

impl FileBlobStore {
    /// Create a new file blob store at the given path
    pub fn new<P: AsRef<Path>>(root: P) -> BlobResult<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;

        Ok(Self { root })
    }

    /// Get the file path for a hash
    fn path_for(&self, hash: &Hash) -> PathBuf {
        let hex = hash.to_hex();
        let (prefix, rest) = hex.split_at(2);
        self.root.join(prefix).join(rest)
    }

    /// Get the directory path for a hash
    fn dir_for(&self, hash: &Hash) -> PathBuf {
        let hex = hash.to_hex();
        let (prefix, _) = hex.split_at(2);
        self.root.join(prefix)
    }
}

impl BlobStore for FileBlobStore {
    fn put(&self, hash: &Hash, data: &[u8]) -> BlobResult<()> {
        let path = self.path_for(hash);

        // Skip if already exists (deduplication)
        if path.exists() {
            return Ok(());
        }

        // Verify hash matches content
        let actual_hash = Hash::from_data(data);
        if actual_hash != *hash {
            return Err(BlobError::Corrupted(format!(
                "hash mismatch: expected {}, got {}",
                hash, actual_hash
            )));
        }

        // Create directory if needed
        let dir = self.dir_for(hash);
        fs::create_dir_all(&dir)?;

        // Write to temp file, then rename for atomicity
        let tmp_path = path.with_extension("tmp");
        {
            let mut file = File::create(&tmp_path)?;
            file.write_all(data)?;
            file.sync_all()?;
        }

        fs::rename(tmp_path, path)?;
        Ok(())
    }

    fn get(&self, hash: &Hash) -> BlobResult<Vec<u8>> {
        let path = self.path_for(hash);

        if !path.exists() {
            return Err(BlobError::NotFound(hash.to_hex()));
        }

        let data = fs::read(&path)?;

        // Verify integrity
        let actual_hash = Hash::from_data(&data);
        if actual_hash != *hash {
            return Err(BlobError::Corrupted(hash.to_hex()));
        }

        Ok(data)
    }

    fn exists(&self, hash: &Hash) -> BlobResult<bool> {
        Ok(self.path_for(hash).exists())
    }

    fn delete(&self, hash: &Hash) -> BlobResult<()> {
        let path = self.path_for(hash);
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    fn sync(&self) -> BlobResult<()> {
        // Files are synced on write, nothing to do
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_file_blob_store_put_get() {
        let temp = TempDir::new().unwrap();
        let store = FileBlobStore::new(temp.path()).unwrap();

        let data = b"hello world";
        let hash = Hash::from_data(data);

        // Put
        store.put(&hash, data).unwrap();
        assert!(store.exists(&hash).unwrap());

        // Get
        let retrieved = store.get(&hash).unwrap();
        assert_eq!(retrieved, data);
    }

    #[test]
    fn test_file_blob_store_dedup() {
        let temp = TempDir::new().unwrap();
        let store = FileBlobStore::new(temp.path()).unwrap();

        let data = b"duplicate data";
        let hash = Hash::from_data(data);

        // Put twice
        store.put(&hash, data).unwrap();
        store.put(&hash, data).unwrap();

        // Should still work
        let retrieved = store.get(&hash).unwrap();
        assert_eq!(retrieved, data);
    }

    #[test]
    fn test_file_blob_store_not_found() {
        let temp = TempDir::new().unwrap();
        let store = FileBlobStore::new(temp.path()).unwrap();

        let hash = Hash::from_data(b"nonexistent");
        let result = store.get(&hash);
        assert!(matches!(result, Err(BlobError::NotFound(_))));
    }

    #[test]
    fn test_file_blob_store_hash_mismatch() {
        let temp = TempDir::new().unwrap();
        let store = FileBlobStore::new(temp.path()).unwrap();

        let data = b"actual data";
        let wrong_hash = Hash::from_data(b"different data");

        let result = store.put(&wrong_hash, data);
        assert!(matches!(result, Err(BlobError::Corrupted(_))));
    }

    #[test]
    fn test_file_blob_store_delete() {
        let temp = TempDir::new().unwrap();
        let store = FileBlobStore::new(temp.path()).unwrap();

        let data = b"to be deleted";
        let hash = Hash::from_data(data);

        store.put(&hash, data).unwrap();
        assert!(store.exists(&hash).unwrap());

        store.delete(&hash).unwrap();
        assert!(!store.exists(&hash).unwrap());
    }
}
