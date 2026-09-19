# Resource Sizing (CPU / Memory) — embyr-server and embyr-agent

Closes Medium finding #52 from
[`docs/product/production-readiness-audit-2026-09-08.md`](../product/production-readiness-audit-2026-09-08.md).

## Honesty disclaimer

**These are reasonable starting estimates derived from the architecture, not empirically
load-tested numbers.** No load test has been run against either binary as of this writing.
Treat every figure below as a starting point for your own staging-environment validation, not
a guarantee. If you have real production telemetry (CPU/memory graphs under known request
rates), replace these numbers with yours and consider contributing them back here.

This gap exists alongside the observability gaps noted in findings #47/#56 (sweeper failure
counters) — until dashboards exist for actual memory/CPU under load, sizing stays estimate-only.

## Who this is for

An operator deciding container resource `requests`/`limits` (Kubernetes) or a VM/container size
(single-host, see [`single-host-deployment.md`](./single-host-deployment.md)) for `embyr-server`
and, optionally, `embyr-agent`.

## embyr-server

### What consumes memory

1. **Tokio runtime + binary baseline.** A multi-threaded Tokio runtime plus the compiled Rust
   binary's static overhead — roughly 50-100 MiB idle, before any request traffic or connection
   pools are counted.
2. **Argon2id hashing (dominant transient cost under auth load).** `argon2_instance()`
   (`crates/embyr-core/src/auth/argon2.rs`) is configured `Params::new(65536, 3, 4, Some(32))` —
   **65536 KiB (64 MiB) of memory per concurrent hash operation**, used for every API-key
   verification (`verify_api_key`) and every hosted-identity password hash/verify
   (`hash_password`/`verify_password`, ADR-036). This memory is claimed only for the duration of
   the hash (tens of milliseconds of CPU time) and released after, but **N concurrent
   sign-in/API-key requests cost roughly N × 64 MiB at that instant.** A burst of 20 concurrent
   auth requests is ~1.3 GiB of transient memory — this is the single largest source of memory
   *spikes* on this binary and the main reason the moderate-production tier below carries more
   headroom than static baseline usage would suggest.
3. **Postgres connection pools.** ADR-079 (see
   [`adr-079-pool-sizing-and-limits.md`](../product/architecture/adr-079-pool-sizing-and-limits.md))
   defines 5 env vars controlling pool sizes:

   | Env var | Default |
   |---|---|
   | `EMBYR_SYSTEM_DB_MAX_CONNECTIONS` | 5 |
   | `EMBYR_TENANT_DB_MAX_CONNECTIONS` | 5 |
   | `EMBYR_LISTENER_DB_MAX_CONNECTIONS` | 2 |

   (plus the two `*_ACQUIRE_TIMEOUT_SECS` vars, which affect latency, not memory). Each open
   `sqlx` connection's client-side memory footprint (socket buffers, prepared-statement cache) is
   small — well under 1 MiB per connection in the common case — so the pool vars mostly matter for
   Postgres server-side `max_connections` capacity planning (see that document), not for
   `embyr-server`'s own memory budget. They matter for *this* document only in one direction:
   **raising the pool-size env vars raises how many concurrent tenant queries can be in flight at
   once, which indirectly raises how much other per-request memory (buffers, in-flight response
   payloads) can be live simultaneously.** Size pools to what the tier's CPU/memory headroom can
   actually sustain, not just to what Postgres's own `max_connections` allows.
4. **In-process session/stream state (ADR-001).** BrowserChannel's `SID`-keyed session map and
   the Listen-stream fan-out registry live in process memory, sized per active client, not
   estimated here — this document covers baseline/idle-to-moderate sizing, not a formula for
   "N active BrowserChannel sessions costs M MiB." If you run at BrowserChannel/Listen scale
   large enough for this to matter, load-test it directly.

### CPU

Tokio's default worker-thread count equals the host's visible CPU count. Argon2id hashing is
CPU-bound (that's the point of the algorithm) and briefly saturates a core per concurrent hash;
under an authentication burst, CPU contention — not memory — is often the first bottleneck.
Give the container at least 1 full vCPU so a hash operation is never fighting for a fractional
CPU share alongside request-routing work.

### Minimum viable tier (evaluation / low traffic)

For a single evaluator, a demo, or genuinely low request volume (occasional requests, no
concurrent sign-in storms):

- **CPU:** request `500m`, limit `1` vCPU
- **Memory:** request `512Mi`, limit `1Gi`
- Pool env vars: leave at ADR-079 defaults (5 / 5 / 2) — this tier does not need more concurrency
  than the defaults already provide.

### Moderate production tier

For a small production workload with occasional concurrent sign-ins/API-key checks and steady
(not extreme) query traffic:

- **CPU:** request `1` vCPU, limit `2` vCPU
- **Memory:** request `1Gi`, limit `2Gi` — the limit headroom above the request is mostly to
  absorb Argon2id concurrency spikes (item 2 above), not steady-state usage.
- Pool env vars: if you raise `EMBYR_TENANT_DB_MAX_CONNECTIONS`/`EMBYR_LISTENER_DB_MAX_CONNECTIONS`
  above the ADR-079 defaults to serve more concurrent tenants, raise the memory limit roughly in
  step — there is no fixed per-connection MiB figure to multiply by (see item 3), but more
  concurrent in-flight queries means more concurrent in-flight response buffers.

If you expect sign-in/API-key verification bursts significantly above ~20 concurrent, budget
extra memory explicitly: `(expected peak concurrent Argon2id ops) × 64 MiB`, added on top of the
tier's baseline.

## embyr-agent

`embyr-agent` (see [ADR-001](../product/architecture/adr-001-process-topology.md)) is a thin,
statically-linked mTLS gRPC proxy that forwards Firestore backend calls into a customer's own
Postgres — it does not run Argon2id, does not hold BrowserChannel/Listen state, and does not run
the REST/admin surface. Its resource profile is much lighter:

- **CPU:** request `250m`, limit `500m`
- **Memory:** request `128Mi`, limit `256Mi`

Raise this only if a single agent instance is expected to proxy unusually high query volume for
its one tenant — the defaults above assume the common case (light forwarding load for one
customer VPC).

## Cross-references

- [ADR-079 (Pool Sizing and Limits)](../product/architecture/adr-079-pool-sizing-and-limits.md) —
  the connection-pool env vars this document's memory guidance builds on.
- [`single-host-deployment.md`](./single-host-deployment.md) — where these numbers would apply
  as `docker run --memory`/`--cpus` flags or a systemd `MemoryMax=`/`CPUQuota=` unit setting on a
  non-Kubernetes host.
- [`network-topology.md`](./network-topology.md) — DNS/load-balancer/firewall guidance for the
  same deployment (finding #53).
- ADR-036 (hosted-identity) — the password-hashing call sites that share Argon2id parameters
  with API-key verification.
