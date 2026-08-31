# ADR-050: `ListDocuments` Is Built Uniformly on `BackendAdapter::run_query` — the Agent's Own Dedicated RPC Is Confirmed Buggy, Not Just Orphaned (Resolves DISCUSS's Escalation 1)

## Status

Accepted

## Context

DISCUSS's own Handoff Package escalated, not resolved, one question: should
the new direct-mode `handle_list_documents` handler (a) call
`BackendAdapter::run_query` uniformly for every `backend_mode` — which "just
works" for `backend_mode=agent` via `AgentBackendAdapter::run_query`'s
existing proxy to the agent's own `RunQuery` RPC, but leaves
`StorageAgent`'s own already-implemented, dedicated `ListDocuments` RPC
(`crates/embyr-agent/src/server.rs::list_documents`, lines 513-557)
permanently unused; or (b) special-case `AgentBackendAdapter` to call that
dedicated RPC directly instead. DISCUSS named a "reuse-target caveat" — the
agent's own handler treats `collection_id` as already a full relative path,
diverging from `docs/SPEC.md`'s `parent`+`collection_id` convention — but
explicitly did not confirm whether this was a real bug or a narrower-but-still
-correct assumption, and had no efficiency/latency evidence to prefer either
option.

## Verification (ground-truth, ADR-041/047/049's own standing practice — do
not trust an escalation's framing without re-reading the code)

Read in full or targeted, at current line numbers:

1. `crates/embyr-agent/src/server.rs:513-557` (`list_documents`) — confirmed
   DISCUSS's own reading: builds `CollectionPath{ collection_path:
   req.collection_id.clone() }` directly, with **no call to
   `build_collection_path`** (the function that exists in this exact file,
   lines 731-746, for precisely this join).
2. `crates/embyr-agent/src/server.rs:262` — confirms `build_collection_path`
   IS used elsewhere in this file (one call site, inside `create_document`),
   proving the correct join logic already exists in the same file
   `list_documents` is defined in and was simply not reused there. This is not
   a case where the join logic doesn't exist yet — it exists, one function
   away, and `list_documents` bypasses it.
3. `crates/embyr-agent/src/server.rs:731-746` (`build_collection_path`, read
   in full) — confirms its own doc comment states the exact semantics
   `list_documents` should have used: `parent` with no trailing document path
   → `collection_path = collection_id`; `parent` with a trailing document path
   (`.../documents/a/b`) → `collection_path = "a/b/{collection_id}"`.
4. `crates/embyr-pg-storage/src/backend_adapter.rs:530-554` (`run_query`, read
   in full at § Reading Confirmation already) — confirms the SQL `run_query`
   emits for `all_descendants: false` (the mode `list_documents` always uses)
   is an EXACT match: `collection_path = <bound value>`.

**Finding 1 (confirms and sharpens DISCUSS's own suspected divergence into a
confirmed functional bug):** for any `parent` naming a document below the
database root — e.g. `parent = "projects/p/databases/(default)/documents/users/maria-santos-a1b2"`,
`collection_id = "trip_entries"` — the agent's own `list_documents` handler
constructs `collection_path = "trip_entries"` and issues an EXACT match
against it. The real data lives at `collection_path =
"users/maria-santos-a1b2/trip_entries"`. The query matches zero rows. This is
not a narrower-but-correct assumption; it is a request that returns an
incorrect (empty) result for exactly the domain example this feature's own
US-01 UAT scenario uses (Maria Santos's nested `trip_entries` collection).
`GetDocument`/`CreateDocument`-shaped nested lookups are unaffected — only
`list_documents`'s own bypass of `build_collection_path` is broken.

**Finding 2 (new — not flagged by DISCUSS, discovered during this
verification, independently sufficient to decide this escalation):** the
agent's own `list_documents` handler has no code path for AC-01-03 at all
("`collection_id` empty → documents from every collection directly under
`parent`"). When `collection_id` is empty, its `CollectionPath.collection_path`
becomes `""`, and `run_query`'s SQL (Verification item 4) performs `WHERE
collection_path = ''` — the `documents` table's `collection_path` column is
never empty for any real row (§ migrations/customer/0001_documents.sql,
DISCUSS's own Reading Confirmation), so this branch silently returns an empty
result set instead of the correct multi-collection fan-out. The agent's own
RPC was never capable of the AC-01-03 behavior this feature requires — routing
`backend_mode=agent` through it would not merely reuse an existing RPC, it
would ship a silent, wrong-answer regression for exactly the scenario
`docs/SPEC.md` itself documents ("If `collection_id` is empty, all collections
under `parent` are included").

## Decision

**`handle_list_documents` is built uniformly on `BackendAdapter::run_query`
for every `backend_mode`, including `agent`.** `AgentBackendAdapter` is not
modified by this feature — `backend_mode=agent` gets `ListDocuments` "for
free" through its existing, already-correct `run_query` proxy to the agent's
own `RunQuery` RPC (a code path Findings 1/2 do not touch — `RunQuery` is a
separate, already-shipped, already-correct handler on both sides of the mTLS
channel). `crates/embyr-agent/` is untouched by this feature, matching
ADR-041/047/049's own "zero agent-binary changes" discipline.

`StorageAgent`'s own dedicated `ListDocuments` RPC and its
`crates/embyr-agent/src/server.rs::list_documents` handler remain fully
orphaned after this feature ships — not merely unused as DISCUSS found, but
now confirmed **incorrect** for any nested `parent` (Finding 1) and
**incapable** of the `collection_id`-empty case (Finding 2). **Named follow-up
candidate, out of scope for this feature**: either delete the dead handler and
its `ListDocuments` RPC declaration from `storage_agent.proto` (a genuine
proto-surface cleanup, since nothing calls it and it cannot be fixed without
also fixing the `run_query`-empty-`collection_path` limitation Finding 2
exposes), or fix it properly if a future need for agent-side unary listing
independent of `RunQuery` ever arises. This ADR does not decide which — it
only confirms the code is dead and now known-broken, which is new information
DISCUSS did not have.

## The shared "immediate children of a path" primitive (why Slice 01 needs a
new trait method after all)

DISCUSS's own Learning Hypothesis for Slice 01 assumed `run_query` alone would
be sufficient, disproving "zero new trait surface is needed." This assumption
does **not** hold for the `collection_id`-empty case (AC-01-03): "every
collection directly under `parent`" is the same "immediate children of a
path" query `ListCollectionIds` needs (§ System Constraints, confirmed by
DISCUSS itself), and no existing `run_query` mode (`all_descendants: false`
exact match, or `all_descendants: true` same-name-at-any-depth collection
group) expresses it. `handle_list_documents`'s `collection_id`-empty branch
therefore depends on the new `BackendAdapter::list_collection_ids` trait
method this feature introduces for Slice 02 (exact signature and SQL: ADR-051)
— called internally, unpaginated (a single large-limit call), to enumerate
every immediate child collection name under `parent`, followed by one
`run_query` call per child collection (existing, unmodified) to fetch that
child's matching documents. Results are merged, sorted deterministically by
`(collection_path, document_id)` for a stable page order, then the existing
`page_size + 1`/offset windowing technique (§ Component Decomposition, this
file's companion `feature-delta.md` DESIGN sections) is applied over the
merged, sorted `Vec` in Rust rather than via SQL `LIMIT`/`OFFSET`.

**Named, accepted simplification (ceiling), not hidden**: this fetches every
matching document across every immediate child collection before paging,
rather than pushing the page window down into SQL. Acceptable for this RPC's
own documented nature — a thin, no-filter, no-order "convenience" enumeration
path (`docs/SPEC.md`'s own framing), where the realistic number of immediate
child collections under one parent, and documents within them, is small
(mirrors this feature's own UAT scenarios: 2-3 subcollections). **Upgrade
path, if ever needed**: push a single SQL predicate ("collection_path is
exactly one segment deeper than `parent`", the same shape `list_collection_ids`
already builds, ADR-051) directly into a document-returning query instead of
enumerating names first — not built now because it would be a second new SQL
shape solving a problem this feature has no evidence exists (no benchmark or
customer report of pathologically wide immediate-child fan-out), mirroring
ADR-049's own "no unevidenced numbers/optimizations" discipline.

## Alternatives Considered

**A. Special-case `AgentBackendAdapter::run_query`... no — special-case a NEW
`AgentBackendAdapter::list_documents` method calling the agent's own dedicated
RPC (DISCUSS's own option (b)).** Rejected on stronger grounds than DISCUSS
had: this is not merely "requires reconciling a path-handling divergence" (as
DISCUSS framed it) — Finding 1 confirms the divergence is an outright bug for
nested parents, and Finding 2 confirms the agent's own RPC cannot serve
AC-01-03 at all without new agent-binary logic. Choosing option (b) would mean
either (i) shipping `backend_mode=agent` `ListDocuments` with a real,
demonstrable correctness bug for nested parents and a missing capability for
the empty-`collection_id` case, or (ii) fixing both inside
`crates/embyr-agent/src/server.rs` — new agent-binary source changes this
feature has no other reason to make, and a second, independent
"immediate-children" query implementation living inside the agent binary
alongside the SQL-side one Slice 02 already needs, doubling the surface area
for the exact primitive ADR-051 designs once.

**B. Fix the agent's own `list_documents` bug (Finding 1) and add the
missing empty-`collection_id` capability (Finding 2) as part of this
feature, then route agent-mode through it (a corrected version of option
(b)).** Rejected: violates walking-skeleton discipline the same way ADR-047's
Alternative A did — this feature's own scope (2 slices, ~2.5 days) would grow
to include debugging and extending a SEPARATE deployment artifact (the
customer-VPC agent binary) for a code path that has a simpler, already-correct
substitute (`run_query`) one layer up. `AgentBackendAdapter::run_query`
already proxies correctly-shaped, correctly-scoped requests; there is no
efficiency case for the RPC hop savings strong enough to justify fixing two
bugs in a binary this feature has no other reason to touch, with zero latency
evidence motivating it (DISCUSS's own explicit finding: none exists).

**C. Route uniformly through `BackendAdapter::run_query` for every backend
mode, including agent, leaving the agent's own dedicated RPC orphaned and now
confirmed-buggy, named as a cleanup/fix candidate (chosen).** Zero new
agent-binary work. Zero risk of inheriting Findings 1/2 into this feature's
own `backend_mode=agent` behavior. `AgentBackendAdapter::run_query`'s own
existing correctness (unaffected by either finding — a structurally different
code path) is reused unchanged, consistent with this session's own
reuse-first discipline (ADR-049 § Decision Drivers 1).

## Consequences

**Positive**: `backend_mode=agent` `ListDocuments` is correct on day one for
both the `collection_id`-set and `collection_id`-empty cases — including the
exact nested-`parent` scenario (`users/maria-santos-a1b2/trip_entries`) this
feature's own UAT scenarios use — because it never touches the two confirmed
bugs in the agent's own dedicated RPC. Zero new agent-binary surface,
`crates/embyr-agent/` untouched, matching ADR-041/047/049's own standing
practice. `StorageAgent`'s own dedicated `ListDocuments` RPC's brokenness is
now a documented, discoverable fact (this ADR) rather than a silent landmine
for a future feature that might have wired it up trusting its existing tests
(if any) to have caught the nested-parent bug.

**Negative, named explicitly**: `StorageAgent`'s own proto and binary continue
to carry a dead, and now confirmed-broken, `ListDocuments` RPC — genuine
technical debt, not zero-cost to leave in place (a future reader unaware of
this ADR could still wire it up, reintroducing both bugs). Flagged as a
concrete cleanup candidate (delete the RPC and its handler, or fix it
properly) for a future feature's own scope, not this one's.

**Residual, non-blocking**: the `collection_id`-empty branch's in-memory
fetch-then-paginate approach (§ The shared "immediate children of a path"
primitive) has no benchmark confirming it is acceptable for pathological
fan-out. Named as a DEVOPS/production-readiness measurement candidate, same
non-blocking treatment as ADR-048/049's own residuals.
