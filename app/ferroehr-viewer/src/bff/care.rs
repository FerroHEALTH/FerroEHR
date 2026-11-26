// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Care relationships over the CDR's care relationship resources.
//!
//! The resources sit under the extensions root: a subject's list by its
//! reference or by its `PERSON`, the reverse reading by carer, one
//! relationship and its versions, and the writes. Each is an
//! `under_care_of` `PARTY_RELATIONSHIP` whose `details` hold its
//! `care_role` and `provider`. Every read is decided by the CDR per subject,
//! so the viewer adds no rule of its own. No openEHR specification governs
//! these resources: FerroEHR's own design over its draft access control
//! package.

use crate::bff::grants::{PartyRef, Validity};
use crate::bff::wire::VersionEntry;
use crate::error::ViewerError;
use crate::locale::policy::PolicyMsg;
use crate::locale::tr;

/// A care role: its rubric, its code and its terminology.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Role {
    /// The rubric.
    pub rubric: String,
    /// The code.
    pub code: String,
    /// The terminology id.
    pub terminology: String,
}

/// How a relationship reaches its carer: the professional's own party or a
/// team they are a member of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CarePath {
    /// The target is a `PERSON` or a `ROLE`.
    Direct,
    /// The target is a care team's `GROUP` or an `ORGANISATION` unit.
    Team,
}

/// One care relationship as the pages draw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CareRow {
    /// The version uid (`OBJECT_VERSION_ID`).
    pub version_uid: String,
    /// The relationship's container id.
    pub id: String,
    /// The subject's `PERSON`.
    pub subject: PartyRef,
    /// The carer.
    pub carer: PartyRef,
    /// The care role.
    pub role: Role,
    /// The provider, its issuer and id.
    pub provider: (String, String),
    /// The period of the care.
    pub period: Validity,
    /// The document as the CDR sent it, pretty-printed.
    pub body: String,
}

impl CareRow {
    /// How the relationship reaches its carer.
    #[must_use]
    pub fn path(&self) -> CarePath {
        if matches!(self.carer.rm_type.as_str(), "GROUP" | "ORGANISATION") {
            CarePath::Team
        } else {
            CarePath::Direct
        }
    }
}

/// The span a subject's list reads: the day its relationships are live on,
/// and whether ended ones are listed too.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Span {
    /// `YYYY-MM-DD`, or today when `None`.
    pub at: Option<String>,
    /// Whether ended relationships are listed too.
    pub all: bool,
}

impl Span {
    /// The query a list sends for this span.
    fn query(&self) -> Vec<(&'static str, String)> {
        let mut query = Vec::new();
        if let Some(at) = self.at.as_ref().filter(|at| !at.is_empty()) {
            query.push(("at", at.clone()));
        }
        if self.all {
            query.push(("all", "true".to_owned()));
        }
        query
    }
}

/// `url` with `query` appended, each value percent-encoded.
fn with_query(url: String, query: &[(&str, String)]) -> String {
    if query.is_empty() {
        return url;
    }
    let pairs: Vec<String> = query
        .iter()
        .map(|(key, value)| format!("{key}={}", urlencoding::encode(value)))
        .collect();
    format!("{url}?{}", pairs.join("&"))
}

/// The care relationships of the subject an EHR names
/// (`GET care_relationship?subject_id=&subject_namespace=`).
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)); [`ViewerError::Failed`]
/// when the answer is not a JSON list.
pub async fn fetch_of_subject(
    subject_id: String,
    subject_namespace: String,
    span: Span,
) -> Result<Vec<CareRow>, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let mut query = vec![
        ("subject_id", subject_id),
        ("subject_namespace", subject_namespace),
    ];
    query.extend(span.query());
    let url = with_query(cdr.extensions_v1("care_relationship"), &query);
    parse_list(&cdr.get(&url).accept("application/json").ok().await?.body)
}

/// The care relationships of the subject a `PERSON` is
/// (`GET demographic/person/{id}/care_relationship`).
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)); [`ViewerError::Failed`]
/// when the answer is not a JSON list.
pub async fn fetch_of_person(person: String, span: Span) -> Result<Vec<CareRow>, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = with_query(
        cdr.extensions_v1(&format!(
            "demographic/person/{}/care_relationship",
            urlencoding::encode(&person)
        )),
        &span.query(),
    );
    parse_list(&cdr.get(&url).accept("application/json").ok().await?.body)
}

/// One page of the relationships that reach a carer
/// (`GET care_relationship?carer=`), `fetch` of them from `offset`.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)); [`ViewerError::Failed`]
/// when the answer is not a JSON list.
pub async fn fetch_of_carer(
    carer: String,
    offset: u32,
    fetch: u32,
) -> Result<Vec<CareRow>, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = with_query(
        cdr.extensions_v1("care_relationship"),
        &[
            ("carer", carer),
            ("offset", offset.to_string()),
            ("fetch", fetch.to_string()),
        ],
    );
    parse_list(&cdr.get(&url).accept("application/json").ok().await?.body)
}

/// One care relationship (`GET care_relationship/{uid}`), or `None` once it
/// is deleted (`204`).
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)), the `404` of one the
/// caller may not read included; [`ViewerError::Failed`] when the answer is
/// not a care relationship.
pub async fn fetch_one(uid: String) -> Result<Option<CareRow>, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.extensions_v1(&format!("care_relationship/{}", urlencoding::encode(&uid)));
    let response = cdr.get(&url).accept("application/json").send().await?;
    if response.is(http::StatusCode::NO_CONTENT) {
        return Ok(None);
    }
    let response = response.ok()?;
    let served = response.etag_version_uid();
    let mut row = parse_one(&response.body)?;
    if let Some(served) = served {
        row.version_uid = served;
    }
    Ok(Some(row))
}

/// The versions of a care relationship, newest first, each with its audit
/// details (`GET versioned_care_relationship/{uid}/revision_history`).
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)); [`ViewerError::Failed`]
/// when the history is not valid JSON.
pub async fn fetch_versions(uid: String) -> Result<Vec<VersionEntry>, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.extensions_v1(&format!(
        "versioned_care_relationship/{}/revision_history",
        urlencoding::encode(&crate::uid::container_uid_of(&uid))
    ));
    crate::bff::wire::revision_history(&cdr, &url).await
}

/// Adds `document` as a care relationship of the subject a `PERSON` is
/// (`POST demographic/person/{id}/care_relationship`); the new relationship's
/// container id.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)); the CDR's `400`,
/// `403` and `422` normalize via [`CdrResponse::ok`](crate::cdr::CdrResponse::ok).
pub async fn create(person: String, document: String) -> Result<String, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.extensions_v1(&format!(
        "demographic/person/{}/care_relationship",
        urlencoding::encode(&person)
    ));
    let response = cdr
        .post(&url)
        .accept("application/json")
        .body("application/json", document)
        .ok()
        .await?;
    Ok(crate::uid::container_uid_of(
        &response.etag_version_uid().unwrap_or_default(),
    ))
}

/// Commits `document` as the next version of the relationship `uid`, over
/// the version `replaces` (`PUT care_relationship/{uid}` with `If-Match`).
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)); the CDR's `400`,
/// `403`, `412` and `422` normalize via
/// [`CdrResponse::ok`](crate::cdr::CdrResponse::ok).
pub async fn update(uid: String, replaces: String, document: String) -> Result<(), ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.extensions_v1(&format!(
        "care_relationship/{}",
        urlencoding::encode(&crate::uid::container_uid_of(&uid))
    ));
    let condition = format!("\"{replaces}\"");
    cdr.put(&url)
        .accept("application/json")
        .body("application/json", document)
        .headers(&[(http::header::IF_MATCH.as_str(), condition.as_str())])
        .ok()
        .await?;
    Ok(())
}

/// Deletes the relationship `uid`, written in error, over the version
/// `replaces` (`DELETE care_relationship/{uid}` with `If-Match`).
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)); the CDR's `400`,
/// `403` and `412` normalize via
/// [`CdrResponse::ok`](crate::cdr::CdrResponse::ok).
pub async fn delete(uid: String, replaces: String) -> Result<(), ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.extensions_v1(&format!(
        "care_relationship/{}",
        urlencoding::encode(&crate::uid::container_uid_of(&uid))
    ));
    let condition = format!("\"{replaces}\"");
    cdr.delete(&url)
        .headers(&[(http::header::IF_MATCH.as_str(), condition.as_str())])
        .ok()
        .await?;
    Ok(())
}

/// What a new care relationship states: its subject, carer, role, provider
/// and period.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewCare {
    /// The subject's `PERSON`.
    pub subject: String,
    /// The carer.
    pub carer: PartyRef,
    /// The care role.
    pub role: Role,
    /// The provider, its issuer and id.
    pub provider: (String, String),
    /// The period of the care.
    pub period: Validity,
}

/// An access terminology code as a `DV_CODED_TEXT`.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer writes the canonical-JSON under_care_of PARTY_RELATIONSHIP of the \
              draft access control package"
)]
fn access(code: &str) -> serde_json::Value {
    coded(code, code, "openehr_access")
}

/// A `DV_CODED_TEXT` of `rubric`, `code` and `terminology`.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer writes the canonical-JSON under_care_of PARTY_RELATIONSHIP of the \
              draft access control package"
)]
fn coded(rubric: &str, code: &str, terminology: &str) -> serde_json::Value {
    serde_json::json!({ "_type": "DV_CODED_TEXT", "value": rubric,
        "defining_code": { "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": terminology },
            "code_string": code } })
}

/// A period as a `DV_INTERVAL<DV_DATE>`, an open bound unbounded.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer writes the canonical-JSON under_care_of PARTY_RELATIONSHIP of the \
              draft access control package"
)]
fn interval(period: &Validity) -> serde_json::Value {
    let mut object = serde_json::Map::new();
    object.insert("_type".to_owned(), "DV_INTERVAL".into());
    for (key, bound) in [("lower", &period.lower), ("upper", &period.upper)] {
        if let Some(bound) = bound {
            object.insert(
                key.to_owned(),
                serde_json::json!({ "_type": "DV_DATE", "value": bound }),
            );
        }
    }
    object.insert("lower_unbounded".to_owned(), period.lower.is_none().into());
    object.insert("upper_unbounded".to_owned(), period.upper.is_none().into());
    serde_json::Value::Object(object)
}

/// The `under_care_of` `PARTY_RELATIONSHIP` a new care relationship is.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer writes the canonical-JSON under_care_of PARTY_RELATIONSHIP of the \
              draft access control package"
)]
#[must_use]
pub fn new_document(care: &NewCare) -> String {
    let reference = |rm_type: &str, id: &str| {
        serde_json::json!({ "_type": "PARTY_REF", "namespace": "demographic", "type": rm_type,
            "id": { "_type": "HIER_OBJECT_ID", "value": id } })
    };
    serde_json::json!({
        "_type": "PARTY_RELATIONSHIP",
        "name": access("under_care_of"),
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.under_care_of.v2",
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "details" },
            "items": [
                { "_type": "ELEMENT", "archetype_node_id": "at0002", "name": access("care_role"),
                  "value": coded(&care.role.rubric, &care.role.code, &care.role.terminology) },
                { "_type": "ELEMENT", "archetype_node_id": "at0003", "name": access("provider"),
                  "value": { "_type": "DV_IDENTIFIER", "issuer": care.provider.0,
                             "id": care.provider.1, "type": "ORGANISATION" } }
            ] },
        "time_validity": interval(&care.period),
        "source": reference("PERSON", &care.subject),
        "target": reference(&care.carer.rm_type, &care.carer.id),
    })
    .to_string()
}

/// The next version of `body`, its care role and period replaced and every
/// other attribute kept as the CDR sent it.
///
/// # Errors
/// [`ViewerError::Failed`] when `body` is not a JSON object.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer edits the canonical-JSON under_care_of PARTY_RELATIONSHIP of the draft \
              access control package"
)]
pub fn changed_document(body: &str, role: &Role, period: &Validity) -> Result<String, ViewerError> {
    let mut value: serde_json::Value = serde_json::from_str(body)
        .map_err(|error| ViewerError::failed(tr(PolicyMsg::CareJson), error))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| ViewerError::Internal(tr(PolicyMsg::CareJson)))?;
    object.remove("uid");
    object.insert("time_validity".to_owned(), interval(period));
    if let Some(items) = value
        .pointer_mut("/details/items")
        .and_then(serde_json::Value::as_array_mut)
    {
        for item in items {
            let named_role = item
                .pointer("/name/defining_code/code_string")
                .and_then(serde_json::Value::as_str)
                == Some("care_role");
            if let Some(element) = item.as_object_mut().filter(|_| named_role) {
                element.insert(
                    "value".to_owned(),
                    coded(&role.rubric, &role.code, &role.terminology),
                );
            }
        }
    }
    Ok(value.to_string())
}

/// The relationships a list body holds.
///
/// # Errors
/// [`ViewerError::Failed`] when the body is not a JSON list.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the canonical-JSON under_care_of PARTY_RELATIONSHIPs of the draft \
              access control package"
)]
pub fn parse_list(body: &str) -> Result<Vec<CareRow>, ViewerError> {
    let items: Vec<serde_json::Value> = serde_json::from_str(body)
        .map_err(|error| ViewerError::failed(tr(PolicyMsg::CareJson), error))?;
    Ok(items.iter().map(care_row).collect())
}

/// The relationship one body is.
///
/// # Errors
/// [`ViewerError::Failed`] when the body is not JSON.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the canonical-JSON under_care_of PARTY_RELATIONSHIP of the draft \
              access control package"
)]
pub fn parse_one(body: &str) -> Result<CareRow, ViewerError> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|error| ViewerError::failed(tr(PolicyMsg::CareJson), error))?;
    Ok(care_row(&value))
}

/// One relationship read into a row.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the canonical-JSON under_care_of PARTY_RELATIONSHIP of the draft \
              access control package"
)]
fn care_row(value: &serde_json::Value) -> CareRow {
    let text = |value: &serde_json::Value, pointer: &str| {
        value
            .pointer(pointer)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let party = |key: &str| PartyRef {
        rm_type: text(value, &format!("/{key}/type")),
        id: text(value, &format!("/{key}/id/value")),
    };
    let detail = |code: &str| {
        value
            .pointer("/details/items")
            .and_then(serde_json::Value::as_array)
            .and_then(|items| {
                items.iter().find(|item| {
                    item.pointer("/name/defining_code/code_string")
                        .and_then(serde_json::Value::as_str)
                        == Some(code)
                })
            })
            .and_then(|item| item.get("value"))
            .cloned()
            .unwrap_or_default()
    };
    let role = detail("care_role");
    let provider = detail("provider");
    let bound = |key: &str| {
        Some(text(value, &format!("/time_validity/{key}/value"))).filter(|bound| !bound.is_empty())
    };
    let version_uid = text(value, "/uid/value");
    CareRow {
        id: crate::uid::container_uid_of(&version_uid),
        version_uid,
        subject: party("source"),
        carer: party("target"),
        role: Role {
            rubric: text(&role, "/value"),
            code: text(&role, "/defining_code/code_string"),
            terminology: text(&role, "/defining_code/terminology_id/value"),
        },
        provider: (text(&provider, "/issuer"), text(&provider, "/id")),
        period: Validity {
            lower: bound("lower"),
            upper: bound("upper"),
        },
        body: serde_json::to_string_pretty(value).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::{CarePath, NewCare, Role, Span, changed_document, new_document, parse_list};
    use crate::bff::grants::{PartyRef, Validity};

    fn care() -> NewCare {
        NewCare {
            subject: "p1".to_owned(),
            carer: PartyRef {
                rm_type: "PERSON".to_owned(),
                id: "d1".to_owned(),
            },
            role: Role {
                rubric: "attender".to_owned(),
                code: "ATND".to_owned(),
                terminology: "HL7_ParticipationType".to_owned(),
            },
            provider: ("urn:oid:2.999.7.9".to_owned(), "dev".to_owned()),
            period: Validity {
                lower: Some("2026-01-01".to_owned()),
                upper: None,
            },
        }
    }

    #[test]
    fn a_new_relationship_reads_back_as_written() {
        let body = format!("[{}]", new_document(&care()));
        let rows = parse_list(&body).expect("a list");
        let row = rows.first().expect("one row");
        assert_eq!("d1", row.carer.id);
        assert_eq!("p1", row.subject.id);
        assert_eq!("ATND", row.role.code);
        assert_eq!("HL7_ParticipationType", row.role.terminology);
        assert_eq!(
            ("urn:oid:2.999.7.9".to_owned(), "dev".to_owned()),
            row.provider
        );
        assert_eq!(Some("2026-01-01".to_owned()), row.period.lower);
        assert_eq!(None, row.period.upper);
        assert_eq!(CarePath::Direct, row.path());
    }

    #[test]
    fn a_change_replaces_the_role_and_the_period_and_keeps_the_rest() {
        let role = Role {
            rubric: "responsible".to_owned(),
            code: "RESP".to_owned(),
            terminology: "HL7_ParticipationType".to_owned(),
        };
        let ended = Validity {
            lower: Some("2026-01-01".to_owned()),
            upper: Some("2026-11-26".to_owned()),
        };
        let changed = changed_document(&new_document(&care()), &role, &ended).expect("a change");
        let rows = parse_list(&format!("[{changed}]")).expect("a list");
        let row = rows.first().expect("one row");
        assert_eq!("RESP", row.role.code);
        assert_eq!(Some("2026-11-26".to_owned()), row.period.upper);
        assert_eq!("d1", row.carer.id);
        assert_eq!("dev", row.provider.1);
    }

    #[test]
    fn a_team_carer_reaches_by_the_team_path() {
        let mut team = care();
        team.carer.rm_type = "GROUP".to_owned();
        let rows = parse_list(&format!("[{}]", new_document(&team))).expect("a list");
        assert_eq!(Some(CarePath::Team), rows.first().map(super::CareRow::path));
    }

    #[test]
    fn a_span_asks_for_ended_relationships_and_a_day() {
        let span = Span {
            at: Some("2026-11-26".to_owned()),
            all: true,
        };
        assert_eq!(2, span.query().len());
        assert_eq!(0, Span::default().query().len());
    }
}
