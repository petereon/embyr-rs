# Slice 04: The Identical History Mechanism Protects Write Rules Too

**Story**: US-04 | **Release**: 1 | **Estimate**: 0.5 day

## Goal
The identical history-capture-and-retrieve-and-restore mechanism (Slices 01-03) generalizes to `write_access_rules`, structurally independent of `access_rules`' own history.

## IN Scope
- A second, independently-stored history table (recommended `write_access_rule_history`), populated alongside the existing, unmodified `upsert_write_access_rule` call.
- Retrieval and restore for write-rule history, mirroring Slices 02/03 exactly.
- Proof that a collection's read-rule history and write-rule history never mix or cross-reference.

## OUT Scope
- `group_access_rules` (Slice 05).
- Any change to `write_access_rules`' own existing write path (ADR-030), or to `access_rules`/`access_rule_history` from Slices 01-03.

## Learning Hypothesis
**Disproves if it fails**: the identical capture-and-retrieve mechanism does NOT extend to `write_access_rules` without a table-specific complication — Resolution 1's central "3 independently-stored, schema-identical tables" hypothesis is disproven for at least this table.
**Confirms if it succeeds**: the mechanism is genuinely a reusable pattern, not a one-off built for `access_rules` alone.

## Acceptance Criteria
- AC-17-168: `write_access_rules` redefinitions are captured, attributed, retrievable — identical mechanism.
- AC-17-169: Read-rule history and write-rule history are structurally independent.
- AC-17-170: A write rule can be restored to any prior entry via the identical mechanism.

## Dependencies
Depends on Slices 01-03 (the mechanism being generalized).

## Effort Estimate
0.5 day.

## Reference Class
Mirrors ADR-030's own "new, independent table, zero shared column, zero modification to the sibling table's write path" precedent exactly — this slice is the history-mechanism's own version of that same discipline.
