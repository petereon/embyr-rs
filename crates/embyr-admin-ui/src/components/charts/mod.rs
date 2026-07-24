//! Chart components — pure SVG stubs.
//!
//! V1: all charts return a simple SVG placeholder element.
//! V2 plan: replace stub bodies with real SVG rendering logic.
//! No external charting library — bundle constraint <5 MB total.
//!
//! Gated behind #[cfg(feature = "csr")].

#[cfg(feature = "csr")]
use leptos::prelude::*;

/// Inline sparkline — a small 100×40 SVG trend indicator.
///
/// V1 stub: renders a "--" placeholder.
#[cfg(feature = "csr")]
#[component]
pub fn Sparkline() -> impl IntoView {
    view! {
        <svg width="100" height="40">
            <text x="10" y="24" class="chart-stub">"--"</text>
        </svg>
    }
}

/// Full-width latency area chart.
///
/// V1 stub: renders a "--" placeholder inside a 100%×200 SVG.
#[cfg(feature = "csr")]
#[component]
pub fn LatencyChart() -> impl IntoView {
    view! {
        <svg width="100%" height="200">
            <text x="10" y="24" class="chart-stub">"--"</text>
        </svg>
    }
}

/// Operations-per-hour bar chart.
///
/// V1 stub: renders a "--" placeholder inside a 100%×160 SVG.
#[cfg(feature = "csr")]
#[component]
pub fn BarChart() -> impl IntoView {
    view! {
        <svg width="100%" height="160">
            <text x="10" y="24" class="chart-stub">"--"</text>
        </svg>
    }
}

/// Donut chart for connection-type breakdown.
///
/// V1 stub: renders a single ring with a "--" centre label.
#[cfg(feature = "csr")]
#[component]
pub fn Donut() -> impl IntoView {
    view! {
        <svg width="130" height="130">
            <circle
                cx="65"
                cy="65"
                r="49"
                fill="none"
                stroke="var(--border)"
                stroke-width="16"
            />
            <text
                x="65"
                y="70"
                text-anchor="middle"
                class="chart-stub"
            >
                "--"
            </text>
        </svg>
    }
}
