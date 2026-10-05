// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `spec_profile` compatibility stamp and the read-time refusal it feeds.
//!
//! No openEHR spec governs runtime specification-generation selection — our own
//! design/extension. The compatibility direction it rests on is spec-governed:
//! the openEHR release strategy
//! (<https://specifications.openehr.org/governance/release_strategy>) defines a
//! minor release as "significant additions that do not change the semantics of
//! the existing part of the release", so a body accepted by a released
//! generation stays acceptable to a later development generation and never the
//! reverse.
//!
//! A commit under the development generations therefore also asks the released
//! generation's reader whether it could read the same body and stores the answer
//! (`version.stable_compatible`). A deployment configured to the `stable`
//! profile refuses to serve a version that answer marks inexpressible, rather
//! than serving a body it does not implement or rewriting one it must not.

use serde_json::Value;

use crate::config::profile::SpecProfile;
use crate::service::error::ServiceError;
use crate::service::status::CallStatusType;
use crate::versioning::Kind;

/// Reads `canonical` with the RELEASED (`stable` profile) generation's strict
/// reader, returning its refusal.
///
/// `Ok(())` means the released generations can express the stored body. The
/// released and development generations differ only in the surface each
/// declares, so this is exactly the question the profile boundary asks.
fn read_as_released(
    kind: Kind,
    canonical: &Value,
) -> Result<(), openehr_its::json::JsonParseError> {
    /// Reads the value as `T` and discards it — only the refusal matters here.
    fn probe<T: serde::de::DeserializeOwned>(
        value: &Value,
    ) -> Result<(), openehr_its::json::JsonParseError> {
        openehr_its::json::from_canonical_value::<T>(value).map(drop)
    }
    match kind {
        Kind::Composition => {
            probe::<openehr_rm::v1_1::composition::composition::Composition>(canonical)
        }
        Kind::EhrStatus => probe::<openehr_rm::v1_1::ehr::ehr_status::EhrStatus>(canonical),
        Kind::EhrAccess => probe::<openehr_rm::v1_1::ehr::ehr_access::EhrAccess>(canonical),
        Kind::Folder => probe::<openehr_rm::v1_1::common::directory::folder::Folder>(canonical),
        Kind::Agent => probe::<openehr_rm::v1_1::demographic::agent::Agent>(canonical),
        Kind::Group => probe::<openehr_rm::v1_1::demographic::group::Group>(canonical),
        Kind::Organisation => {
            probe::<openehr_rm::v1_1::demographic::organisation::Organisation>(canonical)
        }
        Kind::Person => probe::<openehr_rm::v1_1::demographic::person::Person>(canonical),
        Kind::Role => probe::<openehr_rm::v1_1::demographic::role::Role>(canonical),
        Kind::PartyRelationship => {
            probe::<openehr_rm::v1_1::demographic::party_relationship::PartyRelationship>(canonical)
        }
    }
}

/// Whether a version body about to be committed is expressible in the RELEASED
/// generation set — the value stored in `version.stable_compatible`.
///
/// A version with no content (a logical delete stores no node rows — RM common
/// `master06-change_control_package.adoc` §Logical Deletion) carries nothing a
/// generation could fail to express, so it stamps `true`.
pub(crate) fn stable_compatible(profile: SpecProfile, kind: Kind, canonical: &Value) -> bool {
    // Under the `stable` profile the ingress boundary already accepted the body
    // against the released generations, so the stamp holds by construction and
    // no second parse runs.
    if profile == SpecProfile::Stable || canonical.is_null() {
        return true;
    }
    read_as_released(kind, canonical).is_ok()
}

/// Refuses to serve a stored version the active profile's generations cannot
/// express.
///
/// The refusal is the generic SM `conflict` status, `409`, taking the same
/// bridge row every other `409` takes. No openEHR spec governs runtime
/// generation selection, so the status comes from HTTP itself: RFC 9110
/// §15.5.10 assigns `409` to "a conflict with the current state of the target
/// resource" whose resolution the response should describe, which is this case,
/// the stored state conflicting with the declared profile.
fn refuse(
    profile: SpecProfile,
    kind: Kind,
    version_uid: &str,
    refusal: &openehr_its::json::JsonParseError,
) -> ServiceError {
    // NOTE (reliability.md §PHI caveat): the reader diagnostic can quote stored
    // content, so it is traced and never put on the wire; the wire message
    // carries identifiers, the profile and the remedy.
    tracing::warn!(
        spec_profile = profile.as_str(),
        kind = kind.as_str(),
        version_uid,
        error = %refusal,
        "refusing to serve a stored version the active spec_profile cannot express"
    );
    ServiceError::sm(
        CallStatusType::Conflict,
        format!(
            "version {version_uid} ({kind}) was stored using openEHR specification surface the \
             active spec_profile `{profile}` does not define, and this server never \
             down-converts stored content; set spec_profile = \"development\" to read it",
            kind = kind.as_str(),
        ),
    )
}

/// The read-time profile gate: refuses a stored version body the ACTIVE profile
/// cannot express, and passes everything else through untouched.
///
/// `stamp` is the stored `version.stable_compatible`: `Some` from the
/// commit-time assessment, `None` for a row nothing stamped (committed before
/// the column existed, or written by a verbatim-replay path — the EHR-Extract
/// import and the archive load). A `None` row is assessed on the fly with the
/// same reader, without writing the answer back: a read stays a read, and the
/// cost lands only on pre-stamp rows under the non-default profile.
///
/// # Errors
/// A `409`-class [`ServiceError::Conflict`] naming the profile, the version and
/// the remedy — see [`refuse`].
pub(crate) fn gate(
    profile: SpecProfile,
    kind: Kind,
    stamp: Option<bool>,
    canonical: &Value,
    version_uid: &dyn Fn() -> String,
) -> Result<(), ServiceError> {
    if profile != SpecProfile::Stable || canonical.is_null() {
        return Ok(());
    }
    match stamp {
        Some(true) => Ok(()),
        // The stamp says the released generations cannot express this body; the
        // reader is run again only to produce the traced diagnostic, on a
        // refusal path that ends the request anyway.
        Some(false) | None => match read_as_released(kind, canonical) {
            Ok(()) => Ok(()),
            Err(e) => Err(refuse(profile, kind, &version_uid(), &e)),
        },
    }
}

/// The AQL result-assembly profile gate: refuses the query when a whole-object
/// (or subtree) `RESULT_SET` cell would serve content out of a stored version
/// the ACTIVE profile's generations cannot express.
///
/// A whole-object projection serves a stored version body, so it answers the
/// same question [`gate`] answers on the resource reads, keyed on the same
/// per-version stamp. The refusal is the whole query: the ITS-REST `RESULT_SET`
/// is columns and rows of values with no per-row diagnostic channel
/// (`SM/docs/UML/classes/result_set.adoc`; `responses/200_Query.yaml`), so the
/// only per-row alternative would be a silent elision.
///
/// Scalar and leaf cells are not gated: they serve data values over paths the
/// planning gate already bounded to the active generation's declared surface
/// (`crate::aql::analyze`).
///
/// Cost under the default `development` profile is zero, the first line
/// returning. Under `stable` it is one key-lookup statement over the page's
/// distinct `(vo_id, sys_version)` pairs returning only versions not stamped
/// compatible, plus one batched body load for any it returns.
///
/// # Errors
/// The `409`-class [`ServiceError::Conflict`] of [`refuse`], naming the first
/// offending version in `(vo_id, sys_version)` order; a storage failure of the
/// candidate read or the body load.
pub(crate) async fn gate_result_bodies(
    pool: &sqlx::PgPool,
    profile: SpecProfile,
    anchors: &[crate::storage::node_repo::SubtreeAnchor],
) -> Result<(), ServiceError> {
    if profile != SpecProfile::Stable || anchors.is_empty() {
        return Ok(());
    }
    let versions: Vec<(crate::ids::VoId, i32)> = anchors
        .iter()
        .map(|a| (a.vo_id, a.sys_version))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let candidates =
        crate::storage::version_repo::read::read_profile_gate_candidates(pool, &versions).await?;
    if candidates.is_empty() {
        return Ok(());
    }
    // The stamp is a per-VERSION fact, so the assessment reads the whole stored
    // body — the projected subtree is a fragment of it and answers a narrower
    // question than the stamp records.
    let roots: Vec<crate::storage::node_repo::SubtreeAnchor> = candidates
        .iter()
        .filter_map(|c| {
            c.root_interval
                .map(|(num, num_cap)| crate::storage::node_repo::SubtreeAnchor {
                    vo_id: c.vo_id,
                    sys_version: c.sys_version,
                    num,
                    num_cap,
                })
        })
        .collect();
    let bodies = crate::storage::node_repo::read_subtrees_canonical(pool, &roots).await?;
    for candidate in &candidates {
        let kind = Kind::from_type(&candidate.kind).ok_or_else(|| {
            ServiceError::exception(format!(
                "version.kind {:?} of versioned object {} is not an RM versioned type",
                candidate.kind, candidate.vo_id
            ))
        })?;
        let canonical = candidate
            .root_interval
            .and_then(|(num, num_cap)| {
                bodies.get(&crate::storage::node_repo::SubtreeAnchor {
                    vo_id: candidate.vo_id,
                    sys_version: candidate.sys_version,
                    num,
                    num_cap,
                })
            })
            .cloned()
            .unwrap_or(Value::Null);
        gate(
            profile,
            kind,
            candidate.stable_compatible,
            &canonical,
            &|| {
                crate::versioning::object_version_id::object_version_id(
                    candidate.vo_id,
                    &candidate.creating_system_id,
                    crate::versioning::object_version_id::TreeId::from_columns(
                        candidate.trunk_version,
                        candidate.branch_number,
                        candidate.branch_version,
                    ),
                )
            },
        )?;
    }
    Ok(())
}

#[cfg(test)]
#[expect(
    clippy::panic_in_result_fn,
    reason = "Result-returning tests with assertions — the Book ch11 shape \
              (https://doc.rust-lang.org/book/ch11-01-writing-tests.html)"
)]
mod tests {
    use super::*;

    /// A COMPOSITION carrying the given `content` list.
    fn composition(content: &Value) -> Value {
        serde_json::json!({
            "_type": "COMPOSITION",
            "name": { "_type": "DV_TEXT", "value": "generic" },
            "archetype_node_id": "openEHR-EHR-COMPOSITION.encounter.v1",
            "language": { "_type": "CODE_PHRASE",
                          "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" },
                          "code_string": "en" },
            "territory": { "_type": "CODE_PHRASE",
                           "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_3166-1" },
                           "code_string": "NL" },
            "category": { "_type": "DV_CODED_TEXT", "value": "event",
                          "defining_code": { "_type": "CODE_PHRASE",
                                             "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                                             "code_string": "433" } },
            "composer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Test" },
            "content": content
        })
    }

    /// A COMPOSITION whose one `content` item is a `GENERIC_ENTRY` whose `data`
    /// is a `CLUSTER` — surface only the DEVELOPMENT generation defines.
    ///
    /// `GENERIC_ENTRY.data` is typed `ITEM_TREE` in RM 1.1.0 and RETYPED to the
    /// abstract `ITEM` (= `CLUSTER` | `ELEMENT`) after that release: SPECRM-18,
    /// "Change `GENERIC_ENTRY` data attribute type from `ITEM_TREE` to the
    /// abstract `ITEM` class" (`RM/docs/integration/master00-amendment_record.adoc`,
    /// issue 1.0, above the `RM Release 1.1.0` marker). The two types are
    /// disjoint, so a `CLUSTER` here is readable by the development generation
    /// and refused by the released one.
    fn development_only_composition() -> Value {
        composition(&serde_json::json!([ {
            "_type": "GENERIC_ENTRY",
            "name": { "_type": "DV_TEXT", "value": "entry" },
            "archetype_node_id": "openEHR-EHR-GENERIC_ENTRY.msg.v1",
            "data": {
                "_type": "CLUSTER",
                "name": { "_type": "DV_TEXT", "value": "data" },
                "archetype_node_id": "at0000",
                "items": [ {
                    "_type": "ELEMENT",
                    "name": { "_type": "DV_TEXT", "value": "leaf" },
                    "archetype_node_id": "at0001",
                    "value": { "_type": "DV_TEXT", "value": "x" }
                } ]
            }
        } ]))
    }

    /// A COMPOSITION whose one `content` item is an ordinary `SECTION` — a body
    /// both generation sets express.
    fn stable_clean_composition() -> Value {
        composition(&serde_json::json!([ {
            "_type": "SECTION",
            "name": { "_type": "DV_TEXT", "value": "section" },
            "archetype_node_id": "openEHR-EHR-SECTION.adhoc.v1"
        } ]))
    }

    /// A body both generations express stamps `true`.
    #[test]
    fn a_released_generation_body_is_stable_compatible() {
        let body = stable_clean_composition();
        if let Err(e) = read_as_released(Kind::Composition, &body) {
            panic!("the released reader must accept the SECTION body: {e}");
        }
        assert!(stable_compatible(
            SpecProfile::Development,
            Kind::Composition,
            &body
        ));
    }

    /// A body only the development generation expresses stamps `false`.
    #[test]
    fn a_development_only_body_is_not_stable_compatible() {
        let body = development_only_composition();
        if let Err(e) =
            openehr_its::json::from_canonical_value::<openehr_rm::prelude::Composition>(&body)
        {
            panic!("the development reader must accept the CLUSTER body: {e}");
        }
        assert!(!stable_compatible(
            SpecProfile::Development,
            Kind::Composition,
            &body
        ));
    }

    /// Under the `stable` profile the stamp holds by construction — the
    /// ingress boundary already read the body against the released
    /// generations, so no second parse decides it.
    #[test]
    fn stable_profile_stamps_true_without_reparsing() {
        assert!(stable_compatible(
            SpecProfile::Stable,
            Kind::Composition,
            &development_only_composition()
        ));
    }

    /// A logically deleted version (no node rows → `Value::Null`) carries
    /// nothing a generation could fail to express (RM common master06 §Logical
    /// Deletion).
    #[test]
    fn a_deleted_version_is_stable_compatible_and_never_gated() -> Result<(), ServiceError> {
        assert!(stable_compatible(
            SpecProfile::Development,
            Kind::Composition,
            &Value::Null
        ));
        gate(
            SpecProfile::Stable,
            Kind::Composition,
            Some(false),
            &Value::Null,
            &|| "uid".to_owned(),
        )
    }

    /// The gate is a no-op under the development profile, whatever the stamp
    /// says — the development generations express the released surface too.
    #[test]
    fn development_profile_serves_every_stamp() -> Result<(), ServiceError> {
        let body = development_only_composition();
        for stamp in [Some(true), Some(false), None] {
            gate(
                SpecProfile::Development,
                Kind::Composition,
                stamp,
                &body,
                &|| "uid".to_owned(),
            )?;
        }
        Ok(())
    }

    /// Under the `stable` profile a `false` stamp refuses with the `409`-class
    /// conflict naming the profile, the version and the remedy.
    #[test]
    fn stable_profile_refuses_a_false_stamp() {
        let err = gate(
            SpecProfile::Stable,
            Kind::Composition,
            Some(false),
            &development_only_composition(),
            &|| "1e2::sys::1".to_owned(),
        )
        .expect_err("a development-only body is refused under the stable profile");
        let ServiceError::Conflict(sm) = &err else {
            panic!("expected the 409-class conflict, got {err:?}");
        };
        assert_eq!(sm.status, CallStatusType::Conflict);
        assert!(sm.message.contains("1e2::sys::1"), "{}", sm.message);
        assert!(sm.message.contains("stable"), "{}", sm.message);
        assert!(sm.message.contains("development"), "{}", sm.message);
    }

    /// An unstamped (`NULL`) legacy row is assessed on the fly, in both
    /// directions.
    #[test]
    fn a_null_stamp_is_assessed_on_the_fly() -> Result<(), ServiceError> {
        gate(
            SpecProfile::Stable,
            Kind::Composition,
            None,
            &stable_clean_composition(),
            &|| "uid".to_owned(),
        )?;
        assert!(
            gate(
                SpecProfile::Stable,
                Kind::Composition,
                None,
                &development_only_composition(),
                &|| "uid".to_owned(),
            )
            .is_err(),
            "an unstamped development-only body is refused under the stable profile"
        );
        Ok(())
    }

    /// A `true` stamp that the released reader would in fact refuse is still
    /// served: the stamp is the authority on the fast path, which is what
    /// makes it worth storing.
    #[test]
    fn a_true_stamp_is_trusted_without_reparsing() -> Result<(), ServiceError> {
        gate(
            SpecProfile::Stable,
            Kind::Composition,
            Some(true),
            &development_only_composition(),
            &|| "uid".to_owned(),
        )
    }
}
