# Slice 05 — Billing + Query Logs

**Feature:** user-admin-ui  
**Slice:** 05 of 07  
**Estimate:** 1 day  
**Stories:** US-007, US-008  
**Depends on:** Slice 02 (database detail tab routing for Logs tab; Billing nav link from Slice 01)

---

## Goal

Billing section shows time-range-selectable per-database usage breakdown. Query Logs tab (per-database) shows filterable log table when logging is enabled.

## Learning Hypothesis

Disproves: "Usage monitoring still drives support tickets because the numbers are too raw or not actionable."  
Confirms if succeeds: 30-day per-database breakdown + filterable query log is sufficient for P5 to make usage and cost decisions without escalating.

## IN Scope

- Billing section: time range selector (Last 7d / Last 30d / This month / Last month) + per-database breakdown table (Read Ops, Write Ops, Delete Ops, Peak Connections "—", Log Storage) + totals row
- `components/charts/` — Donut chart (optional; secondary visual if time permits; not required for ACs)
- Logs tab (per-database): empty state when logging disabled, log table (Timestamp, Operation, Collection Path, Duration ms, Status, Client), filter controls, CSV export button
- Filter state managed in AppModel (or local reactive signal — acceptable for pure-UI filter state)
- Mock: 50 synthetic log rows in `data.rs`

## OUT Scope

- Real billing data from `daily_project_metrics` (V2)
- Real log queries from `query_logs` (V2)
- Actual CSV file generation from real data (mock CSV can be stub text in V1)
- Donut chart (nice-to-have; not required for ACs)

## Acceptance Criteria

From US-007: AC-007-01 through AC-007-06  
From US-008: AC-008-01 through AC-008-05

## Dependencies

- Slice 01 (Billing nav link)
- Slice 02 (database detail Logs tab entry, logging toggle state already in model)
