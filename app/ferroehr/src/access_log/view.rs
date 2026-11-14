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
//! the hold ends. No openEHR spec governs the access log: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "the subject view is the entry's ITS-JSON form with members left out, which no \
              generated type carries: the draft model holds the link as mandatory"
)]

use ehds_logging::withheld::Holds;
use jiff::Timestamp;
use serde_json::Value;

use crate::ids::EhrId;
use openehr_base::v1_3::base_types::identification::object_id::ObjectId;
use openehr_base::v1_3::base_types::identification::object_ref::ObjectRef;
use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;
use openehr_rm::draft::access_log::access_log::access_log_record::AccessLogRecord;

/// The members of the entry the subject view leaves out.
const ENTRY_WITHHELD: [&str; 4] = ["link", "channel", "security_labels", "other_details"];
/// The members of `ACCESSOR` the subject view leaves out.
const ACCESSOR_WITHHELD: [&str; 3] = ["credential", "delegation", "binding"];
/// The members of `ACCESS_QUERY` the subject view leaves out.
const QUERY_WITHHELD: [&str; 2] = ["text", "text_digest"];
/// The members of each item's `ACCESS_ORIGIN` the subject view leaves out.
const ORIGIN_WITHHELD: [&str; 1] = ["endpoint"];

/// The refusal of a caller who is neither the person nor a reviewer.
pub const LOG_REFUSED: &str = "the access log is read by its subject, their representative, or a \
                               reviewer the admin class admits";

/// Who reads an EHR's access log, which decides what of it they read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogReader {
    /// The subject of care or their representative, under EHDS Art. 9(2).
    Person,
    /// A reviewer of the log (EHDS Annex II 3.3).
    Reviewer,
}

/// What a caller may read of the access log, decided from who they are before
/// any log is named (#5131): the same rules as the log's ITS-REST resources.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LogStanding {
    /// Neither the person nor a reviewer: the log is refused.
    #[default]
    Refused,
    /// The subject of care or their representative, confined to this EHR.
    Person(EhrId),
    /// A reviewer, who reads the whole log in the full view.
    Reviewer,
}

/// `record` as `reader` reads it: the full ITS-JSON record for a reviewer,
/// the subject view of an entry for the person, and `None` for a tombstone the
/// person is never served.
///
/// # Errors
/// The serialisation error when the record has no JSON form.
pub fn view_of(
    record: &AccessLogRecord,
    reader: LogReader,
) -> Result<Option<Value>, serde_json::Error> {
    match (record, reader) {
        (_, LogReader::Reviewer) => serde_json::to_value(record).map(Some),
        (AccessLogRecord::AccessLogEntry(entry), LogReader::Person) => {
            subject_view(entry).map(Some)
        }
        (AccessLogRecord::AccessLogTombstone(_), LogReader::Person) => Ok(None),
    }
}

/// Whether the subject view leaves out what the attribute path `path` of an
/// entry reaches, so a query of the person can neither read nor test it.
#[must_use]
pub fn subject_withholds(path: &[&str]) -> bool {
    match path {
        [member, ..] if ENTRY_WITHHELD.contains(member) => true,
        ["accessor", member, ..] => ACCESSOR_WITHHELD.contains(member),
        ["query", member, ..] => QUERY_WITHHELD.contains(member),
        ["items", "origins", member, ..] => ORIGIN_WITHHELD.contains(member),
        _ => false,
    }
}

/// The subject view of `entry`: its ITS-JSON form without what serves review
/// and integrity alone.
///
/// # Errors
/// The serialisation error when the entry has no JSON form.
pub fn subject_view(entry: &AccessLogEntry) -> Result<Value, serde_json::Error> {
    let mut value = serde_json::to_value(entry)?;
    remove(&mut value, &ENTRY_WITHHELD);
    if let Some(accessor) = value.get_mut("accessor") {
        remove(accessor, &ACCESSOR_WITHHELD);
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
        remove(origin, &ORIGIN_WITHHELD);
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

    /// The shared fixture of the draft's entry.
    const ENTRY: &str =
        include_str!("../../../../crates/openehr-rm/tests/fixtures/draft/access_log_entry.json");

    /// The members of `value` at `prefix`, each as its path.
    fn member_paths(value: &Value, prefix: &[&'static str]) -> Vec<Vec<String>> {
        value
            .as_object()
            .into_iter()
            .flat_map(|members| members.keys())
            .map(|name| {
                let mut path: Vec<String> = prefix.iter().map(|step| (*step).to_owned()).collect();
                path.push(name.clone());
                path
            })
            .collect()
    }

    #[test]
    fn a_path_is_withheld_exactly_when_the_subject_view_leaves_it_out() {
        let entry: AccessLogEntry =
            openehr_its::json::from_canonical_json(ENTRY).expect("the fixture reads");
        let full = serde_json::to_value(&entry).expect("a JSON form");
        let subject = subject_view(&entry).expect("a subject view");
        let mut paths = member_paths(&full, &[]);
        paths.extend(member_paths(&full["accessor"], &["accessor"]));
        assert!(paths.len() > 10, "the fixture covers the entry: {paths:?}");
        for path in paths {
            let steps: Vec<&str> = path.iter().map(String::as_str).collect();
            let shown = steps
                .iter()
                .try_fold(&subject, |value, step| value.get(*step))
                .is_some();
            assert_eq!(subject_withholds(&steps), !shown, "{steps:?}");
        }
    }
}
