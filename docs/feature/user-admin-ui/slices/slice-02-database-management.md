# Slice 02 — Database Management: List, Create, Detail Overview

**Feature:** user-admin-ui  
**Slice:** 02 of 07  
**Estimate:** 2 days  
**Stories:** US-003, US-004  
**Depends on:** Slice 01 (AppModel, mock data, routing in place)

---

## Goal

Databases section fully functional with mock data: list table, create database form, suspend/activate/delete actions, database detail with Overview tab (KPI tiles, latency sparkline, ops bar chart, logging toggle).

## Learning Hypothesis

Disproves: "Self-service database creation via UI still requires operator involvement for most cases."  
Confirms if succeeds: The full create→inspect→log-enable loop is achievable without any API calls; ATDD for US-003/004 passes with mock data.

## IN Scope

- Databases list view: table with Name, Status, Backend Mode, Created, Actions
- "New Database" form: name + backend mode selector (`direct_pg` / `agent_mode`) + inline name-uniqueness validation
- `Msg::DatabaseCreated(Database)` — updates model, navigates to new database detail
- `Msg::SetDbStatus(DbId, DbStatus)` — suspend/activate with confirm modal
- `Msg::DeleteDatabase(DbId)` — delete with name-re-entry confirm modal; cascades `sdk_keys` in model
- Database detail header: database name + back breadcrumb + tab router
- Overview tab: P95 latency tile ("—"), latency sparkline (flat mock), ops bar chart (mock counts), active connections label ("—")
- Logging toggle: `Msg::SetDbLogging(DbId, bool)` + retention selector modal (1d / 7d / 30d)
- Role-based rendering: Viewer role hides Actions column and Edit controls

## OUT Scope

- Connections tab (Slice 03)
- Keys tab (Slice 03)
- Logs tab (Slice 05)
- Real chart data from metrics API (V2)
- Real create/delete/suspend server calls (V2)

## Acceptance Criteria

From US-003: AC-003-01 through AC-003-07  
From US-004: AC-004-01 through AC-004-05

## Dependencies

- Slice 01 (crate scaffold, AppModel stub, routing)
- `components/charts/` — Sparkline + BarChart components (new in this slice)
- `components/primitives/` — Modal, Badge components (new in this slice)
