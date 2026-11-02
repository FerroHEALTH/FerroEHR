// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `Prefer` request header (RFC 7240) and the `Preference-Applied`
//! answer of a write (ITS-REST overview `Requests_and_responses.md`
//! §Representation details negotiation).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 9): the wire boundary — one byte-to-JSON \
              step per route, consumed by the typed decode"
)]

use axum::response::Response;
use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};

use super::body::{body, empty, respond};
use super::{APPLICATION_JSON, APPLICATION_XML, CANONICAL, WireFormat, resolve_accept};

/// The `Preference-Applied` response header (RFC 7240 §3).
pub(crate) const PREFERENCE_APPLIED: HeaderName = HeaderName::from_static("preference-applied");

/// A preference a client may state on `Prefer` (ITS-REST overview
/// `Requests_and_responses.md` §Representation details negotiation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Preference {
    /// `return=representation`: the full resource representation.
    Representation,
    /// `return=identifier`: only the affected resource's identifier.
    Identifier,
    /// `resolve_refs`: the `OBJECT_REF`s of a CONTRIBUTION resolved.
    ResolveRefs,
}

impl Preference {
    /// The preference token as RFC 7240 §2 writes it.
    fn token(self) -> &'static str {
        match self {
            Preference::Representation => "return=representation",
            Preference::Identifier => "return=identifier",
            Preference::ResolveRefs => "resolve_refs",
        }
    }
}

/// Whether `headers` state `preference` (RFC 7240 §2): one `Prefer` field or
/// several, each a comma-separated list whose items may carry parameters
/// after `;`, compared without case and without the optional whitespace
/// around `=`.
pub(crate) fn prefers(headers: &HeaderMap, preference: Preference) -> bool {
    headers
        .get_all(HeaderName::from_static("prefer"))
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|item| item.split(';').next())
        .any(|item| {
            item.split('=')
                .map(str::trim)
                .collect::<Vec<_>>()
                .join("=")
                .eq_ignore_ascii_case(preference.token())
        })
}

/// The return preference a write response ACTUALLY applied — the value the
/// `Preference-Applied` response header declares.
///
/// ITS-REST overview `Requests_and_responses.md` §Representation details
/// negotiation: "The service MAY include a `Preference-Applied` header in the
/// response, such as `Preference-Applied: return=minimal` or
/// `Preference-Applied: return=representation`, to indicate that the client's
/// preference has been honored" — so the header states what the response DID,
/// never what the client merely asked for (RFC 7240 §3, the field
/// definition this section builds on).
///
/// [`Identifier`](AppliedPreference::Identifier) carries the identifier it
/// renders, which makes the identifier branch unreachable without one: that is
/// the structural guarantee behind §"Prefer only identifier" — "a variant of
/// preference that implies minimal response semantics, but with a non-empty
/// response body (i.e. the status will be `201 Created` or `200 OK`, never
/// `204 No Content`)".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppliedPreference<'a> {
    /// `return=minimal` — the default when no `Prefer` is sent: no body, and
    /// "if no response body is returned, the service SHOULD use `204 No
    /// Content`" (§"Prefer minimal, identifier or full representation
    /// response").
    Minimal,
    /// `return=identifier` — a body carrying only the affected resource's
    /// identifier (the `&str` is that identifier).
    Identifier(&'a str),
    /// `return=representation` — the full resource representation.
    Representation,
}

impl AppliedPreference<'_> {
    /// The `Preference-Applied` field value this outcome declares.
    pub(crate) fn token(self) -> &'static str {
        match self {
            AppliedPreference::Minimal => "return=minimal",
            AppliedPreference::Identifier(_) => "return=identifier",
            AppliedPreference::Representation => "return=representation",
        }
    }
}

/// Declare the applied return preference on a write response — the ONE place
/// `Preference-Applied` is written, so every write path (canonical RM,
/// JSON-only, demographic, template upload, item-tag collection, and the
/// Simplified-Formats commit) states its outcome the same way.
///
/// NOTE: the header is emitted on every write, including requests that carry
/// no `Prefer` at all — the applied preference is then the spec default,
/// `return=minimal` ("If no `Prefer` header is provided, the default behavior
/// is assumed to be `return=minimal`", §Representation details negotiation).
/// RFC 7240 §3 permits an unsolicited `Preference-Applied`, and stating the
/// applied default is what makes the behaviour uniform across write paths.
pub(crate) fn set_preference_applied(resp: &mut Response, applied: AppliedPreference<'_>) {
    resp.headers_mut().insert(
        PREFERENCE_APPLIED,
        HeaderValue::from_static(applied.token()),
    );
}

/// The return preference a write can actually apply, given the identifier (if
/// any) the write produced.
///
/// `return=identifier` is honoured only when there IS an identifier to return:
/// its whole contract is a non-empty identifier body with a `201`/`200` status
/// (§"Prefer only identifier"), so a write that produced no resource metadata
/// cannot honour it. Rather than emit an empty (possibly `204`) body while
/// claiming `Preference-Applied: return=identifier`, the server applies — and
/// declares — the default `return=minimal`.
fn resolve_write_preference<'a>(
    headers: &HeaderMap,
    uid: Option<&'a str>,
) -> AppliedPreference<'a> {
    if prefers(headers, Preference::Representation) {
        return AppliedPreference::Representation;
    }
    match (prefers(headers, Preference::Identifier), uid) {
        (true, Some(uid)) => AppliedPreference::Identifier(uid),
        _ => AppliedPreference::Minimal,
    }
}

/// The status of an applied `return=identifier` write: the minimal status
/// unless that is `204 No Content`, which the identifier variant forbids —
/// "the status will be `201 Created` or `200 OK`, never `204 No Content`"
/// (§"Prefer only identifier") — in which case the representation status
/// (`200`/`201`) carries the identifier body.
fn identifier_status(minimal_status: StatusCode, repr_status: StatusCode) -> StatusCode {
    if minimal_status == StatusCode::NO_CONTENT {
        repr_status
    } else {
        minimal_status
    }
}

/// The single `Prefer` seam for a create/update response: resolve the
/// preference the write can apply, render its body, and declare it.
///
/// `representation` renders the full body at the status it is handed; it is
/// only called when `return=representation` is applied. The identifier body is
/// rendered by [`identifier_response`] at [`identifier_status`] — structurally
/// never `204`, because the branch is reachable only with an identifier in
/// hand. Resource headers (`ETag`/`Last-Modified`/`Location`) are the caller's
/// business; this seam owns the body + `Preference-Applied` only.
pub(crate) fn write_negotiated(
    headers: &HeaderMap,
    minimal_status: StatusCode,
    repr_status: StatusCode,
    uid: Option<&str>,
    representation: impl FnOnce(StatusCode) -> Response,
) -> Response {
    let applied = resolve_write_preference(headers, uid);
    let mut out = match applied {
        AppliedPreference::Representation => representation(repr_status),
        AppliedPreference::Identifier(uid) => {
            identifier_response(headers, identifier_status(minimal_status, repr_status), uid)
        }
        AppliedPreference::Minimal => empty(minimal_status),
    };
    set_preference_applied(&mut out, applied);
    out
}

/// Render a `return=identifier` response body: `{ "uid": "<uid>" }` in JSON, or
/// the `<uid>` element when XML is negotiated.
pub(crate) fn identifier_response(headers: &HeaderMap, status: StatusCode, uid: &str) -> Response {
    match resolve_accept(headers, CANONICAL, WireFormat::CanonicalJson) {
        Some(WireFormat::CanonicalXml) => {
            // The OAS defines the identifier body only for JSON; the spec is
            // silent on an XML shape, so we emit a minimal `<uid>` element as
            // the direct XML equivalent of the JSON `{uid}`.
            body(
                status,
                APPLICATION_XML,
                format!("<uid>{}</uid>", xml_escape(uid)),
            )
        }
        _ => body(
            status,
            APPLICATION_JSON,
            openehr_its::json::to_canonical_json(&serde_json::json!({ "uid": uid })),
        ),
    }
}

/// Escape the XML text-content special characters.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Render a sub-collection write (the `ITEM_TAG` list of a target): the stored
/// collection at `repr_status` on `Prefer: return=representation` or
/// `return=identifier`, otherwise the empty `minimal_status` body, and the
/// applied preference declared either way.
///
/// The collection is not a `uid`-versioned resource of its own, so its stored
/// list is the identifier-level answer: each tag is identified by its key
/// within the target. No `ETag`/`Location`: the collection write mints no new
/// resource.
// NOTE: the release is silent on a tag write's `return=identifier` answer, and RFC 7240 §4.2
// lets the server pick it; by owner decision (#4475) it is the stored tag list.
pub(crate) fn write_collection(
    headers: &HeaderMap,
    minimal_status: StatusCode,
    repr_status: StatusCode,
    body: &serde_json::Value,
) -> Response {
    if !prefers(headers, Preference::Representation) && prefers(headers, Preference::Identifier) {
        let mut out = respond(headers, repr_status, body);
        set_preference_applied(&mut out, AppliedPreference::Identifier(""));
        return out;
    }
    write_negotiated(headers, minimal_status, repr_status, None, |status| {
        respond(headers, status, body)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderValue;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        h
    }

    fn preference_applied(resp: &Response) -> Option<String> {
        resp.headers()
            .get("preference-applied")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    }

    #[test]
    fn prefer_default_is_minimal() {
        let representation = Preference::Representation;
        assert!(!prefers(&HeaderMap::new(), representation));
        assert!(!prefers(
            &headers(&[("prefer", "return=minimal")]),
            representation
        ));
        assert!(prefers(
            &headers(&[("prefer", "return=representation")]),
            representation
        ));
        assert!(prefers(
            &headers(&[("prefer", "RETURN=REPRESENTATION")]),
            representation
        ));
    }

    #[test]
    fn prefers_identifier_detects_return_identifier() {
        let identifier = Preference::Identifier;
        assert!(prefers(
            &headers(&[("prefer", "return=identifier")]),
            identifier
        ));
        assert!(prefers(
            &headers(&[("prefer", "RETURN=IDENTIFIER")]),
            identifier
        ));
        assert!(!prefers(
            &headers(&[("prefer", "return=minimal")]),
            identifier
        ));
        assert!(!prefers(&HeaderMap::new(), identifier));
    }

    #[test]
    fn the_preference_is_read_from_every_prefer_field() {
        let mut headers = HeaderMap::new();
        assert!(!prefers(&headers, Preference::Representation));
        headers.append("prefer", HeaderValue::from_static("respond-async, wait=10"));
        assert!(!prefers(&headers, Preference::Representation));
        headers.append(
            "prefer",
            HeaderValue::from_static("return = representation; x=1"),
        );
        assert!(prefers(&headers, Preference::Representation));
        let mut minimal = HeaderMap::new();
        minimal.insert("prefer", HeaderValue::from_static("return=minimal"));
        assert!(!prefers(&minimal, Preference::Representation));
    }

    #[test]
    fn resolve_refs_is_read_among_other_preferences() {
        let h = headers(&[("prefer", "return=representation, resolve_refs")]);
        assert!(prefers(&h, Preference::ResolveRefs));
        assert!(!prefers(&HeaderMap::new(), Preference::ResolveRefs));
    }

    /// `Requests_and_responses.md` §"Prefer only identifier": the identifier
    /// variant "implies minimal response semantics, but with a non-empty
    /// response body (i.e. the status will be `201 Created` or `200 OK`, never
    /// `204 No Content`)" — across every (minimal, representation) status pair
    /// a write route uses.
    #[test]
    fn identifier_status_is_never_204() {
        let pairs = [
            (StatusCode::NO_CONTENT, StatusCode::OK),
            (StatusCode::CREATED, StatusCode::CREATED),
            (StatusCode::OK, StatusCode::OK),
            (StatusCode::NO_CONTENT, StatusCode::CREATED),
        ];
        for (minimal, repr) in pairs {
            let got = identifier_status(minimal, repr);
            assert_ne!(
                got,
                StatusCode::NO_CONTENT,
                "return=identifier is `201 Created` or `200 OK`, never `204 No Content` \
                 (overview §Prefer only identifier); minimal={minimal}, repr={repr}"
            );
            assert!(
                got == StatusCode::OK || got == StatusCode::CREATED,
                "return=identifier status must be 200 or 201, got {got}"
            );
        }
    }

    /// The identifier branch is unreachable without an identifier to render,
    /// so an applied `return=identifier` can never degrade into the empty
    /// (possibly `204`) minimal body.
    #[test]
    fn resolve_write_preference_needs_a_uid_for_identifier() {
        let h = headers(&[("prefer", "return=identifier")]);
        assert_eq!(
            resolve_write_preference(&h, Some("v::s::1")),
            AppliedPreference::Identifier("v::s::1")
        );
        assert_eq!(
            resolve_write_preference(&h, None),
            AppliedPreference::Minimal,
            "an unhonourable return=identifier applies the default return=minimal \
             (overview §Representation details negotiation)"
        );
        assert_eq!(
            resolve_write_preference(&HeaderMap::new(), Some("v::s::1")),
            AppliedPreference::Minimal
        );
        assert_eq!(
            resolve_write_preference(
                &headers(&[("prefer", "return=representation")]),
                Some("v::s::1")
            ),
            AppliedPreference::Representation
        );
    }

    /// `Preference-Applied` on the identifier path is the same header the
    /// shared setter writes for every other write route.
    #[test]
    fn set_preference_applied_renders_each_token() {
        let mut out = empty(StatusCode::OK);
        set_preference_applied(&mut out, AppliedPreference::Minimal);
        assert_eq!(preference_applied(&out).as_deref(), Some("return=minimal"));
        set_preference_applied(&mut out, AppliedPreference::Identifier("v::s::1"));
        assert_eq!(
            preference_applied(&out).as_deref(),
            Some("return=identifier")
        );
        set_preference_applied(&mut out, AppliedPreference::Representation);
        assert_eq!(
            preference_applied(&out).as_deref(),
            Some("return=representation")
        );
    }
}
