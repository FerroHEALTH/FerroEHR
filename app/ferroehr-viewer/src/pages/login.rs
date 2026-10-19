// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `/login` screen: dual Basic + OIDC authentication, drawn by FerroUI's
//! sign-in card (`ferrohealth_ui::view::sign_in`).
//!
//! The Basic path is a plain form `POST` to the [`crate::auth::LoginBasic`]
//! server function, so it submits and redirects before WASM loads and
//! hydration never wipes what was typed (the inputs are named exactly
//! `username`, `password`, `next`, matching the server-fn arguments). The OIDC
//! path is a plain `<a rel="external">` to the BFF's `/auth/oidc/login` axum
//! route. Which paths appear — and the deployment-configured notice and links
//! around them — is decided by [`crate::auth::fetch_login_screen`]. The
//! post-login destination is carried through the `next` query parameter on
//! BOTH paths. An arrival carrying `?expired=1` — the shell's signed-out
//! transition — opens the card with the notice that the previous session
//! ended; one carrying `?refused=1` — where [`crate::auth::login_basic`] sends
//! credentials the CDR refused — opens it with the refusal.

use ferrohealth_ui::tone::Tone;
use ferrohealth_ui::view::notice::{Announce, notice};
use ferrohealth_ui::view::shell::{Footer, centred};
use ferrohealth_ui::view::sign_in::{Basic, Notice, Provider, REFUSAL_ID, SignIn, sign_in};
use leptos::prelude::*;
use leptos::server_fn::ServerFn as _;

use crate::auth::{LoginBasic, fetch_login_screen};
use crate::components::brand::viewer_lockup;

/// The login screen.
///
/// Renders FerroUI's sign-in card, centred: the lockup over the Basic form
/// and/or the OIDC button, gated on the CDR's enabled auth modes (read once via
/// a `Resource` under `<Suspense>`). A `?expired=1` arrival additionally opens
/// the card with the notice that the previous session ended, and a
/// `?refused=1` arrival with the refusal, both read from the URL so the server
/// pass and hydration agree.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn LoginPage() -> impl IntoView {
    let screen = Resource::new(|| (), |()| fetch_login_screen());
    let (next, expired, refused) = login_query();

    let card = view! {
        // The fallback already says what the URL says (the refusal, the ended
        // session), so it shows before the sign-in methods resolve and with no
        // JavaScript at all; the card repeats it once they do.
        <Suspense fallback=move || {
            view! {
                <div class="fui-card fui-sign-in">
                    <div class="fui-sign-in-lockup">{viewer_lockup()}</div>
                    {expired
                        .then(|| {
                            notice(
                                Tone::Warn,
                                Announce::Status,
                                Some("session-expired"),
                                "Your session ended. Sign in again to continue.",
                            )
                        })}
                    {refused
                        .then(|| {
                            notice(
                                Tone::Bad,
                                Announce::Alert,
                                Some(REFUSAL_ID),
                                "Wrong username or password.",
                            )
                        })}
                    {ferrohealth_ui::view::empty::skeleton(3, "Loading the sign-in methods…")}
                </div>
            }
        }>
            {move || {
                let next = next.clone();
                Suspend::new(async move {
                    let (basic, oidc, configured, links, mode_error) = match screen.await {
                        Ok(s) => (s.basic, s.oidc, s.notice, s.links, None),
                        Err(e) => (true, true, String::new(), Vec::new(), Some(e.to_string())),
                    };
                    let mut notices = Vec::new();
                    if expired {
                        notices
                            .push(
                                Notice::new(
                                        Tone::Warn,
                                        "Your session ended. Sign in again to continue.",
                                    )
                                    .id("session-expired"),
                            );
                    }
                    if !configured.is_empty() {
                        notices.push(Notice::new(Tone::Accent, configured));
                    }
                    if let Some(error) = mode_error {
                        notices
                            .push(
                                Notice::new(
                                    Tone::Warn,
                                    format!("Could not read the CDR's login modes: {error}"),
                                ),
                            );
                    }
                    sign_in(SignIn {
                            lockup: viewer_lockup().into_any(),
                            notices,
                            refusal: refused.then(|| "Wrong username or password.".to_owned()),
                            basic: basic
                                .then(|| Basic {
                                    action: LoginBasic::url(),
                                    next: next.clone(),
                                }),
                            provider: oidc
                                .then(|| Provider {
                                    href: oidc_login_href(&next),
                                    label: "Sign in with OIDC".to_owned(),
                                }),
                            links: links.into_iter().map(|link| (link.label, link.href)).collect(),
                        })
                        .into_any()
                })
            }}
        </Suspense>
    }
    .into_any();

    view! {
        <leptos_meta::Title text="Sign in" />
        {centred(
            card,
            Footer {
                product: "FerroEHR Viewer",
                version: env!("CARGO_PKG_VERSION"),
                about: "/system#manufacturer",
            },
        )}
    }
}

/// The URL facts the card reads: the post-login destination (`?next=`, the
/// shell root by default), whether the previous session ended (`?expired=1`,
/// set by `crate::session_client::signed_out_url`) and whether the CDR refused
/// the last Basic sign-in (`?refused=1`). Deterministic from the URL, so
/// hydration-safe.
fn login_query() -> (String, bool, bool) {
    let query = leptos_router::hooks::use_query_map();
    query.with_untracked(|q| {
        (
            q.get("next").unwrap_or_else(|| "/".to_owned()),
            q.get("expired").is_some(),
            q.get("refused").is_some(),
        )
    })
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
