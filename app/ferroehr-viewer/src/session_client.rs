// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The transport backstop that turns "this session is over" into one signal,
//! and the `/login` URL the signed-out transition lands on.
//!
//! Every screen reaches the CDR through a `#[server]` fn, and every one of them
//! refuses a dead session the same two ways: [`ViewerError::Unauthenticated`]
//! (the viewer's own sealed cookie is gone or idle-expired) and
//! [`ViewerError::CdrUnauthorized`] (the CDR answered `401` to the credential
//! the session carries). `server_fn` renders an application error as an HTTP
//! `500` whose body is the encoded error — its `Res::error_response` sets
//! `INTERNAL_SERVER_ERROR` for every error type
//! (<https://docs.rs/server_fn/0.8.13/server_fn/response/trait.Res.html>) — so
//! the answer is read from the BODY, with the status kept as a first check for
//! a `401` produced by anything in front of the viewer.
//!
//! [`SessionAwareClient`] is the `client =` argument of every `#[server]` fn in
//! the crate: it delegates to the browser `fetch` client and watches what comes
//! back. A detection bumps [`session_ended_signal`], which the shell's browser
//! Effect turns into the signed-out transition — so a revoked or expired
//! session cannot leave an interactive shell over a dead credential, whatever
//! the user was doing at the time.
//!
//! NOTE: no openEHR spec governs the viewer session — our own design.

use std::sync::LazyLock;

use futures::{Sink, Stream};
use leptos::prelude::{GetUntracked, Update};
use leptos::reactive::signal::ArcRwSignal;
use leptos::server_fn::Bytes;
use leptos::server_fn::client::Client;
use leptos::server_fn::client::browser::BrowserClient;
use leptos::server_fn::error::FromServerFnError;
use leptos::server_fn::request::browser::BrowserRequest;
use leptos::server_fn::response::ClientRes;
use leptos::server_fn::response::browser::BrowserResponse;

use crate::error::ViewerError;

/// How many times a server function has answered "this session is over".
///
/// A monotone counter rather than a flag: a consumer compares it against the
/// value it read when it mounted, so a fresh shell after a new sign-in starts
/// even with the counter and no reset write is ever needed.
static SESSION_ENDED: LazyLock<ArcRwSignal<u32>> = LazyLock::new(|| ArcRwSignal::new(0));

/// Returns the session-ended counter as a signal to subscribe to.
#[must_use]
pub fn session_ended_signal() -> ArcRwSignal<u32> {
    SESSION_ENDED.clone()
}

/// Returns the session-ended counter's current value without subscribing.
#[must_use]
pub fn session_ended_epoch() -> u32 {
    SESSION_ENDED.get_untracked()
}

/// The `/login` URL the signed-out transition lands on.
///
/// `expired=1` is what makes the login screen say the session ended instead of
/// presenting a bare sign-in card, and `next` carries the screen the user was
/// on so signing in again returns them to it. The destination is percent-encoded
/// with the `urlencoding` crate (the owner's never-hand-roll-a-codec rule);
/// `search` is the raw query string without its `?`, as `leptos_router`'s
/// `Location` reports it.
#[must_use]
pub fn signed_out_url(path: &str, search: &str) -> String {
    let next = if search.is_empty() {
        path.to_owned()
    } else {
        format!("{path}?{search}")
    };
    format!("/login?expired=1&next={}", urlencoding::encode(&next))
}

/// Records that a server function answered "this session is over".
fn note_session_ended() {
    SESSION_ENDED.update(|epoch| *epoch = epoch.wrapping_add(1));
}

/// Inspects one server-fn answer and records a session end when it is one.
///
/// A `401` is authoritative on its own; otherwise only an error status carries
/// an encoded [`ViewerError`], and the two refusals that mean the session is
/// over are the ones the shell must act on. Every other error — a CDR outage,
/// a `403`, a rejected input — leaves the counter alone, so a screen that
/// simply failed never signs the user out.
fn note_if_session_over(status: u16, body: &[u8]) {
    let Ok(code) = http::StatusCode::from_u16(status) else {
        return;
    };
    if code == http::StatusCode::UNAUTHORIZED {
        note_session_ended();
        return;
    }
    if !code.is_client_error() && !code.is_server_error() {
        return;
    }
    if matches!(
        ViewerError::de(Bytes::copy_from_slice(body)),
        ViewerError::Unauthenticated | ViewerError::CdrUnauthorized(_)
    ) {
        note_session_ended();
    }
}

/// The browser `fetch` response, watched for the end of the session.
///
/// Wraps the default browser response and reads the body on its way through,
/// which is the only place a `server_fn` application error is visible: the
/// status is `500` for every one of them (module docs).
pub struct SessionAwareResponse(BrowserResponse);

impl std::fmt::Debug for SessionAwareResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionAwareResponse")
            .finish_non_exhaustive()
    }
}

impl ClientRes<ViewerError> for SessionAwareResponse {
    async fn try_into_string(self) -> Result<String, ViewerError> {
        let status = ClientRes::<ViewerError>::status(&self.0);
        let text = ClientRes::<ViewerError>::try_into_string(self.0).await?;
        note_if_session_over(status, text.as_bytes());
        Ok(text)
    }

    async fn try_into_bytes(self) -> Result<Bytes, ViewerError> {
        let status = ClientRes::<ViewerError>::status(&self.0);
        let bytes = ClientRes::<ViewerError>::try_into_bytes(self.0).await?;
        note_if_session_over(status, &bytes);
        Ok(bytes)
    }

    fn try_into_stream(
        self,
    ) -> Result<impl Stream<Item = Result<Bytes, Bytes>> + Send + Sync + 'static, ViewerError> {
        ClientRes::<ViewerError>::try_into_stream(self.0)
    }

    fn status(&self) -> u16 {
        ClientRes::<ViewerError>::status(&self.0)
    }

    fn status_text(&self) -> String {
        ClientRes::<ViewerError>::status_text(&self.0)
    }

    fn location(&self) -> String {
        ClientRes::<ViewerError>::location(&self.0)
    }

    fn has_redirect(&self) -> bool {
        ClientRes::<ViewerError>::has_redirect(&self.0)
    }
}

/// The `client =` every `#[server]` fn in this crate is declared with.
///
/// It sends exactly what the default browser client sends — the request type is
/// unchanged and nothing is retried, rewritten or swallowed — and only watches
/// the answer, so a call that succeeds costs one status read.
#[derive(Debug, Clone, Copy)]
pub struct SessionAwareClient;

impl Client<ViewerError, ViewerError, ViewerError> for SessionAwareClient {
    type Request = BrowserRequest;
    type Response = SessionAwareResponse;

    async fn send(req: Self::Request) -> Result<Self::Response, ViewerError> {
        <BrowserClient as Client<ViewerError>>::send(req)
            .await
            .map(SessionAwareResponse)
    }

    fn open_websocket(
        path: &str,
    ) -> impl Future<
        Output = Result<
            (
                impl Stream<Item = Result<Bytes, Bytes>> + Send + 'static,
                impl Sink<Bytes> + Send + 'static,
            ),
            ViewerError,
        >,
    > + Send {
        <BrowserClient as Client<ViewerError>>::open_websocket(path)
    }

    fn spawn(future: impl Future<Output = ()> + Send + 'static) {
        <BrowserClient as Client<ViewerError>>::spawn(future);
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "test assertions/fixtures"
    )]

    use super::{note_if_session_over, session_ended_epoch, signed_out_url};

    use leptos::server_fn::error::FromServerFnError;

    use crate::error::ViewerError;

    /// Every `.rs` file under this crate's `src/`, read whole.
    fn crate_sources(dir: &std::path::Path, into: &mut Vec<(std::path::PathBuf, String)>) {
        for entry in std::fs::read_dir(dir).expect("the crate's own source tree should be readable")
        {
            let path = entry
                .expect("a readable directory yields readable entries")
                .path();
            if path.is_dir() {
                crate_sources(&path, into);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let text = std::fs::read_to_string(&path).expect("a Rust source file is UTF-8");
                into.push((path, text));
            }
        }
    }

    /// The backstop only covers what is declared with it, and a new server
    /// function is exactly where that is easy to forget — so the declaration is
    /// checked rather than remembered.
    #[test]
    fn every_server_function_in_this_crate_is_declared_with_the_session_aware_client() {
        let mut sources = Vec::new();
        crate_sources(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut sources,
        );
        assert!(!sources.is_empty(), "no sources found to check");
        let mut missing = Vec::new();
        for (path, text) in &sources {
            for (number, line) in text.lines().enumerate() {
                let line = line.trim_start();
                if line.starts_with("#[server") && !line.contains("SessionAwareClient") {
                    missing.push(format!("{}:{}: {line}", path.display(), number + 1));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "these server functions bypass the session backstop:\n{}",
            missing.join("\n")
        );
    }

    #[test]
    fn the_signed_out_url_carries_the_expiry_flag_and_the_encoded_destination() {
        assert_eq!(
            signed_out_url("/ehrs/abc", "tab=status&page=2"),
            "/login?expired=1&next=%2Fehrs%2Fabc%3Ftab%3Dstatus%26page%3D2"
        );
        assert_eq!(signed_out_url("/", ""), "/login?expired=1&next=%2F");
    }

    /// One test rather than three: the counter is process-global, so splitting
    /// the cases would let two of them observe each other's writes.
    #[test]
    fn the_counter_moves_only_on_the_two_session_refusals() {
        let before = session_ended_epoch();
        note_if_session_over(200, b"{}");
        note_if_session_over(500, &ViewerError::CdrUnreachable("down".to_owned()).ser());
        note_if_session_over(500, &ViewerError::Forbidden("nope".to_owned()).ser());
        note_if_session_over(500, b"not an encoded error");
        assert_eq!(session_ended_epoch(), before);

        note_if_session_over(500, &ViewerError::Unauthenticated.ser());
        assert_eq!(session_ended_epoch(), before + 1);
        note_if_session_over(500, &ViewerError::CdrUnauthorized("gone".to_owned()).ser());
        assert_eq!(session_ended_epoch(), before + 2);

        // A `401` is authoritative whatever the body: a proxy in front of the
        // viewer answers HTML, not an encoded ViewerError.
        note_if_session_over(401, b"<html>proxy</html>");
        assert_eq!(session_ended_epoch(), before + 3);
    }
}
