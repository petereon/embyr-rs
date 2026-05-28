# DISCUSS Decisions — embyr-agent

## Key Decisions

- [D1] Feature type: Cross-cutting — auth (mTLS), storage (all document/query/tx RPCs), change notification (Subscribe), lifecycle. Rationale: these layers cannot be shipped independently; they form one coherent agent binary.
- [D2] Walking skeleton: Brownfield (mTLS skeleton exists from step 09-01). WS = GetDocument proxied end-to-end (S01A). Rationale: proves the SaaS↔agent wiring chain with the simplest possible RPC before investing in 7 more.
- [D3] UX research depth: Lightweight (backend service, happy path focus). Rationale: user-visible behavior is identical to Firestore; the "UX" is the deployment experience (Riley) and the SDK experience (Alex, who can't see the agent at all).
- [D4] JTBD analysis: Full. JOB-04 extended; JOB-07, JOB-08, JOB-09 added to docs/product/jobs.yaml. Rationale: original JOB-04 covered credential isolation as a one-time act; it did not address daily operations, live change delivery, or audit evidence production.
- [D5] Subscribe proto gap: The `Subscribe` RPC is NOT declared in `storage_agent.proto`. Must be added in S05A. This is a blocking constraint for S05A. Decision: extend proto in the same slice that implements the RPC — no separate proto-only slice (would have no user-visible value).
- [D6] BatchGetDocuments proto gap: `BatchGetDocuments` is not in `storage_agent.proto`. DESIGN wave must choose: (a) extend proto with BatchGetDocuments RPC, or (b) implement as N parallel GetDocument calls in AgentAdapter on SaaS side. Both approaches are valid; (b) avoids proto churn if BatchGet is rare. Flagged as open design decision.
- [D7] Startup probe ordering: Postgres probe MUST complete before gRPC listener opens. Source: SPEC.md §embyr Agent §Lifecycle ("Startup: agent connects pool to Postgres, verifies WAL mode / connection, starts gRPC listener"). This ordering is a hard contract, not optional.
- [D8] DSN log discipline: DSN must not appear in any log at any level. Rationale: SPEC.md Invariant 13 ("Customer database credentials never exist in plaintext in the system database"); extended to agent logs by the same principle. Enforced by mandatory negative test AC in US-A06.
- [D9] Slice execution order: S01A → S06A → S02A → S04A → S03A → S05A. Rationale: S01A unblocks everything; S06A is second because audit invariant (DSN-not-in-logs) blocks production deployment; S04A is fourth (OCC latency is the riskiest assumption, test early); S05A is last (most novel async plumbing, pre-spike required).

## Requirements Summary

- Primary jobs: JOB-04 (credential isolation), JOB-07 (agent operations / deployment), JOB-08 (live-sync via Subscribe), JOB-09 (audit proof)
- Walking skeleton scope: S01A — GetDocument RPC end-to-end through mTLS → Postgres
- Feature type: cross-cutting (auth + storage + change notification + lifecycle)
- Slices: 6 (S01A–S06A), each ≤1 day, each with a named learning hypothesis

## Constraints Established

- mTLS is mandatory; no unauthenticated mode (SPEC.md §Security Model)
- DSN never in agent logs at any level (SPEC.md Invariant 13; negative test mandatory)
- Subscribe channel capacity = 64; overflow drops events, not blocks (SPEC.md §Subscribe)
- Startup probe gates gRPC listener (SPEC.md §Lifecycle)
- Subscribe proto and BatchGetDocuments proto gaps must be resolved before S05A and S03A respectively

## Scope Assessment: PASS

6 stories × ≤1 day = ≤6 days. 3 bounded contexts: storage adapter protocol, change notification/Subscribe, agent lifecycle. Within right-sized threshold. Walking skeleton (S01A) estimated ≤6 hours.

## Upstream Changes

- JOB-04 (docs/product/jobs.yaml): original job story is unchanged. Four forces section added. This is an additive extension, not a change to prior DISCOVER assumptions.
- JOB-07, JOB-08, JOB-09: new entries added to docs/product/jobs.yaml (embyr-agent feature).
- SPEC.md says agent behavior but does not contradict prior DISCOVER assumptions. No DISCOVER document changes required.
