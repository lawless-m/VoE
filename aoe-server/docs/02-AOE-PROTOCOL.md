# AoE Protocol

Reference: "The ATA over Ethernet Protocol" - Coile & Hopkins, Coraid Inc.

## Overview

AoE is a Layer 2 protocol (no IP) for accessing block storage over Ethernet. EtherType `0x88A2`.

Simple design:
- Request/response model
- Tag field for correlation
- Two command classes: ATA (data) and Config/Query (discovery)

## Frame Format

### Ethernet + Common AoE Header (24 bytes total)

```
Offset  Size  Field
------  ----  -----
0       6     Destination MAC
6       6     Source MAC
12      2     EtherType (0x88A2)
14      1     Version (4 bits) | Flags (4 bits)
15      1     Error code
16      2     Shelf (major) - big endian
18      1     Slot (minor)
19      1     Command (0=ATA, 1=Config)
20      4     Tag - big endian
```

### Flags (upper nibble of byte 14)

```
Bit 3: R - Response flag (0=request, 1=response)
Bit 2: E - Error flag (1=error in response)
Bits 0-1: Reserved (zero)
```

### Version

Current version: 1 (lower nibble of byte 14)

### Addressing

- Shelf: 16-bit, 0xFFFF = broadcast to all shelves
- Slot: 8-bit, 0xFF = broadcast to all slots in shelf

### Error Codes

```
0 = No error
1 = Unrecognized command code
2 = Bad argument parameter
3 = Device unavailable
4 = Config string present (set failed)
5 = Unsupported version
6 = Target is reserved
```

## ATA Command (Cmd = 0)

### ATA Header (12 bytes after common header)

```
Offset  Size  Field
------  ----  -----
24      1     AFlags
25      1     Err/Feature
26      1     Sector Count
27      1     Cmd/Status
28      1     LBA0 (bits 0-7)
29      1     LBA1 (bits 8-15)
30      1     LBA2 (bits 16-23)
31      1     LBA3 (bits 24-31)
32      1     LBA4 (bits 32-39)
33      1     LBA5 (bits 40-47)
34      2     Reserved (zero)
36      ...   Data (if any)
```

### AFlags

```
Bit 6: E - Extended (LBA48) command
Bit 5: D - Device/head register flag (legacy)
Bit 4: Reserved
Bit 3: Reserved
Bit 2: Reserved
Bit 1: A - Async write (don't wait for disk)
Bit 0: W - Write command (data follows header)
```

### Common ATA Commands

| Command | Code | Description |
|---------|------|-------------|
| READ SECTOR(S) | 0x20 | Read with LBA28 |
| READ SECTOR(S) EXT | 0x24 | Read with LBA48 |
| WRITE SECTOR(S) | 0x30 | Write with LBA28 |
| WRITE SECTOR(S) EXT | 0x34 | Write with LBA48 |
| IDENTIFY DEVICE | 0xEC | Get device info |
| FLUSH CACHE | 0xE7 | Flush write cache |
| FLUSH CACHE EXT | 0xEA | Flush write cache (LBA48) |

### Data Size Limits

Standard Ethernet (MTU 1500):
- Max 2 sectors (1024 bytes) per frame
- Header = 36 bytes, leaves 1464 for data

Jumbo frames (MTU 9000):
- Max 16 sectors (8192 bytes) per frame

## Config/Query Command (Cmd = 1)

### Config Header (8 bytes after common header)

```
Offset  Size  Field
------  ----  -----
24      2     Buffer Count (max data server can handle)
26      2     Firmware Version
28      1     Sector Count (max sectors per ATA command)
29      1     AoE/CCmd (version in high nibble, config command in low)
30      2     Config String Length
32      ...   Config String (variable)
```

### Config Commands (CCmd, low nibble)

```
0 = Read config string
1 = Test config string (exact match)
2 = Test config string (prefix match)
3 = Set config string (if currently empty)
4 = Force set config string
```

### Discovery Flow

1. Client broadcasts with shelf=0xFFFF, slot=0xFF, CCmd=0
2. All targets respond with their config strings
3. Client notes MAC addresses and shelf/slot of each target
4. Subsequent requests sent directly to target's MAC

## Response Handling

Server builds response by:
1. Swapping source/destination MAC
2. Setting R flag (bit 3 of flags)
3. Copying Tag unchanged
4. Setting E flag and error code if error
5. For ATA: copying status registers, appending data if read

## Implementation Notes

### Tag Management

- Client assigns tags, server echoes them
- Use for request/response correlation
- Use for timeout detection
- 32 bits = plenty of space

### Broadcast Handling

Server must accept broadcasts (dst MAC = FF:FF:FF:FF:FF:FF) and directed frames.

Respond only to:
- Exact shelf/slot match
- Broadcast shelf (0xFFFF) with matching or broadcast slot
- Matching shelf with broadcast slot (0xFF)

### Error Responses

On error:
- Set E flag
- Set error code
- Still include shelf/slot/tag
- No data payload

### IDENTIFY DEVICE

Must return 512 bytes of ATA identification data. Key fields:
- Words 27-46: Model number (40 chars)
- Words 10-19: Serial number (20 chars)
- Words 23-26: Firmware revision (8 chars)
- Words 60-61: Total sectors (LBA28)
- Words 100-103: Total sectors (LBA48)
- Word 106: Physical/logical sector size
