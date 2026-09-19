# structured-json-logging

Closes finding #25 (Medium, Reliability) — `docs/product/production-readiness-audit-2026-09-08.md`:
plain-text `tracing_subscriber::fmt()` output is unreliable to parse at scale for Loki/CloudWatch/Datadog.

Combined DISCUSS+DESIGN pass (token-budget mode). Design-only wave — no production code changed.

## Current state (`crates/embyr-server/src/main.rs:84-91`)

```rust
tracing_subscriber::fmt()
    .with_env_filter(
        EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new(&cfg.log_level)),
    )
    .with_writer(std::io::stderr)
    .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
    .init();
```

Same builder chain `deployment-release-process` (finding #19) touched for the ANSI/`IsTerminal` fix — confirmed by reading `docs/evolution/2026-09-15-deployment-release-process.md` and the comment at `main.rs:80-83`. No other tracing-init site exists in `embyr-server` (single `#[tokio::main]` in `main.rs`).

## Design

### Formatter: unconditional `.json()`, no mode switch

```rust
tracing_subscriber::fmt()
    .json()
    .with_env_filter(
        EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new(&cfg.log_level)),
    )
    .with_writer(std::io::stderr)
    .init();
```

- `.with_ansi(...)` line **removed** — `.json()` never colorizes; keeping `with_ansi` would be dead config (the JSON formatter ignores it, and `IsTerminal` import becomes unused → compiler warning / possible `-D warnings` CI failure). ANSI/`IsTerminal` finding #19 logic becomes moot, not adjusted — deleted outright.
- No env-var/mode escape hatch (no `EMBYR_LOG_FORMAT=json|text`). Rationale: this binary runs in exactly two contexts — `cargo run` on a dev laptop and the Docker image (`docker-compose.yml`, CI, prod). `docker-compose.yml` sets no log-format env var and assumes nothing about format (checked — only `DATABASE_URL`/`EMBYR_ADMIN_KEY`/`EMBYR_ENCRYPTION_KEY`). A human reading JSON locally pipes through `jq` (one extra command); a mode-switch is a second code path nobody will remember exists, and this session's own bias (ponytail: fewest branches) argues against it. Add the switch only if a real dev-ergonomics complaint shows up — trivial to bolt on later (`if cfg.log_format == "json"`).
- No new dependency: `.json()` is a method on `tracing_subscriber::fmt::SubscriberBuilder`, gated behind the crate's own `json` cargo feature.

### Cargo.toml — feature flag must be added

Checked `Cargo.toml:65`:
```
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```
`json` is **not** a default feature of `tracing-subscriber` 0.3 (defaults: `fmt`, `ansi`, `tracing-log`, `smallvec`, `std`) and is not currently enabled here. Required change:
```
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
```
`embyr-admin`/`embyr-agent` also depend on `tracing-subscriber.workspace = true` (their own `Cargo.toml`s) but do not call `.init()` with a JSON formatter in this scope — out of scope for finding #25 (finding cites `embyr-server/src/main.rs:73-79` only). Flag as a natural follow-up, not bundled here.

### Structured field call sites: zero changes needed

`tracing::info!(key = value, "message")` macro calls are formatter-agnostic — `tracing`'s `Value`/`Visit` trait is implemented once per subscriber layer; `fmt::Layer` picks plain-text (`key=value`) or JSON (`"key":"value"`) rendering based on which formatter is installed, not on how the call site is written. Confirmed by reading the two representative call sites already in `main.rs`:
- `main.rs:398-405` (Step 12 ready log): `version = env!(...)`, `grpc = %format!(...)`, message `"embyr-server v{} ready"`.
- `main.rs:110-115` (TLS warning): `tls_enabled = false`, `vars = "EMBYR_TLS_CERT_PATH, EMBYR_TLS_KEY_PATH"`.

Every other `tracing::info!`/`warn!`/`error!` call in the codebase (rate-limiter's `project_id`/`error`, CORS origin checks, etc.) uses the same macro form — all render correctly under `.json()` with **zero call-site edits**.

## ADR

None. Formatter-layer swap using an already-available (feature-gated) library capability — no new architectural decision, no new component, no alternative technology considered. Same "no ADR" precedent as the sibling `tls-startup-warning` finding (#22) in this codebase, which used identical reasoning for a comparably small logging change.

## Self-review: existing test compatibility

Grepped `tests/` for every `output.contains(...)` assertion against captured server stdout/stderr (2 files, 3 assertions total):

1. **`tests/deployment_release_process/acceptance/drp01_startup_version_log.rs:63-68` — AT RISK, will break.**
   ```rust
   assert!(
       output.contains(&format!("version={expected_version}"))
           || output.contains(&format!("version=\"{expected_version}\"")),
       ...
   );
   ```
   Both disjuncts assume plain-text `key=value` / `key="value"` syntax. `.json()` renders the same field as `"version":"0.1.1"` (colon, quoted key) — **neither disjunct matches**. This assertion will fail once the formatter switches. The second assertion in the same test (`output.contains(&format!("v{expected_version}"))`, line 70) checks the human message `"embyr-server v0.1.1 ready"`, which JSON still carries verbatim as a string value under `"message"` — that half is unaffected.
   → DELIVER must update this test's structured-field assertion to the JSON shape (e.g. `"version":"{expected_version}"` or `"version":\"{expected_version}\"`, matching whatever `serde_json`-shaped substring the real output produces). Flagging here per the finding's own request; not fixed in this design-only pass.
   Also: the doc comment at `main.rs:80-83` ("ANSI color codes ... structured fields (e.g. `version="0.1.1"`) remain machine-parseable") references the old plain-text shape and should be rewritten in DELIVER, not just the code below it.

2. **`tests/tls_startup_warning/acceptance/ts01_startup_warning.rs:47-51,75-79` — NOT at risk.**
   ```rust
   const WARNING_VARS: &str = "EMBYR_TLS_CERT_PATH, EMBYR_TLS_KEY_PATH";
   assert!(output.contains(WARNING_VARS), ...);
   ```
   This checks a raw substring of the *value* only (no `key=`/`key:` prefix, no surrounding quotes asserted). The value `"EMBYR_TLS_CERT_PATH, EMBYR_TLS_KEY_PATH"` contains no characters JSON needs to escape (comma, space are literal), so it appears byte-identical inside `"vars":"EMBYR_TLS_CERT_PATH, EMBYR_TLS_KEY_PATH"`. Passes unchanged under JSON.

No other test in `tests/` greps captured log output for a format-sensitive pattern (searched full `tests/` tree for `output.contains`).

## Verdict

- Formatter: unconditional `.json()`, no mode switch — justified above.
- Cargo.toml: **yes**, add `"json"` to `tracing-subscriber` features (not currently enabled).
- ANSI/IsTerminal: moot under JSON — delete the `.with_ansi(...)` call and the now-unused `IsTerminal` import; not "adjust," remove.
- Structured field call sites: confirmed zero changes needed anywhere else in the codebase — formatter-agnostic macro usage.
- Test compatibility: **1 test at risk** — `tests/deployment_release_process/acceptance/drp01_startup_version_log.rs` (its `version=`/`version="..."` structured-field assertion), needs a DELIVER-time update to the JSON key:value shape. `ts01_startup_warning.rs` is safe (plain substring, format-agnostic). No ADR.
