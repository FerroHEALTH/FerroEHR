// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The sign-in page (R19): Basic and OIDC, the deployment's notice and links,
//! and every state the page can be in.
//!
//! The Basic form is a plain `POST` to this page, with plain uncontrolled
//! `username` and `current-password` inputs, so it works with no script
//! (FerroUI `console-ui.md` §5): accepted credentials send the reader on with
//! a redirect, and refused ones draw the page again with the refusal under
//! the password and the username kept. The OIDC action is a plain link to
//! the viewer's own `/auth/oidc/login` route. The page shows the session
//! ended (`?expired=1`), signed out (`?signed-out=1`) or refused
//! (`?refused=1`, the OIDC callback's) from its address. Which methods it
//! offers comes from [`crate::auth::fetch_login_screen`].
//!
//! The page's handler signs in, reads the screen and redirects; the view
//! draws the model it leaves. The model carries the username as typed and
//! never the password or a session. No openEHR spec governs the viewer: our
//! own design.

use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::view::shell::{brand_mark, lockup_plain};
use ferrohealth_ui::view::sign_in::{Basic, Notice, Provider, SignIn, State, sign_in};

use crate::auth::LoginScreen;
use crate::auth::{SignedIn, fetch_login_screen, sign_in_basic};
use crate::error::ViewerError;
use crate::locale::signin::SignInMsg;
use crate::locale::{Msg, current, tr, tr_with};
use crate::markup::{h, t};
use crate::routes::Drawn;

/// The BFF's OIDC sign-in URL, carrying the post-login destination.
///
/// The shell root is the route's own fallback, so it is left off. What the
/// route then accepts is decided on the server by
/// `crate::auth::same_origin_next`.
fn oidc_login_href(next: &str) -> String {
    if next == "/" {
        "/auth/oidc/login".to_owned()
    } else {
        format!("/auth/oidc/login?next={}", urlencoding::encode(next))
    }
}

/// What the address says about the sign-in: where to go next and the state
/// the page opens in.
fn arrival() -> (String, State) {
    let next = crate::request::query("next").unwrap_or_else(|| "/".to_owned());
    let state = if crate::request::query("expired").is_some() {
        State::Expired(Notice::new(tr(SignInMsg::Expired)))
    } else if crate::request::query("signed-out").is_some() {
        State::SignedOut(Notice::new(tr(SignInMsg::SignedOut)))
    } else if crate::request::query("refused").is_some() {
        State::Refused(Notice::new(tr(SignInMsg::RefusedProvider)))
    } else {
        State::Ready
    };
    (next, state)
}

/// The sign-in card for the methods of `screen`, in `state`, the basic form
/// holding `username` and saying `error` under the password.
fn card(
    screen: Result<LoginScreen, ViewerError>,
    state: State,
    next: &str,
    username: String,
    error: Option<String>,
) -> Html {
    let (basic, oidc, notice_text, links, state) = match screen {
        Ok(screen) => (
            screen.basic,
            screen.oidc,
            screen.notice,
            screen.links,
            state,
        ),
        Err(error) => {
            let mut unreachable = Notice::new(tr(SignInMsg::CdrUnreachable));
            unreachable.detail = Some(error.to_string());
            (
                true,
                true,
                String::new(),
                Vec::new(),
                match state {
                    State::Ready => State::UpstreamUnreachable(unreachable),
                    other => other,
                },
            )
        }
    };
    let issuer = crate::request::settings().oidc_issuer;
    let host = issuer
        .parse::<http::Uri>()
        .ok()
        .and_then(|uri| uri.host().map(str::to_owned))
        .unwrap_or_else(|| tr(SignInMsg::IdentityProvider));
    let line = (!notice_text.is_empty()).then(|| {
        h(format!(
            r#"<p class="fui-small fui-muted fui-sign-in-line">{}</p>"#,
            t(&notice_text)
        ))
    });
    sign_in(&current(), SignIn {
        lockup: lockup_plain(
            brand_mark(crate::shell::MARK_PATH, crate::shell::MARK_PATH),
            tr(Msg::BrandName),
            tr(Msg::BrandWord),
        ),
        title: Some(tr(SignInMsg::Title)),
        state,
        provider: oidc.then(|| Provider {
            href: oidc_login_href(next),
            label: tr_with(SignInMsg::At, &[("host", host.clone().into())]),
            busy: tr_with(SignInMsg::Redirecting, &[("host", host.into())]),
            glyph: Glyph::LOG_IN,
        }),
        basic: basic.then(|| Basic {
            action: "/login".to_owned(),
            next: next.to_owned(),
            divider: None,
            submit: Some(tr(SignInMsg::BasicSubmit)),
            username,
            error,
        }),
        line,
        note: None,
        links: links
            .into_iter()
            .map(|link| (link.label, link.href))
            .collect(),
        caption: Some(tr(SignInMsg::Caption)),
    })
}

/// The sign-in page's model.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LoginModel {
    /// The card: the methods on offer, the username as typed and the
    /// sentence under the password.
    Card {
        /// What the screen offers, or why it could not be read.
        screen: Result<LoginScreen, ViewerError>,
        /// The username the refused post carried.
        username: String,
        /// Why the post did not sign the reader in.
        error: Option<String>,
    },
    /// The CDR accepted the credentials and the post redirected.
    Left,
}

/// The sign-in page's model: the attempt a post makes, and the screen.
async fn read_login() -> LoginModel {
    let attempt = match crate::posted::posted() {
        Some(posted) => {
            let username = posted.text("username");
            let outcome = sign_in_basic(
                username.clone(),
                posted.get("password").unwrap_or_default(),
                posted.get("next"),
            )
            .await;
            Some((username, outcome))
        }
        None => None,
    };
    let (username, error) = match attempt {
        Some((_, Ok(SignedIn::Accepted(to)))) => {
            crate::request::redirect(&to);
            return LoginModel::Left;
        }
        Some((username, Ok(SignedIn::Refused))) => (username, Some(tr(SignInMsg::Refused))),
        Some((username, Err(error))) => (
            username,
            Some(tr_with(
                SignInMsg::Failed,
                &[("error", error.to_string().into())],
            )),
        ),
        None => (String::new(), None),
    };
    LoginModel::Card {
        screen: fetch_login_screen().await,
        username,
        error,
    }
}

/// The sign-in page, drawn centred with no shell.
pub async fn login_page() -> Drawn {
    let (next, state) = arrival();
    let body = match read_login().await {
        LoginModel::Card {
            screen,
            username,
            error,
        } => card(screen, state, &next, username, error),
        LoginModel::Left => Html::empty(),
    };
    Drawn::bare(tr(SignInMsg::Title), body)
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
