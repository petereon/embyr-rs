# client-auth — Pre-DELIVER Fail-For-The-Right-Reason Gate

Per `nw-distill` § Pre-DELIVER fail-for-the-right-reason gate. Run once, DISTILL wave, 2026-08-17.

## Procedure

For each acceptance test file, the walking-skeleton (or first non-`#[ignore]`)
scenario was run against real Postgres testcontainers; remaining scenarios
were additionally run with `--ignored` as a spot-check. Classification:
`MISSING_FUNCTIONALITY` (panic inside the target RED scaffold, or an
assertion mismatch caused by that panic surfacing as a 500/connection error)
= correct RED. `IMPORT_ERROR`/`FIXTURE_BROKEN`/`SETUP_FAILURE` = would be a
BROKEN classification — none observed.

## Results

| File | Scenario | Result | Classification |
|---|---|---|---|
| `ca01_register_verification_credential.rs` | `alex_registers_trailmarks_verification_credential_and_no_raw_material_is_echoed_back` (WS) | FAIL | `MISSING_FUNCTIONALITY` — panics inside `register_client_identity_credential` at `client_identity.rs:138`, after the (already-correct) ownership/malformed-key checks pass |
| `ca01_...` | 5 remaining `#[ignore]`d scenarios | not run (ignored by design) | N/A — enable one at a time per DISTILL discipline |
| `ca02_signin_and_reject_invalid_tokens.rs` | `marias_valid_token_signs_in_and_her_subsequent_getdoc_call_succeeds` (WS) | FAIL | `MISSING_FUNCTIONALITY` — sign-in handler panics inside `sign_in_with_custom_token` scaffold, surfaces as HTTP 500 (assertion: left 500, right 200) |
| `ca02_...` | 7 remaining `#[ignore]`d scenarios | not run (ignored by design) | N/A |
| `ca03_rotate_verification_credential.rs` | 3 of 4 (`--ignored` spot-check) | FAIL | `MISSING_FUNCTIONALITY` — panics inside `rotate_client_identity_credential` scaffold |
| `ca03_...` | `rotation_without_a_valid_session_is_rejected` (AC-16-13) | PASS | Legitimate GREEN — `session_auth_middleware` (existing, reused code) already rejects the unauthenticated request before the handler is reached. Not a test bug; documented explicitly, kept enabled. |
| `ca04_standalone_verify_debug_check.rs` | 4 of 4 (`--ignored` spot-check) | FAIL | `MISSING_FUNCTIONALITY` — panics inside `verify_client_identity_credential` scaffold |
| `ca05_algorithm_confusion_defense.rs` | 1 of 1 (`--ignored` spot-check) | FAIL | `MISSING_FUNCTIONALITY` — sign-in handler panic surfaces as HTTP 500 (assertion: left 500, right 400) |
| `crates/embyr-core/src/client_identity/mod.rs` (layer-1 unit + PBT) | all 14 (not `#[ignore]`d — inner-loop tests, run directly per Mandate 7) | FAIL (14/14) | `MISSING_FUNCTIONALITY` — `verify_client_identity_token`/`credential_fingerprint` panic unconditionally; zero import/compile errors |
| `crates/embyr-server/src/grpc/handler.rs` (`client_identity_extension_tests`) | all 3 | PASS | Legitimate GREEN — pure metadata-extraction routing (`extract_client_identity_token`) is real, already-correct code per Mandate 7's "routing is not missing functionality" distinction; only the downstream verification computation is a RED scaffold |

**Zero scenarios in category 2 (test bug) or 3 (wrong-shape assertion).** All
FAIL results are genuine RED (implementation missing); the two PASS results
are legitimate GREEN-by-construction (reused, already-correct middleware/
routing code), not fixture-shape false positives — no observable in either
passing test asserts against fixture-provided output (Constraint 7,
No Fixture Theater).

## AC-16-08(a) — full 72-scenario `embyr-rs` regression run (mandatory, run not deferred)

Command (from workspace root):
```
cargo test -p embyr-server \
  --test us_01_configure_sdk --test us_02_write_document --test us_03_read_document \
  --test us_04_query_collection --test us_05_listen_realtime --test us_06_transactions \
  --test us_07_provision_project --test us_08_monitor_project --test us_09_suspend_project \
  --test us_10_aws_secrets --test us_11_gcp_secrets --test us_12_agent_backend \
  --test us_13_browser_transport --test us_14_rate_limiting --test walking_skeleton
```

Result: **69/72 passed unmodified. 3 failures, all in `us_12_agent_backend`
(`agent_exits_nonzero_when_db_dsn_missing`, `agent_starts_with_required_env_vars_and_logs_readiness`,
`connection_without_client_cert_fails_tls_handshake`), all `Os { code: 2,
kind: NotFound }` — the test's own `cargo build -p embyr-agent` subprocess
resolves a binary path this sandbox's shared `CARGO_TARGET_DIR` does not
match.**

**Confirmed pre-existing, not a regression**: reproduced identically via
`git stash` (removing every client-auth change) and re-running the same
command against the clean tree — same 3 failures, same error, same line
numbers. This is a pre-existing environment/target-dir issue in this
sandbox, unrelated to client-auth. Zero NEW failures introduced by this
feature's scaffolds.

Additionally ran the `admin_api_v2` suite (7 binaries, 93 scenarios total —
not part of "the 72" but the other consumer of `build_admin_router`, which
this feature extended with 3 new routes): **93/93 passed unmodified.**

## Additional verification performed this DISTILL run

- `cargo check -p embyr-core --tests`: clean.
- `cargo check -p embyr-server --lib`: clean.
- `cargo check -p embyr-server --bins`: clean (production `main.rs` composition root unaffected).
- `cargo check -p embyr-server --test client_auth_ca0{1..5}...`: clean.
- `cargo deny check bans`: **`bans ok`** — confirms `embyr-core`'s first-time
  `jsonwebtoken` dependency (DDD-CA-8) does not violate the IO-prohibition
  ban list (DESIGN handoff flag #5, resolved during DISTILL rather than
  deferred to DELIVER).

## Conclusion

Gate **PASSED**. Handoff to DELIVER is unblocked.
