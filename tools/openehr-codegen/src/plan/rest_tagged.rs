// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ITS-REST payload members whose value must carry its `_type`, and the
//! field lines `emit-rest` writes for them.
//!
//! The codegen bundle `emit-rest` reads types such a member with the RM schema
//! (`DvText`, whose `_type` has a `default`), so the RM reader takes an untagged
//! object as a `DV_TEXT`. The validation bundle of the same release types it
//! with a `U*` union whose every member lists `_type` under `required`, and
//! ITS-REST `docs/overview/Specifications.md` names that bundle as the one
//! "used by (mock-)servers or applications to validate openEHR request and
//! response payloads". The generated field decodes through
//! `openehr_its::rest::tagged`, which refuses an object without `_type`.

/// One member that must carry its `_type`.
pub(crate) struct RestTaggedMember {
    /// The OAS component schema name.
    pub dto: &'static str,
    /// The OAS property name.
    pub field: &'static str,
    /// The validation-bundle union the member is typed with.
    pub union: &'static str,
    /// The spec evidence.
    pub citation: &'static str,
}

/// The ITS-REST payload members that require their `_type`.
pub(crate) const REST_TAGGED_MEMBERS: &[RestTaggedMember] = &[
    RestTaggedMember {
        dto: "UpdateAudit",
        field: "description",
        union: "UDvText",
        citation: "ITS-REST `schemas/common/UpdateAudit.yaml` types `description` as \
                   `UDvText.yaml`, whose members `UMDvText.yaml` and `UMDvCodedText.yaml` \
                   both list `_type` under `required`",
    },
    RestTaggedMember {
        dto: "UpdateAttestation",
        field: "description",
        union: "UDvText",
        citation: "ITS-REST `schemas/common/UpdateAttestation.yaml` inherits `description` \
                   from `UpdateAudit.yaml` (`UDvText.yaml`; `UMDvText.yaml` and \
                   `UMDvCodedText.yaml` require `_type`)",
    },
    RestTaggedMember {
        dto: "UpdateAttestation",
        field: "reason",
        union: "UDvText",
        citation: "ITS-REST `schemas/common/UpdateAttestation.yaml` types `reason` as \
                   `UDvText.yaml`, whose members `UMDvText.yaml` and `UMDvCodedText.yaml` \
                   both list `_type` under `required`",
    },
];

/// The lines `emit-rest` writes for a `_type`-required member.
pub(crate) struct TaggedField {
    /// The doc line carrying the citation.
    pub doc: String,
    /// The serde attribute routing the decode through `rest::tagged`.
    pub attr: &'static str,
}

/// The field lines for `(dto, field)`, or `None` when the member reads as its
/// RM class does.
pub(crate) fn tagged_field(dto: &str, field: &str, required: bool) -> Option<TaggedField> {
    let member = REST_TAGGED_MEMBERS
        .iter()
        .find(|m| m.dto == dto && m.field == field)?;
    Some(TaggedField {
        doc: format!(
            "    /// `_type` REQUIRED by `{}`: {}\n",
            member.union, member.citation
        ),
        attr: if required {
            "    #[serde(deserialize_with = \"crate::rest::tagged::required\")]\n"
        } else {
            "    #[serde(default, deserialize_with = \"crate::rest::tagged::optional\")]\n"
        },
    })
}

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    reason = "dev tooling over the vendored OAS bundles, read as JSON (#1694)"
)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::Value;

    use super::REST_TAGGED_MEMBERS;
    use crate::load::oas::Oas;

    /// The vendored `*-validation.openapi.yaml` bundles.
    fn validation_bundles() -> Vec<Oas> {
        let dir = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/openehr-its/vendor/rest-oas"
        ));
        let mut paths: Vec<_> = std::fs::read_dir(dir)
            .expect("the vendored OAS directory")
            .map(|e| e.expect("a directory entry").path())
            .filter(|p| {
                p.file_name()
                    .and_then(std::ffi::OsStr::to_str)
                    .is_some_and(|n| n.ends_with("-validation.openapi.yaml"))
            })
            .collect();
        paths.sort();
        paths
            .iter()
            .map(|p| Oas::parse_file(p).expect("a vendored bundle parses"))
            .collect()
    }

    /// Every entry is grounded in the vendored validation bundles: the member
    /// is typed with the cited union, and every member of that union lists
    /// `_type` under `required`.
    #[test]
    fn every_entry_matches_the_validation_bundles() {
        let bundles = validation_bundles();
        assert!(!bundles.is_empty(), "no validation bundle is vendored");
        for m in REST_TAGGED_MEMBERS {
            let mut refs = BTreeSet::new();
            let mut members = Vec::new();
            for bundle in &bundles {
                for (name, schema) in bundle.schemas() {
                    if name == m.dto
                        && let Some(r) = schema
                            .pointer(&format!("/properties/{}", m.field))
                            .and_then(Oas::ref_name)
                    {
                        refs.insert(r);
                    }
                    if name == m.union {
                        for member in schema
                            .get("oneOf")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                        {
                            let required = bundle
                                .resolve(member)
                                .get("required")
                                .and_then(Value::as_array)
                                .is_some_and(|r| r.iter().any(|k| k == "_type"));
                            members.push((Oas::ref_name(member), required));
                        }
                    }
                }
            }
            assert!(
                refs.len() == 1 && refs.contains(m.union),
                "{}.{}: the entry cites {} but the bundles type it {refs:?}",
                m.dto,
                m.field,
                m.union,
            );
            assert!(
                !members.is_empty() && members.iter().all(|(_, required)| *required),
                "{}: every member must require `_type`, the bundles say {members:?}",
                m.union,
            );
        }
    }

    /// A member outside the table keeps its RM reading, and the attribute
    /// follows the member's optionality.
    #[test]
    fn the_attribute_follows_optionality() {
        assert!(super::tagged_field("UpdateAudit", "committer", true).is_none());
        let optional = super::tagged_field("UpdateAudit", "description", false).expect("listed");
        assert!(optional.attr.contains("tagged::optional"));
        let required = super::tagged_field("UpdateAttestation", "reason", true).expect("listed");
        assert!(required.attr.contains("tagged::required"));
    }
}
