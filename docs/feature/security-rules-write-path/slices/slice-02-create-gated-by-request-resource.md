# Slice 02: A Create Is Gated by the Write Rule Using Only the Proposed New Document

**Story**: US-02 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 1.5 days

## Goal
Make `CreateDocument` evaluate the collection's write rule against the proposed new document (`request.resource.data.<field>`), with the pre-existing `resource.data.<field>` operand naturally failing closed since no document exists yet.

## IN Scope
- Extend `embyr_core::access_control`'s `Operand` enum with `RequestResourceField(String)`; extend `parse_condition`'s tokenizer to recognize `request.resource.data.<field>` as distinct from `resource.data.<field>`.
- Extend `evaluate()`'s comparison semantics to resolve `RequestResourceField` values from a caller-supplied field map — reusing the exact `FieldMissing`/fail-closed short-circuit mechanism already proven for `resource.data.<field>`.
- Wire `handle_create_document` to: look up the collection's write rule (short-circuit to unmodified behavior if none, per Slice 06's guardrail), build an empty `resource_fields` map (no document exists yet) and a `request_resource_fields` map from the proposed document, evaluate, and gate the create.

## OUT Scope
- Identity resolution on the create call itself (Slice 05 wires `attach_client_identity_if_present` into all three write handlers together — this slice may stub/assume `None` auth for its own scenarios or land after Slice 05; sequencing note for DESIGN).
- Update/delete gating (Slices 03/04).

## Learning Hypothesis
Disproves: a write condition referencing only `request.resource` cannot be evaluated for a real `CreateDocument` call without the existing fail-closed-on-missing-field mechanism (ADR-027) needing a new, second mechanism to handle the "no document exists yet" case.
Confirms (if it passes): the SAME fail-closed mechanism that already handles "field present on the wrong document" (`security-rules` AC-17-09) also correctly handles "field referenced on a document that doesn't exist yet" — one mechanism, two use cases, zero new special-casing.

## Acceptance Criteria
- AC-17-26, AC-17-27, AC-17-28, AC-17-29.

## Dependencies
- Slice 01 (write-condition storage/admin-API must exist to define the rules this slice evaluates).

## Production-Data Taste Test
Real registered write rule on `trailmark-prod`'s `journal_entries`, real Maria/Dana signed-in sessions, real `CreateDocument` calls with matching and mismatched `owner_id` payloads.

## Reference Class
`security-rules`' own Slice 02 (US-02, signed-in read gating) — same "prove the riskiest new assumption against real state" discipline, applied to the simpler of the two new grammar cases (create needs only one new operand family, no fetch).
