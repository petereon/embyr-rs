# Slice 06 — Settings: Account Config + OIDC Providers + Danger Zone

**Feature:** user-admin-ui  
**Slice:** 06 of 07  
**Estimate:** 1 day  
**Stories:** US-011  
**Depends on:** Slice 01 (nav + AppModel oidc_providers field)

---

## Goal

Settings section fully functional with mock data: account display name edit, OIDC provider list with add/toggle/delete, Danger Zone with delete-account and transfer-ownership flows.

## Learning Hypothesis

Disproves: "OIDC SSO setup still requires operator-level API access for account owners."  
Confirms if succeeds: Owner can configure Google/GitHub OIDC in under 5 minutes via UI; masked client secret handling is correctly enforced.

## IN Scope

- Settings sections: Account, Auth Methods, Danger Zone
- Account section: display name editable → `Msg::PatchAccount` (stub)
- Auth Methods / OIDC Providers list: Issuer, Client ID (visible), Enabled toggle → `Msg::ToggleOidc`
- "Add Provider": issuer URL + client ID + client secret → `Msg::OidcProviderAdded` (mock: adds to model; client secret shown as `••••••` after save)
- Delete provider: confirm modal → removed from model
- Danger Zone — Delete Account: name re-entry confirm → `Msg::DeleteAccount` (mock: signs out + clears model)
- Danger Zone — Transfer Ownership: email input + "Send Transfer Request" → toast confirmation (email delivery V2)
- Owner-only guard: Admin/Viewer see Settings as read-only list (no edit controls rendered)

## OUT Scope

- Real OIDC provider validation (V2)
- Real client secret encryption at rest (V2 — server fn encrypts with AES-256-GCM)
- Real ownership transfer email (V2)
- Security section (session lifetime config) — deferred

## Acceptance Criteria

From US-011: AC-011-01 through AC-011-07

## Dependencies

- Slice 01 (AppModel oidc_providers: Vec<OidcProvider>, Msg::ToggleOidc, nav)
