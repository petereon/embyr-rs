# Mutation Report — request-body-size-limits (finding #24)

Scope: `git diff 3d872f6..2cc8cb3` production code only —
`crates/embyr-server/src/{lib.rs,rest/grpc_web.rs,admin/router.rs}`.

Command (build AND test both scoped to the single `production_readiness`
test binary — see build-scoping note below):

```
cargo mutants -p embyr-server --in-place --in-diff <diff> \
  -C --test -C production_readiness --timeout 150 \
  -- -- --test-threads=1 request_body_ceilings
```

## Build-scoping fix (carried over from `pool-sizing-and-limits`)

`crates/embyr-server/Cargo.toml` has ~196 `[[test]]` targets. Without
`-C --test -C production_readiness`, cargo-mutants' `cargo test --no-run`
build step compiles all of them (observed 300s+/mutant in the prior killed
attempt, and reconfirmed mid-session when a stale duplicate agent's
unscoped run was caught live rebuilding unrelated targets like
`firestore_batch_write_bw01_all_writes_succeed`). With the `-C` pair applied
to every cargo-mutants invocation (build *and* test), the build step's own
log line reads `cargo test --no-run ... --test production_readiness` —
confirmed empirically, only that one binary compiles.

Real per-mutant cost observed after scoping: baseline 89s build (cold-ish
incremental) + 17s test; individual mutants 6–15s build + 10–15s test
(2 Postgres testcontainers per DELIVER's own report). Full 17-mutant run:
6 minutes wall clock, vs. the prior attempt's 374s+ *baseline alone*.

## Results

17 mutants generated, all classified:

| Outcome | Count | Detail |
|---|---|---|
| Caught | 13 | see below |
| Unviable | 4 | `spawn_all_servers -> JoinHandle<()>` replaced with `JoinHandle::new()`/`::new(())`/`::from(())`/`::from_iter([()])` — none compile (wrong arity/trait) |
| Missed → fixed → caught | 1 | see below |
| Timeout | 0 | |

**Caught (12 on first run, arithmetic/`Default` mutants on the three byte
ceilings and the router's early-return):**
- `lib.rs:38` (`MAX_GRPC_MESSAGE_BYTES = 10 * 1024 * 1024`): both `*` → `+`/`/` (4 mutants)
- `lib.rs:39` (`MAX_REST_BODY_BYTES = 2 * 1024 * 1024`): both `*` → `+`/`/` (4 mutants)
- `router.rs:95` (`MAX_ADMIN_BODY_BYTES = 1 * 1024 * 1024`): second `*` → `+`/`/`, first `*` → `/` (3 mutants)
- `router.rs:141` (`build_admin_router -> Router`): replaced with `Default::default()` (1 mutant)

**Missed, then fixed (test-only):** `router.rs:95:39` — mutating the
*first* `*` in `1 * 1024 * 1024` to `+` gives `1 + 1024 * 1024` (operator
precedence: `*` still binds tighter) = ceiling **+ 1 byte**, not the huge
swing a `1024 * 1024` operand mutation produces. The acceptance test's
"over ceiling" boundary used a ±1 KiB margin (deliberate, documented, and
correctly sized for the other two surfaces' operands), which cannot see a
1-byte drift on this specific degenerate `1 * x` operand.

Fix: tightened only the admin "over" boundary in
`tests/production_readiness/acceptance/pr14_request_body_size_limits.rs`
from `ADMIN_CEILING_BYTES + 1024` to `ADMIN_CEILING_BYTES + 1` (byte-exact
— relies on axum `DefaultBodyLimit`'s strict `>` rejection semantics, so a
body sized `ceiling + 1` is rejected under the real ceiling but *not*
rejected under the `ceiling + 1` mutant, flipping the assertion). Re-ran
narrowly (`-f admin/router.rs -F "95:39.*replace \* with \+"`): now
**caught** in 1 mutant / 3 min (144s baseline build — cold — + 15s test).

Final tally: 13 caught, 4 unviable, 0 missed, 0 timeout — 17/17 accounted
for. No further gaps.
