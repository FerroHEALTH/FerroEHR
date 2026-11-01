// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `EXTRACT_SPEC.extract_type` check of an Extract request: the TERM
//! `extract_content_type` vocabulary or an RM-named content type.

use openehr_rm::v1_2::ehr_extract::common::extract_spec::ExtractSpec;
use serde_json::Value;

use crate::service::error::ServiceError;

/// The RM-named string tokens `EXTRACT_SPEC.extract_type` may carry, beside
/// the TERM `extract_content_type` vocabulary codes.
///
/// The first five are the ones the RM names outright — RM `ehr_extract`
/// `master04-common_package.adoc` §Content Criteria Specification:
/// "`_extract_type_`: what kind of Extract this is, e.g. `|openehr-ehr|`,
/// `|openehr-demographic|`, `|openehr-synchronisation|`, `|openehr-generic|`,
/// `|generic-emr|`, etc". TERM `SupportTerminology/master03-terminology.adoc`
/// §Vocabularies additionally binds the attribute to the
/// `extract_content_type` group (concepts 803 "openEHR EHR" … 808 "other"),
/// validated against the bundle in [`validate_extract_type`] — the two value
/// spaces are never reconciled upstream, so both are accepted.
///
/// NOTE: the RM's "etc" (`master04-common_package.adoc` §Content Criteria
/// Specification) makes its list illustrative, and this service still validates,
/// an unrecognized type silently exporting an openEHR-EHR extract being a
/// payload that misdescribes itself; the accepted set is a superset of every
/// code the RM names and of the whole TERM group.
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
