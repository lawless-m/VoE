# AoE Archival Storage Server - Design Documents

## Overview

This project implements an ATA over Ethernet (AoE) server with a content-addressed storage (CAS) backend inspired by Plan 9's Venti. The result: clients see a standard block device, but underneath lies deduplicated, append-only archival storage.

**Key insight**: Venti's flaw was requiring custom clients. By fronting with AoE, any OS with an AoE initiator sees a normal disk. No special software needed.

## Document Guide

Read in this order:

| Document | Purpose |
|----------|---------|
| [01-ARCHITECTURE.md](01-ARCHITECTURE.md) | System layers, data flow, component relationships |
| [02-AOE-PROTOCOL.md](02-AOE-PROTOCOL.md) | AoE wire format, commands, header structures |
| [03-STORAGE-TRAIT.md](03-STORAGE-TRAIT.md) | BlockStorage trait design, backend interface |
| [04-CAS-BACKEND.md](04-CAS-BACKEND.md) | Content-addressed storage, Merkle trees, dedup |
| [05-BLOB-STORE.md](05-BLOB-STORE.md) | BlobStore trait, S3/file/cloud implementations |
| [06-IMPLEMENTATION.md](06-IMPLEMENTATION.md) | Build order, crate structure, coding guidance |

## Reference Materials

These papers inform the design (may be provided separately):

- **AoE Protocol Description** - Coile & Hopkins, Coraid Inc.
- **Venti: a new approach to archival storage** - Quinlan & Dorward, FAST 2002

## Key Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Hash algorithm | BLAKE3 | Fast, modern, 256-bit output |
| Block size | 4096 bytes | Modern standard, good compression ratio |
| Index storage | Merkle tree in CAS | Index is just blocks, survives with only root hash |
| Blob backends | Trait-based | Swap file/S3/Azure/B2 without changing CAS logic |
| Multiple targets | Yes | shelf/slot addressing, minimal complexity |
| Compression | LZ4 | Fast, reasonable ratio, can add zstd later |

## Non-Goals (for v1)

- TCP transport (separate bridge program)
- Encryption at rest (use encrypted backing store)
- Multi-writer (single server owns the store)
- Garbage collection (archival = keep everything)
