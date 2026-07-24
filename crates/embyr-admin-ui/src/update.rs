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

        // All remaining variants are no-ops until their slices are delivered.
        _ => {}
    }
}
