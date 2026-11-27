// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The controller's access policy over the CDR's admin extension
//! (`{ext}/admin/access_policy`, `/history`), and the reading of an
//! `ACCESS_POLICY` document into the attributes a page draws.
//!
//! The policy is FerroEHR's draft access control package, served as
//! canonical JSON: the newest version with its `OBJECT_VERSION_ID` as the
//! `ETag`, a `404` while none is committed, and a write naming the version
//! it replaces in `If-Match` (`If-None-Match: *` for the first). No openEHR
//! specification governs the viewer's reading of it: our own design.

use crate::bff::wire::{VersionEntry, committer_label};
use crate::error::ViewerError;
use crate::locale::policy::PolicyMsg;
use crate::locale::tr;

/// The policy in force: its version and its document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyState {
    /// The `OBJECT_VERSION_ID` the `ETag` names.
    pub version_uid: String,
    /// The `ACCESS_POLICY` document as the CDR sent it.
    pub body: String,
}

/// One version of the policy, as the history lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyVersion {
    /// Its commit facts.
    pub entry: VersionEntry,
    /// Its `ACCESS_POLICY` document, pretty-printed.
    pub body: String,
}

/// The policy in force (`GET admin/access_policy`), or `None` while none is
/// committed (`404`).
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)); any other non-2xx
/// answer normalizes via [`CdrResponse::ok`](crate::cdr::CdrResponse::ok).
pub async fn fetch_policy() -> Result<Option<PolicyState>, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.extensions_v1("admin/access_policy");
    let response = cdr.get(&url).accept("application/json").send().await?;
    if response.is(http::StatusCode::NOT_FOUND) {
        return Ok(None);
    }
    let response = response.ok()?;
    Ok(Some(PolicyState {
        version_uid: response.etag_version_uid().unwrap_or_default(),
        body: crate::bff::pretty_body(&response.body, crate::format::ReprFormat::CanonicalJson),
    }))
}

/// Every version of the policy, newest first
/// (`GET admin/access_policy/history`).
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)); [`ViewerError::Failed`]
/// when the history is not a JSON list.
pub async fn fetch_history() -> Result<Vec<PolicyVersion>, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.extensions_v1("admin/access_policy/history");
    let body = cdr.get(&url).accept("application/json").ok().await?.body;
    parse_history(&body)
}

/// Commits `document` as the next version of the policy
/// (`PUT admin/access_policy`), over the version `replaces` names, or as the
/// first when it is `None`; the new version's `OBJECT_VERSION_ID`.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)); the CDR's `400`,
/// `403`, `412` and `422` normalize via
/// [`CdrResponse::ok`](crate::cdr::CdrResponse::ok).
pub async fn put_policy(replaces: Option<String>, document: String) -> Result<String, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.extensions_v1("admin/access_policy");
    let condition = match &replaces {
        Some(uid) => (http::header::IF_MATCH.as_str(), format!("\"{uid}\"")),
        None => (http::header::IF_NONE_MATCH.as_str(), "*".to_owned()),
    };
    let response = cdr
        .put(&url)
        .accept("application/json")
        .body("application/json", document)
        .headers(&[(condition.0, condition.1.as_str())])
        .ok()
        .await?;
    Ok(response.etag_version_uid().unwrap_or_default())
}

/// The versions a history body lists, in its order (newest first).
///
/// # Errors
/// [`ViewerError::Failed`] when the body is not a JSON list.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the CDR's canonical-JSON ORIGINAL_VERSIONs of the draft access \
              control package, which no openehr-* crate generates"
)]
pub fn parse_history(body: &str) -> Result<Vec<PolicyVersion>, ViewerError> {
    use serde_json::Value;
    let items: Vec<Value> = serde_json::from_str(body)
        .map_err(|error| ViewerError::failed(tr(PolicyMsg::HistoryJson), error))?;
    Ok(items
        .iter()
        .map(|item| {
            let text = |pointer: &str| {
                item.pointer(pointer)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            };
            PolicyVersion {
                entry: VersionEntry {
                    version_id: text("/uid/value"),
                    committed: text("/commit_audit/time_committed/value"),
                    change_type: text("/commit_audit/change_type/value"),
                    committer: committer_label(|key| {
                        Some(text(&format!("/commit_audit/committer/{key}")))
                    }),
                },
                body: item
                    .get("data")
                    .and_then(|data| serde_json::to_string_pretty(data).ok())
                    .unwrap_or_default(),
            }
        })
        .collect())
}

/// A value of the policy as a page draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shown {
    /// Text, a date, a duration or an identifier.
    Text(String),
    /// A coded value: its rubric, terminology and code.
    Coded {
        /// The rubric.
        rubric: String,
        /// The terminology id.
        terminology: String,
        /// The code.
        code: String,
    },
    /// A flag.
    Flag(bool),
    /// A list.
    List(Vec<Shown>),
    /// An object: its name, when it has one, its RM type and its attributes.
    Fields {
        /// `name.value`, when the object is `LOCATABLE`.
        name: Option<String>,
        /// Its `_type`.
        rm_type: String,
        /// Its other attributes, in document order.
        fields: Vec<(String, Shown)>,
    },
}

/// The attributes a `LOCATABLE` carries that the page does not draw as a
/// field: its name is the heading, the rest are its archetyping.
const ENVELOPE: [&str; 6] = [
    "_type",
    "name",
    "archetype_node_id",
    "archetype_details",
    "uid",
    "feeder_audit",
];

impl Shown {
    /// The shown form of a canonical-JSON value.
    #[expect(
        clippy::disallowed_types,
        reason = "the viewer reads the CDR's canonical-JSON ACCESS_POLICY of the draft access \
                  control package, which no openehr-* crate generates"
    )]
    pub fn of(value: &serde_json::Value) -> Self {
        use serde_json::Value;
        let text = |value: &Value, pointer: &str| {
            value
                .pointer(pointer)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        match value {
            Value::Bool(yes) => Self::Flag(*yes),
            Value::String(words) => Self::Text(words.clone()),
            Value::Number(number) => Self::Text(number.to_string()),
            Value::Null => Self::Text(String::new()),
            Value::Array(items) => Self::List(items.iter().map(Self::of).collect()),
            Value::Object(object) => {
                let rm_type = text(value, "/_type");
                match rm_type.as_str() {
                    "DV_CODED_TEXT" => Self::Coded {
                        rubric: text(value, "/value"),
                        terminology: text(value, "/defining_code/terminology_id/value"),
                        code: text(value, "/defining_code/code_string"),
                    },
                    "CODE_PHRASE" => Self::Coded {
                        rubric: String::new(),
                        terminology: text(value, "/terminology_id/value"),
                        code: text(value, "/code_string"),
                    },
                    "DV_TEXT" | "DV_DURATION" | "DV_DATE_TIME" | "DV_DATE" | "DV_TIME"
                    | "DV_EHR_URI" | "DV_URI" | "ARCHETYPE_ID" | "TERMINOLOGY_ID"
                    | "HIER_OBJECT_ID" | "GENERIC_ID" | "OBJECT_VERSION_ID" | "TEMPLATE_ID" => {
                        Self::Text(text(value, "/value"))
                    }
                    "PARTY_REF" | "OBJECT_REF" | "LOCATABLE_REF" => Self::Text(format!(
                        "{} {}",
                        text(value, "/type"),
                        text(value, "/id/value")
                    )),
                    "DV_IDENTIFIER" => {
                        let issuer = text(value, "/issuer");
                        let id = text(value, "/id");
                        Self::Text(if issuer.is_empty() {
                            id
                        } else {
                            format!("{issuer} · {id}")
                        })
                    }
                    "DV_INTERVAL" => {
                        let end = |key: &str| {
                            value
                                .get(key)
                                .map(|bound| text(bound, "/value"))
                                .filter(|bound| !bound.is_empty())
                                .unwrap_or_else(|| "…".to_owned())
                        };
                        Self::Text(format!("{} – {}", end("lower"), end("upper")))
                    }
                    _ => Self::Fields {
                        name: value
                            .pointer("/name/value")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        rm_type,
                        fields: object
                            .iter()
                            .filter(|(key, _)| !ENVELOPE.contains(&key.as_str()))
                            .map(|(key, child)| (key.clone(), Self::of(child)))
                            .collect(),
                    },
                }
            }
        }
    }
}

/// An `ACCESS_POLICY` read into what a page draws: its name and each
/// attribute it holds, in document order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PolicyView {
    /// `name.value`.
    pub name: String,
    /// Every attribute but the envelope, by its name.
    pub attributes: Vec<(String, Shown)>,
}

impl PolicyView {
    /// The attribute named `key`, when the policy holds it.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Shown> {
        self.attributes
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, shown)| shown)
    }
}

/// Reads an `ACCESS_POLICY` document.
///
/// # Errors
/// [`ViewerError::Failed`] when the document is not a JSON object.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the CDR's canonical-JSON ACCESS_POLICY of the draft access control \
              package, which no openehr-* crate generates"
)]
pub fn read_policy(body: &str) -> Result<PolicyView, ViewerError> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|error| ViewerError::failed(tr(PolicyMsg::PolicyJson), error))?;
    match Shown::of(&value) {
        Shown::Fields { name, fields, .. } => Ok(PolicyView {
            name: name.unwrap_or_default(),
            attributes: fields,
        }),
        _ => Err(ViewerError::Internal(tr(PolicyMsg::PolicyJson))),
    }
}

#[cfg(test)]
mod tests {
    use super::{Shown, parse_history, read_policy};

    const POLICY: &str = r#"{
        "_type": "ACCESS_POLICY",
        "name": { "_type": "DV_TEXT", "value": "Hospital access policy" },
        "archetype_node_id": "openEHR-EHR-ACCESS_POLICY.controller.v1",
        "controller": { "_type": "PARTY_REF", "namespace": "demographic", "type": "ORGANISATION",
            "id": { "_type": "HIER_OBJECT_ID", "value": "2c3d4e5f" } },
        "care_relationship_required": true,
        "relationship_grace": { "_type": "DV_DURATION", "value": "P7D" },
        "concealed_confidentialities": [{ "_type": "DV_CODED_TEXT", "value": "very restricted",
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "HL7_Confidentiality" },
                "code_string": "V" } }],
        "purpose_required": false
    }"#;

    #[test]
    fn a_policy_reads_its_name_and_each_attribute_in_order() {
        let view = read_policy(POLICY).expect("a policy");
        assert_eq!("Hospital access policy", view.name);
        let keys: Vec<&str> = view
            .attributes
            .iter()
            .map(|(key, _)| key.as_str())
            .collect();
        assert_eq!(
            vec![
                "controller",
                "care_relationship_required",
                "relationship_grace",
                "concealed_confidentialities",
                "purpose_required"
            ],
            keys
        );
        assert_eq!(
            Some(&Shown::Text("ORGANISATION 2c3d4e5f".to_owned())),
            view.get("controller")
        );
        assert_eq!(
            Some(&Shown::Text("P7D".to_owned())),
            view.get("relationship_grace")
        );
        assert_eq!(
            Some(&Shown::List(vec![Shown::Coded {
                rubric: "very restricted".to_owned(),
                terminology: "HL7_Confidentiality".to_owned(),
                code: "V".to_owned(),
            }])),
            view.get("concealed_confidentialities")
        );
    }

    #[test]
    fn a_document_that_is_no_object_is_refused() {
        assert!(read_policy("[]").is_err());
        assert!(read_policy("not json").is_err());
    }

    #[test]
    fn the_history_reads_each_version_with_its_audit() {
        let versions = parse_history(
            r#"[{"_type":"ORIGINAL_VERSION","uid":{"value":"a::ferroehr.local::2"},
                 "commit_audit":{"committer":{"_type":"PARTY_IDENTIFIED","name":"Admin"},
                   "time_committed":{"value":"2026-10-01T09:00:02Z"},
                   "change_type":{"value":"modification"}},
                 "data":{"_type":"ACCESS_POLICY"}}]"#,
        )
        .expect("a history");
        assert_eq!(1, versions.len());
        let first = versions.first().expect("one version");
        assert_eq!("a::ferroehr.local::2", first.entry.version_id);
        assert_eq!("Admin", first.entry.committer);
        assert_eq!("modification", first.entry.change_type);
        assert!(first.body.contains("ACCESS_POLICY"));
    }
}
