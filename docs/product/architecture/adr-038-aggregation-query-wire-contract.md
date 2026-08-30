# ADR-038: Aggregation Query Wire Contract — RPC Shape, Message Shapes, v1 Restrictions

## Status

Accepted

## Context

`aggregation-queries` (JOB-01) adds `RunAggregationQuery` to the client-facing `google.firestore.v1.Firestore` service. One decision is LOCKED and not reopened here: **OQ-AGG-01, the wire shape, confirmed by the orchestrator 2026-08-30** — `rpc RunAggregationQuery(RunAggregationQueryRequest) returns (stream RunAggregationQueryResponse)`, server-streaming, matching real Firestore's own public proto and `RunQuery`'s own shape in this codebase.

What DISCUSS left to DESIGN: the exact message shapes (`StructuredAggregationQuery`, `Aggregation`, `RunAggregationQueryRequest`, `RunAggregationQueryResponse`), field numbers, alias-synthesis mechanics, and v1's own server-side restrictions (single aggregation per request, no `Count.up_to` capping).

`docs/SPEC.md §RunAggregationQuery`/`§Aggregation` already documents a high-level contract (`structured_aggregation_query` input, `RunAggregationQueryResponse{result: {aggregate_fields: {alias → Value}}, read_time}` output, `COUNT`/`SUM`/`AVG`, alias auto-synthesis, `InvalidArgument`/`Unimplemented` error taxonomy) written BEFORE this feature existed — cross-checked against this architect's own general knowledge of real Firestore's public `StructuredAggregationQuery`/`Aggregation` proto shape (a `StructuredAggregationQuery` wraps a `StructuredQuery` plus a `repeated Aggregation`, each a oneof `count`/`sum`/`avg` with an `alias` string defaulting to `field_0`/`field_1`/... if unset; the response carries a `google.firestore.v1.Value`-typed `aggregate_fields` map, not a generic `google.protobuf.Struct`). **No divergence found** — SPEC.md's own `§Server-Streaming RPCs` section already groups `RunAggregationQuery` alongside `RunQuery`/`BatchGetDocuments`, resolving the apparent tension in its "exactly one `RunAggregationQueryResponse`" phrasing: that language describes message *cardinality within the stream* (one data message, then the stream closes), not a unary transport — fully consistent with the orchestrator's confirmed server-streaming shape, not a second, independent confirmation of it.

## Decision Drivers

1. **Wire-identical SDK compatibility is JOB-01's entire value proposition** — for a server-streaming RPC, real SDK client stubs decode responses using THEIR OWN compiled proto definitions; the protobuf **field numbers**, not just the RPC shape, must match real Firestore's for a genuine `@google-cloud/firestore`/`firebase-admin` client to decode a response correctly. Getting the RPC shape right (OQ-AGG-01) but the field numbers wrong would silently reproduce the same failure mode this feature exists to prevent.
2. SPEC.md's own already-documented contract is a strong prior, not something to reinvent, but must be cross-checked against real Firestore's actual shape (not just internally-consistent with itself).
3. Reuse the existing `tonic-build` auto-compile pipeline (`crates/embyr-proto/build.rs` already globs `firestore.proto`; confirmed no manual generated-file step exists) — zero new build machinery.
4. v1 scope is single-aggregation-per-request (DISCUSS System Constraints, not reopened) — the WIRE shape must still support `repeated Aggregation` from day one to avoid a second proto-breaking change later.

## Decision

### Proto additions to `proto/google/firestore/v1/firestore.proto`

```proto
service Firestore {
  ...
  // Runs an aggregation query.
  rpc RunAggregationQuery(RunAggregationQueryRequest) returns (stream RunAggregationQueryResponse);
}

// The request for [Firestore.RunAggregationQuery].
message RunAggregationQueryRequest {
  string parent = 1;

  oneof query_type {
    StructuredAggregationQuery structured_aggregation_query = 2;
  }

  oneof consistency_selector {
    bytes transaction = 4;
    TransactionOptions new_transaction = 6;
    google.protobuf.Timestamp read_time = 7;
  }
}

// Firestore query for running an aggregation over a StructuredQuery.
message StructuredAggregationQuery {
  oneof query_type {
    StructuredQuery structured_query = 1;
  }

  repeated Aggregation aggregations = 3;

  message Aggregation {
    message Count {
      google.protobuf.Int64Value up_to = 1;
    }
    message Sum {
      StructuredQuery.FieldReference field = 1;
    }
    message Avg {
      StructuredQuery.FieldReference field = 1;
    }
    oneof operator {
      Count count = 1;
      Sum sum = 2;
      Avg avg = 3;
    }
    // Optional. Alias for this aggregation. If not set, the server
    // synthesizes `field_0`, `field_1`, ... in request order.
    string alias = 7;
  }
}

// The streamed response for [Firestore.RunAggregationQuery].
message RunAggregationQueryResponse {
  AggregationResult result = 1;
  bytes transaction = 2;
  google.protobuf.Timestamp read_time = 3;
}

message AggregationResult {
  map<string, Value> aggregate_fields = 2;
}
```

Field numbers mirror this architect's best available knowledge of the real public `googleapis/googleapis` proto — the same source family this codebase's already-vendored `firestore.proto` matches field-for-field elsewhere. **Residual, non-blocking risk, named explicitly**: this sandbox has no `WebFetch`/network access to byte-verify these exact numbers against the live source. Recommend a lightweight confirmation pass in DELIVER (decode a captured real Firestore Admin SDK aggregation request/response, or diff against an updated vendor copy of `googleapis` if the delivering environment has network access) before treating wire-compat as fully proven. This is NOT a re-litigation of OQ-AGG-01 — the RPC shape is settled; this is only the internal field-numbering detail within that settled shape.

### v1 server-side restrictions (not wire-shape — enforced in `grpc/handler.rs`, not the proto)

- `aggregations.len() != 1` → `Status::unimplemented("multiple aggregations per request are not yet supported")`.
- `Aggregation.Count.up_to` set → `Status::unimplemented("count up_to limiting is not yet supported")`.
- `alias` empty → server synthesizes `"field_0"` (always index 0, since exactly one aggregation is permitted in v1).
- Response stream carries exactly one `RunAggregationQueryResponse` message, then closes (no `done`-marker continuation message — unlike `RunQuery`, `RunAggregationQueryResponse` carries no `continuation_selector` in the real proto; a second, terminal message is neither required nor present).
- `read_time` is left unset in v1 — mirrors `handle_run_query`'s own existing, pre-feature behavior (`RunQueryResponse.read_time` is never populated for document results either; confirmed by reading the current handler). Named explicitly so this is not mistaken for an oversight introduced by this feature.
- `transaction`/`new_transaction`/`read_time` consistency-selector fields are present on the wire for schema parity with `RunQueryRequest`, but v1's handler never threads them to the adapter (`transaction_id` is always `None`) — mirrors `handle_run_query`'s own existing precedent (`adapter.run_query(&collection, &domain_query, None)` is already unconditionally `None` today despite the proto supporting `transaction`/`new_transaction`).

## Alternatives Considered

1. **Unary RPC** (`returns (RunAggregationQueryResponse)`, no `stream`) — Rejected. Directly contradicts OQ-AGG-01, which is locked and orchestrator-confirmed.
2. **Reuse `StructuredQuery`'s own `select`/`limit` fields as an ersatz aggregation signal**, avoiding a new message family — Rejected. Would silently diverge from real Firestore's dedicated `StructuredAggregationQuery` message; a real SDK client would never construct a request shaped this way, breaking wire compatibility, the entire point of this feature.
3. **`google.protobuf.Struct` for `aggregate_fields`** (matches this feature's own commissioning prompt's informal description) — Rejected once cross-checked: SPEC.md documents a `Value`-keyed map, and real Firestore's actual wire type is `google.firestore.v1.Value` (the SAME type already used for document fields in this codebase, with an existing `field_value_to_proto`/`fields_to_proto` conversion pair in `embyr-server/src/encoding/firestore_proto.rs`), not the generic `protobuf.Struct`. Using the real `Value` type is both more correct and strictly less new code (zero new conversion logic needed for scalar aggregate results).

## Consequences

**Positive**: zero disruption to the other 10 existing RPCs; zero new build-pipeline step (`build.rs` already globs `firestore.proto`); the message shape is forward-compatible with a later multi-aggregation-per-request slice (no second proto-breaking change).

**Negative**: field-number verification residual risk (named above, non-blocking). v1's single-aggregation limit means a real SDK client combining `count()` and `sum()` in one call gets `Unimplemented` — a real, if narrow, compatibility gap, already named in DISCUSS's own Out of Scope as a future wire-compatible (non-breaking) follow-up slice.
