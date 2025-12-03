# AoE Archival Storage Server

An ATA over Ethernet (AoE) server implementation in Rust with pluggable storage backends, including a content-addressed storage (CAS) backend inspired by Plan 9's Venti.

## Overview

This project combines the simplicity of AoE with the benefits of content-addressed storage. Clients see a standard block device through any AoE initiator, while the server provides deduplicated, append-only archival storage underneath.

**Key insight**: Venti's limitation was requiring custom clients. By using AoE as the frontend, any OS with an AoE initiator (Linux, FreeBSD, etc.) sees a normal disk. No special software needed on the client side.

## Features

- **AoE Protocol**: Standard ATA over Ethernet implementation (EtherType 0x88A2)
- **Multiple Backends**:
  - File-based storage (simple disk images)
  - Content-addressed storage with deduplication
  - Device passthrough (planned)
- **Content-Addressed Storage**:
  - BLAKE3 hashing for fast, secure content addressing
  - Automatic block-level deduplication
  - Merkle tree indexing
  - LZ4 compression
  - Immutable snapshots (just store the root hash)
- **Pluggable Blob Stores**: File system, S3, Azure, Backblaze B2 (trait-based)
- **Multi-target Support**: Multiple shelf/slot combinations from a single server

## Architecture

```
┌─────────────────────┐
│   AoE Clients       │  Any OS with AoE initiator
│ (Linux, FreeBSD...) │
└──────────┬──────────┘
           │ Ethernet (0x88A2)
           ▼
┌─────────────────────┐
│   Protocol Layer    │  AoE/ATA command handling
└──────────┬──────────┘
           │
┌──────────▼──────────┐
│  BlockStorage Trait │  Unified block device interface
└──────────┬──────────┘
           │
    ┌──────┴──────┬──────────┐
    ▼             ▼          ▼
┌────────┐   ┌────────┐  ┌────────┐
│  File  │   │ Device │  │  CAS   │
│Backend │   │Backend │  │Backend │
└────────┘   └────────┘  └────┬───┘
                              │
                         ┌────▼────┐
                         │BlobStore│  File/S3/Azure/B2...
                         └─────────┘
```

## Quick Start

### Prerequisites

- Rust 1.70 or later
- Linux (for raw Ethernet socket support)
- Root/CAP_NET_RAW capability (for packet capture)

### Building

```bash
cargo build --release
```

### Configuration

Copy the example configuration:

```bash
cp config.example.toml config.toml
```

Edit `config.toml` to set your network interface and configure targets:

```toml
[server]
interface = "eth0"
log_level = "info"

[[target]]
shelf = 1
slot = 0
backend = "file"
config_string = "aoe-disk-1"

[target.file]
path = "/data/aoe/disk1.img"
size = 1073741824  # 1 GiB
```

### Running

```bash
sudo ./target/release/aoe-server config.toml
```

### Client Setup (Linux)

```bash
# Load the AoE driver
modprobe aoe

# Discover AoE devices
aoe-discover

# Check discovered devices
cat /dev/etherd/interfaces

# Your device should appear as /dev/etherd/e1.0 (shelf 1, slot 0)
```

## Storage Backends

### File Backend

Simple file-backed storage. Creates a disk image file and serves it as a block device.

```toml
[[target]]
shelf = 1
slot = 0
backend = "file"

[target.file]
path = "/data/disk.img"
size = 1073741824
```

### CAS Backend (Planned)

Content-addressed storage with deduplication and snapshots.

```toml
[[target]]
shelf = 2
slot = 0
backend = "cas"

[target.cas]
block_size = 4096
total_sectors = 2097152  # 8 GiB

[target.cas.blob_store]
type = "file"
path = "/data/blobs"
```

## Documentation

Detailed design documentation is available in the `docs/` directory:

- [00-INDEX.md](docs/00-INDEX.md) - Documentation overview and reading guide
- [01-ARCHITECTURE.md](docs/01-ARCHITECTURE.md) - System architecture and data flow
- [02-AOE-PROTOCOL.md](docs/02-AOE-PROTOCOL.md) - AoE protocol implementation details
- [03-STORAGE-TRAIT.md](docs/03-STORAGE-TRAIT.md) - BlockStorage trait design
- [04-CAS-BACKEND.md](docs/04-CAS-BACKEND.md) - Content-addressed storage internals
- [05-BLOB-STORE.md](docs/05-BLOB-STORE.md) - BlobStore trait and implementations
- [06-IMPLEMENTATION.md](docs/06-IMPLEMENTATION.md) - Implementation roadmap

## Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Hash algorithm | BLAKE3 | Fast, modern, 256-bit output |
| Block size | 4096 bytes | Modern standard, good compression ratio |
| Index storage | Merkle tree in CAS | Index is just blocks, survives with only root hash |
| Blob backends | Trait-based | Swap file/S3/Azure/B2 without changing CAS logic |
| Multiple targets | Yes | shelf/slot addressing, minimal complexity |
| Compression | LZ4 | Fast, reasonable ratio, can add zstd later |

## Development Status

Current implementation status:

- [x] AoE protocol layer (raw Ethernet, frame parsing)
- [x] ATA command handling (READ SECTORS, WRITE SECTORS, IDENTIFY)
- [x] File backend implementation
- [x] Target manager (shelf/slot routing)
- [x] Basic configuration loading
- [ ] CAS backend with Merkle trees
- [ ] BlobStore implementations (S3, Azure, B2)
- [ ] Snapshot management
- [ ] Device backend

## Non-Goals (v1)

- TCP transport (use separate bridge program)
- Encryption at rest (use encrypted backing store)
- Multi-writer support (single server owns the store)
- Garbage collection (archival = keep everything)

## License

MIT

## References

- AoE Protocol Specification - Coile & Hopkins, Coraid Inc.
- "Venti: a new approach to archival storage" - Quinlan & Dorward, FAST 2002
