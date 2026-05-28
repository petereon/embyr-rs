# ADR-003: Async Runtime Selection

## Status

Accepted

## Context

embyr-rs is a network server that must simultaneously manage:

- Thousands of concurrent gRPC streams (Listen RPC is a long-lived bidirectional stream)
- One dedicated Postgres connection per active customer DB for `LISTEN/NOTIFY`
- Multiple background sweeper tasks (transaction expiry, tombstone purge, deleted project purge)
- BrowserChannel long-poll sessions (each held open for seconds to minutes)
- Per-project rate limiter state shared across concurrent requests

These workloads are IO-bound with a mix of:
- High-concurrency / low-compute (Listen streams, BrowserChannel sessions — mostly idle, unblocked by Postgres NOTIFY)
- Low-concurrency / medium-compute (Argon2id verification on credential cache miss — CPU-intensive, ~200–500 ms, but infrequent)
- Background low-priority tasks (sweepers — periodic, non-latency-sensitive)

Rust's standard library provides no built-in async executor. The language provides `async/await` syntax and the `Future` trait; an external executor is required to poll futures to completion. The choice of executor is foundational — it determines the threading model, task scheduling behavior, and compatibility with all downstream crates that require an async context (gRPC framework, database driver, HTTP framework, cloud SDKs).

The executor decision is effectively irreversible at the crate level: `tonic` (the de facto Rust gRPC library), `sqlx` (the selected Postgres driver), `axum` (the selected HTTP framework), and `aws-sdk-secretsmanager` all declare `tokio` as their async runtime dependency. Selecting a different executor would require replacing all four libraries.

### Quality Attribute Priorities

From the System Architecture section:
1. Real-time latency: p99 write → `onSnapshot` ≤ 2 s — demands a scheduler that does not block IO tasks behind CPU-intensive work.
2. Horizontal scalability: stateless for writes, instance-local state for streams — demands a multi-threaded scheduler to utilize all CPU cores.
3. Protocol fidelity: bidirectional streaming gRPC (Listen RPC) — demands stable support for long-lived async tasks.
4. Operational simplicity: single binary — demands all async machinery in one process.

## Decision

**Use `tokio` 1.x with the multi-thread scheduler (`#[tokio::main]` with `worker_threads` defaulting to number of logical CPU cores).**

Configuration:
- `worker_threads`: default (= logical CPUs). Configurable via `TOKIO_WORKER_THREADS` env var.
- `max_blocking_threads`: 512 (default). Argon2id verification runs in `tokio::task::spawn_blocking` to avoid blocking async worker threads during the ~200–500 ms hash computation.
- Task priority: Tokio does not provide priority scheduling. Sweeper tasks use `tokio::time::interval` with `MissedTickBehavior::Delay` — they yield to the scheduler on each interval rather than catching up on missed ticks.

**Argon2id isolation:** Every Argon2id verification call is wrapped in `tokio::task::spawn_blocking(|| argon2::verify(...))`. This is mandatory. An Argon2id call on a worker thread blocks that thread for 200–500 ms, starving all other tasks scheduled on that thread. `spawn_blocking` routes the call to a dedicated blocking thread pool, leaving the async worker threads free for IO dispatch.

**LISTEN dedicated connections:** Each `PostgresNotifyListener` task holds a single dedicated `sqlx::PgConnection` (not from `PgPool`). This connection is managed by a single Tokio task that loops on `PgListener::recv()`. The task is spawned at startup per active project and respawned on connection failure with exponential backoff.

## Alternatives Considered

### Alternative A: `async-std`

`async-std` is a Rust async executor that mirrors the standard library API with async equivalents. It uses a work-stealing multi-thread scheduler similar to Tokio.

**Rejected because:**
- `tonic`, `sqlx`, `axum`, and `aws-sdk-secretsmanager` all target Tokio specifically. `async-std` provides a Tokio compatibility layer (`async-std` + `async-compat`), but this introduces a runtime boundary inside library calls. Library code that internally spawns Tokio tasks (e.g., `sqlx`'s connection pool management) cannot be wrapped cleanly — the Tokio tasks spawned by `sqlx` will not be visible to the `async-std` executor and will silently fail to run.
- The `async-std` ecosystem is significantly smaller than Tokio's for server-side workloads. The last major Tokio-incompatible crate that was maintained primarily for `async-std` has since added Tokio support. There is no server-side library in this stack that is `async-std`-only.
- `tonic`'s `transport` module requires `tokio::net::TcpListener` and `tokio::io::AsyncRead/AsyncWrite` directly. Using `async-std` equivalents would require forking `tonic`.

### Alternative B: `smol`

`smol` is a minimal async executor (< 1000 lines of code). It provides a thread-pool executor and is compatible with `async-std` via `async-compat`.

**Rejected because:**
- Same Tokio library incompatibility as Alternative A — all four major dependency crates require Tokio.
- `smol` is designed for embedding in resource-constrained environments. embyr-rs targets server deployments with 4–8 GB RAM and 4–16 cores — `smol`'s minimalism provides no advantage over Tokio's richer feature set (timers, `select!`, `spawn_blocking`, task budget/cooperation, Tokio console).
- No evidence of `smol` being used in production for high-concurrency gRPC servers.

### Alternative C: Single-threaded runtime (`tokio::runtime::Builder::new_current_thread`)

A single-threaded Tokio runtime avoids the synchronization overhead of `Arc<Mutex<T>>` for in-process state (credential cache, session map, Listen registry). All state could be `Rc<RefCell<T>>` instead.

**Rejected because:**
- The Argon2id `spawn_blocking` calls would saturate the single thread during credential verification, stalling all IO dispatch for 200–500 ms per verification. Even with `spawn_blocking` routing to a thread pool, the single-threaded scheduler itself cannot run tasks in parallel — one task's `spawn_blocking` blocks the reactor loop from advancing other tasks.
- Rust's `tokio` `current_thread` runtime does not utilize multiple CPU cores. At 100 concurrent Listen streams and 500 writes/sec/project, the single thread becomes the bottleneck long before Postgres.
- `Rc<RefCell<T>>` is not `Send`. When the admin port and data port share state (e.g., credential cache), the data must cross thread boundaries even in a single-threaded model (e.g., when `spawn_blocking` returns to a thread pool thread). This forces `Arc<Mutex<T>>` anyway.

### Alternative D: Hybrid runtime (Rayon for CPU + Tokio for IO)

Use Rayon (a data-parallelism library) for CPU-intensive work (Argon2id, ECIES) and Tokio for IO.

**Rejected because:**
- `tokio::task::spawn_blocking` already provides the same isolation: CPU-intensive work runs on a dedicated blocking thread pool; IO tasks run on async worker threads. The two pools communicate via Tokio's channel primitives.
- Rayon's work-stealing is optimized for data-parallel iteration (map/reduce over collections). Argon2id is a single sequential computation — Rayon provides no speedup, only additional scheduler complexity.
- Adding Rayon as a runtime alongside Tokio creates two thread pools competing for CPU cores. On a 4-core instance, 4 Tokio workers + 4 Rayon workers = 8 threads competing for 4 cores, causing context-switch overhead with no throughput gain.

## Consequences

### Positive

- Full compatibility with `tonic`, `sqlx`, `axum`, and all AWS/GCP SDK crates — no runtime bridging required.
- Multi-thread scheduler utilizes all available CPU cores for concurrent Listen stream task scheduling.
- `spawn_blocking` isolation for Argon2id prevents CPU-intensive auth from blocking IO dispatch.
- Tokio console (`console-subscriber`) provides live inspection of async task state during development and debugging — valuable for diagnosing stuck Listen streams or slow sweeper tasks.
- Tokio's `select!` macro enables clean implementation of the Listen stream loop (select on: new DocChange from registry channel, stream close signal, keep-alive timer, RESET trigger).

### Trade-offs and Costs

- Tokio's multi-thread scheduler requires `Arc<Mutex<T>>` or `Arc<RwLock<T>>` for all shared mutable state. This is correct for embyr-rs (the three shared structures — credential cache, session map, Listen registry — are explicitly identified and wrapped). It is not a meaningful overhead given the access patterns (credential cache is read-heavy, suited to `RwLock`; registry is read-heavy with infrequent writes, also suited to `RwLock`; session map is write-heavy on session create/destroy, `Mutex`).
- `spawn_blocking` threads are OS threads with stack allocation (~8 MB per thread default). With `max_blocking_threads=512`, this is potentially 4 GB of reserved stack space in extreme pathologies. In practice, Argon2id verification is infrequent (at most once per 5 minutes per (project, api_key) pair at cache TTL=5 min). The default pool of 512 threads is never fully utilized; it can be lowered to 32 via `Builder::max_blocking_threads(32)` for memory-constrained deployments.
- Tokio's cooperative scheduling (task budget / `yield_now`) requires that long-running synchronous loops inside async code explicitly yield. The Listen stream handler, sweeper loops, and NOTIFY listener loops must not execute tight loops without `.await` points. This is a discipline requirement on implementers, enforced by code review.

## References

- `tonic` 0.12 docs: https://docs.rs/tonic/latest/tonic/ — confirms Tokio 1.x requirement
- `sqlx` 0.7 docs: https://docs.rs/sqlx/latest/sqlx/ — confirms Tokio 1.x feature flag
- Tokio 1.x docs: https://docs.rs/tokio/latest/tokio/
- `docs/product/architecture/brief.md` §§ System Architecture (Scalability Model, Data Flow)
- `docs/product/architecture/adr-004-grpc-framework.md` — Tonic selection (depends on ADR-003 Tokio decision)
