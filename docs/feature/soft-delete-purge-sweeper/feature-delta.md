# Feature Delta: soft-delete-purge-sweeper

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/product/production-readiness-audit-2026-09-08.md` finding #6 confirmed by direct reading:
"Documented 168h soft-delete purge sweeper does not exist. `deleted_at` is write-only (set once on
soft-delete, never read anywhere else) — soft-deleted projects, including ECIES-encrypted customer
DSNs/credentials, persist forever, contradicting an explicit admin-UI promise." Location cited:
`crates/embyr-server/src/admin/handlers/lifecycle.rs:181`; `crates/embyr-admin-ui/src/views/db_detail/
mod.rs:282`. Severity: **Blocker**. Status (at DISCUSS start): **Not started**.

✓ `crates/embyr-server/src/admin/handlers/lifecycle.rs` (full, 197 lines) — `delete_project`
(lines 165-196) cascade-revokes `sdk_api_keys` (lines 172-178, sets `revoked_at = now()`), then sets
`UPDATE projects SET status = 'deleted', deleted_at = now(), updated_at = now() WHERE id = $1 AND
status != 'deleted'` (lines 180-183). Confirmed by full-file read: `deleted_at` is written exactly
once, here, and this file contains no other read of it. Repo-wide grep (`grep -rn deleted_at
crates/embyr-server/src`) returns exactly this one write site — confirms the finding's own
"write-only" framing precisely, no additional hidden reader exists.

✓ `crates/embyr-admin-ui/src/views/db_detail/mod.rs` lines 260-300 (delete confirmation modal) —
literal UI copy confirmed verbatim: `"This database and all its SDK keys will be permanently deleted.
Soft-delete with a 168 h grace window before data purge."` (lines 280-283). The modal's own "Delete"
button (lines 290-294) currently only closes the modal in this V1-mock UI — no discrepancy between UI
and server behavior beyond the promise itself being unfulfilled server-side, which is this finding's
own exact scope.

✓ `crates/embyr-server/src/sweepers/mod.rs` (full, 49 lines) and `crates/embyr-server/src/sweepers/
cap_usage_refresher.rs` (full, 190 lines) and `crates/embyr-server/src/sweepers/transaction_sweeper.rs`
(full, 259 lines) read directly, per the task's own explicit instruction, to understand the
established reuse pattern in full — not merely its shape from a summary. Confirms:
1. `advisory_lock_key(s: &str) -> i64` (`sweepers/mod.rs:18-27`) — a shared FNV-1a hash helper, used
   as the `pg_try_advisory_lock` key by every sweeper in this module. Reused unchanged by design.
2. Both existing sweepers share an IDENTICAL shape: `tokio::spawn` an infinite `tokio::time::interval`
   loop; each tick, `pool().acquire()` ONE `PoolConnection` (session affinity requirement, documented
   in both files' own comments), `pg_try_advisory_lock` on it, skip the cycle silently if not
   acquired (another instance already holds it, or the probe itself failed), run the cycle body,
   `pg_advisory_unlock` on the SAME connection.
3. **Structural difference between the two existing sweepers, directly relevant to this feature's own
   design**: `CapUsageRefresher` (`cap_usage_refresher.rs:100-188`, `run_cycle`) operates ENTIRELY
   against `SystemDb` — no customer-database connection of any kind. `TransactionSweeper`
   (`transaction_sweeper.rs:127-254`) additionally loops over every `SystemDb::list_pg_reachable_
   projects()` row and opens a SEPARATE, per-project connection to each CUSTOMER database (via
   `resolve_dsn_without_api_key` + `PostgresBackendAdapter::new(&dsn)`), because its own target data
   (the `transactions` table) lives in the customer's own Postgres, not the system DB.
   **This feature's own target data (`projects.ecies_encrypted_dsn` / `backend_pg_dsn_enc` /
   `agent_tls_bundle_enc`) lives in the SYSTEM DB itself** (confirmed below) — structurally, this
   sweeper is a closer match to `CapUsageRefresher`'s shape (one `SystemDb`-only UPDATE per cycle, zero
   customer-DB connections, zero DSN resolution) than to `TransactionSweeper`'s per-project connection
   loop. Named explicitly so DESIGN does not default to the more complex sibling by habit.
4. Idempotency is load-bearing, not incidental, in both existing sweepers: `CapUsageRefresher`'s own
   `set_project_status` reuses a `WHERE status IN (...)` clause explicitly documented as idempotent
   (`lifecycle.rs:21-31`); `TransactionSweeper`'s reclaim/purge statements are plain `UPDATE .. WHERE
   status = 'active' AND ..` / `DELETE .. WHERE status IN (...) AND ..`, both naturally idempotent
   (a repeat run over an already-transitioned row matches zero rows, `rows_affected() == 0`, harmless).
   This is why NEITHER sweeper treats "lock not acquired this cycle" as an error — a skipped cycle is
   not a missed obligation, it is picked up next tick. **This feature's own natural mechanism (an
   `UPDATE .. SET col = NULL WHERE .. AND col IS NOT NULL`) is idempotent by the same construction.**

✓ `migrations/0001_initial_schema.sql` lines 2-11 (`CREATE TABLE projects`) read directly —
`ecies_encrypted_dsn BYTEA` is a column on `projects` itself, in the **system DB**, not a
customer-database table. `migrations/0015_projects_admin_columns.sql` (full, 6 lines) adds
`backend_pg_dsn_enc BYTEA` (AES-256-GCM, per `admin-api-v2`'s own precedent) to the SAME `projects`
row. `migrations/0005_agent_endpoint.sql` (full, 5 lines) adds `agent_tls_bundle_enc BYTEA` to the SAME
row, with its own comment confirming it is "ECIES-encrypted" mTLS client-cert/key material for
`backend_mode='agent'` projects. **All three sensitive-credential columns this finding's own framing
names ("ECIES-encrypted customer DSNs/credentials") live directly on the `projects` row in the system
DB** — confirms point 3 above directly from schema, not inference.

✓ `migrations/0006_aws_secret.sql` and `migrations/0007_gcp_secret.sql` (full, 5 lines each) read
directly — `backend_secret_arn` / `backend_secret_gcp` are each explicitly commented "a reference, NOT
a credential" in their own migration files. These are NOT in scope for this finding's own
"credentials persist forever" concern — the actual secret material they reference lives in AWS/GCP
Secrets Manager, outside this system entirely, and revoking that reference is a separate,
un-evidenced concern this DISCUSS does not fold in.

✓ **Full-repo grep for `REFERENCES projects` (34 matches across `migrations/` and `docs/`) performed
directly, per the task's own explicit instruction, to determine whether a hard `DELETE FROM projects`
is safe.** Result: it is **not** safe, confirmed directly, not assumed:
  - `migrations/0012_admin_sdk_api_keys.sql:3` — `sdk_api_keys.project_id TEXT NOT NULL REFERENCES
    projects(id)` — **no `ON DELETE` clause at all**, which is Postgres's `NO ACTION`/`RESTRICT`
    default. Every project has at least one `sdk_api_keys` row created at provisioning time (JOB-02);
    `delete_project` itself only sets `revoked_at`, it never deletes these rows (`lifecycle.rs:172-178`
    re-read above). A hard `DELETE FROM projects WHERE id = $1` while ANY `sdk_api_keys` row for that
    project still exists raises a foreign-key-violation error and fails outright.
  - `migrations/0025_access_rule_history.sql:6`, `migrations/0026_write_access_rule_history.sql:9`,
    `migrations/0033_access_rule_pattern_history.sql:8` — all three `REFERENCES projects(id)` with
    **no `ON DELETE` clause**, same `RESTRICT` default. `access_rule_history.sql`'s own header comment
    (lines 1-3) states explicitly: *"Never UPDATEd or DELETEd (append-only invariant, locked by
    DISCUSS)."* This is a pre-existing, explicit, named business invariant from a prior feature
    (`security-rules-operations`, ADR-035) — a hard `DELETE FROM projects` would either fail the same
    FK-violation way `sdk_api_keys` does (if any history rows exist, which they do for any project
    that ever had an access rule), or — if someone "fixed" this by cascading — would silently violate
    that prior feature's own append-only audit-trail invariant.
  - By contrast, the great majority of the other 27 `REFERENCES projects(id)` matches (e.g.
    `0002_metrics.sql`, `0018_rate_buckets.sql`, `0022_access_rules.sql`, `0023_write_access_
    rules.sql`, `0024_group_access_rules.sql`, `0029_oauth_provider_credentials.sql`, and others) DO
    specify `ON DELETE CASCADE` — these would not themselves block a hard delete.
  - **Conclusion, locked by this DISCUSS, not left open for DESIGN**: a hard `DELETE FROM projects`
    for a purge sweeper is not evidenced-safe — it structurally conflicts with at least one explicit,
    prior-feature-locked data-retention invariant (`access_rule_history`'s append-only guarantee) and
    would additionally require either deleting or cascading `sdk_api_keys` rows the delete-project flow
    deliberately preserves (revoked, not removed) today. See § Business Context for the resulting
    "purge = null the sensitive columns, not drop the row" outcome this finding locks.

✓ Repo-wide grep for `restore|reactivate|undelete` under `crates/embyr-server/src` (case-insensitive)
and a follow-up targeted grep for `fn undelete|fn restore|"/restore"|"/undelete"|reactivate_deleted`
— **zero matches**. Confirms directly: no self-service or operator "undelete a soft-deleted project"
route exists anywhere in this codebase today. The 168h grace window is, today, a promise with nothing
built on either side of it — nothing purges after it, and nothing recovers during it. This feature
closes only the first half (per the finding's own scope); the second half (an actual restore mechanism
consuming the grace window) is named as a related, out-of-scope gap below, not silently ignored.
`docs/feature/user-admin-ui/feature-delta.md:436` (AC-011-05, the sibling account-level Danger-Zone
delete) independently corroborates the same absence at the account level: *"Non-reversible from UI
(operator intervention required to restore)"* — the established pattern in this codebase is that no
UI-driven restore exists for either deletion tier.

✓ `crates/embyr-server/src/adapters/system_db.rs` lines 1694-1724 (`list_pg_reachable_projects`) and
lines 187-198 (`SweeperProjectRow`) re-read directly, confirming `TransactionSweeper`'s own enumeration
query and row shape — evaluated as a candidate reuse target and found NOT to be the right one for this
feature (see point 3 above): that query filters to `backend_mode IN ('direct_pg', 'aws_secret',
'gcp_secret')` specifically because those three modes need a resolvable customer DSN. This feature has
no such restriction — a `'deleted'` project of ANY `backend_mode` (including `'agent'`) still has a
system-DB row with potentially-populated sensitive columns that must be purged; excluding `agent` mode
here (unlike `TransactionSweeper`'s own necessary exclusion) would leave exactly the credential class
(`agent_tls_bundle_enc`) this feature exists to purge for `agent`-mode customers.

✓ `crates/embyr-server/src/config.rs` lines 95-134 and `crates/embyr-server/src/main.rs` lines 246-280
read directly — confirms the established config-and-wiring convention: `EMBYR_CAP_CHECK_INTERVAL_SECS`
(default 30s) and `EMBYR_TRANSACTION_SWEEP_INTERVAL_SECS` (default 300s) / `EMBYR_TRANSACTION_
RETENTION_DAYS` (default 30, "mirroring `SessionCleaner`'s own documented 30-day precedent" per that
field's own doc comment) are both plain `std::env::var(...)` reads with a hardcoded numeric default,
threaded into `ServerConfig`, then passed directly into each sweeper's own `spawn(...)` call
(`main.rs:251-280`) — no dynamic reconfiguration, no external scheduler. This is the established
pattern this feature's own two new config values (sweep interval, grace-window length) should follow.

✓ `docs/product/jobs.yaml` read in full (all 20 jobs, both halves of the file). No job named
"compliance" or "data-retention" exists — confirmed directly, not assumed; the task's own suggested
alternative framing does not correspond to an actual existing job in this file. Two real candidates
were evaluated:
  - **JOB-13 (`production-deployment`, persona P2 Sam Chen)** — evaluated and rejected. This exact
    question (does a new background-sweeper mechanism belong under JOB-13) was already litigated by
    two prior features in this session: `realtime-listener-reconnect` reasoned explicitly against
    reusing JOB-13 for a background-mechanism concern, and `composite-index-real-creation`'s own § Job
    (re-read directly above) restates that reasoning verbatim: *"JOB-13 (`production-deployment`) is
    about safe, fail-fast STARTUP/request-admission-time configuration... not a background-mechanism
    concern."* This feature is squarely a background mechanism, not a startup/admission-time one — the
    same reasoning applies unchanged.
  - **JOB-12 (`observability`, persona P2 Sam Chen)** — evaluated. `customer-db-transaction-sweeper`
    (the `TransactionSweeper` sibling read above) WAS filed under JOB-12, but its own jobs.yaml NOTE
    (lines 681-710, re-read directly) is explicit about why: it "adds a real operator-observable
    surface (the two counters above), not zero user-visible behavior change" — i.e., JOB-12 fit
    because that feature's OWN outcome IS a new Prometheus metrics surface Sam Chen queries to
    diagnose problems. This feature's own primary outcome (§ Business Context) is credential-lifecycle
    correctness, not diagnostic visibility — Prometheus counters are a natural secondary addition
    (mirroring the sibling's own `embyr_transaction_sweeper_reclaimed_total`/`_purged_total` pattern,
    named in § Technical Notes below) but are not the outcome itself, so JOB-12 is a weaker primary fit
    than it was for that sibling feature.
  - **JOB-10 (`account-admin`, persona P5 Chris) — selected.** `docs/feature/user-admin-ui/feature-
    delta.md` read directly (§ US-003, lines 214-236): `job_id: JOB-10`, AC-003-05 is the EXACT story
    that introduced the delete-database action and its own soft-delete semantics
    (`sets deleted_at ... Cascades to revoke all SDK keys ... Deleted databases no longer appear in
    list`) — the SAME action whose own confirmation modal (`db_detail/mod.rs:280-283`, re-read above)
    makes the specific "168h grace window before data purge" promise this finding names as broken.
    JOB-10's own recorded emotional dimension ("feel confident keys and access are under my
    governance") and social dimension ("demonstrate clean secrets hygiene to auditors") match this
    finding's own stated compliance/security-hygiene weight precisely — this is JOB-10's own
    admin-delete promise, finally made real, mirroring this codebase's established "make it real"
    pattern (`admin-api-v2` -> JOB-10's own backend half of `user-admin-ui`; `card-payments-backend`
    -> JOB-14; `aggregation-queries`/`batch-get-documents`/etc. -> JOB-01) rather than inventing a new
    job for what is a direct, evidenced completion of an existing one.

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

- Feature type: **Backend / Reliability-security fix** — closes a Blocker-severity finding by making an
  already-shipped, unmodified admin-UI promise and an already-shipped, unmodified `deleted_at` column
  genuinely true. Zero SDK/data-plane proto change, zero new admin API route, zero new UI change.
- JTBD: **reuse JOB-10** (`account-admin`, P5 Chris) — direct completion of US-003/AC-003-05's own
  soft-delete promise, the "make it real" pattern (§ Reading Confirmation, final bullet).
- Walking Skeleton: **Yes, and the whole feature** — this is a single right-sized story (§ Scope
  Assessment); the walking skeleton IS the release.
- UX Research Depth: **Lightweight** — a background, non-interactive server mechanism; zero new
  admin-UI surface, zero new emotional arc, zero new TUI/journey artifact warranted (matches
  `customer-db-transaction-sweeper`'s and `composite-index-real-creation`'s own identical precedent for
  background-mechanism features).
- **This DISCUSS locks the OUTCOME** (sensitive credential columns on a `'deleted'` project's row are
  genuinely nulled once, and only once, the grace window has elapsed — never before, never left
  ambiguous) **and locks one mechanism-level finding directly answered by evidence** (hard `DELETE FROM
  projects` is not safe — § Reading Confirmation). It leaves the exact SQL/config-naming shape (§
  Central Design Questions) to DESIGN, per this session's established "lock the outcome, flag the
  mechanism" discipline.

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: P5 Chris (Account Admin / Platform Engineer), primary — the account admin who reads and
trusts the literal "168h grace window" promise in the delete-confirmation modal (`db_detail/mod.rs`),
and whose own JOB-10 emotional/social dimensions ("feel confident... under my governance"; "demonstrate
clean secrets hygiene to auditors") are directly at stake.

**Secondary stakeholder**: P2 Sam Chen (Service Operator / Platform Engineer) — operates the
`embyr-server` fleet the sweeper runs inside, is the one who would need to explain to an external
auditor (mirroring JOB-09's own audit-evidence framing) why a soft-deleted customer's encrypted DSN is
still sitting in the system DB a year later, if this feature did not exist. Named explicitly, not
given a separate story, mirroring `composite-index-real-creation`'s own identical primary/secondary
persona split for a background production-safety mechanism.

**Job**: **JOB-10 `account-admin`**, unchanged job_story. Direct completion of US-003/AC-003-05's own
delete-database promise (§ Reading Confirmation, final bullet) — the admin-ui feature that introduced
the promise this feature now backs with real server behavior.

## Wave: DISCUSS / [REF] Business Context

Real deletion, as Chris experiences it today: he clicks "Delete" on a stale database in the admin UI,
reads "Soft-delete with a 168h grace window before data purge," confirms, and `delete_project`
(`lifecycle.rs:165-196`) revokes SDK keys and flips `status='deleted'`/`deleted_at=now()`. Nothing else
in this codebase ever reads `deleted_at` again (confirmed by full-repo grep, § Reading Confirmation).
The project's `ecies_encrypted_dsn` / `backend_pg_dsn_enc` / `agent_tls_bundle_enc` — real, working,
ECIES/AES-GCM-encrypted credentials for a real customer's own Postgres or agent mTLS bundle — sit in
the system DB unchanged forever. The "168h grace window" promise Chris read is, today, meaningless: it
does not describe a real recovery window (no restore route exists — confirmed above) and it does not
describe a real purge deadline (nothing purges, ever).

**What "purge" genuinely means here, locked by this DISCUSS's own FK investigation (§ Reading
Confirmation)**: a hard `DELETE FROM projects` is not evidenced-safe — it structurally conflicts with
`access_rule_history`'s own explicit, prior-feature-locked append-only invariant (no `ON DELETE
CASCADE`) and would additionally require deleting or cascading `sdk_api_keys` rows that `delete_project`
itself deliberately preserves in revoked form today. **The purge this feature builds nulls the three
sensitive credential columns on the `'deleted'` project's own row — `ecies_encrypted_dsn`,
`backend_pg_dsn_enc`, `agent_tls_bundle_enc` — once the grace window has elapsed, leaving the row itself,
its `id`/`status`/`deleted_at`/`account_id`/timestamps, and every FK-referencing child row (billing
history via `daily_project_metrics`, `sdk_api_keys`, `access_rule_history` and siblings) exactly where
they already are today.** This directly and completely closes the finding's own stated harm ("including
ECIES-encrypted customer DSNs/credentials, persist forever") without touching anything the finding did
not name, and without silently breaking any other feature's own already-locked data-retention
invariant. `backend_secret_arn`/`backend_secret_gcp` (AWS/GCP Secrets Manager references, explicitly
"NOT a credential" per their own migration comments, § Reading Confirmation) are out of scope — the
finding's own framing is about credentials embyr itself stores encrypted, not about references to a
customer's own external secret store.

### Central Design Questions (opening recommendations offered, not locked)

**Question A — grace-window config shape and default.** The UI's own literal promise is "168h" — this
DISCUSS locks that the SHIPPED DEFAULT must equal 168 hours (7 days), so the promise Chris already
reads becomes true on day one; the promise itself is not part of this feature's own scope to change.
**Opening recommendation**: `EMBYR_SOFT_DELETE_GRACE_DAYS`, default `7` — a days-typed config value,
matching `EMBYR_TRANSACTION_RETENTION_DAYS`'s own established shape (§ Reading Confirmation) exactly,
rather than inventing an hours-typed variable; 168 hours and 7 days are the identical duration, and the
days-typed precedent is already established in this exact module. DESIGN may choose hours instead if a
concrete reason emerges, but should not invent a third unit convention without one.

**Question B — sweep interval default.** No sub-hour precision is evidenced as necessary for a
week-scale grace window (unlike `CapUsageRefresher`'s 30s cap-enforcement latency requirement or
`TransactionSweeper`'s 60s abandonment threshold). **Opening recommendation**:
`EMBYR_SOFT_DELETE_SWEEP_INTERVAL_SECS`, default on the order of one hour (e.g. `3600`) — coarse enough
to avoid meaningfully increasing steady-state Postgres load, fine enough that the worst-case delay past
the grace window is negligible relative to the 7-day window itself. DESIGN should confirm the exact
default against no stronger evidence than this reasoning, since none exists in this codebase today.

**Question C — single system-DB UPDATE vs. per-row loop.** § Reading Confirmation's own structural
finding (point 3) is that this sweeper's target data lives entirely in the system DB — a single
`UPDATE projects SET ecies_encrypted_dsn = NULL, backend_pg_dsn_enc = NULL, agent_tls_bundle_enc = NULL
WHERE status = 'deleted' AND deleted_at < now() - interval '{grace} days' AND (ecies_encrypted_dsn IS
NOT NULL OR backend_pg_dsn_enc IS NOT NULL OR agent_tls_bundle_enc IS NOT NULL)` per cycle is the
evidently-simplest mechanism mirroring `CapUsageRefresher`'s own single-query-per-cycle shape (not
`TransactionSweeper`'s per-project connection loop, which this feature has no structural need for).
Named as the strong default, not locked, since DESIGN may find a reason (e.g. wanting a per-row
`metrics::counter!` increment, mirroring `TransactionSweeper`'s own `_reclaimed_total`/`_purged_total`
pattern, which a single bulk `UPDATE`'s `rows_affected()` still supports without a per-row loop) to
shape it differently.

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

**Signals checked**: >10 user stories? No (1). >3 bounded contexts/modules? No — confined entirely to
`embyr-server`'s own `sweepers/` module (one new file, mirroring two existing siblings) plus 2 new
`ServerConfig` fields and their `main.rs` wiring; zero change to `embyr-core`, `embyr-pg-storage`,
`embyr-admin-ui`, or any admin HTTP route. Zero schema migration needed — `deleted_at`,
`ecies_encrypted_dsn`, `backend_pg_dsn_enc`, and `agent_tls_bundle_enc` all already exist (§ Reading
Confirmation) — smaller in scope than either sibling sweeper feature, both of which needed at least
config wiring of comparable size and, in `TransactionSweeper`'s case, a two-slice split. Walking
skeleton >5 integration points? No (1): a real `'deleted'` project's real credential columns, nulled by
a real background cycle, verified by directly querying the system DB's own `projects` row before and
after. Estimated effort >2 weeks? No — 1 story, 1-1.5 days, well under any threshold. Multiple
independent user outcomes? No — a single, indivisible outcome (Chris's delete promise becomes true).

**Scope Assessment: PASS** (0 oversizing signals fired) — right-sized as one feature, one story.

## Wave: DISCUSS / [REF] Story Map & Walking Skeleton

### Backbone

Chris deletes a database, reads the 168h promise, and trusts it (already shipped, unchanged) → the
project's own encrypted credentials sit untouched in the system DB indefinitely (today's bug) → a
background sweeper wakes on its own interval, guarded so redundant instances don't collide → any
`'deleted'` project whose grace window has genuinely elapsed has its sensitive columns nulled, exactly
once, never before the window closes → a project still inside its grace window, or never deleted at
all, is left completely untouched, cycle after cycle.

### Walking Skeleton

**US-01** (the entire feature, § Scope Assessment) — the single riskiest, highest-value assumption:
can the grace window and the multi-instance-safe purge both be built as a straightforward reuse of the
already-proven `sweepers/` shape, with zero new schema and zero new admin route, end to end.

### Release 1 — Chris's Delete Promise Becomes True (US-01)

Sequenced as the only release — this feature has no natural second increment; splitting the grace-window
negative case, the multi-instance safety guard, or the config wiring into separate stories would violate
the "split by independently-demonstrable user outcome, not by technical layer" rule (§ nw-leanux-
methodology), since none of those facets is separately valuable to Chris or Sam Chen on its own — all
three are necessary conditions of the SAME outcome, mirroring `CapUsageRefresher`'s own advisory-lock
mechanism never having been split into its own story.

## Wave: DISCUSS / [REF] Elephant Carpaccio Slices

| # | Story | Release | Estimate | Learning Hypothesis | Reference Class |
|---|---|---|---|---|---|
| 01 | US-01 | 1 (Walking Skeleton) | 1-1.5 days | Disproves: this exact grace-window-gated, multi-instance-safe purge cannot be built as a straightforward reuse of the existing `sweepers/` shape without a materially new mechanism class | Nearest and closest-structural-match reference class is `CapUsageRefresher` (ADR-020) — a single system-DB-only `UPDATE` per cycle, zero customer-DB connection — narrower in scope than `TransactionSweeper`'s own per-project connection loop, which this feature does not need |

## Wave: DISCUSS / [REF] Prioritization

Single story, no ordering decision to make — see § Story Map & Walking Skeleton.

## Wave: DISCUSS / [REF] System Constraints

- Zero schema migration required — `projects.deleted_at`, `.ecies_encrypted_dsn`, `.backend_pg_dsn_enc`,
  and `.agent_tls_bundle_enc` all already exist (§ Reading Confirmation).
- The purge mechanism is a column-level `UPDATE ... SET col = NULL`, never a `DELETE FROM projects` —
  a hard row delete is confirmed unsafe by direct FK-reference investigation (§ Reading Confirmation)
  and is out of this feature's own scope, not merely deferred.
- `sdk_api_keys`, `access_rule_history`, `write_access_rule_history`, `access_rule_pattern_history`, and
  every other row referencing the deleted project's `id` are UNCHANGED by this feature — they already
  persist today (some by explicit, prior-feature-locked append-only invariant) and continue to.
- The purge query must run against `SystemDb` only — no customer-database connection, no DSN
  resolution, no `AwsSecretFetcher`/`GcpSecretFetcher` dependency (unlike `TransactionSweeper`) — this
  is a hard structural simplification this feature must take advantage of, not an optional one.
- ALL `backend_mode` values (`direct_pg`, `aws_secret`, `gcp_secret`, `agent`) are in scope — unlike
  `TransactionSweeper`'s own necessary `backend_mode` filter (needed only because it must resolve a
  customer DSN), this feature has no reason to exclude any backend mode; `agent`-mode projects are the
  ones holding `agent_tls_bundle_enc`, a credential class this feature must not skip.
- The shipped default grace window MUST equal 168 hours (7 days) — a hard regression guard matching the
  admin-UI's own already-shipped, unmodified promise text (`db_detail/mod.rs:282`, this feature does
  not touch that file).
- New PURE logic this feature adds (the eligibility predicate: is a given `(status, deleted_at)` pair
  past its grace window) gets unit tests written during DELIVER, consistent with this session's own
  accumulated mutation-testing discipline; a `cargo-mutants` pass is still budgeted at QUALITY_GATE.

## Wave: DISCUSS / [REF] User Stories

### US-01: Chris's Soft-Deleted Database's Encrypted Credentials Are Genuinely Purged After the Grace Window (Walking Skeleton)

**job_id**: JOB-10 | **Release**: 1 | **Persona**: P5 Chris (primary); P2 Sam Chen (secondary,
operational/compliance stakeholder)

#### Elevator Pitch
Before: Chris, an Account Admin at Trailmark Inc., clicks "Delete" on a stale `trailmark-staging`
database in the admin UI. The confirmation modal reads "This database and all its SDK keys will be
permanently deleted. Soft-delete with a 168h grace window before data purge." Chris confirms — SDK keys
are revoked, the project row flips to `status='deleted'` — but `trailmark-staging`'s own ECIES-encrypted
Postgres DSN sits unchanged in the system DB forever afterward, for the life of the deployment, no
matter how much time passes.
After: 7 days (168h) after Chris's delete, the SAME row's `ecies_encrypted_dsn` / `backend_pg_dsn_enc` /
`agent_tls_bundle_enc` columns are genuinely `NULL` — verified by directly querying the system DB's own
`projects` row for `trailmark-staging`, not by trusting the modal's own text.
Decision enabled: Chris can trust that clicking Delete does exactly what the modal told him it would do,
and can tell his own security/compliance reviewers (mirroring JOB-09's own audit-evidence pattern) that
a deleted database's credentials do not persist indefinitely in embyr's own system DB — closing exactly
the question an auditor would ask about a "deleted" customer's data.

#### Who
- Chris (P5) | Account Admin at a customer account (e.g. Trailmark Inc.) with Owner or Admin role in the
  admin UI | Already trusts the delete-confirmation modal's own literal promise (unchanged by this
  feature) | Needs that promise to be genuinely true, not merely displayed.
- Sam Chen (P2, secondary) | Service operator running `embyr-server` in production | Needs deleted
  customers' encrypted credentials to be genuinely gone within a bounded, documented window, so an
  external security/compliance audit of "what happens to a deleted customer's data" has a true, evidenced
  answer instead of an indefinite retention gap.

#### Solution
A new background sweeper (`sweepers::soft_delete_purge_sweeper`, mirroring the existing `sweepers/`
module's interval-loop + advisory-lock shape) periodically nulls the sensitive credential columns
(`ecies_encrypted_dsn`, `backend_pg_dsn_enc`, `agent_tls_bundle_enc`) on any `projects` row whose
`status = 'deleted'` and whose `deleted_at` is at least the configured grace window (default 168h/7
days, matching the admin-UI's own unchanged promise) in the past. Runs against the system DB only — no
customer-database connection, no DSN resolution (§ System Constraints). The row itself, its `id`/
`status`/`deleted_at`/`account_id`, and every other table's own rows referencing that `id` are
unchanged — this feature purges credentials, not the row (§ Business Context). Exact config-variable
naming/typing and cycle-interval default are DESIGN's own investigation (§ Central Design Questions A,
B, C).

#### Domain Examples

**Example 1 (Happy Path — a project past its grace window is genuinely purged)**: Trailmark Inc.'s
`trailmark-staging` project (`backend_mode=direct_pg`) was deleted by Chris 8 days ago
(`deleted_at = now() - interval '8 days'`), with a real `ecies_encrypted_dsn` value populated at
provisioning time. Before this feature: that value is unchanged, still fully readable, 8 days, 8 months,
or 8 years later. After this feature: the next sweep cycle nulls `ecies_encrypted_dsn` for that row —
verified by directly querying `projects` for `trailmark-staging` and confirming the column is `NULL`.

**Example 2 (Edge Case — a project still inside its grace window is left completely alone)**: Trailmark
Inc.'s `trailmark-demo` project was deleted by Chris 3 days ago (`deleted_at = now() - interval '3
days'`), well inside the 168h/7-day window. The sweeper runs its normal cycle. `trailmark-demo`'s own
`ecies_encrypted_dsn` remains exactly as it was — the sweeper's own `WHERE deleted_at < now() -
interval '7 days'` predicate structurally excludes it; no code path treats "inside the window" as a
special case requiring its own guard.

**Example 3 (Error/Boundary — an already-purged row is left alone on the next cycle, no error, no
redundant work)**: `trailmark-staging` (Example 1) was already purged on a prior cycle — its credential
columns are already `NULL`. The next sweep cycle runs again (its own interval tick, unrelated to whether
any row needs work). The `WHERE ... AND (ecies_encrypted_dsn IS NOT NULL OR ...)` predicate matches zero
rows for `trailmark-staging`; `rows_affected() == 0` for that row, no error, no redundant `UPDATE`
issued against an already-`NULL` column.

#### UAT Scenarios (BDD)

```gherkin
Scenario: A database soft-deleted more than the grace window ago has its encrypted credentials purged
  Given Trailmark Inc.'s "trailmark-staging" project was soft-deleted 8 days ago
  And its ecies_encrypted_dsn column holds a real encrypted value
  When the soft-delete purge sweeper's next cycle runs
  Then trailmark-staging's ecies_encrypted_dsn column is NULL
  And this is verified by directly querying the system DB's own projects row, not an in-memory
    assumption

Scenario: A database still inside its 168-hour grace window is left completely untouched
  Given Trailmark Inc.'s "trailmark-demo" project was soft-deleted 3 days ago
  And its ecies_encrypted_dsn column holds a real encrypted value
  When the soft-delete purge sweeper's next cycle runs
  Then trailmark-demo's ecies_encrypted_dsn column is unchanged, still holding its original value

Scenario: An already-purged database is left alone on a later cycle, without error
  Given a project's sensitive credential columns were already nulled by a prior sweep cycle
  When the soft-delete purge sweeper runs again
  Then no error occurs
  And no further write is issued against that project's already-null columns

Scenario: An active (never soft-deleted) database is never touched, regardless of age
  Given Trailmark Inc.'s "trailmark-prod" project has status "active" and was created 2 years ago
  When the soft-delete purge sweeper's next cycle runs
  Then trailmark-prod's ecies_encrypted_dsn column is unchanged

Scenario: The sweeper runs safely across multiple embyr-server instances without duplicate or
  conflicting work
  Given two embyr-server instances are running concurrently, both with the purge sweeper active
  And a project has just become eligible for purge
  When both instances' sweep cycles tick at approximately the same time
  Then exactly one instance performs the purge for that project
  And neither instance errors or crashes as a result of the other instance's concurrent attempt

Scenario: An agent-mode project's encrypted mTLS bundle is purged identically to a direct_pg project's
  encrypted DSN
  Given a backend_mode=agent project was soft-deleted 8 days ago
  And its agent_tls_bundle_enc column holds a real encrypted value
  When the soft-delete purge sweeper's next cycle runs
  Then that project's agent_tls_bundle_enc column is NULL
```

#### Acceptance Criteria
- [ ] AC-SDP-01: a `projects` row with `status = 'deleted'` and `deleted_at` at least the configured
      grace window (default 168h/7 days) in the past has `ecies_encrypted_dsn`, `backend_pg_dsn_enc`,
      and `agent_tls_bundle_enc` all set to `NULL` by the sweeper, for every `backend_mode` value.
- [ ] AC-SDP-02: a `projects` row with `status = 'deleted'` and `deleted_at` LESS than the grace window
      in the past is left completely unchanged — no sensitive column is nulled before the window
      genuinely elapses.
- [ ] AC-SDP-03: a `projects` row whose sensitive columns are already `NULL` (already purged, or never
      populated) is left alone on subsequent cycles — no error, no redundant write.
- [ ] AC-SDP-04: a `projects` row with `status != 'deleted'` (e.g. `active`, `suspended`) is never
      purged, regardless of its `deleted_at`/`created_at` age.
- [ ] AC-SDP-05: the sweeper is guarded by the same `pg_try_advisory_lock`/`pg_advisory_unlock` pattern
      (`sweepers::advisory_lock_key`, reused unchanged) as `CapUsageRefresher`/`TransactionSweeper`, so
      multiple concurrent `embyr-server` instances never perform redundant or conflicting purge work in
      the same cycle.
- [ ] AC-SDP-06: the sweep interval and grace-window length are both configurable via environment
      variables, following the established `EMBYR_*_INTERVAL_SECS`/`EMBYR_*_RETENTION_DAYS` naming and
      wiring convention (`ServerConfig` field + `main.rs` `spawn(...)` call); the SHIPPED DEFAULT grace
      window is exactly 168 hours (7 days), matching the admin-UI's own unchanged promise text.
- [ ] AC-SDP-07 (regression guard): no `projects` row is ever hard-deleted by this feature, and no
      `sdk_api_keys` / `access_rule_history` / `write_access_rule_history` / `access_rule_pattern_
      history` row (or any other row referencing the purged project's `id`) is touched, deleted, or
      otherwise modified by this feature.

#### Outcome KPIs
- **Who**: Chris (Account Admin, trusting the delete-confirmation modal's own promise) and Sam Chen
  (Service Operator, answerable for what a "deleted" customer's credentials look like to an auditor).
- **Does what**: every soft-deleted project's sensitive encrypted credential columns are genuinely
  nulled once the documented grace window has elapsed — not retained indefinitely.
- **By how much**: from 0% of soft-deleted projects' credential columns ever purged today (100%
  retained forever, confirmed by this DISCUSS's own reading of `lifecycle.rs:181` and a repo-wide
  `deleted_at` grep showing no other reader) to 100% of `'deleted'` projects past the grace window
  having `NULL` `ecies_encrypted_dsn`/`backend_pg_dsn_enc`/`agent_tls_bundle_enc`.
- **Measured by**: AC-SDP-01/02's own direct system-DB column query, before and after a sweep cycle,
  for both a past-window and a within-window project.
- **Baseline**: 0% — confirmed directly by this DISCUSS's own reading of `lifecycle.rs:180-183` (the
  one and only write site for `deleted_at`, with no other code path in the repo ever reading it).

## Wave: DISCUSS / [REF] Out of Scope

- **A hard `DELETE FROM projects` row removal** — confirmed unsafe by direct FK-reference investigation
  (§ Reading Confirmation): conflicts with `access_rule_history`'s own explicit, prior-feature-locked
  append-only invariant and `sdk_api_keys`' own no-cascade FK. Named, locked as unsafe, not merely
  deferred — a future feature could revisit this only by first resolving those two invariants, which
  this feature does not attempt.
- **An actual "undelete"/restore mechanism consuming the grace window** — confirmed to not exist
  anywhere in this codebase today (§ Reading Confirmation, zero grep matches for `restore`/`undelete`),
  at either the project level (this finding's own scope) or the account level (`user-admin-ui`'s own
  AC-011-05, "Non-reversible from UI"). This feature does not build one; it only ensures the sweeper
  never purges BEFORE the window closes, which is the entire protection a hypothetical future restore
  feature would need. Named as a real, related, un-evidenced gap — not silently assumed solved.
- **Revoking `backend_secret_arn`/`backend_secret_gcp` in AWS/GCP Secrets Manager itself** — these are
  explicitly "a reference, NOT a credential" (§ Reading Confirmation, both migration files' own
  comments); the actual secret material lives outside this system. Not part of this finding's own
  stated harm.
- **Hard-deleting or otherwise touching `sdk_api_keys` rows beyond their existing `revoked_at`
  cascade-revoke** (already performed, unchanged, by `delete_project` itself) — these rows are not
  "credentials" in the sense this finding names (a revoked key's `key_hash` cannot authenticate
  anything); leaving them in place is also what keeps a hard row-delete unsafe (§ Reading Confirmation),
  so removing them is a separate, un-evidenced, and non-trivial follow-up, not this feature's concern.
- **Account-level (`accounts.deleted_at`) purge** — a structurally similar but textually distinct gap
  (`user-admin-ui`'s own AC-011-05, Danger Zone — Delete Account) not named by finding #6 and not
  investigated by this DISCUSS; a candidate follow-up feature if evidenced, not built here.
- **Prometheus metrics for this sweeper's own purge activity** (mirroring `TransactionSweeper`'s own
  `embyr_soft_delete_purge_sweeper_purged_total`-shaped counter) — a natural, low-cost addition DESIGN
  may include (§ Central Design Questions C), but not required by any AC above; JOB-12's own
  observability job was evaluated and found a weaker primary fit than JOB-10 (§ Reading Confirmation)
  precisely because this feature's own outcome does not depend on a new metrics surface existing.
- **Any code implementation** — this is DISCUSS only, per explicit task instruction; DESIGN performs the
  actual investigation and implementation planning.

---

## Wave: DESIGN / [REF] Reading Confirmation

✓ `crates/embyr-server/src/sweepers/mod.rs` (full, 49 lines) re-read directly. `advisory_lock_key(s: &str)
-> i64` (lines 18-27, FNV-1a) is the shared helper both existing sweepers reuse unchanged; `pub mod
cap_usage_refresher;` / `pub mod transaction_sweeper;` (lines 9-10) is the exact insertion point for a
third `pub mod soft_delete_purge_sweeper;` line.

✓ `crates/embyr-server/src/sweepers/cap_usage_refresher.rs` (full, 190 lines) re-read directly at DESIGN
depth. `spawn()` (lines 28-83) is the exact shape this feature's own `spawn()` reuses unchanged:
`tokio::spawn` → `tokio::time::interval(interval)` loop → per tick, `system_db.pool().acquire()` ONE
`PoolConnection` (session-affinity comment at lines 41-55, reused verbatim as this design's own rationale)
→ `pg_try_advisory_lock` on it → skip cycle silently if not `Some(true)` → `run_cycle(...)` → `pg_advisory_
unlock` on the SAME connection. Confirms this is the closer structural sibling per DISCUSS's own finding
(§ Reading Confirmation point 3) — this feature's own `spawn()` signature needs exactly `Arc<SystemDb>` +
`Duration` + one more scalar (the grace window), nothing else; no `LifecycleDeps`, no `CapStatusCache`
equivalent is needed here (this feature calls no other application service — the UPDATE is the entire
cycle body).

✓ `crates/embyr-server/src/sweepers/transaction_sweeper.rs` (full, 259 lines) re-read directly at DESIGN
depth, specifically the Slice 02 purge step (lines 227-253) — the closest existing precedent for "a
runtime-configurable retention window bound as a query parameter, not interpolated." Confirmed exactly:
`let cutoff = chrono::Utc::now() - chrono::Duration::days(retention_days); ... .bind(cutoff)...`, with its
own doc comment (lines 227-230) stating explicitly why bind (not interpolation) is used here specifically
— "the retention window IS a runtime bind parameter ... it has no sibling constant elsewhere to drift out
of sync with." This feature's own grace window is the same shape (a runtime config value with no sibling
compile-time constant to protect) — the identical `chrono::Utc::now() - chrono::Duration::days(n)` +
`.bind(cutoff)` pattern is reused unchanged, not invented. Also confirms `pub const LOCK_KEY_NAME: &str`
(line 52) and `pub async fn run_cycle(...)` (line 127, exposed specifically so acceptance tests can invoke
the cycle body directly without the interval/lock wrapper) as the naming/visibility convention this
feature's own module follows.

✓ `crates/embyr-server/src/config.rs` re-read directly at the exact insertion points: struct field block
(lines 64-123, `ServerConfig`), `from_env()`'s optional-vars-with-defaults block (lines 324-347,
specifically `cap_check_interval_secs` at 336-339 and `transaction_sweep_interval_secs`/`transaction_
retention_days` at 340-347 — the exact `std::env::var(...).ok().and_then(|v| v.parse::<T>().ok()).unwrap_
or(default)` idiom this feature's own two new fields reuse unchanged), and the `ServerConfig { ... }`
struct-literal construction (lines 349-367, where the two new fields must be threaded through).

✓ `crates/embyr-server/src/main.rs` re-read directly, lines 246-280 (the sweeper-spawn block) — confirmed
the exact surrounding shape and comment style both existing `spawn(...)` calls follow (a multi-line `//`
doc comment citing the originating ADR/finding directly above the `let _foo = ...::spawn(...)` binding,
underscore-prefixed because the `JoinHandle` is intentionally never awaited — fire-and-forget for process
lifetime, matching the OBS-05 pool-gauge task's own precedent cited in both existing comments). This
feature's own `spawn(...)` call is inserted directly after the `transaction_sweeper::spawn(...)` block
(after line 280), before the `── Step 11 ──` comment, matching that exact shape.

✓ `crates/embyr-server/src/admin/handlers/lifecycle.rs` lines 165-196 re-read directly, confirming (unchanged
from DISCUSS's own reading) that `delete_project` writes `status = 'deleted', deleted_at = now(), updated_
at = now()` and nothing else touches `ecies_encrypted_dsn`/`backend_pg_dsn_enc`/`agent_tls_bundle_enc`
after provisioning. Directly relevant to one DESIGN-owned decision below: whether this feature's own
`UPDATE` should also bump `projects.updated_at`. § Business Context's own locked outcome text states the
purge "leav[es] the row itself, its `id`/`status`/`deleted_at`/`account_id`/timestamps ... exactly where
they already are today" — read literally and applied here: **`updated_at` is NOT touched by this
feature's own `UPDATE`** (see § Design Decisions, D2).

✓ `crates/embyr-server/src/adapters/system_db.rs` line 305 (`pub fn pool(&self) -> &PgPool`) confirmed —
the exact accessor `sqlx::query(...).execute(system_db.pool())` binds to, identical to how both existing
sweepers already call it.

✓ `migrations/0001_initial_schema.sql` lines 2-11, `migrations/0004_project_deleted_at.sql` (full, 2
lines), `migrations/0005_agent_endpoint.sql` line 5, `migrations/0015_projects_admin_columns.sql` line 4
— all four re-read directly, confirming exact column names/types: `projects.status VARCHAR(20) NOT NULL
DEFAULT 'active'`, `projects.deleted_at TIMESTAMPTZ` (nullable), `projects.ecies_encrypted_dsn BYTEA`
(nullable), `projects.backend_pg_dsn_enc BYTEA` (nullable), `projects.agent_tls_bundle_enc BYTEA`
(nullable) — no naming drift from DISCUSS's own assumed names; DDL/SQL below uses these exact identifiers.

## Wave: DESIGN / [REF] Design Decisions

**D1 — Idempotency guard: reuse the column-null state itself, no new column.** DISCUSS's own Question C
opening recommendation is confirmed correct and locked: `WHERE ecies_encrypted_dsn IS NOT NULL OR
backend_pg_dsn_enc IS NOT NULL OR agent_tls_bundle_enc IS NOT NULL` is added to the `UPDATE`'s own `WHERE`
clause. An already-purged row (all three columns already `NULL`) fails this predicate and is not matched
— `rows_affected() == 0` for it, no error, no redundant write, satisfying AC-SDP-03 by construction, with
the identical "idempotent `WHERE` clause, not a separate marker column" shape both existing sweepers
already use (`set_project_status`'s `WHERE status IN (...)`; `TransactionSweeper`'s reclaim/purge `WHERE`
clauses — § Reading Confirmation, DISCUSS's own point 4). A `purged_at` marker column was considered and
rejected: it would require a new migration (this feature's own scope explicitly excludes one — § System
Constraints) to record a fact the three existing nullable columns already encode for free — nulled
columns ARE the "already purged" marker. Ponytail rung 2 (already-sufficient signal in the codebase):
reuse, don't add.

**D2 — `updated_at` is NOT touched by the purge `UPDATE`.** § Business Context's own locked outcome text
(re-confirmed above) states timestamps are left "exactly where they already are today." Bumping `updated_
at` on purge would be a plausible-sounding addition (many `UPDATE`s in this codebase do bump it) but is
explicitly excluded by DISCUSS's own locked business outcome, and adds no test-observable value AC-SDP-01
through 07 require — omitted.

**D3 — Config naming, types, and defaults**, following `config.rs`'s own established `std::env::var(...)
.ok().and_then(|v| v.parse::<T>().ok()).unwrap_or(default)` idiom (§ Reading Confirmation):
- `EMBYR_SOFT_DELETE_SWEEP_INTERVAL_SECS` → `soft_delete_sweep_interval_secs: u64`, **default `3600`
  (1 hour)**. Locks DISCUSS's own Question B recommendation. Justification: this sweeper protects a
  7-day (604,800s) window: an hourly tick means the worst-case delay past grace-window expiry is ≤1h —
  0.006% of the window itself, negligible relative to the window's own scale, and two orders of magnitude
  coarser than `TransactionSweeper`'s 300s (which protects a 60-second abandonment threshold, a
  fundamentally tighter latency budget this feature does not share). Coarser than hourly (e.g. daily,
  86400s) was considered and rejected only for being an arbitrary further step in the same direction with
  no evidenced benefit — hourly is already conservative enough that steady-state Postgres load from this
  sweeper (one `UPDATE` per hour against a `projects` table with normally zero-to-few eligible rows) is
  immaterial; a sub-hour interval was rejected as solving a latency problem that does not exist for a
  week-scale grace window.
- `EMBYR_SOFT_DELETE_GRACE_DAYS` → `soft_delete_grace_days: i64`, **default `7`** (locked, matching the
  admin-UI's own unchanged "168h" promise exactly — AC-SDP-06's own hard regression guard). Locks
  DISCUSS's own Question A recommendation: a days-typed value, matching `EMBYR_TRANSACTION_RETENTION_
  DAYS`'s own already-established shape (§ Reading Confirmation) rather than introducing a second,
  hours-typed convention for what is the same 168-hour duration expressed differently. `i64` (not `u64`,
  not `u32`) matches `transaction_retention_days`'s own exact field type — `chrono::Duration::days(i64)`
  takes an `i64` directly, avoiding a cast this design would otherwise need to invent.

**D4 — Advisory lock key: `"embyr_soft_delete_purge"`.** Distinct from `"embyr_cap_check"` and
`"embyr_transaction_sweep"` (§ Reading Confirmation), same `embyr_{verb}_{noun-or-domain}`-shaped
snake_case convention both existing keys already follow — this one names the sweeper's own effect
(purge) on its own domain (soft-deleted projects), mirroring `"embyr_transaction_sweep"`'s own
verb-first shape rather than `"embyr_cap_check"`'s noun-first shape (either convention is precedented;
verb-first was chosen because "purge" is the more specific, less ambiguous word for what this cycle
actually does, avoiding confusion with a hypothetical future "soft-delete" lock guarding something else
entirely, e.g. the delete operation itself).

**D5 — Module shape: `crates/embyr-server/src/sweepers/soft_delete_purge_sweeper.rs`.** Mirrors `Cap
UsageRefresher`'s exact shape (§ Reading Confirmation), not `TransactionSweeper`'s per-project connection
loop — confirmed correct by DISCUSS's own structural finding (point 3) and re-confirmed here: this
sweeper needs zero customer-database connection, zero DSN resolution, zero `AwsSecretFetcher`/
`GcpSecretFetcher` dependency. `spawn()`'s signature is `(system_db: Arc<SystemDb>, interval: Duration,
grace_days: i64) -> tokio::task::JoinHandle<()>` — three parameters, the minimum this cycle body needs,
one fewer even than `CapUsageRefresher`'s own four (no `CapStatusCache`/`LifecycleDeps` equivalent exists
for this feature — the `UPDATE` IS the entire cycle body, calling no other application service). `run_
cycle` is `pub async fn run_cycle(system_db: &Arc<SystemDb>, grace_days: i64)`, `pub` (not `pub(crate)`)
for the same reason `TransactionSweeper::run_cycle` is `pub` (§ Reading Confirmation) — DISTILL's
acceptance tests invoke the cycle body directly, without the interval/lock wrapper, for fast deterministic
assertions.

**D6 — Exact SQL** (the single cycle-body statement, no other query in this module):
```sql
UPDATE projects
SET ecies_encrypted_dsn = NULL,
    backend_pg_dsn_enc = NULL,
    agent_tls_bundle_enc = NULL
WHERE status = 'deleted'
  AND deleted_at < $1
  AND (ecies_encrypted_dsn IS NOT NULL
       OR backend_pg_dsn_enc IS NOT NULL
       OR agent_tls_bundle_enc IS NOT NULL)
```
`$1` is bound to `chrono::Utc::now() - chrono::Duration::days(grace_days)`, computed in Rust once per
cycle — the identical `TransactionSweeper` Slice-02 purge-step pattern (§ Reading Confirmation), not a
Postgres-side `interval '{n} days'` string interpolation (that shape is reserved, per `TransactionSweeper`'s
own doc comment, for a compile-time constant mirroring another file's exact literal — this feature's grace
window has no such sibling constant, so it is a bind parameter like `retention_days`, not an interpolated
literal like `ABANDONMENT_THRESHOLD_SECS`). Every `backend_mode` is matched (no `backend_mode` filter in
the `WHERE` clause) — locked by DISCUSS's own § System Constraints, satisfying AC-SDP-01's "for every
`backend_mode` value" and the `agent`-mode UAT scenario directly (the sixth Gherkin scenario).

**D7 — Observability.** Matches `TransactionSweeper`'s own established shape (§ Reading Confirmation):
`metrics::counter!("embyr_soft_delete_purge_sweeper_purged_total").increment(purged)` when `query_result.
rows_affected() > 0` (naming pattern: `embyr_{sweeper_module_name}_{action}_total`, identical construction
to `embyr_transaction_sweeper_reclaimed_total`/`_purged_total`). On query failure: `tracing::warn!(error =
%e, "SoftDeletePurgeSweeper: purge query failed")`, matching both existing sweepers' own failure-logging
convention exactly (never panics, never aborts the process — the interval loop retries next tick). One
addition beyond the two existing siblings' own minimum: `tracing::info!(rows_purged = purged, "SoftDelete
PurgeSweeper: purged sensitive credential columns for soft-deleted projects past grace window")` alongside
the counter increment when `purged > 0` — a cheap, low-cardinality structured log line (no per-project
detail, just a count) directly serving Sam Chen's own JOB-10/JOB-12 audit-evidence need (§ Persona & Job)
without inventing a new per-project logging surface neither existing sweeper has. This is the only place
this design goes beyond straight reuse of the existing pattern, and it costs nothing structurally (no new
dependency, no new query, no new column) — named explicitly, not silently added.

**D8 — Blast radius, confirmed minimal.** Four files change:
1. `crates/embyr-server/src/sweepers/soft_delete_purge_sweeper.rs` — new file (D5-D7 above).
2. `crates/embyr-server/src/sweepers/mod.rs` — one line added: `pub mod soft_delete_purge_sweeper;`
   (alongside the existing two, § Reading Confirmation's exact insertion point); doc comment at the top
   of the file gains one clause naming the third sweeper.
3. `crates/embyr-server/src/config.rs` — two new `ServerConfig` fields + doc comments (near lines 109-122),
   two new parse lines (near lines 340-347, identical idiom to the two neighboring fields), two new
   entries in the `ServerConfig { ... }` literal (near lines 349-367).
4. `crates/embyr-server/src/main.rs` — one new `spawn(...)` call block inserted after the existing
   `transaction_sweeper::spawn(...)` block (after line 280), before `── Step 11 ──`:
```rust
// soft-delete-purge-sweeper (Blocker finding #6, production-readiness-audit-2026-09-08.md):
// background purge of the 3 sensitive encrypted-credential columns on a
// `'deleted'` project's own row, once the configured grace window (default
// 168h/7 days, matching the admin-UI's own unchanged promise) has elapsed.
// SystemDb-only — no customer-database connection, no DSN resolution needed
// (target columns live on the projects row itself, unlike TransactionSweeper).
let _soft_delete_purge_sweeper = embyr_server::sweepers::soft_delete_purge_sweeper::spawn(
    Arc::clone(&system_db),
    std::time::Duration::from_secs(cfg.soft_delete_sweep_interval_secs),
    cfg.soft_delete_grace_days,
);
```
Zero changes to `embyr-core`, `embyr-pg-storage`, `embyr-admin-ui`, `embyr-proto`, or any admin HTTP
route — confirmed, matching § Scope Assessment's own PASS finding. No new external dependency (`chrono`,
`sqlx`, `metrics`, `tracing`, `tokio` all already workspace dependencies, all already used by the two
existing sweepers this module reuses unchanged).

## Wave: DESIGN / [REF] ADR

`docs/product/architecture/adr-073-soft-delete-purge-sweeper-idempotency-and-scope.md` (new) — records D1
(idempotency guard) and D5 (SystemDb-only scope vs. `TransactionSweeper`'s per-project shape) as the two
decisions with genuine rejected alternatives; D2-D4/D6-D7 are named there as consequences of D1/D5 rather
than separately debated, since none of them had a real second candidate to reject (config naming/interval
default follow established precedent directly, not a choice among competing designs).

## Wave: DESIGN / [REF] Regression Guards Carried Forward

- AC-SDP-01 through 07 (all seven, § Acceptance Criteria) require DISTILL-designed acceptance tests
  against a real running `SystemDb` — no existing test file in the repo exercises this sweeper (it does
  not exist yet); this is entirely new test surface, not a modification to an existing one.
- No existing test file requires updating as a regression guard — this feature adds a new background task
  with zero behavioral change to any existing code path (`delete_project`, `list_pg_reachable_projects`,
  `CapUsageRefresher`, `TransactionSweeper` are all untouched).
- Full workspace `cargo test` (pre-commit gate only, per root `CLAUDE.md`'s own token-discipline rule) is
  the final regression guard, run once before commit.

## Wave: DESIGN / Handoff Package

**Files requiring a change:**
1. `crates/embyr-server/src/sweepers/soft_delete_purge_sweeper.rs` — new file (§ Design Decisions D5-D7).
2. `crates/embyr-server/src/sweepers/mod.rs` — one new `pub mod` line + doc-comment clause.
3. `crates/embyr-server/src/config.rs` — two new `ServerConfig` fields (`soft_delete_sweep_interval_secs:
   u64` default 3600, `soft_delete_grace_days: i64` default 7), following the exact existing parse idiom.
4. `crates/embyr-server/src/main.rs` — one new `spawn(...)` call, inserted after the `transaction_sweeper`
   block (§ Design Decisions D8).

**Files confirmed to need NO change:** `embyr-core` (zero IO, zero new domain type needed — this feature
adds no pure logic beyond a `chrono::Duration` subtraction already inline in the cycle body; DISCUSS's own
"new PURE logic ... the eligibility predicate" — § System Constraints — is expressed directly in the SQL
`WHERE` clause plus one `chrono` subtraction, not a standalone `embyr-core` function, since there is no
second caller and no branching logic complex enough to warrant extraction); `embyr-pg-storage`;
`embyr-admin-ui`; `embyr-proto`; any admin HTTP route; any migration file (all four target columns already
exist, confirmed by direct read, § Reading Confirmation).

**Locked decisions for DISTILL/DELIVER:**
- Exact SQL: § Design Decisions D6.
- Idempotency guard: § Design Decisions D1 — column-null state itself, no new `purged_at` column.
- Config: `EMBYR_SOFT_DELETE_SWEEP_INTERVAL_SECS` (default 3600) / `EMBYR_SOFT_DELETE_GRACE_DAYS` (default
  7) — § Design Decisions D3.
- Advisory lock key: `"embyr_soft_delete_purge"` — § Design Decisions D4.
- Observability: `embyr_soft_delete_purge_sweeper_purged_total` counter + `tracing::warn!`/`tracing::
  info!` — § Design Decisions D7.
- `updated_at` is NOT touched by the purge `UPDATE` — § Design Decisions D2.

**New test scenarios DISTILL must design:** direct-`run_cycle` tests against a real `SystemDb` (mirroring
`TransactionSweeper`'s own `pub run_cycle` test-invocation pattern) for each of the six UAT Gherkin
scenarios: past-window purge (all `backend_mode`s), within-window no-op, already-purged no-op/no-error,
non-`'deleted'` status never touched, concurrent-instance advisory-lock mutual exclusion (two `SystemDb`
handles racing `pg_try_advisory_lock` on the same key), and the `agent`-mode `agent_tls_bundle_enc`-specific
scenario.

**External integrations**: none. This feature makes zero external API call of any kind — no annotation
for contract testing applies (SystemDb/Postgres is an internal system-of-record, not a third-party
service).
