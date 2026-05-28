# ADR-001: Process Topology

## Status

Accepted

## Context

embyr-rs must simultaneously serve four distinct wire protocols from the Firestore surface area:

1. **Pure gRPC over HTTP/2** — used by Node.js SDK, server-side SDKs, and any gRPC-native client
2. **gRPC-Web** — an HTTP/1.1 framing of gRPC that browsers can use (browsers cannot speak raw HTTP/2 streaming)
3. **BrowserChannel / WebChannel** — a long-poll HTTP/1.1 protocol the Firebase JS SDK uses in browsers for `Listen` and `Write` streams
4. **REST/JSON** — the grpc-gateway HTTP/JSON transcoded surface

All four must be served on two externally-visible ports: gRPC port (default 8080, pure gRPC only) and REST port (default 8081, all other transports). A third port (default 9090) serves the admin API and must be unreachable from the public network.

Additionally, embyr-rs must maintain per-project in-process state: active BrowserChannel sessions (keyed by a 24-hex `SID`) and the Listen stream registry (in-process fan-out hub that delivers `DocChange` events from Postgres NOTIFY to active gRPC Listen streams). This state cannot be transparently migrated between processes without externalizing it to a shared store.

The customer story for browser transport (US-13, AC-13d) explicitly states: "No separate server process required for browser transports."

A separately-deployed customer binary (`embyr-agent`) is required for `backend_mode=agent` to satisfy Riley's credential isolation requirement (US-12): DB credentials must never cross the VPC boundary to embyr SaaS. The agent must be structurally separate from embyr SaaS — it runs in the customer's own infrastructure.

## Decision

**embyr-rs** is deployed as a single OS process that opens three TCP listeners:

| Listener | Port | Protocol | Purpose |
|----------|------|----------|---------|
| gRPC | 8080 (configurable) | gRPC / HTTP/2 | Pure gRPC data plane |
| REST | 8081 (configurable) | HTTP/1.1 + HTTP/2 | gRPC-Web, BrowserChannel, REST/JSON, health endpoints |
| Admin | 9090 (configurable) | HTTP/1.1 | Project management API (internal only) |

The REST port uses a single combined handler with a routing decision tree (inspects `Content-Type` and path):
1. `Content-Type: grpc-web*` → gRPC-Web handler (wraps the gRPC server; traverses all gRPC interceptors)
2. Path ends with `/channel` → BrowserChannel handler (no write deadline; long-lived)
3. Health paths (`/healthz`, `/readyz`) → health handlers (bypass auth, bypass CORS)
4. Streaming JSON routes (`:runQuery`, `:batchGet`, `:runAggregationQuery`) → custom JSON-array streamers
5. All other paths → grpc-gateway REST mux (30 s timeout)

**embyr-agent** is a separately compiled, statically-linked Rust binary. It opens one TCP listener on `:9191` (configurable via `EMBYR_AGENT_LISTEN_ADDR`) and serves the `embyr.agent.v1.StorageAgent` gRPC service over mandatory mTLS.

The two binaries share no process boundary. Their only communication is the mTLS gRPC channel initiated by embyr SaaS toward the agent endpoint stored in the project record.

No additional processes are required for any combination of backend modes or transport types.

**Background goroutines within embyr-rs** (not separate processes):
- Transaction expiry sweeper (interval: `transactions.sweep_interval`, default 30 s)
- Tombstone sweeper (sweeps tombstones older than 24 h)
- Deleted project sweeper (sweeps projects with `deleted_at < now - admin.deletion_retention`)
- NOTIFY listener goroutines (one long-lived Postgres connection per active customer DB)
- Agent subscription goroutines (one long-lived gRPC Subscribe stream per `backend_mode=agent` project)

## Consequences

**Benefits:**
- No IPC required for session state: BrowserChannel `SID`-keyed session map and Listen registry live in the same process address space as the handlers that read/write them. No serialization, no network round-trip, no distributed locking.
- AC-13d satisfied structurally: browser transports physically cannot require a separate server process because they are handlers inside the single process.
- Operational simplicity: operators deploy one binary (plus optionally the agent binary if using `backend_mode=agent`). No sidecar required for protocol translation.
- Health check surface is minimal: `/healthz` (process alive) and `/readyz` (system DB reachable) cover the full process.
- Admin port isolation is structural: a separate TCP listener at the OS level. Contrast with path-based routing where a routing bug could expose admin endpoints on the data port.

**Trade-offs and costs:**
- **Sticky routing is required at the LB layer for BrowserChannel clients.** The load balancer must route requests with the same `SID` to the same embyr instance. If an instance dies, all BrowserChannel sessions on that instance die — clients reconnect and create new sessions (SDK handles this transparently). No embyr-level mitigation is possible without externalizing session state.
- **Listen streams are instance-affine by TCP connection.** An instance failure drops all active Listen streams. SDK reconnects; resume tokens allow delta delivery from the surviving instance (the Postgres data is shared). Full re-snapshot is triggered only when the resume token is older than 24 h.
- **In-process state limits horizontal scaling granularity.** Adding instances does not redistribute existing BrowserChannel sessions or Listen streams — it only handles new connections. Draining an instance requires the LB to stop routing new connections, then wait for existing connections to close (or forcibly close them).
- **The agent binary must be separately distributed and version-matched.** Operators using `backend_mode=agent` must deploy the agent binary to their VPC and keep it version-aligned with the embyr SaaS major version. Version mismatch is detected at the gRPC protocol level (proto versioning).
- **Port numbering must be globally unique per host.** All three port values are validated at startup; any collision causes `health.startup.refused: port_conflict`. This prevents silent misconfiguration where the admin port accidentally overlaps the data port.

## Alternatives Considered

### Alternative A: Separate processes per transport

Deploy a gRPC process, a gRPC-Web/REST proxy process, and an admin process separately. Each process is independently scalable and independently restartable.

**Rejected because:**
- BrowserChannel session state and Listen registry would need to be externalized to a shared store (Redis) to allow any process handling a BrowserChannel back-channel GET to find the session created by the forward-channel POST. This contradicts D09 (locked decision: LISTEN/NOTIFY, no distributed pub/sub).
- Violates AC-13d: "No separate server process required for browser transports." The user story explicitly rules this out.
- Adds operational complexity (three process lifecycle policies instead of one) and a new failure mode (inter-process connectivity).

### Alternative B: Single process with externalized session state (Redis)

Single binary, but BrowserChannel sessions and Listen registry stored in Redis. This would enable stateless horizontal scaling with no sticky routing requirement.

**Rejected because:**
- D09 and D10 (locked decisions) explicitly exclude distributed coordination infrastructure. The feature scope is a protocol translation binary, not a distributed session management system.
- Redis becomes a SPOF (or requires its own HA setup), increasing operator burden.
- Per-instance token-bucket rate limiting (D10) cannot be distributed anyway, so the sticky routing requirement for BrowserChannel does not compound a pre-existing sticky routing requirement — the LB already needs to handle connection affinity for gRPC streaming.
- The scale at which Redis would become necessary (tens of millions of concurrent BrowserChannel sessions) is outside the stated scope.

### Alternative C: Merge embyr-agent into embyr-rs with a "local mode"

Run the agent functionality as a code path inside embyr-rs when `backend_mode=local_agent`, eliminating the separate binary.

**Rejected because:**
- Riley's requirement (US-12, JOB-04) is that DB credentials never cross the network to embyr SaaS. If the agent code runs inside embyr SaaS, credentials necessarily exist in the SaaS process. This violates the credential isolation invariant (SPEC invariant 13).
- The agent binary's security model depends on physical process separation: the agent holds credentials in its environment variables, in its own process space, in the customer's VPC. Merging the binaries collapses this boundary.
- Static linking of the agent binary is only meaningful if it is a separate, minimal-surface binary. A merged binary would carry the full embyr SaaS attack surface into the customer VPC.
