# Slice 01: Alex Defines an Independent Collection-Group Rule

**Story**: US-01 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 1 day

## Goal
Alex can define, and idempotently redefine, a collection-group rule for a bare collection id — stored disjointly from `access_rules`/`write_access_rules`, never colliding with either.

## IN Scope
- New disjoint table `group_access_rules`, `PRIMARY KEY (project_id, collection_id)`, schema-identical in column shape to `access_rules`/`write_access_rules` (ADR-028/030 precedent).
- New adapter methods `upsert_group_access_rule`/`get_group_access_rule`, mirroring `upsert_write_access_rule`/`get_write_access_rule`'s exact shape (single indexed PK lookup, `INSERT ... ON CONFLICT ... DO UPDATE`).
- New admin handler `define_group_access_rule`, mirroring `define_write_access_rule`'s shape (Owner/Admin role gate, `parse_condition` validation, `SYNTAX_ERROR`/`UNSUPPORTED_CONSTRUCT` taxonomy reused unchanged).
- Validation: a submitted `collection_id` containing `/` is rejected (a group id is a bare identifier, never a path).

## OUT Scope
- No enforcement wiring yet (Slices 02–04).
- No change to `access_rules`/`write_access_rules` tables or their adapter methods.
- No new admin route validation beyond the bare-id check (no length/charset rules beyond what `access_rules`' `collection_path` already permits, minus `/`).

## Learning Hypothesis
Disproves: "an independent collection-group rule cannot be stored, defined, and idempotently redefined, disjoint from `access_rules`/`write_access_rules`, without either colliding with an existing exact-path row or requiring a schema redesign."
Confirms (if it succeeds): a third disjoint table, mirroring ADR-030's own precedent almost verbatim, is sufficient — no new storage pattern needed.

## Acceptance Criteria
- AC-17-77: first-time group-rule definition is stored and active.
- AC-17-78: redefinition fully replaces, no overlap window.
- AC-17-79: zero observable effect on the same collection id's exact-path rule, and vice versa.
- AC-17-80: a `/`-containing collection id is rejected with a distinguishable reason.

## Dependencies
- ADR-028's `access_rules` schema pattern, ADR-030's `write_access_rules` disjoint-table precedent (both read-only reference, not modified).
- `parse_condition()`/`ConditionParseError` (ADR-027), reused unchanged for validation.

## Effort Estimate
1 day. Reference class: `security-rules-write-path` Slice 01 (`define_write_access_rule` + `write_access_rules` schema), same shape, same effort.

## Pre-Slice SPIKE
None required — direct precedent (ADR-030) exists for the exact schema/adapter/handler shape.
