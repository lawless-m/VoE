//! Blob storage abstraction
//!
//! Defines the BlobStore trait for content-addressed storage backends.

pub mod file;

use std::fmt;
use thiserror::Error;

/// Blob storage errors
#[derive(Debug, Error)]
pub enum BlobError {
    #[error("blob not found: {0}")]
    NotFound(String),

    #[error("data corruption detected for hash {0}")]
    Corrupted(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("backend error: {0}")]
    Backend(String),

    #[error("write quorum not met")]
    QuorumNotMet,
}

/// Result type for blob operations
pub type BlobResult<T> = Result<T, BlobError>;

/// BLAKE3 hash (32 bytes)
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Hash([u8; 32]);

impl Hash {
    /// Zero hash (represents empty/sparse block)
    pub const ZERO: Hash = Hash([0u8; 32]);

    /// Create hash from raw bytes
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Hash(bytes)
    }

    /// Compute hash of data
    pub fn from_data(data: &[u8]) -> Self {
        Hash(blake3::hash(data).into())
    }

    /// Convert to hex string
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Parse from hex string
    pub fn from_hex(s: &str) -> Result<Self, hex::FromHexError> {
        let bytes = hex::decode(s)?;
        if bytes.len() != 32 {
            return Err(hex::FromHexError::InvalidStringLength);
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(Hash(arr))
    }

    /// Check if this is the zero hash
    pub fn is_zero(&self) -> bool {
        self.0 == [0u8; 32]
    }

    /// Get raw bytes
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hash({})", &self.to_hex()[..16])
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

/// Blob store trait - simple key-value interface for content-addressed storage
pub trait BlobStore: Send + Sync {
    /// Store a blob, keyed by its hash.
    /// Implementation should verify hash matches content.
    fn put(&self, hash: &Hash, data: &[u8]) -> BlobResult<()>;

    /// Retrieve a blob by hash.
    /// Returns error if not found or corrupted.
    fn get(&self, hash: &Hash) -> BlobResult<Vec<u8>>;

    /// Check if blob exists without fetching.
    fn exists(&self, hash: &Hash) -> BlobResult<bool>;

    /// Delete a blob (optional, may be no-op for archival).
    fn delete(&self, _hash: &Hash) -> BlobResult<()> {
        Ok(()) // Default: ignore deletes
    }

    /// Sync any pending writes.
    fn sync(&self) -> BlobResult<()>;
}

// Re-export implementations
pub use file::FileBlobStore;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_from_data() {
        let data = b"hello world";
        let hash = Hash::from_data(data);
        assert!(!hash.is_zero());

        // Same data should produce same hash
        let hash2 = Hash::from_data(data);
        assert_eq!(hash, hash2);
    }

    #[test]
    fn test_hash_hex_roundtrip() {
        let hash = Hash::from_data(b"test");
        let hex = hash.to_hex();
        let hash2 = Hash::from_hex(&hex).unwrap();
        assert_eq!(hash, hash2);
    }

    #[test]
    fn test_hash_zero() {
        assert!(Hash::ZERO.is_zero());
        assert!(!Hash::from_data(b"x").is_zero());
    }
}
