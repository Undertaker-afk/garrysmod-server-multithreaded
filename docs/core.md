# Versioned Rust gameplay core

This is newly implemented custom server policy, **not a behavioral clone of
Source NPC AI or Helix**. Integration is opt-in. The one-world model is for one
game-server process; do not share a service between multiple independent servers.

## Concurrency / ownership

1. `GET /v1/world` returns `{version: {epoch, revision}, entities, nodes}`.
2. Publish using that exact version as `base`. The service builds a replacement
   world/index without holding the world lock, then compare-and-swaps it under a
   short write lock. Exactly one competing publisher wins. All others get 409.
3. Query the returned version. Each batch holds an `Arc` of exactly that world;
   another upload cannot mutate it. Queries admitted before a replacement may
   finish after it, but always return their original version.
4. A later request for a superseded version gets 409. No unbounded world-history
   cache exists. At most the latest world and admitted jobs' snapshots remain.
5. A random process epoch changes after service restart; revision starts at zero.
   There is no disk persistence or automatic migration of state. Revision is a
   u32, exactly representable by Lua numbers; overflow rejects publication.

All POST endpoints share the two-job semaphore. Permits remain with running CPU
jobs if the HTTP caller disconnects. Work is bounded, not preemptively cancelled.
The HTTP server itself is not hardened against hostile local socket floods.
Only trusted server-side code should use it.

## HTTP API

All endpoints below require `Authorization: Bearer <SCPRUST_TOKEN>` and JSON
bodies for POSTs. Unknown JSON fields are rejected. Errors are plain text:
400 validation, 401 credentials, 409 version conflict, 413 body size, 429 busy,
500 internal error. `/health` remains unauthenticated process liveness only.

### POST /v1/world

```json
{
  "base": {"epoch": "PROCESS_EPOCH_FROM_GET", "revision": 0},
  "entities": [
    {"key": {"id": 1, "generation": 7}, "position": [0, 0, 0], "faction": 1, "alive": true},
    {"key": {"id": 2, "generation": 3}, "position": [500, 0, 0], "faction": 2, "alive": true}
  ],
  "navigation": [
    {"id": 10, "position": [0, 0, 0], "edges": [{"to": 20, "cost": 500}]},
    {"id": 20, "position": [500, 0, 0], "edges": []}
  ]
}
```

Returns the same metadata shape as GET, at revision 1. Entity IDs are positive
u32 slots; generations are u32; only one generation per slot is allowed in a
snapshot. Factions are application-defined u16 values, not automatically Helix
faction indices. Define and document your schema's faction mapping.

At most 8,192 entities; coordinates must be finite and within ±1,000,000. Entity
positions are still supplied by GMod, not simulated in Rust. `navigation` omitted
or null retains the current immutable graph; `[]` clears it. On a map change,
explicitly replace or clear the graph; there is no automatic map-name detection. Nodes are positive
unique u32 IDs with bounded positions. Each directed edge must reference a node,
have a unique target per source, and cost in (0, 1,000,000,000]. Maximums: 4,096
nodes, 32,768 total edges. Edges need not be geometrically weighted, so searches
use Dijkstra rather than an inadmissible Euclidean A* heuristic. Node positions
are metadata: there is no automatic navmesh import or nearest-node attachment.

### POST /v1/think

```json
{
  "version": {"epoch": "PROCESS_EPOCH_FROM_GET", "revision": 1},
  "items": [{
    "actor": {"id": 1, "generation": 7},
    "perception_radius": 2048,
    "attack_radius": 64,
    "move_speed": 160,
    "target_factions": [2],
    "current_target": null
  }]
}
```

`items` must contain 1..128 agents. Actors must exist with the exact generation.
Radii are finite in 0..1,000,000; attack radius cannot exceed perception radius.
Movement speed is finite in 0..10,000; 1..32 target-faction entries are required.

Policy for each agent:

- Dead actor: idle.
- Preserve `current_target` if still alive, of an allowed faction, and in radius.
- Otherwise select the nearest alive eligible entity, excluding the actor;
  distance ties use entity identity for deterministic output.
- No target: `idle`, null target/distance, zero desired velocity.
- Target in attack radius: `attack_candidate`, zero desired velocity.
- Otherwise: `chase`, desired velocity pointing directly at the target at the
  configured speed. This is **not collision-aware path following**.

Response: `{version, results: [{actor, target, action, desired_velocity, distance}]}`.
Output order matches input order. There is no server-held brain history: pass the
last target back explicitly if target retention is desired. The immutable world
and pure per-agent computation allow parallel execution without shared AI writes.

**Perception is proximity only.** It does not account for walls, PVS, FOV, hiding,
SCP rules, invisibility, doors, character permissions, or friendly-fire rules.
`attack_candidate` never authorizes or applies damage. Those engine/schema checks
must happen on the GMod main thread immediately before a gameplay action.

### POST /v1/routes

```json
{
  "version": {"epoch": "PROCESS_EPOCH_FROM_GET", "revision": 1},
  "items": [{"start": 10, "goal": 20, "max_visited": 4096}]
}
```

1..128 independent queries. Both node IDs must exist; `max_visited` is 1..4096
settled nodes per query. Response: `{version, results: [{status, nodes, cost,
visited}]}`. Status is `found`, `unreachable`, `budget_exceeded`, or
`path_too_long`. Found paths include endpoints, have at most 256 nodes, and carry
a total cost. Other statuses have empty nodes and null cost; they are not partial
paths. The graph is directed: upload reverse edges explicitly if appropriate.

Routes are graph-level plans, not a guarantee an NPC's collision hull can pass.
Dynamic doors, obstacles, jumps, ladders, and changes in access permission must
be handled in the supplied graph or checked by the engine-side integration.

### POST /v1/interest

```json
{
  "version": {"epoch": "PROCESS_EPOCH_FROM_GET", "revision": 1},
  "items": [{"actor": {"id": 1, "generation": 7}, "radius": 2048, "max_entities": 150}]
}
```

1..128 observers, finite radius 0..1,000,000, 1..512 entities per result. Response:
`{version, results: [{actor, entities: [{id, generation}], truncated}]}`. Excludes
the observer and orders nearest-first with identity tie-breaking. It includes
both alive and dead entities, regardless of faction. This is **only a candidate
relevance set for custom addons**, not visibility, permission checking, security,
or a replacement for Source's networking/replication system.

## GMod / Helix API

The autorun loader installs `SCPRust.Core`. All calls are server-only. Each returns
`true` when accepted and invokes `callback(result, error)` exactly once, unless
Lua shuts down. Immediate transport failure can call back before the function
returns. `false, reason` means rejected with no callback. Handle both paths.

```lua
local core = SCPRust.Core
local function submitted(accepted, reason)
    if not accepted then ErrorNoHalt(tostring(reason) .. "\n") end
end

submitted(core.Connect(function(info, err)
    if err then ErrorNoHalt(err .. "\n"); return end
    -- Fill from your schema. 'npc' and 'target' are example live entities.
    submitted(core.Publish({
        {entity = npc, faction = 1, alive = true},
        {entity = target, faction = 2, alive = true}
    }, function(worldInfo, publishError)
        if publishError then ErrorNoHalt(publishError .. "\n"); return end
        submitted(core.Think({{
            entity = npc, perceptionRadius = 2048, attackRadius = 64,
            moveSpeed = 160, targetFactions = {2}
        }}, function(intents, thinkError)
            if thinkError then return end -- skip, don't tight-loop retry
            local intent = intents[1]
            -- intent.actor / target: live entities resolved by generation
            -- intent.action: idle, chase, attack_candidate
            -- intent.desiredVelocity: Vector, NOT automatically applied
            -- Trace, recheck positions/rules, then explicitly decide what to do.
        end))
    end)) -- optional third argument: navigation nodes
end))
```

`Core.Publish(rows, callback, navigation)` takes node positions as **Vectors**,
with Lua node shape `{id, position, edges = {{to, cost}, ...}}`. Navigation nil
retains the graph; `{}` explicitly clears it. Empty arrays are correctly encoded
as `[]`, unlike GMod's default empty-table encoding.

`Core.Routes({{start = 10, goal = 20, maxVisited = 4096}}, callback)` returns raw
validated route results. It needs a successful publication even for an empty
entity snapshot. Store your authored graph positions to turn node IDs into
waypoints. Do not turn graph plans straight into teleportation.

`Core.Interest({{entity = observer, radius = 2048, maxEntities = 150}}, callback)`
returns `{actor, entities, truncated}` results with live entity references.

Only one core request is allowed in flight per Lua bridge. Connect does not
adopt existing entity references: publish your own complete snapshot first.
Snapshot age for decisions/relevance includes Lua capture and encoding; once
older than 0.5 seconds, republish. Routing does not enforce entity snapshot age
because it queries the retained navigation graph, but still enforces versions.
Deleted/recreated actors/targets cause conservative rejection of the batch.

HTTP errors, malformed responses, stale results, and timeouts invalidate local
version/entity state. A publication may have committed even if its reply was
lost. **Reconnect; do not guess the next revision or blindly retry an old body.**
There is no automatic reconnect/retry loop or engine fallback. Keep integration
behind an opt-in switch and skip advisory updates or use a measured safe fallback.

The separate legacy `SCPRust.QueryRadius` API is stateless and still works. It
has its own Lua in-flight limit but shares the service's two-job admission budget.
If both APIs are used concurrently, handle HTTP 429 and retry on a later update.
