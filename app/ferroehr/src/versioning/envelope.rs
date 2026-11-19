// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The CONTRIBUTION commit body as the service reads it: once, from its text,
//! with each member's `data` kept as the text the typed reader takes (#6038).
//!
//! The envelope is the generated `NewContribution` of its API group, read
//! straight from the request text. A body or a member the text readers
//! refuse goes the way every body went before: parsed into a JSON value and
//! read from it, so each refusal keeps its status, its text and its JSON
//! path, and a body only the value reading admits is still committed.
//!
//! No openEHR spec governs how a server reads a request: our own design. The
//! refusals stay the ones ITS-REST `responses/400_CONTRIBUTION.yaml` and
//! overview `Requests_and_responses.md` §HTTP status codes assign.

use std::borrow::Cow;

use openehr_base::prelude::HierObjectId;
use openehr_its::rest::generated::common::{UpdateAudit, UpdateVersion};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use serde_json::value::RawValue;

use crate::error::ServiceError;
use crate::versioning::contribution::decode_new_contribution;

/// The canonical-JSON text of a CONTRIBUTION commit body.
#[derive(Debug, Clone)]
pub struct ContributionText(String);

impl ContributionText {
    fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for ContributionText {
    fn from(text: String) -> Self {
        Self(text)
    }
}

impl From<Value> for ContributionText {
    /// The text of a body its caller holds as a JSON value, rendered once.
    fn from(body: Value) -> Self {
        crate::telemetry::conversion::record("value_to_text");
        Self(body.to_string())
    }
}

/// The `data` of one CONTRIBUTION member, as it was read.
#[derive(Debug)]
pub(crate) enum MemberData {
    /// The member's canonical-JSON text, which the typed reader takes.
    Text(Box<RawValue>),
    /// The member as a JSON value, the reading of a body the text readers
    /// refused.
    Value(Value),
}

impl From<Box<RawValue>> for MemberData {
    fn from(text: Box<RawValue>) -> Self {
        Self::Text(text)
    }
}

impl From<Value> for MemberData {
    fn from(value: Value) -> Self {
        Self::Value(value)
    }
}

impl MemberData {
    /// The member's canonical-JSON text, when it is held as text.
    pub(crate) fn text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text.get()),
            Self::Value(_) => None,
        }
    }

    /// The member as a JSON value, parsed from its text where it is held as
    /// text.
    ///
    /// # Errors
    /// [`ServiceError::BadRequest`] when the text does not parse, which the
    /// envelope's reader has already ruled out.
    pub(crate) fn into_value(self) -> Result<Value, ServiceError> {
        match self {
            Self::Value(value) => Ok(value),
            Self::Text(text) => {
                crate::telemetry::conversion::record("text_to_value");
                serde_json::from_str(text.get()).map_err(|e| {
                    ServiceError::bad_request(format!("invalid JSON in a version's data: {e}"), e)
                })
            }
        }
    }
}

/// A member's `data` with what the plan reads off its root before any member
/// is decoded.
#[derive(Debug)]
pub(crate) struct Member {
    pub(crate) data: MemberData,
    /// The root's `_type`, empty when it states none or one that is not a
    /// string.
    pub(crate) rm_type: String,
    /// The root's `is_modifiable`, which an `EHR_STATUS` carries.
    pub(crate) is_modifiable: Option<bool>,
}

/// The two root members the plan reads, taken off the text in one pass that
/// skips everything else.
#[derive(Deserialize)]
struct Root<'a> {
    #[serde(rename = "_type", borrow, default)]
    rm_type: Option<Cow<'a, str>>,
    #[serde(default)]
    is_modifiable: Option<bool>,
}

impl Member {
    /// Reads the root of `data`. A text whose root this cannot read as the
    /// value reading would (a repeated or mistyped member) is turned into its
    /// value and read from that.
    ///
    /// # Errors
    /// As [`MemberData::into_value`].
    pub(crate) fn read(data: MemberData) -> Result<Self, ServiceError> {
        // NOTE: the derived reader would also take an array as the members in order.
        if let Some(text) = data.text()
            && text.trim_start().starts_with('{')
            && let Ok(root) = serde_json::from_str::<Root<'_>>(text)
        {
            return Ok(Self {
                rm_type: root.rm_type.map(Cow::into_owned).unwrap_or_default(),
                is_modifiable: root.is_modifiable,
                data,
            });
        }
        let value = data.into_value()?;
        Ok(Self {
            rm_type: value
                .get("_type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            is_modifiable: value.pointer("/is_modifiable").and_then(Value::as_bool),
            data: MemberData::Value(value),
        })
    }
}

/// The members of a `NewContribution` the commit reads, whichever API group's
/// generated envelope carried them.
#[derive(Debug)]
pub(crate) struct Parts {
    pub(crate) uid: Option<HierObjectId>,
    pub(crate) versions: Vec<UpdateVersion<MemberData>>,
    pub(crate) audit: UpdateAudit,
}

impl Parts {
    /// The parts of an envelope whose members hold their `data` as `D`.
    pub(crate) fn new<D: Into<MemberData>>(
        uid: Option<HierObjectId>,
        versions: Vec<UpdateVersion<D>>,
        audit: UpdateAudit,
    ) -> Self {
        let versions = versions
            .into_iter()
            .map(|version| UpdateVersion {
                preceding_version_uid: version.preceding_version_uid,
                signature: version.signature,
                lifecycle_state: version.lifecycle_state,
                attestations: version.attestations,
                data: version.data.map(Into::into),
                commit_audit: version.commit_audit,
                additional_properties: version.additional_properties,
            })
            .collect();
        Self {
            uid,
            versions,
            audit,
        }
    }
}

/// A generated `NewContribution` whose members hold their `data` as `D`.
pub(crate) trait Envelope<D> {
    /// The parts the commit reads.
    fn into_parts(self) -> Parts;
}

// NOTE: ITS-REST `schemas/ehr/NewContribution.yaml` and its demographic twin leave
// `additionalProperties` open (`ItemTag.yaml` closes its object with `false`), so an undeclared member is ignored.
impl<D: Into<MemberData>> Envelope<D> for openehr_its::rest::generated::ehr::NewContribution<D> {
    fn into_parts(self) -> Parts {
        Parts::new(self.uid, self.versions, self.audit)
    }
}

impl<D: Into<MemberData>> Envelope<D>
    for openehr_its::rest::generated::demographic::NewContribution<D>
{
    fn into_parts(self) -> Parts {
        Parts::new(self.uid, self.versions, self.audit)
    }
}

/// Reads a CONTRIBUTION commit body through the generated `NewContribution`
/// of its API group: `F` from the text with each member's `data` left as
/// text, or, for a body that reading refuses, `S` from the body's JSON value
/// ([`decode_new_contribution`]), which words the refusal.
///
/// # Errors
/// [`ServiceError::BadRequest`] for text that is not JSON, and the refusal of
/// [`decode_new_contribution`].
pub(crate) fn read<F, S>(body: &ContributionText) -> Result<Parts, ServiceError>
where
    F: DeserializeOwned + Envelope<Box<RawValue>>,
    S: DeserializeOwned + Envelope<Value>,
{
    crate::telemetry::conversion::record("text_to_dto");
    if let Ok(envelope) = serde_json::from_str::<F>(body.as_str()) {
        return Ok(envelope.into_parts());
    }
    crate::telemetry::conversion::record("text_to_value");
    let value: Value = serde_json::from_str(body.as_str())
        .map_err(|e| ServiceError::bad_request(format!("invalid JSON body: {e}"), e))?;
    Ok(decode_new_contribution::<S>(&value)?.into_parts())
}

#[cfg(test)]
mod tests {
    use openehr_its::rest::generated::ehr::NewContribution;
    use serde_json::json;

    use super::*;

    fn read_ehr(text: &str) -> Result<Parts, ServiceError> {
        read::<NewContribution<Box<RawValue>>, NewContribution<Value>>(&ContributionText::from(
            text.to_owned(),
        ))
    }

    const AUDIT: &str = r#"{"_type":"UPDATE_AUDIT","change_type":{"value":"creation","defining_code":{"terminology_id":{"value":"openehr"},"code_string":"249"}},"committer":{"_type":"PARTY_IDENTIFIED","name":"Dr Jones"}}"#;
    const LIFECYCLE: &str = r#"{"value":"complete","defining_code":{"terminology_id":{"value":"openehr"},"code_string":"532"}}"#;

    /// A one-member body whose member's `data` is `data`, with `extra`
    /// written into the envelope after its `versions`.
    fn body(data: &str, extra: &str) -> String {
        format!(
            r#"{{"_type":"CONTRIBUTION","versions":[{{"lifecycle_state":{LIFECYCLE},"commit_audit":{AUDIT},"data":{data}}}]{extra},"audit":{AUDIT}}}"#
        )
    }

    fn only_member(parts: Parts) -> Option<MemberData> {
        assert_eq!(parts.versions.len(), 1);
        parts.versions.into_iter().next().and_then(|v| v.data)
    }

    /// The text reading keeps a member's `data` as the text it arrived as,
    /// byte for byte, and an absent or `null` one as none.
    #[test]
    fn a_member_keeps_its_text() {
        let data = r#"{ "_type" : "COMPOSITION", "n": 1.50 }"#;
        let member = only_member(read_ehr(&body(data, "")).expect("the body reads"));
        assert_eq!(member.as_ref().and_then(MemberData::text), Some(data));
        let deleted = only_member(read_ehr(&body("null", "")).expect("the body reads"));
        assert!(deleted.is_none());
    }

    /// A body the text reading refuses and the value reading admits, here an
    /// envelope member written twice, is read from its value as it was before
    /// the text reading existed.
    #[test]
    fn a_body_only_the_value_reading_admits_is_read_from_its_value() {
        let twice = format!(r#","audit":{AUDIT}"#);
        let member = only_member(
            read_ehr(&body(r#"{"_type":"COMPOSITION"}"#, &twice)).expect("the value reading"),
        );
        assert!(matches!(member, Some(MemberData::Value(_))));
    }

    /// A body neither reading admits is refused in the words of the value
    /// reading.
    #[test]
    fn a_refused_body_is_refused_in_the_value_readings_words() {
        let text = r#"{"versions":[],"audit":{"_type":"UPDATE_AUDIT"}}"#;
        let value: Value = serde_json::from_str(text).expect("JSON");
        let expected = match decode_new_contribution::<NewContribution<Value>>(&value) {
            Err(ServiceError::BadRequest(e)) => e.message,
            other => panic!("expected the 400 shape refusal, got {other:?}"),
        };
        match read_ehr(text) {
            Err(ServiceError::BadRequest(e)) => assert_eq!(e.message, expected),
            other => panic!("expected the 400 shape refusal, got {other:?}"),
        }
        assert!(matches!(
            read_ehr("{ not json"),
            Err(ServiceError::BadRequest(_))
        ));
    }

    /// The root read off a member's text is the root its value holds: the
    /// `_type` and `is_modifiable`, the last occurrence of a repeated member,
    /// and nothing for a member of another JSON type.
    #[test]
    fn the_root_of_a_text_is_the_root_of_its_value() {
        let texts = [
            r#"{"_type":"EHR_STATUS","is_modifiable":true,"other":{"_type":"X","is_modifiable":false}}"#,
            r#"{"is_modifiable":false,"deep":[1,{"a":null}],"_type":"EHR_STATUS"}"#,
            r#"{"_type":"COMPOSITION"}"#,
            r#"{"_type":"COMPOSITION","_type":"FOLDER"}"#,
            r#"{"_type":7,"is_modifiable":"yes"}"#,
            r#"{"_type":null,"is_modifiable":null}"#,
            r"{}",
            r"[1,2]",
            r#"["COMPOSITION",true]"#,
            r#""text""#,
        ];
        for text in texts {
            let value: Value = serde_json::from_str(text).expect("JSON");
            let raw: Box<RawValue> = serde_json::from_str(text).expect("JSON");
            let member = Member::read(MemberData::Text(raw)).expect("the root reads");
            assert_eq!(
                member.rm_type,
                value
                    .get("_type")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                "on {text}"
            );
            assert_eq!(
                member.is_modifiable,
                value.pointer("/is_modifiable").and_then(Value::as_bool),
                "on {text}"
            );
        }
        let held = Member::read(MemberData::Value(json!({ "_type": "FOLDER" }))).expect("reads");
        assert_eq!(held.rm_type, "FOLDER");
    }
}
