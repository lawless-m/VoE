# Implementation Guide

Instructions for building the AoE archival storage server.

## Crate Structure

```
aoe-server/
├── Cargo.toml
├── src/
│   ├── main.rs              # CLI, config loading, server startup
│   ├── lib.rs               # Public API
│   ├── protocol/
│   │   ├── mod.rs
│   │   ├── types.rs         # AoE header structs
│   │   ├── parse.rs         # Frame parsing
│   │   ├── build.rs         # Frame building
│   │   └── ata.rs           # ATA command handling
│   ├── server/
│   │   ├── mod.rs
│   │   ├── listener.rs      # Ethernet listener (pnet)
│   │   └── target.rs        # Target manager (shelf/slot routing)
│   ├── storage/
│   │   ├── mod.rs           # BlockStorage trait
│   │   ├── file.rs          # File backend
│   │   ├── device.rs        # Raw device backend
│   │   └── cas/
│   │       ├── mod.rs       # CasBackend
│   │       ├── tree.rs      # Merkle tree operations
│   │       └── snapshot.rs  # Snapshot management
│   ├── blob/
│   │   ├── mod.rs           # BlobStore trait
│   │   ├── file.rs          # FileBlobStore
│   │   ├── s3.rs            # S3BlobStore
│   │   └── cached.rs        # CachedBlobStore wrapper
│   └── config.rs            # TOML config parsing
├── docs/                    # These design documents
└── tests/
    ├── protocol_tests.rs
    ├── storage_tests.rs
    └── integration/
```

## Build Order

### Phase 1: Protocol Layer

1. **types.rs** - Define header structs
   - `AoeHeader` (common 10-byte header after Ethernet)
   - `AtaHeader` (12-byte ATA command header)
   - `ConfigHeader` (8-byte config header)
   - Use `#[repr(C, packed)]` for wire format

2. **parse.rs** - Parsing functions
   - `parse_frame(&[u8]) -> Result<AoeFrame, ParseError>`
   - Handle endianness (network byte order = big endian)
   - Validate version, flags

3. **build.rs** - Response building
   - `build_response(request, data) -> Vec<u8>`
   - Set R flag, copy tag, swap MACs

4. **ata.rs** - ATA command dispatch
   - `handle_ata(storage, ata_header, data) -> AtaResponse`
   - Implement: READ, WRITE, IDENTIFY, FLUSH
   - Both LBA28 and LBA48 variants

### Phase 2: Storage Trait

5. **storage/mod.rs** - Define traits
   - `BlockStorage` trait
   - `ArchivalStorage` trait (extends BlockStorage)
   - `DeviceInfo` struct
   - `StorageError` enum

6. **storage/file.rs** - File backend
   - Simple seek + read/write
   - Use `std::fs::File` or `memmap2`
   - Good for testing

### Phase 3: Server

7. **server/listener.rs** - Network layer
   - Use `pnet_datalink` to open interface
   - Filter for EtherType 0x88A2
   - Main receive loop

8. **server/target.rs** - Target routing
   - `HashMap<(u16, u8), Box<dyn BlockStorage>>`
   - Handle broadcast addresses
   - Load from config

9. **main.rs** - Tie it together
   - Parse CLI args
   - Load config
   - Create backends
   - Start listener

### Phase 4: CAS Backend

10. **blob/file.rs** - File blob store
    - Directory structure: `XX/XXYYYY...`
    - Atomic writes (temp + rename)
    - Integrity check on read

11. **storage/cas/tree.rs** - Merkle tree
    - `lookup(root, lba) -> Hash`
    - `update(root, lba, hash) -> new_root`
    - Copy-on-write updates

12. **storage/cas/mod.rs** - CasBackend
    - Implement BlockStorage
    - Wire up tree + blob store
    - Implement ArchivalStorage for snapshots

### Phase 5: Polish

13. **blob/s3.rs** - S3 backend
14. **blob/cached.rs** - Caching wrapper
15. **config.rs** - Full config support
16. Tests, documentation, error messages

## Key Dependencies

```toml
[dependencies]
pnet = "0.35"              # Raw ethernet
pnet_datalink = "0.35"
blake3 = "1.5"             # Hashing
thiserror = "2"            # Error types
anyhow = "1"               # Error handling
log = "0.4"                # Logging
env_logger = "0.11"
serde = { version = "1", features = ["derive"] }
toml = "0.8"               # Config parsing
memmap2 = "0.9"            # Memory-mapped I/O
lz4_flex = "0.11"          # Compression

# Optional, for S3
aws-sdk-s3 = { version = "1", optional = true }
tokio = { version = "1", features = ["rt"], optional = true }
```

## Testing Strategy

### Unit Tests
- Protocol parsing: known-good byte sequences
- Tree operations: small trees, edge cases
- Blob store: write/read/verify cycle

### Integration Tests
- Full frame round-trip with file backend
- CAS deduplication (write same data twice)
- Snapshot create/restore

### Manual Testing
```bash
# Linux host with aoe module
sudo modprobe aoe
sudo aoe-discover
sudo aoe-stat

# Should see the target
# Can then partition, format, mount
```

## Platform Notes

### Linux
- Needs CAP_NET_RAW or root for raw sockets
- AoE client: `modprobe aoe`, tools in `aoetools` package

### Permissions
```bash
# Either run as root, or:
sudo setcap cap_net_raw+ep ./target/release/aoe-server
```

## Configuration Example

```toml
[server]
interface = "eth0"
log_level = "info"

[[target]]
shelf = 1
slot = 0
backend = "cas"

[target.cas]
block_size = 4096
total_sectors = 2097152  # 8 GiB

[target.cas.blob_store]
type = "file"
path = "/data/aoe/blobs"

# Or for S3:
# type = "s3"
# bucket = "my-aoe-storage"
# region = "us-east-1"

[[target]]
shelf = 1
slot = 1
backend = "file"
path = "/data/aoe/disk1.img"
```

## Error Handling Philosophy

- Protocol errors → AoE error response (don't crash)
- Storage errors → Log + AoE error response
- Config errors → Fail fast at startup
- Blob corruption → Return error, log loudly, don't serve bad data

## Performance Considerations

- Single-threaded is fine for archival workloads
- Cache hot tree nodes in memory
- Use mmap for file backends
- Compress only if it saves space
- Batch writes to blob store if possible

## Future Enhancements (Out of Scope for v1)

- Async I/O with tokio
- Multiple worker threads
- Admin API (HTTP? Unix socket?)
- Metrics/Prometheus endpoint
- Garbage collection for orphaned blobs
- Encryption at rest
