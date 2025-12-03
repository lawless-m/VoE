# Content-Addressed Storage Backend

Reference: "Venti: a new approach to archival storage" - Quinlan & Dorward, FAST 2002

## Core Concept

Block address = hash of content. Same content = same address. Deduplication automatic.

```
Traditional:    LBA 1234 → offset 1234 * sector_size
CAS:            LBA 1234 → hash → blob_store.get(hash)
```

## Data Model

### Block Types

```rust
enum BlockType {
    Data,       // User data
    Pointer,    // Contains hashes pointing to other blocks
}
```

### Pointer Block Structure

A pointer block contains hashes of child blocks:

```
┌─────────────────────────────────────────┐
│ Hash 0 (32 bytes)                       │
│ Hash 1 (32 bytes)                       │
│ Hash 2 (32 bytes)                       │
│ ...                                     │
│ Hash N (32 bytes)                       │
└─────────────────────────────────────────┘
```

With 4096-byte blocks and 32-byte hashes: 128 hashes per pointer block.

### Tree Structure

```
                    ┌─────────┐
                    │  Root   │ (pointer block)
                    └────┬────┘
           ┌─────────────┼─────────────┐
           ▼             ▼             ▼
      ┌─────────┐   ┌─────────┐   ┌─────────┐
      │Pointer 0│   │Pointer 1│   │Pointer 2│
      └────┬────┘   └────┬────┘   └────┬────┘
           │             │             │
     ┌─────┼─────┐       ...           ...
     ▼     ▼     ▼
   ┌───┐ ┌───┐ ┌───┐
   │D0 │ │D1 │ │D2 │  (data blocks)
   └───┘ └───┘ └───┘
```

Tree depth depends on disk size:

| Sectors | Depth | Notes |
|---------|-------|-------|
| ≤128 | 1 | Root points directly to data |
| ≤16,384 | 2 | One level of pointers |
| ≤2M | 3 | Two levels |
| ≤256M | 4 | Three levels |

128^N sectors at depth N.

## Operations

### Read

```
read(lba, count):
    for each sector in lba..lba+count:
        hash = tree_lookup(root_hash, sector)
        if hash is zero:
            return zero block
        data = blob_store.get(hash)
        append data to result
    return result
```

### Write

```
write(lba, data):
    for each sector in data:
        hash = blake3(sector_data)
        blob_store.put(hash, sector_data)  # dedup happens here
        tree_update(lba + offset, hash)
    
    # Tree update cascades: new pointer blocks,
    # new root hash
    root_hash = new_root
```

### Tree Lookup

```
tree_lookup(root, lba):
    node = blob_store.get(root)
    depth = calculate_depth(total_sectors)
    
    for level in 0..depth:
        index = extract_index(lba, level, depth)
        if level == depth - 1:
            return node.hashes[index]  # data hash
        else:
            next_hash = node.hashes[index]
            if next_hash is zero:
                return zero  # sparse
            node = blob_store.get(next_hash)
    
    return node.hashes[...]
```

### Tree Update

Copy-on-write: changing a leaf requires new nodes up to root.

```
tree_update(lba, new_hash):
    path = []
    node = root
    
    # Walk down, recording path
    for level in 0..depth:
        index = extract_index(lba, level, depth)
        path.push((node, index))
        if level < depth - 1:
            node = blob_store.get(node.hashes[index])
    
    # Update leaf
    path.last().node.hashes[index] = new_hash
    
    # Walk back up, creating new nodes
    for (node, index) in path.reverse():
        new_node_hash = blake3(node)
        blob_store.put(new_node_hash, node)
        # Parent's hash for this child = new_node_hash
```

## State

### In Memory

```rust
struct CasBackend {
    blob_store: Box<dyn BlobStore>,
    root_hash: Hash,              // Current tree root
    info: DeviceInfo,
    block_size: u32,
    depth: u8,
    
    // Caches
    node_cache: LruCache<Hash, Block>,
}
```

### On Disk

Only root hash needs explicit persistence. Everything else is in the blob store.

```
snapshots.json:
[
  {"timestamp": 1701619200, "root": "abc123...", "description": "initial"},
  {"timestamp": 1701705600, "root": "def456...", "description": "daily"}
]
```

## Snapshots

Creating a snapshot = recording the current root hash.

```rust
fn snapshot(&mut self) -> String {
    let id = hex(self.root_hash);
    self.snapshots.push(SnapshotInfo {
        id: id.clone(),
        timestamp: now(),
        description: None,
    });
    id
}
```

Restoring = pointing root_hash at a previous value. All data still exists in blob store.

## Sparse Blocks

Zero hash (all zeros) = unwritten sector. Don't store, return zeros on read. Saves space for sparse disks.

## Compression

Each block compressed before hashing and storage:

```
store(data):
    compressed = lz4_compress(data)
    if compressed.len() < data.len():
        hash = blake3(compressed)
        blob_store.put(hash, compressed, compressed=true)
    else:
        hash = blake3(data)
        blob_store.put(hash, data, compressed=false)
    return hash
```

Blob store tracks compression flag per block.

## Deduplication

Automatic from content addressing:

```
put(hash, data):
    if blob_store.exists(hash):
        return  # Already have it
    blob_store.write(hash, data)
```

Write same data twice → same hash → one copy stored.

## Integrity

On every read:

```
get(hash):
    data = blob_store.read(hash)
    actual_hash = blake3(data)
    if actual_hash != hash:
        return Err(CorruptedData)
    return data
```

Built-in verification, no separate checksums needed.

## Recovery

Lost everything except root hash?

1. Point at blob store
2. Set root_hash
3. Tree structure rebuilds on read

Lost root hash but have blob store?
- Can scan for pointer blocks (they have structure)
- Can find recent roots
- Harder but possible
