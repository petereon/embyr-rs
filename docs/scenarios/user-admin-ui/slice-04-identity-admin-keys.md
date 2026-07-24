# Slice 04 — Identity Management + Admin API Keys

**Feature:** user-admin-ui  
**Slice:** 04 of 07  
**Estimate:** 2 days  
**Stories:** US-009, US-010  
**Depends on:** Slice 01 (sidebar nav, AppModel members/service_accounts/admin_keys fields)

---

## Goal

Identities section with Members and Service Accounts tabs fully functional. Account-level API Keys section with admin key create/revoke. All with mock data; role-gated controls.

## Learning Hypothesis

Disproves: "Team onboarding still requires operator involvement or raw API access after the UI ships."  
Confirms if succeeds: Invite → join → role change → remove flow is self-contained within the UI; sole-owner invariant is enforced by UI alone.

## IN Scope

- Identities section: Members tab + Service Accounts tab (tab router)
- Members table: Email, Display Name, Role, Auth Method, MFA Enabled, Last Login, Actions
- Invite flow: email + role selector → `Msg::MemberInvited` (mock: adds pending member to model; V2 sends email)
- Pending invitees shown with "Pending" last-login; 7-day expiry label
- Role change: dropdown → `Msg::SetMemberRole`; sole-owner guard (disable action + tooltip)
- Remove: confirm modal → `Msg::RemoveMember`; sole-owner guard
- Service Accounts table: Name, Description, Role, Created, Last Used, Actions
- Create SA: name + description + role → `Msg::ServiceAccountCreated`
- Delete SA: confirm modal → `Msg::DeleteServiceAccount` + cascade admin key revocations in model
- API Keys section (account-level): same table + create/revoke UX as SDK keys (US-006 pattern), `embyr_adm_<32>` format
- `Msg::AdminKeyCreated`, `Msg::RevokeAdminKey`
- Role-based rendering: Viewer sees all tables read-only; Admin cannot change Owner role

## OUT Scope

- Actual invitation email (V2 — `IEmailSender` adapter)
- Real session invalidation on member removal (V2)
- OIDC provider linking to service accounts (not in spec)

## Acceptance Criteria

From US-009: AC-009-01 through AC-009-07  
From US-010: AC-010-01 through AC-010-07

## Dependencies

- Slice 01 (AppModel members/service_accounts/admin_keys fields + mock data in data.rs)
- `components/primitives/` — Tabs component (new in this slice if not already added)
