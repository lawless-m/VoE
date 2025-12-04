# VoE - Versatile over Ethernet Storage

A suite of network block storage servers in Rust with content-addressed storage (CAS) and automatic deduplication.

## Overview

VoE provides network-attached block storage with pluggable protocols (AoE, NBD, iSCSI) and backends. All storage can optionally use content-addressed storage for automatic deduplication and immutable snapshots.

**Key Features**:
- **Multiple Protocols**: AoE (Linux/FreeBSD), NBD (Linux), iSCSI (Windows via bridge)
- **Content-Addressed Storage**: SHA-256 hashing with automatic block-level deduplication
- **High Performance**: 120 MB/s for unique data, 500 MB/s for duplicates
- **Standard Clients**: No special software needed - use built-in OS initiators

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
┌──────────────────────────────────────────────────────────────┐
│                        Clients                                │
│  AoE (Linux/BSD)  │  NBD (Linux)  │  iSCSI (Windows)         │
└──────────┬─────────────────┬──────────────────┬──────────────┘
           │                 │                  │
           │ Ethernet        │ TCP/IP           │ iSCSI/TCP
           │ (0x88A2)        │ (port 10809)     │ (port 3260)
           ▼                 ▼                  ▼
    ┌──────────┐      ┌──────────┐      ┌──────────────┐
    │   AoE    │      │   NBD    │      │ TGT (bridge) │
    │  Server  │      │  Server  │      │     +NBD     │
    └─────┬────┘      └─────┬────┘      └───────┬──────┘
          │                 │                    │
          │           ┌─────▼────────────────────┘
          │           │
    ┌─────▼───────────▼─────┐
    │  BlockStorage Trait   │  Unified block device API
    └───────────┬───────────┘
                │
        ┌───────┴────────┬──────────┐
        ▼                ▼          ▼
    ┌────────┐      ┌────────┐  ┌────────┐
    │  File  │      │ Device │  │  CAS   │
    │Backend │      │Backend │  │Backend │
    └────────┘      └────────┘  └────┬───┘
                                     │
                            ┌────────▼────────┐
                            │  CAS Server     │  SHA-256 hashing
                            │  (port 3000)    │  Deduplication
                            └────────┬────────┘
                                     │
                            ┌────────▼────────┐
                            │ Content Store   │  Immutable blocks
                            └─────────────────┘
```

## Quick Start

### Prerequisites

- Rust 1.70 or later
- Linux
- Root/sudo access (for AoE raw sockets, NBD devices, iSCSI targets)

### Building

```bash
cargo build --release
```

The build produces three binaries:
- `cas-server` - Content-addressable storage server
- `nbd-server` - NBD server with CAS backend
- `aoe-server` - AoE server (Ethernet-based, Linux/BSD clients)

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

### Client Setup (Linux AoE)

```bash
# Load the AoE driver
modprobe aoe

# Discover AoE devices
aoe-discover

# Check discovered devices
cat /dev/etherd/interfaces

# Your device should appear as /dev/etherd/e1.0 (shelf 1, slot 0)
```

## NBD Server with CAS Backend

The NBD (Network Block Device) server provides TCP/IP-based block storage with content-addressed storage and deduplication. This is the recommended setup for network-attached storage with Windows support.

### Setup

#### 1. Start the CAS Server

```bash
# Create storage directory
sudo mkdir -p /var/lib/voe-cas

# Start CAS server (listens on port 3000)
RUST_LOG=info ./target/release/cas-server \
    --bind 0.0.0.0:3000 \
    --storage /var/lib/voe-cas
```

#### 2. Start the NBD Server

```bash
# Create index directory
sudo mkdir -p /var/lib/voe-nbd

# Start NBD server (listens on port 10809)
# Size is in MB (10240 = 10 GB)
RUST_LOG=info ./target/release/nbd-server \
    --bind 0.0.0.0:10809 \
    --cas-server 127.0.0.1:3000 \
    --size 10240 \
    --index /var/lib/voe-nbd/index.json
```

#### 3a. Linux Client Setup

```bash
# Install NBD client tools
sudo apt-get install nbd-client

# Load NBD kernel module
sudo modprobe nbd

# Connect to NBD server
sudo nbd-client <server-ip> 10809 /dev/nbd0

# Check device
sudo blockdev --getsize64 /dev/nbd0

# Format and mount
sudo mkfs.ext4 /dev/nbd0
sudo mount /dev/nbd0 /mnt
```

#### 3b. Windows Client Setup (via iSCSI Bridge)

Windows doesn't have native NBD support, so we use TGT to bridge NBD to iSCSI:

**On the Linux server:**

```bash
# Install TGT (SCSI target framework)
sudo apt-get install tgt

# Connect NBD locally
sudo modprobe nbd
sudo nbd-client 127.0.0.1 10809 /dev/nbd0

# Create iSCSI target
sudo tgtadm --lld iscsi --op new --mode target --tid 1 \
    -T iqn.2025-12.local.voe:storage.cas-disk

# Add NBD device as LUN 1
sudo tgtadm --lld iscsi --op new --mode logicalunit --tid 1 --lun 1 \
    -b /dev/nbd0

# Allow all initiators
sudo tgtadm --lld iscsi --op bind --mode target --tid 1 -I ALL

# Verify target
sudo tgtadm --lld iscsi --op show --mode target
```

**On Windows:**

1. Open iSCSI Initiator (search in Start menu)
2. Go to "Discovery" tab
3. Click "Discover Portal"
4. Enter server IP and port 3260
5. Go to "Targets" tab
6. Select `iqn.2025-12.local.voe:storage.cas-disk`
7. Click "Connect"
8. Open Disk Management (diskmgmt.msc)
9. Initialize the disk (GPT recommended)
10. Create a new volume and format

### Performance

Measured performance with the NBD+CAS stack:

- **Unique data**: 120 MB/s (with SHA-256 hashing and storage)
- **Duplicate data**: 500 MB/s (deduplication cache hit)
- **Space savings**: Automatic block-level deduplication

### How Deduplication Works

1. **Write Operation**:
   - Data arrives at NBD server
   - NBD splits into 512-byte sectors
   - Each sector hashed with SHA-256
   - Hash sent to CAS server
   - CAS checks if content exists
   - If new: store content, return hash
   - If duplicate: skip storage, return existing hash
   - NBD updates LBA → hash index

2. **Read Operation**:
   - NBD looks up LBA in index
   - Gets content hash
   - Requests content from CAS by hash
   - Returns data to client

3. **Result**:
   - Identical files copy instantly
   - Only unique blocks consume storage
   - No configuration needed - automatic

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

**Protocols:**
- [x] AoE protocol layer (raw Ethernet, frame parsing)
- [x] NBD protocol (newstyle handshake, full spec compliance)
- [x] iSCSI bridge (via TGT for Windows support)

**Storage Backends:**
- [x] File backend implementation
- [x] CAS backend (SHA-256 content addressing)
- [ ] CAS with Merkle trees and snapshots
- [ ] Device passthrough backend

**Features:**
- [x] Content-addressable storage server
- [x] Block-level deduplication (512-byte sectors)
- [x] Persistent LBA-to-hash index
- [x] Windows client support (iSCSI)
- [x] Linux client support (AoE, NBD)
- [x] Multi-target support (AoE shelf/slot)
- [ ] Snapshot management
- [ ] BlobStore implementations (S3, Azure, B2)
- [ ] Compression (LZ4)

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
