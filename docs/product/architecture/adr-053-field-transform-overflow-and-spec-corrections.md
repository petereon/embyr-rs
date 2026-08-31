# ADR-053: Integer-Overflow Default and `docs/SPEC.md` § Field Transforms Corrections

## Status

Accepted

## Context

DISCUSS escalated two open questions rather than guessing (`docs/feature/firestore-field-transforms/feature-delta.md`
§ Handoff Package):

1. **Escalation 1** — `increment`'s behavior on `i64` overflow. `docs/SPEC.md`
   is silent; DISCUSS has no confirmed real-Firestore evidence either way.
   DISCUSS's own recommendation: `checked_add` → `InvalidArgument`, "the safe
   default (never silently wrap)," pending confirmation.
2. **Escalation 2** — `docs/SPEC.md` § Field Transforms (lines 770-783,
   re-read directly for this ADR, unchanged from DISCUSS's own citation) has
   two real gaps: (a) zero rows for `maximum`/`minimum` — confirmed absent by
   a fresh full-file grep, matching DISCUSS's own finding; (b) an internal
   inconsistency — the `removeAllFromArray` row states "Returns empty array
   as transform result" (implying it POPULATES `transform_results`), while
   the `appendMissingElements` row says nothing about a transform result at
   all. Real Firestore's well-established (not locally documented) contract:
   array-transform kinds do NOT populate `transform_results` at all; only
   value-producing kinds do.

Both require a DESIGN-level decision AND, per DISCUSS's own explicit
recommendation, a real `docs/SPEC.md` edit as part of this feature's own
delivery — `docs/SPEC.md` is this codebase's own local source of truth other
features have built RPCs from directly (ADR-038/046/048 all cite it as the
wire-contract source); leaving it wrong after this feature ships would hand
the identical gap to the next feature that reads it.

## Decision

### Escalation 1 Resolution — `checked_add` → `InvalidArgument`, DISCUSS's recommendation adopted

**Adopted as designed, not re-derived**: `i64::checked_add`; `None` (overflow)
→ `CoreError::InvalidArgument("increment overflow: result exceeds i64 range")`.
Never silently wrap (`wrapping_add`) or saturate (`saturating_add`).

Reasoning, beyond DISCUSS's own recommendation (this DESIGN pass adds the
"why," not just the "what," since no re-litigation was requested but a locked
decision needs its own rationale to survive review):

1. **This feature's entire purpose is trustworthiness restoration** — the
   Elevator Pitch for US-02 is explicitly "Alex can build view counters,
   like counts, and inventory-decrement logic... that are correct under real
   concurrent traffic." A silent wraparound (`i64::MAX` + 1 → `i64::MIN`) on
   a view counter or inventory field is a WORSE failure mode than the
   silent-no-op bug this feature closes — it produces a plausible-looking but
   wrong negative number instead of an absent one, which is harder to detect
   downstream, not easier.
2. **Reuses an existing error category, not a new one** — `CoreError::InvalidArgument`
   already exists and is already the exact variant `docs/SPEC.md` documents
   for `increment`'s own non-numeric-delta case (line 780: "Returns
   InvalidArgument"). Overflow is the same category of "this specific
   operation cannot be honored as requested," not a new failure class
   needing a new variant or a new `core_error_to_status` match arm.
3. **Genuinely rare, genuinely reversible** — `i64` overflow requires
   cumulative increments summing past ~9.2 quintillion; this is not a
   realistic Trailmark `viewCount`/inventory scenario. If real-Firestore
   evidence later surfaces showing a different documented behavior (e.g.
   saturation), superseding this ADR is a single, low-blast-radius change —
   the `checked_add` call site is the only place this logic lives (ADR-052 §
   Decision 5a).
4. **Symmetric with `Maximum`/`Minimum`'s own non-numeric-existing-value
   rejection** (ADR-052 § Decision 5a) — both are "the operation cannot
   proceed without producing a value this codebase cannot represent or
   trust," rejected the same way, for the same reason.

Applies to `Increment` only — `Maximum`/`Minimum` never add (they compare and
select), so no overflow case exists for those two kinds.

### Escalation 2 Resolution — array-transform kinds never populate `transform_results`; `docs/SPEC.md` corrected

**Adopted, DISCUSS's own moderate-confidence recall confirmed as this
feature's implemented behavior**: `transform_results` is populated ONLY for
`setToServerValue`/`increment`/`maximum`/`minimum` (the four kinds that
compute a value the client could not otherwise derive). `appendMissingElements`/
`removeAllFromArray` NEVER contribute an entry, regardless of whether the
array actually changed — array membership is fully client-known (the client
sent the exact elements), so there is nothing for the server to report back
that the client doesn't already have. This resolves AC-03's own open
"transform_results shape for array ops" question (slice-03 brief, § IN
Scope) unambiguously: array-kind `FieldTransform`s always produce `None`
from `apply_field_transform` (ADR-052 § Decision 5a) — zero special-casing
needed at the response-wiring layer, since `WriteResult.transform_results`
is built by simply collecting the `Some` results in order.

**`docs/SPEC.md` § Field Transforms is corrected as part of this feature's
own delivery** (this ADR's own deliverable, not deferred) — see the exact
diff below, applied directly to `docs/SPEC.md`:

1. Add a `maximum`/`minimum` row pair, previously entirely absent.
2. Add an explicit `transform_results` column/note disambiguating which
   kinds populate it — resolves the `removeAllFromArray`/`appendMissingElements`
   asymmetry by making the rule explicit once, above the table, rather than
   repeating (and risking re-diverging) it per-row.
3. Correct the `removeAllFromArray` row: remove "Returns empty array as
   transform result" (wrong — it returns NO entry, not an empty-array entry).
4. Document the `maximum`/`minimum` missing-field semantics DISCUSS itself
   proposed (§ Numeric Type-Preservation Findings): set directly to the given
   value, NOT compared against an assumed 0/0.0 baseline (unlike `increment`).

The exact corrected table (applied verbatim to `docs/SPEC.md`, see the
companion edit to that file):

| Transform | Behavior | On missing field | Populates `transform_results`? |
|---|---|---|---|
| `setToServerValue: REQUEST_TIME` | Sets field to current server UTC timestamp. | Creates the field. | Yes — the computed timestamp. |
| `setToServerValue: <anything else>` | Returns `InvalidArgument`. Field is not modified. | — | — |
| `increment: <integer delta>` | Adds integer delta to existing integer value. Result is integer. `i64` overflow returns `InvalidArgument` (never wraps or saturates). | Treats missing as 0. | Yes — the resulting value. |
| `increment: <double delta>` | Adds double delta to existing numeric value (promotes integer to double). Result is double. | Treats missing as 0.0. | Yes — the resulting value. |
| `increment: <non-numeric delta>` | Returns `InvalidArgument`. | — | — |
| `increment` against a non-numeric existing value | Returns `InvalidArgument`. Field is not modified. | — | — |
| `maximum: <value>` | Sets field to the greater of its current value and the given value, type-preserved (promotes to double if either side is double). | Sets the field directly to the given value (not compared against 0). | Yes — the resulting value. |
| `minimum: <value>` | Sets field to the lesser of its current value and the given value, type-preserved (promotes to double if either side is double). | Sets the field directly to the given value (not compared against 0). | Yes — the resulting value. |
| `maximum`/`minimum` against a non-numeric existing value | Returns `InvalidArgument`. Field is not modified. | — | — |
| `appendMissingElements: <array>` | Merges incoming values into the current array, skipping values already present (structural equality). | Creates the field as the incoming array. | **No** — array transforms never populate `transform_results`. |
| `removeAllFromArray: <array>` | Removes all elements matching any value in the input (structural equality). | No-op; does not create the field. | **No** — array transforms never populate `transform_results`. |

## Alternatives Considered

### Escalation 1 — overflow behavior

**A. `wrapping_add` (silent wraparound).** Rejected: matches Rust's own
`i64` default overflow behavior in release builds, cheapest to implement, but
directly contradicts this feature's own purpose (§ Decision reasoning 1) —
would silently corrupt exactly the counter/inventory data `increment()`
exists to protect.

**B. `saturating_add` (clamp to `i64::MAX`/`i64::MIN`).** Considered: less
destructive than wrapping, and arguably closer to some systems' documented
behavior. Rejected: not evidenced against real Firestore either, and
`InvalidArgument` is a more honest signal — Alex's code finds out immediately
something is wrong (a request fails) rather than silently getting a "wrong
but plausible" saturated number that only degrades AT the boundary, an easy
condition to miss in testing since it never triggers until real production
scale.

**C. `checked_add` → `InvalidArgument` (chosen, DISCUSS's own recommendation).**
Never silently wrong; reuses an existing error category; the failure is
visible to the caller at the moment it happens.

### Escalation 2 — SPEC.md correction scope

**A. Leave `docs/SPEC.md` as-is, resolve only in the ADR/code.** Rejected:
directly contrary to DISCUSS's own explicit recommendation and this
codebase's own established practice (ADR-038/046/048 all treat `docs/SPEC.md`
as the load-bearing wire-contract source future features read directly, not
merely descriptive) — leaving it wrong would reproduce the identical
DISCUSS-time discovery cost for the next feature that touches field
transforms.

**B. Correct `docs/SPEC.md` as part of this feature's own delivery
(chosen).** Exact diff specified above and applied directly
(`docs/SPEC.md` § Field Transforms) — DESIGN's own deliverable, not deferred
to DELIVER to improvise.

## Consequences

**Positive**: `docs/SPEC.md` § Field Transforms becomes complete (6 of 6
kinds documented, was 4 of 6) and internally consistent (the
`transform_results` rule stated once, unambiguously, instead of
inconsistently per-row); the overflow default is documented in the same
place a future reader would look, not only in this ADR or in code comments;
zero new `CoreError` variant for either resolution.

**Negative, named explicitly**: neither resolution is verified against a
live real-Firestore instance in this sandbox — both are locked with explicit
reasoning (§ Decision) rather than empirical confirmation, matching this
session's own established residual-finding treatment (ADR-038/046/048).
Superseding either resolution, if contrary evidence surfaces, is a
single-ADR, single-call-site change (§ Decision reasoning 3).
