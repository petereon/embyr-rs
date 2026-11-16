//! Pure update function — the heart of the TEA loop.
//!
//! Contract:
//!   - No IO, no async, no tokio::spawn, no println!
//!   - Deterministic: same (model, msg) → same new model state
//!   - Exhaustive: all Msg variants handled (enforced by compiler)

use uuid::Uuid;

use crate::api::{ApiCall, ApiError, CallKind};
use crate::model::{
    AppModel, DbPatch, DbStatus, Member, Role, Section, Toast, ToastId, ToastLevel,
};
use crate::msg::{Effect, Msg};

const SESSION_EXPIRED: &str = "Session expired - please sign in again";
const ACCOUNT_LOCKED: &str = "Account locked - try again in 15 minutes";
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
            if let ApiCall::SignIn { .. } = call {
                model.sign_in_error = None;
                model.account_locked = false;
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
            if model.nav.section == Section::Login {
                model.nav.section = Section::Dashboard;
            }
            effects.extend(section_loads(model));
        }
        Msg::SignOut => {
            *model = AppModel::default();
        }

        // ── US-002: Dashboard ──────────────────────────────────────────────
        Msg::SetDatabases(dbs) => {
            model.databases = dbs
                .into_iter()
                .filter(|db| db.status != crate::model::DbStatus::Deleted)
                .collect();
        }
        Msg::NavigateToDb(_) | Msg::NavigateToNewDb => {}

        // ── US-003: Database Management ────────────────────────────────────
        Msg::DatabaseCreated(db) => {
            model.databases.push(db);
        }
        Msg::DeleteDatabase(db_id) => {
            model.databases.retain(|db| db.id != db_id);
            model.api_key_info.remove(&db_id);
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
            effects.extend(section_loads(model));
        }

        // ── US-101..109: Billing / Payments (card-payments) ─────────────────────
        Msg::SetSubscription(sub) => {
            model.subscription = sub;
        }
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

/// Mark `call` in flight and request it. A call of the same kind already in flight is not repeated.
fn begin(model: &mut AppModel, call: ApiCall) -> Option<Effect> {
    model.pending.insert(call.kind()).then_some(Effect::Http(call))
}

/// The loads a section needs when it opens.
fn section_loads(model: &mut AppModel) -> Option<Effect> {
    match model.nav.section {
        Section::Dashboard | Section::Databases => begin(model, ApiCall::LoadProjects),
        _ => None,
    }
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

fn push_error_toast(model: &mut AppModel, message: String) {
    model.toast_seq += 1;
    model.toasts.push(Toast {
        id: ToastId(Uuid::from_u128(u128::from(model.toast_seq))),
        message,
        level: ToastLevel::Error,
    });
}

/// Count the number of members with the Owner role.
///
/// Used by the sole-Owner invariant guards in `SetMemberRole` and `RemoveMember`.
fn count_owners(members: &[Member]) -> usize {
    members
        .iter()
        .filter(|member| member.role == Role::Owner)
        .count()
}
