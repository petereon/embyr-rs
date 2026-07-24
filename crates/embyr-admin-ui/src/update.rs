//! Pure update function — the heart of the TEA loop.
//!
//! Contract:
//!   - No IO, no async, no tokio::spawn, no println!
//!   - Deterministic: same (model, msg) → same new model state
//!   - Exhaustive: all Msg variants handled (enforced by compiler)

use crate::model::{AppModel, NavState};
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

        // All remaining variants are no-ops until their slices are delivered.
        _ => {}
    }
}
