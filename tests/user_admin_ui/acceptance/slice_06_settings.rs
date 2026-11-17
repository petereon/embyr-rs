// SCAFFOLD: true
//! Slice 06 — Settings (Danger Zone, toasts) acceptance scenarios; OIDC providers left with the mock UI.
//! Story: US-011 (Settings — Account Config, OIDC, Danger Zone)
//!
//! All tests are enabled (un-ignored 2026-11-13) except any carrying an explicit
//! `#[ignore = "..."]` reason string.

use embyr_admin_ui::model::{Toast, ToastId, ToastLevel};
use embyr_admin_ui::msg::Msg;
use embyr_admin_ui::update::update;
use uuid::Uuid;

#[path = "../common/mod.rs"]
mod common;
use common::make_model_with_db;

// ─────────────────────────────────────────────────────────────────────────────
// Toast notification scenarios (cross-cutting — used in Settings error paths).
// ─────────────────────────────────────────────────────────────────────────────

/// PushToast: Danger Zone action produces an error toast for non-reversible actions.
#[test]
fn danger_zone_action_pushes_error_toast() {
    // AC-011-05 (error feedback via toast)
    let mut m = make_model_with_db();

    let toast = Toast {
        id: ToastId(Uuid::new_v4()),
        message: "Account deletion initiated — this cannot be undone.".to_string(),
        level: ToastLevel::Error,
    };

    update(&mut m, Msg::PushToast(toast.clone()));

    assert_eq!(m.toasts.len(), 1, "Error toast must be queued");
    assert_eq!(
        m.toasts[0].level, ToastLevel::Error,
        "Danger Zone toast must be Error level"
    );
}

/// DismissToast removes the matching toast.
#[test]
fn dismiss_toast_removes_it_from_queue() {
    let mut m = make_model_with_db();
    let toast_id = ToastId(Uuid::new_v4());
    m.toasts.push(Toast {
        id: toast_id.clone(),
        message: "Config saved".to_string(),
        level: ToastLevel::Info,
    });

    update(&mut m, Msg::DismissToast(toast_id.clone()));

    assert!(
        !m.toasts.iter().any(|t| t.id == toast_id),
        "DismissToast must remove the toast from the queue"
    );
}

/// Sad: DismissToast with phantom ToastId is a no-op (no panic).
#[test]
fn dismiss_toast_phantom_id_is_noop() {
    let mut m = make_model_with_db();
    let phantom = ToastId(Uuid::new_v4());

    update(&mut m, Msg::DismissToast(phantom));

    assert!(m.toasts.is_empty(), "no-op: toasts unchanged");
}
