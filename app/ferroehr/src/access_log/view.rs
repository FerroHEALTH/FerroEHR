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

use openehr_base::v1_3::base_types::identification::object_id::ObjectId;
use openehr_base::v1_3::base_types::identification::object_ref::ObjectRef;
use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;

/// The members of the entry the subject view leaves out.
const ENTRY_WITHHELD: [&str; 4] = ["link", "channel", "security_labels", "other_details"];
/// The members of `ACCESSOR` the subject view leaves out.
const ACCESSOR_WITHHELD: [&str; 3] = ["credential", "delegation", "binding"];
/// The members of `ACCESS_QUERY` the subject view leaves out.
const QUERY_WITHHELD: [&str; 2] = ["text", "text_digest"];

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
