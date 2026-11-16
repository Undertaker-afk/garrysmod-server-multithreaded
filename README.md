# SCPRust: experimental Rust server reimplementation

An **experimental, partial server-logic reimplementation** targeting a Debian
SCP:RP server. Rust now owns a reusable world model, custom NPC decision policy,
graph navigation, and custom relevance calculations—not just a radius helper.
There are three distinct executables:

- **`scprust-server`**: a standalone, dependency-free Rust UDP query prototype;
  it is not an addon and does not launch or wrap srcds.
- **`scprust-map`**: offline BSP v20 inspection and world-brush collision probes;
  this does not activate a map in the UDP process.
- **`scprust-workers`**: the earlier HTTP gameplay-offload service, still available
  for experiments alongside GMod.

This is **not a drop-in replacement for srcds**, a complete implementation of
GMod's client protocol, or proof of 150-player capacity. Existing Helix code is not
automatically migrated. Source is already native code; Rust does not inherently
make it faster or guarantee future performance. Algorithms, batching, integration,
and measurement determine whether a rewrite helps.

## Standalone server milestone: connectionless UDP

The replacement-server path now has a bounded Source A2S query codec and a real
UDP process with challenge tokens, source-IP limits, and bounded peer state.
INFO, PLAYER and RULES exchanges were tested with the independent `python-a2s`
client. It advertises `[QUERY ONLY]` and **zero playable slots**. There is no
client sign-on, Steam authentication, map loading, or gameplay support yet.

```sh
cargo run -p scprust-server -- --bind 127.0.0.1:27025 --name SCPRust
# In another terminal:
python3 tools/query_probe.py --host 127.0.0.1 --port 27025
```

Only loopback binds are permitted. Do not replace the live srcds process with
this prototype. **[Standalone server guide, protocol provenance, and compatibility
ledger](docs/server.md)** documents the implemented subset and the missing pieces.
The probe can also collect ordinary A2S metadata from your authorized original
server to establish shareable test evidence; no decompilation is required for
this milestone.

The core and standalone process can be built and socket-tested without Cargo or
registry access (requires `rustc` and Python 3):

```sh
tools/test-standalone.sh
```

## Engine-facing milestone: BSP data, brush entities, spawns, movement, bitstreams

Added offline BSP v20 entity/geometry parsing, bounded iterative BSP traversal,
world + static `*N` brush-entity point and AABB clipping, `info_player_*`
spawn extraction/validation, a bounded deterministic AABB movement controller,
contents masks, and ordered parallel trace batches. Unsupported
layouts/compression and malformed references fail closed. Displacements, props,
moving entities, Source movement/trace parity, and playable map activation are
**not implemented**. The UDP process remains query-only.

```sh
cargo run -p scprust-server --bin scprust-map -- inspect /path/to/map.bsp
cargo run -p scprust-server --bin scprust-map -- models /path/to/map.bsp
cargo run -p scprust-server --bin scprust-map -- spawns /path/to/map.bsp
cargo run -p scprust-server --bin scprust-map -- ray /path/to/map.bsp 1 -100 0 64 100 0 64
```

Checked LSB-first bit fields, signed fields, canonical LEB128, and byte strings
are also available as library primitives, **not a client sign-on/netchannel**.
See [supported formats, exclusions, tests, and provenance](docs/maps.md).

**Whole-server completion is not claimed to be 30% or 50%.** The scope and
acceptance criteria are not established, and no GMod client can join yet. The
[acceptance ledger](docs/progress.md) distinguishes tested components from
missing end-to-end compatibility instead of inventing a percentage.

## Existing gameplay-offload boundary

| Rust core (`crates/scprust-core`) | Still owned by GMod / Helix |
| --- | --- |
| Immutable generation-aware entity snapshots and reusable spatial index | Live entities, physics, collision, traces, maps |
| Atomic, versioned world publication with optimistic concurrency | Lua execution, hooks, engine userdata |
| Nearest-enemy selection, target retention, idle/chase/attack-candidate policy | Authoritative movement, damage, access checks |
| Directed weighted graph routing with bounded Dijkstra searches | Source navmesh import, doors, hull clearance, dynamic obstacles |
| Bounded nearest-first relevance sets for custom addons | Source replication, PVS, client protocol and Steam services |

The core forbids unsafe code and has no default third-party dependencies; Serde
is optional. It does not depend on HTTP, Tokio, Rayon, Lua, or engine ABIs.
The service parallelizes independent core jobs using a fixed Rayon pool, so the
same core can later be hosted in a native GMod module without rewriting it.
There is no autonomous simulation loop: requests compute against a published
snapshot; they do not advance physics or apply intents.

```text
GMod main thread                           Rust
capture primitive entity snapshot  ──────> validate + build world/index
                                      ┌──> parallel NPC decisions
publish once, reuse same revision ─────┼──> parallel route searches
                                      └──> parallel custom relevance queries
validate age + identity             <───── versioned results
trace / recheck / apply explicitly
```

One service owns **one world**, with one integration responsible for publication.
Stale revisions and previous-process epochs return HTTP 409. Publishing a world
is atomic; invalid or racing updates cannot partially change it. In-flight reads
retain their old immutable snapshot and return its version, never a mixed frame.
Navigation can be retained across uploads instead of being resent each time.
See **[the core API and integration guide](docs/core.md)** for the full contract.

### Resource and integration limits

- Four compute workers by default; two admitted CPU jobs shared by all endpoints.
  Saturated requests receive HTTP 429 instead of growing a CPU-job queue.
- 2 MiB request bodies; 8,192 entities, 4,096 graph nodes, 32,768 directed edges.
- Up to 128 independent items per think/route/interest batch; bounded route
  expansion and output; at most 512 relevance results per observer.
- Loopback-only HTTP and server-only bearer credentials; no engine pointers
  cross threads or processes. Lua Lanes has the same engine-isolation constraint.
- Lua core operations are serialized, time out at two seconds, and require
  decisions/relevance results to be within 0.5 seconds of snapshot capture.
  The core bridge caps responses at 4 MiB. Timeouts/conflicts invalidate local
  session state: reconnect and republish.

HTTP/JSON is a simple ABI-independent starting transport, not a zero-copy design.
Snapshot capture and encoding still cost GMod main-thread time. A native
`ents.FindInSphere` can be much cheaper than a round trip. Offload large custom
batches rather than wrapping small engine functions or copying every entity each
tick. No timers, global hooks, or engine functions are automatically replaced.

## Build and test

Use a current stable Rust toolchain with Cargo. Python 3 runs HTTP tests; Lua
5.1 runs mocked bridge tests. No GMod installation is needed for those checks.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test -p scprust-core --no-default-features
cargo build --workspace --bins
python3 tests/http_smoke.py target/debug/scprust-workers
python3 -m unittest discover -s tests -p test_query_probe.py
python3 tests/query_smoke.py target/debug/scprust-server
python3 tests/map_smoke.py target/debug/scprust-map
lua5.1 tests/bridge_test.lua
lua5.1 tests/core_bridge_test.lua
cargo build --release --bin scprust-workers
```

The engine-independent core can also be tested using only `rustc`, without
Cargo, registry access, or any external crate:

```sh
rustc --edition 2021 --test crates/scprust-core/src/lib.rs -D warnings -o /tmp/scprust-core-tests
/tmp/scprust-core-tests
```

A [manual CI template](docs/ci-check.yml) includes workspace and HTTP checks.
It is not installed: the configured GitHub App cannot create workflows without
additional permission. An authorized repository owner can review/copy it to
`.github/workflows/check.yml`. No remote CI success is claimed.
Core tests cover concurrent CAS
publication, snapshot isolation, identity reuse, spatial queries against a brute
force reference, AI behavior, graph lifecycle, and weighted routing against a
Floyd-Warshall reference. Mock Lua tests cover request lifecycle, stale/invalid
results, identity reuse, empty JSON arrays, and uncertain-commit recovery.
These tests do **not** validate actual GMod/Helix loading or real performance.
There is not yet a committed Cargo lockfile; generate and pin one before deployment.

### Compute benchmark

```sh
cargo run --release --bin core-bench
```

Compares one versus four Rayon threads on a synthetic scene with 150 players
and 512 custom actors (128 thinkers + 64 route requests). It checks identical
results and prints p50/p95/p99 computation latency. It excludes JSON, network,
GMod capture/application, engine physics, and real addon costs. It is **not** a
150-player capacity test or a speedup claim. Measure whole-server tick times,
dropped/stale jobs, and gameplay equivalence with the actual schema before
using any rewrite in production.

## Debian installation: offload service

The Rust service and GMod must run on the **same host and network namespace**.
If using containers, `127.0.0.1` inside one container does not reach another;
this version does not support a publicly exposed worker endpoint.

After building, install the executable and unit:

```sh
sudo install -m 0755 target/release/scprust-workers /usr/local/bin/scprust-workers
sudo install -m 0644 deploy/scprust-workers.service /etc/systemd/system/
```

Run the following addon/data commands as the game-server owner. Adjust `GMOD`
to the directory containing `srcds_run` and the `garrysmod` subdirectory:

```sh
GMOD=/srv/gmod
mkdir -p "$GMOD/garrysmod/addons/scprust" "$GMOD/garrysmod/data"
cp -a addon/. "$GMOD/garrysmod/addons/scprust/"

# One secret shared by systemd and the server-only Lua bridge; do not commit it.
TOKEN="$(python3 -c 'import secrets; print(secrets.token_hex(32))')"
(umask 077; printf '%s\n' "$TOKEN" > "$GMOD/garrysmod/data/scprust_token.txt")
printf 'SCPRUST_TOKEN=%s\nSCPRUST_THREADS=4\n' "$TOKEN" \
  | sudo install -m 0600 /dev/stdin /etc/scprust-workers.env
unset TOKEN
sudo systemctl daemon-reload
sudo systemctl enable --now scprust-workers
sudo systemctl status scprust-workers
```

Add **`-allowlocalhttp`** to the GMod server launch arguments, then restart GMod.
The addon reads its token once at load. Restart GMod after token rotation.
The service accepts only ASCII-alphanumeric tokens of at least 32 characters.
Do not send the token to clients, put it in replicated convars, or include it in
logs. Only trusted server-side addons should call this API. Enabling local HTTP
also allows other server-side Lua to reach local services: audit your addons.

The unprivileged service uses `DynamicUser=yes`; systemd reads the root-owned
environment file before starting it. GMod's token file must be readable by the
GMod process owner. Check service logs with:

```sh
sudo journalctl -u scprust-workers -n 100 --no-pager
curl --fail http://127.0.0.1:27185/health
```

Start with **4 compute workers**, not 16. Your 8-core/16-thread CPU still needs
CPU time for GMod's main thread, the OS, and other services. This is a starting
configuration, not a tuned recommendation; physical cores and SMT threads are
not interchangeable performance budgets. `SCPRUST_BIND` optionally changes the
loopback listener; if changed, update both `endpoint` in `sv_bridge.lua` and `baseURL` in `sv_core.lua` too.

### Optional Helix example

Copy `integrations/helix/rust_workers` into your schema's `plugins` directory.
With the addon installed, a connected admin can run `/RustNearby` in chat. It
queries players within 512 units of the admin's captured position, including
the admin. `/RustPlan` additionally connects, uploads a small player snapshot,
and previews one Rust decision. Neither command moves entities or applies damage.
**`/RustPlan` replaces the service's one world snapshot**: do not run it alongside
another integration that owns that world.

## Stateless radius API (backward compatible)

Call from **server-side** Lua:

```lua
local accepted, reason = SCPRust.QueryRadius(player.GetAll(), {
    {center = somePositionVector, radius = 1024, maxHits = 150},
    {center = anotherPositionVector, radius = 512, maxHits = 150}
}, function(results, err)
    if err then
        -- Skip this advisory update, retry later, or use a measured fallback.
        -- Do NOT immediately retry in a tight loop.
        return
    end

    for i, result in ipairs(results) do
        -- result.entities: surviving GMod entities matching query i's snapshot
        -- result.truncated: more matches existed than maxHits
        -- Recheck current state before performing any gameplay action.
    end
end)

if not accepted then
    -- No callback will run: bad arguments, missing token, or local backpressure.
    print(reason)
end
```

Both inputs must be dense arrays; invalid entities are skipped and duplicates
are removed. Coordinates/radii must be finite and within the documented Rust
limits (absolute coordinates ≤ 1,000,000; radius 0..1,000,000). `maxHits` defaults
to 512. Accepted calls invoke the callback once on success or failure, except
when the Lua state shuts down. Immediate submission/encoding failures may call
back before `QueryRadius` returns. Late HTTP callbacks are ignored after timeout.

**Results are historical.** An entity can move while Rust works. The bridge
checks age and identity, not current position or line of sight. Never use this
alone for damage, access control, visibility, or authoritative proximity checks.
It is not a synchronous drop-in replacement for `ents.FindInSphere`.

### Radius HTTP contract

`GET /health` is unauthenticated process liveness only (not a readiness or
capacity guarantee). `POST /v1/radius` requires `Authorization: Bearer <token>`.
Unknown JSON fields, duplicate/zero point IDs, invalid bounds, and excessive
batch counts are rejected. Successful responses are ordered like the queries.

```json
{
  "points": [{"id": 1, "position": [0, 0, 0]}],
  "queries": [{"center": [1, 0, 0], "radius": 10, "max_hits": 50}]
}
```

Response:

```json
{"results": [{"ids": [1], "truncated": false}]}
```

Errors use HTTP status codes: 400 invalid input, 401 invalid token, 409 world version conflict, 413 oversized
body, 429 compute slots busy, 500 worker failure. Error bodies are plain text.
The compute permit stays with an admitted job even if its client disconnects;
the worker does not cancel already-running computation. Request-body and socket
handling are not designed to withstand hostile local connection floods: do not
expose the endpoint through a proxy or port forward.

## Remaining work toward a replacement server

The current useful scope is replacing **custom gameplay calculations** behind an
opt-in integration. Your actual SCP:RP/Helix schema is needed to preserve its
semantics and replace its real hotspots; none was included in this repository.

A binary-compatible replacement would additionally need the client protocol,
Lua/native binding compatibility, physics/entity semantics, map/asset support,
replication, and platform integration. None of these are faked by the core.
A native job-queue module could remove HTTP overhead while reusing this crate,
but must target a known GMod build and architecture. Engine-level patches need
the same build-specific validation and profiling.

No proprietary server code or decompiled material is included. This partial
reimplementation does not require decompilation.
