// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `FEEDER_AUDIT` provenance builder for the FHIR connector.
//!
//! The connector as a whole is spec-silent, but this submodule builds RM-typed
//! data: the `FEEDER_AUDIT` / `FEEDER_AUDIT_DETAILS` provenance stamped on the
//! imported COMPOSITION. Its shape is governed by RM common `FEEDER_AUDIT`
//! (`RM/docs/UML/classes/org.openehr.rm.common.feeder_audit.adoc` and
//! `…feeder_audit_details.adoc`) with the semantics at
//! `RM/docs/common/master03-archetyped_package.adoc` §Feeder System Audit; the
//! `DV_IDENTIFIER` shape and its `Id_valid` invariant are RM `data_types`.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 6): FHIR resources are an external standard \
              with no RM type (typed-FHIR evaluation tracked separately)"
)]

use openehr_rm::prelude::{DvDateTime, DvIdentifier, FeederAudit, FeederAuditDetails};
use serde_json::Value;

/// The `system_id` recorded in the built COMPOSITION's `FEEDER_AUDIT`
/// originating-system audit (RM common `FEEDER_AUDIT_DETAILS`), naming the
/// import channel.
pub const ORIGINATING_SYSTEM: &str = "fhir-connector";

/// The resource's logical id (`id`), or a non-empty fallback (`DV_IDENTIFIER`'s
/// `Id_valid` invariant forbids an empty id).
pub fn resource_id(resource: &Value, resource_type: &str) -> String {
    super::mapping::resolve(resource, "id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map_or_else(|| format!("{resource_type}/unknown"), str::to_owned)
}

/// The resource version (`meta.versionId`), if present.
pub fn resource_version(resource: &Value) -> Option<String> {
    super::mapping::resolve(resource, "meta.versionId")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// The FHIR import timestamp for `FEEDER_AUDIT` (ISO 8601, UTC).
#[must_use]
pub fn now_iso() -> String {
    jiff::Timestamp::now().to_string()
}

/// Builds the `FEEDER_AUDIT` (canonical JSON) recording the FHIR import trail.
///
/// Originating system `fhir-connector`, the resource type/id as an
/// originating-system item id, and the resource version + import time on the
/// originating-system audit (RM common `FEEDER_AUDIT_DETAILS`).
pub fn feeder_audit(
    resource_type: &str,
    resource_id: &str,
    version_id: Option<&str>,
    time_iso: &str,
) -> Value {
    let audit = FeederAudit {
        originating_system_item_ids: Some(vec![DvIdentifier {
            issuer: Some("FHIR".to_owned()),
            assigner: None,
            id: resource_id.to_owned(),
            r#type: Some(resource_type.to_owned()),
        }]),
        feeder_system_item_ids: openehr_base::containers::present(Vec::new()),
        original_content: None,
        originating_system_audit: Box::new(FeederAuditDetails {
            system_id: ORIGINATING_SYSTEM.to_owned(),
            location: None,
            subject: None,
            provider: None,
            time: Some(DvDateTime {
                normal_status: None,
                normal_range: None,
                other_reference_ranges: None,
                magnitude_status: None,
                accuracy: None,
                value: time_iso.to_owned(),
            }),
            version_id: version_id.map(ToOwned::to_owned),
            other_details: None,
        }),
        feeder_system_audit: None,
    };
    openehr_its::json::to_canonical_value(&audit)
}

/// Attach a `FEEDER_AUDIT` to a canonical-JSON COMPOSITION object.
pub fn inject_feeder_audit(comp: &mut Value, feeder_audit: Value) {
    if let Value::Object(m) = comp {
        m.insert("feeder_audit".to_owned(), feeder_audit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn feeder_audit_shape() {
        let fa = feeder_audit("Observation", "obs-1", Some("2"), "2026-07-11T00:00:00Z");
        assert_eq!(fa["_type"], json!("FEEDER_AUDIT"));
        assert_eq!(
            fa["originating_system_audit"]["system_id"],
            json!(ORIGINATING_SYSTEM)
        );
        assert_eq!(fa["originating_system_audit"]["version_id"], json!("2"));
        assert_eq!(fa["originating_system_item_ids"][0]["id"], json!("obs-1"));
        assert_eq!(
            fa["originating_system_item_ids"][0]["type"],
            json!("Observation")
        );
    }

    #[test]
    fn resource_id_falls_back_when_absent() {
        assert_eq!(
            resource_id(&json!({}), "Observation"),
            "Observation/unknown"
        );
        assert_eq!(resource_id(&json!({ "id": "bp-1" }), "Observation"), "bp-1");
    }

    #[test]
    fn resource_version_reads_meta() {
        assert_eq!(
            resource_version(&json!({ "meta": { "versionId": "3" } })).as_deref(),
            Some("3")
        );
        assert_eq!(resource_version(&json!({})), None);
    }
}
