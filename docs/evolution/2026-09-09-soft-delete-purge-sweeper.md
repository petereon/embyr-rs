# Evolution: soft-delete-purge-sweeper

**Date:** 2026-09-09
**Feature:** Soft-deleted projects' encrypted credentials (ECIES DSN, backend DSN, agent TLS
bundle) are now genuinely purged 168h/7 days after deletion, matching the admin-UI's own
long-standing but previously-fake promise.
**Job:** JOB-10 (`account-admin`) — reused, persona Chris.
**ADRs:** ADR-073 (new) — idempotency guard + sweeper scope decisions.

## This closes finding #6 from `docs/product/production-readiness-audit-2026-09-08.md`

## Business Context

`delete_project` set `status='deleted', deleted_at=now()` — nothing else in the codebase ever read
`deleted_at`. Soft-deleted projects, including their ECIES-encrypted customer database
credentials, persisted forever, directly contradicting the admin-UI's own explicit "Soft-delete
with a 168h grace window before data purge" promise text.

## Key Decisions (ADR-073)

| Decision | Verdict |
|---|---|
| 1 — Purge mechanism | Null 3 sensitive columns (`ecies_encrypted_dsn`, `backend_pg_dsn_enc`, `agent_tls_bundle_enc`) on the row, never a hard `DELETE` — 34 `REFERENCES projects` matches with no `ON DELETE CASCADE`, and `access_rule_history` is an explicit append-only-invariant table that would FK-violate |
| 2 — Idempotency | The `WHERE ... IS NOT NULL` guard on the purge `UPDATE` is its own "already purged" marker — no new column |
| 3 — Sweeper shape | Mirror `cap_usage_refresher.rs` (SystemDb-only, no customer-DB connection), not `transaction_sweeper.rs`'s heavier per-project loop — this feature never needs to touch a customer database, the target columns live on the `projects` row itself |
| 4 — Grace window | 168h/7 days default, matching the admin-UI's unchanged promise text — confirmed no undelete/restore route exists anywhere, so the window protects nothing today but the promise itself is unchanged |

## Steps Completed

Full nWave subagent pipeline (DISCUSS=`nw-product-owner`, DESIGN=`nw-solution-architect` with peer
review, DISTILL=`nw-acceptance-designer`, DELIVER=`nw-software-crafter`):

1. **DISCUSS**: reused JOB-10 (not JOB-13/12, both explicitly ruled out — this feature's outcome is
   credential-lifecycle correctness, not deployment/observability). Grepped all 34
   `REFERENCES projects` matches to determine hard-DELETE was unsafe. Confirmed no undelete route
   exists. DoR 9/9 passed, 0 critical/high on peer review.
2. **DESIGN**: peer-reviewed, 0 critical/high/medium. Locked the exact SQL, config field names/
   defaults, advisory-lock key, and observability additions, correctly identifying this sweeper as
   structurally closer to `cap_usage_refresher.rs` than `transaction_sweeper.rs`.
3. **A minor incident during DISTILL**: a subagent accidentally ran `cargo fmt -p embyr-server`,
   reformatting the whole crate. Recovered responsibly (stash, not discard); the orchestrator
   independently investigated the fallout and confirmed all 4 files with pre-existing uncommitted
   diffs were formatting-only — no semantic work lost. Documented as a new operational lesson.
4. **DISTILL**: wrote 7 acceptance scenarios, confirmed correct RED (exactly one diagnostic — the
   module doesn't exist yet — no cascading errors, meaning every SQL/type/counter-reading line
   already compiled cleanly).
5. **DELIVER**: implemented per ADR-073 exactly. Found and fixed a real test-fixture bug (not
   production code): a missing `::int` cast on a `make_interval` bind parameter. 7/7 scenarios
   green across repeated runs; sibling sweeper regression guards unmodified and green.
6. **Orchestrator's full-workspace regression**: hit real external interference — a background
   `cargo-sweep` process escalated to a full `cargo clean` (125GB, 413K files) mid-run, corrupting
   in-flight build artifacts. Confirmed via `cargo-sweep.log` this was environmental, not a code
   issue, and re-ran cleanly from scratch once the sweep genuinely finished. One remaining failure
   (`drl_b12_postgres_rate_limit`) confirmed as the already-documented, pre-existing flake from
   earlier in this session via isolated rerun.
7. **QUALITY_GATE**: scoped mutation run, 4 caught / 5 unviable / 2 missed. **Investigated both
   misses empirically, not assumed.** The lock-decision-inversion mutant (`!=`→`==`) was manually
   re-applied with temporary debug tracing added — confirmed it only "passed" the acceptance test
   by accident (a connection-pool-churn artifact: the mutated code skips the unlock call, leaking
   the lock to a later tick's different pooled connection, which then fails to acquire it and
   stumbles into running via the inverted branch) — not a real guarantee, and plausibly exploitable
   in production with a larger, less self-colliding connection pool. Fixed via a pure
   `should_run_cycle()` extraction with a direct 3-case unit test. The second miss (`purged > 0`
   → `>= 0`) was confirmed genuinely cosmetic (u64 tautology, counter-value-identical either way,
   only an extra harmless log line) — documented, not fixed.

## Lessons Learned

1. **A mutant that "survives" an acceptance test can still be a real gap even when it appears to
   pass by a plausible-looking mechanism — verify empirically (manually apply the mutation, add
   debug tracing, actually run it) rather than assuming survival means safety.** The lock-inversion
   mutant passed the test, but only via an accidental side effect of small-connection-pool churn in
   the test environment specifically — a different environment (production's larger pool) could
   see the same inverted code genuinely fail to ever run the sweeper. This is the third feature
   this session (after realtime-listener-reconnect and composite-index-real-creation) where a
   mutation-testing finding required real empirical investigation, not code-reading alone, to
   correctly classify.
2. **`cargo fmt -p <crate>`/bare `cargo fmt` reformats the WHOLE crate, not just the file you're
   editing** — a genuinely new operational hazard this session, now documented
   (`feedback_cargo_fmt_package_blast_radius.md`). The recovering subagent's own response (stash,
   not discard) was correct; the orchestrator's own follow-up investigation (checking the stash's
   actual diff content per file, not just assuming "clean now" means "nothing lost") is what
   confirmed the incident was low-risk.
3. **Not every mutation-testing miss is worth fixing — a miss can be genuinely cosmetic
   (semantically equivalent for all practical purposes) and the responsible move is to document
   that reasoning explicitly, not force a fix for its own sake.** The `purged >= 0` tautology
   miss has zero observable effect on the counter's own value; chasing 100% mutant-kill on a
   provably-equivalent mutation would be effort spent for no real coverage gain.

## Key Files

- `crates/embyr-server/src/sweepers/soft_delete_purge_sweeper.rs` (new) — the sweeper,
  `should_run_cycle()` pure decision function (QUALITY_GATE fix).
- `crates/embyr-server/src/sweepers/mod.rs`, `config.rs`, `main.rs` — wiring.
- `tests/soft_delete_purge_sweeper/acceptance/us01_purge_credentials_after_grace_window.rs` (new)
  — 7 scenarios covering AC-SDP-01 through 07.
- `docs/product/architecture/adr-073-soft-delete-purge-sweeper-idempotency-and-scope.md`.
- `docs/feature/soft-delete-purge-sweeper/deliver/mutation/mutation-report.md` — full account.

## Follow-Up Work

Findings #7-#8 from the same audit remain: hardcoded-None aws/gcp secret fetchers; no build/
release path for embyr-agent. Note: finding #7 may directly relate to the `resolve_aws_secret_dsn`
gap flagged as follow-up in `composite-index-real-creation`'s own evolution doc.
