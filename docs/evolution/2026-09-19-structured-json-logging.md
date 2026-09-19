# Evolution: structured-json-logging

Closes Medium finding #25 (Reliability) from `docs/product/production-readiness-audit-2026-09-08.md`.

## Business Context

`embyr-server` logged plain-text key=value lines, which production log aggregators (Loki/CloudWatch/Datadog)
parse unreliably at scale. Switching to structured JSON output makes every log line machine-parseable
without changing what gets logged.

## Key Decisions

| Decision | Rationale |
|---|---|
| Unconditional `.json()`, no mode-switch/env flag | Nothing depends on plain-text output; a toggle would be unused config surface |
| No ADR | Single builder-method change to an existing subscriber, not an architectural decision |
| ANSI/`IsTerminal` code (finding #19) removed outright | JSON output never colorizes — the interactivity check became dead code, not just unused |

## Lessons

- DESIGN's own self-review flagged `drp01_startup_version_log.rs`'s `version=`/`version="..."` substring
  assertion as at-risk under the JSON shape change. DELIVER fixed it correctly — real `serde_json::from_str`
  parsing of every captured log line, not another fragile substring check — closing the risk durably instead
  of patching around the new format.

## Key Files

- `crates/embyr-server/src/main.rs:73-79` — the `.json()` / `.with_ansi(...)`-removal diff
- `tests/deployment_release_process/acceptance/drp01_startup_version_log.rs` — updated to real JSON parsing
- `docs/feature/structured-json-logging/deliver/mutation/mutation-report.md` — mutation report (1/1 caught)

## Follow-Up

- Finding #26 (field-path SQL interpolation defense-in-depth) and later Medium/Low items in
  `docs/product/production-readiness-audit-2026-09-08.md` remain open.
