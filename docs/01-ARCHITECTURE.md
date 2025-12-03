# Architecture

## System Layers

```
┌─────────────────────────────────────────────────────────┐
│                      AoE Clients                        │
│              (Linux aoe driver, etc.)                   │
└─────────────────────────┬───────────────────────────────┘
                          │ Ethernet (0x88A2)
                          ▼
┌─────────────────────────────────────────────────────────┐
│                    AoE Server                           │
│  ┌───────────────────────────────────────────────────┐  │
│  │              Protocol Layer                       │  │
│  │   - Raw Ethernet receive/send (pnet)              │  │
│  │   - AoE header parsing                            │  │
│  │   - ATA command handling                          │  │
│  │   - Config/Query handling                         │  │
│  └───────────────────────┬───────────────────────────┘  │
│                          │                              │
│  ┌───────────────────────▼───────────────────────────┐  │
│  │           Target Manager                          │  │
│  │   - shelf/slot → backend mapping                  │  │
│  │   - broadcast handling                            │  │
│  └───────────────────────┬───────────────────────────┘  │
│                          │                              │
│  ┌───────────────────────▼───────────────────────────┐  │
│  │         BlockStorage Trait                        │  │
│  │   - read(lba, count) → data                       │  │
│  │   - write(lba, data)                              │  │
│  │   - info() → DeviceInfo                           │  │
│  └───────────────────────┬───────────────────────────┘  │
│                          │                              │
│         ┌────────────────┼────────────────┐             │
│         ▼                ▼                ▼             │
│  ┌───────────┐    ┌───────────┐    ┌───────────┐        │
│  │   File    │    │  Device   │    │    CAS    │        │
│  │  Backend  │    │  Backend  │    │  Backend  │        │
│  └───────────┘    └───────────┘    └─────┬─────┘        │
│                                          │              │
│                          ┌───────────────▼───────────┐  │
│                          │     BlobStore Trait       │  │
│                          │  - put(hash, data)        │  │
│                          │  - get(hash) → data       │  │
│                          │  - exists(hash) → bool    │  │
│                          └───────────────┬───────────┘  │
│                                          │              │
│                      ┌───────────────────┼───────────┐  │
│                      ▼         ▼         ▼           │  │
│               ┌──────────┐ ┌──────┐ ┌────────┐       │  │
│               │FileStore │ │  S3  │ │ Azure  │ ...   │  │
│               └──────────┘ └──────┘ └────────┘       │  │
└─────────────────────────────────────────────────────────┘
```

## Data Flow

### Read Path

```
1. Ethernet frame arrives (EtherType 0x88A2)
2. Parse AoE header → extract shelf, slot, tag
3. Parse ATA header → extract LBA, sector count
4. Lookup backend by (shelf, slot)
5. backend.read(lba, count)
   - For CAS: walk Merkle tree, fetch data blocks from BlobStore
6. Build response frame with data
7. Send response (swap src/dst MAC, set R flag)
```

### Write Path

```
1. Ethernet frame arrives with data payload
2. Parse headers → LBA, sector count, data
3. Lookup backend by (shelf, slot)
4. backend.write(lba, data)
   - For CAS:
     a. Hash each block
     b. Store new blocks to BlobStore (dedup automatic)
     c. Update Merkle tree
     d. New tree nodes also stored to BlobStore
5. Build response frame (no data)
6. Send response
```

### Snapshot

```
1. Trigger snapshot (external command, timer, API)
2. CAS backend: current root hash = snapshot ID
3. Record: (timestamp, root_hash, description)
4. That's it - data already immutable in BlobStore
```

## Component Responsibilities

### Protocol Layer
- Owns the network interface
- Parses/builds AoE frames
- Validates checksums, flags
- Routes to correct target
- Handles Config/Query commands

### Target Manager
- Maps (shelf, slot) to BlockStorage instance
- Handles broadcast addresses (0xFFFF shelf, 0xFF slot)
- Loads configuration
- Lifecycle management

### BlockStorage Implementations
- Present uniform block device interface
- Hide backend complexity
- File/Device: thin wrapper, direct I/O
- CAS: Merkle tree, dedup, delegates to BlobStore

### BlobStore Implementations
- Simple key-value: hash → bytes
- Handles actual persistence
- May be local (files) or remote (S3, etc.)
- No knowledge of block semantics

## Threading Model

For v1: single-threaded event loop.

- pnet receive is blocking
- Process one frame at a time
- Sufficient for archival workloads
- Can add async/threading later if needed

## Configuration

TOML file specifying:

```toml
[server]
interface = "eth0"

[[target]]
shelf = 1
slot = 0
backend = "cas"

[target.cas]
blob_store = "file"
path = "/data/aoe-store"
block_size = 4096

[[target]]
shelf = 1
slot = 1
backend = "file"
path = "/data/disk.img"
```
