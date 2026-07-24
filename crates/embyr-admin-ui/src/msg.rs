// SCAFFOLD: true
//! Msg — the exhaustive message enum for the embyr admin UI TEA loop.
//!
//! Every user interaction and async data arrival dispatches one of these variants.
//! `update(&mut AppModel, Msg)` handles all variants with an exhaustive match.

use crate::model::{
    AdminKey, Database, DbId, DbPatch, KeyId, LogRetention, Member, OidcId, OidcProvider,
    Role, SdkKey, ServiceAccount, ServiceAccountId, Toast, ToastId, UserId,
};

/// All messages that can be dispatched to the TEA update loop.
///
/// Variants are grouped by user story for navigability.
/// `Clone` is required: Leptos `Callback<Msg>` clones the value on dispatch.
#[derive(Clone, Debug)]
pub enum Msg {
    // ── US-001: Authentication ─────────────────────────────────────────────
    /// User submitted the sign-in form (V1 mock: any non-empty credentials succeed).
    SignIn,
    /// User clicked "Sign Out"; session should be cleared.
    SignOut,
    /// Increment TOTP failure counter; lock if threshold reached.
    TotpFailure,
    /// Reset TOTP failure counter on success.
    TotpSuccess,

    // ── US-002: Dashboard ──────────────────────────────────────────────────
    /// Async data load: replace database list.
    SetDatabases(Vec<Database>),
    /// Navigate to a database detail view.
    NavigateToDb(DbId),
    /// Navigate to the "New Database" form.
    NavigateToNewDb,

    // ── US-003: Database Management ────────────────────────────────────────
    /// Append a newly created database.
    DatabaseCreated(Database),
    /// Remove a database (soft-delete; cascades to revoke its SDK keys).
    DeleteDatabase(DbId),
    /// Change a database's status (suspend / activate).
    SetDbStatus(DbId, crate::model::DbStatus),

    // ── US-004: Database Detail / Overview ─────────────────────────────────
    /// Toggle query logging for a database.
    SetDbLogging(DbId, bool),
    /// Set the log retention period.
    SetLogRetention(DbId, LogRetention),
    /// Switch the visible tab in the database detail view.
    SetDbTab(DbId, crate::model::DbTab),

    // ── US-005: Connections ────────────────────────────────────────────────
    /// Apply a partial update to a database's backend config.
    PatchDb(DbId, DbPatch),

    // ── US-006: SDK Keys ───────────────────────────────────────────────────
    /// Append a new SDK key for the given database.
    SdkKeyCreated { db_id: DbId, key: SdkKey },
    /// Remove an SDK key by id (revoke).
    RevokeSdkKey(DbId, KeyId),

    // ── US-007: Query Logs ─────────────────────────────────────────────────
    /// (No model-mutating msg needed beyond SetDbLogging; logs are read-only in V1.)

    // ── US-008: Billing ────────────────────────────────────────────────────
    /// (Billing uses SetDatabases as its data source in V1.)

    // ── US-009: Members ────────────────────────────────────────────────────
    /// Append a newly invited member (or pending member).
    MemberInvited(Member),
    /// Change a member's role (subject to sole-Owner invariant).
    SetMemberRole(UserId, Role),
    /// Remove a member (subject to sole-Owner invariant).
    RemoveMember(UserId),

    // ── US-010: Service Accounts + Admin Keys ──────────────────────────────
    /// Append a newly created service account.
    ServiceAccountCreated(ServiceAccount),
    /// Delete a service account (cascades to revoke its admin keys).
    DeleteServiceAccount(ServiceAccountId),
    /// Append a newly created admin API key.
    AdminKeyCreated(AdminKey),
    /// Revoke an admin API key by id.
    RevokeAdminKey(KeyId),

    // ── US-011: Settings ───────────────────────────────────────────────────
    /// Add a new OIDC provider.
    OidcProviderAdded(OidcProvider),
    /// Toggle enabled state for an OIDC provider.
    ToggleOidc(OidcId),
    /// Remove an OIDC provider.
    RemoveOidcProvider(OidcId),

    // ── Toast notifications ────────────────────────────────────────────────
    /// Push a toast to the queue.
    PushToast(Toast),
    /// Dismiss a toast by id.
    DismissToast(ToastId),

    // ── Navigation ─────────────────────────────────────────────────────────
    /// Navigate to a top-level section.
    NavigateTo(crate::model::Section),
}
