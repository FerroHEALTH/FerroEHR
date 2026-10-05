// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `/login` screen: dual Basic + OIDC authentication.
//!
//! The Basic path is an `<ActionForm>` bound to the [`crate::auth::LoginBasic`]
//! server action, so it submits and redirects even before WASM loads
//! (progressive enhancement — the inputs are named exactly `username`,
//! `password`, `next`, matching the server-fn arguments). The OIDC path is a
//! plain `<a>` to the BFF's `/auth/oidc/login` axum route, styled as the
//! secondary design-system button. Which paths appear — and the
//! deployment-configured notice and links around them — is decided by
//! [`crate::auth::fetch_login_screen`]. The post-login destination is carried
//! through the `next` query parameter on BOTH paths: the Basic form posts it
//! as a hidden field, the OIDC link appends it to its href. An arrival
//! carrying `?expired=1` — the shell's signed-out transition — opens the card
//! with the notice that the previous session ended.

use leptos::prelude::*;

use crate::auth::{LoginBasic, fetch_login_screen};
use crate::components::brand::Wordmark;
use crate::components::field::{BTN_PRIMARY, BTN_SECONDARY, INPUT, LABEL};
use crate::components::surface::CARD_PAD;

/// The login screen.
///
/// Renders a centered auth card — the wordmark over the Basic credential form
/// and/or the OIDC button, gated on the CDR's enabled auth modes (read once via
/// a `Resource` under `<Suspense>`). The action's error value renders in a
/// MessageBar; the submit button reflects the action's pending state. A
/// `?expired=1` arrival — the shell's signed-out transition — additionally
/// opens the card with the notice that the previous session ended.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[expect(
    clippy::too_many_lines,
    reason = "action/resource setup plus the Basic + OIDC view — one cohesive component"
)]
#[component]
pub fn LoginPage() -> impl IntoView {
    let action = ServerAction::<LoginBasic>::new();
    let screen = Resource::new(|| (), |()| fetch_login_screen());
    let pending = action.pending();
    let value = action.value();

    // The post-login destination, taken from `?next=…` (default the shell
    // root). Deterministic from the URL, so hydration-safe.
    let query = leptos_router::hooks::use_query_map();
    let next = Signal::derive(move || {
        query
            .with(|q| q.get("next"))
            .unwrap_or_else(|| "/".to_owned())
    });
    // `?expired=1` is set by the shell's signed-out transition
    // (`crate::session_client::signed_out_url`): the user did not come here to
    // sign in, their session ended under them, and the card says so instead of
    // looking like a plain sign-in they asked for.
    let expired = Signal::derive(move || query.with(|q| q.get("expired").is_some()));

    let expiry_notice = view! {
        {move || {
            expired
                .get()
                .then(|| {
                    view! {
                        <div
                            id="session-expired"
                            role="status"
                            class="rounded-control border border-warn/40 bg-warn-subtle px-3 py-2 text-sm text-ink"
                        >
                            "Your session ended. Sign in again to continue."
                        </div>
                    }
                })
        }}
    }
        .into_any();

    let error_bar = view! {
        {move || {
            value
                .get()
                .and_then(Result::err)
                .map(|err| {
                    view! {
                        <thaw::MessageBar intent=thaw::MessageBarIntent::Error>
                            <thaw::MessageBarBody>{err.to_string()}</thaw::MessageBarBody>
                        </thaw::MessageBar>
                    }
                })
        }}
    }
    .into_any();

    let forms = view! {
        <Suspense fallback=|| {
            view! {
                <div class="flex justify-center py-4">
                    <thaw::Spinner />
                </div>
            }
        }>
            {move || {
                Suspend::new(async move {
                    let (basic, oidc, notice, links, mode_error) = match screen.await {
                        Ok(s) => (s.basic, s.oidc, s.notice, s.links, None),
                        Err(e) => (true, true, String::new(), Vec::new(), Some(e.to_string())),
                    };
                    let notice_note = if notice.is_empty() {
                        ().into_any()
                    } else {
                        // whitespace-pre-line: a configured notice states its
                        // lines (credentials, expectations) deliberately.
                        view! {
                            <div class="rounded-control border border-accent/40 bg-accent-subtle px-3 py-2 text-xs text-accent-ink whitespace-pre-line">
                                {notice}
                            </div>
                        }
                            .into_any()
                    };
                    let link_row = if links.is_empty() {
                        ().into_any()
                    } else {
                        view! {
                            <div class="flex flex-wrap justify-center gap-x-4 gap-y-1 text-xs">
                                {links
                                    .into_iter()
                                    .map(|link| {
                                        // rel=external: these point at non-app
                                        // URLs (an API reference, docs); the
                                        // client router must not intercept a
                                        // same-origin one.
                                        view! {
                                            <a
                                                href=link.href
                                                rel="external noopener"
                                                class="text-accent underline underline-offset-2 hover:text-accent-hover"
                                            >
                                                {link.label}
                                            </a>
                                        }
                                    })
                                    .collect::<Vec<_>>()}
                            </div>
                        }
                            .into_any()
                    };
                    let mode_warning = match mode_error {
                        Some(e) => {
                            // The Err arm renders both paths plus a warning — an
                            // Err fallback that silently DIFFERS from what the
                            // server may have rendered is a hydration hazard, so
                            // the failure is explicit and visible.
                            view! {
                                <div
                                    role="alert"
                                    class="rounded-control border border-warn/40 bg-warn-subtle px-3 py-2 text-xs text-ink"
                                >
                                    {format!("Could not read the CDR's login modes: {e}")}
                                </div>
                            }
                                .into_any()
                        }
                        None => ().into_any(),
                    };
                    let basic_form = if basic {
                        basic_login_form(action, pending, next).into_any()
                    } else {
                        ().into_any()
                    };
                    let separator = if basic && oidc {
                        view! {
                            <div class="flex items-center gap-3 text-xs text-ink-faint">
                                <span class="h-px flex-1 bg-edge"></span>
                                "or"
                                <span class="h-px flex-1 bg-edge"></span>
                            </div>
                        }
                            .into_any()
                    } else {
                        ().into_any()
                    };
                    let oidc_button = if oidc {
                        // A plain anchor to the BFF's axum route, styled as the
                        // secondary design-system button (an <a> — nesting a
                        // <button> inside would be invalid HTML).
                        view! {
                            <a
                                // The destination rides the query string, so
                                // the OIDC round trip returns the user to the
                                // screen an expiry took them off — the same
                                // `next` the Basic form posts.
                                href=move || oidc_login_href(&next.get())
                                // rel=external: the client router must NOT
                                // intercept this same-origin anchor — it is a
                                // BFF axum route, not a client route.
                                rel="external"
                                class=format!("{BTN_SECONDARY} w-full justify-center")
                            >
                                "Sign in with OIDC"
                            </a>
                        }
                            .into_any()
                    } else {
                        ().into_any()
                    };
                    let none_available = if basic || oidc {
                        ().into_any()
                    } else {
                        // The intersection of the viewer's configured modes and
                        // the CDR's advertised schemes can be empty — say so
                        // instead of rendering a blank card.
                        view! {
                            <div
                                role="alert"
                                class="rounded-control border border-warn/40 bg-warn-subtle px-3 py-2 text-sm text-ink"
                            >
                                "No sign-in method is available: the viewer's configured "
                                "login modes and the CDR's advertised authentication "
                                "schemes do not overlap. Align the viewer auth "
                                "configuration with the CDR's."
                            </div>
                        }
                            .into_any()
                    };
                    view! {
                        <div class="flex flex-col gap-4">
                            {notice_note} {mode_warning} {basic_form} {separator} {oidc_button}
                            {none_available} {link_row}
                        </div>
                    }
                        .into_any()
                })
            }}
        </Suspense>
    }
    .into_any();

    view! {
        <leptos_meta::Title text="Sign in" />
        <main class="min-h-screen bg-surface flex items-center justify-center p-4">
            <div class=format!("w-full max-w-sm {CARD_PAD}")>
                <div class="flex flex-col gap-4">
                    <div class="flex justify-center">
                        <Wordmark />
                    </div>
                    {expiry_notice}
                    {error_bar}
                    {forms}
                </div>
            </div>
        </main>
    }
}

/// The BFF's OIDC sign-in URL, carrying the post-login destination.
///
/// The shell root is the route's own fallback, so it is left off rather than
/// spelled out. Everything else is percent-encoded with the `urlencoding`
/// crate (the owner's never-hand-roll-a-codec rule); what the route then
/// ACCEPTS is decided server-side by `crate::oidc::same_origin_next`, since a
/// link is not a place a security property can live.
fn oidc_login_href(next: &str) -> String {
    if next == "/" {
        "/auth/oidc/login".to_owned()
    } else {
        format!("/auth/oidc/login?next={}", urlencoding::encode(next))
    }
}

/// The Basic credential form: username + password inputs, a hidden `next`
/// field, and a pending-aware submit button, all inside an `<ActionForm>`
/// bound to the login action. Named inputs (`username`/`password`/`next`)
/// mirror the [`crate::auth::login_basic`] arguments so the form submits
/// correctly without JavaScript.
fn basic_login_form(
    action: ServerAction<LoginBasic>,
    pending: Memo<bool>,
    next: Signal<String>,
) -> impl IntoView {
    view! {
        <ActionForm action=action>
            <div class="flex flex-col gap-3">
                // Plain labels + explicit stable input ids: thaw::Field
                // hardwires its <label for> to a per-render random UUID,
                // which breaks SSR↔hydration determinism; an explicit id
                // keeps the association deterministic.
                <label class=LABEL r#for="login-username">
                    "Username"
                </label>
                // Plain UNCONTROLLED inputs: a controlled (signal-driven)
                // input resets to its empty signal at hydration, wiping
                // anything the user typed before WASM attached — on the
                // login form that silently swallows credentials (found by
                // the E2E battery as intermittent empty-credential posts).
                <input
                    id="login-username"
                    name="username"
                    placeholder="Username"
                    autocomplete="username"
                    class=INPUT
                />
                <label class=LABEL r#for="login-password">
                    "Password"
                </label>
                <input
                    id="login-password"
                    name="password"
                    type="password"
                    placeholder="Password"
                    autocomplete="current-password"
                    class=INPUT
                />
                <input type="hidden" name="next" value=move || next.get() />
                <button
                    type="submit"
                    class=format!("{BTN_PRIMARY} w-full justify-center")
                    prop:disabled=move || pending.get()
                >
                    {move || if pending.get() { "Signing in…" } else { "Sign in" }}
                </button>
            </div>
        </ActionForm>
    }
}

#[cfg(test)]
mod tests {
    use super::oidc_login_href;

    #[test]
    fn the_oidc_link_carries_an_encoded_destination_and_omits_the_default_one() {
        assert_eq!(oidc_login_href("/"), "/auth/oidc/login");
        assert_eq!(
            oidc_login_href("/ehrs/01a06375?tab=status"),
            "/auth/oidc/login?next=%2Fehrs%2F01a06375%3Ftab%3Dstatus"
        );
    }
}
