# Slice 01 (Walking Skeleton): A Rule's Every State Is Captured, Attributed, and Timestamped

**Story**: US-01 | **Release**: 1 | **Estimate**: 1 day

## Goal
Every successful call to the existing `access_rules` define/redefine action also captures a history entry — the condition just made active, who set it, and when — with zero change to the existing define/redefine behavior itself.

## IN Scope
- A new, additive history table (recommended `access_rule_history`) capturing: project id, collection path, the condition just made active, acting admin `account_id`, and the capture time.
- A new `INSERT` into that table, executed alongside (not instead of) the existing, unmodified `upsert_access_rule` call in `define_access_rule`.
- Correct behavior on a rule's very first-ever definition (produces exactly one history entry, not zero).
- Correct ordering across two rapid successive redefinitions (two distinct entries, chronologically ordered).

## OUT Scope
- Retrieving/viewing history (Slice 02).
- Restoring a rule to a prior state (Slice 03).
- `write_access_rules`/`group_access_rules` (Slices 04/05).
- Any change to `access_rules`' own schema, response shape, or read-path behavior.

## Learning Hypothesis
**Disproves if it fails**: a rule's prior condition cannot be captured, attributed, and timestamped on every redefine without either mutating `access_rules`' own locked single-row upsert semantics (ADR-028) or requiring a new evaluation-path change.
**Confirms if it succeeds**: history capture is a genuinely additive operation, layered alongside the existing upsert, with zero regression to any already-shipped behavior.

## Acceptance Criteria
- AC-17-156: Every successful define/redefine captures a history entry (condition, actor, time).
- AC-17-157: A rule's first-ever definition produces exactly one history entry.
- AC-17-158: Two successive redefinitions produce two distinct, correctly-ordered entries.
- AC-17-159: `access_rules`, its response shape, and AC-17-01 through AC-17-19 are completely unmodified.

## Dependencies
None — foundational story.

## Effort Estimate
1 day.

## Reference Class
Mirrors ADR-028's own "additive, alongside the existing upsert, zero touch to the existing statement" pattern; the history table itself mirrors `access_rules`' own schema-identical-sibling-table precedent (ADR-030/032), applied here to a NEW capture-only table rather than a second rule-definition table.

## Pre-Slice SPIKE
Not needed — the reuse pattern (additive `INSERT` alongside an existing, unmodified upsert; actor sourced from `SessionContext.account_id`, already in scope) is directly precedented and confirmed by direct code read, not novel.
