# Slice 05: The Identical History Mechanism Protects Collection-Group Rules Too

**Story**: US-05 | **Release**: 1 | **Estimate**: 0.5 day

## Goal
The identical history mechanism generalizes to `group_access_rules`, completing symmetric history/versioning/rollback coverage across all 3 rule tables this initiative has ever shipped.

## IN Scope
- A third, independently-stored history table (recommended `group_access_rule_history`), populated alongside the existing, unmodified `upsert_group_access_rule` call.
- Retrieval and restore for collection-group rule history, mirroring Slices 02/03/04.
- Proof that a collection-group rule's history is structurally independent of any same-named exact-path rule's own history (mirrors ADR-032's own AC-17-93/94/95/96 non-interference discipline).

## OUT Scope
- Any change to `group_access_rules`' own existing write path (ADR-032), or to any of the other two rule tables' history mechanisms (Slices 01-04).

## Learning Hypothesis
**Disproves if it fails**: the identical mechanism does NOT extend to `group_access_rules` without a table-specific complication — the last of Resolution 1's three-table generalization claims is disproven.
**Confirms if it succeeds**: this feature's central deliverable is complete — no rule change, of any kind, in any of the 3 rule tables this initiative has ever shipped, is ever silently lost again.

## Acceptance Criteria
- AC-17-171: `group_access_rules` redefinitions are captured, attributed, retrievable — identical mechanism.
- AC-17-172: A collection-group rule's history is structurally independent of any same-named exact-path rule's own history.
- AC-17-173: A collection-group rule can be restored to any prior entry via the identical mechanism.

## Dependencies
Depends on Slices 01-04 (the mechanism proven twice already: directly, then generalized once).

## Effort Estimate
0.5 day.

## Reference Class
Mirrors ADR-032's own "third schema-identical, independently-stored table" precedent exactly — the last of three applications of the same, now twice-validated, pattern.
