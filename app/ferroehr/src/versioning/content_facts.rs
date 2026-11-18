// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What the write rules of a caller read of a COMPOSITION: its composer, its
//! feeder system, its facility and its participations' performers (#6003).
//!
//! The person-insert rule ([`super::person`]), the system-client rule
//! ([`super::system`]) and the external-caller rule ([`super::external`]) decide
//! over these facts alone, read off a typed COMPOSITION or off a canonical
//! value, so the two forms cannot be judged apart. No openEHR spec governs
//! these rules: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the canonical value a CONTRIBUTION member or a \
              stored body still carries is one of the two forms the facts are read off"
)]

use openehr_rm::prelude::{Composition, PartyIdentified, PartyProxy};
use openehr_rm::v1_2::data_types::basic::dv_identifier::DvIdentifier;
use serde_json::Value;

/// The facts of one COMPOSITION.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ContentFacts<'a> {
    /// The composer's RM type (`PARTY_SELF`, `PARTY_IDENTIFIED`, …).
    pub(crate) composer_type: Option<&'a str>,
    /// The `type` of the composer's `external_ref`.
    pub(crate) composer_ref_type: Option<&'a str>,
    /// The composer's `(issuer, id)` identifiers, skipping one without an issuer.
    pub(crate) composer_identifiers: Vec<(&'a str, &'a str)>,
    /// `feeder_audit.originating_system_audit.system_id`.
    pub(crate) feeder_system_id: Option<&'a str>,
    /// The `(issuer, id)` identifiers of the facility the COMPOSITION names.
    pub(crate) facility: Vec<(&'a str, &'a str)>,
    /// Where that facility sits: `EVENT_CONTEXT.health_care_facility`, or with
    /// no context `feeder_audit.originating_system_audit.location`.
    pub(crate) facility_path: &'static str,
    /// Whether a participation's performer is a `PARTY_IDENTIFIED` carrying an
    /// identifier.
    pub(crate) identified_performer: bool,
}

/// The path of a context's facility.
const CONTEXT_FACILITY: &str = "EVENT_CONTEXT.health_care_facility";
/// The path of a context-less COMPOSITION's facility.
const FEEDER_FACILITY: &str = "COMPOSITION.feeder_audit.originating_system_audit.location";

impl<'a> ContentFacts<'a> {
    /// The facts of a typed COMPOSITION.
    pub(crate) fn of_typed(composition: &'a Composition) -> Self {
        let (composer_type, composer_ref_type, composer_identifiers) = proxy(&composition.composer);
        let origin = composition
            .feeder_audit
            .as_ref()
            .map(|feeder| feeder.originating_system_audit.as_ref());
        let (facility, facility_path) = match composition.context.as_ref() {
            Some(context) => (
                context
                    .health_care_facility
                    .as_deref()
                    .map(identified)
                    .unwrap_or_default(),
                CONTEXT_FACILITY,
            ),
            None => (
                origin
                    .and_then(|details| details.location.as_deref())
                    .map(identified)
                    .unwrap_or_default(),
                FEEDER_FACILITY,
            ),
        };
        let identified_performer = composition
            .context
            .as_ref()
            .and_then(|context| context.participations.as_ref())
            .is_some_and(|participations| {
                participations.iter().any(|participation| {
                    let (kind, _, identifiers) = proxy(&participation.performer);
                    kind == Some("PARTY_IDENTIFIED") && !identifiers.is_empty()
                })
            });
        Self {
            composer_type,
            composer_ref_type,
            composer_identifiers,
            feeder_system_id: origin.map(|details| details.system_id.as_str()),
            facility,
            facility_path,
            identified_performer,
        }
    }

    /// The facts of a canonical COMPOSITION value.
    pub(crate) fn of_canonical(canonical: &'a Value) -> Self {
        let composer = canonical.get("composer");
        let (facility, facility_path) = facility_of(canonical);
        let identified_performer = canonical
            .pointer("/context/participations")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|participation| participation.get("performer"))
            .any(|party| {
                party.get("_type").and_then(Value::as_str) == Some("PARTY_IDENTIFIED")
                    && identifiers(Some(party)).next().is_some()
            });
        Self {
            composer_type: composer
                .and_then(|c| c.get("_type"))
                .and_then(Value::as_str),
            composer_ref_type: canonical
                .pointer("/composer/external_ref/type")
                .and_then(Value::as_str),
            composer_identifiers: identifiers(composer).collect(),
            feeder_system_id: canonical
                .pointer("/feeder_audit/originating_system_audit/system_id")
                .and_then(Value::as_str),
            facility: identifiers(facility).collect(),
            facility_path,
            identified_performer,
        }
    }
}

/// A party proxy's RM type, `external_ref` type and identifiers.
fn proxy(party: &PartyProxy) -> (Option<&'static str>, Option<&str>, Vec<(&str, &str)>) {
    match party {
        PartyProxy::PartySelf(party) => (
            Some("PARTY_SELF"),
            party.external_ref.as_ref().map(|r| r.r#type.as_str()),
            Vec::new(),
        ),
        PartyProxy::PartyIdentified(PartyIdentified::PartyIdentified(party)) => (
            Some("PARTY_IDENTIFIED"),
            party.external_ref.as_ref().map(|r| r.r#type.as_str()),
            issued(party.identifiers.as_deref()),
        ),
        PartyProxy::PartyIdentified(PartyIdentified::PartyRelated(party)) => (
            Some("PARTY_RELATED"),
            party.external_ref.as_ref().map(|r| r.r#type.as_str()),
            issued(party.identifiers.as_deref()),
        ),
    }
}

/// The identifiers of a `PARTY_IDENTIFIED` of either form.
fn identified(party: &PartyIdentified) -> Vec<(&str, &str)> {
    match party {
        PartyIdentified::PartyIdentified(party) => issued(party.identifiers.as_deref()),
        PartyIdentified::PartyRelated(party) => issued(party.identifiers.as_deref()),
    }
}

/// Every `(issuer, id)` of `identifiers`, skipping one without an issuer.
fn issued(identifiers: Option<&[DvIdentifier]>) -> Vec<(&str, &str)> {
    identifiers
        .unwrap_or_default()
        .iter()
        .filter_map(|identifier| Some((identifier.issuer.as_deref()?, identifier.id.as_str())))
        .collect()
}

/// The facility `canonical` names, with its path: its
/// `EVENT_CONTEXT.health_care_facility`, or with no context its
/// `feeder_audit.originating_system_audit.location`.
pub(crate) fn facility_of(canonical: &Value) -> (Option<&Value>, &'static str) {
    match canonical.get("context").filter(|c| !c.is_null()) {
        Some(context) => (context.get("health_care_facility"), CONTEXT_FACILITY),
        None => (
            canonical.pointer("/feeder_audit/originating_system_audit/location"),
            FEEDER_FACILITY,
        ),
    }
}

/// Every `(issuer, id)` of `party`'s `identifiers`, skipping one without an
/// issuer.
pub(crate) fn identifiers(party: Option<&Value>) -> impl Iterator<Item = (&str, &str)> {
    party
        .and_then(|party| party.get("identifiers"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|identifier| {
            Some((
                identifier.get("issuer")?.as_str()?,
                identifier.get("id")?.as_str()?,
            ))
        })
}

#[cfg(test)]
mod tests {
    use openehr_rm::prelude::Composition;
    use serde_json::Value;

    use super::ContentFacts;

    /// The facts read off every typed COMPOSITION of the corpora are the facts
    /// read off its canonical value.
    #[test]
    fn typed_and_canonical_compositions_give_the_same_facts() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut stack: Vec<std::path::PathBuf> = ["corpus", "crates", "docs/specs/openehr/CNF"]
            .iter()
            .map(|d| root.join(d))
            .collect();
        let mut compared = 0_usize;
        let mut identified = 0_usize;
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for path in entries.flatten().map(|e| e.path()) {
                if path.is_dir() {
                    if path.file_name().is_some_and(|n| n != "target") {
                        stack.push(path);
                    }
                    continue;
                }
                let Some(typed) = path
                    .extension()
                    .filter(|e| *e == "json")
                    .and_then(|_| std::fs::read_to_string(&path).ok())
                    .filter(|text| text.contains("\"COMPOSITION\""))
                    .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                    .and_then(|v| openehr_its::json::from_canonical_value::<Composition>(&v).ok())
                else {
                    continue;
                };
                for variant in [typed.clone(), identified_variant(&typed)] {
                    let canonical = openehr_its::json::to_canonical_value(&variant);
                    let facts = ContentFacts::of_typed(&variant);
                    assert_eq!(
                        facts,
                        ContentFacts::of_canonical(&canonical),
                        "{}",
                        path.display()
                    );
                    identified += usize::from(facts.identified_performer);
                }
                compared += 1;
            }
        }
        assert!(compared >= 180, "{compared} compositions");
        assert!(identified >= 100, "{identified} identified performers");
    }

    /// `composition` with a composer, a facility, a feeder audit and a
    /// performer that each carry identifiers.
    fn identified_variant(composition: &Composition) -> Composition {
        let mut value = openehr_its::json::to_canonical_value(composition);
        let party = serde_json::json!({
            "_type": "PARTY_IDENTIFIED",
            "external_ref": { "_type": "PARTY_REF", "namespace": "demographic", "type": "ORGANISATION",
                "id": { "_type": "GENERIC_ID", "value": "org-1", "scheme": "local" } },
            "identifiers": [
                { "_type": "DV_IDENTIFIER", "issuer": "urn:oid:2.999.1", "id": "ward-7" },
                { "_type": "DV_IDENTIFIER", "id": "no-issuer" }
            ]
        });
        value["composer"] = party.clone();
        value["feeder_audit"] = serde_json::json!({ "_type": "FEEDER_AUDIT",
            "originating_system_audit": { "_type": "FEEDER_AUDIT_DETAILS", "system_id": "pump-1",
                "location": party.clone() } });
        if let Some(context) = value.get_mut("context").and_then(Value::as_object_mut) {
            context.insert("health_care_facility".to_owned(), party.clone());
            context.insert(
                "participations".to_owned(),
                serde_json::json!([{ "_type": "PARTICIPATION",
                    "function": { "_type": "DV_TEXT", "value": "performer" },
                    "performer": party }]),
            );
        }
        openehr_its::json::from_canonical_value(&value).expect("the variant reads as typed RM")
    }
}
