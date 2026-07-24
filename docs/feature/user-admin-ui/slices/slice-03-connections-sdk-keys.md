# Slice 03 — Connections + SDK Keys

**Feature:** user-admin-ui  
**Slice:** 03 of 07  
**Estimate:** 1 day  
**Stories:** US-005, US-006  
**Depends on:** Slice 02 (database detail tabs routing in place)

---

## Goal

Connections tab renders backend config in read/edit mode for both `direct_pg` and `agent_mode`. Per-database Keys tab lists SDK keys and supports create (once-shown key modal) and revoke.

## Learning Hypothesis

Disproves: "Agent_mode backend configuration is too complex for a simple form UI — teams still need raw API access."  
Confirms if succeeds: Both backend modes (direct + agent with all 3 secret backends) are configurable without API knowledge; key copy-once UX is sufficient for safe credential distribution.

## IN Scope

- Connections tab Panel 1: mode-conditional fields (direct_pg: masked DSN; agent_mode: endpoint + secret backend + secret name/ARN)
- Edit mode (Owner/Admin): fields editable, Save → `Msg::PatchDb`, Cancel discards
- Info banner on save: "Config saved — takes effect on next connection"
- Connections tab Panel 2: active connections "—" with V2 badge + explanatory tooltip
- Keys tab (per-database): table with Name, Created, Last Used ("—"), Prefix, Revoke
- "Create Key" flow: name input → `Msg::SdkKeyCreated` → once-shown modal with `embyr_sdk_<32>` + clipboard copy + "I've copied the key" checkbox
- Revoke: confirm modal with irreversibility warning → `Msg::RevokeSdkKey`
- Role-based rendering: Viewer sees both tabs read-only

## OUT Scope

- Real PATCH to embyr-admin server fn (V2)
- Real key generation (mock uses uuid v4 prefix)
- Live connection counts (V2)

## Acceptance Criteria

From US-005: AC-005-01 through AC-005-05  
From US-006: AC-006-01 through AC-006-06

## Dependencies

- Slice 02 (database detail tab router)
- `components/primitives/` — Toggle, Input, Menu already scaffolded in Slice 01/02
