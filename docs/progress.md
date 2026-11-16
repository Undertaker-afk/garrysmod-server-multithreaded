# Replacement-server acceptance ledger

Status as of 2026-11-16. **No honest whole-server completion percentage is
available.** There is no pinned GMod build, complete compatibility inventory,
weighted work estimate, or end-to-end acceptance suite. Passing a fraction of
rows below would not establish "30% implemented"; the remaining systems differ
greatly in size/risk. This repository is not represented as 30% of a playable
GMod server and does not yet support a single connecting GMod player.

| Boundary | Implemented evidence | Missing acceptance evidence |
| --- | --- | --- |
| Connectionless discovery | Bounded standalone UDP; A2S INFO/PLAYER/RULES with independent Python client | Valve browser and target GMod build comparison |
| Bit primitives | Checked fields, signed values, LEB128, byte strings; synthetic vectors | Build-specific packet framing, sequence/reliability/compression |
| Map data | Offline v20 directory/entities/planes/nodes/leaves/models/brush subset | Real-map corpus, other versions/compression, game-world activation |
| Static collision | Brush-only point/AABB clipping; BSP broadphase; ordered parallel batches | Source differential tests, bevel completeness, displacements, props, dynamic entities |
| Custom gameplay core | Immutable CAS snapshots, AI policy, weighted routing, custom relevance | Actual Helix/schema semantics and integration/performance evidence |
| Authentication | No success responses fabricated | Normal Steam registration/ticket validation and lifecycle |
| Client sign-on | None | Compatible authenticated client, downloads, string tables, baselines |
| Replication/player spawn | None | Unmodified client loads supported map, spawns, receives valid snapshots |
| Lua compatibility | Server-side bridge examples only | LuaJIT behavior, GMod bindings, hooks, userdata and native-module ABI |
| Physics/movement | No Source simulation replacement | Hull movement, stepping, gravity, constraints, vehicles, collision parity |
| Helix/SCP:RP | Optional read-only admin demos | Schema/addon inventory, character/inventory/database behavior regression suite |
| Capacity/reliability | Bounded prototypes and synthetic compute benchmark | 150-player workload, tick-time distributions, soak/reconnect/loss tests |

## Checks currently reproducible locally

`tools/test-standalone.sh` builds real Rust executables without registry access,
runs the core and server unit tests, six probe tests, UDP smoke exchange and
map-inspection/collision CLI smoke tests. `SCPRUST_TEST_A2S=1` additionally enables
the pinned independent query client. Lua bridge tests can run separately under
Lua 5.1. The [manual CI template](ci-check.yml) additionally specifies full Cargo
workspace and HTTP checks. The GitHub App rejected workflow installation because
it lacks `workflows` permission; the template is committed outside the Actions
workflow directory and cannot run automatically. An authorized owner may review
and install it. No remote CI check has run or passed for this milestone.

Local validation: 43 native Rust tests, six Python probe tests, real UDP smoke
including `python-a2s==1.4.2`, offline map CLI smoke, and both Lua 5.1 mock suites
passed. Direct native Clippy/format checks passed for the new server components.
The full dependency-bearing Cargo/HTTP build remains unverified; this is not
hidden behind the local standalone test result.

Synthetic BSP/packet fixtures establish internal contracts, **not actual Source
or GMod behavior**. No real map, proprietary server binary, authentication ticket,
addon secret, or live capture is committed. The new map parser and collision
library are offline components; the UDP server remains query-only.

## Next externally meaningful gates

1. Pin server build/architecture and collect authorized, minimized connection
   fixtures. Inventory required Lua/native APIs from the actual schema/addons.
2. Implement and test normal authenticated connection lifecycle plus bounded
   netchannel delivery against that target—without bypassing authentication.
3. Produce the first unmodified-client sign-on, map load, and player spawn test.
4. Validate map/collision behavior against a legal representative real-map corpus
   and target-engine trace results, documenting unsupported collider types.
5. Integrate simulation/replication and Lua progressively; prove addon semantics
   and whole-server performance rather than extrapolating from Rust microbenchmarks.

Instructions and provenance: [standalone queries](server.md), [map/bitstream
subset](maps.md), [custom offload core](core.md).
