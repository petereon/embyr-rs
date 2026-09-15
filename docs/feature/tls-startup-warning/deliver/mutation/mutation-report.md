# Mutation Report — tls-startup-warning

Scope: `cargo mutants -p embyr-server --in-diff <9-line diff df5486e..562e9a3>` (crates/embyr-server/src/main.rs, the `if cfg.tls.is_none() { tracing::warn!(...) }` block only). `-j 1`, `--test-threads=1`, filtered to `--test tls_startup_warning`.

## Result

| Metric | Value |
|---|---|
| Mutants found | 1 |
| Caught | 1 |
| Missed | 0 |
| Unviable | 0 |
| Timeout | 0 |

Baseline build 381s, baseline test 14.8s. Single mutant (`main.rs:74:5: replace main with ()`, whole-fn-body genre — no sub-expr mutant generated for the macro call/`is_none()` condition) killed in 0.7s by the acceptance tests (`warns_on_startup_when_tls_is_disabled`, `does_not_warn_on_startup_when_tls_is_configured`).

## Verdict

100% kill rate (1/1). Clean — matches session precedent that 0-2 mutants / 0 viable-missed is expected and valid for a diff this size (log-line nudge, no branching logic beyond the existing `is_none()` check).
