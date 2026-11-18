//! App root component and effect runtime (ADR-082).
//!
//! Only compiled under the `csr` feature (WASM target).
//! Host-target tests import api/model/msg/update only.

#[cfg(feature = "csr")]
mod inner {
    use crate::api::{
        decode, decode_network_failure, to_request, HttpRequest, HttpResponse, Method,
    };
    use crate::components::ToastStack;
    use crate::api::{
        checkout_return_from_query, invite_token_from_query, reset_token_from_query, sso_provider_from_query,
        sso_return_from_query, ApiCall, SsoIntent,
    };
    use crate::model::{AppModel, AuthScreen, CheckoutReturn, SsoReturn};
    use crate::msg::{Effect, Msg};
    use crate::update::update as tea_update;
    use crate::views::{AcceptInviteView, AuthView, ForgotPasswordView, ResetPasswordView, ShellView};
    use leptos::prelude::*;
    use leptos::task::spawn_local;
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;

    /// Run `msg` through the pure `update`, then execute the effects it asks for. Results come back
    /// as further messages.
    fn dispatch(model: RwSignal<AppModel>, msg: Msg) {
        let effects = model.try_update(|m| tea_update(m, msg)).unwrap_or_default();
        for effect in effects {
            match effect {
                Effect::Http(call) => spawn_local(async move {
                    let msgs = match fetch(to_request(&call)).await {
                        Ok(response) => decode(&call, response),
                        Err(reason) => decode_network_failure(&call, reason),
                    };
                    for msg in msgs {
                        dispatch(model, msg);
                    }
                }),
                Effect::Redirect(url) => {
                    if let Some(window) = web_sys::window() {
                        let _ = window.location().set_href(&url);
                    }
                }
            }
        }
    }

    /// Same-origin fetch: the HttpOnly session cookie is sent by default; the JS side never sees it.
    async fn fetch(request: HttpRequest) -> Result<HttpResponse, String> {
        let js = |e: JsValue| format!("{e:?}");
        let init = web_sys::RequestInit::new();
        init.set_method(match request.method {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Patch => "PATCH",
            Method::Delete => "DELETE",
        });
        if let Some(body) = &request.body {
            let headers = web_sys::Headers::new().map_err(js)?;
            headers.set("Content-Type", "application/json").map_err(js)?;
            init.set_headers(&headers);
            init.set_body(&JsValue::from_str(&body.to_string()));
        }
        let req = web_sys::Request::new_with_str_and_init(&request.path, &init).map_err(js)?;
        let window = web_sys::window().ok_or("no window")?;
        let response: web_sys::Response = JsFuture::from(window.fetch_with_request(&req))
            .await
            .map_err(js)?
            .dyn_into()
            .map_err(js)?;
        let body = JsFuture::from(response.text().map_err(js)?)
            .await
            .map_err(js)?
            .as_string()
            .unwrap_or_default();
        Ok(HttpResponse {
            status: response.status(),
            retry_after_ms: response
                .headers()
                .get("retry-after-ms")
                .ok()
                .flatten()
                .and_then(|v| v.parse().ok()),
            body,
        })
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Screen {
        Booting,
        Unreachable,
        Shell,
        SignIn,
        Forgot,
        Reset,
        Invite,
    }

    /// Link parameters the SPA consumes at start: the emailed `?reset=` / `?invite=` tokens, Stripe's
    /// `?checkout=` return, the Owner's `?sso=<provider id>` sign-in link and the SSO callback's
    /// `?error=sso_failed` / `?sso_linked=1` markers.
    struct UrlParams {
        reset: Option<String>,
        invite: Option<String>,
        checkout: Option<CheckoutReturn>,
        sso: Option<String>,
        sso_return: Option<SsoReturn>,
    }

    /// Read the link parameters and, if any is present, replace the URL with the bare path right
    /// away so a token cannot linger in the address bar, history, or a later Referer.
    fn take_url_params() -> UrlParams {
        let none = UrlParams { reset: None, invite: None, checkout: None, sso: None, sso_return: None };
        let Some(window) = web_sys::window() else { return none };
        let location = window.location();
        let search = location.search().unwrap_or_default();
        let params = UrlParams {
            reset: reset_token_from_query(&search),
            invite: invite_token_from_query(&search),
            checkout: checkout_return_from_query(&search),
            sso: sso_provider_from_query(&search),
            sso_return: sso_return_from_query(&search),
        };
        if params.reset.is_some()
            || params.invite.is_some()
            || params.checkout.is_some()
            || params.sso.is_some()
            || params.sso_return.is_some()
        {
            let path = location.pathname().unwrap_or_else(|_| "/".into());
            if let Ok(history) = window.history() {
                let _ = history.replace_state_with_url(&JsValue::NULL, "", Some(&path));
            }
        }
        params
    }

    /// Root application component.
    ///
    /// Provides the model and a dispatch callback via context, starts the boot session check, and
    /// renders the screen the model calls for. The `Memo` keeps a view mounted across unrelated
    /// model changes.
    #[component]
    pub fn App() -> impl IntoView {
        let model = RwSignal::new(AppModel::default());
        let dispatcher: Callback<Msg> = Callback::new(move |msg: Msg| dispatch(model, msg));

        provide_context(model);
        provide_context(dispatcher);
        let params = take_url_params();
        let reset_token = RwSignal::new(params.reset);
        let invite_token = RwSignal::new(params.invite);
        if let Some(outcome) = params.checkout {
            dispatch(model, Msg::CheckoutReturned(outcome));
        }
        if let Some(outcome) = params.sso_return {
            dispatch(model, Msg::SsoReturned(outcome));
        }
        // An invitation outranks a reset link if both are somehow present.
        if let Some(token) = invite_token.get_untracked() {
            dispatch(model, Msg::ShowAuth(AuthScreen::AcceptInvite));
            // Password or SSO invitation? The answer picks the form.
            dispatch(model, Msg::Request(ApiCall::LookupInvitation { token }));
        } else if reset_token.get_untracked().is_some() {
            dispatch(model, Msg::ShowAuth(AuthScreen::ResetPassword));
        }
        // The Owner's SSO link: start the round trip now. The provider id goes straight into the request
        // and is never stored; a failure shows on the sign-in screen once the boot check settles.
        if let Some(provider_id) = params.sso {
            dispatch(model, Msg::Request(ApiCall::StartSso { provider_id, intent: SsoIntent::SignIn }));
        }
        dispatch(model, Msg::Boot);

        let screen = Memo::new(move |_| {
            model.with(|m| {
                if m.booting {
                    Screen::Booting
                } else if m.server_unreachable {
                    Screen::Unreachable
                } else if m.authed {
                    Screen::Shell
                } else {
                    match m.auth_screen {
                        AuthScreen::SignIn => Screen::SignIn,
                        AuthScreen::ForgotPassword => Screen::Forgot,
                        AuthScreen::ResetPassword => Screen::Reset,
                        AuthScreen::AcceptInvite => Screen::Invite,
                    }
                }
            })
        });
        // Tokens are single-use: forget each as soon as its screen is left. Booting and Unreachable
        // are not "left" (the boot check runs before the screen shows), or the token would be
        // dropped before first render.
        leptos::prelude::Effect::new(move |_| {
            let settled = !matches!(screen.get(), Screen::Booting | Screen::Unreachable);
            if settled && screen.get() != Screen::Reset {
                reset_token.set(None);
            }
            if settled && screen.get() != Screen::Invite {
                invite_token.set(None);
            }
        });

        view! {
            <ToastStack />
            {move || match screen.get() {
                Screen::Booting => view! {
                    <div style="display:grid;place-items:center;height:100%;color:var(--text-3)">"Loading…"</div>
                }
                .into_any(),
                Screen::Unreachable => view! {
                    <div style="display:grid;place-items:center;height:100%">
                        <div class="col" style="gap:12px;align-items:center">
                            <span>"Cannot reach the server."</span>
                            <button class="btn btn-primary" on:click=move |_| dispatcher.run(Msg::Boot)>"Retry"</button>
                        </div>
                    </div>
                }
                .into_any(),
                Screen::Shell => view! { <ShellView /> }.into_any(),
                Screen::SignIn => view! { <AuthView /> }.into_any(),
                Screen::Forgot => view! { <ForgotPasswordView /> }.into_any(),
                Screen::Reset => view! { <ResetPasswordView token=reset_token /> }.into_any(),
                Screen::Invite => view! { <AcceptInviteView token=invite_token /> }.into_any(),
            }}
        }
    }
}

#[cfg(feature = "csr")]
pub use inner::App;
