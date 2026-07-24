//! Pure update function — the heart of the TEA loop.
//!
//! Contract:
//!   - No IO, no async, no tokio::spawn, no println!
//!   - Deterministic: same (model, msg) → same new model state
//!   - Exhaustive: all Msg variants handled (enforced by compiler)

use crate::model::{AppModel, DbPatch, DbStatus, Member, NavState, Role};
use crate::msg::Msg;

/// Threshold for TOTP consecutive failures before account lockout.
const TOTP_LOCKOUT_THRESHOLD: u8 = 3;

/// Apply `msg` to `model`, mutating in place.
///
/// Pure function — no IO, no side effects. All state mutation flows through here.
pub fn update(model: &mut AppModel, msg: Msg) {
    match msg {
        // ── US-001: Authentication ─────────────────────────────────────────
        Msg::SignIn => {
            model.authed = true;
        }
        Msg::SignOut => {
            model.authed = false;
            model.nav = NavState::default();
            model.toasts.clear();
        }
        Msg::TotpFailure => {
            model.totp_failures = model.totp_failures.saturating_add(1);
            if model.totp_failures >= TOTP_LOCKOUT_THRESHOLD {
                model.account_locked = true;
            }
        }
        Msg::TotpSuccess => {
            model.totp_failures = 0;
            model.account_locked = false;
        }

        // ── US-002: Dashboard ──────────────────────────────────────────────
        Msg::SetDatabases(dbs) => {
            model.databases = dbs
                .into_iter()
                .filter(|d| d.status != crate::model::DbStatus::Deleted)
                .collect();
        }
        Msg::NavigateToDb(_) | Msg::NavigateToNewDb => {}

        // ── US-003: Database Management ────────────────────────────────────
        Msg::DatabaseCreated(db) => {
            model.databases.push(db);
        }
        Msg::DeleteDatabase(db_id) => {
            model.databases.retain(|d| d.id != db_id);
            model.sdk_keys.remove(&db_id);
        }
        Msg::SetDbStatus(db_id, status) => {
            if let Some(db) = model.databases.iter_mut().find(|d| d.id == db_id) {
                db.status = status;
            }
        }

        // ── US-004: Database Detail / Overview ─────────────────────────────
        Msg::SetDbLogging(db_id, enabled) => {
            if let Some(db) = model.databases.iter_mut().find(|d| d.id == db_id) {
                db.logging_enabled = enabled;
            }
        }
        Msg::SetLogRetention(db_id, retention) => {
            if let Some(db) = model.databases.iter_mut().find(|d| d.id == db_id) {
                db.log_retention = Some(retention);
            }
        }
        Msg::SetDbTab(_, _) => {}

        // ── US-005: Connections ────────────────────────────────────────────
        Msg::PatchDb(db_id, patch) => {
            if let Some(db) = model.databases.iter_mut().find(|d| d.id == db_id) {
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

        // ── US-006: SDK Keys ───────────────────────────────────────────────
        Msg::SdkKeyCreated { db_id, key } => {
            // Only append if the database exists; unknown db_id is a no-op.
            if model.databases.iter().any(|d| d.id == db_id) {
                model.sdk_keys.entry(db_id).or_default().push(key);
            }
        }
        Msg::RevokeSdkKey(db_id, key_id) => {
            if let Some(keys) = model.sdk_keys.get_mut(&db_id) {
                keys.retain(|k| k.id != key_id);
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
                .find(|m| m.id == uid)
                .map(|m| m.role == Role::Owner)
                .unwrap_or(false)
                && count_owners(&model.members) == 1;
            if !is_sole_owner || new_role == Role::Owner {
                if let Some(m) = model.members.iter_mut().find(|m| m.id == uid) {
                    m.role = new_role;
                }
            }
        }
        Msg::RemoveMember(uid) => {
            let is_sole_owner = model
                .members
                .iter()
                .find(|m| m.id == uid)
                .map(|m| m.role == Role::Owner)
                .unwrap_or(false)
                && count_owners(&model.members) == 1;
            if !is_sole_owner {
                model.members.retain(|m| m.id != uid);
            }
        }

        // All remaining variants are no-ops until their slices are delivered.
        _ => {}
    }
}

/// Count the number of members with the Owner role.
///
/// Used by the sole-Owner invariant guards in `SetMemberRole` and `RemoveMember`.
fn count_owners(members: &[Member]) -> usize {
    members.iter().filter(|m| m.role == Role::Owner).count()
}
