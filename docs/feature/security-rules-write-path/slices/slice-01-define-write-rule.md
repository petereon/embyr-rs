# Slice 01: Alex Defines (and Redefines) an Independent Write Rule for a Collection

**Story**: US-01 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 1.5 days

## Goal
Let Alex define a write condition for a collection that is stored and evaluated completely independently of that collection's existing `security-rules` read condition.

## IN Scope
- Extend the `access_rules` schema (System DB) with a write-condition slot, keyed the same way as the existing read condition — `(project_id, collection_path)` — but stored/updated independently (schema-extension mechanism is DESIGN's call).
- Extend (or add a sibling to) the admin write-rule-definition action to accept and validate a write condition using the existing `parse_condition` (extended per Slice 02's grammar addition) before storing.
- Idempotent upsert semantics for the write condition, mirroring `security-rules`' Resolution 3 exactly: same action defines and redefines, no overlap window.
- Prove independence: redefining the write condition never touches the read condition, and vice versa.
- Reuse the existing `SYNTAX_ERROR`/`UNSUPPORTED_CONSTRUCT` rejection taxonomy and admin-session role gate (Owner/Admin only, mirroring AC-17-05).

## OUT Scope
- Actual write-time evaluation (Slices 02–04).
- The `request.resource` grammar addition itself (Slice 02 introduces the operand; this slice only needs the storage/admin-API shape to accept arbitrary valid v1-grammar text, extended or not).
- Simulation extension (Slice 07).

## Learning Hypothesis
Disproves: an independent write condition cannot be added to the existing `access_rules` schema, defined/redefined idempotently, without either colliding with the existing read condition or requiring a wholesale schema redesign.
Confirms (if it passes): the read condition (`security-rules`) and the new write condition can coexist as two independently-upsertable slots on the same `(project_id, collection_path)` key, with zero code path shared between their write operations.

## Acceptance Criteria
- AC-17-20, AC-17-21, AC-17-22, AC-17-23 (parses `request.resource.data.<field>` once Slice 02 lands the grammar — for this slice, verify the endpoint accepts and stores arbitrary valid text), AC-17-24, AC-17-25.

## Dependencies
- `security-rules` FINALIZED (`docs/evolution/2026-08-18-security-rules.md`) — the `access_rules` table, `parse_condition`, `define_access_rule` handler, and `verify_project_ownership` all exist and are reused.

## Production-Data Taste Test
Real System DB row extension against `trailmark-prod`, real admin Bearer credential, real existing `journal_entries` read rule confirmed unchanged before and after this slice's write-condition operations.

## Reference Class
Direct structural precedent: `security-rules`' own Slice 01 (`docs/feature/security-rules/slices/slice-01-define-and-redefine-rule.md`) and ADR-028's `upsert_access_rule`/`get_access_rule` adapter-method shape.
