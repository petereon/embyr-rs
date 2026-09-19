# request-body-size-limits (finding #24)

**Status:** FINALIZED 2026-09-19
**Closes:** Medium/Reliability finding #24 from `docs/product/production-readiness-audit-2026-09-08.md`

## Business Context

Every listener surface (gRPC :8080, gRPC-Web/REST :8081, admin :9090) was
relying on tonic's/axum's implicit library-default body-size ceilings
rather than a deliberate, tested value. That's a silent reliability risk:
an oversized payload's actual fate (accepted? OOM? clean rejection?) was
never verified. This feature makes every ceiling explicit, backed by one
acceptance test covering all four surfaces (three new ceilings + the
pre-existing Stripe webhook ceiling as a regression guard).

## Key Decisions

| Surface | Ceiling | Rationale |
|---|---|---|
| gRPC native (:8080) + gRPC-Web (:8081) | 10 MiB | `FirestoreServer::max_decoding_message_size` — raised from tonic's implicit 4 MiB to match Firestore-protocol parity (real Firestore documents can approach 1 MiB × batch fan-out); shared crate-root constant so the two surfaces cannot drift apart |
| REST (:8081 accounts bridge) | 2 MiB | `axum::extract::DefaultBodyLimit` — matches axum's own implicit default exactly; zero behavior change, now explicit and tested |
| Admin (:9090) | 1 MiB | `DefaultBodyLimit`, tightened from axum's implicit 2 MiB default — reflects real admin payload shape (project/index/secret-rotation config, all sub-KB to low-KB in practice); single choke point in `build_admin_router` so it covers `main.rs` and every test-server wrapper; does not govern `/admin/v1/webhooks/stripe` (that route reads the raw body directly via `to_bytes`, bypassing the `DefaultBodyLimit` extension — untouched, still governed by its own independent 5 MiB ceiling, ADR-070) |
| — | ADR-081 written | Documents all three ceilings as protocol/payload-shape security decisions, not operator-tunable knobs — no env var |

## Lessons

- **Test-client-vs-production-code decode-limit confusion (DELIVER's own root cause, correctly diagnosed):** early manual verification nearly mistook a *test HTTP client's* own body-size behavior for the *server's* enforcement. DELIVER traced it back to the real production knobs (`max_decoding_message_size`, `DefaultBodyLimit`) before writing the acceptance test, avoiding a test that would have passed for the wrong reason.
- **The `-C`/`--cargo-arg` build-scoping lesson (from `pool-sizing-and-limits`) needs to be applied proactively, not rediscovered each time.** This session's QUALITY_GATE still started down the naive path once (a stale duplicate agent instance briefly ran cargo-mutants unscoped against `embyr-server`'s ~196 `[[test]]` targets before being caught and killed) before the `-C --test -C production_readiness` fix — applied to *both* the build and test cargo invocations — cut it to a clean 6-minute, 17-mutant run. Also hit a second, smaller syntax trap worth recording: cargo-mutants requires a *double* `--` before libtest flags like `--test-threads` (`-- -- --test-threads=1 <filter>`) when `-C` args are also in play, or cargo rejects them as unrecognized `cargo test` options.

## Mutation Testing

17 mutants: 13 caught, 4 unviable, 0 missed (1 initially-missed off-by-one
arithmetic mutant on the admin ceiling's degenerate `1 * x` operand was
closed by tightening the acceptance test's boundary from a ±1 KiB margin to
a byte-exact one). Full detail:
`docs/feature/request-body-size-limits/deliver/mutation/mutation-report.md`

## Key Files

- `crates/embyr-server/src/lib.rs` — gRPC + REST ceiling constants and wiring
- `crates/embyr-server/src/rest/grpc_web.rs` — gRPC-Web ceiling (shares the gRPC constant)
- `crates/embyr-server/src/admin/router.rs` — admin ceiling, single choke point
- `tests/production_readiness/acceptance/pr14_request_body_size_limits.rs` — the one acceptance test covering all four surfaces
- `docs/product/architecture/adr-081-request-body-size-limits.md`
- `docs/feature/request-body-size-limits/feature-delta.md`

## Follow-Up

Finding #25 (structured JSON logging) is next in the audit's Medium queue.
