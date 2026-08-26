# ADR-033: Listen Compliance Composition and Collection-Scoping

## Status

Accepted

## Context

`security-rules` (Epic 2a, ADR-027/028/029) gave Alex per-collection read-path
enforcement on `GetDocument`. `security-rules-write-path` (Epic 2b, ADR-030)
extended it to `CreateDocument`/`UpdateDocument`/`DeleteDocument`.
`security-rules-query-path` (Epic 2c, ADR-031) closed the `RunQuery` bypass
via `check_query_compliance()`. `security-rules-collection-group-rules`
(Epic 2d... numbering note: the epic id sequence in `feature-delta.md`
calls this feature "Epic 2d" and collection-group-rules "Epic 2c" sibling —
see that feature's own delta for the exact numbering; irrelevant to this
ADR's own content) closed the collection-group `RunQuery` bypass via a new,
disjoint `group_access_rules` table.

None of the four touched `Listen` (`onSnapshot()`). DISCUSS
(`docs/feature/security-rules-realtime/feature-delta.md`, Resolution 1 +
Resolution 2, LOCKED) confirmed, by direct code read, that `Listen`'s
enforcement gap is structurally different from — and larger than —
`RunQuery`'s own, because `Listen`'s underlying delivery mechanism (BC-3,
project-wide Postgres NOTIFY fan-out) has two pre-existing, non-security
-rules-caused correctness gaps `RunQuery`'s request/response model never had
occasion to develop:

- **Finding 2**: `handle_add_target` hardcodes `domain_query.filter: None` —
  the client's own `StructuredQuery.where_` clause is silently dropped, never
  extracted, never consulted. Every Listen subscription's initial snapshot
  returns the entire, unfiltered collection.
- **Finding 5**: `ListenRegistry::fan_out()` delivers every event on a
  project's NOTIFY channel to every subscriber on that channel, with zero
  filtering by collection — a subscriber to `journal_entries` also receives
  `trip_photos`/`app_config`/any other collection's live writes in the same
  project.

DISCUSS locked both findings as IN SCOPE for this feature (Handoff Package
flag 3, orchestrator-confirmed per the calling context of this DESIGN pass —
not re-litigated here) because they are CAUSALLY, not merely thematically,
prerequisite to this feature's own enforcement mechanism meaning anything:
`check_query_compliance()` cannot decide compliance against a filter that is
always `None` (Finding 2 must precede subscribe-time admission), and
per-event enforcement cannot mean anything if the delivered event might
belong to an entirely different, unrelated collection (Finding 5 must
precede per-event re-check). This ADR implements DISCUSS's locked
resolution, not re-litigates it:

1. **A composed mechanism, not a single reused function** (Resolution 1,
   Option C): `check_query_compliance()` (ADR-031, UNCHANGED) gates the
   initial snapshot at subscribe time; `evaluate()` (ADR-027/030, UNCHANGED)
   re-checks each individually-delivered document-change event; PLUS a new,
   BC-3-internal collection-scoping filter with no analog in any prior
   epic (Finding 5's own fix). Subscribe-time-only enforcement (Option A) is
   explicitly rejected — insufficient by direct construction, not merely a
   smaller version of the correct answer (see DISCUSS Resolution 1). A
   wholly-invented third compliance function (Option B) is also rejected —
   unjustified given two proven mechanisms already answer Listen's two
   sub-problems.
2. **Both subscribe-time admission AND per-event re-check are required**
   (Resolution 2, Option C) — a document's rule-governing field can change
   after subscribe-time admission (the task's own owner-field-change
   hypothesis, confirmed structurally reachable); a subscribe-time-only
   check would create a false sense of security. Real Firestore itself
   re-evaluates rules on every listener delivery, not merely at listener
   creation.
3. **`check_query_compliance()`/`QueryComplianceOutcome`/
   `UnsatisfiedConjunct` (ADR-031) and `evaluate()` (ADR-027/030) are reused
   completely UNCHANGED** — no new decidable shape, no new evaluator
   branch, zero modification to `embyr_core::access_control` anywhere in
   this feature. Confirmed by full-file read
   (`crates/embyr-core/src/access_control/mod.rs`, 1496 lines): this
   feature is the first in the initiative where **BC-4 Access Control gains
   zero new storage, zero new type, and zero new function** — every
   genuinely new artifact belongs to BC-3 Real-Time Delivery instead.
4. **This feature reads `access_rules` only** — never `write_access_rules`
   or `group_access_rules`. `Listen`'s v1 enforcement surface is non-group,
   query-shaped targets only (Finding 1: `TargetType::Documents` unhandled;
   Finding 3: `all_descendants` hardcoded `false`) — both explicitly out of
   scope (§ Out of Scope, mirroring `security-rules-collection-group-rules`'
   own OQ-SRCG-03 precedent for an analogous pre-existing gap).
5. **`GetDocument`, every write handler, and `RunQuery` (group and
   non-group) receive ZERO code changes** — confirmed by direct code read.
   This feature's entire footprint is new call sites inside
   `crates/embyr-server/src/realtime/*` plus two additive, visibility-only
   changes in `crates/embyr-server/src/grpc/handler.rs`.

This ADR combines the collection-scoping, composition, and delete
-non-leakage decisions into one ADR, mirroring ADR-030/031/032's own
"smaller, bounded decision surface" precedent.

## Decision Drivers

1. **No false-allow, at subscribe time or at any point during a
   subscription's lifetime** (US-01 through US-05) — this feature's own
   North Star KPI. A subscribe-time-only check is a structurally
   insufficient answer (Decision Driver of Resolution 2).
2. **The collection-scoping fix is structural, not rule-dependent, and is
   this feature's single highest-consequence design risk** — it must hold
   for EVERY Listen subscription, including collections with no rule
   defined at all (worse than a false-allow on a ruled collection, since it
   currently affects every collection in every project). Designated
   mutation-testing surface (per-feature strategy, `CLAUDE.md`), alongside
   the per-event fail-closed correctness (US-04).
3. **Zero added I/O beyond what `fetch_event()` already performs** — Finding
   6 (a fully-fetched `FirestoreDocument` already exists in memory, before
   fan-out, for every `Changed` event) is the load-bearing structural fact
   this ADR's entire per-event mechanism depends on. This ADR must confirm,
   not merely assert, that the claim holds against the real code shape (see
   § Decision — Per-Event Composition, "I/O accounting").
4. **Structural, not conventional, independence from `GetDocument`, writes,
   and `RunQuery`** (US-06/07) — this feature's new call sites live
   entirely inside `crates/embyr-server/src/realtime/*`; none of the five
   existing handlers is modified.
5. **Simplest solution first** (Principle 8) — no new crate, no new
   dependency, no new bounded context, no new table, no new migration, no
   new admin route. Confirmed below (§ Reuse Analysis, `feature-delta.md`):
   this feature's CREATE-NEW footprint is the smallest of the five epics in
   this initiative, despite having the largest user-story footprint.
6. **`ListenRegistry`/`PostgresNotifyListener`'s channel topology is not
   redesigned** — DISCUSS's own OQ-SRRT-05 named a per-collection-channel
   redesign as a candidate future optimization, not required now. This ADR
   confirms, against the real code shape, that a delivery-time filter is
   sufficient for every one of this feature's own domain examples.
7. **Existence non-leakage extends to delete events** (US-05) — mirrors
   `security-rules`/`security-rules-write-path`'s own established
   discipline (AC-17-10/34/38), now extended to a fan-out/delivery context
   none of the four prior epics had.

## Decision — Collection-Scoping (BC-3, Finding 5's fix)

**Mechanism: a delivery-time filter inside `handle_add_target`'s own
event-consumption loop (the consuming side), not a `ListenRegistry`
/`PostgresNotifyListener` channel-topology redesign.**

Each subscriber's own loop compares the delivered `ListenEvent`'s own
`collection_path` against `collection.collection_path` — the subscriber's
own subscribed collection, already a bound local in `handle_add_target`
today — and drops (never forwards) any event whose collection does not
match, for BOTH `ListenEvent::Changed` and `ListenEvent::Removed`. This
check runs FIRST, unconditionally, before any rule-dependent branch — it
must hold identically regardless of whether either collection involved has
any access rule defined (AC-17-108).

```rust
event = event_rx.recv() => {
    match event {
        Some(ListenEvent::Changed(doc)) => {
            // US-01 (Finding 5 fix) — FIRST, unconditional, rule-independent.
            if doc.path.collection_path != collection.collection_path {
                continue;
            }
            // US-04 per-event recheck — see § Decision — Per-Event Composition.
            ...
        }
        Some(ListenEvent::Removed { path, fields }) => {
            if path.collection_path != collection.collection_path {
                continue;
            }
            // US-05 delete non-leakage — see § Decision — Delete Non-Leakage.
            ...
        }
        None => break,
    }
}
```

**Why the consuming side, not `ListenRegistry::fan_out()`**: `fan_out()` and
`SubscriberEntry` are generic, project-scoped infrastructure — they hold no
per-subscriber collection metadata today, and adding it would widen
`register()`'s signature and `SubscriberEntry`'s shape, a structural change
to shared infrastructure every subscriber (including future non-Listen
consumers of the same registry, if any) would inherit. Filtering on the
consuming side requires ZERO change to `ListenRegistry`/`SubscriberEntry` —
`fan_out()` still enqueues every event onto every project subscriber's
channel exactly as it does today (unchanged bandwidth on the internal
`mpsc` channel — see § Consequences, Negative), but each subscriber's own
loop discards events for the wrong collection before they are ever
serialized into a proto response or written to the gRPC stream. This is the
delivery-time filter DISCUSS's own OQ-SRRT-05 named as the lower-effort,
lower-risk option, verified here against the real code shape: `ListenEvent`
already carries `collection_path` on both variants (`FirestoreDocument.path
.collection_path` for `Changed`, `DocumentPath.collection_path` for
`Removed`), so the comparison is a zero-allocation string comparison against
values already resident in memory — no new type, no new field, no new I/O.

**Considered and rejected: per-collection NOTIFY channels** (the channel
-topology redesign OQ-SRRT-05 named). Rejected for this feature's own scope
— it would require a new channel-naming scheme (today: `dc_<hex16>
(project_id)`, project-scoped only), coordinated changes to
`embyr-pg-storage`'s own NOTIFY-triggering side (outside this feature's
touched files entirely), and `PostgresNotifyListener::start`'s one
-listener-per-project lifecycle assumption — a materially larger change for
a benefit (reduced internal `mpsc` fan-out volume under high
multi-collection write load) no domain example in this feature requires.
Left as a candidate future optimization, re-evaluated only if real
production fan-out volume evidences it (OQ-SRRT-05, carried).

## Decision — Subscribe-Time Composition (US-02/US-03)

### Exact insertion point (`handle_listen`, `crates/embyr-server/src/grpc/handler.rs`)

`handle_listen` gains ONE new call, mirroring `handle_run_query`'s own
`attach_client_identity_if_present` placement exactly — the function itself
is unchanged, this is a new consumer of its existing return value — inserted
after the existing `authenticate()`/suspend check and the existing
`PostgresNotifyListener`-provisioning block, before the `tokio::spawn` that
runs `handle_add_target`:

```rust
let verified_identity = self
    .attach_client_identity_if_present(&request, &project_id)
    .await;

// ...existing resume_token extraction, channel setup...

let system_db = Arc::clone(&self.system_db);

tokio::spawn(async move {
    if let Err(status) = crate::realtime::listen_handler::handle_add_target(
        &first_msg,
        &adapter,
        &system_db,
        verified_identity,
        &tx,
        keepalive,
        registry,
        &channel,
        resume_token,
    )
    .await
    {
        let _ = tx.send(Err(status)).await;
    }
    while (in_stream.next().await).is_some() {}
});
```

`Arc::clone(&self.system_db)` mirrors how `adapter`/`channel`/`resume_token`
are already moved into this same spawn today — `FirestoreService.system_db:
Arc<SystemDb>` (confirmed, `grpc/handler.rs:62`) is trivially cloneable, the
identical pattern `admin::state`/`admin::router` already use throughout.

### Exact insertion point (`handle_add_target`, `crates/embyr-server/src/realtime/listen_handler.rs`)

`handle_add_target` gains two new parameters (`system_db: &Arc<SystemDb>`,
`verified_identity: Option<VerifiedEndUserIdentity>`) and its query
-extraction step is widened from "collection id only" (Finding 2's bug) to
the full `StructuredQuery`, reusing `translate_filter()` — the EXACT
function `handle_run_query` already uses — for the `where_` clause:

```rust
let (project_id, collection_id, filter) = match &add_target.target_type {
    Some(TargetType::Query(qt)) => {
        let sq = structured_query_from_query_target(qt)?;
        let collection_id = collection_id_from_structured_query(sq)?;
        let project_id = project_id_from_parent(&qt.parent)?;
        // US-02 (Finding 2 fix): reuses translate_filter() unchanged — the
        // SAME function handle_run_query already uses. `all_descendants`
        // remains hardcoded `false`, deliberately — Finding 3/OQ-SRRT-04
        // (collection-group Listen) stays out of this feature's scope.
        let filter = sq
            .r#where
            .as_ref()
            .and_then(crate::grpc::handler::translate_filter)
            .transpose()
            .map_err(Status::invalid_argument)?;
        (project_id, collection_id, filter)
    }
    _ => return Err(Status::invalid_argument("target must have Query target type")),
};
```

`translate_filter` and `query_compliance_rejection` (both currently private
`fn` in `grpc/handler.rs`, same file as `handle_run_query`) are widened to
`pub(crate) fn` — a visibility-only change, zero behavior change — so
`realtime::listen_handler` can call them without a second,
independently-maintained copy.

Immediately after `domain_query`/`collection` are built (US-02), and BEFORE
the existing `adapter.run_query()` initial-snapshot call, `handle_add_target`
gains the subscribe-time gate — the IDENTICAL composition shape
`handle_run_query`'s own non-group arm already uses (ADR-031), applied at a
new call site inside an async streaming handler rather than a
request/response one:

```rust
let auth_ctx = verified_identity
    .as_ref()
    .map(|v| AuthContext { uid: v.end_user_id.clone() });

let rule_row = system_db
    .get_access_rule(&project_id, &collection.collection_path)
    .await
    .map_err(Status::internal)?;

// `condition` is retained for the REST OF THIS FUNCTION's lifetime — see
// § Decision — Per-Event Composition for why.
let condition: Option<Condition> = match rule_row {
    None => None, // US-06: no rule -> unrestricted, both subscribe-time and per-event.
    Some(row) => {
        let condition = parse_condition(&row.condition_source).map_err(|e| {
            Status::internal(format!("stored access rule failed to re-parse: {e:?}"))
        })?;
        match check_query_compliance(&condition, filter.as_ref(), auth_ctx.as_ref()) {
            QueryComplianceOutcome::Admitted => {}
            // AC-17-116: SAME query_compliance_rejection() RunQuery already
            // uses -> Status::permission_denied with the SAME [REASON_CODE]
            // convention -> distinguishable from authenticate()'s own
            // Status::unauthenticated/permission_denied("project is
            // suspended") and from Status::internal (genuine server error).
            outcome => return Err(query_compliance_rejection(&outcome)),
        }
        Some(condition)
    }
};
```

This runs strictly BEFORE `registry.register()`/`adapter.run_query()` (the
existing initial-snapshot call) — a non-compliant subscription is rejected
outright, before any row is read for the initial snapshot (AC-17-114),
mirroring `handle_run_query`'s own compliance-before-execution ordering
(ADR-031 § OQ-SRQ-03 Resolution).

### `handle_add_target`'s error type: `Result<(), String>` → `Result<(), Status>`

A mechanical signature change — every existing `String`-producing error site
(`project_id_from_parent`, `ProjectId::new`, `adapter.run_query()`,
`"channel closed"`, `"target must have Query target type"`) is wrapped via
`.map_err(Status::internal)` or constructed directly as a `Status`, mirroring
how `handle_run_query`/`handle_get_document` already return `Result<_,
Status>` throughout. `handle_listen`'s own spawn site simplifies from
`Status::internal(e)` (always) to forwarding whatever `Status` `
handle_add_target` returns — this is the mechanism that makes AC-17-116's
distinguishability requirement possible: without it, a compliance rejection
and a genuine internal error would be indistinguishable at the gRPC status
-code level, which `Status::internal` alone cannot express.

## Decision — Per-Event Composition (US-04)

### The parsed `Condition` is retained for the subscription's entire lifetime

`condition: Option<Condition>` (built once, above, at subscribe time) is a
local binding that lives for the ENTIRE duration of `handle_add_target`'s
own `loop { tokio::select! { ... } }` — reused, unmodified, for every
subsequent per-event `evaluate()` call, not re-parsed per event and not
re-fetched from `access_rules` per event. `auth_ctx` is likewise built once
and reused. This is the concrete mechanism this ADR uses to go further than
Decision Driver 3's own "zero added I/O" claim: the per-event re-check adds
**zero additional Postgres queries of any kind** (not merely "reuses the
already-fetched document") — `get_access_rule` runs exactly once per
subscription (identical cost profile to `handle_run_query`'s own one-time
lookup), and `parse_condition` runs exactly once per subscription, not once
per delivered event.

### `Changed` arm

```rust
Some(ListenEvent::Changed(doc)) => {
    if doc.path.collection_path != collection.collection_path {
        continue; // US-01
    }
    if let Some(condition) = &condition {
        let empty_fields: BTreeMap<String, FieldValue> = BTreeMap::new();
        // ADR-030's own empty-map convention: Listen has no "proposed new
        // document" concept, mirrors handle_get_document exactly.
        if evaluate(condition, auth_ctx.as_ref(), &doc.fields, &empty_fields)
            == EvaluationOutcome::Deny
        {
            continue; // US-04: withheld, never sent, never a crash.
        }
    }
    // ...existing DocumentChange send logic, unchanged...
}
```

### I/O accounting (Decision Driver 3, confirmed against the real code shape)

`PostgresNotifyListener::start`'s background task calls `fetch_event()` —
already a real SQL fetch — on EVERY NOTIFY, BEFORE `registry.fan_out()` is
ever called (Finding 6, confirmed by direct code read,
`postgres_notify_listener.rs:57-75`). This happens ONCE per NOTIFY, shared
across every subscriber on that project's channel via `fan_out()`'s existing
`entry.tx.try_send(event.clone())` (`ListenEvent` already derives `Clone`).
Each subscriber's own `evaluate()` call inside its own `handle_add_target`
loop therefore operates on an ALREADY-in-memory, ALREADY-cloned
`FirestoreDocument` — the per-subscriber marginal cost of US-04's own check
is pure CPU (`evaluate()`'s own deterministic tree walk), zero I/O,
confirmed exactly as AC-17-121 requires and Decision Driver 3 demands.

## Decision — Delete Non-Leakage (US-05, resolves Handoff Package flag 7 / OQ-SRRT-02)

**Mechanism: widen `fetch_event()`'s EXISTING single SQL query's predicate —
do not add a second query, and do not query per-subscriber.**

`fetch_event()` (`crates/embyr-server/src/adapters/postgres_notify_listener.rs`)
today runs:

```sql
SELECT fields, version, create_time, update_time
FROM documents
WHERE project_id = $1 AND collection_path = $2 AND document_id = $3
  AND NOT deleted
```

and treats "no row found" (either genuinely absent OR soft-deleted, since
`documents` never issues a hard `DELETE` — Finding 7, confirmed
`backend_adapter.rs:508,826`) as `ListenEvent::Removed(DocumentPath)`, with
no field data. This ADR drops `AND NOT deleted` and additionally selects the
`deleted` column, branching in Rust instead of in SQL:

```sql
SELECT fields, version, create_time, update_time, deleted
FROM documents
WHERE project_id = $1 AND collection_path = $2 AND document_id = $3
```

```rust
match row_opt {
    Ok(Some(row)) if !row.deleted_column() => ListenEvent::Changed(/* unchanged construction */),
    Ok(Some(row)) /* row.deleted_column() == true */ => ListenEvent::Removed {
        path: doc_path,
        fields: /* the SAME json_to_fields(&fields_json) conversion Changed already uses */,
    },
    _ /* row genuinely absent — defensive, should not occur for a NOTIFY-triggered event */ => {
        ListenEvent::Removed { path: doc_path, fields: BTreeMap::new() }
    }
}
```

`ListenEvent::Removed(DocumentPath)` (a tuple variant) becomes
`ListenEvent::Removed { path: DocumentPath, fields: BTreeMap<String,
FieldValue> }` — a struct variant carrying the pre-deletion field snapshot.
`ListenRegistry`'s own `Clone`/`Debug` derives on `ListenEvent` require no
change (`BTreeMap<String, FieldValue>` is already `Clone`/`Debug`, used
throughout `FirestoreDocument`). This is a contained, BC-3-internal type
change — `fields` is NEVER serialized into the wire-level `DocumentDelete`
proto response (which carries no field data in real Firestore either); it
exists ONLY as `evaluate()`'s own decision input inside each subscriber's
loop, preserving the existing non-leakage discipline (a denied caller learns
nothing beyond "no event arrived" — identical in kind to `handle_get_document`'s
own AC-17-10 guarantee).

**Why widen the existing query, not add a second one, and not query
per-subscriber**: the existing query ALREADY pays a full round-trip for
every delete NOTIFY today (its result is simply discarded into a
no-field-data `Removed` today) — widening its `WHERE` predicate and `SELECT`
list is a ZERO-additional-round-trip change. This preserves Finding 6's own
load-bearing structural property — "one document, already fetched, before
fan-out, shared across all subscribers via `fan_out()`'s existing clone" —
symmetrically for the delete case, exactly as it already holds for the
`Changed` case. A per-subscriber fetch (each `handle_add_target` loop
independently re-querying on `Removed`) was considered and rejected: it
would cost N queries for N subscribers of the same collection instead of
one, the opposite of Finding 6's own guarantee, and would reintroduce a
per-event DB round-trip this feature's own NFR discipline (Decision Driver
3) exists to avoid.

Each subscriber's own loop applies `evaluate()` identically to the `Changed`
arm:

```rust
Some(ListenEvent::Removed { path, fields }) => {
    if path.collection_path != collection.collection_path {
        continue; // US-01
    }
    if let Some(condition) = &condition {
        let empty_fields: BTreeMap<String, FieldValue> = BTreeMap::new();
        if evaluate(condition, auth_ctx.as_ref(), &fields, &empty_fields)
            == EvaluationOutcome::Deny
        {
            continue; // US-05: withheld, never sent.
        }
    }
    // ...existing DocumentDelete send logic, unchanged — fields never
    // enter the proto response...
}
```

For content-blind rules (`request.auth != null`, bare `true`), `evaluate()`'s
own decision never dereferences `fields` at all (AC-17-125) — correct
regardless of whether `fields` is populated or empty, by the same
type-level guarantee ADR-027's fail-closed semantics already establish.

## Decision — Admin Surface (US-08): reuse `simulate_query_compliance` UNCHANGED — an evaluated departure from ADR-032's own precedent, not a blind mirror

DISCUSS's own (non-binding) Technical Note left this as DESIGN's call: "a
dedicated route alias" vs. "existing documentation suffices." This ADR
evaluates the request-contract-identity question independently, as
ADR-032's own `simulate_group_query_compliance` decision did — and reaches
the OPPOSITE conclusion from ADR-032, because the two situations are not, in
fact, alike.

**ADR-032 introduced a new sibling handler because the request contract
GENUINELY differed**: `simulate_query_compliance`'s `condition: String` is
required; the collection-group case needed `group_condition:
Option<String>` to make US-04's "no group rule" default a first-class
simulatable scenario — a real semantic difference `None` had to carry.

**Listen's subscribe-time gate has no analogous difference.** It calls
`check_query_compliance(condition, filter, auth)` with the EXACT SAME input
shape `handle_run_query`'s own non-group arm already uses — a required
candidate `condition: String`, an optional `auth`, and a `query_filters`
list. There is no genuinely different request semantics to justify a new
type, and therefore no justification for a new handler or route.
`SimulateQueryComplianceResponse { compliant, reasons }` already expresses
everything US-08's own AC require (AC-17-134/135).

**Decision: `simulate_query_compliance` (`POST .../access_rules/
simulate_query`) is reused completely UNCHANGED** — zero new route, zero
new handler, zero new request/response type. A one-line doc-comment addition
to the existing handler in `admin/handlers/access_rules.rs` notes that it
also models Listen's own subscribe-time compliance gate (ADR-033), since
both call sites share the identical `check_query_compliance()` input shape.

**AC-17-136 ("admitted for a candidate collection id with no rule")**: this
AC, as literally worded, presumes a "candidate collection id" input
`simulate_query_compliance`'s existing contract does not have (it takes a
candidate `condition` text, never a real collection reference, and never
reads `access_rules` — confirmed by the handler's own doc comment: "never
read from or written to `access_rules`"). This ADR resolves AC-17-136 as
satisfied STRUCTURALLY, not via a new live-simulation call: a collection
with no defined rule is, by construction, US-06's own unchanged "no rule ⇒
unrestricted" default (`get_access_rule() -> None` short-circuits before any
compliance check is attempted, identical to `RunQuery`'s own existing
guarantee) — there is no candidate rule TEXT to simulate for "no rule," so
there is nothing for a simulate call to test. Flagged explicitly, not
silently absorbed — see § Open Questions (DESIGN additions),
`feature-delta.md`.

## Consequences

### Positive

- Listen's enforcement mechanism reuses `check_query_compliance()`/
  `evaluate()` — the identical, already-proven compliance mechanisms real
  `RunQuery`/`GetDocument` enforcement already use — zero new decidable
  -shape logic, zero new evaluator branch, zero risk of a third,
  independently-drifting compliance algorithm.
- `embyr_core::access_control` gains ZERO new code — the first feature in
  this initiative where BC-4 grows by nothing at all.
- `GetDocument`, every write handler, and `RunQuery` (group and non-group)
  receive ZERO code changes — verifiable by diff; every new call site lives
  inside `crates/embyr-server/src/realtime/*` plus two visibility-only
  changes in `grpc/handler.rs`.
- The collection-scoping fix (Finding 5) is reached via an unconditional,
  rule-independent check, structurally first in the event loop — a missing
  case is a compile-visible omission from the match arm, not a silent
  runtime gap.
- The subscribe-time gate pays exactly one new indexed `access_rules` lookup
  per subscription (not per event) — identical cost profile to
  `RunQuery`'s own one-time lookup; the per-event re-check (US-04) and the
  delete non-leakage fetch (US-05) each add ZERO additional Postgres
  queries beyond what `fetch_event()` already performs for every NOTIFY.
- This feature's CREATE-NEW footprint is the smallest of the five epics in
  this initiative (zero new tables, zero new migrations, zero new admin
  routes) despite having the largest user-story footprint — every genuinely
  new artifact is a widened predicate, a widened enum variant, or a new
  branch inside an already-existing function.

### Negative / Trade-offs

- The collection-scoping filter (§ Decision — Collection-Scoping) does NOT
  reduce internal `mpsc` fan-out volume — `ListenRegistry::fan_out()` still
  enqueues every project event onto every subscriber's channel exactly as
  it does today; only the SEND-to-gRPC-client step is newly filtered. A
  project with many collections and many concurrent Listen subscriptions
  pays the identical internal enqueue cost as before this feature. The
  per-collection-channel redesign (OQ-SRRT-05) would close this gap but is
  deliberately not built here (see § Decision — Collection-Scoping,
  "Considered and rejected").
- `fetch_event()`'s widened SELECT (§ Decision — Delete Non-Leakage) fetches
  pre-deletion field data for EVERY delete event project-wide, even when no
  active subscriber's rule is content-referencing (or no subscriber exists
  at all for that collection) — a small, fixed marginal cost (one extra
  selected column, same query, same round-trip count) accepted in exchange
  for preserving Finding 6's "fetch once, share via fan-out" property rather
  than N per-subscriber re-fetches.
- `handle_add_target`'s error type change (`String` → `Status`) touches
  every existing error-construction site in the function — a mechanical,
  zero-behavior-change refactor, but a larger diff than a purely additive
  change would have been.
- AC-17-136 is satisfied structurally, not via a new live-simulation
  capability — if the orchestrator/product owner later wants Alex to be
  able to ask "does collection X currently have a rule at all" through the
  simulate surface directly, that is a genuinely new capability (a
  collection-id-aware admin query), not something this ADR builds.

## Enforcement

Style: Hexagonal (ports-and-adapters), unchanged project-wide pattern. No
new crate, no new bounded context. BC-3 Real-Time Delivery gains its first
-ever internal correctness/scoping mechanism in this initiative (the
collection-scoping filter and the widened `fetch_event()` predicate); BC-4
Access Control gains two new, unchanged-mechanism call sites and zero new
code.

Rules enforced (existing, applying unchanged):

- `embyr-core::access_control` retains zero IO imports (`cargo-deny`,
  `deny.toml`) — this feature adds no code to that module at all.
- `embyr-core` defines the value-type/function surface; `embyr-server`
  consumes it — dependency direction inward, unchanged.

**No new driven port, no new Earned Trust probe (Principle 12 discipline,
explicit reasoning required, mirroring ADR-029/030/031/032 § Enforcement
verbatim):**

- `get_access_rule` (new call site inside `handle_add_target`, existing
  method, zero modification) executes through the existing, already-probed
  `SystemDb` connection pool — identical substrate every prior epic's own
  rule lookup already uses.
- `fetch_event()`'s widened query executes through the existing,
  already-probed customer-Postgres pool `PostgresNotifyListener::start`
  already holds — a predicate/column-list change to an existing,
  already-proven query shape, not a new substrate dependency.
- `check_query_compliance`/`evaluate` are unchanged, pure, deterministic CPU
  computation — this feature adds new CALL SITES, not new logic, already
  covered by ADR-031's/ADR-027's own Earned Trust reasoning.
- The substrate this feature adds new reliance on is exactly zero — no
  filesystem, network, subprocess, clock, or vendor-SDK dependency anywhere
  in this feature's own call graph beyond the already-probed Postgres pools
  named above.

`cargo-deny`/`deny.toml` unaffected — no new workspace dependency.

## References

- `docs/feature/security-rules-realtime/feature-delta.md` § Job Discovery —
  Framing Resolution (Resolutions 1-2), § System Constraints, § Handoff
  Package (flags 1-10, including flag 7's genuine judgment call this ADR
  resolves), § User Stories (US-01 through US-08), § Prior Wave
  Consultation — Reading Confirmation.
- `docs/product/architecture/adr-027-access-rule-grammar-and-evaluation.md`,
  `adr-028-access-rule-storage-and-lifecycle.md`,
  `adr-029-access-control-composition-and-bounded-context.md`,
  `adr-030-write-path-grammar-storage-and-composition.md`,
  `adr-031-query-shape-compliance-check.md`,
  `adr-032-collection-group-rule-storage-and-composition.md` — the machinery
  this ADR composes, not replaces; ADR-032 specifically for the "evaluated,
  not blindly mirrored" admin-surface reasoning this ADR extends in the
  opposite direction.
- `crates/embyr-core/src/access_control/mod.rs` (full, 1496 lines) —
  confirmed `check_query_compliance`/`evaluate`/`QueryComplianceOutcome`/
  `UnsatisfiedConjunct` exact current shape; confirmed zero new type or
  function is needed here.
- `crates/embyr-server/src/grpc/handler.rs` (targeted full reads:
  `handle_run_query` lines 1131-1364, `handle_listen` lines 1366-1462,
  `attach_client_identity_if_present` lines 358-383, `handle_get_document`
  lines 506-629, `query_compliance_rejection`/`translate_filter` lines
  1600-1719) — the exact current composition this ADR's new call sites are
  inserted into or mirror.
- `crates/embyr-server/src/realtime/listen_handler.rs` (full, 251 lines) —
  the exact current shape of Findings 1/2/3/4/5 this ADR fixes.
- `crates/embyr-server/src/realtime/listen_registry.rs` (full, 109 lines) —
  confirmed `ListenEvent`/`SubscriberEntry`/`fan_out()`'s exact current
  shape; confirmed `fan_out()` itself requires ZERO change (the filter is
  applied on the consuming side).
- `crates/embyr-server/src/adapters/postgres_notify_listener.rs` (full, 154
  lines) — confirmed `fetch_event()`'s exact current query and its
  "already fetched, before fan-out" property (Finding 6) this ADR extends
  symmetrically to the delete case.
- `crates/embyr-pg-storage/src/backend_adapter.rs` (targeted, lines 508/826)
  — confirmed the `documents` table's soft-delete mechanism (`deleted`
  boolean, never a hard `DELETE`) — Finding 7, the evidence behind §
  Decision — Delete Non-Leakage.
- `crates/embyr-server/src/adapters/system_db.rs` (targeted, lines 360-380)
  — confirmed `get_access_rule`'s exact current signature and `Ok(None)`
  short-circuit contract, reused unchanged.
- `crates/embyr-server/src/admin/handlers/access_rules.rs` (full, 751
  lines) — confirmed `simulate_query_compliance`'s exact current request
  /response contract, the direct evidence behind § Decision — Admin
  Surface's "identical shape, no new type" conclusion.
