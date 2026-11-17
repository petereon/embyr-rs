//! Pure update function — the heart of the TEA loop.
//!
//! Contract:
//!   - No IO, no async, no tokio::spawn, no println!
//!   - Deterministic: same (model, msg) → same new model state
//!   - Exhaustive: all Msg variants handled (enforced by compiler)

use std::collections::HashMap;

use uuid::Uuid;

use crate::api::{ApiCall, ApiError, CallKind};
use crate::model::{
    AppModel, AuthScreen, Database, DbPatch, DbStatus, InviteStep, Member, MetricsWindow, Role, Section, Toast,
    ToastId, ToastLevel,
};
use crate::msg::{Effect, Msg};

const SESSION_EXPIRED: &str = "Session expired - please sign in again";
const ACCOUNT_LOCKED: &str = "Account locked - try again in 15 minutes";
const RESET_LINK_SENT: &str = "If that address belongs to an account, a reset link is on its way. It expires in 1 hour.";
const PASSWORD_UPDATED: &str = "Password updated. Sign in with your new password.";
const ACCOUNT_CREATED: &str = "Your account is ready. Sign in with your password and an authenticator code.";
/// G-11: the session cookie is `Secure`, so a plain-HTTP non-localhost origin silently drops it.
const COOKIE_REJECTED: &str =
    "Signed in, but the browser did not keep the session. The console needs HTTPS or localhost.";

/// Apply `msg` to `model`, mutating in place, and return the side effects to run.
///
/// Pure function — no IO, no side effects of its own. All state mutation flows through here;
/// HTTP is requested as data (`Effect`), never performed.
pub fn update(model: &mut AppModel, msg: Msg) -> Vec<Effect> {
    let mut effects = Vec::new();
    match msg {
        // ── US-001: Authentication / server calls ──────────────────────────
        Msg::Boot => {
            model.booting = true;
            model.server_unreachable = false;
            effects.extend(begin(model, ApiCall::LoadSession));
        }
        Msg::Request(call) => {
            match call {
                ApiCall::SignIn { .. } => {
                    model.sign_in_error = None;
                    model.account_locked = false;
                }
                ApiCall::RequestPasswordReset { .. }
                | ApiCall::ConfirmPasswordReset { .. }
                | ApiCall::AcceptInvitation { .. }
                | ApiCall::ConfirmInvitation { .. } => {
                    model.reset_error = None;
                    model.auth_notice = None;
                }
                ApiCall::DeleteAccount { .. } => model.account_delete_error = None,
                _ => {}
            }
            effects.extend(begin(model, call));
        }
        Msg::Settled(kind) => {
            model.pending.remove(&kind);
        }
        Msg::ApiFailed { call, error } => api_failed(model, call, error),
        Msg::SessionEstablished(user) => {
            model.authed = true;
            model.booting = false;
            model.server_unreachable = false;
            model.sign_in_error = None;
            model.account_locked = false;
            model.current_user = Some(user);
            // Coming back from Stripe: land on Billing, where the outcome is shown.
            if model.checkout_return.is_some() {
                model.nav.section = Section::Billing;
            } else if model.nav.section == Section::Login {
                model.nav.section = Section::Dashboard;
            }
            effects.extend(section_loads(model));
            // The billing-hold banner shows on every screen, so the status loads once at sign-in.
            effects.extend(begin(model, ApiCall::LoadSubscription));
        }
        Msg::ShowAuth(screen) => {
            model.auth_screen = screen;
            model.reset_error = None;
            model.auth_notice = None;
            // Leaving the invitation screen forgets the authenticator secret and recovery codes.
            model.invite_step = InviteStep::Details;
        }
        Msg::InviteAccepted(enrolment) => model.invite_step = InviteStep::Enrol(enrolment),
        Msg::InviteConfirmed(codes) => model.invite_step = InviteStep::Codes(codes),
        Msg::InviteFinished => {
            model.invite_step = InviteStep::Details;
            model.auth_screen = AuthScreen::SignIn;
            model.auth_notice = Some(ACCOUNT_CREATED.into());
        }
        Msg::OpenDeleteAccount => {
            model.delete_account_modal_open = model.can_delete_account();
            model.account_delete_error = None;
        }
        Msg::CloseDeleteAccount => {
            model.delete_account_modal_open = false;
            model.account_delete_error = None;
        }
        // The server already cleared the cookie; start over from an empty model.
        Msg::AccountDeleted(purge_after) => {
            *model = AppModel::default();
            model.auth_notice = Some(format!(
                "Your account is scheduled for deletion. Its data is permanently removed after {}. Sign-in is disabled meanwhile.",
                purge_after.format("%Y-%m-%d")
            ));
        }
        Msg::PasswordResetRequested => model.auth_notice = Some(RESET_LINK_SENT.into()),
        Msg::PasswordResetDone => {
            model.auth_screen = AuthScreen::SignIn;
            model.auth_notice = Some(PASSWORD_UPDATED.into());
        }
        Msg::SignOut => {
            *model = AppModel::default();
        }

        // ── US-002: Dashboard ──────────────────────────────────────────────
        Msg::SetDatabases(dbs) => {
            // The list response has no retention; keep what the detail view already loaded.
            let retention: HashMap<_, _> = model
                .databases
                .iter()
                .filter_map(|db| db.log_retention.clone().map(|r| (db.id.clone(), r)))
                .collect();
            model.databases = dbs
                .into_iter()
                .filter(|db| db.status != DbStatus::Deleted)
                .map(|mut db| {
                    db.log_retention = db.log_retention.or_else(|| retention.get(&db.id).cloned());
                    db
                })
                .collect();
        }
        Msg::NavigateToDb(_) | Msg::NavigateToNewDb => {}
        Msg::OpenCreateDb => model.create_db_modal_open = model.can_manage_databases(),
        Msg::CloseCreateDb => model.create_db_modal_open = false,
        Msg::DatabaseLoaded(db) => match model.databases.iter_mut().find(|d| d.id == db.id) {
            Some(existing) => *existing = Database { usage: existing.usage.clone(), ..db },
            None => model.databases.push(db),
        },
        Msg::SetMetricsSummary(rows) => model.metrics_summary = rows.into_iter().collect(),
        Msg::SetDbMetrics(id, metrics) => {
            model.db_metrics.insert(id, metrics);
        }
        Msg::SetConnection(id, connection) => {
            model.connections.insert(id, connection);
        }
        Msg::ConnectionSaved(id, mode) => {
            model.connection_saves += 1;
            if let Some(db) = model.databases.iter_mut().find(|db| db.id == id) {
                db.backend_mode = mode;
            }
        }
        Msg::SecretRevealed(secret) => model.revealed_secret = Some(secret),
        Msg::CloseSecret => model.revealed_secret = None,

        // ── US-003: Database Management ────────────────────────────────────
        Msg::DatabaseCreated(db) => {
            model.databases.push(db);
        }
        Msg::DeleteDatabase(db_id) => {
            model.databases.retain(|db| db.id != db_id);
            model.api_key_info.remove(&db_id);
            model.db_metrics.remove(&db_id);
            model.metrics_summary.remove(&db_id);
            model.connections.remove(&db_id);
            model.query_logs.remove(&db_id);
            // Leave the detail view of the database that just went away (the list reloads).
            if model.nav.section == Section::DbDetail(db_id) {
                model.nav.section = Section::Databases;
                effects.extend(section_loads(model));
            }
        }
        Msg::SetDbStatus(db_id, status) => {
            if let Some(db) = model.databases.iter_mut().find(|db| db.id == db_id) {
                db.status = status;
            }
        }

        // ── US-004: Database Detail / Overview ─────────────────────────────
        Msg::SetDbLogging(db_id, enabled) => {
            if let Some(db) = model.databases.iter_mut().find(|db| db.id == db_id) {
                db.logging_enabled = enabled;
            }
        }
        Msg::SetLogRetention(db_id, retention) => {
            if let Some(db) = model.databases.iter_mut().find(|db| db.id == db_id) {
                db.log_retention = Some(retention);
            }
        }
        Msg::SetDbTab(_, _) => {}

        // ── US-007: Query Logs ─────────────────────────────────────────────
        Msg::SetQueryLogs { db, total, entries, append } => {
            let logs = model.query_logs.entry(db).or_default();
            logs.total = total;
            if append {
                logs.entries.extend(entries);
            } else {
                logs.entries = entries;
            }
        }
        Msg::HostedIdentityEnabled(db) => {
            model.hosted_identity.insert(db);
            push_toast(model, "Hosted identity is enabled for this database".into(), ToastLevel::Info);
        }

        // ── US-005: Connections ────────────────────────────────────────────
        Msg::PatchDb(db_id, patch) => {
            if let Some(db) = model.databases.iter_mut().find(|db| db.id == db_id) {
                match patch {
                    DbPatch::Dsn(_) => {
                        // No dsn field on Database in V1 model — no-op until model expanded.
                    }
                    DbPatch::AgentEndpoint(_) => {
                        // No agent_endpoint field on Database in V1 model — no-op until model expanded.
                    }
                    DbPatch::LoggingEnabled(enabled, retention) => {
                        db.logging_enabled = enabled;
                        if let Some(r) = retention {
                            db.log_retention = Some(r);
                        }
                    }
                    DbPatch::BackendMode(mode) => {
                        db.backend_mode = mode;
                    }
                    DbPatch::Suspended(suspended) => {
                        db.status = if suspended {
                            DbStatus::Suspended
                        } else {
                            DbStatus::Active
                        };
                    }
                }
            }
        }

        // ── US-006: Project API key (read-only until rotation lands) ───────
        Msg::ApiKeyInfoLoaded(db_id, info) => {
            // Unknown db_id is a no-op.
            if model.databases.iter().any(|d| d.id == db_id) {
                model.api_key_info.insert(db_id, info);
            }
        }

        // ── US-009: Members ────────────────────────────────────────────────
        Msg::SetMembers(members) => model.members = members,
        Msg::InvitationRevoked(id) => model.members.retain(|m| !(m.pending && m.id == id)),
        Msg::MemberInvited(member) => {
            model.members.push(member);
        }
        Msg::SetMemberRole(uid, new_role) => {
            let is_sole_owner = model
                .members
                .iter()
                .find(|member| member.id == uid)
                .map(|member| member.role == Role::Owner)
                .unwrap_or(false)
                && count_owners(&model.members) == 1;
            if !is_sole_owner || new_role == Role::Owner {
                if let Some(member) = model.members.iter_mut().find(|member| member.id == uid) {
                    member.role = new_role;
                }
            }
        }
        Msg::RemoveMember(uid) => {
            let is_sole_owner = model
                .members
                .iter()
                .find(|member| member.id == uid)
                .map(|member| member.role == Role::Owner)
                .unwrap_or(false)
                && count_owners(&model.members) == 1;
            if !is_sole_owner {
                model.members.retain(|member| member.id != uid);
            }
        }

        // ── US-010: Service Accounts + Admin Keys ──────────────────────────
        Msg::SetServiceAccounts(accounts) => model.service_accounts = accounts,
        Msg::SetAdminKeys(keys) => model.admin_keys = keys,
        Msg::ServiceAccountCreated(sa) => {
            model.service_accounts.push(sa);
        }
        Msg::DeleteServiceAccount(id) => {
            model.service_accounts.retain(|sa| sa.id != id);
        }
        Msg::AdminKeyCreated(key) => {
            model.admin_keys.push(key);
        }
        Msg::RevokeAdminKey(id) => {
            model.admin_keys.retain(|key| key.id != id);
        }

        // ── US-011: Settings ───────────────────────────────────────────────────
        Msg::OidcProviderAdded(provider) => {
            model.oidc_providers.push(provider);
        }
        Msg::ToggleOidc(id) => {
            if let Some(provider) = model
                .oidc_providers
                .iter_mut()
                .find(|provider| provider.id == id)
            {
                provider.enabled = !provider.enabled;
            }
        }
        Msg::RemoveOidcProvider(id) => {
            model.oidc_providers.retain(|provider| provider.id != id);
        }

        // ── Toast notifications ────────────────────────────────────────────────
        Msg::PushToast(toast) => {
            model.toasts.push(toast);
        }
        Msg::DismissToast(id) => {
            model.toasts.retain(|toast| toast.id != id);
        }

        // ── Navigation ─────────────────────────────────────────────────────────
        Msg::NavigateTo(section) => {
            model.nav.section = section;
            model.revealed_secret = None;
            effects.extend(section_loads(model));
        }

        // ── US-101..109: Billing / Payments (card-payments) ─────────────────────
        Msg::SetSubscription(sub) => {
            model.subscription = sub;
        }
        Msg::SetBilling { status, cap_status } => {
            model.billing.status = Some(status);
            model.billing.cap_status = cap_status;
            model.billing.unavailable = false;
        }
        Msg::SetUsage(rows) => model.billing.usage = Some(rows),
        Msg::RedirectToHosted(url) => effects.push(Effect::Redirect(url)),
        Msg::CheckoutReturned(outcome) => model.checkout_return = Some(outcome),
        Msg::DismissCheckoutNotice => model.checkout_return = None,
        Msg::SetInvoices(invoices) => {
            model.invoices = invoices;
        }
        Msg::SetCard(card) => {
            model.subscription.card = Some(card);
        }
        Msg::SetPlan(plan) => {
            model.subscription.plan = plan;
        }
        Msg::SetPaymentFailure(failed) => {
            model.subscription.payment_failure = failed;
        }
        Msg::OpenCardModal => {
            model.card_modal_open = true;
        }
        Msg::CloseCardModal => {
            model.card_modal_open = false;
        }
        Msg::OpenUpgradeModal => {
            model.upgrade_modal_open = true;
            model.upgrade_modal_step = crate::model::UpgradeModalStep::Compare;
        }
        Msg::CloseUpgradeModal => {
            model.upgrade_modal_open = false;
        }
        Msg::SetUpgradeModalStep(step) => {
            model.upgrade_modal_step = step;
        }
    }
    effects
}

/// Mark `call` in flight and request it. A call of the same kind already in flight is not repeated,
/// except per-database reads (`ApiCall::repeatable`).
fn begin(model: &mut AppModel, call: ApiCall) -> Option<Effect> {
    let first = model.pending.insert(call.kind());
    (first || call.repeatable()).then_some(Effect::Http(call))
}

/// The loads a section needs when it opens.
fn section_loads(model: &mut AppModel) -> Vec<Effect> {
    let calls = match model.nav.section.clone() {
        Section::Dashboard => vec![ApiCall::LoadProjects, ApiCall::LoadMetricsSummary],
        Section::Databases => vec![ApiCall::LoadProjects],
        Section::Billing => vec![ApiCall::LoadSubscription, ApiCall::LoadUsage],
        Section::Identities => vec![ApiCall::LoadMembers, ApiCall::LoadServiceAccounts],
        Section::ApiKeys => vec![ApiCall::LoadAdminKeys, ApiCall::LoadServiceAccounts],
        Section::DbDetail(id) => vec![
            ApiCall::LoadProject(id.clone()),
            ApiCall::LoadDbMetrics { id: id.clone(), window: MetricsWindow::default() },
            ApiCall::LoadConnection(id),
        ],
        _ => vec![],
    };
    calls.into_iter().filter_map(|call| begin(model, call)).collect()
}

fn api_failed(model: &mut AppModel, call: CallKind, error: ApiError) {
    let unauthorized = matches!(error, ApiError::Unauthorized(_));
    match call {
        CallKind::Session if model.booting && unauthorized => model.booting = false,
        CallKind::Session if model.booting => {
            model.booting = false;
            model.server_unreachable = true;
        }
        CallKind::Session if !model.authed => {
            model.sign_in_error = Some(if unauthorized { COOKIE_REJECTED.into() } else { error.to_string() });
        }
        CallKind::SignIn => sign_in_failed(model, error),
        CallKind::ResetRequest | CallKind::ResetConfirm | CallKind::InviteAccept | CallKind::InviteConfirm => {
            reset_failed(model, error)
        }
        CallKind::DeleteAccount => account_delete_failed(model, error),
        // G-52: without a Stripe key the first call answers 502. Say so on the Billing screen, no toast
        // (it also runs at every sign-in).
        CallKind::Subscription if matches!(error, ApiError::Upstream) => model.billing.unavailable = true,
        _ if unauthorized => {
            *model = AppModel::default();
            model.sign_in_error = Some(SESSION_EXPIRED.into());
        }
        _ => push_error_toast(model, error.to_string()),
    }
}

fn sign_in_failed(model: &mut AppModel, error: ApiError) {
    model.sign_in_error = Some(match error {
        ApiError::Throttled { retry_after_ms: Some(ms) } => {
            format!("Too many attempts, retry in {} s", ms.div_ceil(1000))
        }
        ApiError::Throttled { retry_after_ms: None } => {
            model.account_locked = true;
            ACCOUNT_LOCKED.into()
        }
        other => other.to_string(),
    });
}

/// A wrong second factor is a 401 here, not an expired session: stay on the form and show the message.
fn reset_failed(model: &mut AppModel, error: ApiError) {
    model.reset_error = Some(match error {
        ApiError::Throttled { retry_after_ms: Some(ms) } => {
            format!("Too many attempts, retry in {} s", ms.div_ceil(1000))
        }
        ApiError::Throttled { retry_after_ms: None } => ACCOUNT_LOCKED.into(),
        other => other.to_string(),
    });
}

/// A wrong password or second factor is a 401 here, not an expired session: the modal stays open.
fn account_delete_failed(model: &mut AppModel, error: ApiError) {
    model.account_delete_error = Some(match error {
        ApiError::Throttled { retry_after_ms: Some(ms) } => {
            format!("Too many attempts, retry in {} s", ms.div_ceil(1000))
        }
        ApiError::Throttled { retry_after_ms: None } => ACCOUNT_LOCKED.into(),
        other => other.to_string(),
    });
}

fn push_error_toast(model: &mut AppModel, message: String) {
    push_toast(model, message, ToastLevel::Error);
}

fn push_toast(model: &mut AppModel, message: String, level: ToastLevel) {
    model.toast_seq += 1;
    model.toasts.push(Toast { id: ToastId(Uuid::from_u128(u128::from(model.toast_seq))), message, level });
}

/// Count the number of members with the Owner role.
///
/// Used by the sole-Owner invariant guards in `SetMemberRole` and `RemoveMember`.
fn count_owners(members: &[Member]) -> usize {
    members
        .iter()
        .filter(|member| member.role == Role::Owner && !member.pending)
        .count()
}
