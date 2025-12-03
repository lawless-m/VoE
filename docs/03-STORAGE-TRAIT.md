# Storage Trait Design

## Overview

The `BlockStorage` trait is the boundary between AoE protocol handling and storage backends. Implementations present block device semantics regardless of how data is actually stored.

## Core Trait

```rust
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
}
```

## DeviceInfo

```rust
pub struct DeviceInfo {
    pub model: String,          // "AoE Virtual Disk"
    pub serial: String,         // Unique identifier
    pub firmware: String,       // Server version
    pub total_sectors: u64,     // LBA48 max
    pub sector_size: u32,       // 512 or 4096
    pub lba48: bool,            // Always true for us
}
```

Used to respond to ATA IDENTIFY DEVICE command.

## Error Types

```rust
pub enum StorageError {
    Io(std::io::Error),
    OutOfRange { lba: u64, max: u64 },
    InvalidSectorCount(u8),
    Backend(String),
    ReadOnly,
}
```

## Extended Trait for Archival

```rust
pub trait ArchivalStorage: BlockStorage {
    /// Create snapshot, return identifier (root hash).
    fn snapshot(&mut self) -> StorageResult<String>;

    /// List available snapshots.
    fn list_snapshots(&self) -> StorageResult<Vec<SnapshotInfo>>;

    /// Restore to a snapshot (reads will see that version).
    fn restore(&mut self, snapshot_id: &str) -> StorageResult<()>;
}
```

This is optional - File/Device backends don't implement it.

## Backend Implementations

### FileBackend

Simplest possible:

```rust
struct FileBackend {
    file: File,          // Or mmap
    info: DeviceInfo,
}
```

- `read()`: seek + read
- `write()`: seek + write
- `flush()`: fsync

File size determines total_sectors.

### DeviceBackend

Same as FileBackend but opens `/dev/sdX` directly. May use O_DIRECT.

### CasBackend

Complex - see [04-CAS-BACKEND.md](04-CAS-BACKEND.md).

- `read()`: traverse Merkle tree, fetch from BlobStore
- `write()`: hash blocks, store new ones, update tree
- `flush()`: ensure tree persisted

## Sector Size Considerations

| Size | Pros | Cons |
|------|------|------|
| 512 | Max compatibility | More blocks to manage |
| 4096 | Modern standard, less overhead | Some old tools expect 512 |

Recommend: 4096 for CAS backend (fewer hashes, better compression). Can bridge with a translation layer if needed.

## Thread Safety

Trait requires `Send + Sync`. Implementations must handle concurrent access or document that they serialize internally.

For v1, single-threaded server, but trait ready for future.

## Example: Using the Trait

```rust
fn handle_read(
    storage: &dyn BlockStorage,
    lba: u64,
    count: u8,
) -> Result<Vec<u8>, AoeError> {
    // Validate
    if count == 0 || count > MAX_SECTORS {
        return Err(AoeError::BadArg);
    }
    storage.validate_range(lba, count)?;
    
    // Read
    let data = storage.read(lba, count)?;
    Ok(data)
}
```

The protocol layer doesn't know or care what kind of backend it's talking to.
