# field-path-defense-in-depth — DISCUSS+DESIGN (combined, token-budget mode)

Closes finding #26 (Medium, Security/DB), `docs/product/production-readiness-audit-2026-09-08.md`.

## 1. Finding, restated

`validate_field_path` (`crates/embyr-core/src/domain/query.rs:16-24`) is the one gate every
client-supplied field path must pass before `embyr-pg-storage` raw-interpolates it into SQL text
(values are always `push_bind`, never at risk — only the field path itself is unparameterizable
in Postgres). Today the safety is **structural**: every current caller happens to validate
upstream before calling into the SQL-building functions. There is **no check at the SQL-building
functions themselves**. A future caller (new RPC, internal tool, test harness, or a
`sqlx::QueryBuilder`-based feature added later) that reaches these functions without validating
first gets silent SQL injection, not a rejected request.

This is a **different layer** than finding #11 (`agent-field-path-validation`, closed
2026-09-12): #11 deleted `embyr-agent`'s own *weak, duplicate* validator and pointed its one
caller at the real `embyr_core::domain::query::validate_field_path`. That fixed a wrong check
being called. This finding is about a boundary that currently has **no check at all** —
purely additive hardening, zero mechanism overlap with #11.

## 2. All call sites needing the guard (enumerated by reading, not assumed)

Grepped every `format!(` in `embyr-pg-storage` that interpolates a `field_path`/`fp` variable
into SQL text. 17 raw-interpolation sites total, collapsing to **7 guard-insertion points**
(inserting once at each function/block entry covers every `format!` downstream of it, since the
private helpers below are single-caller):

| # | Location | Function / block | `format!` sites covered |
|---|----------|-------------------|--------------------------|
| 1 | `crates/embyr-pg-storage/src/encoding/query.rs:45` | `append_field_filter(qb, f)` | Its own 5 (IsNan/IsNotNan/IsNull/IsNotNull/NotIn-empty, lines 49/53/64/68/170) **plus** everything in `push_scalar_comparison` (6 sites, lines 206/210/214/218/229/249/267), `push_array_contains` (1 site, line 291), `push_value_equality` (1 site, line 326) — all three are private (`fn`, not `pub fn`) with `append_field_filter` as their only caller, so one guard at the top of `append_field_filter` covers all 13 transitively |
| 2 | `crates/embyr-pg-storage/src/encoding/query.rs:335` | `order_by_expr(ob)` | line 340 |
| 3 | `crates/embyr-pg-storage/src/encoding/query.rs:344` | `order_by_expr_bigint(field_path, dir)` | line 345 — **dead code today** (grepped, zero callers anywhere in the workspace), but it's `pub fn` and part of the exported SQL-building surface, so still in scope |
| 4 | `crates/embyr-pg-storage/src/backend_adapter.rs:883-912` | `run_query`'s startAt/startAfter cursor block | 3 sites (lines 890, 897, 904), all keyed on the same `ob.field_path` |
| 5 | `crates/embyr-pg-storage/src/backend_adapter.rs:917-946` | `run_query`'s endAt/endBefore cursor block | 3 sites (lines 924, 931, 938), same `ob.field_path` |
| 6 | `crates/embyr-pg-storage/src/backend_adapter.rs:1097-1115` | `run_aggregation_query`'s `AggregationKind::Sum(field_path)` arm | line 1111-1114 |
| 7 | `crates/embyr-pg-storage/src/backend_adapter.rs:1146-1167` | `run_aggregation_query`'s `AggregationKind::Avg(field_path)` arm | line 1163-1166 |

**Explicitly excluded** (not field-path interpolation): `push_all_descendants_predicate`
(`encoding/query.rs:366-386`) interpolates `collection_id`, always via `push_bind` — including
its `format!("%/{collection_id}")` at lines 377/383, which is itself passed to `push_bind`, not
raw SQL text. Out of scope for this finding.

## 3. Design: validate at point of use

Each of the 7 sites gets one call to `embyr_core::domain::query::validate_field_path(path)`
immediately before the first `format!`/SQL-text construction that uses that path — not
per-`format!`-call (redundant within a single function/block that reuses the same path
variable across several branches).

## 4. Error handling: panic, not `Result`, not `debug_assert!`

**Decision: a real `panic!()` (always active, not `debug_assert!`), matching this file's own
existing convention for the identical category of defect.**

Reasoning, checked against this codebase's own precedent rather than assumed:

- **This file already has the pattern.** `append_field_filter`'s own match has
  `_ => panic!("unsupported filter op: {:?}", f.op)` (line 182) and
  `push_scalar_comparison` has `_ => panic!("unsupported filter value type: {:?}", value)`
  (line 280) — both are "assumed validated/filtered upstream, this can only fire on a caller
  bug" panics, in this exact file, on this exact class of already-supposedly-impossible input.
  A field path reaching these functions unvalidated is the same shape of defect. Consistency
  wins over inventing a third pattern.
- **`debug_assert!` was rejected**: it compiles out in `--release`. The whole point of this
  finding is a defense-in-depth guard that survives in *production*, not just in dev/test
  builds. A guard that's absent from the release binary defeats the finding.
- **`Result<_, CoreError>` was rejected**: it would require re-typing `append_field_filter`,
  `append_filter` (which recurses into itself for `Composite`/`CompositeOr`), `order_by_expr`,
  `order_by_expr_bigint`, and both cursor blocks and aggregation arms to propagate `?` — a
  wide, purely-defensive-code-driven signature change across a file with ~10+ call sites,
  for a codepath every current caller already proves unreachable. That blast radius isn't
  justified by a check whose only job is to catch a *future* bug that hasn't happened yet.
- **Panic here is safe, not a DoS risk**: this only fires when a *new, buggy* caller skips
  upstream validation — every real, current call path already validates before reaching these
  functions (confirmed: `handler.rs` and `server.rs` call `validate_field_path` before
  `translate_filter`/`proto_filter_to_domain` ever hand a path to `embyr-pg-storage`). Attacker
  -supplied field paths never reach this function unvalidated today, so this is not an
  attacker-triggerable panic — it is a build-breaks-loudly-in-CI safety net for a regression a
  human introduces later. Default `panic = "unwind"` (no override in workspace `Cargo.toml`,
  confirmed by grep) means a panic here fails only the one request task under tonic/tokio, not
  the whole server.

## 5. Performance cost — checked, not assumed

`validate_field_path` is a single linear scan over ASCII chars (`is_ascii_alphanumeric` / `_` /
`.`) on a field-path string that's realistically under ~100 bytes. Each of the 7 sites calls it
**once per field path per query build** (not per row, not per document) — e.g. one call per
filter clause, one per order-by clause, one per aggregation field. This is a double-validation
relative to the existing upstream RPC-boundary check, but the second call costs low-nanoseconds
against a query that's about to do a network round-trip to Postgres. Negligible — stated
explicitly rather than silently assumed, per the task's own instruction.

## 6. ADR decision: none needed

Confirmed by checking: this reuses `validate_field_path`, a function that already exists,
is already tested (`crates/embyr-core/src/domain/query.rs` proptest + unit tests), and is
already the established single source of truth (per ADR-040 §1, ADR-072 §A, and the
`agent-field-path-validation` evolution doc). Applying it at 7 additional call sites inside the
one crate that already assumes it is a hardening/enforcement change, not a new architectural
decision — no alternatives-with-trade-offs to weigh. No new ADR file.

## 7. Self-review

- **Closes the finding**: yes — every SQL-building function that raw-interpolates a field path
  now independently proves its own input is safe immediately before building SQL, rather than
  trusting that every past-and-future caller upstream got it right. Defense-in-depth now exists
  at the dangerous boundary itself, not just at the RPC ingestion boundary.
- **Not redundant/wasteful**: 7 guard insertions is the minimum that covers all 17
  interpolation sites (verified: no smaller placement is possible without missing a branch;
  no larger placement is needed since the 3 private helpers are single-caller).
- **No overlap with #11**: confirmed architecturally distinct — #11 was "the wrong validator is
  being called" (a correctness bug in `embyr-agent`); this finding is "no validator is called
  at this boundary at all" (a defense-in-depth gap in `embyr-pg-storage`). Different crate,
  different defect class, same underlying `validate_field_path` function reused, zero
  duplicated work.

## Out of scope / follow-ups (named, not silently dropped)

- `order_by_expr_bigint` is dead code (zero callers) — hardening it is cheap and included above
  since it's `pub fn`, but a separate/future cleanup could consider removing it instead if it
  stays unused.
- No production code is changed by this document — DESIGN-only per task constraints. DELIVER
  wave implements the 7 guard insertions plus one unit test per new-in-2026-09 pattern (call
  each guarded function with a deliberately invalid field path — e.g. containing `'` or `;` —
  and assert it panics) using existing `#[should_panic]` conventions; no new test infra needed.
