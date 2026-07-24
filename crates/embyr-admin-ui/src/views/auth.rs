//! AuthView — sign-in form.
//!
//! Gated behind #[cfg(feature = "csr")].
//! Dispatches `Msg::SignIn` when the user clicks the sign-in button.

#[cfg(feature = "csr")]
use leptos::prelude::*;
#[cfg(feature = "csr")]
use crate::msg::Msg;

/// Auth gate view.
///
/// Renders email + password fields and a submit button.
/// On click, dispatches `Msg::SignIn` through the context callback.
#[cfg(feature = "csr")]
#[component]
pub fn AuthView() -> impl IntoView {
    let dispatch = use_context::<Callback<Msg>>().expect("dispatch context missing");

    view! {
        <div class="auth-root">
            <div class="auth-form">
                <h1 class="auth-title">"Sign in"</h1>
                <p class="auth-sub">"Welcome back. Continue to your console."</p>
                <div class="auth-fields">
                    <label class="auth-label">
                        "Email"
                        <input
                            class="auth-input"
                            type="email"
                            name="email"
                            placeholder="you@example.com"
                        />
                    </label>
                    <label class="auth-label">
                        "Password"
                        <input
                            class="auth-input"
                            type="password"
                            name="password"
                            placeholder="••••••••"
                        />
                    </label>
                </div>
                <button
                    class="auth-submit"
                    type="button"
                    on:click=move |_| dispatch.run(Msg::SignIn)
                >
                    "Continue"
                </button>
            </div>
        </div>
    }
}
