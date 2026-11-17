//! Danger zone — delete a database (`DELETE /projects/:id`, Owner only).
//!
//! Soft delete: the database's keys stop working at once and the data is purged after the
//! server's grace window. The confirm button stays disabled until the database id is typed
//! exactly. The model changes only after the server's 2xx (AC-AUW-19): a failure leaves the modal
//! open and shows a toast.

#[cfg(feature = "csr")]
use crate::api::{ApiCall, CallKind};
#[cfg(feature = "csr")]
use crate::components::primitives::Modal;
#[cfg(feature = "csr")]
use crate::model::{confirms_delete, AppModel, DbId};
#[cfg(feature = "csr")]
use crate::msg::Msg;
#[cfg(feature = "csr")]
use leptos::prelude::*;

#[cfg(feature = "csr")]
#[component]
pub fn DangerZone(id: DbId) -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    let can_delete = move || model.with(|m| m.can_delete_databases());
    let deleting = move || model.with(|m| m.pending.contains(&CallKind::DeleteProject));
    let open = RwSignal::new(false);
    let typed = RwSignal::new(String::new());

    let close = move || {
        open.set(false);
        typed.set(String::new());
    };
    // `Copy` handle: the `Show` body below must be `Fn`, so it cannot move an owned id out.
    let id = StoredValue::new(id);
    let matches = move || confirms_delete(&typed.get(), &id.get_value());

    view! {
        <div class="card" style="margin-top:16px;border-color:color-mix(in oklab, var(--red) 30%, var(--border))">
            <div class="row" style="padding:15px 20px;gap:12px;align-items:center">
                <div class="col" style="gap:2px;flex:1">
                    <span style="font-weight:550;font-size:13.5px;color:var(--red)">"Delete database"</span>
                    <span style="font-size:12px;color:var(--text-3)">
                        {move || if can_delete() {
                            "Its API keys stop working immediately. Only the account Owner can do this."
                        } else {
                            "Only the account Owner can delete a database."
                        }}
                    </span>
                </div>
                <button
                    class="btn btn-danger btn-sm"
                    type="button"
                    disabled=move || !can_delete()
                    on:click=move |_| open.set(true)
                >"Delete database"</button>
            </div>
        </div>

        <Show when=move || open.get() fallback=|| ()>
            <Modal title="Delete database" on_close=Callback::new(move |_| close())>
                <p style="color:var(--text-2);font-size:13.5px;line-height:1.55;margin:0">
                    "This deletes "<strong class="mono">{move || id.get_value().0}</strong>
                    ". Its API keys stop working immediately and its data is purged after the retention window. \
                     Type the database id to confirm."
                </p>
                <div class="modal-field">
                    <input class="modal-input mono" type="text" autocomplete="off" spellcheck="false"
                        prop:value=move || typed.get()
                        on:input=move |e| typed.set(event_target_value(&e))/>
                </div>
                <div class="modal-footer">
                    <button class="btn btn-ghost" type="button" on:click=move |_| close()>"Cancel"</button>
                    <button
                        class="btn btn-danger"
                        type="button"
                        disabled=move || deleting() || !can_delete() || !matches()
                        on:click=move |_| dispatch.run(Msg::Request(ApiCall::DeleteProject(id.get_value())))
                    >{move || if deleting() { "Deleting…" } else { "Delete database" }}</button>
                </div>
            </Modal>
        </Show>
    }
}
