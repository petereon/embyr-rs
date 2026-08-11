// SCAFFOLD: true
//! Slice 06 — Invoice History acceptance scenarios.
//! Story: US-107 (Review My Invoice History)
//!
//! All tests are #[ignore] (RED). Enable one at a time in DELIVER.
//!
//! Out of model-layer test scope (view-rendering concern): the literal
//! PDF-download `<a href>` markup — this file tests the underlying
//! `InvoiceStatus` data that gates it (AC-107-02).

use embyr_admin_ui::model::{Invoice, InvoiceId, InvoiceStatus, Plan};
use embyr_admin_ui::msg::Msg;
use embyr_admin_ui::update::update;
use uuid::Uuid;

#[path = "../common/mod.rs"]
mod common;
use common::model_on_plan;

fn sample_invoice(period: &str, status: InvoiceStatus) -> Invoice {
    Invoice {
        id: InvoiceId(Uuid::new_v4()),
        date: None,
        period_label: period.to_string(),
        base: 49.0,
        overage: 18.40,
        total: 67.40,
        status,
    }
}

/// AC-107-01: a Pro-plan admin sees itemized invoice rows — Date/Period/
/// Base/Overage/Total/Status — populated via `Msg::SetInvoices`.
#[test]
fn pro_admin_sees_itemized_invoice_rows_via_set_invoices() {
    // Given: Northwind Data has 3 paid invoices and 1 upcoming invoice.
    let mut model = model_on_plan(Plan::Pro);
    let invoices = vec![
        sample_invoice("May 2026", InvoiceStatus::Paid),
        sample_invoice("Jun 2026", InvoiceStatus::Paid),
        sample_invoice("Jul 2026", InvoiceStatus::Paid),
        sample_invoice("Aug 2026", InvoiceStatus::Upcoming),
    ];

    // When: Dana navigates to Billing → Invoices.
    update(&mut model, Msg::SetInvoices(invoices));

    // Then: all 4 invoices appear, itemized.
    assert_eq!(model.invoices.len(), 4, "AC-107-01: all invoices from SetInvoices must appear");
}

/// AC-107-02: paid invoices are distinguished from the upcoming invoice by
/// status — paid rows get a PDF-download link, the upcoming row does not.
#[test]
fn paid_invoices_and_upcoming_invoice_are_distinguished_by_status() {
    // Given: Northwind Data has an upcoming invoice for the current period.
    let mut model = model_on_plan(Plan::Pro);
    let invoices = vec![
        sample_invoice("Jul 2026", InvoiceStatus::Paid),
        sample_invoice("Aug 2026", InvoiceStatus::Upcoming),
    ];

    // When: Dana views the Invoices table.
    update(&mut model, Msg::SetInvoices(invoices));

    // Then: exactly one row is Upcoming (no PDF link) and one is Paid (PDF
    // link present).
    let upcoming_count = model.invoices.iter().filter(|i| i.status == InvoiceStatus::Upcoming).count();
    let paid_count = model.invoices.iter().filter(|i| i.status == InvoiceStatus::Paid).count();
    assert_eq!(upcoming_count, 1, "AC-107-02: exactly one upcoming row, no PDF link");
    assert_eq!(paid_count, 1, "AC-107-02: exactly one paid row, PDF link present");
}

/// AC-107-03: a Free-plan admin who has never been on a paid plan sees the
/// documented no-charges empty state, not a table.
#[test]
fn free_plan_admin_with_no_invoice_history_sees_empty_state() {
    // Given: Aperture Labs has never been on a paid plan.
    let model = model_on_plan(Plan::Free);
    assert!(model.invoices.is_empty(), "precondition: no invoice history");

    // When: Chris navigates to Billing → Invoices.
    let empty_state = model.invoices_empty_state();

    // Then: the documented empty-state copy renders instead of a table.
    assert!(empty_state.is_some(), "AC-107-03: empty-state copy must render for no invoice history");
}

/// AC-107-04: invoice history survives a plan downgrade — Solstice
/// Analytics' 2 historical Pro-plan invoices are still listed after
/// downgrading to Free.
#[test]
fn invoice_history_persists_across_plan_downgrade() {
    // Given: Solstice Analytics has 2 historical Pro-plan invoices.
    let mut model = model_on_plan(Plan::Pro);
    let invoices = vec![
        sample_invoice("Mar 2026", InvoiceStatus::Paid),
        sample_invoice("Apr 2026", InvoiceStatus::Paid),
    ];
    update(&mut model, Msg::SetInvoices(invoices));

    // When: the account downgrades to Free (Slice 05's SetPlan).
    update(&mut model, Msg::SetPlan(Plan::Free));

    // Then: both historical invoices are still listed.
    assert_eq!(
        model.invoices.len(), 2,
        "AC-107-04: invoice history must not be cleared by a plan downgrade"
    );
}

/// AC-107-03, Error/Boundary: a Free-plan account that HAS invoice history
/// (post-downgrade, per AC-107-04) must NOT show the empty state — the
/// empty-state condition is "no invoices," not "plan == Free" alone.
#[test]
fn account_with_invoice_history_does_not_show_empty_state_even_on_free() {
    // Given: Solstice Analytics is on Free but retains 2 historical
    // invoices (chained from the persistence scenario above).
    let mut model = model_on_plan(Plan::Pro);
    update(&mut model, Msg::SetInvoices(vec![
        sample_invoice("Mar 2026", InvoiceStatus::Paid),
        sample_invoice("Apr 2026", InvoiceStatus::Paid),
    ]));
    update(&mut model, Msg::SetPlan(Plan::Free));

    // When: Priya navigates to Billing → Invoices.
    let empty_state = model.invoices_empty_state();

    // Then: the table renders (empty-state copy does NOT show).
    assert!(
        empty_state.is_none(),
        "AC-107-03: a Free-plan account WITH invoice history must not show the empty state"
    );
}
