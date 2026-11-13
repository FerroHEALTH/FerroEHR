// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The class invariants of the draft access log, checked over the generated
//! types of [`crate::draft::access_log`].
//!
//! Each invariant is the BMM's own, named as the BMM names it
//! (`tools/openehr-codegen/draft/bmm/openehr_access_log_0.1.0.bmm.json`). The
//! draft is FerroEHR's change proposal and no openEHR release, so these checks
//! are our own design over our own model (#5124).
//!
//! The four terminology invariants (`Action_valid`, `Kind_valid` of `ACCESSOR`
//! and of `ACCESS_AUTHORISATION`, `Result_valid`) ask for a code of a new
//! openEHR Terminology group. The draft defines each group by its rubrics and
//! leaves the codes to the terminology editors, so a coded value holds when its
//! terminology is `openehr` and its value is one of the group's rubrics.

use crate::draft::access_log::access_log::access_authorisation::AccessAuthorisation;
use crate::draft::access_log::access_log::access_credential::AccessCredential;
use crate::draft::access_log::access_log::access_log_checkpoint::AccessLogCheckpoint;
use crate::draft::access_log::access_log::access_log_entry::AccessLogEntry;
use crate::draft::access_log::access_log::access_log_link::AccessLogLink;
use crate::draft::access_log::access_log::access_log_record::AccessLogRecord;
use crate::draft::access_log::access_log::access_log_tombstone::AccessLogTombstone;
use crate::draft::access_log::access_log::access_origin::AccessOrigin;
use crate::draft::access_log::access_log::access_outcome::AccessOutcome;
use crate::draft::access_log::access_log::access_query::AccessQuery;
use crate::draft::access_log::access_log::accessed_item::AccessedItem;
use crate::draft::access_log::access_log::accessor::Accessor;
use crate::draft::invariants::{Check, Violations};
use crate::v1_2::data_types::text::dv_coded_text::DvCodedText;

/// The rubrics of the draft `access action` group.
const ACCESS_ACTION: &[&str] = &[
    "read", "query", "create", "update", "delete", "export", "import",
];
/// The rubrics of the draft `access outcome` group.
const ACCESS_OUTCOME: &[&str] = &["granted", "partially granted", "refused", "failed"];
/// The rubrics of the draft `accessor kind` group.
const ACCESSOR_KIND: &[&str] = &[
    "person",
    "delegated person",
    "subject of care",
    "representative",
    "external person",
    "organisation",
    "system",
];
/// The rubrics of the draft `access authorisation` group.
const ACCESS_AUTHORISATION: &[&str] = &[
    "role",
    "treatment relationship",
    "agreement",
    "consent",
    "permit",
    "emergency",
    "subject of care",
    "representation",
    "legal obligation",
];

/// Whether `coded` is a term of the draft openEHR group whose rubrics are
/// `group`.
fn in_group(coded: &DvCodedText, group: &[&str]) -> bool {
    coded.defining_code.terminology_id.value == "openehr" && group.contains(&coded.value.as_str())
}

/// Checks every invariant of an `ACCESS_LOG_ENTRY` or an
/// `ACCESS_LOG_TOMBSTONE` and of the objects it holds.
///
/// # Errors
/// [`Violations`] naming each invariant the record breaks.
pub fn check_record(record: &AccessLogRecord) -> Result<(), Violations> {
    match record {
        AccessLogRecord::AccessLogEntry(entry) => check_entry(entry),
        AccessLogRecord::AccessLogTombstone(tombstone) => check_tombstone(tombstone),
    }
}

/// Checks every invariant of an `ACCESS_LOG_ENTRY` and of the objects it
/// holds.
///
/// # Errors
/// [`Violations`] naming each invariant the entry breaks.
pub fn check_entry(entry: &AccessLogEntry) -> Result<(), Violations> {
    let mut check = Check::default();
    entry_rules(&mut check, entry);
    check.finish()
}

/// Checks every invariant of an `ACCESS_LOG_TOMBSTONE` and of its link.
///
/// # Errors
/// [`Violations`] naming each invariant the tombstone breaks.
pub fn check_tombstone(tombstone: &AccessLogTombstone) -> Result<(), Violations> {
    let mut check = Check::default();
    check.hold(
        !tombstone.system_id.is_empty(),
        "ACCESS_LOG_RECORD",
        "System_id_valid",
    );
    link_rules(&mut check, &tombstone.link);
    check.finish()
}

/// Checks every invariant of an `ACCESS_LOG_CHECKPOINT`.
///
/// # Errors
/// [`Violations`] naming each invariant the checkpoint breaks.
pub fn check_checkpoint(checkpoint: &AccessLogCheckpoint) -> Result<(), Violations> {
    let mut check = Check::default();
    check.hold(
        checkpoint.sequence >= 1,
        "ACCESS_LOG_CHECKPOINT",
        "Sequence_valid",
    );
    check.hold(
        !checkpoint.proof.is_empty(),
        "ACCESS_LOG_CHECKPOINT",
        "Proof_valid",
    );
    check.finish()
}

fn entry_rules(check: &mut Check, entry: &AccessLogEntry) {
    check.hold(
        !entry.system_id.is_empty(),
        "ACCESS_LOG_RECORD",
        "System_id_valid",
    );
    link_rules(check, &entry.link);
    check.hold(
        in_group(&entry.action, ACCESS_ACTION),
        "ACCESS_LOG_ENTRY",
        "Action_valid",
    );
    let items = entry.items.as_deref().unwrap_or_default();
    check.hold(
        items.is_empty() || entry.ehr_id.is_some(),
        "ACCESS_LOG_ENTRY",
        "Items_need_ehr",
    );
    outcome_rules(check, &entry.outcome);
    accessor_rules(check, &entry.accessor);
    for item in items {
        item_rules(check, item);
    }
    if let Some(query) = &entry.query {
        query_rules(check, query);
    }
    for authorisation in entry.authorisations.as_deref().unwrap_or_default() {
        authorisation_rules(check, authorisation);
    }
}

fn link_rules(check: &mut Check, link: &AccessLogLink) {
    check.hold(link.sequence >= 1, "ACCESS_LOG_LINK", "Sequence_valid");
    check.hold(
        !link.algorithm.is_empty(),
        "ACCESS_LOG_LINK",
        "Algorithm_valid",
    );
    check.hold(
        !link.previous_digest.is_empty()
            && !link.content_digest.is_empty()
            && !link.digest.is_empty(),
        "ACCESS_LOG_LINK",
        "Digests_valid",
    );
}

fn outcome_rules(check: &mut Check, outcome: &AccessOutcome) {
    check.hold(
        in_group(&outcome.result, ACCESS_OUTCOME),
        "ACCESS_OUTCOME",
        "Result_valid",
    );
    check.hold(
        outcome.delivered.is_none_or(|delivered| delivered >= 0),
        "ACCESS_OUTCOME",
        "Delivered_valid",
    );
}

fn accessor_rules(check: &mut Check, accessor: &Accessor) {
    check.hold(
        in_group(&accessor.kind, ACCESSOR_KIND),
        "ACCESSOR",
        "Kind_valid",
    );
    let credentials = accessor
        .credential
        .iter()
        .chain(accessor.delegation.as_deref().unwrap_or_default());
    for credential in credentials {
        credential_rules(check, credential);
    }
}

fn credential_rules(check: &mut Check, credential: &AccessCredential) {
    check.hold(
        !credential.issuer.is_empty(),
        "ACCESS_CREDENTIAL",
        "Issuer_valid",
    );
    check.hold(
        !credential.subject.is_empty(),
        "ACCESS_CREDENTIAL",
        "Subject_valid",
    );
}

fn item_rules(check: &mut Check, item: &AccessedItem) {
    check.hold(
        item.object.is_some()
            || item.version_id.is_some()
            || !item.categories.as_deref().unwrap_or_default().is_empty(),
        "ACCESSED_ITEM",
        "Item_named",
    );
    for origin in item.origins.as_deref().unwrap_or_default() {
        origin_rules(check, origin);
    }
}

fn origin_rules(check: &mut Check, origin: &AccessOrigin) {
    check.hold(
        !origin.system_id.is_empty(),
        "ACCESS_ORIGIN",
        "System_id_valid",
    );
}

fn query_rules(check: &mut Check, query: &AccessQuery) {
    check.hold(
        query.stored_query.is_some() || query.text_digest.is_some(),
        "ACCESS_QUERY",
        "Query_named",
    );
    check.hold(
        query.rows.is_none_or(|rows| rows >= 0),
        "ACCESS_QUERY",
        "Rows_valid",
    );
}

fn authorisation_rules(check: &mut Check, authorisation: &AccessAuthorisation) {
    check.hold(
        in_group(&authorisation.kind, ACCESS_AUTHORISATION),
        "ACCESS_AUTHORISATION",
        "Kind_valid",
    );
}
