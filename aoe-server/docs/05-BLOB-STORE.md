# Blob Store Abstraction

## Overview

BlobStore is where bytes actually live. Simple key-value interface: hash → data.

CAS backend doesn't care if blobs are on local disk, S3, or Mars.

## Trait Definition

```rust
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
    fn delete(&self, hash: &Hash) -> BlobResult<()> {
        Ok(())  // Default: ignore deletes
    }
    
    /// Sync any pending writes.
    fn sync(&self) -> BlobResult<()>;
}
```

## Hash Type

```rust
pub struct Hash([u8; 32]);  // BLAKE3 output

impl Hash {
    pub fn from_data(data: &[u8]) -> Self {
        Hash(blake3::hash(data).into())
    }
    
    pub fn to_hex(&self) -> String { ... }
    pub fn from_hex(s: &str) -> Result<Self, ...> { ... }
    
    pub fn is_zero(&self) -> bool {
        self.0 == [0u8; 32]
    }
}
```

## Implementations

### FileBlobStore

Blobs stored as files, hash as filename.

```
/data/blobs/
  ab/
    ab3f7c9d...  (first 2 chars = subdirectory)
  cd/
    cd8e2a1b...
```

```rust
struct FileBlobStore {
    root: PathBuf,
}

impl BlobStore for FileBlobStore {
    fn put(&self, hash: &Hash, data: &[u8]) -> BlobResult<()> {
        let path = self.path_for(hash);
        if path.exists() {
            return Ok(());  // Dedup
        }
        fs::create_dir_all(path.parent())?;
        
        // Write to temp, rename for atomicity
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, data)?;
        fs::rename(tmp, path)?;
        Ok(())
    }
    
    fn get(&self, hash: &Hash) -> BlobResult<Vec<u8>> {
        let data = fs::read(self.path_for(hash))?;
        
        // Verify integrity
        let actual = Hash::from_data(&data);
        if actual != *hash {
            return Err(BlobError::Corrupted);
        }
        Ok(data)
    }
    
    fn exists(&self, hash: &Hash) -> BlobResult<bool> {
        Ok(self.path_for(hash).exists())
    }
}
```

### S3BlobStore

Object storage backend.

```rust
struct S3BlobStore {
    client: S3Client,
    bucket: String,
    prefix: Option<String>,
}

impl BlobStore for S3BlobStore {
    fn put(&self, hash: &Hash, data: &[u8]) -> BlobResult<()> {
        let key = self.key_for(hash);
        
        // S3 PutObject is idempotent for same content
        self.client.put_object()
            .bucket(&self.bucket)
            .key(&key)
            .body(data.into())
            .send()?;
        Ok(())
    }
    
    fn get(&self, hash: &Hash) -> BlobResult<Vec<u8>> {
        let resp = self.client.get_object()
            .bucket(&self.bucket)
            .key(self.key_for(hash))
            .send()?;
        
        let data = resp.body.collect()?.to_vec();
        
        // Verify
        let actual = Hash::from_data(&data);
        if actual != *hash {
            return Err(BlobError::Corrupted);
        }
        Ok(data)
    }
    
    fn exists(&self, hash: &Hash) -> BlobResult<bool> {
        match self.client.head_object()
            .bucket(&self.bucket)
            .key(self.key_for(hash))
            .send()
        {
            Ok(_) => Ok(true),
            Err(e) if e.is_not_found() => Ok(false),
            Err(e) => Err(e.into()),
        }
    }
}
```

### Other Backends

Same pattern for:
- Azure Blob Storage
- Google Cloud Storage
- Backblaze B2
- MinIO (S3-compatible)
- SFTP (for remote file storage)

## Caching Layer

Wrap remote stores with local cache:

```rust
struct CachedBlobStore {
    cache: FileBlobStore,     // Local SSD
    backend: Box<dyn BlobStore>,  // S3, etc.
    max_cache_size: u64,
}

impl BlobStore for CachedBlobStore {
    fn get(&self, hash: &Hash) -> BlobResult<Vec<u8>> {
        // Try cache first
        if let Ok(data) = self.cache.get(hash) {
            return Ok(data);
        }
        
        // Fetch from backend
        let data = self.backend.get(hash)?;
        
        // Populate cache (best effort)
        let _ = self.cache.put(hash, &data);
        
        Ok(data)
    }
    
    fn put(&self, hash: &Hash, data: &[u8]) -> BlobResult<()> {
        // Write to both
        self.cache.put(hash, data)?;
        self.backend.put(hash, data)?;
        Ok(())
    }
}
```

## Compression Handling

Two approaches:

### 1. Blob Store Handles It

Store compressed, key is hash of compressed data:

```rust
fn put(&self, hash: &Hash, data: &[u8], compressed: bool) {
    // hash was computed on possibly-compressed data
}
```

### 2. CAS Backend Handles It (Recommended)

Compress before hashing, store raw bytes:

```rust
// In CAS backend
fn store_block(&self, data: &[u8]) -> Hash {
    let compressed = lz4::compress(data);
    let stored = if compressed.len() < data.len() {
        &compressed
    } else {
        data
    };
    let hash = Hash::from_data(stored);
    self.blob_store.put(&hash, stored);
    hash
}
```

Need to track which blocks are compressed. Options:
- Magic bytes prefix
- Separate metadata store
- Always try decompress, fall back

## Replication

For durability, write to multiple stores:

```rust
struct ReplicatedBlobStore {
    stores: Vec<Box<dyn BlobStore>>,
    write_quorum: usize,
}

impl BlobStore for ReplicatedBlobStore {
    fn put(&self, hash: &Hash, data: &[u8]) -> BlobResult<()> {
        let results: Vec<_> = self.stores.iter()
            .map(|s| s.put(hash, data))
            .collect();
        
        let successes = results.iter().filter(|r| r.is_ok()).count();
        if successes >= self.write_quorum {
            Ok(())
        } else {
            Err(BlobError::QuorumNotMet)
        }
    }
    
    fn get(&self, hash: &Hash) -> BlobResult<Vec<u8>> {
        // Try each store until success
        for store in &self.stores {
            if let Ok(data) = store.get(hash) {
                return Ok(data);
            }
        }
        Err(BlobError::NotFound)
    }
}
```

## Error Types

```rust
pub enum BlobError {
    NotFound,
    Corrupted,
    Io(std::io::Error),
    Backend(String),
    QuorumNotMet,
}
```
