# Mutation Testing Report — wire-secret-fetchers

**Tool**: cargo-mutants
**Scope**: `--in-diff` scoped to the DELIVER commit diff (`git diff a73a511 3c6e49f -- crates/embyr-server/src/main.rs crates/embyr-server/src/config.rs`).

Discipline followed: confirmed via `ps -p 28384` → not running (process genuinely exited) before
reading results.

## Command

```
cargo mutants -p embyr-server --in-place --timeout 300 --in-diff /tmp/wsf_mut_diff.diff -- \
  --test production_readiness pr09 -- --test-threads=1 --include-ignored \
  --skip gcp_secret_project_wiring_reached_when_token_configured
```

`gcp_secret_project_wiring_reached_when_token_configured` was deliberately excluded from the
per-mutant test loop: it requires real network egress to Google's Secret Manager API (tagged
`@requires_external` by DISTILL, the only test in this feature reaching outside this repo's own
infrastructure) — running it dozens of times across an automated mutation loop is both slow and an
unreasonable load against a live third-party API. All other 5 `pr09` scenarios (walking skeleton +
4 `#[ignore]`d regression/edge guards) ran for every mutant via `--include-ignored`.

## Result: 2 mutants tested in 8m — 1 caught, 1 missed (investigated, confirmed a scoping artifact)

```
MISSED   crates/embyr-server/src/main.rs:180:21: delete ! in main
```

Only 2 mutants existed in the diff's own scope — the rest of the diff is straightforward
construction/field-threading code with no further mutatable conditionals or arithmetic.

## Interpretation

**The single miss is a direct, expected consequence of excluding the network-dependent test —
verified empirically, not assumed.** The mutant deletes the `!` in
`.filter(|v| !v.is_empty())` (the `EMBYR_GCP_ACCESS_TOKEN` presence/non-empty gate), inverting it:
under the mutant, only an EMPTY token string would pass the filter, meaning `gcp_secret_fetcher`
would stay `None` for every REAL (non-empty) token — the sweeper case DISTILL's own excluded test
specifically exists to prove. Manually re-applied this exact mutation to the file, cleaned up, and
ran ONLY the excluded network-dependent test against it (a single run, not repeated, to respect the
same "don't hammer Google's real API" reasoning that excluded it from the loop in the first place):

```
FAILED: assertion `left != right` failed: a 500 here means gcp_secret_fetcher is still None --
main.rs never reached the Some(...) construction arm; got 500 Internal Server Error:
{"error":"gcp_secret_fetcher_not_configured"}
```

Confirmed: the excluded test reliably catches this mutant. The miss is a scoping artifact of this
mutation run's own test-command choice (a deliberate, reasoned exclusion), not a genuine gap in
the acceptance suite as a whole. File reverted to clean (`git checkout --`) after the manual
verification, confirmed via empty `git diff`.

## Final state

All 6 `pr09` scenarios (including the excluded-from-mutation-loop network-dependent one)
independently reconfirmed passing. `us_10_aws_secrets.rs`/`us_11_gcp_secrets.rs` regression guards
unmodified and green (4/4 each, confirmed by DELIVER and independently by the orchestrator).
