//! Merkle tree operations for CAS backend
//!
//! Implements a content-addressed tree structure where each node contains
//! hashes pointing to child nodes or data blocks.

use crate::blob::{BlobError, BlobStore, Hash};

/// Number of hashes per pointer block (4096 / 32 = 128)
pub const FANOUT: usize = 128;

/// Block size in bytes
pub const BLOCK_SIZE: usize = 4096;

/// Hash size in bytes
pub const HASH_SIZE: usize = 32;

/// Merkle tree for mapping LBAs to content hashes
pub struct MerkleTree<'a> {
    blob_store: &'a dyn BlobStore,
    root_hash: Hash,
    depth: u8,
    total_sectors: u64,
}

impl<'a> MerkleTree<'a> {
    /// Create a new Merkle tree view
    pub fn new(
        blob_store: &'a dyn BlobStore,
        root_hash: Hash,
        total_sectors: u64,
    ) -> Self {
        let depth = calculate_depth(total_sectors);
        Self {
            blob_store,
            root_hash,
            depth,
            total_sectors,
        }
    }

    /// Get the current root hash
    pub fn root_hash(&self) -> Hash {
        self.root_hash
    }

    /// Look up the data hash for a given LBA
    pub fn lookup(&self, lba: u64) -> Result<Hash, BlobError> {
        if lba >= self.total_sectors {
            return Err(BlobError::Backend(format!(
                "LBA {} out of range (max {})",
                lba, self.total_sectors
            )));
        }

        // Special case: empty tree
        if self.root_hash.is_zero() {
            return Ok(Hash::ZERO);
        }

        // For depth 1, root directly contains data hashes
        if self.depth == 1 {
            let root_block = self.blob_store.get(&self.root_hash)?;
            let index = lba as usize;
            return Ok(extract_hash(&root_block, index));
        }

        // Walk down the tree
        let mut current_hash = self.root_hash;
        for level in 0..self.depth {
            if current_hash.is_zero() {
                return Ok(Hash::ZERO); // Sparse region
            }

            let node = self.blob_store.get(&current_hash)?;
            let index = extract_index(lba, level, self.depth);

            if level == self.depth - 1 {
                // Last level - return the data hash
                return Ok(extract_hash(&node, index));
            } else {
                // Intermediate level - follow the pointer
                current_hash = extract_hash(&node, index);
            }
        }

        Ok(Hash::ZERO)
    }
}

/// Mutable Merkle tree for updates
pub struct MerkleTreeMut<'a> {
    blob_store: &'a dyn BlobStore,
    root_hash: Hash,
    depth: u8,
    total_sectors: u64,
}

impl<'a> MerkleTreeMut<'a> {
    /// Create a new mutable Merkle tree
    pub fn new(
        blob_store: &'a dyn BlobStore,
        root_hash: Hash,
        total_sectors: u64,
    ) -> Self {
        let depth = calculate_depth(total_sectors);
        Self {
            blob_store,
            root_hash,
            depth,
            total_sectors,
        }
    }

    /// Create a new empty tree
    pub fn empty(blob_store: &'a dyn BlobStore, total_sectors: u64) -> Self {
        Self::new(blob_store, Hash::ZERO, total_sectors)
    }

    /// Get the current root hash
    pub fn root_hash(&self) -> Hash {
        self.root_hash
    }

    /// Update the hash for a given LBA
    pub fn update(&mut self, lba: u64, data_hash: Hash) -> Result<(), BlobError> {
        if lba >= self.total_sectors {
            return Err(BlobError::Backend(format!(
                "LBA {} out of range (max {})",
                lba, self.total_sectors
            )));
        }

        // Build path from root to leaf, creating nodes as needed
        let mut path: Vec<(Vec<u8>, usize)> = Vec::with_capacity(self.depth as usize);
        let mut current_hash = self.root_hash;

        for level in 0..self.depth {
            let index = extract_index(lba, level, self.depth);

            // Get or create the node at this level
            let node = if current_hash.is_zero() {
                // Create empty node
                vec![0u8; BLOCK_SIZE]
            } else {
                self.blob_store.get(&current_hash)?
            };

            if level < self.depth - 1 {
                // Get hash of next level
                current_hash = extract_hash(&node, index);
            }

            path.push((node, index));
        }

        // Update the leaf node with the data hash
        if let Some((ref mut leaf, index)) = path.last_mut() {
            set_hash(leaf, *index, &data_hash);
        }

        // Walk back up, updating each node and computing new hashes
        let mut child_hash = data_hash;
        for (level, (mut node, index)) in path.into_iter().enumerate().rev() {
            if level == self.depth as usize - 1 {
                // Leaf level - already updated above
                set_hash(&mut node, index, &child_hash);
            } else {
                // Intermediate level - update pointer to child
                set_hash(&mut node, index, &child_hash);
            }

            // Compute and store new node hash
            let new_hash = Hash::from_data(&node);
            self.blob_store.put(&new_hash, &node)?;
            child_hash = new_hash;
        }

        // Update root
        self.root_hash = child_hash;
        Ok(())
    }

    /// Look up the data hash for a given LBA
    pub fn lookup(&self, lba: u64) -> Result<Hash, BlobError> {
        let tree = MerkleTree::new(self.blob_store, self.root_hash, self.total_sectors);
        tree.lookup(lba)
    }
}

/// Calculate tree depth for given number of sectors
pub fn calculate_depth(total_sectors: u64) -> u8 {
    if total_sectors == 0 {
        return 1;
    }

    let mut depth = 1u8;
    let mut capacity = FANOUT as u64;

    while capacity < total_sectors {
        depth += 1;
        capacity *= FANOUT as u64;
    }

    depth
}

/// Extract the index at a given level for an LBA
fn extract_index(lba: u64, level: u8, depth: u8) -> usize {
    // At level 0 (root), we use the most significant bits
    // At level depth-1 (leaf), we use the least significant bits
    let shift = (depth - 1 - level) as u32 * 7; // 7 bits = log2(128)
    ((lba >> shift) & 0x7F) as usize
}

/// Extract a hash from a node at the given index
fn extract_hash(node: &[u8], index: usize) -> Hash {
    let start = index * HASH_SIZE;
    let end = start + HASH_SIZE;

    if end > node.len() {
        return Hash::ZERO;
    }

    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&node[start..end]);
    Hash::from_bytes(bytes)
}

/// Set a hash in a node at the given index
fn set_hash(node: &mut [u8], index: usize, hash: &Hash) {
    let start = index * HASH_SIZE;
    let end = start + HASH_SIZE;

    if end <= node.len() {
        node[start..end].copy_from_slice(hash.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blob::FileBlobStore;
    use tempfile::TempDir;

    #[test]
    fn test_calculate_depth() {
        assert_eq!(calculate_depth(1), 1);
        assert_eq!(calculate_depth(128), 1);
        assert_eq!(calculate_depth(129), 2);
        assert_eq!(calculate_depth(16384), 2);
        assert_eq!(calculate_depth(16385), 3);
    }

    #[test]
    fn test_extract_index() {
        // For depth 1, index is just lba
        assert_eq!(extract_index(0, 0, 1), 0);
        assert_eq!(extract_index(127, 0, 1), 127);

        // For depth 2:
        // Level 0 uses bits 7-13 (high 7 bits of 14-bit address)
        // Level 1 uses bits 0-6 (low 7 bits)
        assert_eq!(extract_index(0, 0, 2), 0);
        assert_eq!(extract_index(0, 1, 2), 0);
        assert_eq!(extract_index(128, 0, 2), 1);
        assert_eq!(extract_index(128, 1, 2), 0);
        assert_eq!(extract_index(129, 0, 2), 1);
        assert_eq!(extract_index(129, 1, 2), 1);
    }

    #[test]
    fn test_tree_update_and_lookup() {
        let temp = TempDir::new().unwrap();
        let store = FileBlobStore::new(temp.path()).unwrap();

        let mut tree = MerkleTreeMut::empty(&store, 256);

        // Write some data
        let hash1 = Hash::from_data(b"block 0");
        let hash2 = Hash::from_data(b"block 100");

        tree.update(0, hash1).unwrap();
        tree.update(100, hash2).unwrap();

        // Verify lookups
        assert_eq!(tree.lookup(0).unwrap(), hash1);
        assert_eq!(tree.lookup(100).unwrap(), hash2);

        // Unwritten blocks should return zero hash
        assert!(tree.lookup(50).unwrap().is_zero());
    }

    #[test]
    fn test_tree_persistence() {
        let temp = TempDir::new().unwrap();
        let store = FileBlobStore::new(temp.path()).unwrap();

        let root_hash;
        {
            let mut tree = MerkleTreeMut::empty(&store, 256);
            let hash = Hash::from_data(b"persistent data");
            tree.update(42, hash).unwrap();
            root_hash = tree.root_hash();
        }

        // Create new tree from saved root
        let tree = MerkleTree::new(&store, root_hash, 256);
        let expected = Hash::from_data(b"persistent data");
        assert_eq!(tree.lookup(42).unwrap(), expected);
    }
}
