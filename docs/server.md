# Standalone replacement-server path

`scprust-server` is a **separate Rust executable**, not an addon or a wrapper
around `srcds`. Its network endpoint implements only a small connectionless UDP query
subset. Offline BSP/brush/bitstream tools are described in [maps.md](maps.md). It cannot host a game, accept a GMod player, run Helix, or replace your
live server. Keep your existing server running.

This is the first wire-compatibility milestone. A successful A2S query is **not**
a successful GMod connection. No engine binaries, decompiled code, Steam tickets,
or real-server packet captures are included.

## Run locally on Debian/Linux

With a stable Rust toolchain:

```sh
cargo run -p scprust-server -- --bind 127.0.0.1:27025 --name SCPRust
```

In another terminal:

```sh
python3 tools/query_probe.py --host 127.0.0.1 --port 27025
```

The executable uses a different port from the typical live-server port and
rejects every non-loopback bind, including `0.0.0.0`. Numeric IPv6 loopback is
also supported (`--bind '[::1]:27025'`); use `--host ::1` for the probe. Port 0
selects an ephemeral UDP port, printed as `QUERY_ONLY_READY <address>`.
`--help` lists the intentionally small set of options. Stop with Ctrl+C.
Linux `/dev/urandom` is required for query tokens; entropy errors fail closed.

Do not port-forward or proxy this endpoint publicly. No master-server listing,
Steam registration, RCON, or server administration protocol is implemented. The
existing `deploy/scprust-workers.service` runs the **other** executable (the
experimental offload service), not this standalone prototype.

### Test without Cargo/registry access

The new server crate has no third-party dependencies. The script below compiles
and executes both Rust core and transport tests, builds the actual standalone
executable, tests its UDP socket, and removes temporary build products:

```sh
tools/test-standalone.sh
```

It requires `rustc`, Python 3, and ordinary Debian shell tools. `RUSTC` and
`PYTHON` can select specific executable paths. This bypasses workspace dependency
resolution when a Cargo registry is unavailable. Alternatively:

```sh
cargo test -p scprust-server
cargo build -p scprust-server
python3 -m unittest discover -s tests -p test_query_probe.py
python3 tests/query_smoke.py target/debug/scprust-server
```

Independent query-client interoperability is an explicit additional check:

```sh
# Install into your preferred virtual environment, not the server source tree.
python3 -m pip install python-a2s==1.4.2
SCPRUST_TEST_A2S=1 python3 tests/query_smoke.py target/debug/scprust-server
```

The [manual CI template](ci-check.yml) includes this independent-client test;
it is not installed or executed remotely. It verifies A2S info, empty player
lists, and empty rules; it does not test Valve's browser or a GMod client.

## Implemented wire contract

Only single-packet modern Source queries are accepted. The four-byte packet
header is `ff ff ff ff`. Requests must have exact lengths; trailing bytes,
truncated payloads, split packets, unknown commands, and non-query traffic are
silently dropped. Received oversized datagrams cannot be truncated into an
accepted short query. Replies stay below 1,200 bytes and need no fragmentation.

| Request type | Payload after header | Initial reply | Verified reply |
| --- | --- | --- | --- |
| A2S_INFO (`54`) | `Source Engine Query\0`, optionally four token bytes | `41` + token | `49` + info fields |
| A2S_PLAYER (`55`) | four token bytes (typically initially `ff ff ff ff`) | `41` + token | `44 00` (empty list) |
| A2S_RULES (`56`) | four token bytes | `41` + token | `45 00 00` (zero rules, little-endian u16) |

Info metadata is deliberately synthetic and unmistakable:

- Name is always prefixed with `[QUERY ONLY]`; custom labels are bounded and
  cannot contain NULs or other control characters.
- Map is `no_map_loaded`; folder is `garrysmod`; app ID is the **target** app ID
  4000, not a compatibility certification.
- Players, playable slots, and bots are zero. The process has no gameplay slots.
- VAC is false. No Steam server identity or security status is fabricated.
- Version identifies `scprust-query`, not a GMod build.
- Protocol field 17 is a conventional Source-query metadata placeholder. It is
  **not a negotiated GMod netchannel/sign-on protocol**, and must not be used to
  select a client build. Extra-data flags are zero.

### Challenge and resource model

Every query type, including INFO, needs a valid source-bound four-byte token
before returning metadata/list data. Tokens come from OS entropy, are tied to
both source IP and source UDP port, and expire 30 seconds after issue. Live
tokens are reused across reordered queries from the same socket. Zero and
`ff ff ff ff` are excluded as generated tokens for client interoperability.

There are at most 1,024 challenge entries and 1,024 source-IP windows. Fixed
process-second limits admit at most 100 valid query requests globally and 20
per source IP, including challenge requests. Port rotation does not bypass the
IP limit. Window boundaries can permit back-to-back bursts; this is not a
rolling-window guarantee. Unexpired entries are not evicted to make space for
new sources: a full cache drops new work; expired entries are reclaimed.

Malformed traffic allocates no peer state. No task is spawned per packet and no
application request queue grows with traffic. Query I/O is serial because these
small control-plane packets are not useful CPU-parallel work; this is not a
multithreaded gameplay loop. Periodic aggregate counters do not log raw packets,
player data, or addresses. The kernel socket receive buffer remains OS-managed.

These measures limit state and query amplification, **not** DDoS-proof the
process. Four-byte A2S tokens are not account authentication, session encryption,
or authorization. This prototype stays loopback-only, including after tests.

## Observe your original server without decompilation

A standard A2S query can collect public metadata from your own authorized server:

```sh
# On the host running your original server, using its actual query port:
python3 tools/query_probe.py --host 127.0.0.1 --port 27015 \
  --output original-a2s.local.json
```

The probe targets **one endpoint**, uses a connected UDP socket, has bounded
challenge retries/timeouts, and never attempts login or sends credentials.
It parses modern single-packet INFO plus known optional EDF fields. It rejects
GoldSrc, split/compressed packets, unknown flags, and unparsed trailing bytes
rather than pretending to understand them. Unsupported servers produce an error.

The JSON report contains parsed fields plus the raw INFO packet in hex. Existing
output files are not overwritten. Reports may expose names, maps, IP addresses,
keywords, or an optional server Steam ID: review/redact them before sharing.
Local reports named `*.local.json` are ignored by Git. Keep raw captures and
proprietary binaries out of Git; only add minimal shareable/sanitized fixtures.

The advertised protocol/version does not fully identify the GMod server branch,
architecture, or negotiated connection protocol. Also record console `version`,
process bitness, installed addons/schema versions, and the exact build being
compared. Those inputs are still missing from this project.

## Compatibility milestone ledger

No whole-server percentage (including 30%) is claimed; see the
[acceptance ledger](progress.md) for the scope and missing evidence.

| Capability | Status / required evidence |
| --- | --- |
| Bounded standalone UDP process | Implemented and socket-tested locally |
| Modern A2S INFO / PLAYER / RULES | Implemented query-only subset; independent `python-a2s` check |
| Exact target GMod build | Unknown; no live-server build information supplied |
| Connection handshake and normal Steam authentication | Not implemented |
| Netchannel, reliable delivery, sign-on and downloads | Not implemented |
| Offline map/brush data and collision probes | Partial BSP v20 world-brush subset; synthetic fixtures only ([details](maps.md)) |
| Map activation, entity replication, player spawn | Not implemented |
| Bitstream fields | Checked library primitives only; not netchannel framing |
| LuaJIT/GMod engine API and Helix compatibility | Not implemented |
| Game-client interoperability | Not tested; a client cannot join this prototype |
| 150-player throughput or speedup | Not established; no capacity claim |

Next: pin a real build and produce minimal, authorized handshake fixtures with
byte-level expected outcomes. Implement the normal authenticated connection
state machine and bounded netchannel handling against those fixtures, then
validate sign-on/map/spawn with an unmodified client. Do not fabricate a
successful Steam authentication response to advance a connection state. Reliable
channels and packet parsing need limits, replay handling, and malformed-input
tests before exposing any game protocol publicly.

The separate `scprust-core` algorithms may eventually back gameplay simulation,
but they do not currently feed an engine-compatible replication pipeline.

## Public-format provenance

This is an independent implementation of protocol field layouts, not a port of
proprietary engine code. The following public MIT-licensed query client was
reviewed as an interoperability cross-reference at commit
`6d21dfbdb27d5cfbf48f13faea87c01c4e3213aa`:

- [Info layout and optional fields](https://github.com/Yepoleb/python-a2s/blob/6d21dfbdb27d5cfbf48f13faea87c01c4e3213aa/a2s/info.py)
- [Single-packet header and challenge exchange](https://github.com/Yepoleb/python-a2s/blob/6d21dfbdb27d5cfbf48f13faea87c01c4e3213aa/a2s/a2s_sync.py)
- [Player query layout](https://github.com/Yepoleb/python-a2s/blob/6d21dfbdb27d5cfbf48f13faea87c01c4e3213aa/a2s/players.py)
- [Rules query layout](https://github.com/Yepoleb/python-a2s/blob/6d21dfbdb27d5cfbf48f13faea87c01c4e3213aa/a2s/rules.py)
- [Upstream license](https://github.com/Yepoleb/python-a2s/blob/6d21dfbdb27d5cfbf48f13faea87c01c4e3213aa/LICENSE)

No upstream implementation files are vendored. Golden packets and malformed
fixtures in this repository are synthetic. Live GMod behavior and any additional
public-source or reverse-engineered findings must be recorded with exact build,
source/license, and test provenance; discovery compatibility must not be promoted
to gameplay compatibility without separate evidence.
