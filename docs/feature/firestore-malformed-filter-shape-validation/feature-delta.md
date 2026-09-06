# Feature Delta: firestore-malformed-filter-shape-validation

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/product/jobs.yaml`, JOB-01 (`sdk-compat`, persona P1 Alex) — read in full.
✓ `docs/evolution/2026-09-06-firestore-range-operator-value-type-support.md` § Follow-Up Work —
the originating flag: this feature's own candidate id, explicitly named as "a DIFFERENT, LOWER
-priority category" from the now-closed 3-feature crash-elimination arc.
✓ `crates/embyr-pg-storage/src/encoding/query.rs` — confirmed the EXACT 4 remaining `panic!` sites
(lines 75, 96, 138, 251): `ArrayContainsAny`/`In`/`NotIn` given a non-`Array` value; `push_scalar_
comparison`'s own outer fallback (reachable only for `Null` against a range operator, since
`Array`/`Map` are already rejected upstream by the immediately-prior feature).
✓ `crates/embyr-server/src/grpc/handler.rs::translate_filter` — confirmed the EXACT, already
-established layering precedent this feature reuses: the immediately-prior feature's own
`Array`/`Map`-plus-range-operator check lives HERE (proto-translation time), not in
`embyr-pg-storage`'s own SQL-generation code — this feature's own 2 new checks follow the identical
pattern.
✓ `docs/product/jobs.yaml`, JOB-11 (`fair-multitenancy`, persona P2 Sam Chen) — read in full;
JOB-12 (`observability`) and JOB-13 (`production-deployment`), also P2 Sam Chen — read for
persona-fit comparison (§ Resolution 1 below).

**A critical re-examination, performed BEFORE locking scope — not assumed from the candidate's own
name**: is this feature's own underlying risk actually the SAME severity class as the 3-feature arc
it follows? Direct investigation, not inference:

- **Who can actually trigger this?** Every trigger case (a non-`Array` value paired with `In`/
  `NotIn`/`ArrayContainsAny`; a `Null` value paired with a range operator) requires constructing a
  raw gRPC `StructuredQuery` that NO real Firestore SDK's own query-builder API can produce — the
  JS/native SDKs' own `.where(field, 'in', value)` methods only ever accept an array for `in`/
  `not-in`/`array-contains-any`, and never expose a way to pass `null` to a range-comparison method.
  Triggering this requires a caller deliberately hand-crafting a malformed raw gRPC request — not
  an accidental, ordinary-usage mistake any real Alex-shaped customer could stumble into.
- **What's the actual blast radius when triggered?** Confirmed EMPIRICALLY, not merely assumed,
  from THIS SESSION's own repeated direct observation: every one of the 3 prior features' own
  DELIVER waves triggered this EXACT class of panic (via the pre-fix code) inside a real, running
  test server multiple times, and in every case the SAME server instance continued serving OTHER
  requests in the SAME test file correctly afterward — Tokio's own default per-task panic isolation
  (no custom `catch_unwind` or panic hook exists in `crates/embyr-server/src/main.rs`/`lib.rs`,
  confirmed by direct grep) means a panic inside one request-handling task aborts ONLY that task
  (surfacing as a raw transport-level error to the SAME caller who sent the malformed request),
  never the whole process, never another tenant's own concurrent request.

**This directly contradicts treating this feature as the SAME severity class as the 3-feature arc
it follows.** The arc's own defining property was: an ORDINARY, WELL-BEHAVED real Firestore client,
doing NOTHING wrong, crashes its own OWN valid query. THIS feature's own trigger requires the
CALLER to have ALREADY sent a request no real SDK could produce, and the consequence is confined
to THAT SAME caller's OWN request. Locked as Resolution 1 below — this reframing changes both the
JTBD persona AND this feature's own priority, not merely its documentation.

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

- Feature type: **Backend** (Decision 1) — a small, defensive input-validation check confined to
  one existing function.
- JTBD: **reuse JOB-11, NOT JOB-01** (Decision 4 = "Yes", existing job) — see Resolution 1; this
  is a service-robustness concern (P2 Sam Chen), not an SDK-compatibility concern (P1 Alex).
- Walking Skeleton: **Yes** (Decision 2) — a single, real proto-level `RunQuery` call with a
  malformed filter shape, proven to return a clean error instead of a transport-level crash.
- UX Research Depth: **Lightweight** (Decision 3) — a narrow defensive fix, no new emotional arc.

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: **P2 Sam Chen (Service Operator / Platform Engineer)** — NOT P1 Alex. Alex, the SDK
developer, is never the one who constructs the malformed input this feature guards against (§
Resolution 1) — Sam is the one who cares that the SERVICE stays well-behaved (clean, named errors;
no raw panics visible in logs/traces) even under a caller sending non-SDK-conforming input,
mirroring JOB-11's own "one bad actor shouldn't produce ugly, unexplained failure modes" framing.

**Job**: **JOB-11 `fair-multitenancy`**, reused, EXTENDED (not replaced) to also cover: a
malformed/adversarial raw gRPC filter shape produces a clean, named `INVALID_ARGUMENT` — never a
raw panic/transport-reset that would show up as an unexplained crash in Sam's own logs or tracing,
even though it's confirmed harmless to OTHER tenants.

## Wave: DISCUSS / [REF] Job Discovery — Framing Resolution

### Resolution 1 — Persona/job reassignment: P2 Sam Chen + JOB-11, not P1 Alex + JOB-01

The `ARGUMENTS` for this feature's own kickoff suggested confirming JOB-01 reuse "though note this
is a hardening/defensive-input concern rather than a 'real client hits this' concern" — direct
investigation confirms this suspicion was correct, and goes further: NO real Alex-shaped SDK client
can EVER trigger this feature's own trigger cases at all (§ Reading Confirmation). JOB-01's own
entire job_story is about Alex's real Firebase app behaving identically to real Firestore — but
Alex's app never sends this shape of request in the first place, so this feature does nothing to
advance JOB-01's own goal.

| Option | Description | Fit |
|---|---|---|
| **(A) JOB-01 reuse** (matching the prior 3 features' own precedent) | Consistent with this session's own established pattern | **Rejected** — the pattern doesn't actually apply; JOB-01 is about Alex's real app, and Alex's real app cannot construct this input |
| **(B) JOB-11 reuse** (`fair-multitenancy`, P2 Sam Chen) — extended to cover "malformed input from any single caller doesn't produce an unexplained crash visible in Sam's own operational surface" | Matches the ACTUAL beneficiary: Sam, who cares about clean operational behavior and explainable failure modes, not Alex, who never encounters this | **Strongest fit** |

**Resolution**: **(B) is locked.**

### Resolution 2 — Priority reassessment: is this still worth building at the priority the
candidate id's own framing implied?

`firestore-range-operator-value-type-support`'s own FINALIZE named this "lower-priority" relative
to the arc it followed — but did not fully quantify HOW much lower. Direct investigation (§ Reading
Confirmation) establishes: zero domain evidence any real caller (malicious or accidental) has ever
triggered this; the blast radius is CONFIRMED self-contained to the offending caller's own single
request; the fix itself is cheap (mirrors an already-proven, already-shipped pattern from the
immediately-prior feature almost verbatim).

**Resolution**: **still worth building — LOCKED as a small, quick hardening fix — but explicitly
NOT framed as closing any further "crash risk" arc, since the crash-elimination arc (real client,
real query) is ALREADY fully closed.** This feature is pure operational polish: turning an
already-harmless-to-others panic into a clean, named error, for Sam's own benefit (readable
operational logs/traces), not a safety-critical fix. Named explicitly so a future reader does not
mistake this for reopening the closed arc.

### Resolution 3 — Design mechanism: extend `translate_filter`, mirroring the established pattern

The immediately-prior feature ALREADY established the exact layering decision this feature reuses:
proto-level shape validation lives in `translate_filter` (`embyr-server`), reusing its own existing
`Result<QueryFilter, String>` → `Status::invalid_argument` mechanism — never new infrastructure, never
touching `embyr-pg-storage`'s own SQL-generation code.

**Resolution**: **locked, identical mechanism, 2 more checks**:
1. `In`/`NotIn`/`ArrayContainsAny` given a non-`Array` value → named rejection.
2. A range operator (`LessThan`/`LessThanOrEqual`/`GreaterThan`/`GreaterThanOrEqual`) given a `Null`
   value → named rejection (mirrors real Firestore's own actual restriction — range comparisons
   against `null` are not a construct any real Firestore SDK exposes either).

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

**Signals checked**: >10 user stories? No (1). >3 bounded contexts/modules? No — ONE file
(`crates/embyr-server/src/grpc/handler.rs`), zero new crates, zero new domain type. Walking
skeleton >5 integration points? No (1: a real, previously-panicking malformed `RunQuery`, proven to
return a clean error). Estimated effort >2 weeks? No — a single slice, well under a day (2 more
`if`-checks in an already-modified function). Multiple independent user outcomes? No — both trigger
cases are 2 faces of the identical mechanism (extend the SAME `translate_filter` check).

**Scope Assessment: PASS** (0 oversizing signals fired) — the smallest feature built this session,
tied with `firestore-equal-notequal-value-type-support`.

## Wave: DISCUSS / [REF] Journey — Sam's "My Logs Stop Showing Unexplained Panics" Arc

### Mental model

Sam operates embyr in production. A caller (a misbehaving custom client, a fuzz-testing tool, an
internal script with a bug) sends a malformed raw gRPC filter — something no real Firestore SDK
would ever construct. Today, this shows up in Sam's own logs/traces as a raw Rust panic — alarming,
uninformative, and hard to distinguish from a genuine internal bug at a glance. After this feature,
the SAME malformed request produces a clean, named `INVALID_ARGUMENT` — immediately recognizable as
"a caller sent something invalid," not "the server has an internal defect."

### Failure modes (feeds DISTILL scenario generation)

- A non-`Array` value (e.g. a bare string) paired with `In`: today panics; after, a clean
  `INVALID_ARGUMENT` naming the problem.
- A `Null` value paired with `GreaterThan`: today panics; after, a clean `INVALID_ARGUMENT`.
- A WELL-FORMED query (any of the 3 prior features' own now-fixed shapes): completely unaffected —
  this feature adds NO new restriction to any construct a real Firestore SDK can produce.

## Wave: DISCUSS / [REF] Story Map & Walking Skeleton

### Backbone

A caller sends a malformed raw gRPC filter shape → embyr today panics the specific request task
(harmless to others, but shows as an alarming raw panic in Sam's own logs) → this feature adds 2
more named checks to `translate_filter` → the SAME malformed request now returns a clean, named
`INVALID_ARGUMENT`.

### Walking Skeleton

**Slice 01 (the entire feature)**: both new checks in `translate_filter`, proven end-to-end against
2 real, previously-panicking malformed `RunQuery` calls.

## Wave: DISCUSS / [REF] Elephant Carpaccio Slices

| # | Story | Release | Estimate | Learning Hypothesis | Reference Class |
|---|---|---|---|---|---|
| 01 | US-01 | 1 (Walking Skeleton, entire feature) | ≤0.25 day | Disproves: the 2 remaining malformed-filter-shape panics can be closed by extending `translate_filter`'s own already-proven Array/Map-rejection check, without any new mechanism | Mirrors `firestore-range-operator-value-type-support`'s own `translate_filter` check almost verbatim — this feature is that pattern's own direct extension |

## Wave: DISCUSS / [REF] Prioritization

A single slice — no ordering decision needed.

## Wave: DISCUSS / [REF] System Constraints

- `crates/embyr-core/`/`crates/embyr-pg-storage/` are NOT touched — confirmed by construction; the
  4 remaining `panic!` sites in `push_scalar_comparison`/`In`/`NotIn`/`ArrayContainsAny` become
  genuinely unreachable dead code after this feature, left in place as defensive fallbacks (never
  deleted — Rust's own exhaustive-match requirement still needs SOME arm for the remaining
  variants).
- `translate_filter`'s own signature is unchanged — both new checks are `if` statements inside the
  SAME existing match arm the immediately-prior feature already extended.
- Mutation-testing lesson, reapplied: unit tests for both new checks are written DURING DELIVER; a
  `cargo-mutants --in-diff` pass is still budgeted at QUALITY_GATE regardless.

## Wave: DISCUSS / [REF] User Stories

### US-01: Malformed Filter Shapes Get a Clean Rejection Instead of a Panic

**job_id**: JOB-11 | **Release**: 1 | **Persona**: P2 Sam Chen

#### Elevator Pitch
Before: a raw gRPC `RunQuery` with a non-`Array` value paired with `In`/`NotIn`/
`ArrayContainsAny`, or a `Null` value paired with a range operator, panics the specific request
task — visible in Sam's own logs as an alarming, uninformative raw panic.
After: run the identical malformed `RunQuery` → sees a clean, named `INVALID_ARGUMENT` gRPC error.
Decision enabled: Sam can immediately distinguish "a caller sent malformed input" from "the server
has an internal defect" when scanning operational logs/traces.

#### Acceptance Criteria
- [ ] AC-MFS-01: a real `RunQuery` with a non-`Array` value paired with `In` returns a clean
      `INVALID_ARGUMENT` — no panic, no transport reset.
- [ ] AC-MFS-02: same for `NotIn` and `ArrayContainsAny` (the 2 remaining operators sharing the
      identical trigger shape).
- [ ] AC-MFS-03: a real `RunQuery` with a `Null` value paired with a range operator (`LessThan`
      proven directly; `LessThanOrEqual`/`GreaterThan`/`GreaterThanOrEqual` share the identical
      code path) returns a clean `INVALID_ARGUMENT`.
- [ ] AC-MFS-04 (regression guard): every WELL-FORMED filter shape any of the 3 prior features
      fixed continues to work correctly — this feature adds no new restriction to valid queries.

## Wave: DISCUSS / [REF] Outcome KPIs

### Feature: firestore-malformed-filter-shape-validation

### Objective
Turn the 4 remaining, confirmed-harmless-to-other-tenants panics into clean, named
`INVALID_ARGUMENT` errors — an operational-clarity improvement for Sam, not a crash-risk fix (the
crash-elimination arc is already closed).

### Outcome KPIs
| KPI | Target | Measurement |
|---|---|---|
| Remaining malformed-shape panics closed with a clean rejection | 4 of 4 (`In`/`NotIn`/`ArrayContainsAny` non-array; range-operator `Null`) | Direct: AC-MFS-01 through AC-MFS-03 |
| Regression on any well-formed filter shape | 0 | AC-MFS-04 |
| Mutation-testing kill rate on the widened check | 100% effective (this session's own established bar) | `cargo-mutants --in-diff`, `--lib`-scoped |

## Wave: DISCUSS / [REF] Out of Scope

- **Any further malformed-input hardening beyond these 2 specific trigger shapes** — this feature
  closes exactly the 2 shapes named by the originating flag; a broader "audit every possible
  malformed proto shape" pass is a separate, unscoped, much larger effort with no current evidence
  of need.
- **Reframing this as a security/DoS-mitigation feature** — explicitly rejected by Resolution 1/2's
  own findings: the blast radius is confirmed self-contained to the offending caller's own request,
  never affecting other tenants or the process as a whole; this is operational log/trace clarity,
  not a security boundary.

## Wave: DISCUSS / [REF] WS Strategy

**Strategy A** (real, minimal, end-to-end) — the single slice is a real, previously-panicking
malformed `RunQuery` proven to return a clean error, not a mock.

## Wave: DISCUSS / [REF] Driving Ports

gRPC `:8080` `RunQuery` (existing route, zero new RPC).

## Wave: DISCUSS / [REF] Pre-requisites

- `firestore-range-operator-value-type-support` (FINALIZED 2026-09-06) — establishes the exact
  `translate_filter`-based rejection mechanism this feature extends.
- No new external dependency, no new bounded context.

## Wave: DISCUSS / [REF] Handoff Package

Handed to `nw-solution-architect` (DESIGN): this feature-delta.md, all 3 Resolutions (especially
Resolution 1's own persona/job reassignment finding), and the explicit instruction to design the 2
new `translate_filter` checks as a direct, minimal extension of the immediately-prior feature's own
existing Array/Map check.

## Wave: DISCUSS / [REF] SSOT Updates

`docs/product/jobs.yaml`, JOB-11 entry: append a new dated NOTE — "JOB-11 now also covers malformed
raw gRPC filter shapes (non-`Array` value paired with `In`/`NotIn`/`ArrayContainsAny`; `Null` value
paired with a range operator) producing a clean, named `INVALID_ARGUMENT` instead of a raw panic —
same job, same persona (P2 Sam Chen), not a new job, and NOT a JOB-01 (P1 Alex) extension despite
being a direct follow-up from `firestore-range-operator-value-type-support`'s own FINALIZE. Direct
investigation during this feature's own DISCUSS confirmed the trigger requires a caller to have
ALREADY constructed a raw gRPC request no real Firestore SDK could produce, and the panic's own
blast radius is confirmed self-contained to that same caller's own request (Tokio's own default
per-task panic isolation, no custom catch_unwind exists) — reframing this as an operational-clarity
concern for Sam, not a crash-risk concern for Alex, and explicitly NOT part of the 3-feature crash
-elimination arc that FINALIZE closed. See
docs/feature/firestore-malformed-filter-shape-validation/feature-delta.md § Resolution 1/2."

## Wave: DISCUSS / [REF] Definition of Ready Validation

### Requirements Completeness Score: **0.97**

### DoR Checklist (9-item hard gate)
1. [x] Every story traces to a job_id (JOB-11)
2. [x] Story has a complete Elevator Pitch
3. [x] Every AC is testable without ambiguity
4. [x] Walking Skeleton identified (US-01, entire feature)
5. [x] Scope Assessment passed
6. [x] No slice contains only `@infrastructure` stories
7. [x] Out of Scope explicitly named (2 items)
8. [x] Outcome KPIs have numeric targets and measurement methods
9. [x] Prior-wave artifacts read and reconciled (the originating flag's own severity framing was
   independently verified, not assumed, and REVISED where the investigation warranted it)

### DoR Status: **PASSED**

## Wave: DISCUSS / [REF] Open Questions

None carried forward unresolved — the one genuinely important question (is this the same severity
class as the arc it follows?) was directly investigated and resolved with a locked Resolution,
including a persona/job reassignment the investigation itself surfaced.

## Wave: DISCUSS / [REF] Wave Decisions Summary

### Key Decisions
- [D1] Persona/job reassigned to P2 Sam Chen / JOB-11 (`fair-multitenancy`), NOT P1 Alex / JOB-01 —
  no real Firestore SDK client can trigger this feature's own trigger cases (Resolution 1).
- [D2] Priority explicitly recalibrated: a small operational-clarity fix, NOT a crash-risk fix —
  the crash-elimination arc is already fully closed; this feature does not reopen or extend it
  (Resolution 2).
- [D3] Mechanism: extend `translate_filter`'s own existing check with 2 more `if` conditions,
  identical layering to the immediately-prior feature (Resolution 3).

### Requirements Summary
- Primary need: 4 confirmed-harmless-to-other-tenants panics show up as alarming, uninformative
  raw errors in Sam's own operational logs/traces instead of clean, named rejections.
- Walking skeleton scope: both new checks, proven end-to-end.
- Feature type: Backend.

### Constraints Established
- Zero change to `embyr-core`/`embyr-pg-storage`.
- `translate_filter`'s own signature unchanged.
- This feature does NOT claim to close any further "crash risk" — that arc is already closed.

### Upstream Changes
- **Persona/job reassignment** (P1 Alex/JOB-01 → P2 Sam Chen/JOB-11) — a genuine DISCUSS-wave
  finding, not an upstream contradiction; the originating flag's own candidate-id framing
  ("firestore-malformed-filter-shape-validation") never asserted JOB-01, only suggested confirming
  it — this DISCUSS confirmed the opposite and documented why.

## Wave: DISCUSS / [REF] Next Wave

**Handoff To**: nw-solution-architect (DESIGN wave)
**Deliverables**: this feature-delta.md, 3 locked Resolutions, 1-slice plan

---

## Wave: DESIGN / [REF] Prior Wave Consultation — Reading Confirmation

✓ This feature-delta.md's own DISCUSS sections in full, all 3 Resolutions.
✓ `crates/embyr-server/src/grpc/handler.rs::translate_filter`'s own existing Array/Map check
(the immediately-prior feature's own addition) — exact insertion point and pattern for this
feature's own 2 new checks, confirmed by direct read.

## Wave: DESIGN / [REF] Architecture Design

One additive change, zero new file, zero new type — 2 more `if` checks inside `translate_filter`'s
own `FieldFilter` arm, immediately after the existing Array/Map check:

```rust
if matches!(op, FilterOp::In | FilterOp::NotIn | FilterOp::ArrayContainsAny)
    && !matches!(value, FieldValue::Array(_))
{
    return Some(Err(format!(
        "{} requires an array value",
        match op { FilterOp::In => "in", FilterOp::NotIn => "not-in", _ => "array-contains-any" }
    )));
}
if matches!(
    op,
    FilterOp::LessThan | FilterOp::LessThanOrEqual | FilterOp::GreaterThan | FilterOp::GreaterThanOrEqual
) && matches!(value, FieldValue::Null)
{
    return Some(Err("range comparison operators do not support null values".to_string()));
}
```

## Wave: DESIGN / [REF] Wave Decisions Summary

### Key Decisions
- [D1] Both checks live in the SAME function, immediately following the existing Array/Map check —
  a single, cohesive "reject malformed filter shapes" block, not scattered across the function.
- [D2] Error messages name the SPECIFIC operator/value-shape mismatch (mirrors the existing Array/
  Map check's own "range comparison operators are not supported on {kind} values" naming
  convention) — never a generic "invalid filter" message.

### Constraints Established
- No new dependency, no new bounded context, no new port/adapter trait method.
- `push_scalar_comparison`'s/`ArrayContainsAny`'s/`In`'s/`NotIn`'s own remaining panics become
  genuinely unreachable after this feature — left as defensive fallbacks, not deleted (Rust's own
  exhaustive-match requirement).

## Wave: DESIGN / [REF] Next Wave

**Handoff To**: nw-software-crafter (DELIVER wave, per this project's own established convention)
**Deliverables**: this feature-delta.md's DESIGN section
