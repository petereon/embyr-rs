# QUALITY_GATE Report — embyr-agent-release-pipeline

**Note on format**: this feature's own diff is CI YAML + a Dockerfile, not application code —
`cargo-mutants` doesn't meaningfully apply (there's no Rust logic to mutate; the "logic" is
GitHub Actions job structure and shell commands). This report follows the same directory/naming
convention as every sibling feature's own `deliver/mutation/mutation-report.md` for consistency,
but documents the actual QUALITY_GATE discipline applied instead: multiple independent,
empirical verification passes of the real artifacts (the built binary, the container image, the
actual test suite), which is the correct analogue for infrastructure-as-code.

## Verification passes (3 independent, not counting one another)

1. **DISTILL** (`nw-acceptance-designer`): ran DESIGN's exact toolchain/build recipe inside a real
   `ubuntu:24.04 --platform linux/amd64` container (macOS host can't natively cross-compile to
   musl). Confirmed a genuine static-pie ELF musl binary, 11.4 MiB. Found and fixed a real blocker
   — the root `.dockerignore` excludes `target/`, breaking DESIGN's own `docker build` command —
   reproduced the failure, validated a 3-line fix. Traced `us_a06_lifecycle.rs`'s 3 `#[ignore]`d
   tests to a stale, unrelated ignore-reason and confirmed all 3 pass today, unmodified.
2. **DELIVER** (`nw-software-crafter`): independently re-ran every one of DISTILL's own
   verification steps AFTER making its own file changes (not trusting DISTILL's prior run) — fresh
   musl build (exit 0, confirmed via `file` as `ELF 64-bit ... static-pie linked`, 11,963,128
   bytes), fresh Docker build + run (hit real `AgentConfig::from_env()` diagnostics, confirming a
   genuine, non-empty binary), 3 unignored tests run twice (3/3 both times), full `embyr_agent`
   acceptance suite (37/37, 10 pre-existing unrelated ignores).
3. **Orchestrator** (this pass): independently re-verified YAML syntax and the `agent` job's own
   `needs:` field via a fresh Ruby YAML parse, and independently re-ran the 3 tests twice plus the
   full `embyr_agent` suite once more.

## Genuine gap found during the orchestrator's own independent verification

**Not caught by DISTILL or DELIVER's own verification, because both ran the 3 tests directly via
`cargo test`, not through the actual CI job's own invocation.** Reading the `agent` CI job's own
`Build + verify release musl binary` step revealed it named only 2 of the 3 unignored test
functions in its `--` filter (`agent_logs_storage_readiness_before_accepting_connections`,
`storage_credential_never_appears_in_agent_logs`) — omitting
`agent_exits_without_binding_port_when_storage_unreachable`, the fail-closed-on-unreachable-storage
test. DELIVER's own report said "all 3 previously-ignored tests pass... run at least twice," which
was TRUE (they DO all pass when run directly), but the actual CI wiring only exercised 2 of them —
a real, if narrow, gap between "the tests pass" and "the tests actually run in CI."

This is the CI-infrastructure analogue of a mutation-testing miss: the underlying test coverage
existed and was correct, but the WIRING that would make it run in the actual pipeline had a gap.
Fixed by adding the third test name to the job's own filter. Reverified: 3/3 pass together (2
independent runs), full suite (37/37) unaffected.

## Final state

- `.github/workflows/ci.yml`'s new `agent` job: valid YAML, no `needs:` edge (confirmed via
  parse), runs all 3 (not 2) of the relevant acceptance tests against the real compiled musl
  artifact.
- `crates/embyr-agent/Dockerfile`: builds successfully after the `.dockerignore` fix, produces a
  genuine, runnable (though credential-incomplete without real env vars, as expected) image.
- Full workspace regression: clean (1 unrelated `admin_api_v2_b06_oidc_billing` failure traced to
  the same already-documented external `cargo-sweep` interference class hit repeatedly this
  session — confirmed via `cargo-sweep.log` and an isolated 15/15 rerun).

This closes the LAST of the 8 Blockers from `docs/product/production-readiness-audit-2026-09-08.md`.
