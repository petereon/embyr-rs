# ADR-008: Crate Structure — embyr-admin-ui as Separate Workspace Member

## Status

Accepted

## Context

The admin UI is a browser-side WASM application built with Leptos 0.8 CSR (ADR-005). It must be integrated into the embyr-rs Cargo workspace and served by `embyr-admin`. Three structural options exist:

1. **Separate workspace member** (`crates/embyr-admin-ui/`) — a distinct Cargo package within the workspace, with its own `Cargo.toml`, its own build target (WASM via `trunk`), and its own source tree.
2. **Inline in `embyr-admin`** — the Leptos UI lives in `crates/embyr-admin/` alongside the Axum server code, with conditional compilation (`#[cfg(feature = "csr")]` / `#[cfg(feature = "ssr")]`).
3. **Monorepo sibling** — the UI is a separate `crates/admin-ui/` directory that is NOT part of the Cargo workspace, managed by its own Cargo.lock and `trunk` in complete isolation from the server workspace.

The build pipeline constraint is: **`trunk build` is not part of `cargo build`**. The UI is compiled by `trunk` separately and the resulting `dist/` directory is served by `embyr-admin` via `ServeDir`. This build separation is a locked decision from the design spec.

The existing workspace has 6 crates (after the `embyr-pg-storage` introduction in the embyr-agent feature): `embyr-proto`, `embyr-core`, `embyr-pg-storage`, `embyr-server`, `embyr-admin`, `embyr-agent`.

## Decision

**`crates/embyr-admin-ui/` is a new, separate Cargo workspace member** added to the root `Cargo.toml` workspace members list.

### Crate properties

| Property | Value |
|----------|-------|
| Crate name | `embyr-admin-ui` |
| Crate type | `bin` (WASM target, not a native binary) |
| Build tool | `trunk` (not `cargo build`) |
| Entry point | `src/main.rs` → `leptos::mount_to_body(App)` |
| Target triple | `wasm32-unknown-unknown` (via `trunk`) |
| Dependencies | `leptos`, `wasm-bindgen`, `web-sys`, `js-sys`, `serde`, `uuid`, `chrono` |
| Workspace-level deps | May depend on `embyr-core` for shared domain types (V2 consideration; V1 uses locally-defined types in `model.rs`) |

### Workspace registration

```toml
# Cargo.toml (root workspace)
[workspace]
members = [
    "crates/embyr-proto",
    "crates/embyr-core",
    "crates/embyr-pg-storage",
    "crates/embyr-server",
    "crates/embyr-admin",
    "crates/embyr-agent",
    "crates/embyr-admin-ui",    # new
]
```

### Build pipeline

```makefile
# Makefile (root)
ui:
    cd crates/embyr-admin-ui && trunk build --release
    mkdir -p admin-ui/dist
    cp -r crates/embyr-admin-ui/dist/* admin-ui/dist/

# embyr-admin serves from this path:
# axum: nest_service("/admin", ServeDir::new("admin-ui/dist"))
```

`cargo build` does NOT build `embyr-admin-ui`. The WASM target is invisible to `cargo build --workspace` because the crate's `Cargo.toml` only declares a `[lib]` or `[[bin]]` for `wasm32-unknown-unknown` which `cargo build` (targeting the host platform) skips. A custom `.cargo/config.toml` may set:

```toml
[build]
# Do NOT set target = "wasm32-unknown-unknown" here — would break server crates
```

Instead, `Trunk.toml` in `crates/embyr-admin-ui/` controls the WASM build. `cargo clippy --workspace` may warn on unused features in the UI crate if not excluded; the CI pipeline uses `--exclude embyr-admin-ui` for native lint passes and runs `trunk build --release` separately as a dedicated CI job.

### `cargo deny` scope

`embyr-admin-ui` is excluded from the `embyr-core` crate-boundary rules (since it legitimately depends on `web-sys`, `wasm-bindgen`, and similar browser crates that would be disallowed in `embyr-core`). The `deny.toml` for `embyr-admin-ui` has its own deny rules: no native IO crates (`tokio`, `sqlx`, `tonic`, `axum`).

## Consequences

**Benefits:**

- Complete build separation: `trunk build` compiles the UI to WASM; `cargo build --workspace` compiles the server binaries. Neither blocks the other. A UI change does not invalidate the server build cache; a server change does not invalidate the WASM build cache.
- Dependency isolation: `leptos`, `wasm-bindgen`, and `web-sys` live only in `embyr-admin-ui/Cargo.toml`. They are not transitive dependencies of `embyr-server` or `embyr-admin`. This keeps server build times fast and avoids WASM-specific crate requirements leaking into server compilation units.
- Future shared types: if `embyr-admin-ui` needs to share domain types with `embyr-core` (e.g., for V2 `#[server]` function signatures), the workspace structure enables `embyr-admin-ui` to `use embyr_core::...` directly. The workspace resolver handles cross-target dependencies.
- Independent versioning: the UI crate has its own version in `Cargo.toml`. Breaking UI changes can be tracked independently from server binary versions.
- `cargo test` works on the UI crate for pure Rust unit tests (testing `update()`, mock data constructors, SVG path math functions) without requiring a browser or WASM runner. Only tests that exercise DOM APIs require `wasm-pack test` or `trunk test`.

**Trade-offs and costs:**

- Two build commands are required to produce a deployable `embyr-admin`: `make ui && cargo build -p embyr-admin`. The Makefile `make all` target chains them. CI must run both steps.
- `cargo clippy --workspace` may fail or emit confusing diagnostics on the `embyr-admin-ui` crate if the host platform is not `wasm32-unknown-unknown`. The CI pipeline excludes `embyr-admin-ui` from the native clippy pass and runs `trunk check` (or `cargo clippy --target wasm32-unknown-unknown -p embyr-admin-ui`) as a separate step.
- Developers must install `trunk` and the `wasm32-unknown-unknown` target (`rustup target add wasm32-unknown-unknown`) to build the UI locally. The project `README.md` or `CLAUDE.md` must document this prerequisite.

## Alternatives Considered

### Alternative A: Inline in embyr-admin

Add the Leptos UI source to `crates/embyr-admin/src/` with conditional compilation. The same Cargo package produces both the native Axum binary and the WASM SPA.

**Rejected because:**
- `leptos`, `wasm-bindgen`, and `web-sys` would be dependencies of `embyr-admin`. When `cargo build -p embyr-admin` runs on the server (targeting the native platform), these crates compile but their browser APIs are not available. This works via conditional compilation (`#[cfg(target_arch = "wasm32")]`) but creates a brittle interleaving of server and browser code in a single crate.
- `trunk` would need to target the same `Cargo.toml` that `cargo build` uses, with `--features csr`. This couples the build flags in a non-obvious way — a wrong flag combination silently produces an incorrect binary.
- Any Leptos version bump might affect the native compilation of `embyr-admin` even if the UI code is unchanged, increasing coupling between unrelated concerns.
- The SSR migration path (V2) requires `embyr-admin` to depend on `leptos_axum`. Inlining the UI means the SSR migration changes `embyr-admin/Cargo.toml` directly, affecting the server binary's dependency surface. As a separate crate, the migration is isolated to `embyr-admin-ui/Cargo.toml`.

### Alternative B: Monorepo sibling (outside workspace)

Place `crates/admin-ui/` outside the Cargo workspace, with its own `Cargo.lock`. Complete isolation.

**Rejected because:**
- Workspace-level dependency deduplication is lost. `serde`, `uuid`, and `chrono` used by both server and UI crates would maintain separate versions.
- Future shared types between `embyr-core` and `embyr-admin-ui` would require either duplication or a separate published crate. With workspace membership, `embyr-admin-ui` can `path = "../embyr-core"` and the workspace resolver deduplicates.
- `cargo deny`, `cargo audit`, and workspace-level CI tooling naturally exclude non-workspace members, requiring a second CI configuration for the standalone crate.
- The design spec placed `embyr-admin-ui` within `crates/` alongside all other workspace crates. Departing from this convention requires explanation in every developer onboarding document.
