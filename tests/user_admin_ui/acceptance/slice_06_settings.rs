// SCAFFOLD: true
//! Slice 06 — Settings (OIDC Providers, Danger Zone) acceptance scenarios.
//! Story: US-011 (Settings — Account Config, OIDC, Danger Zone)
//!
//! All tests are #[ignore] (RED). Enable one at a time in DELIVER.

use embyr_admin_ui::model::{AppModel, OidcId, OidcProvider, Toast, ToastId, ToastLevel};
use embyr_admin_ui::msg::Msg;
use embyr_admin_ui::update::update;
use uuid::Uuid;

#[path = "../common/mod.rs"]
mod common;
use common::make_model_with_db;

// ─────────────────────────────────────────────────────────────────────────────
// US-011 / AC-011-02: OIDC provider list.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-011-02: OidcProviderAdded appends a new OIDC provider.
#[test]
#[ignore = "RED — implement update(_, Msg::OidcProviderAdded)"]
fn add_oidc_provider_appends_to_list() {
    // AC-011-02
    let mut m = make_model_with_db();

    let provider = OidcProvider {
        id: OidcId(Uuid::new_v4()),
        issuer: "https://accounts.google.com".to_string(),
        client_id: "client-abc-123".to_string(),
        enabled: true,
    };

    update(&mut m, Msg::OidcProviderAdded(provider.clone()));

    assert_eq!(m.oidc_providers.len(), 1, "AC-011-02: OIDC provider must be appended");
    assert!(
        m.oidc_providers.iter().any(|p| p.issuer == "https://accounts.google.com"),
        "AC-011-02: Google OIDC provider must appear in list"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// US-011 / AC-011-04: Toggle OIDC provider enabled state.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-011-04: ToggleOidc disables an enabled provider.
#[test]
#[ignore = "RED — implement update(_, Msg::ToggleOidc) disable path"]
fn toggle_oidc_disables_enabled_provider() {
    // AC-011-04
    let mut m = make_model_with_db();
    let provider_id = OidcId(Uuid::new_v4());
    m.oidc_providers.push(OidcProvider {
        id: provider_id.clone(),
        issuer: "https://accounts.google.com".to_string(),
        client_id: "client-abc".to_string(),
        enabled: true, // currently enabled
    });

    update(&mut m, Msg::ToggleOidc(provider_id.clone()));

    let p = m.oidc_providers.iter().find(|p| p.id == provider_id).unwrap();
    assert!(!p.enabled, "AC-011-04: ToggleOidc must disable an enabled provider");
}

/// AC-011-04: ToggleOidc enables a disabled provider.
#[test]
#[ignore = "RED — implement update(_, Msg::ToggleOidc) enable path"]
fn toggle_oidc_enables_disabled_provider() {
    // AC-011-04
    let mut m = make_model_with_db();
    let provider_id = OidcId(Uuid::new_v4());
    m.oidc_providers.push(OidcProvider {
        id: provider_id.clone(),
        issuer: "https://accounts.google.com".to_string(),
        client_id: "client-abc".to_string(),
        enabled: false, // currently disabled
    });

    update(&mut m, Msg::ToggleOidc(provider_id.clone()));

    let p = m.oidc_providers.iter().find(|p| p.id == provider_id).unwrap();
    assert!(p.enabled, "AC-011-04: ToggleOidc must enable a disabled provider");
}

/// Sad: ToggleOidc with non-existent OidcId is a no-op (no panic).
#[test]
#[ignore = "RED — implement ToggleOidc no-op guard"]
fn toggle_oidc_phantom_id_is_noop() {
    let mut m = make_model_with_db();
    let phantom = OidcId(Uuid::new_v4());

    update(&mut m, Msg::ToggleOidc(phantom));

    assert!(m.oidc_providers.is_empty(), "no-op: oidc_providers unchanged");
}

// ─────────────────────────────────────────────────────────────────────────────
// US-011 / AC-011-02: Remove OIDC provider.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-011-02: RemoveOidcProvider deletes the provider from the list.
#[test]
#[ignore = "RED — implement update(_, Msg::RemoveOidcProvider)"]
fn remove_oidc_provider_removes_it() {
    // AC-011-02 (delete action)
    let mut m = make_model_with_db();
    let provider_id = OidcId(Uuid::new_v4());
    m.oidc_providers.push(OidcProvider {
        id: provider_id.clone(),
        issuer: "https://github.com".to_string(),
        client_id: "gh-client".to_string(),
        enabled: true,
    });

    update(&mut m, Msg::RemoveOidcProvider(provider_id.clone()));

    assert!(
        !m.oidc_providers.iter().any(|p| p.id == provider_id),
        "AC-011-02: removed OIDC provider must be absent from list"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Toast notification scenarios (cross-cutting — used in Settings error paths).
// ─────────────────────────────────────────────────────────────────────────────

/// PushToast: Danger Zone action produces an error toast for non-reversible actions.
#[test]
#[ignore = "RED — implement update(_, Msg::PushToast)"]
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
#[ignore = "RED — implement update(_, Msg::DismissToast)"]
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
#[ignore = "RED — implement DismissToast no-op guard"]
fn dismiss_toast_phantom_id_is_noop() {
    let mut m = make_model_with_db();
    let phantom = ToastId(Uuid::new_v4());

    update(&mut m, Msg::DismissToast(phantom));

    assert!(m.toasts.is_empty(), "no-op: toasts unchanged");
}

/// Multiple OIDC providers can coexist and be toggled independently.
#[test]
#[ignore = "RED — implement ToggleOidc isolation (multiple providers)"]
fn toggle_one_oidc_provider_does_not_affect_others() {
    // AC-011-04 isolation
    let mut m = make_model_with_db();
    let id1 = OidcId(Uuid::new_v4());
    let id2 = OidcId(Uuid::new_v4());

    m.oidc_providers.push(OidcProvider {
        id: id1.clone(),
        issuer: "https://accounts.google.com".to_string(),
        client_id: "g-client".to_string(),
        enabled: true,
    });
    m.oidc_providers.push(OidcProvider {
        id: id2.clone(),
        issuer: "https://github.com".to_string(),
        client_id: "gh-client".to_string(),
        enabled: true,
    });

    // Toggle only id1.
    update(&mut m, Msg::ToggleOidc(id1.clone()));

    let p1 = m.oidc_providers.iter().find(|p| p.id == id1).unwrap();
    let p2 = m.oidc_providers.iter().find(|p| p.id == id2).unwrap();

    assert!(!p1.enabled, "toggled provider must be disabled");
    assert!(p2.enabled, "untouched provider must remain enabled");
}
