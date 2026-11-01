// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `/templates` screen — the Template Manager list + template upload.
//!
//! One screen, two template FAMILIES, switched by `?family=` (the switch is
//! deep-linkable URL state, not a private signal): ADL 1.4 operational
//! templates (`definition/template/adl1.4`, OPT/XML upload) and ADL2 ones
//! (`definition/template/adl2`, `text/plain` ADL2 source upload). Each family
//! has its own list, its own upload affordance, and its own detail route; the
//! client-side text filter and the URL page window serve both. No openEHR spec
//! governs the viewer — our own design / product extension; the wire it
//! reads/writes is the ITS-REST Definition API.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST, not the CDR internal seams \
              (#1694)"
)]

use crate::adl2::TemplateFamily;
use crate::bff::dashboard::Count;
use crate::error::ViewerError;
use crate::locale::{Msg, tr_served, tr_served_with};

/// Which template a delete addresses — the id, and the family that decides
/// WHICH wire route removes it.
///
/// The family travels with the id rather than being read off the URL at
/// confirm time: the row that opened the dialog is the one that knows its own
/// family, so the confirmation copy and the dispatched route can never
/// disagree with the row the reader clicked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteTarget {
    /// The archetype-model family the template belongs to.
    pub family: TemplateFamily,
    /// The `template_id` (ADL 1.4) or artefact HRID (ADL2) to remove.
    pub template_id: String,
}

/// One row of the template list, distilled from the ITS-REST Definition list
/// shape (`template_id` / `concept` / `created_timestamp`).
///
/// Shared across both compilation targets, so it carries only fixed-size,
/// client-safe fields (no `usize` in serialized types).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TemplateRow {
    /// The operational-template id (the detail route segment).
    pub template_id: String,
    /// The template concept / display name.
    pub concept: String,
    /// The root archetype id the template constrains.
    pub archetype_id: String,
    /// The creation timestamp as the CDR reported it (raw string).
    pub created: String,
}

impl TemplateRow {
    /// Whether the template describes a COMPOSITION, the one class a
    /// composition editor commits and the query builder reaches.
    ///
    /// The root archetype id names its RM class in its `rm_entity` part
    /// (`openEHR-EHR-COMPOSITION.encounter.v1`; BASE `master05-identification_package.adoc`
    /// §Archetype identifier: `rm_originator-rm_name-rm_entity`, compared
    /// case-insensitively). A demographic template
    /// (`openEHR-DEMOGRAPHIC-ROLE.Role.v1`) describes a party instead. A row
    /// whose root the CDR did not report is kept, since nothing says what it
    /// describes.
    #[must_use]
    pub fn is_composition(&self) -> bool {
        root_is_composition(&self.archetype_id)
    }
}

/// Whether `archetype_id` names a COMPOSITION root (see
/// [`TemplateRow::is_composition`]); an empty id is not known not to.
fn root_is_composition(archetype_id: &str) -> bool {
    if archetype_id.trim().is_empty() {
        return true;
    }
    archetype_id
        .split('.')
        .next()
        .and_then(|qualified| qualified.splitn(3, '-').nth(2))
        .is_some_and(|entity| entity.eq_ignore_ascii_case("COMPOSITION"))
}

/// List the CDR's ADL 1.4 operational templates.
///
/// GET `definition/template/adl1.4?version=*` with
/// `Accept: application/json`; the body is an array whose elements are parsed
/// defensively (missing fields become empty strings) into [`TemplateRow`]s.
/// `version=*` pins the FULL inventory: an absent `version` collapses the
/// CDR's listing to the latest version of each template (the released OAS
/// `parameters/query/filter_version.yaml`), and a management client must
/// show every stored version, not just the latest.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] / [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] from the CDR; [`ViewerError::Internal`]
/// when the CDR body is not valid JSON.
pub async fn list_templates() -> Result<Vec<TemplateRow>, ViewerError> {
    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
    let url = state.cdr.rest_v1("definition/template/adl1.4?version=*");
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    let body = crate::cdr::CdrClient::expect_success(response)?.body;
    let value = serde_json::from_str::<serde_json::Value>(&body).map_err(|e| {
        ViewerError::Internal(tr_served_with(
            Msg::TemplateListJson,
            &[("e", (e).to_string().into())],
        ))
    })?;
    let rows = value
        .as_array()
        .map(|items| items.iter().map(template_row).collect())
        .unwrap_or_default();
    Ok(rows)
}

/// Distil one Definition-list element into a [`TemplateRow`], reading each
/// field defensively so a missing/renamed field yields an empty string rather
/// than dropping the row.
fn template_row(item: &serde_json::Value) -> TemplateRow {
    let text = |key: &str| {
        item.get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    TemplateRow {
        template_id: text("template_id"),
        concept: text("concept"),
        archetype_id: text("archetype_id"),
        created: text("created_timestamp"),
    }
}

/// Upload an operational template (OPT/XML) to the CDR.
///
/// POST `definition/template/adl1.4` with `Content-Type: application/xml`,
/// `Accept: application/xml` and `Prefer: return=representation`, as
/// [`upload_adl2_template`] does: a `400` names each violation
/// ([`TemplateUpload::Invalid`]), and the `201` carries the stored OPT, so
/// the `template_id` is read from its `ETag`, else from its `Location`. The
/// other refusals (a `400` without violations, `409` duplicate) surface through
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] for empty content;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] / [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] from the CDR (the diagnostic verbatim).
pub async fn upload_template(
    // The operational-template XML to upload, as read in the browser.
    opt_xml: String,
) -> Result<TemplateUpload, ViewerError> {
    let session = crate::session::require_session()?;
    if opt_xml.trim().is_empty() {
        return Err(ViewerError::Invalid(tr_served(Msg::SelectedFileWasEmpty)));
    }
    let state = crate::scope::state()?;
    let url = state.cdr.rest_v1("definition/template/adl1.4");
    let response = state
        .cdr
        .post(
            &session.credential,
            &url,
            "application/xml",
            "application/xml",
            &[("Prefer", "return=representation")],
            opt_xml,
        )
        .await?;
    upload_answer(response)
}

/// List the CDR's ADL2 operational templates.
///
/// GET `definition/template/adl2?version=*` with
/// `Accept: application/json`. The row shape is the same four fields the ADL
/// 1.4 listing carries (`template_id` / `concept` / `archetype_id` /
/// `created_timestamp`), so both families share one [`TemplateRow`] and one
/// parser. `version=*` pins the FULL inventory: without it the CDR collapses
/// the listing to the latest version of each HRID family (the released OAS
/// `parameters/query/filter_version.yaml`), which would hide every superseded
/// ADL2 artefact from a management client.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] / [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] from the CDR; [`ViewerError::Internal`]
/// when the CDR body is not valid JSON.
pub async fn list_adl2_templates() -> Result<Vec<TemplateRow>, ViewerError> {
    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
    let url = state.cdr.rest_v1("definition/template/adl2?version=*");
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    let body = crate::cdr::CdrClient::expect_success(response)?.body;
    let value = serde_json::from_str::<serde_json::Value>(&body).map_err(|e| {
        ViewerError::Internal(tr_served_with(
            Msg::Adl2TemplateListJson,
            &[("e", (e).to_string().into())],
        ))
    })?;
    let rows = value
        .as_array()
        .map(|items| items.iter().map(template_row).collect())
        .unwrap_or_default();
    Ok(rows)
}

/// What the CDR answered a template upload it did not refuse outright.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateUpload {
    /// Stored, under the id the answer names, when it names one.
    Stored(Option<String>),
    /// Refused `400`, with every violation the CDR found, in its order.
    Invalid(Vec<Violation>),
}

/// One violation the CDR found in a template: the rule it breaks, or the
/// path it applies at, and what the CDR says about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// The rule code, such as AOM2's `VARXR` (`master04.5` §Validity Rules),
    /// or the path the violation applies at.
    pub rule: String,
    /// The CDR's own words for this occurrence.
    pub detail: String,
}

/// The violations a refusal's `errors` member lists, in the CDR's order;
/// none when the body is no problem or carries no `errors`.
///
/// The CDR names the rule code of a violation in the entry's `path` and
/// lists the entries only to a request that prefers the representation
/// (ITS-REST `Requests_and_responses.md` §HTTP status codes). The shape is
/// the CDR's own; no openEHR spec governs it (our own design).
#[must_use]
pub fn violations(body: &str) -> Vec<Violation> {
    // NOTE: our own design: a body that is no problem document lists no
    // violation, and the caller draws the refusal from the status instead.
    let Ok(problem) = serde_json::from_str::<Problem>(body) else {
        return Vec::new();
    };
    problem
        .errors
        .into_iter()
        .filter(|entry| !entry.path.is_empty() || !entry.detail.is_empty())
        .map(|entry| Violation {
            rule: entry.path,
            detail: entry.detail,
        })
        .collect()
}

/// The members of a refusal [`violations`] reads.
#[derive(Debug, serde::Deserialize)]
struct Problem {
    /// Every violation, when the request preferred the representation.
    #[serde(default)]
    errors: Vec<ProblemError>,
}

/// One violation of a refusal.
#[derive(Debug, serde::Deserialize)]
struct ProblemError {
    /// Where it applies, or the rule code it breaks.
    #[serde(default)]
    path: String,
    /// What it says.
    #[serde(default)]
    detail: String,
}

/// Upload an ADL2 operational template (the artefact SOURCE) to the CDR.
///
/// POST `definition/template/adl2` with `Content-Type: text/plain`, the
/// operation's single declared body type, and
/// `Prefer: return=representation`: a `4xx` then carries the additional
/// error details (ITS-REST `Requests_and_responses.md` §HTTP status codes),
/// so a `400` names each AOM2 rule the source breaks
/// ([`TemplateUpload::Invalid`]). The `201` carries the stored source as its
/// body, so the stored HRID is read from its `ETag`, else from the last
/// segment of its `Location` (§Prefer minimal, identifier or full
/// representation response). The other refusals (a `400` without violations
/// for an unparseable source, `409` duplicate HRID) surface through
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] for empty content;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] / [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] from the CDR (the diagnostic verbatim).
pub async fn upload_adl2_template(
    // The ADL2 operational-template source, pasted or read from a file.
    adl2_source: String,
) -> Result<TemplateUpload, ViewerError> {
    let session = crate::session::require_session()?;
    if adl2_source.trim().is_empty() {
        return Err(ViewerError::Invalid(tr_served(
            Msg::ThereNoAdl2SourceUpload,
        )));
    }
    let state = crate::scope::state()?;
    let url = state.cdr.rest_v1("definition/template/adl2");
    let response = state
        .cdr
        .post(
            &session.credential,
            &url,
            "text/plain",
            "text/plain",
            &[("Prefer", "return=representation")],
            adl2_source,
        )
        .await?;
    upload_answer(response)
}

/// The [`TemplateUpload`] of the CDR's `response` to an upload of either
/// family.
fn upload_answer(response: crate::cdr::CdrResponse) -> Result<TemplateUpload, ViewerError> {
    // NOTE: both uploads declare only 201, 400 and 409
    // (`operations/definition_template_adl1.4_upload.yaml`, `_adl2_upload.yaml`).
    if response.is(http::StatusCode::BAD_REQUEST) {
        let found = violations(&response.body);
        if !found.is_empty() {
            return Ok(TemplateUpload::Invalid(found));
        }
    }
    let stored = crate::cdr::CdrClient::expect_success(response)?;
    Ok(TemplateUpload::Stored(stored_id(&stored)))
}

/// The template id a `201` names: its `ETag`, else the last segment of its
/// `Location`, decoded.
fn stored_id(response: &crate::cdr::CdrResponse) -> Option<String> {
    response.etag_version_uid().or_else(|| {
        let location = response.location.as_deref()?;
        let segment = location.trim_end_matches('/').rsplit('/').next()?;
        let decoded = urlencoding::decode(segment).ok()?;
        (!decoded.is_empty()).then(|| decoded.into_owned())
    })
}

/// Delete one stored ADL2 artefact
/// (`DELETE definition/artefact/adl2/{artefact_id}`).
///
/// The ADL2 store's templates are removed through the ARTEFACT resource, not
/// through the Admin API's `admin/template/{template_id}` (which addresses the
/// ADL 1.4 store alone): the route is the CDR's own extension, whose unknown
/// artefact is the `404` — which is also the answer for
/// a malformed HRID, since the store key is opaque. A template-kind artefact
/// that committed versions still reference is refused `409` with the
/// referencing count in the diagnostic, and that text surfaces verbatim
/// through [`delete_failure_copy`](crate::admin::delete_failure_copy).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] for an empty id;
/// [`ViewerError::Cdr`] / [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] /
/// [`ViewerError::CdrUnreachable`] from the CDR.
pub async fn delete_adl2_artefact(
    // The AOM2 artefact HRID to remove.
    artefact_id: String,
) -> Result<(), ViewerError> {
    let session = crate::session::require_session()?;
    if artefact_id.trim().is_empty() {
        return Err(ViewerError::Invalid(tr_served(Msg::NoAdl2ArtefactIdDelete)));
    }
    let state = crate::scope::state()?;
    let url = state
        .cdr
        .extensions_v1(&crate::adl2::artefact_path(&artefact_id));
    let response = state.cdr.delete(&session.credential, &url, &[]).await?;
    crate::cdr::CdrClient::expect_success(response)?;
    Ok(())
}

/// The compositions committed against one template, counted by the template
/// id their `archetype_details` carry (RM `LOCATABLE.archetype_details`,
/// `ARCHETYPED.template_id`). Validated by the `release_count_aql_parses` test.
const COMPOSITIONS_OF_TEMPLATE_AQL: &str = "SELECT COUNT(*) FROM EHR e CONTAINS COMPOSITION c WHERE c/archetype_details/template_id/value = $template_id";

/// How many compositions the CDR holds against each of `template_ids`, in
/// their order: the [`CountShape`](crate::bff::dashboard::CountShape) when
/// the CDR answered a result set of another shape, `Err` with the CDR's
/// refusal of that one count.
///
/// One `POST query/aql` per template, the id a query parameter and never
/// part of the query text, bounded by
/// [`FANOUT_CONCURRENCY`](crate::cdr::FANOUT_CONCURRENCY).
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session; each count
/// carries its own CDR error.
pub async fn count_compositions(
    // The templates to count the compositions of.
    template_ids: Vec<String>,
) -> Result<Vec<Result<Count, ViewerError>>, ViewerError> {
    use futures::stream::StreamExt;

    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
    let (state, session) = (&state, &session);
    Ok(
        futures::stream::iter(template_ids.into_iter().map(|template_id| async move {
            crate::bff::dashboard::run_count_aql(
                state,
                session,
                COMPOSITIONS_OF_TEMPLATE_AQL,
                serde_json::json!({ "template_id": template_id }),
            )
            .await
        }))
        .buffered(crate::cdr::FANOUT_CONCURRENCY)
        .collect()
        .await,
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_composition_root_is_a_composition_template() {
        use super::root_is_composition;
        assert!(root_is_composition("openEHR-EHR-COMPOSITION.encounter.v1"));
        assert!(root_is_composition("openEHR-EHR-composition.minimal.v1"));
        assert!(!root_is_composition("openEHR-DEMOGRAPHIC-ROLE.Role.v1"));
        assert!(!root_is_composition("openEHR-DEMOGRAPHIC-PERSON.person.v2"));
        assert!(!root_is_composition(
            "openEHR-EHR-OBSERVATION.blood_pressure.v2"
        ));
        assert!(root_is_composition(""), "an unreported root is kept");
    }

    /// Both families' listings are read through this one parser, so it is
    /// pinned against the four fields the Definition API returns.
    #[test]
    fn a_list_element_distils_into_a_row_and_missing_fields_stay_empty() {
        let item = serde_json::json!({
            "template_id": "openEHR-EHR-COMPOSITION.cnf_vitals.v1.0.0",
            "concept": "cnf_vitals",
            "archetype_id": "openEHR-EHR-COMPOSITION.cnf_vitals.v1.0.0",
            "created_timestamp": "2026-08-22T14:51:32.596215Z",
        });
        let row = crate::bff::templates::template_row(&item);
        assert_eq!(row.template_id, "openEHR-EHR-COMPOSITION.cnf_vitals.v1.0.0");
        assert_eq!(row.concept, "cnf_vitals");
        assert_eq!(row.created, "2026-08-22T14:51:32.596215Z");
        // A renamed or absent field empties that cell rather than dropping the
        // whole row from the listing.
        let sparse = serde_json::json!({ "template_id": "only-the-id" });
        let row = crate::bff::templates::template_row(&sparse);
        assert_eq!(row.template_id, "only-the-id");
        assert_eq!(row.concept, "");
        assert_eq!(row.archetype_id, "");
        assert_eq!(row.created, "");
    }

    #[test]
    fn release_count_aql_parses() {
        openehr_query::parser::parse_str(super::COMPOSITIONS_OF_TEMPLATE_AQL)
            .expect("the release count AQL must parse");
    }

    fn answer(
        status: http::StatusCode,
        etag: Option<&str>,
        location: Option<&str>,
        body: &str,
    ) -> crate::cdr::CdrResponse {
        crate::cdr::CdrResponse {
            status,
            content_type: None,
            etag: etag.map(str::to_owned),
            location: location.map(str::to_owned),
            last_modified: None,
            preference_applied: None,
            body: body.to_owned(),
        }
    }

    #[test]
    fn a_stored_upload_names_the_hrid_its_etag_or_location_carries() {
        use super::{TemplateUpload, upload_answer};
        const HRID: &str = "openEHR-EHR-COMPOSITION.cnf_minimal.v1.0.0";
        let source = "template (adl_version=2.4.0; rm_release=1.1.0)";
        let by_etag = answer(
            http::StatusCode::CREATED,
            Some(&format!("W/\"{HRID}\"")),
            None,
            source,
        );
        assert_eq!(
            upload_answer(by_etag),
            Ok(TemplateUpload::Stored(Some(HRID.to_owned())))
        );
        let by_location = answer(
            http::StatusCode::CREATED,
            None,
            Some(&format!(
                "https://cdr.example/rest/openehr/v1/definition/template/adl2/{HRID}"
            )),
            source,
        );
        assert_eq!(
            upload_answer(by_location),
            Ok(TemplateUpload::Stored(Some(HRID.to_owned())))
        );
        assert_eq!(
            upload_answer(answer(http::StatusCode::CREATED, None, None, source)),
            Ok(TemplateUpload::Stored(None))
        );
    }

    #[test]
    fn a_refused_upload_lists_each_violation_and_other_refusals_stay_errors() {
        use super::Violation;
        use super::{TemplateUpload, upload_answer};
        let problem = r#"{"type":"about:blank","status":400,"detail":"fails","errors":[
            {"path":"VARXR","detail":"external reference \"openEHR-EHR-OBSERVATION.cnf_constituent.v1\" does not resolve"}]}"#;
        assert_eq!(
            upload_answer(answer(
                http::StatusCode::BAD_REQUEST,
                None,
                None,
                problem
            )),
            Ok(TemplateUpload::Invalid(vec![Violation {
                rule: "VARXR".to_owned(),
                detail: "external reference \"openEHR-EHR-OBSERVATION.cnf_constituent.v1\" does not resolve"
                    .to_owned(),
            }]))
        );
        let bare = upload_answer(answer(
            http::StatusCode::BAD_REQUEST,
            None,
            None,
            r#"{"status":400,"detail":"fails"}"#,
        ));
        assert_eq!(
            bare.map_err(|refused| refused.status_code()),
            Err(Some(http::StatusCode::BAD_REQUEST))
        );
        let duplicate = upload_answer(answer(
            http::StatusCode::CONFLICT,
            None,
            None,
            r#"{"status":409,"detail":"the template is already stored"}"#,
        ));
        assert_eq!(
            duplicate.map_err(|refused| refused.status_code()),
            Err(Some(http::StatusCode::CONFLICT))
        );
    }

    #[test]
    fn the_violations_are_the_errors_of_the_problem_in_order() {
        use super::{Violation, violations};
        let body = r#"{"type":"validation-failed","status":422,"detail":"fails","errors":[
            {"path":"VARXR","detail":"external reference \"openEHR-EHR-OBSERVATION.a.v1\" does not resolve"},
            {"path":"VCORMT","detail":"the occurrences do not conform (at id5)"}]}"#;
        assert_eq!(
            violations(body),
            vec![
                Violation {
                    rule: "VARXR".to_owned(),
                    detail: "external reference \"openEHR-EHR-OBSERVATION.a.v1\" does not resolve"
                        .to_owned(),
                },
                Violation {
                    rule: "VCORMT".to_owned(),
                    detail: "the occurrences do not conform (at id5)".to_owned(),
                },
            ]
        );
        assert_eq!(violations(r#"{"status":422,"detail":"fails"}"#), Vec::new());
        assert_eq!(violations("not json"), Vec::new());
    }

    #[test]
    fn a_stored_adl14_upload_names_its_template_id_and_a_refused_one_its_violations() {
        use super::{TemplateUpload, Violation, upload_answer};
        let opt = r#"<?xml version="1.0" encoding="UTF-8"?><template/>"#;
        let stored = answer(
            http::StatusCode::CREATED,
            Some(r#"W/"cnf.minimal v1""#),
            Some("https://cdr.example/rest/openehr/v1/definition/template/adl1.4/cnf.minimal%20v1"),
            opt,
        );
        assert_eq!(
            upload_answer(stored),
            Ok(TemplateUpload::Stored(Some("cnf.minimal v1".to_owned())))
        );
        let by_location = answer(
            http::StatusCode::CREATED,
            None,
            Some("https://cdr.example/rest/openehr/v1/definition/template/adl1.4/cnf.minimal%20v1"),
            opt,
        );
        assert_eq!(
            upload_answer(by_location),
            Ok(TemplateUpload::Stored(Some("cnf.minimal v1".to_owned())))
        );
        let problem = r#"{"type":"about:blank","status":400,"detail":"fails","errors":[
            {"path":"/definition/attributes[1]","detail":"the node id is not in the ontology"}]}"#;
        assert_eq!(
            upload_answer(answer(http::StatusCode::BAD_REQUEST, None, None, problem)),
            Ok(TemplateUpload::Invalid(vec![Violation {
                rule: "/definition/attributes[1]".to_owned(),
                detail: "the node id is not in the ontology".to_owned(),
            }]))
        );
    }
}
