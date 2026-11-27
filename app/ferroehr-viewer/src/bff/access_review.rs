// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access log as a reviewer reads it: the records across EHRs, the
//! erasure of an entry, and the log's tamper evidence.
//!
//! FerroEHR serves these under `{ext}/admin/access-log` for a caller the
//! admin class admits: the records with their tombstones, `/erase`, `/seals`,
//! `/checkpoint` and `/verify` (book `audit.md` §Seals, checkpoints and the
//! transfer of a log). A finding of a verification is an answer, never a
//! failed request. No openEHR spec governs the viewer: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST, not the CDR internal seams \
              (#1694)"
)]

use serde_json::Value;

use crate::bff::access_log::{
    Coded, Entry, each, entry_of, json, party_words, push_days, push_filled, read_page, text_at,
};
use crate::error::ViewerError;

/// The filters of the review across EHRs, each as the CDR names it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReviewFilters {
    /// The EHR the entries are about.
    pub ehr_id: String,
    /// The id of the accessing party.
    pub party_id: String,
    /// The issuer of the accessor's credential.
    pub issuer: String,
    /// The subject of the accessor's credential.
    pub subject: String,
    /// The request the entries were written in.
    pub request_id: String,
    /// The first day selected, `YYYY-MM-DD`.
    pub from: String,
    /// The last day selected, `YYYY-MM-DD`.
    pub to: String,
    /// The outcome, the token `terminology_id|code`.
    pub outcome: String,
    /// `justification` or `emergency`.
    pub justification: String,
}

impl ReviewFilters {
    /// The query pairs of the filters a review sends, each day widened to
    /// the instants that bound it.
    #[must_use]
    pub fn query_pairs(&self) -> Vec<(&'static str, String)> {
        let mut pairs = Vec::new();
        push_filled(
            &mut pairs,
            [
                ("ehr_id", &self.ehr_id),
                ("party_id", &self.party_id),
                ("issuer", &self.issuer),
                ("subject", &self.subject),
                ("request_id", &self.request_id),
            ],
        );
        push_days(&mut pairs, &self.from, &self.to);
        push_filled(
            &mut pairs,
            [
                ("outcome", &self.outcome),
                ("justification", &self.justification),
            ],
        );
        pairs
    }
}

/// An `ACCESS_LOG_TOMBSTONE`: an entry removed on an order, its grounds kept.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tombstone {
    /// The uid of the entry it replaced.
    pub uid: String,
    /// When the entry was recorded.
    pub recorded: String,
    /// When it was removed.
    pub removed: String,
    /// The grounds of the removal.
    pub grounds: Vec<Coded>,
}

/// One record of the review: an entry, or the tombstone of a removed one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    /// An entry, with the EHR it is about when it names one.
    Entry(Box<Entry>, Option<String>),
    /// A tombstone.
    Tombstone(Tombstone),
}

/// The tombstone `node` as a page draws it.
fn tombstone_of(node: &Value) -> Tombstone {
    Tombstone {
        uid: text_at(node, "/uid/value"),
        recorded: text_at(node, "/time_recorded/value"),
        removed: text_at(node, "/time_removed/value"),
        grounds: each(node, "grounds")
            .filter_map(|ground| Coded::of(Some(ground)))
            .collect(),
    }
}

/// The record `node`, `None` when it carries no uid.
fn record_of(node: &Value) -> Option<Record> {
    if node.get("_type").and_then(Value::as_str) == Some("ACCESS_LOG_TOMBSTONE") {
        return Some(Record::Tombstone(tombstone_of(node)));
    }
    let ehr = text_at(node, "/ehr_id/value");
    entry_of(node).map(|entry| Record::Entry(Box::new(entry), (!ehr.is_empty()).then_some(ehr)))
}

/// The records across EHRs that `filters` select, newest first, one page at
/// `offset`, in the full view a reviewer reads.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)): the `403` of a caller
/// who is no reviewer, the `405` of a CDR whose admin group is off, the `409`
/// of a CDR whose audit store is off; [`ViewerError::Failed`] when the body
/// is not JSON.
pub async fn review(
    // The filters the review sends.
    filters: &ReviewFilters,
    // First record of the page.
    offset: u32,
    // How many records the page holds.
    fetch: u32,
) -> Result<Vec<Record>, ViewerError> {
    let doc = read_page("admin/access-log", filters.query_pairs(), offset, fetch).await?;
    Ok(each(&doc, "entries").filter_map(record_of).collect())
}

/// Replaces the entry `uid` with a tombstone on the coded `grounds`, and
/// answers the tombstone.
///
/// # Errors
/// The errors of a CDR call: the `400` of a body with no grounds, the `404`
/// of an unknown uid, the `409` of a tombstone already.
pub async fn erase(
    // The uid of the entry erased.
    uid: &str,
    // The grounds of the erasure, each a coded term.
    grounds: &[Coded],
) -> Result<Tombstone, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let grounds: Vec<Value> = grounds
        .iter()
        .map(|ground| {
            serde_json::json!({
                "_type": "DV_CODED_TEXT",
                "value": ground.value,
                "defining_code": {
                    "_type": "CODE_PHRASE",
                    "terminology_id": {"_type": "TERMINOLOGY_ID", "value": ground.terminology},
                    "code_string": ground.code
                }
            })
        })
        .collect();
    let body = serde_json::json!({ "uid": uid, "grounds": grounds }).to_string();
    let url = cdr.extensions_v1("admin/access-log/erase");
    let answer = cdr
        .post(&url)
        .body("application/json", body)
        .ok()
        .await?
        .body;
    let doc = json(&answer)?;
    Ok(tombstone_of(&doc))
}

/// The seal of one window of the log.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Seal {
    /// The window's number.
    pub window: i64,
    /// Its first entry id.
    pub first: i64,
    /// Its last entry id.
    pub last: i64,
    /// How many entries it seals.
    pub entries: i64,
    /// The digest of the seal before it.
    pub previous: String,
    /// Its digest.
    pub digest: String,
    /// Whether retention folded it.
    pub folded: bool,
}

/// The seals of the windows that cover the entry ids `from` to `to`.
///
/// # Errors
/// The errors of a CDR call: the `400` of a span the CDR does not take.
pub async fn seals(
    // The first entry id of the span.
    from: i64,
    // The last entry id of the span.
    to: i64,
) -> Result<Vec<Seal>, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = format!(
        "{}?from_id={from}&to_id={to}",
        cdr.extensions_v1("admin/access-log/seals")
    );
    let body = cdr.get(&url).ok().await?.body;
    let doc = json(&body)?;
    let number =
        |node: &Value, key: &str| node.get(key).and_then(Value::as_i64).unwrap_or_default();
    Ok(each(&doc, "seals")
        .map(|seal| Seal {
            window: number(seal, "window_no"),
            first: number(seal, "first_id"),
            last: number(seal, "last_id"),
            entries: number(seal, "entries"),
            previous: text_at(seal, "/previous_digest"),
            digest: text_at(seal, "/digest"),
            folded: seal
                .get("folded")
                .and_then(Value::as_bool)
                .unwrap_or_default(),
        })
        .collect())
}

/// A signed checkpoint of the log.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Checkpoint {
    /// The entry id it covers the log up to.
    pub sequence: i64,
    /// The digest it signs.
    pub digest: String,
    /// The signature's algorithm.
    pub algorithm: String,
    /// When it was sealed.
    pub sealed: String,
    /// Who signed it.
    pub signer: String,
    /// The signature.
    pub proof: String,
}

/// The checkpoint `node`.
fn checkpoint_of(node: &Value) -> Checkpoint {
    Checkpoint {
        sequence: node
            .get("sequence")
            .and_then(Value::as_i64)
            .unwrap_or_default(),
        digest: text_at(node, "/digest"),
        algorithm: text_at(node, "/algorithm"),
        sealed: text_at(node, "/time_sealed/value"),
        signer: party_words(node.get("signer")),
        proof: text_at(node, "/proof"),
    }
}

/// The newest stored checkpoint, `None` when none is stored.
///
/// # Errors
/// The errors of a CDR call other than the `404` of no checkpoint.
pub async fn newest_checkpoint() -> Result<Option<Checkpoint>, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.extensions_v1("admin/access-log/checkpoint");
    let response = cdr.get(&url).send().await?;
    if response.is(http::StatusCode::NOT_FOUND) {
        return Ok(None);
    }
    let body = response.ok()?.body;
    let doc = json(&body)?;
    Ok(Some(checkpoint_of(&doc)))
}

/// Signs a checkpoint of the head of the log now.
///
/// # Errors
/// The errors of a CDR call: the `404` of an empty log, the `409` of a
/// keyring with no key to sign with.
pub async fn sign_checkpoint() -> Result<Checkpoint, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.extensions_v1("admin/access-log/checkpoint");
    let body = cdr.post(&url).ok().await?.body;
    let doc = json(&body)?;
    Ok(checkpoint_of(&doc))
}

/// What a verification of the log found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Verification {
    /// Whether a checkpoint was there to verify against and nothing failed.
    pub intact: bool,
    /// The sequence of the checkpoint verified against.
    pub checkpoint: Option<i64>,
    /// Each finding, in the CDR's words.
    pub findings: Vec<String>,
}

/// The words of one finding the verification answered.
fn finding(node: &Value) -> String {
    if let Some(text) = node.as_str() {
        return text.to_owned();
    }
    let found = text_at(node, "/finding");
    let at = node
        .get("chain_seq")
        .or_else(|| node.get("checkpoint"))
        .and_then(Value::as_i64);
    let system = text_at(node, "/system_id");
    match (system.is_empty(), at) {
        (true, Some(at)) => format!("{at}: {found}"),
        (false, Some(at)) => format!("{system} {at}: {found}"),
        _ => found,
    }
}

/// Verifies the log against its newest checkpoint.
///
/// # Errors
/// The errors of a CDR call; [`ViewerError::Failed`] when the body is not
/// JSON.
pub async fn verify() -> Result<Verification, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.extensions_v1("admin/access-log/verify");
    let body = cdr.post(&url).ok().await?.body;
    let doc = json(&body)?;
    Ok(Verification {
        intact: doc
            .get("intact")
            .and_then(Value::as_bool)
            .unwrap_or_default(),
        checkpoint: doc.pointer("/checkpoint/sequence").and_then(Value::as_i64),
        findings: ["findings", "chain", "imported"]
            .into_iter()
            .flat_map(|key| each(&doc, key))
            .map(finding)
            .collect(),
    })
}

/// The path of the export of the EHR `ehr_id`'s log.
#[must_use]
pub fn export_path(ehr_id: &str) -> String {
    format!(
        "admin/access-log/export?ehr_id={}",
        urlencoding::encode(ehr_id)
    )
}

#[cfg(test)]
mod tests {
    use super::{Record, ReviewFilters, finding, record_of};

    #[test]
    fn a_review_reads_a_tombstone_and_an_entry_with_its_ehr() {
        let tombstone = serde_json::json!({
            "_type": "ACCESS_LOG_TOMBSTONE",
            "uid": {"value": "t"},
            "time_removed": {"value": "2026-11-26T12:00:00Z"},
            "grounds": [{"value": "court order", "defining_code": {"code_string": "art-x"}}]
        });
        let Some(Record::Tombstone(read)) = record_of(&tombstone) else {
            panic!("a tombstone reads as one");
        };
        assert_eq!(read.grounds[0].words(), "court order");
        let entry = serde_json::json!({"uid": {"value": "e"}, "ehr_id": {"value": "x"}});
        assert!(matches!(record_of(&entry), Some(Record::Entry(_, Some(ehr))) if ehr == "x"));
        let pairs = ReviewFilters {
            ehr_id: "x".to_owned(),
            justification: "emergency".to_owned(),
            ..ReviewFilters::default()
        }
        .query_pairs();
        assert_eq!(
            pairs,
            vec![
                ("ehr_id", "x".to_owned()),
                ("justification", "emergency".to_owned())
            ]
        );
    }

    #[test]
    fn a_finding_names_where_it_is() {
        assert_eq!(finding(&serde_json::json!("not signed")), "not signed");
        assert_eq!(
            finding(&serde_json::json!({"chain_seq": 4, "finding": "modified"})),
            "4: modified"
        );
        assert_eq!(
            finding(&serde_json::json!({"system_id": "a", "checkpoint": 9, "finding": "broken"})),
            "a 9: broken"
        );
    }
}
