# Mutation Testing Report — stripe-webhook-body-limit

**Tool**: cargo-mutants 27.0.0
**Scope**: `crates/embyr-server/src/admin/middleware/stripe_signature.rs`, `--in-diff` scoped to
the DELIVER commit diff only (`git diff 6b2d3c3 a104691 -- crates/embyr-server/src/admin/middleware/stripe_signature.rs`).

Discipline followed: confirmed via `ps -p 16582` → not running (process genuinely exited) and
`git diff crates/embyr-server/src/admin/middleware/stripe_signature.rs` → empty (no leftover
mutation corruption) before reading results. See `feedback_mutation_testing_docker_contention.md`.

## Command

```
cargo mutants -p embyr-server --in-place --timeout 240 --in-diff /tmp/pr07_diff.diff -- --test production_readiness --lib
```

No `--ignored` filter needed this time — unlike the sibling `stripe-webhook-secret-required` and
`rate-limiter-project-id-validation` features, DELIVER removed `#[ignore]` from all 4 of DISTILL's
`pr07_stripe_webhook_body_limit.rs` tests (single-story feature, no reason to keep them
subprocess-gated behind an opt-in flag), so cargo-mutants' default test invocation already
exercises them.

## Result: 7 mutants tested in 38m — 7 caught, 0 missed, 0 unviable

```
=== caught ===
crates/embyr-server/src/admin/middleware/stripe_signature.rs:28:55: replace * with +
crates/embyr-server/src/admin/middleware/stripe_signature.rs:28:55: replace * with /
crates/embyr-server/src/admin/middleware/stripe_signature.rs:28:48: replace * with +
crates/embyr-server/src/admin/middleware/stripe_signature.rs:28:48: replace * with /
crates/embyr-server/src/admin/middleware/stripe_signature.rs:45:5: replace stripe_signature_middleware -> Result<Response, StatusCode> with Ok(Default::default())
crates/embyr-server/src/admin/middleware/stripe_signature.rs:77:5: replace read_body_bounded -> Result<Bytes, StatusCode> with Ok(Default::default())
crates/embyr-server/src/admin/middleware/stripe_signature.rs:85:24: delete ! in read_body_bounded
```

## Interpretation

**Fully clean result — first 100%-caught mutation run of this session's 3 blocker fixes so far.**
Every mutant in the diff is caught, with no missed or unviable mutants to investigate:

- The two `5 * 1024 * 1024` ceiling-arithmetic mutants (line 28, both operand positions, both
  `+` and `/` substitutions) are caught by `pr07`'s boundary tests
  (`correctly_signed_webhook_comfortably_within_ceiling_still_succeeds` at ~1MiB and
  `oversized_body_rejected_with_413_and_bounded_memory_growth` at 50MiB) — a wrong ceiling value
  would either reject the legitimate 1MiB webhook or accept the 50MiB attack payload, and both
  are asserted.
- `stripe_signature_middleware -> Ok(Default::default())` (line 45) is caught because a
  short-circuited success would fail every negative-path assertion (missing signature → 401,
  oversized body → 413) across `pr06` and `pr07`.
- `read_body_bounded -> Ok(Default::default())` (line 77) is caught the same way — an empty
  `Bytes::default()` would fail signature verification against the real HMAC in the happy-path
  tests, and would never produce the 413 the oversized-body tests require.
- **`delete ! in read_body_bounded` (line 85: `if !too_large`)** is the single most
  security-critical mutant in this diff — it guards the "don't append bytes past the limit"
  invariant that keeps memory bounded. Deleting the `!` would make the buffer accumulate ALL
  bytes once past the limit was first hit, then discard nothing — defeating the entire point of
  this feature. Caught by `oversized_body_rejected_with_413_and_bounded_memory_growth`'s
  RSS-delta assertion, which is exactly the test DISCUSS insisted on (not just a status-code
  check) for precisely this reason.

No follow-up investigation needed — no missed, no unviable, nothing to accept or defer.
