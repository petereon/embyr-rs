# ADR-057: AgentTransactionSweeper — Purge SQL, Status Vocabulary, and Retention Config

## Status

Accepted

## Context

`agent-mode-transaction-purge` (DISCUSS, feature-delta.md) closes the
agent-mode counterpart of the gap `customer-db-transaction-sweeper` US-02
closed for `direct_pg`/`aws_secret`/`gcp_secret` customer databases:
terminal `transactions` rows accumulate forever with no purge path.
`AgentTransactionSweeper` (`crates/embyr-agent/src/sweeper.rs`, shipped)
already reclaims (hard-deletes) abandoned `'active'` rows on a 60s TTL /
30s interval. Its own `sweep_once()` never touches any other status.

DISCUSS's own System Constraints state: "No new `transactions.status`
value. Purge targets the existing `'committed'` value only." This ADR
verifies that claim directly against the shared code
(`PostgresBackendAdapter`, `crates/embyr-pg-storage/src/backend_adapter.rs`,
used identically by `direct_pg` and `embyr-agent`) rather than accepting it,
per this session's Earned Trust discipline, and corrects it where the
verification disagrees.

### Verification: the agent-mode `transactions.status` vocabulary is four-valued, not two

Direct read of `backend_adapter.rs`:

- `begin_transaction` (line ~906): inserts `status` defaulted to `'active'`
  (migration default).
- `commit_transaction` (line 944): if the 60s window has already elapsed
  when a client calls `Commit`, sets `status = 'expired'` reactively —
  **before** the sweeper's own reclaim step necessarily runs (there is a
  race window between the row crossing 60s and the next 30s-interval
  sweep tick during which a client's own `Commit` call can still observe
  and act on the row first).
- `commit_transaction` (line 1230): on success, sets `status = 'committed'`.
- `rollback_transaction` (line 1253-1254): sets `status = 'rolled_back'`
  for a row still `'active'`.

All four call sites are in `PostgresBackendAdapter`, shared verbatim
between `direct_pg` and agent-mode — confirmed by direct reading, matching
the feature-delta's own Reading Confirmation claim that this code path is
identical across both. **`'expired'` and `'rolled_back'` are therefore
reachable, existing status values in agent-mode's own `transactions` table
today, exactly as they are in the non-agent tables — DISCUSS's own
"`'committed'` value only" framing under-scoped the purge target.** This
mirrors the non-agent sibling's own ADR-054 (D2), which independently
reached the same conclusion for `'expired'` ("the `'expired'` status value
... same semantic `commit_transaction`'s own reactive check already
writes") but did not extend its own purge query to `'rolled_back'` —
noted here as a finding for that already-shipped feature, not corrected by
this ADR (out of this feature's own scope).

Root-cause framing (not just the DISCUSS-named symptom): the actual defect
is "every terminal status value accumulates forever in agent-mode," of
which `'committed'` is the highest-volume but not the only case. Fixing
only `'committed'` would leave an identical, un-purged growth path open
for `'expired'`/`'rolled_back'` rows — the same class of gap this feature
exists to close.

## Decision

### D1 — Purge SQL: second `DELETE` statement inside `sweep_once`, all three terminal statuses

```sql
DELETE FROM transactions
WHERE status IN ('committed', 'expired', 'rolled_back')
  AND started_at < NOW() - $1 * INTERVAL '1 day'
```

`$1` binds `retention_days: i64`. Kept as a **second, separate** SQL
statement in the same `sweep_once` call (not merged into one combined
`WHERE ... OR ...` with the existing reclaim `DELETE`), mirroring ADR-054's
own D2 precedent of keeping reclaim and purge as two statements in one
cycle — the two queries have different predicates (`status = 'active'` +
TTL vs. terminal statuses + retention window) and merging them would
obscure the two independent policies being enforced.

**SQL-side interval arithmetic, not Rust-side `chrono` timestamp
computation** (diverges from ADR-054's own Rust-side
`Utc::now() - chrono::Duration::days(retention_days)` style). Rationale:
`sweep_once`'s existing reclaim query already computes its cutoff entirely
in SQL (`NOW() - $1 * INTERVAL '1 second'`, binding `ttl_secs: i64`); the
purge query reuses the identical idiom at day granularity
(`NOW() - $1 * INTERVAL '1 day'`, binding `retention_days: i64`). This is
the simpler, already-established pattern for this exact 39-line file —
introducing a second computation style (Rust-side `chrono`) for one query
in a file that otherwise does all date arithmetic in SQL is unwarranted
divergence for no behavioral difference. `chrono` is already a workspace
dependency of `embyr-agent` (transitively required elsewhere in the crate),
so this is not a dependency-avoidance argument either way — purely
consistency with the file's existing idiom.

### D2 — Status vocabulary: `'committed'`, `'expired'`, `'rolled_back'` (supersedes DISCUSS's `'committed'`-only framing)

Justified by the Context section's direct verification. `'active'` is
excluded — already exclusively owned by the existing reclaim `DELETE`,
unchanged, no overlap (a row cannot be both `'active'` and one of the
three terminal statuses simultaneously).

### D3 — Retention window: `retention_days: i64` constructor parameter, `EMBYR_AGENT_TRANSACTION_RETENTION_DAYS` env var, default `30`

`AgentTransactionSweeper::new` gains a new parameter:

```rust
pub fn new(
    pool: sqlx::PgPool,
    ttl_secs: i64,
    retention_days: i64,
    interval: std::time::Duration,
) -> Self
```

Config: `AgentConfig` (`crates/embyr-agent/src/config.rs`) gains
`transaction_retention_days: i64`, read from
`EMBYR_AGENT_TRANSACTION_RETENTION_DAYS` (optional, default `30`) via a new
`parse_optional_i64` helper, added alongside the file's existing
`parse_optional_u32`/`parse_optional_u64` helpers — same pattern, `i64`
because `sqlx::query::bind` needs to match the column's implicit numeric
domain used in the interval-multiplication expression (matching
`ttl_secs: i64`'s own existing type).

Env var name uses the `EMBYR_AGENT_*` prefix — every existing var in this
file's config surface does (`EMBYR_AGENT_DB_DSN`,
`EMBYR_AGENT_MAX_CONNS`, `EMBYR_AGENT_SHUTDOWN_TIMEOUT_SECS`, etc.) — a
deliberate deviation from the non-agent sibling's own
`EMBYR_TRANSACTION_RETENTION_DAYS` (no `_AGENT_` infix), because the two
crates each already establish their own independent, internally-consistent
env var namespace; matching the agent's own local convention beats
matching the sibling crate's unrelated one. Default value (`30`) mirrors
the sibling's own default for behavioral parity across backend modes, per
the feature's own stated goal ("every backend mode now has consistent,
bounded transaction-table growth").

### D4 — `sweep_once` return value: unchanged signature, summed count

`sweep_once(&self) -> Result<u64, sqlx::Error>` keeps its existing
signature. The returned `u64` becomes `reclaimed_rows + purged_rows`
(both statements' own `PgQueryResult::rows_affected()`, summed). Verified
safe: the only two call sites in the repo
(`crates/embyr-agent/src/server.rs`, which discards the return value into
`let _sweep_handle`, and
`tests/acceptance/embyr_agent/us_a04_transactions.rs`, which only asserts
`.expect("sweep_once must not error")` and never inspects the numeric
return value) do not depend on the return value's exact composition. A
combined count is simpler than introducing a new return type (e.g., a
`SweepResult { reclaimed: u64, purged: u64 }` struct) for zero current
consumer need — the crafter should introduce that struct later only if a
concrete need arises (e.g., per-status Prometheus counters), not
speculatively now.

### D5 — Composition root wiring (`crates/embyr-agent/src/server.rs::run`)

```rust
let sweeper = AgentTransactionSweeper::new(
    pool,
    60,
    config.transaction_retention_days,
    std::time::Duration::from_secs(30),
);
```

`config.transaction_retention_days` (an `i64`, `Copy`) remains readable
after `config.project_id` is moved into `StorageAgentService::new` at line
826 (Rust partial-move semantics — only the moved field becomes
unavailable; sibling fields, and `&config.listen_addr` at line 837, are
unaffected). No reordering of existing statements required.

## Alternatives Considered

### D1 alternative — merge reclaim and purge into one `WHERE` clause

Rejected. `WHERE (status = 'active' AND started_at < NOW() - $1 * INTERVAL
'1 second') OR (status IN (...) AND started_at < NOW() - $2 * INTERVAL '1
day')` is a single round-trip but obscures that two independently-tunable
policies (abandonment TTL vs. retention window) are being enforced, and
loses per-policy `rows_affected()` visibility that ADR-054's own D6
(metrics) precedent treats as valuable (even though this feature does not
add Prometheus counters — see D-Optional below — a future counter split
is cheaper against two statements than against one merged one). Matches
ADR-054's own D2 "two separate statements" precedent for the sibling
feature exactly.

### D2 alternative — purge `'committed'` only, per DISCUSS's literal AC text

Rejected as under-scoped. DISCUSS's own AC text names only `'committed'`
because its own Reading Confirmation cited `commit_transaction`'s success
path but not its reactive-expiry branch (line 944) or
`rollback_transaction` (line 1253-1254) — both in the identical shared
file DISCUSS did cite for the success path. Purging only `'committed'`
would ship a fix that still lets `'expired'` and `'rolled_back'` rows
accumulate forever, the identical defect class this feature exists to
close, discovered only because this ADR re-verified the shared code
directly rather than trusting the citation. This is a data-vocabulary
correction, not a scope-creep judgment call — the feature's own stated
Outcome KPI ("`transactions` table has bounded row-count growth") is not
met by a `'committed'`-only query, since `'expired'`/`'rolled_back'` rows
are part of the same table's unbounded growth.

### D3 alternative — reuse `EMBYR_TRANSACTION_RETENTION_DAYS` (no `_AGENT_` infix), matching the sibling literally

Rejected. Would break this file's own 100%-consistent `EMBYR_AGENT_*`
prefix convention for a var that lives in `AgentConfig`, for a marginal
and purely cosmetic cross-crate-naming benefit; an operator configuring
`embyr-agent` (a separately deployed binary, run inside a customer's own
VPC, per `docs/product/architecture/brief.md`'s deployment architecture)
never sets `embyr-server`'s env vars in the same place, so name reuse
buys no operational simplification, only inconsistency with every other
var in this specific config file.

### D3 alternative (Optional) — Prometheus counter for purged rows

Considered per the feature-delta's own Handoff Package note ("optional,
non-blocking... DESIGN's call"). **Deferred, not built.** `embyr-agent`
exposes no metrics endpoint today (confirmed: no `metrics`/`prometheus`
crate in `crates/embyr-agent/Cargo.toml`, no `/metrics` route in
`server.rs`) — adding one metrics-crate dependency and an HTTP route for a
single counter is a materially larger change than this feature's own
0.5-1 day estimate, and the feature's own Outcome KPI measurement plan
already specifies a manual operator spot-check (`SELECT count(*) FROM
transactions WHERE status = 'committed'`) as the v1 measurement method,
not an automated metric. Building agent-mode metrics infrastructure is a
separate, larger, unscoped feature — named here as a follow-up, not built
speculatively now (YAGNI, matches ADR-054's own `resolve_dsn_without_api_key`
extraction deferral discipline).

## Consequences

**Positive**: closes the full terminal-status accumulation gap, not just
the `'committed'` subset DISCUSS's own AC text named — verified against
the actual shared code rather than trusting a citation (Earned Trust
applied to the DISCUSS handoff itself); zero new files (pure extension of
`sweeper.rs` + `config.rs` + one call-site update in `server.rs`); zero new
dependency (`chrono` already present, unused by this change); config
naming stays internally consistent with every other `embyr-agent` env var.

**Negative**: `AgentTransactionSweeper::new`'s constructor signature
changes (new required parameter), a breaking change for its two existing
call sites (`server.rs`, `tests/acceptance/embyr_agent/us_a04_transactions.rs`)
— both are in-repo and must be updated in the same change; no external
consumer exists (the type is not exported outside the `embyr-agent` crate
in a way that affects any other crate). The purge query's `'expired'`/
`'rolled_back'` scope now exceeds DISCUSS's own literal AC text; the
acceptance-test/implementation step should verify all three statuses, not
only `'committed'`, even though the DISCUSS UAT scenarios only enumerate
`'committed'` — this ADR is the authoritative scope for the SQL, DISCUSS's
AC text is corrected by reference to it.
