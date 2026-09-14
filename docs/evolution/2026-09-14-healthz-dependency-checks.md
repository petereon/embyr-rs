# Evolution: healthz-dependency-checks

**Date:** 2026-09-14
**Feature:** `/healthz` is redefined as a real readiness check (reachability of the shared
system Postgres pool), and a new `/livez` endpoint is added as a pure liveness check
(zero I/O, never depends on Postgres) — so an orchestrator can distinguish "stop routing
traffic here" from "restart this pod," and a DB outage can no longer trigger a
restart-storm.
**ADR:** `docs/product/architecture/adr-078-liveness-readiness-split.md`

## This closes finding #15 from `docs/product/production-readiness-audit-2026-09-08.md`

## Business Context

`/healthz` was a hardcoded `200 OK`, mounted identically on :8081 and :9090, with zero
dependency checks. If the shared system Postgres pool died mid-flight, the orchestrator
had no signal to stop routing traffic to the affected pod — it would keep reporting
healthy indefinitely. Confirmed independently by 2 separate audit agents (DevOps + SRE).

## Key Decisions

| Decision | Verdict |
|---|---|
| Endpoint split (OQ-HDC-01) | `/healthz` keeps its EXISTING meaning (readiness — 3 pre-existing regression tests already assume this) and gains a real Postgres check; a NEW `/livez` is added for liveness. This inverts the conventional Kubernetes-docs naming (`/healthz`=liveness there) but preserves this repo's own established semantics — documented in ADR-078 with 3 rejected alternatives. |
| Liveness must never touch Postgres | Hard constraint from DISCUSS — wiring a DB dependency into liveness means a DB outage triggers an orchestrator to restart every pod, which can't fix the outage and can worsen a reconnection storm. `livez_handler` is a literal zero-I/O copy of the old hardcoded handler. |
| Readiness check | Reuses the EXISTING, already-tested `SystemDb::probe()` (`SELECT 1` + schema check) — no new probe logic, scoped to the shared system Postgres pool only, not per-tenant customer DBs (unbounded cost, wrong blast radius). |
| Caching (OQ-HDC-02) | None — fresh probe every request; caching risked delaying the fail/recover-within-one-probe-interval requirement for no measurable benefit. |
| Timeout (OQ-HDC-03) | Explicit 3s `tokio::time::timeout` wrapped around the `probe()` call at the `/healthz` handler only — not inside `SystemDb::probe()` itself, to avoid changing the startup-gate caller's own tested behavior. |
| New leak-surface finding | `SystemDb::probe()`'s `Err` variants embed raw driver/schema text; ADR-075's sanitization sweep only covered `tonic::Status` sites, never this HTTP/axum boundary. Added as AC-HDC-11: the 503 body is a fixed generic string, never the raw error. |
| ADR | New ADR-078 — this establishes a repo-wide endpoint→semantics operational contract that future K8s manifest work (finding #19) depends on; getting it wrong would silently reopen the restart-storm risk. Warrants a real ADR, unlike smaller findings this session that were pure call-order fixes. |

## Steps Completed

Full nWave subagent pipeline (DISCUSS=`nw-product-owner`, DESIGN=`nw-solution-architect` with peer review, DISTILL=`nw-acceptance-designer` with peer review, DELIVER=`nw-software-crafter`):

1. **DISCUSS**: read `healthz.rs`, both mount points, `SystemDb::probe()`, and 3 existing regression tests. Identified the restart-storm anti-pattern risk as a hard constraint before DESIGN could get it wrong. 2 stories, DoR 9/9 both. Extended `JOB-13` in `docs/product/jobs.yaml` with a dated NOTE (same job/persona, not a new job).
2. **DESIGN**: resolved all 3 open questions, wrote ADR-078, found and corrected a citation error in DISCUSS's own line numbers (`main.rs:977` doesn't exist — it's `lib.rs:977`), flagged the `Arc::clone`-before-move ordering hazard at that call site, and found the new AC-HDC-11 leak-surface gap. Peer review: approved, 0 critical/high, first iteration.
3. **DISTILL**: 4 scenarios (1 walking skeleton + 3 focused), simulating Postgres-unreachable via real testcontainers `stop()`/`start()` with a fixed host-port mapping (reused from `pr08_realtime_listener_reconnect.rs`'s own pattern). Explicitly skipped a dedicated 3s-timeout test as unjustifiably fragile (would need TCP blackholing via a new dependency) — documented as a deliberate, evidence-based decision, not a gap. Caught and fixed its own false-GREEN mid-pass (a per-tenant-DB-down test that needed a system-DB-outage precondition chained in first). Peer review: approved, 0 blockers, 9.2/10 average.
4. **DELIVER**: implemented exactly as designed (commit `b07ab57`). One test-infra fix (not an assertion change): a reqwest client helper timeout bumped 2s→6s after discovering a stopped testcontainers Postgres can leave `connect()` blackholed rather than fast-failing, making the 503 transition unobservable within 2s regardless of server correctness — consistent with a finding already documented in `pr08`. All 4 pr10 scenarios + all 3 pre-existing regression tests green (19/19 combined run before scoping down for later steps).
5. **Independent verification**: fresh subagent re-confirmed all 6 code-level claims (especially `livez_handler`'s zero-I/O body and the `Arc::clone`-before-move ordering at all 3 wiring sites) and re-ran the 7 relevant tests clean.
6. **QUALITY_GATE**: 7 mutants in scope, 2 viable (both caught), 5 unviable (compiler-rejected candidates, not coverage gaps — verified via log inspection). `livez_handler` itself produced zero mutants (single unconditional return, nothing to mutate) — correctness there is proven by the structural AC-HDC-02 test instead. Commit `041ccbd`.

## Lessons Learned

1. **This is the first High finding this session requiring a genuinely new ADR** (ADR-078) rather than a call-order/reuse fix — the distinguishing signal was that it establishes an operational CONTRACT (endpoint→semantics mapping) that future infrastructure work depends on, not just an internal code-path fix.
2. **A structural/inverted naming decision can be correct even when it contradicts the "obvious" external convention** — Kubernetes docs conventionally pair `/healthz`=liveness, but this repo's own 3 pre-existing regression tests already gave `/healthz` a different meaning (readiness). Preserving existing, tested behavior won over matching an external naming convention, and the deliberate inversion is documented rather than silently done.
3. **A stopped testcontainer can leave `connect()` blackholed rather than fast-failing** — DELIVER's own reqwest client timeout (2s) raced the server's own mandated probe timeout (3s), producing flaky-looking failures unrelated to server correctness. Fixed by widening the TEST's client timeout, not the server's behavior — the server was already correct.
4. Continues this session's now well-established discipline of preferring a structural/source-scan test (AC-HDC-02: parse `healthz.rs`, assert `livez_handler`'s body excludes DB-related identifiers) over a fragile timing-based one, when proving "this code path never does X" is the actual requirement.

## Key Files

- `crates/embyr-server/src/grpc/healthz.rs` — `livez_handler` (new), `healthz_handler` (rewritten).
- `crates/embyr-server/src/lib.rs` — `spawn_all_servers`, `start_test_server_with_tls` wiring.
- `crates/embyr-server/src/main.rs` — admin router (:9090) wiring.
- `tests/production_readiness/acceptance/pr10_healthz_dependency_checks.rs` (new, 4 scenarios).
- `docs/product/architecture/adr-078-liveness-readiness-split.md`.
- `docs/feature/healthz-dependency-checks/deliver/mutation/mutation-report.md`.
- `docs/product/jobs.yaml` — `JOB-13` extended with a dated NOTE (same job, not new).

## Follow-Up Work

- Finding #16+ (High) remain — next in audit order.
- Finding #19 (K8s manifests/Helm) can now correctly reference `/healthz` (readiness) vs `/livez` (liveness) once that work starts — ADR-078 is the contract to follow.
- Finding #29 (Grafana/runbook) — not touched, still open.
