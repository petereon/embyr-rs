//! InvoicesTab — invoice history table with empty-state fallback.
//!
//! Gated behind #[cfg(feature = "csr")].
//!
//! AC-107-01: renders `model.invoices` as a Date/Period/Base/Overage/Total/
//! Status table when non-empty.
//! AC-107-02: a PDF-download affordance renders only on rows where
//! `status == InvoiceStatus::Paid` (Upcoming invoices have no PDF yet).
//! AC-107-03: `AppModel::invoices_empty_state()` copy renders instead of the
//! table for Free-plan accounts with no invoice history.

#[cfg(feature = "csr")]
use crate::model::{AppModel, InvoiceStatus};
#[cfg(feature = "csr")]
use leptos::prelude::*;

/// Invoices tab content — itemized invoice table, or empty-state copy.
#[cfg(feature = "csr")]
#[component]
pub fn InvoicesTab() -> impl IntoView {
    let model = use_context::<RwSignal<AppModel>>().expect("model context missing");
    let empty_copy = move || model.with(|m| m.invoices_empty_state());
    let invoices = move || model.with(|m| m.invoices.clone());

    view! {
        {move || match empty_copy() {
            Some(copy) => view! { <p class="card-sub">{copy}</p> }.into_any(),
            None => view! {
                <div class="tbl-wrap">
                    <table class="tbl">
                        <thead>
                            <tr>
                                <th>"Date"</th>
                                <th>"Period"</th>
                                <th>"Base"</th>
                                <th>"Overage"</th>
                                <th>"Total"</th>
                                <th>"Status"</th>
                                <th></th>
                            </tr>
                        </thead>
                        <tbody>
                            {move || {
                                invoices()
                                    .into_iter()
                                    .map(|inv| {
                                        let is_paid = inv.status == InvoiceStatus::Paid;
                                        view! {
                                            <tr>
                                                <td class="mono">
                                                    {inv.date.map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default()}
                                                </td>
                                                <td>{inv.period_label}</td>
                                                <td class="mono">{format!("${:.2}", inv.base)}</td>
                                                <td class="mono">{format!("${:.2}", inv.overage)}</td>
                                                <td class="mono">{format!("${:.2}", inv.total)}</td>
                                                <td>
                                                    <span class=if is_paid { "badge badge-green" } else { "badge badge-neutral" }>
                                                        {if is_paid { "Paid" } else { "Upcoming" }}
                                                    </span>
                                                </td>
                                                <td class="td-actions">
                                                    {is_paid.then(|| view! {
                                                        <a
                                                            class="btn btn-ghost btn-sm"
                                                            href="#"
                                                            title="Download PDF"
                                                            on:click=move |e| e.prevent_default()
                                                        >
                                                            "⬇ PDF"
                                                        </a>
                                                    })}
                                                </td>
                                            </tr>
                                        }
                                    })
                                    .collect_view()
                            }}
                        </tbody>
                    </table>
                </div>
            }.into_any(),
        }}
    }
}
