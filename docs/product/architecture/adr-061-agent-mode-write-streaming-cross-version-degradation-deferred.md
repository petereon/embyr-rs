# ADR-061: Cross-Version Graceful Degradation — No New Machinery for Any of the 3 Wire-Touching Agent-Mode Features (Resolves Escalation 2)

## Status

Accepted

## Context

DISCUSS's own Handoff Package (Escalation 2) names this "the single most
important finding across all 4 `backend_mode=agent` parity-gap features
investigated this session": `docs/SPEC.md` line 503 documents a POLICY ("the
agent binary version must match the embyr SaaS major version") with no
RUNTIME enforcement — `Ping` carries no version field. DISCUSS asks DESIGN to
decide whether today's implicit "clean gRPC error bubbles to the SDK caller"
is acceptable, or whether `embyr-server` should proactively detect an old
agent and surface a clearer message, "ideally in a way all 3 wire-touching
sibling features... can reuse identically."

**This is no longer a hypothetical framing.** Two sibling DESIGN waves
running concurrently this session have already reached their own decision
points and explicitly deferred to THIS feature's own resolution rather than
re-deciding independently:
- `agent-mode-list-collection-ids` (ADR-059) — adds a genuinely new unary RPC
  + message pair to `storage_agent.proto`. Its own `brief.md` summary states:
  "Cross-version graceful degradation: explicitly deferred, not
  re-litigated — resolved once, across all wire-touching sibling features, by
  `agent-mode-write-streaming`'s own DESIGN wave."
- `agent-mode-field-transforms` (ADR-057) — adds new `Transform`/
  `FieldTransform`/`ServerValue` wire types to `storage_agent.proto`. Its own
  `brief.md` summary states the same deferral: "the one open question this
  feature carries is explicitly deferred to a sibling feature's own DESIGN
  wave, not decided here."

Both of those features DO add new wire surface an old, un-upgraded
`embyr-agent` binary could genuinely lack — unlike THIS feature (ADR-060),
which adds none. This ADR therefore makes the actual, general, group-level
decision those two features are waiting on, not merely a scoped-to-`Write`
finding.

## Decision Drivers

1. **No unevidenced numbers or mechanisms (mirrors ADR-049's own Decision
   Driver 2)** — no customer complaint, support ticket, or telemetry exists
   showing a real `backend_mode=agent` customer has ever hit a
   version-skew-caused `Unimplemented`/`InvalidArgument` and been confused by
   it. Building a `protocol_version`-on-`Ping` detection-and-messaging
   mechanism against zero evidence of the problem it solves would be
   exactly the premature-machinery pattern this session's own standing
   practice (ADR-049's own Consequences, "premature optimization DISCUSS's
   own DoR note already warns against") already rejects for adjacent
   questions.
2. **gRPC's own wire protocol already produces a correct, if unpolished,
   outcome** — an `embyr-agent` binary that predates a given RPC or field
   returns `UNIMPLEMENTED`/rejects an unrecognized field cleanly, at the
   transport level, with no server-side code required to produce that
   behavior. `agent-mode-field-transforms`'s own DISCUSS/DESIGN already
   confirms its own fallback (an unrecognized `Write.operation`/transform kind
   maps to a clean `invalid_argument`, not a crash) is safe by construction.
   Neither sibling's own correctness depends on the customer receiving a
   BETTER message — only on the failure being clean, bounded, and
   non-corrupting. It already is, for both.
3. **`agent-mode-write-streaming` (ADR-060) is the WORST-positioned feature
   in this group to design a general mechanism** — it is the one feature of
   the three that adds no new wire surface at all, so it has no concrete
   "old agent lacks RPC X" scenario to design and test the mechanism
   against. Designing it here would mean designing blind, against a
   scenario this feature cannot itself exercise.
4. **Named trade-offs over invented mitigations (this session's own
   established practice)** — ADR-046's access-control-gap Consequence,
   ADR-047's unserved-segment Consequence, ADR-049's latency Consequence all
   choose to name a real, accepted gap explicitly rather than build an
   unevidenced fix. This ADR applies the identical discipline to the
   version-skew UX question.

## Decision

**No new version-negotiation or degradation machinery is built by any of the
3 wire-touching agent-mode features this session — not `agent-mode-write-streaming`
(ADR-060, which has no new wire surface to protect), not
`agent-mode-list-collection-ids` (ADR-059), not `agent-mode-field-transforms`
(ADR-057).** `Ping`/`PingRequest`/`PingResponse` are NOT modified to add a
`protocol_version` field. `embyr-server` does NOT proactively probe or
version-check a connected agent before dispatching `ListCollectionIds`,
`Write`, or a `Write` carrying `update_transforms`/`Transform`.

**Accepted v1 behavior for all three**: an `embyr-agent` binary that predates
a given feature's own wire addition produces gRPC's own natural
`Status::unimplemented` (missing RPC, `agent-mode-list-collection-ids`) or a
handler-level `Status::invalid_argument` (unrecognized field/variant silently
present but unhandled, `agent-mode-field-transforms`'s own already-designed
fallback) — bubbling to the SDK caller as an ordinary, non-corrupting gRPC
error. `agent-mode-write-streaming` itself has no version-skew exposure at
all (ADR-060 § Verification — it adds no new RPC/field an old agent could
lack).

**Named, evidence-gated follow-up candidate (not built now)**: a
customer-facing "your `embyr-agent` binary predates this capability, upgrade
to version X" message, surfaced via a `protocol_version` field on `Ping` (or
equivalent), IS a legitimate future feature — gated on real evidence
(production telemetry showing customers actually hit and are confused by
today's opaque error) rather than built speculatively. If and when that
evidence exists, it should be designed ONCE, as its own feature, informed by
whichever of the (by then, already-shipped) wire additions actually
generated the confusing error in practice — not designed today against zero
real cases.

## Alternatives Considered

**A. Build a `protocol_version` field on `Ping` now, in whichever feature
reaches DESIGN first (`agent-mode-write-streaming`, this feature).**
Rejected: this feature has no version-skew risk of its own to mitigate (§
Decision Driver 3) — building the mechanism here would size and shape a
cross-cutting concern based on a feature that doesn't exercise it, and would
itself be a new wire-protocol change, ironically undermining this feature's
own "zero new wire surface" property (ADR-060). `docs/SPEC.md`'s own
version-lockstep policy is a deployment-operations concern, not something
this ADR is positioned to redesign as a wire-protocol feature without
evidence it is needed.

**B. Assign the mechanism to one of the two siblings that DO add wire
surface (`agent-mode-list-collection-ids` or `agent-mode-field-transforms`),
since either has a concrete scenario to design against.** Rejected on the
same evidentiary grounds as Alternative A — neither feature's own
correctness or safety depends on it (§ Decision Driver 2); building it inside
either would smuggle a customer-UX feature into a parity-gap feature's own
scope without product/roadmap sign-off, and would still be speculative
relative to real customer impact.

**C. Explicitly resolve "no new machinery, name the evidence-gated follow-up,
decided once for all three" (chosen).** Matches this session's own
established convention (ADR-047's own "defer explicitly, named trade-off,
candidate follow-up" pattern) while actually closing the loop the two waiting
siblings need — a definitive "you do not need to build this to ship" answer,
not a further deferral.

## Consequences

**Positive**: `agent-mode-list-collection-ids` and `agent-mode-field-transforms`
are both unblocked to proceed to DELIVER without adding speculative
version-detection code. Zero new wire-protocol surface added anywhere for a
UX polish item with no supporting evidence. This session's own "named,
evidence-gated follow-up, not silently dropped" convention is upheld — a
future reader with real telemetry has a clear, actionable starting point
(this ADR) rather than an unrecorded assumption.

**Negative, named explicitly**: until this follow-up (if ever commissioned)
ships, a `backend_mode=agent` customer running a sufficiently stale
`embyr-agent` binary against `ListCollectionIds` or a transform-carrying
`Write`/`Commit`/`BatchWrite` sees an opaque gRPC error (`Unimplemented` or
`InvalidArgument`) with no embyr-specific guidance to redeploy their agent
binary. This is accepted, not hidden — the SAME accepted trade-off
`docs/SPEC.md`'s own documented (non-runtime-enforced) version-lockstep
policy already implies, unchanged by any of the 3 features in this group.

**Neutral**: this ADR does not amend `docs/SPEC.md` — the documented policy
(operational version lockstep, customer's own deployment responsibility)
remains accurate; no runtime mechanism is added or promised.
