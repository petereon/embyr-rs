# Go-Live Assessment — 2026-11-13

Read-only review of master (code + docs, nothing executed) across four domains.
Findings marked *(agent)* were reported by a review agent and not independently re-run; the subcollection-query finding was re-verified in `handler.rs`.
Security findings are agent-reported (code-read, not exercised) except where noted.

## Verdict

| Launch type | Verdict | Why |
|---|---|---|
| General availability / self-serve paying | **NO-GO** | Core Firestore gaps, security-rules bypasses, no account bootstrap, mock-only UI, no HA/release story |
| Operator-assisted design-partner beta (server-side/Admin-SDK clients, single host) | **CONDITIONAL GO** | Only after "Beta gate" below, including the security blockers |

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

## 4. Security & auth — NO-GO for serving real end users

| # | Sev | Finding |
|---|---|---|
| X1 | Blocker | **Path-pattern / recursive-wildcard rules (ADR-063/064) enforced only on GetDocument, Create, Update, Delete, Listen.** ListDocuments (`handler.rs:2369`), BatchGet (`:2618`), RunQuery/RunAggregationQuery (`:3206`) and all of Commit/BatchWrite/Write stream (`evaluate_write_rule_for_commit`, `:919`) look up exact-match rules only. Commit is what SDK `setDoc` uses. A project protecting nested collections with wildcard rules is open to anyone with the API key. |
| X2 | Blocker | **Project API key cannot be rotated or revoked.** Nothing writes `api_key_hash_previous`; `sdk_api_keys` rows are never read by data-plane `authenticate()` (`system_db.rs:270`), so SDK keys cannot authenticate and revoking one does nothing. DSN is encrypted under an API-key-derived key, so re-keying = re-provisioning. |
| X3 | High | **REST accounts handlers run Argon2 inline on async threads** (`rest/sign_in_with_password.rs:164`, `adapters/project_auth.rs:112`, sign_up, reset_password) with no `spawn_blocking`. The Argon2 gate (PR #11) blocks on a `Condvar`, so past N=cores concurrent requests tokio workers stall — a wrong API key suffices; freezes `/healthz`/gRPC. gRPC path is fine. Fix: wrap in `spawn_blocking`. *(Interaction with the gate added in PR #11 — verify.)* |
| X4 | High | **Rate limiter runs before auth, keyed by project_id only** (`handler.rs:1296`, `rest_rate_limit_middleware`): an unauthenticated caller who knows a project ID drains its bucket and locks out real users. |
| X5 | High | No per-account throttle on end-user sign-in (credential stuffing up to project limit); `sendOobCode` unthrottled (email bombing). |
| X6 | High | **Suspension/deletion/DSN changes don't reach other replicas.** `CredentialCache` is a per-process LRU with no TTL; `evict_project` is local; `patch_project` evicts only on DSN change; dunning suspension same. |
| X7 | High | Audit #58–60, #64, #65 still open on master (fix for #58–60 is PR #14, unmerged). JWKS fetched with no timeout. |
| X8 | Medium | Admin sign-in throttle keyed on TCP peer IP (`handlers/auth.rs:162`); behind an LB all admins share one bucket → one attacker locks everyone out. No X-Forwarded-For handling. |
| X9 | Medium | No rules = unrestricted access (not Firestore's deny-by-default); `ListCollectionIds` (`handler.rs:2463`) never checks rules (leaks collection names). |
| X10 | Medium | Customer DB TLS not enforced (no `sslmode` handling); DSN host/port unvalidated (#65). |
| X11 | Medium | Each REST accounts request on a credential-cache hit builds a new Postgres pool to the customer DB (`project_auth.rs:92`) — handshake per sign-in, adds to DoS surface. |
| X12 | Medium | RunQuery/aggregation pass empty `claims` (`handler.rs:3183,3226`): custom-claim rules can't work on queries (fails closed — correctness, not bypass). |
| X13 | Low | Open Listen streams don't re-evaluate on rule change or identity/project revocation; reset token consumed before password-strength check; hosted tokens (1 h) not revoked on password reset; API key in `?key=` query string hits LB logs; TOTP no replay guard, non-atomic failure counter; `EMBYR_ADMIN_KEY` no minimum length, operator routes unthrottled; admin keys accept unchecked `member_id` (Admin can make an Owner service account); cross-account project IDs give 403 vs 404 (existence oracle). |

Verified OK: operator/session middleware (constant-time compare), RBAC on session routes except #59/#60, ownership scoping, admin cookie flags (HttpOnly/Secure/SameSite=Strict/Path=/admin), Stripe webhook HMAC + bounded body, ECIES KDF binds both public keys, JWT alg pinned + audience checked, admin sign-in Argon2 uses `spawn_blocking`, CORS default-deny, no secrets in logs.

## Path to launch

**Beta gate (design-partner, operator-assisted)** — smallest set:
1. Merge PR #14 (OIDC off, patch role gate, invite escalation) and PR #16.
2. Fix P1 (subcollection scope) and P2-doc-listener at minimum, or scope the beta to clients that avoid them and say so.
3. SQL seed script for first account/owner/TOTP (replaces S1/S2 for beta); manual plan setting/invoicing.
4. ToS, privacy policy, support address.
5. Immutable image tag + git tag + CHANGELOG (O1); SIGTERM drain deadline + readiness flip (O3); "service down" alerts (O6); make SMTP required in prod (O8).
6. Stopgap for SSRF (#65) before accepting real BYOC DSNs.
7. Security blockers/highs: X1 (rules on every read/query/commit path), X2 (key rotation/revocation the data plane honours), X3 (`spawn_blocking` for REST Argon2), X4 (auth before spending project tokens, or pre-auth limiter), X6 (credential-cache TTL/invalidation), X7 (merge PR #14).

**GA / self-serve paying** adds: P2 (full multiplexed Listen), P3, P4–P6; S1–S4 (bootstrap, UI wiring slices 1–6, serve SPA); Stripe Checkout; password reset; migration policy (O2); hosting topology + load test + SLO (O4/O5).

**Defer:** SSO, hourly metrics, self-service account deletion (handle manually), `dsn-single-store` beyond the SSRF stopgap, vector search/PartitionQuery/TTL unless a customer needs them.

## Suggested delivery order
0. X1, X3, X4, X6 (security; X3 is a small change) →
1. P1 + P6 + P8 (small, high-impact correctness) → 2. O1/O3/O6/O8 (ops quick wins) → 3. bootstrap + UI wiring skeleton → 4. P2/P3/P4 → 5. topology/load test.
