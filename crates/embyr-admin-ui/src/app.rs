//! App root component — creates the Leptos reactive tree and mounts it to the DOM.
//!
//! Only compiled under the `csr` feature (WASM target).
//! Host-target tests import model/msg/update only.

#[cfg(feature = "csr")]
mod inner {
    use leptos::prelude::*;
    use crate::model::AppModel;
    use crate::msg::Msg;
    use crate::update::update as tea_update;
    use crate::views::{AuthView, ShellView};

    /// Root application component.
    ///
    /// - Creates a `RwSignal<AppModel>` from `AppModel::from_mock()`.
    /// - Creates a `Callback<Msg>` that calls the pure `update()` function.
    /// - Provides both via Leptos context so descendant components can read
    ///   the model and dispatch messages.
    /// - Renders `AuthView` when `!model.authed`, `ShellView` when authed.
    #[component]
    pub fn App() -> impl IntoView {
        let model = RwSignal::new(AppModel::from_mock());
        let dispatch: Callback<Msg> = Callback::new(move |msg: Msg| {
            model.update(|m| tea_update(m, msg));
        });

        provide_context(model);
        provide_context(dispatch);

        view! {
            <Show
                when=move || model.with(|m| m.authed)
                fallback=|| view! { <AuthView /> }
            >
                <ShellView />
            </Show>
        }
    }
}

#[cfg(feature = "csr")]
pub use inner::App;
