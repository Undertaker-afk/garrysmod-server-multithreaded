# Offline Source BSP geometry, static brush entities, spawns and movement

The `scprust-server` library now has **offline** map inspection, entity-lump
parsing, BSP traversal, static brush-entity transform/trace support (`*N`
models), `info_player_*` spawn validation, and a bounded deterministic AABB
movement controller. The `scprust-map` executable exercises the inspection,
model, spawn, and trace components without loading proprietary server code.

**This is not playable map activation, full Source collision/movement parity, or
a replacement for `util.TraceLine`/`util.TraceHull`.** The UDP process still
advertises `no_map_loaded`, zero playable slots, and `[QUERY ONLY]`. Reading a
map with this tool does not change its network state. No map assets or real
captures are committed; tests generate small synthetic format fixtures at
runtime.

## Commands

Use only maps you have permission to inspect. Commands are local and never start
srcds, contact Steam, execute entity scripts, extract the embedded pakfile, or
write to the input map.

```sh
cargo run -p scprust-server --bin scprust-map -- inspect /path/to/map.bsp
cargo run -p scprust-server --bin scprust-map -- models /path/to/map.bsp
cargo run -p scprust-server --bin scprust-map -- spawns /path/to/map.bsp

# Explicit numeric contents mask; 1 selects CONTENTS_SOLID, not Source MASK_SOLID.
# Traces world + static brush-entity models by default; pass --world or --model N to narrow.
cargo run -p scprust-server --bin scprust-map -- ray /path/to/map.bsp 0x1 \
  -100 0 64 100 0 64

# Same segment, with an AABB relative to its moving reference position:
cargo run -p scprust-server --bin scprust-map -- hull /path/to/map.bsp 0x1 \
  -100 0 64 100 0 64 -16 -16 0 16 16 72
```

Success prints one JSON object. Metadata includes revision, entity/model/
solid-model/plane/node/leaf counts, the number of distinct world-model brushes,
whether the file declares displacement data, and extracted/valid spawn counts.
`models` lists resolved `*N` brush-entity transforms (`origin`, `angles`, brush
count). `spawns` lists deterministic `info_player_*` candidates with ground
validation and rejection reasons. Trace JSON includes fraction, end position,
normal (or null), brush index (or null), contents bits, tested-brush count,
`start_solid`, `all_solid`, explicit `collision_scope` (`world_and_static_brush_models`,
`world_brushes_only`, or `single_brush_model`), and `source_parity: false`.
Exit status 2 means command/input-coordinate error; status 1 means file/parser
failure. Contents masks accept decimal or `0x` hex u32 values, including zero.

## Exact supported format

Only **little-endian `VBSP`, file version 20**, standard 64-entry directory, and
these consumed lump versions/layouts are supported:

| Lump | ID | Version | Record bytes |
| --- | --- | --- | --- |
| Entities | 0 | 0 | byte text |
| Planes | 1 | 0 | 20 |
| Nodes | 5 | 0 | 32 |
| Leaves | 10 | 1 | 32 |
| Models | 14 | 0 | 48 |
| Leaf-brush indices | 17 | 0 | 2 |
| Brushes | 18 | 0 | 12 |
| Brush sides | 19 | 0 | 8 |

The reader rejects BSP 19/21 and other variants rather than guessing their
layouts. In particular, it does not reinterpret the older version-0 leaf layout
or alternative version-21 directories. Consumed compressed/nonzero-fourCC lumps
and LZMA markers are rejected. Other lumps are not decompressed, interpreted, or
semantically validated, but every directory range is bounds/overlap checked.
No guarantee is made that a file accepted by this subset is valid for Valve's
engine or vice versa.

Limits/validation:

- File input at most 256 MiB, including a bound on bytes read if a file grows.
- Nonempty lump ranges cannot overlap each other or the 1,036-byte header.
- At most 65,536 planes, 131,072 nodes, 65,536 leaves, 65,536 brushes,
  262,144 brush sides/leaf-brush indices, and 1,024 models.
- Plane vectors/distances and model vectors must be finite/bounded. Plane normal
  squared length must be in [0.99, 1.01]. Node, model, brush-side, and leaf-brush
  references/ranges are checked before traversal. Brushes have 4..128 sides.
- Iterative cycle detection covers every node, including unreachable/submodel
  nodes. Query traversal also deduplicates visited nodes, leaves, and brushes.
- Entity text is bounded to 8 MiB / 16,384 entities / 131,072 total properties /
  256 properties per entity / 4,096 bytes per key/value. Exactly one leading
  `worldspawn` is required. Quoted tokens, braces, `//` comments, escaped quote
  and backslash, and terminal NUL padding are supported. Duplicate keys and
  non-UTF-8 bytes are retained. This is not a general KeyValues implementation;
  unquoted tokens and scripts are not interpreted.

Malformed fixture tests include header/record truncation, overlap, negative and
oversized ranges, unsupported versions/compression, NaNs, cyclic nodes, invalid
children, and seeded byte mutations. Synthetic files are not claimed to be
playable Valve-compiled maps. **Real GMod-map differential tests are still
missing.** Use staging and compare against the actual target build before
relying on any results.

## Collision semantics, movement approximation, and exclusions

The world model's BSP headnode determines reachable world leaves and brushes.
Entity `model "*N"` references resolve static brush models with entity `origin`
and `angles` (`(forward, -right, up)` basis); `Map::trace` queries world brushes
only, `Map::trace_model` queries one model, and `Map::trace_solids` queries
world plus static brush-entity models (disabling broadphase filtering on
transformed models). A conservative whole-segment BSP broadphase gathers
candidate world brushes. The narrow phase clips the segment against declared
outward-facing brush half-spaces; point traces are exact mathematical
half-space intersections up to floating-point roundoff. Declared planes do not
prove the input geometry is a valid closed compiler-generated convex brush.

For AABB sweeps, planes expand by the hull's support projection. Bevel planes in
compiled brushes matter: without all needed bevels this can be conservative
(false-positive contacts). Both symmetric and asymmetric hull offsets are
supported. Touching a boundary counts as contact, with **no Source epsilon**,
collision group, surface flags, hitbox logic, or engine movement tolerances.
`start_solid` includes boundary starts; `all_solid` merges occupied intervals
across overlapping brushes and reports complete segment coverage. These are
this library's documented mathematical semantics, not Source parity claims.

`spawn::spawn_points` / `first_spawn` extract `info_player_*` entities in
deterministic `(classname, x, y, z)` order and validate them with a downward
player-hull trace against `trace_solids`. `movement::step` provides a bounded,
substepped (`dt <= 0.25s`, substeps `<= 1/60s`) AABB controller (gravity,
friction, ground/air acceleration, jumping, step climbing, slope threshold,
wall sliding, ground snapping). Both are offline approximations: no entity
I/O, triggers, spawn-room filters, ducking, water, ladders, bhop tuning, prop
physics, or client prediction are implemented.

Only brushes matching the explicit contents bitmask participate. The query is
not line-of-sight authorization or gameplay permission checking. It omits:

- Displacement triangle collision (presence is reported, geometry not decoded).
- Static props and embedded VPhysics meshes; dynamic entities, players, hitboxes.
- Moving doors/lifts/animated brush entities, collision groups and filters.
- Water/trigger-specific gameplay behavior, ducking, ladders, prediction.
- Physics simulation, materials/surface response, PVS, networking and replication.

Do not replace authoritative GMod collision with this prototype. Missing
colliders cause false negatives, and hull expansion may cause false positives.

### Native parallel batch API

`Map::trace_batch(&[Sweep], workers)` validates a maximum of 1,024 queries before
work, uses 1..16 explicitly requested workers, and preserves input order. A
single-worker call runs inline; multiworker calls use scoped OS threads joined
before returning. Queries share an immutable map and produce independent
results. Tests compare ordered parallel results with serial results.

Threads are created per batch, **not** a persistent engine scheduler. If already
running in a worker pool, call `Map::trace` there instead of nesting pools. There
are no measured production speedups or 150-player capacity claims. This API is
not exposed to untrusted remote packets or installed as an automatic GMod hook.

## Checked bitstreams

`bitstream` supplies bounded LSB-first fields (0..32 bits), signed two's-complement
fields, canonical unsigned LEB128 u32, and bounded byte C strings. Buffers cap at
64 KiB. Failed reads/writes, including compound varints/strings, leave the cursor
unchanged; writes preflight capacity before modifying bytes. Golden vectors,
widths/offsets, overflow, truncated, unterminated, and noncanonical inputs are
tested.

These primitives are **not a netchannel**, message-ID table, reliable protocol,
sequence/replay handler, packet compressor, or sign-on decoder. LEB128 is not
Source's separate `UBitVar`. Bit ordering is not evidence that a specific GMod
build's message framing has been implemented. No fabricated Steam authentication
or client connection response was added.

## Public-format provenance

Format/layout and bit-order cross-references were inspected in Valve's public
Source SDK at commit `e1f1fd42271805072f052051c26c21caa2c87b04`:

- [BSP directory and record layouts](https://github.com/ValveSoftware/source-sdk-2013/blob/e1f1fd42271805072f052051c26c21caa2c87b04/src/public/bspfile.h)
- [Bit-buffer declarations and bit ordering](https://github.com/ValveSoftware/source-sdk-2013/blob/e1f1fd42271805072f052051c26c21caa2c87b04/src/public/tier1/bitbuf.h)
- [Source 1 SDK license](https://github.com/ValveSoftware/source-sdk-2013/blob/e1f1fd42271805072f052051c26c21caa2c87b04/LICENSE)

The SDK is **not MIT-licensed or unrestricted**. No SDK implementation files or
assets are vendored, and this code is independently written Rust reading format
fields—not a copied/translated engine collision routine. Public availability is
not permission to redistribute proprietary engine code. Any future use of SDK
implementation material must be assessed against its actual license; any
reverse-engineered findings need exact build, provenance, and shareable tests.
