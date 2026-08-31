# Slice 01: Server Timestamp Actually Persists (Walking Skeleton)

**Feature**: firestore-field-transforms | **Story**: US-01 | **Release**: 1 | **Estimate**: 2 days

## Goal

Prove the full four-layer plumbing (domain model → both wire shapes → PG persistence → response wiring) end-to-end using the simplest transform kind (`setToServerValue: REQUEST_TIME`), which needs no read-before-write and no type reasoning — isolating "does the pipe exist" from "is the computation correct" (Slice 02/03's own concern).

## IN Scope

- `embyr-core::storage::backend_adapter::FieldTransform::ServerTimestamp` actually constructed (today: dead code, never built anywhere).
- `translate_one_write_for_commit`'s `Operation::Transform` arm actually reads `dt.field_transforms` instead of discarding it into `vec![]`.
- `translate_one_write_for_commit`'s `Update` arm reads `proto_write.update_transforms` (field 7) — today: not read at all, no comment, silently ignored. Requires a domain-model decision for "one write, both fields and a transform" (see § System Constraints, feature-delta.md) — resolved by DESIGN, implemented here.
- `embyr-core::domain::document::WriteResult` gains a field to carry computed transform-result values, threaded through `BackendAdapter::commit_transaction`'s return value to all 5 existing proto-response call sites in `handler.rs` (currently all hardcode `transform_results: vec![]`).
- PG `commit_transaction`'s apply loop actually writes the computed timestamp into the `fields` JSONB column for a `ServerTimestamp` transform (today: no-op, `WriteResult` returned but nothing written).
- Backend modes: `direct_pg`, `aws_secret`, `gcp_secret` only (agent-mode explicitly out — see feature-delta.md § Out of Scope).
- Top-level field paths only (dotted/nested paths out — see feature-delta.md § Out of Scope).
- `InvalidArgument` rejection for unsupported `ServerValue` enum values, document left unmodified.

## OUT Scope

- `increment`/`maximum`/`minimum` (Slice 02) — no read-before-write logic needed here.
- `appendMissingElements`/`removeAllFromArray` (Slice 03).
- `backend_mode=agent` (hard proto wall on the separately-deployed `embyr-agent` binary — feature-delta.md § Reading Confirmation).
- Dotted/nested `field_path` targets — no existing nested-map get/set primitive in this codebase to reuse.

## Learning Hypothesis

**Disproves if it fails**: the full four-layer plumbing (domain `FieldTransform`/`WriteResult` model extension, both wire-shape translations, PG persistence, response wiring at all 5 call sites) cannot be wired end-to-end for even the simplest transform kind without requiring a new `BackendAdapter` trait method, or without a breaking change to `Write`'s own enum shape that Slice 02/03 could not build on cleanly.

**Confirms if it succeeds**: the plumbing this feature's every other slice depends on works, and the combined update+transform domain-model shape (resolved by DESIGN) is sound.

## Acceptance Criteria

- [ ] AC-01-01: A standalone `serverTimestamp()` transform write persists a real server-generated timestamp, readable via `GetDocument` immediately after.
- [ ] AC-01-02: A `serverTimestamp()` transform attached to a regular `update` (via `update_transforms`) persists BOTH the regular field change and the transformed field in the same write.
- [ ] AC-01-03: A `serverTimestamp()` transform creates the target field if it does not already exist.
- [ ] AC-01-04: `WriteResult.transform_results` contains the computed timestamp value, positionally aligned to the transform's own position.
- [ ] AC-01-05: An unsupported `ServerValue` is rejected with `InvalidArgument`, and the document is left unmodified.

## Production-Data Taste Test

Real Postgres, a real `trip_entries/kilimanjaro-trek` document for Maria Santos: one `Commit` call with a standalone `serverTimestamp()` transform, and a second `Commit` call mixing regular `update` fields with an attached `update_transforms` sentinel — asserting the persisted `updatedAt` field is a real server timestamp (not client-supplied, not absent), readable via a real `GetDocument` call. No mocked adapter.

## Dependencies

- `translate_one_write_for_commit`/`handle_commit` (shipped) — reuse TARGET, modified in place, not reused unchanged.
- `BackendAdapter::commit_transaction`'s existing single-`pg_txn` apply loop (shipped) — extended, not replaced.
- `security-rules-write-path` (shipped) — per-write access-rule evaluation, already applied to transform writes unchanged.

## Reference Class

`verify_versions`'s own `FOR UPDATE` row-locking idiom (pattern reference only — this slice's own `ServerTimestamp` computation needs no read-before-write, unlike Slice 02) + the JSON↔`FieldValue` encoding boundary (`fields_to_json`/`json_to_field_value`) already proven for `Update` writes.

## Pre-Slice SPIKE

Not needed for the computation itself (a pure function of "now," already computed once per `commit_transaction` call). A short DESIGN-time decision IS required (not a SPIKE): the exact domain-model shape for a `Write` carrying both regular fields and attached transforms — see feature-delta.md § System Constraints, flagged for DESIGN, not requiring exploratory research to resolve.
