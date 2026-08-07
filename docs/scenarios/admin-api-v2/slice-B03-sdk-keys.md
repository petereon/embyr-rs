# Slice B-03 — SDK Key CRUD

**Feature:** admin-api-v2
**Slice:** B-03 of B-06
**Estimate:** 0.5 days
**Stories:** US-B03
**Depends on:** B-02 complete

---

## Goal

Create, list, and revoke SDK API keys via the admin UI. The most-used self-service action.

## Learning Hypothesis

Disproves: "SDK key generation via the UI bypasses the ECIES scheme, creating keys the Firestore gRPC layer cannot authenticate."  
Confirms if succeeds: A key generated via `POST .../sdk_keys` can authenticate a real `GetDocument` RPC call in the integration test.

## IN Scope

- `GET /admin/v1/projects/:id/sdk_keys` — list (any role)
- `POST /admin/v1/projects/:id/sdk_keys` — create (Owner/Admin); uses `RotateAuthKey` on BC-1 `Project` aggregate
- `DELETE /admin/v1/projects/:id/sdk_keys/:key_id` — revoke (Owner/Admin)
- Cascade: `DELETE /admin/v1/projects/:id` revokes all SDK keys in same transaction
- Integration test: generate key → call `GetDocument` gRPC → expect 200 (not 401)
- Role enforcement: Viewer → 403 on create/delete

## OUT Scope

- Key rotation window (dual-hash) — V2 enhancement

## Acceptance Criteria

- AC-B03-01 through AC-B03-07 (see feature-delta.md US-B03)

## Dependencies

- B-02 complete (project list + account scope)
- `embyr-core` `RotateAuthKey` command confirmed available

## Effort Estimate

0.5 days. Three handlers + cascade + one ECIES integration test. ECIES path already implemented in `provision.rs` — reuse pattern.
