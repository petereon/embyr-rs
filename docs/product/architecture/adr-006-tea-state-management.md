# ADR-006: TEA State Management — RwSignal<AppModel> + Callback<Msg> via Leptos Context

## Status

Accepted

## Context

The embyr admin UI is a browser SPA with four bounded contexts (Auth, Database Management, Identity/Access, Billing) and roughly 30+ distinct user-triggered state transitions (Msg variants). State must be:

- **Predictable**: given the same model and the same message, the next state is deterministic. Bugs in view code cannot corrupt model state.
- **Testable**: the state transition function must be exercisable without a browser, without any async runtime, and without Leptos reactive infrastructure.
- **Shared**: components at any depth in the tree (sidebar, topbar, nested db_detail sub-views) must read and write the same model without prop-drilling through every intermediate component.
- **Efficient**: only components that actually depend on a changed piece of state should re-render. Global `setState` patterns that cause full-tree re-renders are unacceptable at the scale of 11 views + 8 primitives.

Three concrete patterns are in scope:

1. **TEA (The Elm Architecture)** via Leptos context: `RwSignal<AppModel>` + `Callback<Msg>` provided at the root; all components receive both via `use_context`.
2. **Prop drilling**: pass `model: ReadSignal<AppModel>` and `dispatch: Callback<Msg>` through component props from root to leaf.
3. **External state library**: `leptos-use` signals store, or a custom `Arc<Mutex<AppModel>>` shared between components via a custom hook.

## Decision

**TEA pattern via Leptos context: `RwSignal<AppModel>` provided as `ReadSignal<AppModel>` + `Callback<Msg>` injected into the Leptos context tree at the root.**

### Implementation contract

The root component (`src/app.rs`) owns all mutable state:

```
let model: RwSignal<AppModel> = RwSignal::new(AppModel::from_mock());
let dispatch: Callback<Msg> = Callback::new(move |msg: Msg| {
    model.update(|m| crate::update::update(m, msg));
});
provide_context(model.read_only());   // ReadSignal<AppModel>
provide_context(dispatch);            // Callback<Msg>
```

Any component at any depth accesses state:

```
let model = use_context::<ReadSignal<AppModel>>().expect("AppModel not in context");
let dispatch = use_context::<Callback<Msg>>().expect("dispatch not in context");
```

The `update` function signature is the contract that the DELIVER wave must honour:

```
// src/update.rs
pub fn update(model: &mut AppModel, msg: Msg) { ... }
```

Properties of this function:
- Takes `&mut AppModel` (no allocation, no async, no IO)
- Pattern-matches exhaustively on `Msg` (compiler enforces completeness)
- Pure: no `println!`, no `thread::spawn`, no `tokio::spawn`, no external calls
- Deterministic: same `(model, msg)` always produces the same next model
- Returns `()` — never fails; all error states are modelled as `Msg::PushToast`

### Reactive efficiency

Leptos 0.8 fine-grained reactivity means that a component reading `model.with(|m| m.nav.section)` re-renders only when `nav.section` changes — even though `AppModel` is a single struct. Components may use `Signal::derive()` to create narrower derived signals:

```
let section = Signal::derive(move || model.with(|m| m.nav.section.clone()));
```

This is the primary reactivity optimisation pattern. Components reading only `nav.section` do not re-render when `databases` changes.

### Async / IO boundary

`Resource` and `Action` (see ADR-007) own all async IO. When a `Resource` or `Action` completes, it calls `dispatch(Msg::SetDatabases(dbs))` via the context-provided `Callback<Msg>`. The `update` function then applies the change synchronously. The async/sync boundary is always at the `dispatch()` call site, never inside `update()`.

## Consequences

**Benefits:**

- The `update()` function is a pure Rust function. Unit tests are `update(&mut model, msg)` calls with assertions on model fields — no browser, no WASM, no Leptos. This is testable with standard `cargo test`.
- Exhaustiveness is enforced by the compiler. Adding a `Msg` variant without updating `update()` is a compile error.
- No prop-drilling: a deeply nested `logs.rs` view component can `use_context::<Callback<Msg>>()` and dispatch `Msg::PushToast` without the parent or grandparent components knowing about it.
- Single source of truth: `AppModel` is the only place application state lives. There are no per-component `useState` islands that can desync.
- V2 migration: when `#[server]` functions are added (ADR-007), they call `dispatch` on completion. No component changes required.

**Trade-offs and costs:**

- `AppModel` must implement `Clone` (required by Leptos's `RwSignal` machinery for reactive propagation). All domain types must be `Clone`. This is a mild constraint; the compiler enforces it.
- The `Msg` enum grows as features are added. At 30+ variants it is already large; at 50+ variants it may benefit from sub-enums (e.g., `Msg::Db(DbMsg)`) for organisation. This is a maintainability concern for V2, not a blocking issue for V1.
- Context lookup (`use_context`) panics if the provider is not in scope. This is a programming error, not a runtime failure. Tests must ensure context is provided; the `App` root component always provides it.

## Alternatives Considered

### Alternative A: Prop drilling

Pass `model: ReadSignal<AppModel>` and `dispatch: Callback<Msg>` as props through every component.

**Rejected because:**
- A component like `db_detail/logs.rs` is three levels deep (App → DatabaseDetail → LogsView → LogRow → FilterBar). Prop drilling through all intermediate components means every component in the chain must accept and re-export `model` and `dispatch` props even if it only uses one of them. This is mechanical, error-prone, and makes component refactoring fragile.
- Adding a new field to `AppModel` accessed by a leaf component requires updating every intermediate component's prop signature. Leptos context avoids this entirely.

### Alternative B: External state library (leptos-use store or custom Arc<Mutex<AppModel>>)

Use a third-party state management layer on top of Leptos signals.

**Rejected because:**
- `leptos-use` (0.10.x) provides useful utility hooks but does not provide a TEA-style update pattern. Its store primitives are per-signal, not per-model.
- A custom `Arc<Mutex<AppModel>>` would require explicit lock acquisition (`model.lock().unwrap()`) in view code — verbose, and locks cannot be held across `await` points in Leptos effects.
- Adding a state library dependency increases bundle size and introduces a versioning dependency outside the core Leptos ecosystem. The entire pattern (RwSignal + context + Callback) is built into Leptos 0.8 core with zero additional dependencies.
- The design spec was authored around the native Leptos primitives. Departing from them would require explaining the divergence to every contributor.

### Alternative C: Multiple RwSignal<T> per domain area (databases: RwSignal<Vec<Database>>, members: RwSignal<Vec<Member>>, …)

Split state into per-domain signals instead of a single `AppModel`.

**Rejected because:**
- Cross-domain state transitions (e.g., deleting a database cascades to SDK key revocation) require coordinating multiple `RwSignal` updates atomically. With a single `AppModel` this is one `model.update(|m| update(m, Msg::DeleteDatabase(id)))` call that handles both the database removal and the key cascade inside `update()`. With multiple signals, the component must update `databases.update(...)` and `sdk_keys.update(...)` separately — introducing a window between the two updates where the state is inconsistent.
- The `update()` function's testability argument collapses: there is no single function to test for cross-domain transitions.
- The design spec's `AppModel` struct definition is the SSOT for state shape. Fragmenting it into multiple signals would diverge from the spec without benefit.
