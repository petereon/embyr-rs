# Slice 01: Enumerate Subcollections Against an Agent-Mode Project (Walking Skeleton)

**Story**: US-01 | **Release**: 1 | **Estimate**: 1.5 days

## Goal

A new unary RPC on the agent proto returns the correct, distinct, paginated set of child collection IDs under a parent document.

## IN Scope

- New unary RPC + messages on `storage_agent.proto`
- New distinct-child-collection query primitive (agent-local or shared with non-agent `ListCollectionIds`, per DESIGN)
- Pagination reusing `build_collection_path`/`encode_page_token`/`decode_page_token`
- Sibling-parent scoping correctness
- New `AgentBackendAdapter` method

## OUT Scope

- Cross-version graceful degradation mechanism (feature-level escalation, resolved once across 3 sibling features)

## Learning Hypothesis

Disproves: the "distinct child collection IDs" query cannot be expressed against this codebase's existing document-storage schema without a new query primitive incompatible with the current `PostgresBackendAdapter` shape.
Confirms (if it succeeds): the schema already supports this query class cheaply, no schema migration needed.

## Acceptance Criteria

- [ ] Returns exactly the correct, distinct set of child collection IDs for a document with subcollections
- [ ] Returns an empty list (not an error) for a document with no subcollections
- [ ] Pagination returns the complete set with no duplicates or gaps
- [ ] Subcollections are correctly scoped to their own parent document
- [ ] Exercised against a real `embyr-agent` binary and real Postgres

## Dependencies

None (first and only slice).

## Effort Estimate

1.5 days.

## Reference Class

`firestore-list-rpcs` US-02 (non-agent `ListCollectionIds`) — same query class, different proto family and transport.
