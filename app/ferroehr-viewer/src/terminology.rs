// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The CDR's **terminology surface** as the viewer consumes it.
//!
//! Six reads — the known terminology ids, one terminology's descriptor, a term
//! definition, a strict subsumption test, a value set's members, and a
//! value-set membership test — behind the viewer's own session guard.
//!
//! NOTE: no openEHR spec governs this wire shape — the CDR's own extension
//! realizing SM `I_TERMINOLOGY_SERVICE`
//! (`docs/specs/openehr/SM/docs/openehr_platform/master12-terminology_service.adoc`).
//!
//! **Absence is a state, not an error.** The whole group is config-gated on the
//! CDR side (`[terminology].api_enabled`, off by default) and answers `404` as
//! if unmounted when it is off — the same `404` an unknown terminology, code or
//! value set produces. Every read below therefore returns `Ok(None)` for a
//! `404` and lets the screen say what is absent, exactly as
//! [`crate::management`] does for the management surface; an error is reserved
//! for a CDR that refused or failed.
//!
//! Every `#[server]` fn guards with
//! [`require_session`](crate::session::require_session) first (a server fn is a
//! publicly reachable HTTP endpoint) and keeps the CDR credential server-side.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694)"
)]

use leptos::server;
use serde::{Deserialize, Serialize};

use crate::error::ViewerError;

/// One terminology's descriptor, flattened for the descriptor card.
///
/// The attributes are `Terminology_description`'s own
/// (`docs/specs/openehr/SM/docs/UML/classes/terminology_description.adoc`):
/// `publisher` (1..1), `available_versions` (0..1), `attributes` (0..1) and
/// `uri` (1..1). An absent optional reads as empty rather than failing the
/// card, and `name` is carried only because a CDR may publish one — it is
/// rendered when present and never invented.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TerminologyDescriptor {
    /// The terminology id the descriptor was read for.
    pub terminology_id: String,
    /// A display name, when the CDR published one; empty otherwise.
    pub name: String,
    /// `publisher` — the publishing organisation.
    pub publisher: String,
    /// `uri` — the published identifying URI.
    pub uri: String,
    /// `available_versions` — empty when the CDR published none.
    pub available_versions: Vec<String>,
    /// `attributes` — the meta-model attributes an extract request may ask
    /// for; empty when the CDR published none.
    pub attributes: Vec<String>,
}

/// One term of a `Terminology_extract`, flattened for the terms table.
///
/// A `_terms_` entry is either a bare `Term_code` or a fully defined
/// `Defined_term` (`terminology_extract.adoc`); the bare form leaves
/// [`Self::text`] and [`Self::language`] empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermRow {
    /// `Term_code.code`.
    pub code: String,
    /// `Defined_term.text` — empty for a bare code.
    pub text: String,
    /// `Defined_term.language` — empty when the extract carried none.
    pub language: String,
    /// `Defined_term.is_preferred_term`, defaulting to false when absent.
    pub preferred: bool,
}

impl TermRow {
    /// Renders the term as `code — text`, or the bare code when the extract
    /// carried no text.
    ///
    /// The one rubric spelling in the viewer: the terminology browser's table
    /// and the query builder's validated code chips both read it, so a term
    /// looks the same wherever it appears.
    #[must_use]
    pub fn rubric(&self) -> String {
        if self.text.is_empty() {
            self.code.clone()
        } else {
            format!("{} — {}", self.code, self.text)
        }
    }
}

/// One `Term_relationship` of an extract (`term_relationship.adoc`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermRelationshipRow {
    /// `origin_code` — the left-hand concept.
    pub origin_code: String,
    /// `relation_name` — the relation this row instantiates.
    pub relation_name: String,
    /// `target_codes` — the right-hand concepts; empty when absent.
    pub target_codes: Vec<String>,
}

/// A `Terminology_extract` flattened for rendering (`terminology_extract.adoc`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TerminologyExtractView {
    /// `terminology_id` — the namespace the terms belong to.
    pub terminology_id: String,
    /// `terminology_version` — empty when the CDR published none.
    pub version: String,
    /// The extract's terms, code-sorted so both render passes agree
    /// (hydration determinism).
    pub terms: Vec<TermRow>,
    /// The extract's relationships, in the order the CDR listed them.
    pub relationships: Vec<TermRelationshipRow>,
}

/// The terminology ids of a `GET /terminology` body
/// (`{"terminology_ids": [..]}`), in the order the CDR listed them.
///
/// Read defensively: a body without the field yields no ids rather than
/// failing the screen.
#[must_use]
pub fn terminology_ids(body: &serde_json::Value) -> Vec<String> {
    string_list(body.get("terminology_ids"))
}

/// Flatten a `Terminology_description` body into a [`TerminologyDescriptor`].
///
/// `terminology_id` comes from the request, not the body: the descriptor
/// resource is addressed by id and the released shape carries none.
#[must_use]
pub fn descriptor_view(terminology_id: &str, body: &serde_json::Value) -> TerminologyDescriptor {
    TerminologyDescriptor {
        terminology_id: terminology_id.to_owned(),
        name: text_at(body, "name"),
        publisher: text_at(body, "publisher"),
        uri: text_at(body, "uri"),
        available_versions: string_list(body.get("available_versions")),
        attributes: string_list(body.get("attributes")),
    }
}

/// Flatten a `Terminology_extract` body into a [`TerminologyExtractView`].
///
/// `_terms_` is a code-keyed map whose values are a bare `Term_code` or a
/// `Defined_term`; both shapes carry `code`, so the row is built from the
/// value and falls back to the map key when the value omits it.
#[must_use]
pub fn extract_view(body: &serde_json::Value) -> TerminologyExtractView {
    let mut terms: Vec<TermRow> = body
        .get("terms")
        .and_then(serde_json::Value::as_object)
        .map(|map| {
            map.iter()
                .map(|(key, value)| TermRow {
                    code: {
                        let code = text_at(value, "code");
                        if code.is_empty() { key.clone() } else { code }
                    },
                    text: text_at(value, "text"),
                    language: text_at(value, "language"),
                    preferred: value
                        .get("is_preferred_term")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                })
                .collect()
        })
        .unwrap_or_default();
    terms.sort_by(|a, b| a.code.cmp(&b.code));
    let relationships = body
        .get("relationships")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| TermRelationshipRow {
                    origin_code: text_at(item, "origin_code"),
                    relation_name: text_at(item, "relation_name"),
                    target_codes: string_list(item.get("target_codes")),
                })
                .collect()
        })
        .unwrap_or_default();
    TerminologyExtractView {
        terminology_id: text_at(body, "terminology_id"),
        version: text_at(body, "terminology_version"),
        terms,
        relationships,
    }
}

/// Read a boolean verdict body (`{"subsumes": bool}` / `{"valid": bool}`),
/// defaulting to false when the field is absent or not a boolean.
#[must_use]
pub fn verdict(body: &serde_json::Value, key: &str) -> bool {
    body.get(key)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// A string leaf of an object, or an empty string when absent or not a string.
fn text_at(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// Every string element of an optional JSON array, dropping non-strings.
fn string_list(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// GET one terminology resource as the session's credential, mapping the CDR's
/// `404` to `Ok(None)` — the extension being off, and the addressed
/// terminology/code/value set not existing, are all that one status.
///
/// Guards the viewer session first: the server fns below are publicly
/// reachable endpoints, and this is the one place their CDR call is made.
#[cfg(feature = "ssr")]
async fn terminology_get(path: &str) -> Result<Option<serde_json::Value>, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = leptos::prelude::expect_context();
    let url = state.cdr.rest_v1(path);
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    if response.is(http::StatusCode::NOT_FOUND) {
        return Ok(None);
    }
    let body = crate::cdr::CdrClient::expect_success(response)?.body;
    serde_json::from_str::<serde_json::Value>(&body)
        .map(Some)
        .map_err(|e| ViewerError::Internal(format!("terminology JSON: {e}")))
}

/// The optional `at_date` query fragment, introduced by `lead` (`?` when it
/// opens the query string, `&` when it extends one) — empty when the caller
/// gave no date, which asks the CDR for the current definition.
#[cfg(feature = "ssr")]
fn at_date_query(at_date: &str, lead: char) -> String {
    let at_date = at_date.trim();
    if at_date.is_empty() {
        String::new()
    } else {
        format!("{lead}at_date={}", urlencoding::encode(at_date))
    }
}

/// Refuse a blank identifier before it reaches a CDR path or query parameter.
#[cfg(feature = "ssr")]
fn require_value(value: &str, what: &str) -> Result<String, ViewerError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ViewerError::Invalid(format!("a {what} is required")));
    }
    Ok(value.to_owned())
}

/// The terminology ids the CDR serves (`GET /terminology`).
///
/// `Ok(None)` is the first-class "the terminology extension is disabled on this
/// server" state (`[terminology].api_enabled` off — the CDR answers `404` as if
/// the routes were unmounted), not an error.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] when the CDR
/// refuses this session;
/// [`ViewerError::Cdr`] / [`ViewerError::CdrUnreachable`] from the CDR;
/// [`ViewerError::Internal`] when the body is not JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn list_terminologies() -> Result<Option<Vec<String>>, ViewerError> {
    let Some(body) = terminology_get("terminology").await? else {
        return Ok(None);
    };
    Ok(Some(terminology_ids(&body)))
}

/// One terminology's descriptor (`GET /terminology/{terminology_id}`).
///
/// `Ok(None)` = unknown terminology, or the extension is disabled.
///
/// # Errors
/// [`ViewerError::Invalid`] for a blank id; otherwise as
/// [`list_terminologies`].
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_terminology_description(
    /// The terminology to describe.
    terminology_id: String,
) -> Result<Option<TerminologyDescriptor>, ViewerError> {
    let terminology_id = require_value(&terminology_id, "terminology id")?;
    let path = format!("terminology/{}", urlencoding::encode(&terminology_id));
    Ok(terminology_get(&path)
        .await?
        .map(|body| descriptor_view(&terminology_id, &body)))
}

/// One term's definition
/// (`GET /terminology/{terminology_id}/term/{code}?at_date=…`).
///
/// `Ok(None)` = unknown terminology OR unknown code, or the extension is
/// disabled — the screen says which of those it asked for. An empty `at_date`
/// is omitted from the request, which asks for the current definition.
///
/// # Errors
/// [`ViewerError::Invalid`] for a blank terminology id or code; otherwise as
/// [`list_terminologies`].
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_term(
    /// The terminology holding the code.
    terminology_id: String,
    /// The term code to define.
    code: String,
    /// An optional ISO-8601 effective date; empty asks for the current
    /// definition.
    at_date: String,
) -> Result<Option<TerminologyExtractView>, ViewerError> {
    let terminology_id = require_value(&terminology_id, "terminology id")?;
    let code = require_value(&code, "term code")?;
    let path = format!(
        "terminology/{}/term/{}{}",
        urlencoding::encode(&terminology_id),
        urlencoding::encode(&code),
        at_date_query(&at_date, '?')
    );
    Ok(terminology_get(&path)
        .await?
        .map(|body| extract_view(&body)))
}

/// The strict subsumption verdict for two codes
/// (`GET /terminology/{terminology_id}/subsumes?ref_code=…&candidate=…`).
///
/// `Ok(None)` = unknown terminology, or the extension is disabled. Both codes
/// are required by the wire (`400` otherwise), so both are checked here first.
///
/// # Errors
/// [`ViewerError::Invalid`] for a blank terminology id or code; otherwise as
/// [`list_terminologies`].
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn check_subsumption(
    /// The terminology both codes belong to.
    terminology_id: String,
    /// The reference (ancestor-candidate) code.
    ref_code: String,
    /// The candidate (descendant) code.
    candidate: String,
) -> Result<Option<bool>, ViewerError> {
    let terminology_id = require_value(&terminology_id, "terminology id")?;
    let ref_code = require_value(&ref_code, "reference code")?;
    let candidate = require_value(&candidate, "candidate code")?;
    let path = format!(
        "terminology/{}/subsumes?ref_code={}&candidate={}",
        urlencoding::encode(&terminology_id),
        urlencoding::encode(&ref_code),
        urlencoding::encode(&candidate)
    );
    Ok(terminology_get(&path)
        .await?
        .map(|body| verdict(&body, "subsumes")))
}

/// A value set's member terms
/// (`GET /terminology/{terminology_id}/value_set/{value_set_id}`).
///
/// `Ok(None)` = unknown terminology or value set, or the extension is disabled.
///
/// # Errors
/// [`ViewerError::Invalid`] for a blank terminology id or value-set id;
/// otherwise as [`list_terminologies`].
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_value_set(
    /// The terminology holding the value set.
    terminology_id: String,
    /// The value set to expand.
    value_set_id: String,
) -> Result<Option<TerminologyExtractView>, ViewerError> {
    let terminology_id = require_value(&terminology_id, "terminology id")?;
    let value_set_id = require_value(&value_set_id, "value set id")?;
    let path = format!(
        "terminology/{}/value_set/{}",
        urlencoding::encode(&terminology_id),
        urlencoding::encode(&value_set_id)
    );
    Ok(terminology_get(&path)
        .await?
        .map(|body| extract_view(&body)))
}

/// Whether a code is a member of a value set
/// (`GET /terminology/{terminology_id}/value_set/{value_set_id}/validate?candidate_code=…`).
///
/// `Ok(None)` = the extension is disabled (the membership test itself answers
/// `false` rather than `404` for a value set the CDR does not know).
///
/// # Errors
/// [`ViewerError::Invalid`] for a blank terminology id, value-set id or
/// candidate code; otherwise as [`list_terminologies`].
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn validate_value_set_code(
    /// The terminology holding the value set.
    terminology_id: String,
    /// The value set to test membership in.
    value_set_id: String,
    /// The code whose membership is in question.
    candidate_code: String,
    /// An optional ISO-8601 effective date; empty asks for the current value
    /// set.
    at_date: String,
) -> Result<Option<bool>, ViewerError> {
    let terminology_id = require_value(&terminology_id, "terminology id")?;
    let value_set_id = require_value(&value_set_id, "value set id")?;
    let candidate_code = require_value(&candidate_code, "candidate code")?;
    let path = format!(
        "terminology/{}/value_set/{}/validate?candidate_code={}{}",
        urlencoding::encode(&terminology_id),
        urlencoding::encode(&value_set_id),
        urlencoding::encode(&candidate_code),
        at_date_query(&at_date, '&')
    );
    Ok(terminology_get(&path)
        .await?
        .map(|body| verdict(&body, "valid")))
}

#[cfg(test)]
mod tests {
    use super::{
        TermRow, TerminologyDescriptor, descriptor_view, extract_view, terminology_ids, verdict,
    };

    #[test]
    fn terminology_ids_read_the_wire_order_and_survive_a_missing_field() {
        let body = serde_json::json!({
            "terminology_ids": ["openehr", "ISO_639-1", "ISO_3166-1"]
        });
        assert_eq!(
            terminology_ids(&body),
            vec![
                "openehr".to_owned(),
                "ISO_639-1".to_owned(),
                "ISO_3166-1".to_owned()
            ]
        );
        assert!(terminology_ids(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn the_descriptor_carries_the_requested_id_and_its_optionals() {
        // The bundle provider's own openEHR descriptor shape: publisher + uri
        // (1..1), available_versions present, attributes absent.
        let body = serde_json::json!({
            "publisher": "openEHR Foundation",
            "available_versions": ["3.1.0"],
            "attributes": null,
            "uri": "https://github.com/openEHR/terminology"
        });
        assert_eq!(
            descriptor_view("openehr", &body),
            TerminologyDescriptor {
                terminology_id: "openehr".to_owned(),
                name: String::new(),
                publisher: "openEHR Foundation".to_owned(),
                uri: "https://github.com/openEHR/terminology".to_owned(),
                available_versions: vec!["3.1.0".to_owned()],
                attributes: Vec::new(),
            }
        );
        // A descriptor with nothing in it still renders — the card never
        // invents a publisher or a uri it was not given.
        let empty = descriptor_view("ISO_639-1", &serde_json::json!({}));
        assert_eq!(empty.terminology_id, "ISO_639-1");
        assert!(empty.publisher.is_empty() && empty.uri.is_empty());
    }

    #[test]
    fn an_extract_flattens_defined_and_bare_terms_code_sorted() {
        // A `Defined_term` (code + text + language) beside a bare `Term_code`
        // (`terminology_extract.adoc` — `_terms_` admits both subtypes).
        let body = serde_json::json!({
            "terminology_id": "openehr",
            "terminology_version": "3.1.0",
            "terms": {
                "532": { "code": "532", "text": "complete", "language": "en",
                         "is_preferred_term": true },
                "249": { "code": "249", "text": "creation", "language": "en" },
                "111": { "code": "111" }
            }
        });
        let view = extract_view(&body);
        assert_eq!(view.terminology_id, "openehr");
        assert_eq!(view.version, "3.1.0");
        let codes: Vec<&str> = view.terms.iter().map(|t| t.code.as_str()).collect();
        assert_eq!(codes, vec!["111", "249", "532"]);
        assert_eq!(view.terms[1].rubric(), "249 — creation");
        assert!(!view.terms[1].preferred);
        assert!(view.terms[2].preferred);
        // The bare code carries no text, so its rubric IS the code.
        assert_eq!(view.terms[0].rubric(), "111");
        assert!(view.terms[0].language.is_empty());
        assert!(view.relationships.is_empty());
    }

    #[test]
    fn a_term_entry_without_its_own_code_falls_back_to_the_map_key() {
        let body = serde_json::json!({ "terms": { "249": { "text": "creation" } } });
        let view = extract_view(&body);
        assert_eq!(view.terms.len(), 1);
        assert_eq!(view.terms[0].code, "249");
        assert_eq!(view.terms[0].rubric(), "249 — creation");
    }

    #[test]
    fn relationships_render_their_origin_relation_and_targets() {
        let body = serde_json::json!({
            "terminology_id": "openehr",
            "relationships": [
                { "origin_code": "249", "relation_name": "is_a",
                  "target_codes": ["250", "251"] },
                { "origin_code": "532", "relation_name": "is_a" }
            ]
        });
        let view = extract_view(&body);
        assert_eq!(view.relationships.len(), 2);
        assert_eq!(view.relationships[0].origin_code, "249");
        assert_eq!(view.relationships[0].relation_name, "is_a");
        assert_eq!(
            view.relationships[0].target_codes,
            vec!["250".to_owned(), "251".to_owned()]
        );
        // An absent `target_codes` (0..1) is an empty list, never a panic.
        assert!(view.relationships[1].target_codes.is_empty());
    }

    #[test]
    fn a_verdict_body_reads_its_own_key_and_defaults_to_false() {
        assert!(verdict(
            &serde_json::json!({ "subsumes": true }),
            "subsumes"
        ));
        assert!(!verdict(
            &serde_json::json!({ "subsumes": false }),
            "subsumes"
        ));
        assert!(verdict(&serde_json::json!({ "valid": true }), "valid"));
        // A body missing the key is NOT a true verdict.
        assert!(!verdict(&serde_json::json!({}), "valid"));
        assert!(!verdict(&serde_json::json!({ "valid": "yes" }), "valid"));
    }

    #[test]
    fn the_rubric_is_the_one_code_plus_text_spelling() {
        let defined = TermRow {
            code: "249".to_owned(),
            text: "creation".to_owned(),
            language: "en".to_owned(),
            preferred: false,
        };
        assert_eq!(defined.rubric(), "249 — creation");
        let bare = TermRow {
            text: String::new(),
            ..defined
        };
        assert_eq!(bare.rubric(), "249");
    }
}
