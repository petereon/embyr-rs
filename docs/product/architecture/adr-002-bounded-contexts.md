# ADR-002: Bounded Context Split

## Status

Accepted

## Context

embyr-rs is a Firestore protocol translation server with no document storage of its own. It accepts Firebase SDK gRPC traffic and maps operations to customer-owned Postgres. The system serves multiple distinct actors — SDK developers (Alex), service operators (Sam), tenant admins (Morgan), and compliance-first tenants (Riley) — each with a distinct vocabulary and distinct concerns.

Before beginning tactical design (aggregates, repositories, services), the bounded contexts must be established. Without this, the codebase risks:

- Tenant management concepts (project lifecycle, credential encryption, usage metering) entangled with document CRUD logic
- Real-time delivery state (listen targets, resume tokens, NOTIFY fan-out) mixed with document storage queries
- A "god service" pattern where a single module handles authentication, document persistence, and stream management simultaneously

The question to answer: How many bounded contexts does embyr-rs have, and where are the boundaries?

## Decision Drivers

1. **Language divergence** (primary signal): When the same word means different things to different personas, a boundary exists.
2. **Storage isolation**: System DB vs. Customer DB vs. in-process state are structural isolation points that domain boundaries must respect.
3. **Consistency model divergence**: Tenant Management requires synchronous, per-request consistency (suspension takes effect in < 1 request). Document Storage uses OCC consistency (version-based conflict detection). Real-Time Delivery uses eventual consistency (NOTIFY-driven fan-out).
4. **Failure independence**: A Listen stream registry overflow (BC-3) must not fail a document write (BC-2). A credential cache miss (BC-1) must not affect already-authenticated streams (BC-3).
5. **Codebase isolation**: Contexts must map to modules or crates that can be developed, tested, and reasoned about independently.

## Considered Options

### Option A: One bounded context (monolithic domain model)

All concepts — Project, Document, Transaction, Index, ListenTarget, ResumeToken, BrowserChannelSession, DailyProjectMetrics — live in a single domain model.

**Rejected.** "Project" in a single model would need to carry both tenant-lifecycle concerns (suspension, deletion, auth key rotation) and query-routing concerns (which Postgres DSN to use) and listen-scoping concerns (which NOTIFY channel to subscribe to). The model becomes a god object. Vocabulary conflicts immediately: "project" means a billing unit to Sam, a namespace to Alex, and a connection target to the NOTIFY listener. The inconsistency is unresolvable in a single model.

### Option B: Two bounded contexts (Tenant + Storage)

Split: Tenant Management handles project lifecycle and authentication. Storage handles documents, transactions, indexes, queries, and real-time delivery together.

**Rejected.** Putting listen targets alongside document CRUD conflates two fundamentally different consistency models and storage locations (Customer DB vs. in-process). "Listen target" has no meaning in document mutation vocabulary. Real-time delivery has unique concepts — resume tokens, snapshot/delta delivery modes, buffer overflow, RESET semantics — that are absent from document storage. Grouping them forces every document-storage module to be aware of stream state, or creates internal sub-modules that are effectively separate bounded contexts with informal boundaries.

### Option C: Three bounded contexts (Tenant Management + Document Storage + Real-Time Delivery)

**Accepted.** See below.

### Option D: Four bounded contexts (split Credential Resolution from Tenant Management)

Credential resolution (ECIES decrypt, AWS/GCP secret fetch, credential cache) as a separate context.

**Rejected.** Credential resolution has no entities, no aggregate roots, no lifecycle, and no invariants of its own. It is a stateless translation of a `BackendConfig` value object (owned by the `Project` aggregate in BC-1) into a live storage connection. Making it a context would create a context with no domain objects — only a service. That is a domain service within BC-1, not a separate context.

## Decision

Adopt three bounded contexts:

### BC-1: Tenant Management

**Responsibility**: Owns the lifecycle of a `Project` (the tenant unit). Provisions, suspends, activates, and soft-deletes projects. Owns authentication (Argon2id verification, dual-hash rotation window). Owns credential resolution (ECIES decrypt for `direct_pg`; AWS/GCP secret fetch; agent endpoint). Owns `DailyProjectMetrics` accumulation.

**Ubiquitous language (BC-1)**: Project, ProjectId, AuthKey, Argon2idHash, DualHashWindow, BackendConfig, EncryptedDsn, BackendMode, AuthMode, ProjectStatus, CredentialCache, CredentialFingerprint, DailyProjectMetrics, AdminKey, DeletionRetentionWindow, SuspensionEffect.

**Storage boundary**: System DB only. BC-1 never reads or writes the Customer DB directly. It resolves a `BackendAdapter` from `BackendConfig` and hands it to BC-2 and BC-3.

**Consistency requirement**: Synchronous. Suspension must take effect within the next SDK request (< 1 request latency). Project status is read per-request from the System DB — no separate status cache that could be stale.

**Downstream consumers**: BC-2 and BC-3 receive `ProjectId` + `BackendAdapter` from BC-1. They do not access BC-1's data store.

---

### BC-2: Document Storage

**Responsibility**: Owns the Firestore document model: `Document` (CRUD, OCC via `version`), `Transaction` (Begin/Commit/Rollback, TTL expiry), `Index` (Creating → Ready lifecycle), `Tombstone` (delete record for delta delivery). Executes `RunQuery`, `BatchGet`, `RunAggregationQuery` against customer-owned Postgres.

**Ubiquitous language (BC-2)**: Document, DocumentPath, Collection, CollectionGroup, Fields, Version, Transaction, TransactionId, OccConflict, Mutation, Index, IndexState, Tombstone, StructuredQuery, QueryCursor, BatchGetRequest, AggregationQuery.

**Storage boundary**: Customer DB only. BC-2 never reads the System DB. It receives a `BackendAdapter` from BC-1.

**Consistency requirement**: OCC. Concurrent writes conflict on `version`; losing writer receives `ABORTED` and must retry. No pessimistic locking — incompatible with connection pooling.

**Signal to BC-3**: Document writes produce a `DocChange` signal via Postgres NOTIFY (fired within the committing transaction). This is the integration point between BC-2 and BC-3.

---

### BC-3: Real-Time Delivery

**Responsibility**: Owns the live subscription model: `ListenTarget` (query subscription with resume token), `BrowserChannelSession` (long-poll session state), `ListenRegistry` (in-process fan-out map). Delivers initial snapshots and delta updates to SDK clients. Handles overflow/RESET semantics.

**Ubiquitous language (BC-3)**: ListenTarget, TargetId, ResumeToken, SnapshotDelivery, DeltaDelivery, DocChange, ChangeType, ListenRegistry, RegistryOverflow, RESET, CURRENT Marker, NO_CHANGE, BrowserChannelSession, SessionId, StreamToken.

**Storage boundary**: In-process only. No persistent storage. `ResumeToken` is a value handed back to the client; the client presents it on reconnect. BC-3 does not store tokens durably — the 24-hour tombstone retention window in BC-2 is the durability mechanism that makes delta delivery possible after reconnect.

**Consistency requirement**: Eventual. Postgres NOTIFY delivers `DocChange` to the embyr instance holding the LISTEN connection for the project. Fan-out to individual `ListenTarget` handlers occurs in-process, asynchronously. p99 target: write → `onSnapshot` callback ≤ 2 s.

**Dependency on BC-2**: BC-3 consumes `DocChange` signals from BC-2 (via Postgres NOTIFY). On each `DocChange`, the Listen handler re-fetches the full document via a BC-2 read (GetDocument) to work around the 8 KB NOTIFY payload cap.

## Consequences

### Positive

- Each context can be developed in its own Rust module (or crate) with explicit public surface area. BC-1 logic cannot accidentally call into BC-2 storage code.
- The System DB and Customer DB are accessed by different modules, enforcing the isolation invariant structurally.
- Testing BC-2 (document CRUD) does not require a project record or auth logic. A mock `BackendAdapter` suffices.
- Testing BC-3 (real-time delivery) does not require a Customer DB. A mock `DocChange` channel and a mock read interface suffice.
- Suspension (BC-1) takes effect without any coordination signal to BC-2 or BC-3 — the auth middleware reads project status before dispatching to either context.

### Negative / Trade-offs

- The `BackendAdapter` interface is a coupling point between BC-1 (which resolves it) and BC-2/BC-3 (which use it). The adapter abstraction must be designed carefully to avoid leaking BC-1 concepts (credential modes, ECIES details) into BC-2/BC-3.
- BC-3's dependency on BC-2 for the re-fetch path (GetDocument after NOTIFY) means BC-3 must hold a reference to BC-2's read interface. This is acceptable — it is a read-only, non-transactional dependency — but it must not become a write dependency.
- In-process `ListenTarget` state (BC-3) means that instance restarts cause all active listeners to re-snapshot. This is an accepted trade-off (locked decision D09).

## Context Map Summary

```
Firebase SDK (OHS consumer)
    → BC-1 Tenant Management  [OHS + Published Language: Firestore gRPC proto]
    → BC-2 Document Storage   [OHS + Published Language]
    → BC-3 Real-Time Delivery [OHS + Published Language]

BC-1 Tenant Management
    → BC-2 Document Storage   [Customer-Supplier: BC-1 supplies ProjectId + BackendAdapter]
    → BC-3 Real-Time Delivery [Customer-Supplier: same]
    → AWS Secrets Manager     [ACL: BackendConfig.AwsSecret → resolved DSN]
    → GCP Secret Manager      [ACL: BackendConfig.GcpSecret → resolved DSN]

BC-2 Document Storage
    → BC-3 Real-Time Delivery [Domain Signal: DocChange via Postgres NOTIFY]
    → embyr-agent             [Conformist: StorageAgent proto, agent mode only]

BC-3 Real-Time Delivery
    → BC-2 Document Storage   [read-only dependency: GetDocument on DocChange]
    → embyr-agent             [Conformist: StorageAgent proto, agent mode only]
```

## References

- `SPEC.md` — Firestore protocol and data model (authoritative)
- `docs/product/architecture/brief.md` §§ Domain Model, System Architecture
- `docs/feature/embyr-rs/discuss/feature-delta.md` §§ Locked Decisions (D9, D10)
- Vaughn Vernon, "Implementing Domain-Driven Design," Chapter 2 (Bounded Contexts) and Chapter 10 (Aggregates)

---

## Changed Assumptions (appended by feature `security-rules`, DESIGN wave, 2026-08-17)

**Original assumption, quoted verbatim (§ Option D, above):**

> Credential resolution has no entities, no aggregate roots, no lifecycle, and no
> invariants of its own. It is a stateless translation of a `BackendConfig` value
> object (owned by the `Project` aggregate in BC-1) into a live storage
> connection. Making it a context would create a context with no domain objects —
> only a service. That is a domain service within BC-1, not a separate context.

**Why this is being appended, not reopened:** Option D's rejection stands
unchanged for credential resolution — nothing about that reasoning is wrong or
being walked back. This amendment records a *new* subsystem, evaluated fresh
against the same test Option D applied, that reaches the opposite conclusion.

**New assumption:** The `security-rules` feature (Epic 2a, `docs/feature/
security-rules/feature-delta.md`) introduces a per-collection access-control
`AccessRule` subsystem that — unlike credential resolution — **does** have an
entity with identity (`(project_id, collection_path)` → condition), **does** have
a lifecycle (define → redefine, an idempotent-upsert lifecycle locked by that
feature's Resolution 3), and **does** have invariants of its own (grammar
validity of the stored condition). It fails all three of the absence-tests Option
D's rejection turned on. Applying Option D's fold-into-BC-1 conclusion to a case
that fails Option D's own test would be inconsistent with this ADR's stated
methodology (Decision Drivers 1–5, applied per-case, not by inertia).

**Decision:** A fourth bounded context, **BC-4: Access Control**, is added. Full
alternatives analysis (Option A: extend BC-1; Option B: extend BC-2; Option C:
new BC-4, accepted) against this ADR's own five decision drivers is recorded in
`docs/product/architecture/adr-029-access-control-composition-and-bounded-context.md`
§ Considered Options — Bounded-Context Placement. That ADR also establishes BC-4's
read-only, non-transactional dependency on BC-2 (for `resource.data` during
evaluation) as directly mirroring this ADR's own already-established BC-3→BC-2
dependency shape (§ BC-3, "Dependency on BC-2," above) — no new kind of
inter-context relationship is introduced, only a new instance of a kind ADR-002
already sanctions.

**Context Map addition** (additive to § Context Map Summary, above, not a
rewrite):

```
BC-4 Access Control
    → BC-2 Document Storage   [read-only, non-transactional: resource.data during GetDocument evaluation]
    → BC-1 Tenant Management  [read-only, indirect: AuthContext built from VerifiedEndUserIdentity, never re-verified]
```

This context map entry does not modify BC-1, BC-2, or BC-3's own existing map
entries above — it is purely additive.

---

## Changed Assumptions (appended by feature `client-auth-hosted-identity`, DESIGN wave, 2026-08-27)

**Why this is being appended, not reopened:** the `security-rules` amendment
above (BC-4) and Option D's original rejection (credential resolution) both
stand unchanged — nothing about either is walked back. This amendment
records a *second* new subsystem, evaluated fresh against the identical
Option-D three-part test, that also reaches the "new context" conclusion —
confirming the test is being applied per-case, not by inertia, a second time.

**New assumption:** `client-auth-hosted-identity` (JOB-18) introduces a
hosted-identity `Account` entity that has an identity of its own
(`(project_id, email)`), a lifecycle of its own (create → reset →
[deferred: disable/delete]), and invariants of its own (email uniqueness per
project, password-strength rules, single-use/expiring reset tokens) — it
passes all three of Option D's tests, exactly as `AccessRule` did for BC-4.

**Decision:** a fifth bounded context, **BC-5: Hosted Identity**, is added.
Full alternatives analysis and the storage-boundary split this context
introduces (the first bounded context in this system with storage split
across both System DB and Customer DB) are recorded in
`docs/product/architecture/adr-036-hosted-identity-bounded-context-and-storage.md`
§ Decision 1 — Bounded-Context Placement.

**Context Map addition** (additive to § Context Map Summary and to the BC-4
addition above, not a rewrite of either):

```
BC-5 Hosted Identity
    → BC-1 Tenant Management  [read-only: Project.backend_mode gate; reads and
                                decrypts its own System-DB-resident signing key]
    → BC-2 Document Storage   [shares BC-2's existing PostgresBackendAdapter +
                                migrations/customer/ mechanism — a new CONSUMER
                                of an already-established mechanism, not a new
                                mechanism]
```

This entry does not modify BC-1, BC-2, BC-3, or BC-4's own existing map
entries above — it is purely additive.

---

## Changed Assumptions (appended by feature `oauth-providers`, DESIGN wave, 2026-08-30)

**Why this is being appended, not reopened:** both amendments above (BC-4,
BC-5) stand unchanged. This amendment records a **third** application of the
Option-D three-part test, applied fresh to a genuinely different candidate
entity — and, unlike the two amendments above, this application does **not**
add a new bounded context. That outcome is itself the point being recorded:
Option D is a per-case test, not a rule that always produces "add a new BC"
once a codebase has added two.

**Candidate entity evaluated**: `OAuthProviderCredential`
(`project_id, provider → client_id` — the Google OAuth Client ID `oauth-providers`
US-01 lets a project owner register).

| Test | Result |
|---|---|
| Entity with identity of its own? | Yes — `(project_id, provider)`. |
| Lifecycle of its own? | Yes, but thin — register → redefine (idempotent upsert) → [deferred: deregister]. |
| Invariants of its own? | Yes, but thin — non-empty `client_id`, uniqueness per `(project_id, provider)`. |

**Decision: no new bounded context.** `OAuthProviderCredential` is
structurally the SAME kind of thing `client_identity_credentials` already
is — project-scoped, System-DB-resident, no-confidentiality-property auth
material — and that entity was never itself treated as warranting a
standalone context; it lives inside BC-1 without controversy. Extending BC-1
Tenant Management is the correct, evidence-based conclusion, not an
inertia-driven one — full three-part-test table and the rejected
alternatives (a new BC-6; extending BC-5 instead) are recorded in
`docs/product/architecture/adr-037-oauth-providers-signing-key-and-verification-composition.md`
§ Decision 1.

**BC-1 ubiquitous language addition** (additive, not a rewrite of BC-1's
existing vocabulary, § BC-1 above): `OAuthProviderCredential`,
`OAuthSigningKey`, `GoogleIdToken`, `VerifiedOAuthIdentity`.

No Context Map addition — this feature introduces no new inter-context
relationship; `OAuthProviderCredential` and `oauth_signing_keys` are read/
written entirely within BC-1's own existing storage boundary (System DB),
exactly as `client_identity_credentials` already is.
