// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `EXTRACT_SPEC.extract_type` check of an Extract request: the TERM
//! `extract_content_type` vocabulary or an RM-named content type.

use openehr_rm::v1_2::ehr_extract::common::extract_spec::ExtractSpec;
use serde_json::Value;

use crate::error::ServiceError;

/// The RM-named string tokens `EXTRACT_SPEC.extract_type` may carry, beside
/// the TERM `extract_content_type` codes.
///
/// RM `ehr_extract` `master04-common_package.adoc` §Content Criteria
/// Specification names these tokens with an "etc", and TERM
/// `SupportTerminology/master03-terminology.adoc` §Vocabularies binds the
/// attribute to the `extract_content_type` group, checked in
/// [`validate_extract_type`]. Both value spaces are accepted, and nothing
/// else, so an Extract never misdescribes itself.
const EXTRACT_CONTENT_TYPES: [&str; 6] = [
    "openehr-ehr",
    "openehr-demographic",
    "openehr-synchronisation",
    "openehr-generic",
    "generic-emr",
    "other",
];

/// `EXTRACT_SPEC.extract_type` must be coded from the TERM
/// `extract_content_type` vocabulary (openEHR terminology, concepts 803–808 —
/// TERM `SupportTerminology/master03-terminology.adoc` §Vocabularies) or from
/// the RM-named tokens in [`EXTRACT_CONTENT_TYPES`].
pub(super) fn validate_extract_type(spec: &ExtractSpec) -> Result<(), ServiceError> {
    let value = openehr_its::json::to_canonical_value(&spec.extract_type);
    let code = value
        .pointer("/defining_code/code_string")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let terminology = value
        .pointer("/defining_code/terminology_id/value")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if terminology == "openehr"
        && openehr_term::bundle::openehr().is_valid_code("extract_content_type", code)
    {
        return Ok(());
    }
    if EXTRACT_CONTENT_TYPES.contains(&code) {
        return Ok(());
    }
    Err(ServiceError::precondition(format!(
        "EXTRACT_SPEC.extract_type {code:?} is neither a member of the openEHR \
         extract_content_type vocabulary nor an RM-named extract content type \
         ({})",
        EXTRACT_CONTENT_TYPES.join(" | ")
    )))
}
