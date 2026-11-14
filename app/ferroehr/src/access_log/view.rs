// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The two views of an access-log record (#5130): the full view a reviewer
//! reads, and the subject view the subject of care and their representative
//! read under EHDS Art. 9(2) (`docs/law/eu/ehds/text.html`).
//!
//! The draft's semantics page (`docs/plans/access-log.md` §Patient access,
//! the proposal's §Views) fixes the subject view as a reduction of the entry:
//! it keeps who accessed, when, the action and outcome, the items and
//! categories, the purposes and the emergency mark (Art. 9(2)(a) to (c), Art.
//! 11(5)), and leaves out the credential, the delegation chain, the binding
//! outcome, the channel, the query text and its digest, the security labels,
//! `other_details`, each origin's endpoint and the link. An entry about data
//! held from the person under Art. 3(3) is left out of the subject view until
//! the hold ends. Where the controller's `ACCESS_POLICY` declares an Art.
//! 9(3) restriction, the subject view of an entry whose accessor is of a kind
//! it names withholds the accessor attributes it names (#5570); the full entry
//! is unchanged. No openEHR spec governs the access log: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "the subject view is the entry's ITS-JSON form with members left out, which no \
              generated type carries: the draft model holds the link as mandatory"
)]

use std::collections::{BTreeMap, BTreeSet};

use ehds_logging::withheld::Holds;
use jiff::Timestamp;
use serde_json::Value;

use openehr_base::v1_3::base_types::identification::object_id::ObjectId;
use openehr_base::v1_3::base_types::identification::object_ref::ObjectRef;
use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
use openehr_rm::draft::access_control::access_control::access_policy::AccessPolicy;
use openehr_rm::draft::access_control::terminology::{Term, Withheld};
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;

/// The members of the entry the subject view leaves out.
const ENTRY_WITHHELD: [&str; 4] = ["link", "channel", "security_labels", "other_details"];
/// The members of `ACCESSOR` the subject view leaves out.
const ACCESSOR_WITHHELD: [&str; 3] = ["credential", "delegation", "binding"];
/// The members of `ACCESS_QUERY` the subject view leaves out.
const QUERY_WITHHELD: [&str; 2] = ["text", "text_digest"];

/// An accessor kind as the draft `accessor kind` group names it: the
/// terminology id and the rubric.
type KindCode = (String, String);

/// The EHDS Art. 9(3) restrictions of the subject view a policy declares.
///
/// They are read from `ACCESS_POLICY.log_restrictions`: per accessor kind, the
/// `ACCESSOR` members the view withholds. With no policy or no restriction,
/// they withhold nothing beyond the subject view's own reduction.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Restrictions {
    withheld: BTreeMap<KindCode, BTreeSet<&'static str>>,
}

impl Restrictions {
    /// The restrictions `policy` declares; none without a policy.
    #[must_use]
    pub fn of(policy: Option<&AccessPolicy>) -> Self {
        let mut withheld: BTreeMap<KindCode, BTreeSet<&'static str>> = BTreeMap::new();
        let restrictions = policy
            .and_then(|policy| policy.log_restrictions.as_deref())
            .unwrap_or_default();
        for restriction in restrictions {
            // NOTE: the policy write refused a code outside `access log withheld` (`Withheld_valid`).
            let members = restriction
                .withheld
                .iter()
                .filter_map(|coded| Withheld::read_coded(coded).ok())
                .map(Withheld::member);
            let members: BTreeSet<&'static str> = members.collect();
            for kind in &restriction.accessor_kinds {
                withheld
                    .entry(kind_code(kind))
                    .or_default()
                    .extend(members.iter().copied());
            }
        }
        Self { withheld }
    }

    /// The `ACCESSOR` members withheld from an accessor of `kind`.
    fn members(&self, kind: &DvCodedText) -> impl Iterator<Item = &'static str> + '_ {
        self.withheld
            .get(&kind_code(kind))
            .into_iter()
            .flatten()
            .copied()
    }
}

/// The terminology id and rubric of `kind`.
fn kind_code(kind: &DvCodedText) -> KindCode {
    // NOTE: the draft access log's terminology invariants, a group's rubric names its term until
    // the terminology editors assign codes (`openehr_rm::draft::access_log::invariants`).
    (
        kind.defining_code.terminology_id.value.clone(),
        kind.value.clone(),
    )
}

/// The subject view of `entry` under `restrictions`.
///
/// It is the entry's ITS-JSON form without what serves review and integrity
/// alone, and without the accessor members `restrictions` withholds from an
/// accessor of the entry's kind (EHDS Art. 9(3)).
///
/// # Errors
/// The serialisation error when the entry has no JSON form.
pub fn subject_view(
    entry: &AccessLogEntry,
    restrictions: &Restrictions,
) -> Result<Value, serde_json::Error> {
    let mut value = serde_json::to_value(entry)?;
    remove(&mut value, &ENTRY_WITHHELD);
    if let Some(accessor) = value.get_mut("accessor") {
        remove(accessor, &ACCESSOR_WITHHELD);
        let restricted: Vec<&str> = restrictions.members(&entry.accessor.kind).collect();
        remove(accessor, &restricted);
    }
    if let Some(query) = value.get_mut("query") {
        remove(query, &QUERY_WITHHELD);
    }
    let origins = value
        .get_mut("items")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get_mut("origins").and_then(Value::as_array_mut))
        .flatten();
    for origin in origins {
        remove(origin, &["endpoint"]);
    }
    Ok(value)
}

/// Whether `entry` names an object `holds` withholds from the person at
/// `now` (EHDS Art. 3(3)), by its versioned object or by a version of it.
#[must_use]
pub fn withheld(entry: &AccessLogEntry, holds: &Holds, now: Timestamp) -> bool {
    entry
        .items
        .as_deref()
        .unwrap_or_default()
        .iter()
        .flat_map(|item| {
            let object = item.object.as_ref().and_then(object_id);
            let version = item.version_id.as_ref().map(ObjectVersionId::value);
            [object, version]
        })
        .flatten()
        .any(|id| holds.withholds(id, now))
}

/// The id an `OBJECT_REF` an item names carries, as the entry writer writes
/// one: a `HIER_OBJECT_ID` or a local `GENERIC_ID`.
fn object_id(object: &ObjectRef) -> Option<&str> {
    let ObjectRef::ObjectRef(data) = object else {
        return None;
    };
    match &data.id {
        ObjectId::HierObjectId(id) => Some(id.value()),
        ObjectId::GenericId(id) => Some(id.value.as_str()),
        _ => None,
    }
}

/// `value` without the members `names`.
fn remove(value: &mut Value, names: &[&str]) {
    if let Some(members) = value.as_object_mut() {
        for name in names {
            members.remove(*name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shared entry fixture: a professional's read, `person` its
    /// accessor kind.
    fn entry() -> AccessLogEntry {
        serde_json::from_str(include_str!(
            "../../../../crates/openehr-rm/tests/fixtures/draft/access_log_entry.json"
        ))
        .expect("the fixture reads")
    }

    /// A policy whose one restriction withholds `withheld` from accessors of
    /// `kinds`.
    fn policy(kinds: &[&str], withheld: &[&str]) -> AccessPolicy {
        let coded = |terminology: &str, code: &str| {
            serde_json::json!({ "_type": "DV_CODED_TEXT", "value": code,
                "defining_code": { "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": terminology },
                    "code_string": code } })
        };
        let control = openehr_rm::draft::access_control::terminology::TERMINOLOGY_ID_ACCESS_CONTROL;
        serde_json::from_value(serde_json::json!({
            "_type": "ACCESS_POLICY",
            "name": { "_type": "DV_TEXT", "value": "Policy" },
            "archetype_node_id": "openEHR-EHR-ACCESS_POLICY.controller.v1",
            "controller": { "_type": "PARTY_REF", "namespace": "demographic",
                "type": "ORGANISATION",
                "id": { "_type": "HIER_OBJECT_ID", "value": "2c3d4e5f-6a7b-4c8d-9e0f-a1b2c3d4e5f7" } },
            "care_relationship_required": false,
            "purpose_required": false,
            "log_restrictions": [{ "_type": "ACCESS_LOG_RESTRICTION",
                "name": { "_type": "DV_TEXT", "value": "Restriction" },
                "archetype_node_id": "id11",
                "accessor_kinds": kinds.iter().map(|kind| coded("openehr", kind)).collect::<Vec<_>>(),
                "withheld": withheld.iter().map(|code| coded(control, code)).collect::<Vec<_>>(),
                "legal_grounds": [coded("urn:example:national-law", "art-1")] }]
        }))
        .expect("the policy reads")
    }

    #[test]
    fn a_declared_restriction_withholds_its_members_for_its_accessor_kind() {
        let restrictions = Restrictions::of(Some(&policy(
            &["person"],
            &["accessor_party", "accessor_provider"],
        )));
        let view = subject_view(&entry(), &restrictions).expect("the view");
        assert!(view["accessor"].get("party").is_none(), "{view}");
        assert!(view["accessor"].get("provider").is_none(), "{view}");
        assert_eq!(view["accessor"]["kind"]["value"], "person", "{view}");
        assert!(view.get("time_recorded").is_some(), "{view}");
    }

    #[test]
    fn a_restriction_of_another_accessor_kind_changes_nothing() {
        let unrestricted = subject_view(&entry(), &Restrictions::default()).expect("the view");
        let restrictions = Restrictions::of(Some(&policy(
            &["external person", "organisation"],
            &["accessor_party"],
        )));
        let view = subject_view(&entry(), &restrictions).expect("the view");
        assert_eq!(view, unrestricted);
        assert!(view["accessor"].get("party").is_some(), "{view}");
    }

    #[test]
    fn no_policy_or_no_restriction_withholds_nothing_more() {
        assert_eq!(Restrictions::of(None), Restrictions::default());
        let mut without = policy(&["person"], &["accessor_party"]);
        without.log_restrictions = None;
        assert_eq!(Restrictions::of(Some(&without)), Restrictions::default());
        let view = subject_view(&entry(), &Restrictions::default()).expect("the view");
        assert!(view["accessor"].get("party").is_some(), "{view}");
        assert!(view["accessor"].get("provider").is_some(), "{view}");
        assert!(view["accessor"].get("credential").is_none(), "{view}");
    }
}
