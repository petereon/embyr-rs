//! Keys tab — read-only "Project API key" panel (api-key-rotation, ADR-085).
//!
//! Shows when the key was last rotated and until when the previous key is
//! still accepted. Rotate / Revoke now are present but disabled until the
//! server endpoints land (slice 4).
//!
//! Gated behind #[cfg(feature = "csr")].

#[cfg(feature = "csr")]
use crate::model::{AppModel, Database};
#[cfg(feature = "csr")]
use leptos::prelude::*;

#[cfg(feature = "csr")]
fn fmt_ts(ts: Option<chrono::DateTime<chrono::Utc>>, none: &str) -> String {
    ts.map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| none.to_string())
}

/// Project API key panel for the Keys tab.
#[cfg(feature = "csr")]
#[component]
pub fn KeysView(db: Database) -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let db_id = db.id.clone();
    let info = Memo::new(move |_| {
        model.with(|m| m.api_key_info.get(&db_id).cloned().unwrap_or_default())
    });

    view! {
        <div class="keys-panel fade-in">
            <div class="row keys-header">
                <div class="col">
                    <span class="keys-title">"Project API key"</span>
                    <span class="sub">
                        "The key Firestore SDK clients use to authenticate to this database"
                    </span>
                </div>
            </div>
            <div class="card">
                <dl>
                    <dt>"Last rotated"</dt>
                    <dd>{move || fmt_ts(info.get().rotated_at, "Never")}</dd>
                    <dt>"Previous key accepted until"</dt>
                    <dd>{move || fmt_ts(info.get().previous_expires_at, "—")}</dd>
                </dl>
                <div class="row modal-footer">
                    <button class="btn btn-primary" type="button" disabled=true>"Rotate"</button>
                    <button class="btn btn-danger" type="button" disabled=true>"Revoke now"</button>
                </div>
                <p class="sub">"Rotate and Revoke now are coming soon."</p>
            </div>
        </div>
    }
}
