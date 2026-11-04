// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The roots a CDR request may address, and the check every request passes
//! before it is sent.
//!
//! The path of a CDR URL carries values the browser chose (an EHR id, a
//! stored-query name, a template id), each percent-encoded with
//! `urlencoding`, which leaves `.` unreserved. A value of `..` would survive
//! that encoding, and the URL parser would remove the dot segment and the
//! segment before it (RFC 3986 §5.2.4); the WHATWG URL Standard the parser
//! follows also reads `%2e` as `.`, a `\` as `/` and drops tabs and newlines.
//! So each URL is held to the configured roots before any request leaves.
//!
//! NOTE: no openEHR spec governs the viewer's outbound URLs, our own design.

use crate::error::ViewerError;
use crate::locale::server::ServerMsg;
use crate::locale::tr_served;

/// The roots one [`super::CdrClient`] may address, each a prefix that ends
/// in `/`.
#[derive(Debug, Clone)]
pub(super) struct Roots(Vec<Root>);

/// One root: the prefix the client's URL builders write, and its parsed form.
#[derive(Debug, Clone)]
struct Root {
    prefix: String,
    parsed: url::Url,
}

/// A configured CDR URL does not parse as an absolute URL.
#[derive(Debug, thiserror::Error)]
#[error("the configured CDR URL `{name}` does not parse as an absolute URL")]
pub struct RootError {
    name: &'static str,
    #[source]
    source: url::ParseError,
}

impl Roots {
    /// The roots `prefixes` names, each by the configuration key it comes from.
    ///
    /// # Errors
    /// [`RootError`] when a prefix does not parse as an absolute URL.
    pub(super) fn new(prefixes: &[(&'static str, String)]) -> Result<Self, RootError> {
        prefixes
            .iter()
            .map(|(name, prefix)| {
                let prefix = format!("{}/", prefix.trim_end_matches('/'));
                url::Url::parse(&prefix)
                    .map(|parsed| Root { prefix, parsed })
                    .map_err(|source| RootError { name, source })
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Self)
    }

    /// `url` parsed, when it stays under one of the roots.
    ///
    /// The URL must start with a root as written, hold no control character,
    /// no `\` and no `.` or `..` segment in its path in any spelling, and
    /// still lie under that root once parsed.
    ///
    /// # Errors
    /// [`ViewerError::Invalid`] when `url` leaves every root.
    pub(super) fn admit(&self, url: &str) -> Result<url::Url, ViewerError> {
        let refused = || ViewerError::Invalid(tr_served(ServerMsg::CdrUrlOutsideBase));
        let (root, rest) = self
            .0
            .iter()
            .find_map(|root| {
                url.strip_prefix(root.prefix.as_str())
                    .map(|rest| (root, rest))
            })
            .ok_or_else(refused)?;
        let path = rest.split(['?', '#']).next().unwrap_or(rest);
        if url.chars().any(char::is_control)
            || path.contains('\\')
            || path.split('/').any(is_dot_segment)
        {
            return Err(refused());
        }
        let parsed = url::Url::parse(url).map_err(|_unparsed| refused())?;
        let inside = parsed.origin() == root.parsed.origin()
            && parsed.username() == root.parsed.username()
            && parsed.password() == root.parsed.password()
            && parsed.path().starts_with(root.parsed.path());
        if inside { Ok(parsed) } else { Err(refused()) }
    }
}

/// Whether `segment` is `.` or `..` once percent-decoded, the two spellings
/// the URL parser resolves as a step (RFC 3986 §5.2.4).
fn is_dot_segment(segment: &str) -> bool {
    let decoded = urlencoding::decode_binary(segment.as_bytes());
    matches!(decoded.as_ref(), b"." | b"..")
}

#[cfg(test)]
mod tests {
    use super::Roots;
    use crate::error::ViewerError;

    fn roots() -> Roots {
        Roots::new(&[
            ("cdr.base_path", "http://cdr:8080/ferroehr/rest".to_owned()),
            (
                "cdr.management_base_url",
                "http://cdr:9464/management".to_owned(),
            ),
            ("cdr.base_url", "http://cdr:8080/health".to_owned()),
        ])
        .expect("the roots parse")
    }

    #[test]
    fn a_url_under_a_root_is_admitted_as_written() {
        let roots = roots();
        for url in [
            "http://cdr:8080/ferroehr/rest/openehr/v1/ehr/7d44b88c-4199-4bad-97dc-d78268e01398",
            "http://cdr:8080/ferroehr/rest/openehr/v1/query/org.example%3A%3Avitals/1.0.0",
            "http://cdr:8080/ferroehr/rest/openehr/v1/definition/template/adl1.4/vital%20signs",
            "http://cdr:8080/ferroehr/rest/openehr/v1/definition/template/adl2?version=*",
            "http://cdr:8080/ferroehr/rest/fhir/r4/AuditEvent?date=ge2026-11-01",
            "http://cdr:8080/ferroehr/rest/status",
            "http://cdr:9464/management/metrics/aql_queries_total",
            "http://cdr:8080/health/readiness",
            // A dot inside a value is no dot segment.
            "http://cdr:8080/ferroehr/rest/openehr/v1/ehr/a..b/composition/.x",
            // A doubly encoded dot reaches the CDR as the text `%2e`.
            "http://cdr:8080/ferroehr/rest/openehr/v1/ehr/%252e%252e",
        ] {
            let admitted = roots.admit(url).expect(url);
            assert_eq!(admitted.as_str(), url);
        }
    }

    #[test]
    fn a_dot_segment_in_any_spelling_is_refused() {
        let roots = roots();
        for url in [
            "http://cdr:8080/ferroehr/rest/openehr/v1/query/../..",
            "http://cdr:8080/ferroehr/rest/openehr/v1/ehr/..",
            "http://cdr:8080/ferroehr/rest/openehr/v1/ehr/./x",
            "http://cdr:8080/ferroehr/rest/openehr/v1/query/%2e%2e/%2E%2E",
            "http://cdr:8080/ferroehr/rest/openehr/v1/query/.%2e/x",
            "http://cdr:8080/ferroehr/rest/openehr/v1/ehr/%2e",
            "http://cdr:9464/management/../env",
            "http://cdr:8080/health/../ferroehr/rest/admin",
        ] {
            assert!(
                matches!(roots.admit(url), Err(ViewerError::Invalid(_))),
                "{url}"
            );
        }
    }

    #[test]
    fn a_separator_the_parser_would_rewrite_is_refused() {
        let roots = roots();
        for url in [
            // WHATWG URL: a `\` is a `/` in a special scheme's path.
            "http://cdr:8080/ferroehr/rest/openehr/v1/ehr/..\\..\\x",
            // Tabs and newlines are dropped, so `.\t.` would read as `..`.
            "http://cdr:8080/ferroehr/rest/openehr/v1/ehr/.\t./x",
            "http://cdr:8080/ferroehr/rest/openehr/v1/ehr/.\n./x",
        ] {
            assert!(
                matches!(roots.admit(url), Err(ViewerError::Invalid(_))),
                "{url:?}"
            );
        }
    }

    #[test]
    fn another_scheme_host_port_or_user_is_refused() {
        let roots = roots();
        for url in [
            "https://cdr:8080/ferroehr/rest/openehr/v1/ehr",
            "http://evil.example/ferroehr/rest/openehr/v1/ehr",
            "http://cdr:8081/ferroehr/rest/openehr/v1/ehr",
            "http://cdr:8080@evil.example/ferroehr/rest/openehr/v1/ehr",
            "http://user@cdr:8080/ferroehr/rest/openehr/v1/ehr",
            "//evil.example/ferroehr/rest/openehr/v1/ehr",
            "http://cdr:8080/ferroehr/other",
            "http://cdr:8080/ferroehr/restricted/openehr/v1",
            "http://cdr:8080/management/env",
            "",
        ] {
            assert!(
                matches!(roots.admit(url), Err(ViewerError::Invalid(_))),
                "{url}"
            );
        }
    }

    #[test]
    fn a_root_that_does_not_parse_refuses_the_client() {
        let error =
            Roots::new(&[("cdr.base_url", "cdr.example.org/ferroehr".to_owned())]).unwrap_err();
        assert_eq!(
            error.to_string(),
            "the configured CDR URL `cdr.base_url` does not parse as an absolute URL"
        );
    }
}
