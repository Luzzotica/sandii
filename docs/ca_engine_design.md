# High-Performance 64-Bit Cellular Automata Engine

## 1) Core philosophy

This engine uses a hybrid architecture:

- Infinite-world chunk addressing through a spatial hash map.
- Lock-free, phase-partitioned multithreading.
- Strict 8-byte cell packing and 64x64 chunk sizing to align with L1 cache behavior.

Primary performance goals:

- Keep hot simulation data in contiguous chunk memory.
- Maximize local-index arithmetic and branch predictability.
- Avoid per-cell heap churn by using chunk object pooling.

## 2) 64-bit cell structure

Each simulated cell is exactly 64 bits (`8 bytes`).

### Canonical bit layout

| Bits   | Width | Field           | Description                                |
| ------ | ----: | --------------- | ------------------------------------------ |
| 0..9   |    10 | `type`          | Primary material id (`0..1023`)            |
| 10..19 |    10 | `ctype`         | Secondary/previous material id (`0..1023`) |
| 20..23 |     4 | `vel_x`         | Signed nibble (`-8..+7`)                   |
| 24..27 |     4 | `vel_y`         | Signed nibble (`-8..+7`)                   |
| 28..35 |     8 | `lifetime_data` | General purpose timer (`0..255`)           |
| 36..37 |     2 | `frame_tracker` | Last processed frame modulo 4 (`0..3`)     |
| 38..63 |    26 | `flags`         | Runtime boolean flags (expanded capacity)  |

Total: `10 + 10 + 4 + 4 + 8 + 2 + 26 = 64`.

### Bitfield diagram

```text
63                                                             0
+----------------------------+--+--------+----+----+----------+----------+
|         flags (26)         |fr|lifetime|vy  |vx  | ctype10  |  type10  |
+----------------------------+--+--------+----+----+----------+----------+
 63                        38 37 36    35 34 31 30 27 26    19 18       0

fr = frame_tracker (2 bits)
vx/vy = signed 4-bit values with sign extension on read
```

### Invariants

- Cell memory size is always exactly `8 bytes`.
- `type` and `ctype` are each capped to `0..1023`.
- `frame_tracker` is modulo-4 and must prevent same-frame double-processing.
- Chunk size is fixed to `64x64` cells (`4096` cells, `32768` bytes per chunk).

## 3) World management and memory

### Chunk object

- One chunk contains a contiguous `Cell[4096]`.
- Chunk metadata includes activity/sleep state, dirty markers, and optional counters (for example, active cell count).

### Object pool

- Engine startup preallocates chunk storage blocks.
- Newly discovered chunks are checked out from the pool.
- Unloaded chunks are reset and returned to the pool.
- This avoids runtime allocator fragmentation and GC-like stalls.

### Spatial hash map

- Active world chunks are addressed by integer chunk coordinates (`ChunkCoord`).
- Hash lookup resolves chunk handles in expected `O(1)` time.
- Rendering and simulation iterate only loaded/active chunks.

## 4) Multithreading architecture (4-phase lock-free)

### Global frame loop

1. `current_frame = (current_frame + 1) & 0b11`
2. Phase 0: process chunks at evenX-evenY
3. Barrier
4. Phase 1: process chunks at oddX-evenY
5. Barrier
6. Phase 2: process chunks at evenX-oddY
7. Barrier
8. Phase 3: process chunks at oddX-oddY
9. Barrier

No two simultaneously processed chunks are adjacent (including diagonals), eliminating write-write conflicts on neighborhood operations without per-cell mutexes.

### Boundary double-processing prevention

During update:

```text
if cell.frame_tracker == current_frame:
    skip
else:
    simulate
    write target frame_tracker = current_frame
```

Any move/transformation that transfers ownership to another location must tag the destination with `current_frame` so it is not stepped again this frame.

## 5) Mathematical operations

### Local coordinate fast path

Most movement and reaction logic stays chunk-local:

- Indexing with 1D offsets (`+1`, `-1`, `+64`, `-64`, diagonals).
- No world-space conversion for interior cells.

### Global boundary path

When local coordinates touch chunk borders (`x=0|63`, `y=0|63`):

- Convert to neighbor chunk coordinate.
- Resolve chunk in spatial hash.
- Compute destination local index in neighbor chunk.
- Perform cross-chunk injection/move under phase-safe rules.

## 6) Migration assumptions and implementation constraints

- The packed cell representation is the source of truth for simulation state.
- Existing behavior can be bridged through adapters during migration, but final stepping must use packed accessors directly.
- Deterministic four-phase ordering is required for lock-free correctness reasoning.
- Persistence formats should be versioned when layout/storage changes are introduced.
