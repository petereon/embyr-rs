//! Msg — the exhaustive message enum for the embyr admin UI TEA loop.
//!
//! Every user interaction and async data arrival dispatches one of these variants.
//! `update(&mut AppModel, Msg)` handles all variants with an exhaustive match.

use crate::api::{ApiCall, ApiError, CallKind};
use crate::model::{
    AdminKey, Card, Connection, CurrentUser, Database, DbBackendMode, DbId, DbMetrics, DbPatch, Invoice, KeyId,
    LogRetention, Member, MetricsSummaryRow, OidcId, OidcProvider, Plan, ProjectApiKeyInfo, RevealedSecret, Role,
    ServiceAccount, ServiceAccountId, Subscription, Toast, ToastId, UpgradeModalStep, UserId,
};

/// All messages that can be dispatched to the TEA update loop.
///
/// Variants are grouped by user story for navigability.
/// `Clone` is required: Leptos `Callback<Msg>` clones the value on dispatch.
#[derive(Clone, Debug)]
pub enum Msg {
    // ── US-001: Authentication / server calls (ADR-082) ────────────────────
    /// App start: check for an existing session (also the "Retry" after an unreachable server).
    Boot,
    /// A user intent that needs the server. `update` marks it pending and returns the effect.
    Request(ApiCall),
    /// A call finished (any outcome): no longer in flight.
    Settled(CallKind),
    /// A call failed. Loaded data is kept; `update` decides between a toast and sign-in.
    ApiFailed { call: CallKind, error: ApiError },
    /// `GET /auth/session` succeeded: the shell can be shown.
    SessionEstablished(CurrentUser),
    /// Switch the signed-out screen (sign in / forgot password / reset password).
    ShowAuth(crate::model::AuthScreen),
    /// Reset request was accepted (202): tell the user to check their email.
    PasswordResetRequested,
    /// Password reset confirmed: back to sign-in with a notice.
    PasswordResetDone,
    /// Server confirmed sign-out; reset the whole model to its empty default.
    SignOut,

    // ── US-002: Dashboard ──────────────────────────────────────────────────
    /// Async data load: replace database list.
    SetDatabases(Vec<Database>),
    /// Navigate to a database detail view.
    NavigateToDb(DbId),
    /// Navigate to the "New Database" form.
    NavigateToNewDb,
    /// Open the create-database modal (Owner/Admin only).
    OpenCreateDb,
    /// Close the create-database modal (also sent once a create succeeds).
    CloseCreateDb,
    /// `GET /projects/:id` (or a PATCH response): upsert the database's server-side detail.
    DatabaseLoaded(Database),
    /// `GET /metrics/summary`: replace the per-database dashboard figures.
    SetMetricsSummary(Vec<(DbId, MetricsSummaryRow)>),
    /// `GET /projects/:id/metrics`: replace one database's hourly metrics.
    SetDbMetrics(DbId, DbMetrics),
    /// `GET /projects/:id/connection`: replace one database's masked connection.
    SetConnection(DbId, Connection),
    /// `PUT /projects/:id/connection` returned 2xx: the database now runs in this mode.
    ConnectionSaved(DbId, DbBackendMode),
    /// A one-time secret arrived: show it in the reveal modal.
    SecretRevealed(RevealedSecret),
    /// The reveal modal was closed: forget the secret.
    CloseSecret,

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

    // ── US-006: Project API key (api-key-rotation, ADR-085) ────────────────
    /// Record the read-only rotation metadata of a database's API key.
    ApiKeyInfoLoaded(DbId, ProjectApiKeyInfo),

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

    // ── US-101..109: Billing / Payments (card-payments) ────────────────────
    /// Async data load: replace subscription snapshot (mock init V1;
    /// `#[server]` V2, ADR-007).
    SetSubscription(Subscription),
    /// Async data load: replace invoice history (mock init V1; `#[server]`
    /// V2, ADR-007).
    SetInvoices(Vec<Invoice>),
    /// CardModal submit: replaces (never appends) the card on file.
    /// AC-105-03/04.
    SetCard(Card),
    /// UpgradeModal confirm: change subscription plan. AC-106-02/04. No
    /// separate "clear suspension" message needed — `effective_status()`
    /// re-derives automatically. AC-106-05.
    SetPlan(Plan),
    /// TestClockCard Segmented toggle (dev-only, US-109). Mirrors the JSX
    /// prototype's `setPaymentFailure` directly.
    SetPaymentFailure(bool),
    /// Open the Card modal. Dispatched from PaymentMethodCard's Add/Update
    /// button AND from SuspensionBanner's past_due CTA — global state, not
    /// view-local (ADR-019).
    OpenCardModal,
    CloseCardModal,
    /// Open the Upgrade modal (resets step to Compare). Dispatched from
    /// PlanCard's Upgrade button AND from SuspensionBanner's
    /// free_cap_exceeded CTA (ADR-019).
    OpenUpgradeModal,
    CloseUpgradeModal,
    /// Drive the UpgradeModal's compare → confirm-downgrade step transition.
    SetUpgradeModalStep(UpgradeModalStep),
}

/// A side effect requested by `update`, executed by the runtime (`app.rs`, `csr` only).
/// Plain data: `update` stays pure and deterministic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    Http(ApiCall),
}
