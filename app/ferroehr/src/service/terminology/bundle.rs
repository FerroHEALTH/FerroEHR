// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The in-process openEHR-bundle provider for `I_TERMINOLOGY_SERVICE`, backed
//! by the compile-time-embedded `openehr-term` bundle (TERM 3.1.0).
//!
//! Spec: `docs/specs/openehr/SM/docs/UML/classes/i_terminology_service.adoc`
//! (the 9 calls plus `Pre_has_terminology`/`Pre_has_term`/`Pre_has_value_set`)
//! and the extract model (`terminology_extract.adoc`, `term_code.adoc`,
//! `defined_term.adoc`). `BASE/docs/architecture_overview/
//! master12-terminology.adoc` grounds the "openehr" id and the group/value-set
//! split. The extract types live in [`super::types`]; this module is the
//! DB-free mapping onto [`openehr_term::bundle::openehr`], the enumerable local
//! default the routing layer ([`super::routing`]) selects between alongside the
//! remote FHIR TS ([`super::fhir`]).
//!
//! # Bundle mapping
//!
//! - Terminologies. The primary id is `"openehr"`, the internal openEHR
//!   vocabulary (all `<group>`s and internal `<codeset>`s). The four external
//!   code sets (ISO 639-1 languages, ISO 3166-1 countries, IANA character sets,
//!   IANA media types) are exposed as separate terminologies addressed by their
//!   `external_id`, so `get_terminology_ids` lists `"openehr"` plus those four.
//! - Terms and value sets. openEHR terminology codes are group-scoped (code
//!   `532` is `complete` in `version_lifecycle_state` and `completed` in
//!   `instruction_states` — SPECPR-51), so the code-only `has_term`/`get_term`
//!   calls treat `"openehr"` as flat: a term is any concept id present in any
//!   group and `get_term` returns the first matching group's rubric.
//!   Group-scoped access is the value-set calls, whose `value_set_id` is the
//!   group.
//! - Value sets. For `"openehr"` a `value_set_code` resolves in order to an
//!   internal group by `openehr_id`, a group by display name, or an internal
//!   code set by `openehr_id`; an external terminology's code set is its own
//!   single value set. `value_set_validate` is set membership.
//! - `subsumes`. The openEHR vocabulary is flat, so this answers identity only,
//!   and being strict (`i_terminology_service.adoc`) even the identity case is
//!   `false`. Hierarchical subsumption is the FHIR provider's `$subsumes`.
//! - `at_date`. The bundle is a single pinned version, so the temporal
//!   parameter never changes the answer here; it is honoured by the FHIR
//!   provider.
//! - `attributes`. No meta-model attributes are defined for the openEHR bundle
//!   (`Terminology_description.attributes` is `None`), so the `get_term`
//!   allow-list has nothing to filter.
//! - URI. The TERM spec defines no canonical machine URI for the internal
//!   terminology, so we publish the openEHR terminology repository URI
//!   (`TERM/docs/SupportTerminology/master00-amendment_record.adoc`). External
//!   sets publish their `external_id`.

use openehr_term::bundle::openehr;
use openehr_term::v3_1::terminology::code_set::CodeSet;

use crate::service::status::{CallStatusType, SmError};
use crate::service::terminology::types::{
    DefinedTerm, TermCode, TermEntry, TerminologyDescription, TerminologyExtract,
};
use crate::versioning::audit::OPENEHR;

/// The published identifying URI for the internal openEHR terminology.
const OPENEHR_TERM_URI: &str = "https://github.com/openEHR/terminology";
/// The canonical rubric language for `get_term`/`get_value_set` display text.
const CANONICAL_LANG: &str = "en";

/// A `Pre_has_terminology` failure → `NotFound` (the abstract SM error has no
/// ITS-REST wire binding — the terminology surface is native-API-only;
/// `NotFound` is the natural HTTP reading of an unknown terminology).
fn unknown_terminology(id: &str) -> SmError {
    SmError::new(
        CallStatusType::VersionedObjectDoesNotExist,
        format!("terminology `{id}`"),
    )
}

/// The external code set addressed by `terminology_id` (its `external_id` or
/// `openehr_id`), if any.
fn external_terminology(terminology_id: &str) -> Option<&'static CodeSet> {
    openehr().external_code_sets().iter().find(|cs| {
        cs.external_id.as_deref() == Some(terminology_id) || cs.openehr_id == terminology_id
    })
}

/// The value-set members (`code`, optional display text) for a
/// `value_set_code` within `terminology_id`, or `None` if no such value set
/// exists.
fn resolve_value_set(
    terminology_id: &str,
    value_set_code: &str,
) -> Option<Vec<(String, Option<String>)>> {
    let t = openehr();
    if terminology_id == OPENEHR {
        // An internal group, by openehr_id or by display name.
        let group = t
            .group(value_set_code)
            .or_else(|| t.group_id(value_set_code).and_then(|id| t.group(id)));
        if let Some(g) = group {
            return Some(
                g.concepts
                    .iter()
                    .flatten()
                    .map(|c| (c.id.clone(), Some(c.rubric.clone())))
                    .collect(),
            );
        }
        // An internal code set, by openehr_id.
        if let Some(cs) = t.code_set(value_set_code) {
            return Some(code_set_members(cs));
        }
        None
    } else if let Some(cs) = external_terminology(terminology_id) {
        // An external code set is its own single value set.
        let matches = value_set_code == terminology_id
            || value_set_code == cs.openehr_id
            || cs.external_id.as_deref() == Some(value_set_code);
        matches.then(|| code_set_members(cs))
    } else {
        None
    }
}

/// Members of a code set as (`code`, optional description).
fn code_set_members(cs: &CodeSet) -> Vec<(String, Option<String>)> {
    cs.codes
        .iter()
        .flatten()
        .map(|c| (c.value.clone(), c.description.clone()))
        .collect()
}

/// Build a `Terminology_extract` from resolved members: a member with display
/// text becomes a `Defined_term`, a bare code a `Term_code`.
fn extract_from_members(
    terminology_id: &str,
    members: Vec<(String, Option<String>)>,
) -> TerminologyExtract {
    let terms = members
        .into_iter()
        .map(|(code, text)| {
            let entry = match text {
                Some(text) => TermEntry::Defined(DefinedTerm {
                    code: code.clone(),
                    text,
                    language: Some(CANONICAL_LANG.to_owned()),
                    is_preferred_term: None,
                }),
                None => TermEntry::Bare(TermCode { code: code.clone() }),
            };
            (code, entry)
        })
        .collect();
    TerminologyExtract {
        terminology_id: terminology_id.to_owned(),
        terminology_version: bundle_version(),
        terms: Some(terms),
        // The openEHR bundle is flat (no subsumption/relationship
        // meta-model), so no `Term_relationship`s are emitted (NOTE,
        // module head).
        relationships: None,
        relations: None,
    }
}

/// The pinned bundle version (`"3.1.0"`), if the asset carries one.
fn bundle_version() -> Option<String> {
    openehr().terminology().version.clone()
}

// ─── the 9 SM calls, as DB-free functions the routing layer delegates to ─────

/// `get_terminology_ids` — `"openehr"` plus every external code set's id.
pub(super) fn terminology_ids() -> Vec<String> {
    let mut ids = vec![OPENEHR.to_owned()];
    ids.extend(openehr().external_code_sets().iter().map(|cs| {
        cs.external_id
            .clone()
            .unwrap_or_else(|| cs.openehr_id.clone())
    }));
    ids
}

/// `has_terminology`.
pub(super) fn has_terminology(terminology_id: &str) -> bool {
    terminology_id == OPENEHR || external_terminology(terminology_id).is_some()
}

/// `get_terminology_description`. `Pre_has_terminology`.
pub(super) fn terminology_description(
    terminology_id: &str,
) -> Result<TerminologyDescription, SmError> {
    if terminology_id == OPENEHR {
        return Ok(TerminologyDescription {
            publisher: "openEHR Foundation".to_owned(),
            available_versions: bundle_version().map(|v| vec![v]),
            // No meta-model attributes are exposed for the openEHR bundle
            // (`terminology_description.adoc` `attributes` 0..1).
            attributes: None,
            uri: OPENEHR_TERM_URI.to_owned(),
        });
    }
    match external_terminology(terminology_id) {
        Some(cs) => Ok(TerminologyDescription {
            publisher: cs.issuer.clone(),
            available_versions: None,
            attributes: None,
            uri: cs
                .external_id
                .clone()
                .unwrap_or_else(|| cs.openehr_id.clone()),
        }),
        None => Err(unknown_terminology(terminology_id)),
    }
}

/// `has_term`. `Pre_has_terminology` → `NotFound` on an unknown terminology.
pub(super) fn has_term(terminology_id: &str, code: &str) -> Result<bool, SmError> {
    if !has_terminology(terminology_id) {
        return Err(unknown_terminology(terminology_id));
    }
    if terminology_id == OPENEHR {
        // A term is any concept id present in any group (flat view — see the
        // module NOTE on group-scoping).
        Ok(openehr()
            .terminology()
            .vocabularies
            .iter()
            .flatten()
            .any(|g| g.concepts.iter().flatten().any(|c| c.id == code)))
    } else {
        Ok(external_terminology(terminology_id)
            .is_some_and(|cs| cs.codes.iter().flatten().any(|c| c.value == code)))
    }
}

/// `get_term`. `Pre_has_terminology` + `Pre_has_term` (both → `NotFound`).
pub(super) fn get_term(terminology_id: &str, code: &str) -> Result<TerminologyExtract, SmError> {
    if !has_terminology(terminology_id) {
        return Err(unknown_terminology(terminology_id));
    }
    if !has_term(terminology_id, code)? {
        return Err(SmError::new(
            CallStatusType::VersionedObjectDoesNotExist,
            format!("term `{code}` in terminology `{terminology_id}`"),
        ));
    }
    let member = if terminology_id == OPENEHR {
        // First group containing the concept supplies the rubric.
        let rubric = openehr()
            .terminology()
            .vocabularies
            .iter()
            .flatten()
            .find_map(|g| g.concepts.iter().flatten().find(|c| c.id == code))
            .map(|c| c.rubric.clone());
        (code.to_owned(), rubric)
    } else {
        let description = external_terminology(terminology_id)
            .and_then(|cs| cs.codes.iter().flatten().find(|c| c.value == code))
            .and_then(|c| c.description.clone());
        (code.to_owned(), description)
    };
    Ok(extract_from_members(terminology_id, vec![member]))
}

/// `subsumes` — SM master12 `i_terminology_service.adoc`: "True if
/// `candidate_child_code` is in the **strict** subsumption of `ref_code`".
/// Strict subsumption excludes the code itself, and the openEHR bundle's
/// vocabularies are flat (no is-a hierarchy), so no code strictly subsumes
/// any other here — always False. `Pre_has_terminology`. (Hierarchical
/// subsumption is served by the external FHIR provider's
/// `CodeSystem/$subsumes`.)
pub(super) fn subsumes(
    terminology_id: &str,
    _ref_code: &str,
    _candidate_child_code: &str,
) -> Result<bool, SmError> {
    if !has_terminology(terminology_id) {
        return Err(unknown_terminology(terminology_id));
    }
    Ok(false)
}

/// `has_value_set` — total (no precondition); unknown terminology → `false`.
pub(super) fn has_value_set(terminology_id: &str, value_set_code: &str) -> bool {
    resolve_value_set(terminology_id, value_set_code).is_some()
}

/// `value_set_validate` — set membership. `Pre_has_terminology`.
pub(super) fn value_set_validate(
    terminology_id: &str,
    value_set_id: &str,
    candidate_code: &str,
) -> Result<bool, SmError> {
    if !has_terminology(terminology_id) {
        return Err(unknown_terminology(terminology_id));
    }
    Ok(resolve_value_set(terminology_id, value_set_id)
        .is_some_and(|members| members.iter().any(|(code, _)| code == candidate_code)))
}

/// `get_value_set`. `Pre_has_terminology` + `Pre_has_value_set` (both →
/// `NotFound`).
pub(super) fn get_value_set(
    terminology_id: &str,
    value_set_code: &str,
) -> Result<TerminologyExtract, SmError> {
    if !has_terminology(terminology_id) {
        return Err(unknown_terminology(terminology_id));
    }
    match resolve_value_set(terminology_id, value_set_code) {
        Some(members) => Ok(extract_from_members(terminology_id, members)),
        None => Err(SmError::new(
            CallStatusType::VersionedObjectDoesNotExist,
            format!("value set `{value_set_code}` in terminology `{terminology_id}`"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_include_openehr_and_externals() {
        let ids = terminology_ids();
        assert!(ids.iter().any(|id| id == "openehr"));
        assert!(ids.iter().any(|id| id == "ISO_639-1"));
        assert!(ids.iter().any(|id| id == "ISO_3166-1"));
        // Every advertised id passes has_terminology.
        assert!(ids.iter().all(|id| has_terminology(id)));
        assert!(!has_terminology("no_such_terminology"));
    }

    #[test]
    fn description_openehr_and_external() {
        let d = terminology_description("openehr").expect("openehr description");
        assert_eq!(d.publisher, "openEHR Foundation");
        assert_eq!(d.uri, OPENEHR_TERM_URI);
        assert_eq!(
            d.available_versions.as_deref(),
            Some(&["3.1.0".to_owned()][..])
        );

        let ext = terminology_description("ISO_639-1").expect("ISO_639-1 description");
        assert_eq!(ext.publisher, "ISO");
        assert_eq!(ext.uri, "ISO_639-1");

        assert!(matches!(
            terminology_description("bogus"),
            Err(SmError {
                status: CallStatusType::VersionedObjectDoesNotExist,
                ..
            })
        ));
    }

    #[test]
    fn has_term_and_get_term_openehr() {
        // 249 is `creation` in the audit_change_type group.
        assert!(has_term("openehr", "249").unwrap());
        assert!(!has_term("openehr", "not-a-code").unwrap());

        let extract = get_term("openehr", "249").expect("get_term 249");
        assert_eq!(extract.terminology_id, "openehr");
        let terms = extract.terms.expect("terms present");
        match terms.get("249").expect("249 term") {
            TermEntry::Defined(dt) => {
                assert_eq!(dt.code, "249");
                assert_eq!(dt.text, "creation");
                assert_eq!(dt.language.as_deref(), Some("en"));
            }
            TermEntry::Bare(_) => panic!("expected a defined term with rubric"),
        }
    }

    #[test]
    fn get_term_unknown_code_is_not_found() {
        assert!(matches!(
            get_term("openehr", "not-a-code"),
            Err(SmError {
                status: CallStatusType::VersionedObjectDoesNotExist,
                ..
            })
        ));
        // Unknown terminology → NotFound (Pre_has_terminology) on has_term too.
        assert!(matches!(
            has_term("bogus", "249"),
            Err(SmError {
                status: CallStatusType::VersionedObjectDoesNotExist,
                ..
            })
        ));
    }

    #[test]
    fn has_term_external_terminology() {
        // "en" is a member of the ISO 639-1 languages code set.
        assert!(has_term("ISO_639-1", "en").unwrap());
        assert!(!has_term("ISO_639-1", "zz").unwrap());
        let extract = get_term("ISO_639-1", "en").expect("get_term en");
        // External codes have no rubric → bare Term_code.
        assert!(extract.terms.unwrap().contains_key("en"));
    }

    #[test]
    fn subsumes_is_strict_so_flat_vocabularies_never_subsume() {
        // SM master12: strict subsumption excludes identity; the bundle is
        // flat, so subsumes is uniformly false (incl. the identity case).
        assert!(!subsumes("openehr", "249", "249").unwrap());
        assert!(!subsumes("openehr", "249", "250").unwrap());
        assert!(matches!(
            subsumes("bogus", "a", "a"),
            Err(SmError {
                status: CallStatusType::VersionedObjectDoesNotExist,
                ..
            })
        ));
    }

    #[test]
    fn value_set_group_membership() {
        // The audit_change_type group as a value set: 249 in, bogus out.
        assert!(has_value_set("openehr", "audit_change_type"));
        assert!(value_set_validate("openehr", "audit_change_type", "249").unwrap());
        assert!(!value_set_validate("openehr", "audit_change_type", "99999").unwrap());

        let vs = get_value_set("openehr", "audit_change_type").expect("value set");
        let terms = vs.terms.expect("members");
        assert!(terms.contains_key("249"));
        // 249 → creation (defined term with rubric).
        assert!(matches!(terms.get("249"), Some(TermEntry::Defined(_))));

        assert!(!has_value_set("openehr", "no_such_group"));
        assert!(matches!(
            get_value_set("openehr", "no_such_group"),
            Err(SmError {
                status: CallStatusType::VersionedObjectDoesNotExist,
                ..
            })
        ));
        // value_set_validate against an unknown value set → false (no
        // precondition on the membership test itself).
        assert!(!value_set_validate("openehr", "no_such_group", "249").unwrap());
    }

    #[test]
    fn value_set_by_display_name() {
        // Groups are also addressable by their display name.
        assert!(has_value_set("openehr", "audit change type"));
        assert!(value_set_validate("openehr", "audit change type", "249").unwrap());
    }

    #[test]
    fn at_date_is_accepted_but_answers_from_pinned_version() {
        // (The at_date threading lives in the routing layer; here we assert
        // the pinned version is what the extract reports.)
        let extract = get_term("openehr", "249").unwrap();
        assert_eq!(extract.terminology_version.as_deref(), Some("3.1.0"));
    }
}
