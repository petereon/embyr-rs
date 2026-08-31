# Slice 01: Wire Field Transforms Through to the Already-Shared Compute Path (Walking Skeleton)

**Story**: US-01 | **Release**: 1 | **Estimate**: 1 day

## Goal

A new wire representation for field transforms on `storage_agent.proto` reaches the already-shared, already-correct `apply_field_transform`/`commit_transaction` compute path in `embyr-pg-storage` — no new compute logic.

## IN Scope

- New `Transform`/`FieldTransform` message + `Write` oneof variant on `storage_agent.proto`
- `proto_write_to_domain` translation edit (`crates/embyr-agent/src/server.rs`)
- Remove the `.filter_map` drop of `Write::Transform` (`crates/embyr-server/src/adapters/agent_backend.rs`, line 559)
- All 5 transform types (`serverTimestamp`, `increment`, `maximum`, `minimum`, `arrayUnion`/`arrayRemove`) covered as scenarios in the same slice — uniform wire path, no per-type slicing needed

## OUT Scope

- Any change to `apply_field_transform`'s own compute semantics (already correct, reused unchanged)
- Cross-version graceful degradation mechanism (feature-level escalation, resolved once across 3 sibling features)

## Learning Hypothesis

Disproves: the already-shared `apply_field_transform`/`commit_transaction` compute path cannot actually be reached from the agent's own `commit()` handler without a deeper refactor than adding a wire representation.
Confirms (if it succeeds): this is genuinely a thin wire-only fix, matching the Reading Confirmation's own finding.

## Acceptance Criteria

- [ ] `serverTimestamp()` persists a real, commit-time-matching timestamp
- [ ] `increment()` on an absent field defaults to the increment amount
- [ ] `increment()` on an existing numeric field adds correctly
- [ ] `arrayUnion()` adds new values without duplicating existing ones
- [ ] A standalone transform-only write applies correctly without touching unrelated fields
- [ ] Exercised against a real `embyr-agent` binary and real Postgres

## Dependencies

None (first and only slice).

## Effort Estimate

1 day.

## Reference Class

`firestore-field-transforms` (non-agent modes) — same compute semantics, reused unchanged; this slice is wire-only work.
