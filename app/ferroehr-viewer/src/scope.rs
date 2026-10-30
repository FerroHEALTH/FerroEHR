// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The request a handler draws its page for, in reach of every function the
//! handler calls.
//!
//! A page handler runs inside [`run`], which holds the request's [`Scope`]
//! in a Tokio task-local: the viewer's state, the request's parts, the form
//! it posted, the locale and the answer the page builds up (its status, its
//! redirect, the cookies it sets and its docked bar). The page's code reads
//! them through [`crate::request`], [`crate::posted`], [`crate::locale`] and
//! [`crate::shell`], so a reader never threads the request through by hand.
//! A future the handler spawns onto another task leaves the scope; every
//! read of a page runs on the handler's own task. No openEHR spec governs
//! this: our own design.

use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use ferrohealth_ui::html::Html;
use ferrohealth_ui::i18n::Locale;

use crate::error::ViewerError;
use crate::posted::Posted;
use crate::state::AppState;

/// The answer a page builds up as it is drawn.
#[derive(Debug, Default)]
pub struct Answer {
    /// The status the page answers with, `200` when none is set.
    pub status: Option<http::StatusCode>,
    /// Where the page sends the browser instead of drawing itself.
    pub redirect: Option<String>,
    /// The `Set-Cookie` values the answer carries, in the order set.
    pub cookies: Vec<http::HeaderValue>,
    /// The docked decision bar the page set for the shell to draw.
    pub dock: Option<Html>,
    /// Whether a read met the end of the session, so the page answers with
    /// the sign-in page ([`crate::session_client`]).
    pub session_over: bool,
}

/// One request, as the code drawing its page reads it.
#[derive(Debug)]
pub struct Scope {
    /// The viewer's state.
    pub state: AppState,
    /// The request's method, address and headers.
    pub parts: http::request::Parts,
    /// The form the request posted, for a page answering its own post.
    pub posted: Option<Posted>,
    /// The locale the page is drawn in.
    pub locale: Locale,
    /// The instant the page is drawn at, read once.
    pub now: jiff::Timestamp,
    /// The CDR's status document, read at most once for the page.
    pub status: tokio::sync::OnceCell<Result<String, ViewerError>>,
    /// The answer the page builds up.
    answer: Mutex<Answer>,
}

impl Scope {
    /// The scope of the request `parts`, with the form `posted`.
    #[must_use]
    pub fn new(state: AppState, parts: http::request::Parts, posted: Option<Posted>) -> Self {
        let locale = crate::locale::negotiated(&state, &parts.headers);
        Self {
            state,
            parts,
            posted,
            locale,
            now: jiff::Timestamp::now(),
            status: tokio::sync::OnceCell::new(),
            answer: Mutex::new(Answer::default()),
        }
    }

    /// Changes the answer with `change`.
    pub fn answer<R>(&self, change: impl FnOnce(&mut Answer) -> R) -> R {
        let mut answer = self.answer.lock().unwrap_or_else(PoisonError::into_inner);
        change(&mut answer)
    }

    /// The answer the page built up, taken out of the scope.
    #[must_use]
    pub fn take_answer(&self) -> Answer {
        self.answer(std::mem::take)
    }
}

tokio::task_local! {
    /// The scope of the request the running task draws a page for.
    static SCOPE: Arc<Scope>;
}

/// Runs `page` with `scope` in reach of everything it calls.
pub async fn run<F: Future>(scope: Arc<Scope>, page: F) -> F::Output {
    SCOPE.scope(scope, page).await
}

/// The scope of the request being drawn, `None` outside one.
#[must_use]
pub fn current() -> Option<Arc<Scope>> {
    SCOPE.try_with(Arc::clone).ok()
}

/// What `read` makes of the scope of the request being drawn, `None` outside
/// one.
pub fn with<R>(read: impl FnOnce(&Scope) -> R) -> Option<R> {
    SCOPE.try_with(|scope| read(scope)).ok()
}

/// The viewer's state of the request being drawn.
///
/// # Errors
/// [`ViewerError::Internal`] outside a request, which is a defect of the
/// caller.
pub fn state() -> Result<AppState, ViewerError> {
    with(|scope| scope.state.clone()).ok_or_else(crate::session::outside_request)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{Scope, run};
    use crate::posted::Posted;

    /// The scope of a request for `path_and_query`, with `posted` and drawn
    /// at `now`.
    fn scope(path_and_query: &str, posted: Option<Posted>, now: &str) -> Arc<Scope> {
        let config = crate::config::ViewerConfig::default();
        let state = crate::state::AppState {
            cdr: crate::cdr::CdrClient::new(&config.cdr).expect("the default CDR client builds"),
            config: Arc::new(config),
            oidc: None,
            session_keys: crate::session::SessionKeys::from_secret("")
                .expect("an empty secret always yields an ephemeral key"),
        };
        let (parts, ()) = http::Request::builder()
            .uri(path_and_query)
            .body(())
            .expect("a request")
            .into_parts();
        let mut scope = Scope::new(state, parts, posted);
        scope.now = now.parse().expect("an instant");
        Arc::new(scope)
    }

    #[tokio::test]
    async fn a_page_reads_the_path_and_the_query_of_its_request() {
        let read = run(
            scope("/ehrs?q=a+b&empty=&q=c", None, "2026-10-21T09:42:11Z"),
            async {
                (
                    crate::request::path(),
                    crate::request::path_and_query(),
                    crate::request::query("q"),
                    crate::request::query("empty"),
                    crate::request::query_pairs().len(),
                )
            },
        )
        .await;
        assert_eq!(
            (
                "/ehrs".to_owned(),
                "/ehrs?q=a+b&empty=&q=c".to_owned(),
                Some("a b".to_owned()),
                None,
                3
            ),
            read
        );
        assert_eq!("/", crate::request::path(), "outside a request");
    }

    #[tokio::test]
    async fn the_page_counts_from_the_instant_its_request_was_read_at() {
        let read: jiff::Timestamp = "2026-10-23T23:30:00Z".parse().expect("an instant");
        let (first, second, today, last) = run(scope("/", None, "2026-10-23T23:30:00Z"), async {
            (
                crate::request::now(),
                crate::request::now(),
                crate::pages::common::today(),
                crate::pages::common::last_days(&[], 30).last,
            )
        })
        .await;
        assert_eq!(read, first);
        assert_eq!(
            read, second,
            "every read of the page answers the same instant"
        );
        assert_eq!(
            ferrohealth_ui::calendar::Day::new(2026, 10, 23),
            Some(today)
        );
        assert_eq!("2026-10-23", last);
    }

    #[tokio::test]
    async fn the_form_answers_the_page_it_was_posted_to() {
        let form = Posted::new(vec![("intent".to_owned(), "save".to_owned())]);
        let posted = run(
            scope("/ehrs/new", Some(form.clone()), "2026-10-21T09:42:11Z"),
            async { (crate::posted::posted(), crate::posted::posted_for("save")) },
        )
        .await;
        assert_eq!((Some(form.clone()), Some(form)), posted);
        assert_eq!(None, crate::posted::posted(), "outside a request");
    }

    #[tokio::test]
    async fn a_redirect_keeps_its_status_and_a_session_end_is_noted() {
        let scope = scope("/ehrs", None, "2026-10-21T09:42:11Z");
        run(Arc::clone(&scope), async {
            crate::request::redirect("/ehrs/abc");
            crate::request::set_status(http::StatusCode::NOT_FOUND);
            crate::request::set_cookie("a=b");
            crate::session_client::note_if_over(&crate::error::ViewerError::Forbidden(
                "no".to_owned(),
            ));
        })
        .await;
        let answer = scope.take_answer();
        assert_eq!(Some("/ehrs/abc".to_owned()), answer.redirect);
        assert_eq!(None, answer.status, "a redirect is never overwritten");
        assert_eq!(1, answer.cookies.len());
        assert!(!answer.session_over);
        run(Arc::clone(&scope), async {
            crate::session_client::note_if_over(&crate::error::ViewerError::Unauthenticated);
        })
        .await;
        assert!(scope.take_answer().session_over);
    }
}
