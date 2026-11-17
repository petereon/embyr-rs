//! CreateDatabaseModal — `POST /projects` (Owner/Admin). Open state is `model.create_db_modal_open`.
//!
//! The form offers `direct_pg` and `agent` (G-06; secret-manager modes are set up outside the
//! console). The DSN / PEMs live only in this component's local signals and vanish with it when the
//! modal closes; they never enter the model. A success closes the modal and the key comes back in
//! the shared reveal-once modal; a failure leaves the form as typed and the error shows as a toast.

#[cfg(feature = "csr")]
use crate::api::{is_valid_project_id, ApiCall, CallKind, ConnectionConfig, NewProject};
#[cfg(feature = "csr")]
use crate::components::primitives::Modal;
#[cfg(feature = "csr")]
use crate::model::AppModel;
#[cfg(feature = "csr")]
use crate::msg::Msg;
#[cfg(feature = "csr")]
use leptos::prelude::*;

#[cfg(feature = "csr")]
#[component]
pub fn CreateDatabaseModal() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let is_open = move || model.with(|m| m.create_db_modal_open);

    // `Show` unmounts the form on close, so every typed secret is dropped with it.
    view! {
        <Show when=is_open fallback=|| ()>
            <CreateDatabaseForm />
        </Show>
    }
}

#[cfg(feature = "csr")]
#[component]
fn CreateDatabaseForm() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let creating = move || model.with(|m| m.pending.contains(&CallKind::CreateProject));

    let name = RwSignal::new(String::new());
    let project_id = RwSignal::new(String::new());
    let agent = RwSignal::new(false);
    let dsn = RwSignal::new(String::new());
    let endpoint = RwSignal::new(String::new());
    let ca_pem = RwSignal::new(String::new());
    let client_cert_pem = RwSignal::new(String::new());
    let client_key_pem = RwSignal::new(String::new());

    let new_project = move || NewProject {
        project_id: project_id.get(),
        name: name.get(),
        config: if agent.get() {
            ConnectionConfig::Agent {
                endpoint: endpoint.get(),
                ca_pem: ca_pem.get(),
                client_cert_pem: client_cert_pem.get(),
                client_key_pem: client_key_pem.get(),
            }
        } else {
            ConnectionConfig::DirectPg { dsn: dsn.get() }
        },
    };
    let id_invalid = move || {
        let id = project_id.get();
        !id.is_empty() && !is_valid_project_id(&id)
    };

    view! {
        <Modal title="New database" on_close=Callback::new(move |_| dispatch.run(Msg::CloseCreateDb))>
            <div class="modal-field">
                <label class="modal-label">"Name (optional)"</label>
                <input class="modal-input" type="text" placeholder="Shop" maxlength="100"
                    prop:value=move || name.get()
                    on:input=move |e| name.set(event_target_value(&e))/>
            </div>
            <div class="modal-field">
                <label class="modal-label">"Database id"</label>
                <input
                    class=move || if id_invalid() { "modal-input mono input-err" } else { "modal-input mono" }
                    type="text" placeholder="shop" maxlength="63" autocomplete="off" spellcheck="false"
                    prop:value=move || project_id.get()
                    on:input=move |e| project_id.set(event_target_value(&e))/>
                <span style="font-size:11.5px;color:var(--text-3)">
                    "Lowercase letters, digits and hyphens, starting with a letter. Cannot be changed later."
                </span>
            </div>

            <div class="row" style="gap:8px">
                <button
                    class=move || if !agent.get() { "btn btn-primary btn-sm" } else { "btn btn-ghost btn-sm" }
                    type="button"
                    on:click=move |_| agent.set(false)
                >"Direct PG"</button>
                <button
                    class=move || if agent.get() { "btn btn-primary btn-sm" } else { "btn btn-ghost btn-sm" }
                    type="button"
                    on:click=move |_| agent.set(true)
                >"Agent mode"</button>
            </div>

            {move || if agent.get() {
                view! {
                    <div class="modal-field">
                        <label class="modal-label">"Agent endpoint"</label>
                        <input class="modal-input mono" type="text" placeholder="10.0.0.5:9191"
                            autocomplete="off" spellcheck="false"
                            prop:value=move || endpoint.get()
                            on:input=move |e| endpoint.set(event_target_value(&e))/>
                    </div>
                    <div class="modal-field">
                        <label class="modal-label">"CA certificate (PEM)"</label>
                        <textarea class="input mono" rows="3" spellcheck="false"
                            prop:value=move || ca_pem.get()
                            on:input=move |e| ca_pem.set(event_target_value(&e))/>
                    </div>
                    <div class="modal-field">
                        <label class="modal-label">"Client certificate (PEM)"</label>
                        <textarea class="input mono" rows="3" spellcheck="false"
                            prop:value=move || client_cert_pem.get()
                            on:input=move |e| client_cert_pem.set(event_target_value(&e))/>
                    </div>
                    <div class="modal-field">
                        <label class="modal-label">"Client key (PEM)"</label>
                        <textarea class="input mono" rows="3" spellcheck="false"
                            prop:value=move || client_key_pem.get()
                            on:input=move |e| client_key_pem.set(event_target_value(&e))/>
                    </div>
                }
                .into_any()
            } else {
                view! {
                    <div class="modal-field">
                        <label class="modal-label">"Postgres DSN"</label>
                        <input class="modal-input mono" type="password"
                            placeholder="postgres://user:pass@host:5432/db"
                            autocomplete="off" spellcheck="false"
                            prop:value=move || dsn.get()
                            on:input=move |e| dsn.set(event_target_value(&e))/>
                    </div>
                }
                .into_any()
            }}

            <p style="font-size:11.5px;color:var(--text-3);margin:0">
                "Creating connects to the backend first. The API key is shown once, right after."
            </p>
            <div class="modal-footer">
                <button class="btn btn-ghost" type="button"
                    on:click=move |_| dispatch.run(Msg::CloseCreateDb)>"Cancel"</button>
                <button
                    class="btn btn-primary"
                    type="button"
                    disabled=move || creating() || !new_project().is_ready()
                    on:click=move |_| dispatch.run(Msg::Request(ApiCall::CreateProject(new_project())))
                >{move || if creating() { "Creating…" } else { "Create database" }}</button>
            </div>
        </Modal>
    }
}
