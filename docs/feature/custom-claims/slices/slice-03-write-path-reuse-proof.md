# Slice 03: The Identical Mechanism Gates Writes, For Free

**Story**: US-03 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 0.5 day

## Goal
Prove that the SAME `Operand::AuthTokenClaim` extension shipped in Slice 02 also correctly gates `write_access_rules`-based `CreateDocument`/`UpdateDocument`/`DeleteDocument` enforcement, with zero additional production code.

## IN Scope
- A `write_access_rules` condition referencing `request.auth.token.<claim>`, defined independently from the read rule (existing read/write-rule-independence convention, unchanged).
- Acceptance-test proof that `handle_update_document`'s existing, unmodified `evaluate()` call site correctly resolves the new operand.

## OUT Scope
- Any new production code beyond what Slice 02 already shipped — if this slice requires new code to pass, the hypothesis is disproven and DESIGN must revisit the write-path composition, not force a workaround here.
- Query-path (US-05) or string-literal (US-06) concerns.

## Learning Hypothesis
**Disproves if it fails**: Resolution 4's central hypothesis — that `AuthContext`'s uniform construction across all already-shipped call sites means a single, small `access_control` change propagates automatically. If write-path support requires ANY new code beyond Slice 02's own change, this feature's own "2 bounded contexts, 7 stories" scope claim is wrong and DESIGN must re-scope.

**Confirms if it succeeds**: the single most important structural claim in this feature's whole DISCUSS.

## Acceptance Criteria
- AC-17-144: The identical `request.auth.token.<claim>` operand, used in a `write_access_rules` condition, correctly gates Create/Update/Delete with zero additional production code.
- AC-17-145: A GetDocument claim-based rule and a write-path claim-based rule for the same collection are independently authored and evaluated.

## Dependencies
Depends on Slice 02 (reuses its exact code change).

## Production-Data Taste Test
Real `write_access_rules` condition on `flagged_content` using the identical claim operand, real Priya `updateDoc()` call, real System DB rule state — zero new production code touched.

## Effort Estimate
0.5 day. This is primarily a proof/verification slice, not new implementation — mirrors `security-rules`'s own US-04 ("primarily a proof obligation... not new production logic").

## Pre-Slice SPIKE
Not needed.
