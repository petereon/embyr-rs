# Slice Prioritization — embyr-rs

> Feature: embyr-rs full implementation
> Wave: DISCUSS / Phase 2.5
> Updated: 2026-05-23

---

## Execution Order

### Wave 1 — Walking Skeleton (establishes all four system layers)

| Order | Slice | Reason |
|-------|-------|--------|
| 1 | S01 gRPC + GetDocument + direct_pg | Foundation. Every other slice builds on this. Learning: is the Firestore proto encoding achievable? |
| 2 | S02 CreateDocument + UpdateDocument | Proves write path before any streaming slice. |
| 3 | S10 Admin API (direct_pg) | Unlocks real project provisioning for all subsequent slices; replaces test harness fixtures. |

### Wave 2 — Core SDK Compatibility (JOB-01)

| Order | Slice | Reason |
|-------|-------|--------|
| 4 | S03 DeleteDocument + tombstones | Completes CRUD before building query/stream on top. |
| 5 | S04 RunQuery simple | Query is a user-visible capability; validates SQL translation early. |
| 6 | S09 Transactions | OCC failure modes need early exposure; retries are subtle. |
| 7 | S05 RunQuery composite + indexes | Builds on S04; index enforcement is a hard constraint. |

### Wave 3 — Live Sync (JOB-03, highest-uncertainty slices)

| Order | Slice | Reason |
|-------|-------|--------|
| 8 | S06 Listen initial snapshot | Prerequisite for S07, S08. |
| 9 | S07 Listen live changes (NOTIFY) | Highest uncertainty (latency, fan-out); schedule early while other waves can continue if this stalls. |
| 10 | S08 Listen resume token + delta | Builds on S07; resume logic is complex. |

### Wave 4 — Transports + Lifecycle (JOB-02, JOB-06)

| Order | Slice | Reason |
|-------|-------|--------|
| 11 | S11 Project suspension | Depends on S10; operational control needed before cloud backends. |
| 12 | S12 gRPC-Web + BrowserChannel | Independent of backend work; can parallelize with Wave 5. |

### Wave 5 — Credential Modes (JOB-04, JOB-05)

| Order | Slice | Reason |
|-------|-------|--------|
| 13 | S13 embyr agent (mTLS) | Novel infra; isolate cert management risk early. |
| 14 | S14 Cloud secret (AWS + GCP) | Depends on Admin API (S10); straightforward after IAM is sorted. |

### Wave 6 — Operational Controls

| Order | Slice | Reason |
|-------|-------|--------|
| 15 | S15 Rate limiting + usage metrics | Last because it cross-cuts all operations; must be added after traffic patterns are understood. |

---

## Carpaccio Taste Tests

| Test | Result |
|------|--------|
| Any slice ships 4+ new components? | S01 ships 4 (transport, auth, adapter, encoding) — by design: it IS the walking skeleton. All others ≤2 new components. **PASS** |
| Every slice depends on a new abstraction? | S01 introduces StorageAdapter trait. S02+ implement it. No premature generalization. **PASS** |
| Every slice disproves a pre-commitment? | Yes — each slice has a named learning hypothesis (see story-map.md). **PASS** |
| Any slice uses only synthetic data? | S10 uses real Postgres migrations; S14 uses real cloud IAM. **PASS** |
| 2+ slices identical except for scale? | S04 and S05 differ by capability (simple vs composite), not just scale. **PASS** |

---

## Dependencies

```
S01 ──► S02 ──► S03
     └──► S04 ──► S05
     └──► S06 ──► S07 ──► S08
     └──► S09
     └──► S10 ──► S11
               └──► S13
               └──► S14
S12 (independent, run in parallel with Wave 4-5)
S15 (cross-cutting, run last)
```
