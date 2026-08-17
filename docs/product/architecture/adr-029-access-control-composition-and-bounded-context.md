# ADR-029: Access-Control Enforcement Composition, Identity Reuse, and Bounded-Context Placement

## Status

Accepted

## Context

Two things must be decided together because they shape each other: (1) exactly
how rule evaluation composes with the existing `handle_get_document` call — call
order, identity reuse, the existence-non-leakage mechanism, and the structural
no-rule-defined guardrail — and (2) where the new rule-definition/evaluation
subsystem sits relative to ADR-002's three bounded contexts (BC-1 Tenant
Management, BC-2 Document Storage, BC-3 Real-Time Delivery). DISCUSS flagged
placement explicitly (`docs/feature/security-rules/feature-delta.md` §
System Constraints, § Handoff Package flag 7) rather than resolving it, noting
that ADR-002's Option D rejection reasoning (folding credential resolution into
BC-1 because it "has no entities, no aggregate roots, no lifecycle... of its own")
does not straightforwardly apply to a rule subsystem that *does* have an aggregate
with identity and a define→redefine lifecycle.

## Decision Drivers

### For composition (identity reuse, evaluation order, non-leakage)

1. **Identity reuse, not re-derivation** (§ Handoff Package flag 5, HIGH
   integration risk) — the evaluation step's `request.auth` must consume the
   exact `VerifiedEndUserIdentity`/`None` value `attach_client_identity_if_present()`
   already computes.
2. **`GetDocument`-only enforcement surface** (§ Handoff Package flag 6) — exactly
   one call site is touched; no other RPC handler is modified.
3. **Structural regression guardrail** (§ Handoff Package flag 3, AC-17-14/15/16)
   — a collection with no rule defined must be provably, not just testedly,
   unaffected.
4. **Existence non-leakage** (§ Handoff Package flag 4, AC-17-10) — a denied read
   must not reveal, via any difference in its response, whether the target
   document exists.
5. **Simulation shares the exact evaluation routine** (§ Handoff Package flag 8,
   HIGH integration risk) — US-05 must not duplicate ADR-027's `evaluate()`.

### For bounded-context placement

6. **ADR-002's own five decision drivers** — language divergence, storage
   isolation, consistency-model divergence, failure independence, codebase
   isolation (`docs/product/architecture/adr-002-bounded-contexts.md` §
   Decision Drivers) — applied fresh to this feature, not assumed.
7. **ADR-002's Option D precedent, read precisely** — Option D was rejected
   specifically because credential resolution lacks entities, aggregate roots,
   lifecycle, and invariants. This ADR must determine whether the access-rule
   subsystem shares that absence (→ fold into BC-1, matching Option D's logic) or
   lacks it (→ the logic that rejected Option D does not transfer).

## Considered Options — Bounded-Context Placement

### Option A: Extend BC-1 Tenant Management

Add `AccessRule`/`Condition` to BC-1's ubiquitous language, alongside `Project`,
`AuthKey`, `BackendConfig`, `ClientIdentityCredential`.

**Rejected.** Storage locality (System DB) superficially matches BC-1, but
Decision Driver 7 is the deciding factor: unlike credential resolution, an access
rule *does* have an entity with identity (`(project_id, collection_path)` →
condition), a lifecycle (define → redefine, ADR-028), and invariants (grammar
validity, ADR-027) — exactly the three properties Option D's rejection turned on
being *absent*. Folding it into BC-1 anyway would apply Option D's fold-in
conclusion to a case that fails Option D's own test, continuing a "BC-1 as
junk drawer for anything project-scoped" drift that ADR-002 did not intend
(BC-1's defined responsibility is project lifecycle, authentication, and
credential resolution — not arbitrary boolean-expression evaluation over document
content, which is a categorically different kind of computation from anything
else BC-1 does).

### Option B: Extend BC-2 Document Storage

Add `AccessRule` alongside `Document`, `Transaction`, `Index` — evaluation already
needs `resource.data` (BC-2's own domain), so co-locating simplifies the read-time
wiring.

**Rejected — the weakest fit of the three, and a direct boundary violation.**
ADR-002's primary signal (Decision Driver 1, storage isolation) states BC-2's
storage boundary is "Customer DB only... BC-2 never reads the System DB." Rule
*storage* (ADR-028) is unambiguously System-DB-scoped, project-level,
cross-collection state — folding it into BC-2 would require BC-2 to gain a System
DB dependency it has never had, breaking the exact invariant ADR-002 calls its
"primary signal" for drawing boundaries in the first place. BC-2's consistency
model (OCC via `version`) also has no analog for a rule — a rule redefine is a
simple last-write-wins replace (ADR-028), not a version-conflict-detected write.
Evaluation *reading* `resource.data` (already-fetched by BC-2, see § Decision —
Composition below) does not require rule *storage* to live inside BC-2; the two
are separable, and only the latter would actually violate the boundary.

### Option C: A fourth bounded context, BC-4 Access Control — Accepted

A new context owns the `AccessRule` aggregate (definition, storage, lifecycle) and
the pure evaluation function, with a read-only dependency on BC-2's already
-fetched `resource.data` for the duration of a single `GetDocument` call.

**Accepted.** Evaluated against all five of ADR-002's own drivers, applied fresh:

- **Language divergence (driver 1):** `Rule`, `Condition`, `RuleEvaluation`,
  `Allow`/`Deny`, `RuleSimulation` is vocabulary that exists nowhere in BC-1
  (project/auth/credential terms) or BC-2 (document/transaction/index/OCC terms).
  This is the strongest of the five signals and points decisively at a new
  context.
- **Storage isolation (driver 2):** rule storage lives in its own dedicated
  System DB table (`access_rules`, ADR-028), not inside `projects` or
  `client_identity_credentials`. ADR-002 already establishes that BC-1 and BC-3
  can be structurally separate contexts while both ultimately touching
  System-DB-adjacent state (BC-3 is in-process only, but its `ResumeToken`
  durability depends on BC-2's tombstone retention window) — sharing a storage
  *class* (Postgres, System DB) does not by itself collapse two contexts into
  one; what matters is aggregate ownership, and BC-4 owns an aggregate neither
  BC-1 nor BC-2 owns.
- **Consistency-model divergence (driver 3):** rule *definition* requires
  synchronous, immediate-effect consistency (Resolution 3: "immediately and fully
  active") — structurally similar in *class* to BC-1's suspension-takes-effect
  -in-under-one-request requirement, but over a different aggregate with
  different invariants. Rule *evaluation* has no consistency model of its own at
  all — it is a pure computation over data BC-2 has already fetched within the
  same request, with no independent read or write of its own. Neither of BC-1's
  nor BC-2's consistency requirements describes this correctly; it is its own,
  simpler thing.
- **Failure independence (driver 4):** a bug in rule storage/lookup should be
  distinguishable from a bug in project-status/credential logic (BC-1) or
  document-storage logic (BC-2) — keeping `access_rules` lookups in their own
  adapter methods (ADR-028), separate from `get_project_for_auth`/
  `get_client_identity_credential`, means a rule-lookup failure surfaces as its
  own error class, not conflated with an authentication failure.
- **Codebase isolation (driver 5):** `embyr-core::access_control` (ADR-027) is
  independently unit-testable with no `Project` aggregate, no `Document`
  aggregate, and no database — only a `Condition` AST and a plain `BTreeMap`.
  This is a stronger isolation property than either BC-1 or BC-2's own domain
  logic currently has (both require at least a value object from their own
  aggregate to test meaningfully).

**The BC-3 precedent, directly applicable:** ADR-002 already establishes that a
context may hold a "read-only, non-transactional dependency" on another context's
data without being folded into it — BC-3's Listen handler re-fetches a document
via BC-2's `GetDocument` on every `DocChange` (ADR-002 § BC-3, "Dependency on
BC-2"), and ADR-002's own Consequences section calls this "acceptable — it is a
read-only, non-transactional dependency." BC-4's dependency on BC-2 is the same
shape, and structurally *lighter*: BC-4 does not even issue a second call — it
reads the `resource.data` that BC-2's `GetDocument` handler already fetched within
the same request (see § Decision — Composition below), never touching BC-2's
storage adapter itself.

## Decision — Bounded-Context Placement

**BC-4: Access Control** is added as a fourth bounded context.

**Responsibility:** Owns the `AccessRule` aggregate — a per-`(project_id,
collection_path)` boolean condition, its define→redefine lifecycle (ADR-028), and
the pure evaluation function that resolves it against an `(auth, resource)` pair
(ADR-027).

**Ubiquitous language (BC-4):** AccessRule, Condition, ConditionParseError,
EvaluationOutcome (Allow/Deny), AuthContext, RuleSimulation.

**Storage boundary:** System DB only (`access_rules` table, ADR-028) — matching
the storage-boundary *shape* of BC-1 without being BC-1 (different aggregate,
different table, different failure domain).

**Consistency requirement:** Rule definition is synchronous (immediate, full
replace — ADR-028). Rule evaluation has no independent consistency requirement of
its own; it is a pure function evaluated inline within a single BC-2 `GetDocument`
request, over data BC-2 has already fetched.

**Dependency on BC-2:** Read-only, non-transactional, in-process (not even a
second query) — BC-4's evaluator consumes the `resource.data` fields BC-2's
existing `adapter.get_document()` call already returns within `handle_get_document`.
This never becomes a write dependency, and BC-4 never calls into BC-2's storage
adapter directly.

**Dependency on BC-1:** Read-only, indirect — BC-4's `AuthContext` is constructed
from BC-1's `VerifiedEndUserIdentity` (via `client-auth`'s existing
`attach_client_identity_if_present()`), not re-derived. BC-4 never touches BC-1's
`client_identity_credentials` table or verification logic directly.

**This is an amendment to ADR-002**, not a silent reinterpretation. See
`docs/product/architecture/adr-002-bounded-contexts.md` § Changed Assumptions
(appended by this feature) for the formal back-propagation, quoting Option D's
original rejection text verbatim.

## Decision — Composition

### Single call site

`crates/embyr-server/src/grpc/handler.rs::handle_get_document` (lines 506-551 as
read during DESIGN) is the **only** call site touched. No other `handle_*` method
(`handle_create_document`, `handle_update_document`, `handle_delete_document`,
`handle_batch_get_documents`, `handle_begin_transaction`, `handle_commit`,
`handle_rollback`, `handle_run_query`, `handle_listen`) is modified — confirming
§ Handoff Package flag 6.

### Identity reuse (§ Handoff Package flag 5)

The existing line

```
let _verified_identity = self
    .attach_client_identity_if_present(&request, &project_id)
    .await;
```

has its binding renamed to `verified_identity` (dropping the underscore — it
becomes consumed, not discarded) and threaded, unchanged, into the new evaluation
step below. `attach_client_identity_if_present()` itself is not modified in any
way — this feature adds a consumer of its existing return value, not a new code
path into it. `Option<VerifiedEndUserIdentity>` maps to `Option<AuthContext>` via
`verified_identity.as_ref().map(|v| AuthContext { uid: v.end_user_id.clone() })` —
a pure, local translation at the call site, not inside `embyr-core::access_control`
itself (which never constructs an `AuthContext` on its own — see ADR-027).

### Structural no-rule-defined guardrail (§ Handoff Package flag 3, AC-17-14/15/16)

```
let rule_row = self
    .system_db
    .get_access_rule(&project_id, &path.collection_path)
    .await
    .map_err(|e| Status::internal(e.to_string()))?;
```

is called once `path` is available (immediately before the existing document
fetch). When `rule_row` is `None`, the response path is **exactly today's code,
unmodified** — `embyr_core::access_control::evaluate()` is never called. This is
the same short-circuit shape as `attach_client_identity_if_present`'s own
AC-16-08(c) guarantee ("absent header → zero calls into
`embyr_core::client_identity`"): a cheap existence-check against the rule store,
keyed only on `(project_id, collection_path)` — no document fetch, no Argon2id
-scale cost — gates entry into any evaluation logic at all. This mirrors the
project's own established "fast-path status checks before the expensive
verification step" precedent (`authenticate()`'s suspended/deleted check before
Argon2id), which DISCUSS's NFR note explicitly asked DESIGN to reuse.

**This is what makes AC-17-14/AC-17-15/AC-17-16 structural, not just tested:** a
regression scenario from the existing 113-scenario suite exercises a project/
collection with no row in `access_rules`. `get_access_rule` returns `None` for
every one of them (no test-writing effort created any such row), so every one of
those 113 scenarios takes the identical, unmodified code path this feature adds
zero new logic to. The guarantee is a property of the `None` branch containing no
new code, not a property that has to be re-verified by running the suite (though
AC-17-16 does exactly that too, as a second, independent confirmation).

### Existence non-leakage (§ Handoff Package flag 4, AC-17-10)

Ordering: the document fetch (`adapter.get_document(&path)`) happens **before**
the allow/deny decision is finalized, exactly as it does today for the `rule_row
== None` path. When `rule_row` is `Some`:

1. `resource_fields` is built from the fetched document's fields if it exists, or
   an **empty field map** if it does not (`doc_opt.as_ref().map(|d| &d.fields)
   .unwrap_or(&EMPTY_FIELDS)`).
2. `evaluate(&condition, auth_ctx.as_ref(), resource_fields)` is called
   unconditionally — the same call whether or not the document exists.
3. **`Deny` always produces the identical `PermissionDenied` response** (same
   status code, same message, no metadata distinguishing "document existed but
   condition was false" from "document did not exist") — regardless of
   `doc_opt`'s value.
4. **`Allow`** only then branches on `doc_opt`: `Some(doc)` returns the document
   (unchanged from today); `None` returns `NotFound` (unchanged from today).

This achieves non-leakage *for exactly the class of rule ADR-027's fail-closed
semantics protects*: any condition that references `resource.data.<field>`
evaluates every such reference as "missing" against an empty field map (the same
mechanism as AC-17-09), so it denies identically whether the document exists with
a mismatched field or does not exist at all — this is the literal mechanism behind
AC-17-10's UAT scenario, which is written specifically against the ownership rule
(`request.auth.uid == resource.data.owner_id`).

**Scoped clarification, flagged for DISTILL (not silently narrowed):** a rule that
references *only* `request.auth` and literals (never `resource.data`, e.g.
`allow read: if true` or `request.auth != null`) has nothing document-specific to
hide, so an `Allow` verdict against a non-existent document still resolves to the
existing `NotFound` response — which *is* technically existence-revealing, but
only for a rule that never inspected the document's content in the first place.
This matches real Firestore's own behavior for content-blind rules and is not
contradicted by AC-17-10's own UAT scenario (written against a content-referencing
rule). Recorded as **OQ-SR-06** for DISTILL to confirm this reading is in scope,
rather than DESIGN silently deciding it either way.

### Simulation shares the exact evaluation routine (§ Handoff Package flag 8)

Two call sites, one function (`embyr_core::access_control::evaluate`, ADR-027):

1. **Real enforcement** — `handle_get_document` (above), `auth` sourced from
   `attach_client_identity_if_present()`'s already-computed result, `resource_fields`
   sourced from `adapter.get_document()`'s already-fetched result.
2. **Simulation (US-05)** — new admin handler
   `embyr-server::admin::handlers::access_rules::simulate_access_rule`, `auth`
   sourced from the caller-supplied synthetic identity in the request body (or
   `None` for the anonymous case, AC-17-19), `resource_fields` sourced from the
   caller-supplied synthetic document payload in the request body. Both call sites
   also share `parse_condition` (real enforcement re-parses the stored
   `condition_source`; simulation parses the caller-supplied *candidate*
   condition, which may not be stored at all — Alex is explicitly testing before
   publishing).

No third, independent evaluation implementation exists anywhere. This is what
makes AC-17-17's "the same allow/deny outcome real evaluation would produce"
guarantee true by construction, not by convention — mirroring
`client-auth`'s own `verify_client_identity_credential` (US-04) debug-verify
precedent, which calls the identical `verify_client_identity_token()` real sign-in
uses.

## Consequences

### Positive

- Every one of § Handoff Package's flags 3–8 is satisfied by a structural
  property of the code (a branch that doesn't execute, a value that's threaded
  not re-derived, a function that's called from exactly one production call site
  plus one simulation call site), not by a tested convention alone.
- BC-4's placement is justified against ADR-002's own five drivers, not asserted
  by inertia — and the one driver that could have argued against it (storage
  isolation, since BC-4 shares System DB with BC-1) is explicitly addressed by
  precedent (BC-1/BC-3 already coexist as separate contexts without collapsing on
  storage-class alone).
- The `EMPTY_FIELDS` sentinel for non-existent documents reuses the exact same
  fail-closed mechanism (ADR-027) that already handles "field present on the
  wrong document" — no second non-leakage mechanism was invented.

### Negative / Trade-offs

- `handle_get_document` gains one more System DB round-trip (`get_access_rule`) on
  every call, even for collections with no rule — mitigated by the existence
  -check being a single indexed lookup on a composite primary key, the cheapest
  possible query shape, and structurally analogous to `authenticate()`'s own
  existing per-request System DB read.
- The content-blind-rule existence-leak nuance (OQ-SR-06) means "never reveals
  document existence" is true for the evidenced domain examples but not
  universally true for every conceivable rule shape within the locked grammar —
  flagged explicitly rather than either over-claimed or silently narrowed.

## Enforcement

Style: Hexagonal (ports-and-adapters), unchanged. BC-4 is a new inner hexagon
within the same Cargo-workspace enforcement mechanism (AD-01/AD-06) — no new
crate, no new `cargo-deny` configuration; `embyr-core::access_control` falls under
the existing zero-IO rule that already covers all of `embyr-core`.

**Driven Ports — no new port, no new probe (Principle 12 discipline, explicit
reasoning required):** `get_access_rule`/`upsert_access_rule` (ADR-028) execute
through the existing, already-probed `SystemDb` connection pool — the same
substrate BC-1's `get_project_for_auth`/`get_client_identity_credential` already
use. No new adapter, no new external dependency, no new substrate-lie scenario is
introduced. `embyr_core::access_control::evaluate()`/`parse_condition()` are pure
CPU computation with no partial-trust surface (identical reasoning to ADR-024's
"no new probe" justification for `verify_client_identity_token`) — a condition
either parses/evaluates deterministically or it does not; there is no environment
that can lie to a pure function operating on values already in memory.

## References

- `docs/feature/security-rules/feature-delta.md` § System Constraints, §
  Handoff Package flags 3–8.
- `docs/product/architecture/adr-002-bounded-contexts.md` — Option D's rejection
  text (quoted verbatim in the § Changed Assumptions amendment this feature
  appends), § BC-3's "read-only, non-transactional dependency" precedent this
  ADR's BC-4→BC-2 dependency directly mirrors.
- `docs/product/architecture/adr-026-client-identity-composition-with-api-key-auth.md`
  — the structural-non-regression argument shape this ADR's § Structural
  no-rule-defined guardrail directly extends.
- `crates/embyr-server/src/grpc/handler.rs:506-551` (`handle_get_document`),
  `:358-383` (`attach_client_identity_if_present`) — read in full during DESIGN;
  exact current shape of `_verified_identity` confirmed discarded prior to this
  ADR.
