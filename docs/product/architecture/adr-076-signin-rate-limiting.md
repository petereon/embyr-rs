# ADR-076: Admin Signin Rate Limiting — Source-IP Token Bucket, Postgres-Backed

## Status

Accepted

## Context

`production-readiness-audit-2026-09-08.md` findings #12 and #13 (both High) confirm
`POST /admin/v1/auth/signin` (`crates/embyr-server/src/admin/handlers/auth.rs`) has zero
rate-limit protection: an unthrottled flood of wrong-password requests can (a) consume ~6GB of
memory and starve the async reactor (finding #12 — compounded by Argon2id running inline instead
of in `spawn_blocking`, see the companion mechanical fix in this same feature), and (b) cheaply
enumerate every admin email address on an account via a timing side-channel (finding #13),
because nothing bounds request volume from a single source.

The obvious reuse candidate — `RateLimiter`/`rate_buckets` (ADR-015, closes JOB-11's
fair-multitenancy finding for the Firestore data plane) — is schema-bound: `rate_buckets.project_id`
carries `REFERENCES projects(id) ON DELETE CASCADE` (`migrations/0018_rate_buckets.sql`). A source
IP address is not a project and has no corresponding `projects` row; inserting one would violate
the FK. `RateLimiter`'s own public surface (`check(project_id: &str)`, the `"project_id"`
Prometheus label) is likewise project-shaped, not a generic keyed limiter. This was investigated
and confirmed during DISCUSS (feature-delta.md § Investigation 2) — reuse-verbatim is not possible.

`embyr-server` is deployed as N horizontally-scaled instances behind a load balancer — this is not
a hypothetical: it is the explicit, already-solved problem ADR-015 itself exists to fix ("with N
embyr instances behind a load balancer, a project configured for 1000 RPS can actually fire N ×
1000 RPS cluster-wide before any single node rejects a request"). An in-process-only limiter on the
admin signin route would reopen the identical N× loophole for the admin plane that ADR-015 already
closed for the data plane — for the same underlying promise this feature's own user story names
explicitly: "the admin plane now has the same 'one bad actor cannot starve everyone else' guarantee
JOB-11 already gives the customer data plane."

## Decision

**A new, small, FK-free Postgres table (`signin_rate_limits`) backing a new `SigninRateLimiter`
struct, reusing the `TokenBucket` continuous-refill ALGORITHM from `middleware/rate_limit.rs`
(bumped to `pub(crate)` for reuse) but not its table, its struct, or its `project_id`-shaped API.**
Postgres-backed with a 20ms-timeout in-process fallback, mirroring ADR-015's own fail-open shape.

### Throttling key: source IP only

`peer.ip()` from the real TCP connection — not the attempted email, and not a compound
`(ip, email)` key. Rationale:

- **Per-email-only** fails AC-ASH-06 outright: an attacker probing 500 distinct candidate emails
  from one source would get a full, fresh bucket for *each* email (500 × free burst), completely
  defeating the enumeration throttle the story requires.
- **Compound `(ip, email)`** has the same failure mode for the identical reason — a fresh key per
  candidate email — while adding no benefit over IP-only for the single-source guessing scenario
  (US-01 Scenario 3) IP-only already covers.
- **IP-only** correctly throttles both named threat shapes (sustained guessing against one email,
  and enumeration sweeps across many emails) because both are, by the story's own UAT wording,
  "from one source."
- **Accepted residual gap** (not solved here, and not claimed to be): a *distributed* attacker
  rotating source IPs defeats per-IP keying. This is a volumetric/distributed-attack class
  ordinarily handled at a WAF/CDN layer, not a single-route application limiter, and DISCUSS's own
  scope excludes it (see feature-delta.md § Out of Scope).

### Capacity and refill rate

`capacity = 150.0` tokens, `refill_rate = 10.0 / 60.0` tokens/sec (**10 attempts/minute sustained**,
per source IP).

The capacity number is not arbitrary: it is directly forced by AC-ASH-02's own already-locked
wording — "~100 concurrent wrong-password requests" plus a concurrent legitimate signin from a
*different* client that must also succeed within the same test run. In a real test harness, both
the flood and the legitimate request typically originate from the same loopback source IP (the
test machine) — an IP-keyed bucket sized below ~100 would make AC-ASH-02's own walking-skeleton
scenario internally impossible to satisfy without also breaking AC-ASH-03. Capacity 150 absorbs the
named 100-concurrent burst with headroom for the legitimate request, while a 10/min sustained refill
means a *genuinely sustained* attack (the actual threat named in finding #12 — an ongoing flood, not
one 100-request burst) throttles hard immediately after the burst is spent, and a 500-candidate-email
enumeration sweep (AC-ASH-06) is throttled after its first 150 probes and reduced to 10/min
thereafter (~50 minutes to complete a 500-email sweep from one source, versus unbounded today).

This is a two-tier shape (generous one-time burst, tight sustained rate), not a single flat rate —
deliberately, so the mechanism that fixes *resource exhaustion* (spawn_blocking, tested under
concurrent burst) and the mechanism that fixes *sustained abuse* (this rate limiter, tested under
sustained volume) can be exercised by two distinct tests without one mechanism's test load
accidentally tripping the other's threshold.

### Position in the `signin` handler: first statement, before any DB call

The check runs before Step 1's `SELECT ... FROM users WHERE email = $1` — before *any* handler
logic, matching this session's established "reject before any handler logic runs" pattern. This
guarantees: (a) a throttled request never reaches Argon2id (AC-ASH-03/06's explicit "no additional
Argon2id verification is performed for throttled attempts"), (b) a throttled request never reaches
the DB at all, and (c) the 429 response time is now *identical and minimal* regardless of whether
the submitted email exists — the throttle path itself introduces no new timing side-channel between
known/unknown emails, because it never gets far enough to know.

### No new middleware layer — gated in-handler

Implemented as the first statements inside `signin()`, not an `axum::middleware::from_fn` layer on
`public_router`. This mirrors the established convention already used throughout
`crates/embyr-server/src/admin/router.rs`'s own comments for route-specific gates ("Owner/Admin gate
... enforced inside the handlers", repeated for `sdk_keys`, `access_rules`, `hosted_identity`,
`oauth_providers`, `anonymous_identity` — none of these use a per-route middleware layer either). A
middleware layer is the right shape for a check applied identically across *many* routes (as
`rest_rate_limit_middleware` is, across every `:project_id` REST route); this is a single route with
a single, route-specific throttle. `router.rs`'s `.route_layer(...)` calls are unmodified by this
feature — confirmed zero diff in that file's middleware stack.

### Peer IP plumbing: `spawn_admin_server`'s accept loop (lib.rs)

The admin listener does not use `axum::serve`/`IntoMakeServiceWithConnectInfo` — it is a hand-rolled
hyper accept loop (`crates/embyr-server/src/lib.rs::spawn_admin_server`) that today discards the
peer address (`let (stream, _peer) = listener.accept().await`). This is the one plumbing change
outside `auth.rs`: capture `peer` and insert `req.extensions_mut().insert(ConnectInfo(peer))` before
dispatching to the axum `Router`, so `signin`'s own `ConnectInfo<SocketAddr>` extractor parameter
resolves the genuine per-connection peer address exactly as it would under
`IntoMakeServiceWithConnectInfo`. Additive only — every other route is unaffected; this is the
single shared accept-loop code path used by both `main.rs` (production) and every
`build_with_*` test-server constructor in `lib.rs`, so acceptance tests hitting a real bound
`AdminTestContext` server via `reqwest` get a real, correct peer address with no test-only branching.

**Known limitation, accepted for V1**: if the admin port (`:9090`) is ever placed behind a reverse
proxy/L7 LB that terminates the client's own TCP connection, `peer` becomes the proxy's IP, not the
true client's. ADR-001 states the admin port "must be unreachable from the public network" —
implying direct/VPN/internal access is the intended deployment shape, not a public-facing shared
LB. If a future deployment introduces a proxy in front of `:9090`, `X-Forwarded-For` parsing would
need to be added at that time — out of scope here.

### Reused as-is from `embyr_core::auth::argon2`

`signin`'s own inline `Argon2::new(...)` + `PasswordHash::new(...)` + `.verify_password(...)` is
replaced with a call to `embyr_core::auth::argon2::verify_password(password: &[u8], phc_hash: &str)
-> Result<bool, CoreError>` — an existing, already-tested, IO-free function
(`crates/embyr-core/src/auth/argon2.rs`, added for client-auth-hosted-identity/ADR-036) that
`signin` was, until now, the one caller NOT using, duplicating its own copy of the same Argon2id
parameter set instead. This closes a second, smaller pre-existing gap (parameter-set duplication)
as a side effect of the primary `spawn_blocking` fix, at zero extra cost — the function is exactly
what needs to move into the blocking closure.

### `signin_rate_limits` table (`migrations/0037_signin_rate_limits.sql`)

```sql
CREATE TABLE signin_rate_limits (
    source_key  VARCHAR(45)      NOT NULL PRIMARY KEY, -- max textual IPv6 length
    tokens      DOUBLE PRECISION NOT NULL,
    last_refill TIMESTAMPTZ      NOT NULL
);
```

No FK, no backfill (unlike `rate_buckets`, whose rows are pre-provisioned per known project; this
table's key space is unbounded and lazily populated — one row per newly-seen source IP, created by
the atomic UPSERT in `SigninRateLimiter::check_pg`).

### `SigninRateLimiter::check_pg` — single-statement atomic UPSERT

Unlike `RateLimiter::check_pg` (which relies on `rate_buckets` rows already existing per project and
falls back to a 2-step "check existence, insert default" dance for pre-migration projects), this key
space has no pre-existing rows to assume — every row is created on first sight. A single UPSERT
handles both "new source" and "existing source" in one round trip:

```sql
INSERT INTO signin_rate_limits (source_key, tokens, last_refill)
VALUES ($3, $1::float8 - 1.0, now())
ON CONFLICT (source_key) DO UPDATE
SET tokens = LEAST($1::float8,
                    signin_rate_limits.tokens
                      + EXTRACT(EPOCH FROM (now() - signin_rate_limits.last_refill)) * $2::float8
                   ) - 1.0,
    last_refill = now()
WHERE signin_rate_limits.tokens
        + EXTRACT(EPOCH FROM (now() - signin_rate_limits.last_refill)) * $2::float8
      >= 1.0
RETURNING tokens
```

`$1` = capacity, `$2` = refill_rate, `$3` = source_key. 1 row returned → allowed (covers both a
brand-new source and an existing source with tokens available). 0 rows returned → the `ON CONFLICT
DO UPDATE ... WHERE` guard suppressed the update — genuinely rate-limited; a follow-up `SELECT`
computes the current token level for the `retry-after-ms` header, mirroring `RateLimiter::check_pg`'s
own identical "genuinely rate-limited — query current token level for headers" step.

20ms `tokio::time::timeout` wraps `check_pg`, falling through to the in-process `TokenBucket` map on
timeout or hard Postgres error — same fail-open shape, same hard-coded bound, as ADR-015 D3.

### `SigninRateLimiter` is a concrete struct, not a port/trait

Re-affirms ADR-015's own already-decided reasoning (Alternative 4) rather than re-litigating it:
rate limiting has exactly one implementation shape here too. Tests use `SigninRateLimiter::new(cap,
refill)` (in-process-only); production uses `SigninRateLimiter::with_pg(cap, refill, pool)`.

### Composition root wiring

- `UserAdminState` (`admin/state.rs`) gains `pub signin_rate_limiter: Arc<SigninRateLimiter>`.
- `build_admin_router` (`admin/router.rs`) gains one new parameter, threaded into `UserAdminState`.
- `build_with_secret_fetchers` (the single internal call site all `build`/`build_with_aws`/
  `build_with_gcp` test wrappers funnel through) constructs `SigninRateLimiter::new(150.0, 10.0/60.0)`
  (in-process-only) inline — mirrors `RateLimiter::new(_, _, None)`'s existing test-safety precedent,
  so no acceptance test run under a shared testcontainers Postgres instance can pollute another
  test's throttle counters via a shared table row.
- `main.rs` constructs `SigninRateLimiter::with_pg(150.0, 10.0/60.0, system_db.pool().clone())` at
  the real composition root and spawns `signin_rate_limit_sweeper::spawn(system_db, Duration::from_secs(3600))`
  alongside the other three existing sweepers (transaction, tombstone/soft-delete via
  `soft_delete_purge_sweeper`, cap-usage-refresher) already listed in ADR-001.

### `signin_rate_limit_sweeper` — new 4th background sweeper

Unlike `rate_buckets` (row count bounded by provisioned-project count), `signin_rate_limits` rows
are created for *any* source IP that ever calls the signin route — an attacker rotating source IPs
would otherwise grow this table without bound, becoming a storage-exhaustion vector in its own
right. Mirrors `SoftDeletePurgeSweeper`'s exact spawn/run_cycle/advisory-lock shape: `DELETE FROM
signin_rate_limits WHERE last_refill < now() - interval '24 hours'`, hourly, gated by
`pg_try_advisory_lock` (new lock-key namespace `"embyr_signin_rate_limit_sweep"`, distinct from the
three existing lock keys). 24h retention is comfortably beyond the bucket's own refill-to-full time
(well under 15 minutes at this capacity/refill), so a swept-then-reinserted row never hands a
returning attacker more tokens than a continuously-tracked row would have.

### Metric

`embyr_signin_rate_limit_requests_total{outcome}` (`outcome` = `allowed`|`rejected`) and
`embyr_signin_rate_limit_pg_timeout_total` — **deliberately no source-key label.** Labeling by raw
attacker-controlled IP would replay the exact unbounded-cardinality problem ADR-069 already fixed
for `project_id`; this metric carries no per-key dimension at all.

## Alternatives Considered

### Alternative 1: Extend `RateLimiter`/`rate_buckets` to accept a non-project key (rejected)

Would require dropping or making nullable the `project_id` FK on `rate_buckets`, weakening the
invariant every existing data-plane caller relies on, and would conflate two different key spaces
(bounded, pre-provisioned project IDs vs. unbounded, lazily-seen source IPs) in one table with two
different lifecycle/retention needs. Confirmed structurally incompatible in DISCUSS (Investigation 2).

### Alternative 2: In-process-only bucket, no Postgres backing (rejected)

Simpler (no migration, no sweeper), but reopens the exact N-instance loophole ADR-015 exists to
close, for the same job (JOB-10) whose story explicitly asks for parity with JOB-11's cross-instance
guarantee. Rejected because `embyr-server`'s own deployment model (N instances behind an LB, per
ADR-015's own stated context) makes this a real gap, not a hypothetical one, and admin signin volume
is low enough that the Postgres-backed cost (one small UPSERT per attempt, 20ms-bounded) is
negligible.

### Alternative 3: Redis (rejected)

Same rejection as ADR-015 Alternative 1: introduces a new coordination-plane dependency this
system's own Operational Simplicity quality attribute explicitly excludes, for no benefit over the
already-accepted Postgres-timeout-with-fallback pattern this system already runs in production for
the data plane.

### Alternative 4: Extend `failed_totp_attempts`/`locked_until` to cover password failures instead of adding a rate limiter (rejected — OQ-ASH-02)

Investigated in DISCUSS (Investigation 3) and confirmed here: this would let a third party who
merely *knows* a legitimate admin's email lock that admin out of their own account with zero need to
guess correctly, a denial-of-service the audit itself does not name and that this feature must not
introduce. The existing TOTP-only lockout is left completely untouched; per-source rate limiting is
the primary defense against sustained password guessing instead. The TOTP-only lockout's own
identical-shape weaponization risk is accepted (already shipped) specifically because it gates a
*second* factor reachable only after a correct password — a materially higher bar than the
zero-knowledge password-guessing case this feature addresses.

### Alternative 5: Dummy Argon2id verify on the unknown-email path for full constant-time parity (rejected — OQ-ASH-03)

Investigated in DISCUSS (Investigation 4) and confirmed here: the unknown-email path is the one path
an attacker controls the volume of at zero cost today (any string is a candidate email). Adding a
full ~50-100ms Argon2id verify to it converts a cheap-to-reject path into an equally expensive one
for the attacker's own most abundant input — worsening finding #12's resource-exhaustion risk to
close finding #13's confidentiality leak. With this ADR's rate limiter in place, the timing oracle's
*practical* exploitability is bounded to 10 timed probes/minute per source after the initial burst —
a 500-email sweep takes on the order of 50 minutes from one source, and real-world network/OS
scheduling jitter further degrades the signal at that trickle rate. **Decision: accept the residual
timing difference; do not add a dummy verify.** AC-ASH-08 codifies this choice — no dummy verify is
added to the unknown-email path.

## Consequences

### Positive

- Closes both #12 (resource exhaustion — combined with the companion `spawn_blocking` fix) and #13
  (enumeration — bounded to a slow trickle, not eliminated) with one mechanism, keyed once.
- Cluster-wide enforcement: consistent with the guarantee ADR-015 already gives the data plane —
  a horizontally-scaled `embyr-server` deployment cannot be defeated by simply spreading load across
  instances.
- Zero new runtime dependencies (reuses `sqlx`, `tokio`, `axum`, `metrics` — all already present).
- No new middleware layer; `router.rs`'s existing `.route_layer(...)` stack is untouched.
- `embyr_core::auth::argon2::verify_password` reuse removes a second, smaller pre-existing
  parameter-duplication gap as a side effect.

### Negative / Trade-offs

- A 150-token burst is generous — a determined attacker gets up to 150 free attempts per source IP
  before throttling engages. This ceiling is a direct, documented consequence of AC-ASH-02's own
  already-locked "~100 concurrent" wording combined with IP-based keying; shrinking it would require
  either re-litigating that AC or moving the walking-skeleton's concurrent-load proof to a
  differently-sourced client (out of scope for this ADR to decide unilaterally).
- Per-IP keying means a shared IP (e.g. an office NAT) pools its rate-limit budget across everyone
  behind it, including any legitimate admin sharing that IP with an attacker. This is inherent to
  IP-based limiting generally, not specific to this design, and is not worsened relative to the
  status quo (today, that shared IP has *no* limit at all).
- A reverse proxy in front of `:9090` would break the peer-IP assumption (see plumbing section
  above) — accepted as out of scope given ADR-001's own "admin port not public-facing" constraint.
- One new background sweeper (4th, after transaction/soft-delete/cap-usage-refresher) — small,
  mirrors an existing shape exactly, hourly cadence, negligible overhead.

## Enforcement

- `signin_rate_limits` migration runs at startup via `sqlx-migrate` before any listener opens —
  structural proof the table exists before `check_pg` is ever called (mirrors ADR-015's own
  enforcement note).
- **Behavioral (Earned Trust)**: a fault-injection integration test analogous to
  `tests/acceptance/us_drl_03_graceful_fallback.rs` (ADR-015) should be added in DELIVER — inject a
  simulated Postgres delay and assert the 20ms timeout fires with in-process fallback activation and
  `embyr_signin_rate_limit_pg_timeout_total` incrementing.
- **CI grep gate (new, recommended)**: ADR-003's `spawn_blocking` mandate for Argon2id had zero
  automated enforcement — this is precisely why `signin`'s own violation went undetected until this
  audit. Recommend a CI check: no direct `argon2::Argon2::new(` or `PasswordVerifier::verify_password(`
  call site may exist anywhere under `crates/embyr-server/src/` (excluding test modules) — every
  Argon2id call must route through `embyr_core::auth::argon2::{hash_api_key,verify_api_key,
  hash_password,verify_password}`, whose own 3 existing call sites (`provision.rs`, `sdk_keys.rs`,
  and now `auth.rs`) are all already `spawn_blocking`-wrapped. A single `rg` invocation in CI is
  sufficient — no new tooling dependency.

## References

- `docs/product/architecture/adr-015-distributed-rate-limiter-postgres.md` — algorithm/shape mirrored
- `docs/product/architecture/adr-003-async-runtime.md` — spawn_blocking mandate this feature applies
- `docs/product/architecture/adr-001-process-topology.md` — admin port not public-facing; N-instance deployment model
- `docs/product/architecture/adr-069-rate-limit-metric-label-cardinality-bounding.md` — precedent this ADR's metric design avoids re-violating
- `docs/feature/admin-signin-hardening/feature-delta.md` — DISCUSS Investigations 1-4, OQ-ASH-01/02/03
