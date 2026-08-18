# Slice 03: An Update Is Gated by the Write Rule Using Both the Existing and Proposed Document

**Story**: US-03 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 2 days

## Goal
Make `UpdateDocument` fetch the document's pre-write state and evaluate the write rule against BOTH `resource.data.<field>` (existing) and `request.resource.data.<field>` (proposed) in the same condition — the two-value comparison OQ-SR-02 named, and this feature's single riskiest mechanism.

## IN Scope
- Add a document fetch to `handle_update_document`, analogous to `GetDocument`'s existing fetch-then-decide pattern, to obtain `resource_fields` before the write executes.
- Evaluate the collection's write rule with both `resource_fields` (pre-write) and `request_resource_fields` (proposed, from the update body) populated.
- Gate the update: `Deny` → `PermissionDenied`, identical response regardless of document existence (existence non-leakage, extended from `security-rules`' AC-17-10 precedent). `Allow` → proceed to the existing `adapter.update_document` call, unchanged.
- Prove the immutable-field pattern: a condition comparing `request.resource.data.owner_id == resource.data.owner_id` correctly denies an owner-changing update while allowing a preserving one.

## OUT Scope
- Create/delete gating (Slices 02/04 — this slice's fetch-before-decide pattern is reused by Slice 04, not duplicated).
- Simulation extension (Slice 07).

## Learning Hypothesis
Disproves: a condition cannot meaningfully compare a document's pre-write state against its proposed new state without either fetching the pre-write document via a new, separate mechanism from `GetDocument`'s own fetch-then-decide pattern, or requiring more grammar investment than one new operand family provides.
Confirms (if it passes): the existing `Condition`/`Operand` AST's boolean combinators (`&&`/`||`/`!`/`==`/`!=`) are sufficient to express old-vs-new comparisons once both operand families exist — no new comparison operator, no new AST node type needed.

## Acceptance Criteria
- AC-17-30, AC-17-31, AC-17-32, AC-17-33, AC-17-34.

## Dependencies
- Slice 02 (the `RequestResourceField` operand and evaluator support must exist first).

## Production-Data Taste Test
Real owner-preserving and owner-changing update payloads against real `journal_entries/maria-trip-042`, real Maria/Dana signed-in sessions, real pre-write document fetch.

## Reference Class
No direct precedent in this codebase — this is the genuinely novel mechanism this feature exists to prove (the two-value comparison `security-rules`' own Resolution 1 confidence/escalation note flagged as unresolved). Sequenced first among the three operation-shape slices (see Prioritization in `feature-delta.md`) precisely because it is the riskiest.
