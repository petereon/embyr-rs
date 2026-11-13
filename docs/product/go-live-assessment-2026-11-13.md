# Go-Live Assessment — 2026-11-13

Read-only review of master (code + docs, nothing executed) across four domains.
Findings marked *(agent)* were reported by a review agent and not independently re-run; the subcollection-query finding was re-verified in `handler.rs`.
**Security & auth review: pending at time of writing — section 4 must be filled in before any go decision.**

## Verdict

| Launch type | Verdict | Why |
|---|---|---|
| General availability / self-serve paying | **NO-GO** | Core Firestore gaps, no account bootstrap, mock-only UI, no HA/release story |
| Operator-assisted design-partner beta (server-side/Admin-SDK clients, single host) | **CONDITIONAL GO** | After "Beta gate" below |

## 1. Protocol correctness & data — NO-GO for SDK users

| # | Sev | Finding |
|---|---|---|
| P1 | Blocker | **Subcollection queries return empty.** RunQuery/RunAggregationQuery/Listen scope by `from[0].collection_id` only; `req.parent` is used just for the project id (`handler.rs:3046-3128`, `listen_handler.rs:80,309`). Silent wrong results, not an error. |
| P2 | Blocker | **Listen: one collection-Query target per stream.** Documents targets rejected (`onSnapshot(doc)`); later Add/RemoveTarget ignored (multiple listeners break); orderBy/limit/offset dropped. |
| P3 | Blocker (web) | **BrowserChannel is a stub** (`rest/browser_channel.rs`, 118 lines) — web SDK fallback gets no real-time. |
| P4 | High | Aggregations: one per request, no `count up_to`; agent mode rejects SUM/AVG/OR/unary. |
| P5 | High | No PartitionQuery, vector search, TTL; `read_time`/`new_transaction` handling unclear. |
| P6 | High | Cursor values that fail conversion silently drop pagination (`handler.rs:3080-3090`); `panic!`s remain in `encoding/query.rs`, `write_stream.rs:209`. |
| P7 | Medium | No 1 MiB document / nesting-depth limits. |
| P8 | Medium | Composite-index gate maps FailedPrecondition → `unimplemented` (`handler.rs:4227`), breaking SDK index hints; ListDocuments pages in memory by offset. |
| P9 | Low | Transaction abort codes (ABORTED vs FAILED_PRECONDITION) not traced; SDK retry depends on ABORTED. |

## 2. Product surface — NO-GO for self-serve

| # | Sev | Finding |
|---|---|---|
| S1 | Blocker | No production path creates the first account/owner/TOTP (audit #63). `account-bootstrap-and-invitations` is design-only. |
| S2 | Blocker | Invitations cannot be accepted (no token/accept/revoke); invite email has no link. |
| S3 | Blocker | Admin UI is mock-only (`AppModel::from_mock()`); 52 wiring gaps unbuilt. |
| S4 | Blocker | `embyr-server` does not serve the SPA; Dockerfile doesn't `trunk build`. |
| S5 | High | OIDC callback still mounted on master with wrong-user-session bug (#58) until PR #14 merges. |
| S6 | High | DSN SSRF (#65) and dual DSN stores (#62) open; `dsn-single-store` design-only. |
| S7 | High | No password reset/recovery; no ToS, privacy policy, support contact. |
| S8 | Medium | No Stripe Checkout/payment-method collection; latency metrics hardcoded 0; no account deletion (no GDPR path); account can undo operator suspension (#64); delete-service-account leaves keys (#71). |

## 3. Operations & deployment — CONDITIONAL GO (single host only)

| # | Sev | Finding |
|---|---|---|
| O1 | High | `:v<version>` image tag overwritten each master merge; no git tags, no `:sha-` tag, empty CHANGELOG; documented rollback can't work. |
| O2 | High | 39 forward-only migrations run on every replica start; no rollback / N-1 compatibility policy. |
| O3 | High | SIGTERM: gRPC waits indefinitely on Listen streams; REST/admin drop immediately; `/healthz` doesn't flip. |
| O4 | High | Hosting topology undefined (no IaC/K8s/HA); RTO/RPO aspirational. |
| O5 | High | No load test; pool/resource sizing unvalidated; no SLO. |
| O6 | Medium | No "service down"/restart alerts; no on-call/delivery wiring. |
| O7 | Medium | No image scan/SBOM/signing; actions not SHA-pinned; ghcr visibility unverified; `single-host-deployment.md` stale. |
| O8 | Medium | Missing SMTP only warns (invites silently dropped); GCP token static. |

## 4. Security & auth — PENDING

To be filled from the security review. Known open items from the audit: #58–60 (PR #14, CI pending), #62/#65/#64, #66/#67.

## Path to launch

**Beta gate (design-partner, operator-assisted)** — smallest set:
1. Merge PR #14 (OIDC off, patch role gate, invite escalation) and PR #16.
2. Fix P1 (subcollection scope) and P2-doc-listener at minimum, or scope the beta to clients that avoid them and say so.
3. SQL seed script for first account/owner/TOTP (replaces S1/S2 for beta); manual plan setting/invoicing.
4. ToS, privacy policy, support address.
5. Immutable image tag + git tag + CHANGELOG (O1); SIGTERM drain deadline + readiness flip (O3); "service down" alerts (O6); make SMTP required in prod (O8).
6. Stopgap for SSRF (#65) before accepting real BYOC DSNs.
7. Complete the security section above.

**GA / self-serve paying** adds: P2 (full multiplexed Listen), P3, P4–P6; S1–S4 (bootstrap, UI wiring slices 1–6, serve SPA); Stripe Checkout; password reset; migration policy (O2); hosting topology + load test + SLO (O4/O5).

**Defer:** SSO, hourly metrics, self-service account deletion (handle manually), `dsn-single-store` beyond the SSRF stopgap, vector search/PartitionQuery/TTL unless a customer needs them.

## Suggested delivery order
1. P1 + P6 + P8 (small, high-impact correctness) → 2. O1/O3/O6/O8 (ops quick wins) → 3. bootstrap + UI wiring skeleton → 4. P2/P3/P4 → 5. topology/load test.
