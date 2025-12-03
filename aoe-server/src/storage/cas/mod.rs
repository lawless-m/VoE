//! Content-Addressed Storage backend
//!
//! Implements BlockStorage using a Merkle tree structure with content-addressed
//! block storage. Provides automatic deduplication and snapshot capabilities.

mod snapshot;
mod tree;

pub use snapshot::SnapshotManager;
pub use tree::{calculate_depth, MerkleTree, MerkleTreeMut, BLOCK_SIZE, FANOUT};

use crate::blob::{BlobStore, Hash};
use crate::storage::{
    ArchivalStorage, BlockStorage, DeviceInfo, SnapshotInfo, StorageError, StorageResult,
};
use std::path::Path;
use std::sync::Mutex;

/// Content-Addressed Storage backend
///
/// Uses a Merkle tree to map LBAs to content hashes, with automatic
/// deduplication through the underlying blob store.
pub struct CasBackend {
    /// Blob store for actual data
    blob_store: Box<dyn BlobStore>,
    /// Current root hash
    root_hash: Mutex<Hash>,
    /// Device information
    info: DeviceInfo,
    /// Snapshot manager
    snapshots: Mutex<SnapshotManager>,
    /// Whether to compress data
    compress: bool,
}

impl CasBackend {
    /// Create a new CAS backend
    pub fn new(
        blob_store: Box<dyn BlobStore>,
        total_sectors: u64,
        snapshot_path: &Path,
    ) -> StorageResult<Self> {
        let snapshots = SnapshotManager::new(snapshot_path)
            .map_err(|e| StorageError::Backend(format!("failed to load snapshots: {}", e)))?;

        // Try to load from latest snapshot, or start fresh
        let root_hash = snapshots.latest().unwrap_or(Hash::ZERO);

        let info = DeviceInfo {
            model: "AoE CAS Backend".to_string(),
            serial: format!("{:016X}", hash_path(snapshot_path)),
            firmware: env!("CARGO_PKG_VERSION").to_string(),
            total_sectors,
            sector_size: 512,
            lba48: true,
        };

        Ok(Self {
            blob_store,
            root_hash: Mutex::new(root_hash),
            info,
            snapshots: Mutex::new(snapshots),
            compress: true,
        })
    }

    /// Create with explicit root hash (for restoring)
    pub fn with_root(
        blob_store: Box<dyn BlobStore>,
        total_sectors: u64,
        snapshot_path: &Path,
        root_hash: Hash,
    ) -> StorageResult<Self> {
        let snapshots = SnapshotManager::new(snapshot_path)
            .map_err(|e| StorageError::Backend(format!("failed to load snapshots: {}", e)))?;

        let info = DeviceInfo {
            model: "AoE CAS Backend".to_string(),
            serial: format!("{:016X}", hash_path(snapshot_path)),
            firmware: env!("CARGO_PKG_VERSION").to_string(),
            total_sectors,
            sector_size: 512,
            lba48: true,
        };

        Ok(Self {
            blob_store,
            root_hash: Mutex::new(root_hash),
            info,
            snapshots: Mutex::new(snapshots),
            compress: true,
        })
    }

    /// Store a data block, optionally with compression
    fn store_block(&self, data: &[u8]) -> StorageResult<Hash> {
        // Check for zero block (sparse)
        if data.iter().all(|&b| b == 0) {
            return Ok(Hash::ZERO);
        }

        let (stored_data, hash) = if self.compress {
            let compressed = lz4_flex::compress_prepend_size(data);
            if compressed.len() < data.len() {
                // Compression helped - store compressed with marker
                let mut with_marker = vec![0x01]; // Compressed marker
                with_marker.extend_from_slice(&compressed);
                let hash = Hash::from_data(&with_marker);
                (with_marker, hash)
            } else {
                // Compression didn't help - store uncompressed
                let mut with_marker = vec![0x00]; // Uncompressed marker
                with_marker.extend_from_slice(data);
                let hash = Hash::from_data(&with_marker);
                (with_marker, hash)
            }
        } else {
            let mut with_marker = vec![0x00];
            with_marker.extend_from_slice(data);
            let hash = Hash::from_data(&with_marker);
            (with_marker, hash)
        };

        self.blob_store
            .put(&hash, &stored_data)
            .map_err(|e| StorageError::Backend(e.to_string()))?;

        Ok(hash)
    }

    /// Retrieve a data block, decompressing if needed
    fn retrieve_block(&self, hash: &Hash) -> StorageResult<Vec<u8>> {
        if hash.is_zero() {
            // Sparse block - return zeros
            return Ok(vec![0u8; 512]);
        }

        let stored_data = self
            .blob_store
            .get(hash)
            .map_err(|e| StorageError::Backend(e.to_string()))?;

        if stored_data.is_empty() {
            return Err(StorageError::Corrupted);
        }

        let marker = stored_data[0];
        let payload = &stored_data[1..];

        match marker {
            0x00 => {
                // Uncompressed
                Ok(payload.to_vec())
            }
            0x01 => {
                // Compressed
                lz4_flex::decompress_size_prepended(payload)
                    .map_err(|_| StorageError::Corrupted)
            }
            _ => Err(StorageError::Corrupted),
        }
    }
}

impl BlockStorage for CasBackend {
    fn read(&self, lba: u64, count: u8) -> StorageResult<Vec<u8>> {
        self.validate_range(lba, count)?;

        let root_hash = *self.root_hash.lock().unwrap();
        let tree = MerkleTree::new(self.blob_store.as_ref(), root_hash, self.info.total_sectors);

        let mut result = Vec::with_capacity(count as usize * 512);

        for i in 0..count as u64 {
            let data_hash = tree
                .lookup(lba + i)
                .map_err(|e| StorageError::Backend(e.to_string()))?;

            let block = self.retrieve_block(&data_hash)?;
            result.extend_from_slice(&block);
        }

        Ok(result)
    }

    fn write(&mut self, lba: u64, data: &[u8]) -> StorageResult<()> {
        let count = (data.len() / 512) as u8;
        self.validate_range(lba, count)?;

        let mut root_hash = self.root_hash.lock().unwrap();
        let mut tree =
            MerkleTreeMut::new(self.blob_store.as_ref(), *root_hash, self.info.total_sectors);

        for (i, chunk) in data.chunks(512).enumerate() {
            let data_hash = self.store_block(chunk)?;
            tree.update(lba + i as u64, data_hash)
                .map_err(|e| StorageError::Backend(e.to_string()))?;
        }

        *root_hash = tree.root_hash();
        Ok(())
    }

    fn flush(&mut self) -> StorageResult<()> {
        self.blob_store
            .sync()
            .map_err(|e| StorageError::Backend(e.to_string()))
    }

    fn info(&self) -> &DeviceInfo {
        &self.info
    }
}

impl ArchivalStorage for CasBackend {
    fn snapshot(&mut self, description: Option<&str>) -> StorageResult<String> {
        let root_hash = *self.root_hash.lock().unwrap();
        let mut snapshots = self.snapshots.lock().unwrap();

        snapshots
            .create(root_hash, description)
            .map_err(|e| StorageError::Backend(format!("failed to create snapshot: {}", e)))
    }

    fn list_snapshots(&self) -> StorageResult<Vec<SnapshotInfo>> {
        let snapshots = self.snapshots.lock().unwrap();
        Ok(snapshots.list())
    }

    fn restore(&mut self, snapshot_id: &str) -> StorageResult<()> {
        let snapshots = self.snapshots.lock().unwrap();
        let hash = snapshots
            .get(snapshot_id)
            .ok_or_else(|| StorageError::Backend(format!("snapshot not found: {}", snapshot_id)))?;

        let mut root_hash = self.root_hash.lock().unwrap();
        *root_hash = hash;
        Ok(())
    }
}

/// Hash a path for generating serial numbers
fn hash_path(path: &Path) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blob::FileBlobStore;
    use tempfile::TempDir;

    fn create_test_backend() -> (TempDir, CasBackend) {
        let temp = TempDir::new().unwrap();
        let blob_path = temp.path().join("blobs");
        let snapshot_path = temp.path().join("snapshots.json");

        let store = Box::new(FileBlobStore::new(&blob_path).unwrap());
        let backend = CasBackend::new(store, 1024, &snapshot_path).unwrap();

        (temp, backend)
    }

    #[test]
    fn test_cas_read_write() {
        let (_temp, mut backend) = create_test_backend();

        // Write a sector
        let write_data = vec![0xAA; 512];
        backend.write(0, &write_data).unwrap();

        // Read it back
        let read_data = backend.read(0, 1).unwrap();
        assert_eq!(read_data, write_data);
    }

    #[test]
    fn test_cas_sparse_read() {
        let (_temp, backend) = create_test_backend();

        // Read unwritten sector - should be zeros
        let data = backend.read(100, 1).unwrap();
        assert_eq!(data, vec![0u8; 512]);
    }

    #[test]
    fn test_cas_multiple_sectors() {
        let (_temp, mut backend) = create_test_backend();

        // Write multiple sectors
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
    fn test_cas_deduplication() {
        let temp = TempDir::new().unwrap();
        let blob_path = temp.path().join("blobs");
        let snapshot_path = temp.path().join("snapshots.json");

        let store = Box::new(FileBlobStore::new(&blob_path).unwrap());
        let mut backend = CasBackend::new(store, 1024, &snapshot_path).unwrap();

        // Write same data to two different locations
        let data = vec![0xBB; 512];
        backend.write(0, &data).unwrap();
        backend.write(100, &data).unwrap();

        // Both should read back correctly
        assert_eq!(backend.read(0, 1).unwrap(), data);
        assert_eq!(backend.read(100, 1).unwrap(), data);

        // The blob store should only have one copy (plus tree nodes)
        // We can't easily verify this without exposing internals,
        // but the implementation uses content-addressing
    }

    #[test]
    fn test_cas_snapshots() {
        let (_temp, mut backend) = create_test_backend();

        // Write initial data
        backend.write(0, &vec![0x11; 512]).unwrap();
        let snap1 = backend.snapshot(Some("version 1")).unwrap();

        // Modify data
        backend.write(0, &vec![0x22; 512]).unwrap();
        let _snap2 = backend.snapshot(Some("version 2")).unwrap();

        // Verify current state
        assert_eq!(backend.read(0, 1).unwrap(), vec![0x22; 512]);

        // Restore to first snapshot
        backend.restore(&snap1).unwrap();
        assert_eq!(backend.read(0, 1).unwrap(), vec![0x11; 512]);
    }

    #[test]
    fn test_cas_list_snapshots() {
        let (_temp, mut backend) = create_test_backend();

        backend.snapshot(Some("first")).unwrap();
        backend.snapshot(Some("second")).unwrap();

        let snapshots = backend.list_snapshots().unwrap();
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].description, Some("first".to_string()));
        assert_eq!(snapshots[1].description, Some("second".to_string()));
    }

    #[test]
    fn test_cas_compression() {
        let (_temp, mut backend) = create_test_backend();

        // Write highly compressible data (all same byte)
        let data = vec![0x00; 512];
        backend.write(0, &data).unwrap();

        // Should still read back correctly
        let read = backend.read(0, 1).unwrap();
        assert_eq!(read, data);

        // Write less compressible data
        let mut random_data = vec![0u8; 512];
        for (i, b) in random_data.iter_mut().enumerate() {
            *b = (i * 7 % 256) as u8;
        }
        backend.write(1, &random_data).unwrap();

        let read = backend.read(1, 1).unwrap();
        assert_eq!(read, random_data);
    }
}
