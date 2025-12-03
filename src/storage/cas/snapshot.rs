//! Snapshot management for CAS backend
//!
//! Handles creating, listing, and restoring snapshots.
//! A snapshot is simply a recorded root hash at a point in time.

use crate::blob::Hash;
use crate::storage::SnapshotInfo;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Snapshot entry for persistence
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SnapshotEntry {
    /// Root hash as hex string
    root: String,
    /// Unix timestamp
    timestamp: u64,
    /// Optional description
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

/// Manages snapshots for a CAS backend
pub struct SnapshotManager {
    /// Path to the snapshots file
    path: PathBuf,
    /// Loaded snapshots
    snapshots: Vec<SnapshotEntry>,
}

impl SnapshotManager {
    /// Create a new snapshot manager
    pub fn new<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let snapshots = if path.exists() {
            let content = fs::read_to_string(&path)?;
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            Vec::new()
        };

        Ok(Self { path, snapshots })
    }

    /// Create a new snapshot
    pub fn create(&mut self, root_hash: Hash, description: Option<&str>) -> io::Result<String> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let entry = SnapshotEntry {
            root: root_hash.to_hex(),
            timestamp,
            description: description.map(String::from),
        };

        self.snapshots.push(entry);
        self.save()?;

        Ok(root_hash.to_hex())
    }

    /// List all snapshots
    pub fn list(&self) -> Vec<SnapshotInfo> {
        self.snapshots
            .iter()
            .map(|entry| SnapshotInfo {
                id: entry.root.clone(),
                timestamp: entry.timestamp,
                description: entry.description.clone(),
            })
            .collect()
    }

    /// Get root hash for a snapshot ID
    pub fn get(&self, snapshot_id: &str) -> Option<Hash> {
        self.snapshots
            .iter()
            .find(|s| s.root == snapshot_id)
            .and_then(|s| Hash::from_hex(&s.root).ok())
    }

    /// Get the most recent snapshot
    pub fn latest(&self) -> Option<Hash> {
        self.snapshots
            .last()
            .and_then(|s| Hash::from_hex(&s.root).ok())
    }

    /// Delete a snapshot by ID
    pub fn delete(&mut self, snapshot_id: &str) -> io::Result<bool> {
        let original_len = self.snapshots.len();
        self.snapshots.retain(|s| s.root != snapshot_id);

        if self.snapshots.len() < original_len {
            self.save()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Save snapshots to disk
    fn save(&self) -> io::Result<()> {
        let content = serde_json::to_string_pretty(&self.snapshots)?;
        fs::write(&self.path, content)
    }

    /// Get the path to the snapshot file
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_snapshot_create_and_list() {
        let temp = TempDir::new().unwrap();
        let snapshot_path = temp.path().join("snapshots.json");

        let mut manager = SnapshotManager::new(&snapshot_path).unwrap();

        let hash1 = Hash::from_data(b"root1");
        let hash2 = Hash::from_data(b"root2");

        manager.create(hash1, Some("first")).unwrap();
        manager.create(hash2, None).unwrap();

        let snapshots = manager.list();
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].description, Some("first".to_string()));
        assert!(snapshots[1].description.is_none());
    }

    #[test]
    fn test_snapshot_persistence() {
        let temp = TempDir::new().unwrap();
        let snapshot_path = temp.path().join("snapshots.json");

        let hash = Hash::from_data(b"persistent");

        {
            let mut manager = SnapshotManager::new(&snapshot_path).unwrap();
            manager.create(hash, Some("test")).unwrap();
        }

        // Reload
        let manager = SnapshotManager::new(&snapshot_path).unwrap();
        let snapshots = manager.list();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].id, hash.to_hex());
    }

    #[test]
    fn test_snapshot_get() {
        let temp = TempDir::new().unwrap();
        let snapshot_path = temp.path().join("snapshots.json");

        let mut manager = SnapshotManager::new(&snapshot_path).unwrap();
        let hash = Hash::from_data(b"findme");
        manager.create(hash, None).unwrap();

        let found = manager.get(&hash.to_hex());
        assert!(found.is_some());
        assert_eq!(found.unwrap(), hash);

        let not_found = manager.get("nonexistent");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_snapshot_delete() {
        let temp = TempDir::new().unwrap();
        let snapshot_path = temp.path().join("snapshots.json");

        let mut manager = SnapshotManager::new(&snapshot_path).unwrap();
        let hash = Hash::from_data(b"deleteme");
        let id = manager.create(hash, None).unwrap();

        assert_eq!(manager.list().len(), 1);
        assert!(manager.delete(&id).unwrap());
        assert_eq!(manager.list().len(), 0);
    }
}
