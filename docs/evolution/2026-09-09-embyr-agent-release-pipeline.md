# Evolution: embyr-agent-release-pipeline

**Date:** 2026-09-09
**Feature:** `embyr-agent` (the customer-VPC privacy-boundary binary) now has a genuine CI
build/release path — a new `agent` job builds a static musl binary, verifies it against real
acceptance tests, uploads it as a GitHub Actions artifact, and packages it into a container image.
**Job:** JOB-13 (`production-deployment`) — reused, persona P2 Sam Chen, Riley Nakamura as
downstream beneficiary.
**ADRs:** ADR-074 (new) — agent release pipeline (musl target, dual CI job/artifact mechanism).

## This closes finding #8 from `docs/product/production-readiness-audit-2026-09-08.md` — the LAST of 8 Blockers

## Business Context

`embyr-agent` — the product's own "core privacy boundary" (it lets a customer's database
credentials stay inside their own VPC, never crossing to embyr SaaS) — had NO build/release path
anywhere: no Dockerfile, no CI target, nothing in `.github/workflows/ci.yml` referenced it at all.
`embyr-server` had a full multi-stage Dockerfile and a CI job building it on every push; the agent,
despite being architecturally more security-critical (ADR-001: its whole security model depends on
being a separate, minimal-surface, statically-linked binary), simply could not be distributed to a
customer.

## Key Decisions (ADR-074)

| Decision | Verdict |
|---|---|
| Target | `x86_64-unknown-linux-musl` only — arm64 explicitly deferred (YAGNI, no persona names it) |
| CI job shape | New `agent` job, deliberately no `needs:` on `test` — a compile break must show as a genuinely FAILED job, not silently SKIPPED (which `needs:[test]` would cause, since `test` already compiles the whole workspace) |
| Distribution mechanism | Dual: primary = `actions/upload-artifact@v4` (zero new infra, matches ADR-001's literal "statically-linked binary" language); secondary = new `crates/embyr-agent/Dockerfile` (`FROM scratch`, wraps the pre-built binary, no compilation inside) for operators deploying via Kubernetes (`agent-deployment.yaml`'s own `kubectl apply` step implies a container image, not a bare binary) |
| Verification | Reuses `tests/acceptance/embyr_agent/us_a06_lifecycle.rs`'s 3 pre-existing (but `#[ignore]`d) tests against the REAL compiled musl artifact in the same CI step — not a separate glibc build, not new smoke-test infrastructure |

## Steps Completed

Full nWave subagent pipeline (DISCUSS=`nw-product-owner`, DESIGN=`nw-solution-architect` with peer
review, DISTILL=`nw-acceptance-designer`, DELIVER=`nw-software-crafter`), plus a second DISCUSS
dispatch after the first hit a session rate limit mid-flight:

1. **DISCUSS**: reused JOB-13 (rejected JOB-04/07's Riley-primary framing after directly reading
   `agent-deployment.yaml` — that journey assumes a deployable artifact already exists, never
   covers how it comes to exist). Confirmed the musl-static-binary architecture is locked
   (ADR-001) but genuinely unbuilt anywhere (no `.cargo/config.toml`, no musl target in CI). DoR
   9/9.
2. **DESIGN**: peer-reviewed, approved iteration 1. Locked the no-`needs:` CI shape (a genuinely
   subtle, correct call — a naive mirror of the existing `docker` job's own `needs:[test]` would
   have silently hidden agent compile breaks as "skipped" rather than "failed"), the dual
   artifact/Dockerfile mechanism, and the reuse of existing (currently-ignored) acceptance tests
   as the verification vehicle.
3. **DISTILL**: did unusually thorough empirical pre-validation before DELIVER touched any
   production file — ran DESIGN's own exact build recipe in a real container, found and validated
   the fix for a genuine blocker (`.dockerignore` excluding `target/`, breaking the Dockerfile's
   own `COPY`), and traced the 3 `#[ignore]`d tests to a stale, unrelated reason, confirming all 3
   already pass today.
4. **DELIVER**: implemented exactly DISTILL's pre-validated recipe. Independently re-ran every
   verification step fresh (not trusting DISTILL's prior run) — musl build, Docker build, 3
   unignored tests (twice), full `embyr_agent` suite (37/37).
5. **Orchestrator's own independent verification found ONE MORE real gap**, not caught by either
   subagent: DELIVER's own report said all 3 tests were unskipped and verified passing (true —
   they do pass when run directly), but reading the actual CI job's own YAML showed its test
   invocation only named 2 of the 3 by filter, omitting the fail-closed-on-unreachable-storage
   test. The underlying test coverage was correct; the CI WIRING had a gap between "the tests
   pass" and "the tests actually run in the pipeline." Fixed directly, reverified (2 more
   independent runs, full suite unaffected).
6. **Full-workspace regression**: clean (1 unrelated `PortNotExposed`/`cargo-sweep`-class flake,
   confirmed transient via isolated rerun — the same already-documented interference hit 3+ times
   this session).
7. **QUALITY_GATE**: no `cargo-mutants` applicability (CI YAML + Dockerfile, not application code)
   — documented the actual discipline applied instead: 3 independent verification passes
   (DISTILL, DELIVER, orchestrator), the one the orchestrator's own pass caught that the other two
   didn't.

## Lessons Learned

1. **"The tests pass when I run them" and "the tests actually run in the pipeline" are two
   different claims — verifying the first doesn't prove the second.** DELIVER's own report was
   entirely honest and its own verification was real, but it verified test correctness (running
   the 3 tests directly via `cargo test`) rather than pipeline wiring correctness (reading whether
   the CI job's own invocation actually includes all 3). This is the CI-infrastructure analogue of
   a mutation-testing miss — the coverage existed, the wiring to exercise it in the real pipeline
   had a gap. For any future CI/pipeline feature, an explicit "read the actual job YAML and confirm
   every intended test name literally appears in its invocation" check belongs in QUALITY_GATE,
   not just re-running the tests standalone.
2. **A naive "mirror the sibling job's own `needs:` shape" instinct can silently break failure
   visibility.** DESIGN's own correct call — no `needs:[test]` for the new `agent` job — required
   understanding a GitHub Actions semantic (a job whose dependency FAILS shows as SKIPPED, not
   FAILED) that isn't obvious from surface-level YAML-copying. Worth remembering for any future new
   CI job in this repo: default to independence unless there's a genuine reason to gate on another
   job's success.
3. **This closes the entire 8-Blocker production-readiness-audit arc**, mirroring the earlier
   8-item `known-gaps.md` arc closed earlier this session — both fully closed via the same nWave
   subagent pipeline discipline, with the orchestrator's own independent re-verification catching
   real, distinct gaps in 4 of the 8 features (realtime-listener-reconnect's poisoned-connection
   hang, composite-index-real-creation's asymmetric-mutation gap, soft-delete-purge-sweeper's
   lock-inversion gap, and this feature's CI-wiring gap) — none of which the individual DELIVER
   subagents' own testing caught on their own.

## Key Files

- `.github/workflows/ci.yml` — new `agent` job.
- `crates/embyr-agent/Dockerfile` (new).
- `.dockerignore` — 3 negation lines un-excluding the musl release binary path.
- `tests/acceptance/embyr_agent/us_a06_lifecycle.rs` — 3 `#[ignore]` attributes removed.
- `docs/product/architecture/adr-074-agent-release-pipeline.md`.
- `docs/feature/embyr-agent-release-pipeline/deliver/mutation/mutation-report.md` — full account.

## Follow-Up Work

**All 8 Blockers from `docs/product/production-readiness-audit-2026-09-08.md` are now CLOSED.**
Findings #9-#20+ (High/Medium/Low severity, plus the 9-item bloat list) remain, documented in the
audit doc's own table — not blockers for launch, but real findings worth working through next if
continuing down the list. Findings #9-#14 (already listed in the audit table, all "Not started")
are High-severity and the natural next targets: client-controlled OCC precondition panic (#9),
raw-driver-error leakage (#10), embyr-agent's own weaker field-path validator (#11), inline
Argon2id blocking the async runtime + no rate limiting on admin signin (#12), a timing-oracle
account-email leak (#13), and pre-auth 3-round-trip amplification against the shared system DB
(#14). Finding #19 (release-versioning/changelog infra, High) was explicitly named as this
feature's own out-of-scope follow-up during DISCUSS.
