// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The two operational queues an operator tends.
//!
//! They are the delivery of audit records to their sinks
//! (`{ext}/admin/audit-delivery`) and the refused messages of the patient
//! identity feed (`{ext}/admin/pmir/messages`).
//!
//! A queue whose route answers `404` is not served by the CDR. No openEHR
//! spec governs these routes: the CDR's own extension.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST, not the CDR internal seams \
              (#1694)"
)]

use serde::Deserialize;

use crate::error::ViewerError;
use crate::locale::server::ServerMsg;
use crate::locale::tr_served;

/// One sink's cursor of one log.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Cursor {
    /// The sink: `fhir_feed` or `syslog`.
    pub sink: String,
    /// The log: `system` or `access_log`.
    pub chain: String,
    /// The id at or below which every record was delivered or parked.
    pub through_id: i64,
    /// The records past it the sink has still to deliver.
    pub behind: i64,
    /// Whether the configuration names the sink.
    pub configured: bool,
}

/// One record a sink parked.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Parked {
    /// The sink that parked it.
    pub sink: String,
    /// Its log.
    pub chain: String,
    /// Its id in its log.
    pub record_id: i64,
    /// Why it was parked.
    pub reason: String,
    /// When it was parked.
    #[serde(rename = "parked_at")]
    pub at: String,
    /// Whether an operator asked to retry it.
    #[serde(default)]
    pub retry: bool,
}

/// The delivery of audit records to their sinks.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Delivery {
    /// Each cursor, configured or not.
    #[serde(default)]
    pub cursors: Vec<Cursor>,
    /// Each parked record, oldest first.
    #[serde(default)]
    pub parked: Vec<Parked>,
}

/// A parked record as the retry and the dismiss name it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParkedRef {
    /// Its sink.
    pub sink: String,
    /// Its log.
    pub chain: String,
    /// Its id, as typed; a value that is no number goes as it stands and
    /// the CDR refuses it.
    pub record_id: String,
}

impl ParkedRef {
    /// The body the retry and the dismiss take.
    #[must_use]
    pub fn body(&self) -> String {
        let id = self.record_id.parse::<i64>().map_or_else(
            |_| serde_json::Value::from(self.record_id.as_str()),
            serde_json::Value::from,
        );
        serde_json::json!({ "sink": self.sink, "chain": self.chain, "record_id": id }).to_string()
    }
}

/// The delivery view's path under the extensions root.
pub const DELIVERY: &str = "admin/audit-delivery";

/// Reads the admin queue at `path` ([`DELIVERY`] reads a [`Delivery`]):
/// `None` when the CDR serves it not.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)), the CDR's words for
/// a local audit store that is off (`409`) and an admin group that is off
/// (`405`); [`ViewerError::Failed`] for an answer that does not read.
pub async fn read<T: serde::de::DeserializeOwned>(path: &str) -> Result<Option<T>, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.extensions_v1(path);
    let response = cdr.get(&url).send().await?;
    if response.is(http::StatusCode::NOT_FOUND) {
        return Ok(None);
    }
    let body = response.ok()?.body;
    serde_json::from_str(&body)
        .map(Some)
        .map_err(|error| ViewerError::failed(tr_served(ServerMsg::QueueJson), error))
}

/// Posts `body` to the admin route `path`.
async fn post(path: &str, body: String) -> Result<(), ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.extensions_v1(path);
    cdr.post(&url).body("application/json", body).ok().await?;
    Ok(())
}

/// Asks the sink to offer the parked record `parked` again.
///
/// # Errors
/// The errors of a CDR call, the CDR's words for a record that is not
/// parked or a body it refuses.
pub async fn retry_parked(parked: &ParkedRef) -> Result<(), ViewerError> {
    post("admin/audit-delivery/retry", parked.body()).await
}

/// Removes the parked record `parked` for good.
///
/// # Errors
/// The errors of [`retry_parked`].
pub async fn dismiss_parked(parked: &ParkedRef) -> Result<(), ViewerError> {
    post("admin/audit-delivery/dismiss", parked.body()).await
}

/// Removes the cursors and parked records of the sink `sink`, which the
/// configuration no longer names.
///
/// # Errors
/// The errors of [`retry_parked`], and the CDR's words for a sink the
/// configuration still names.
pub async fn forget_sink(sink: &str) -> Result<(), ViewerError> {
    post(
        "admin/audit-delivery/forget",
        serde_json::json!({ "sink": sink }).to_string(),
    )
    .await
}

/// The identifiers and record subjects a refused message keeps split from
/// one `PERSON`, as counts.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Split {
    /// The `PERSON`.
    pub person: String,
    /// How many identifiers.
    #[serde(default)]
    pub identifiers: u64,
    /// How many record subjects.
    #[serde(default)]
    pub subjects: u64,
}

/// One refused message of the identity feed.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Message {
    /// Its `MessageHeader.id`.
    #[serde(rename = "message_id")]
    pub id: String,
    /// The refusal code.
    pub reason: String,
    /// When it arrived.
    #[serde(default)]
    pub received_at: String,
    /// When it was refused.
    #[serde(default)]
    pub refused_at: String,
    /// How often it was applied.
    #[serde(default)]
    pub attempts: u64,
    /// The splits it keeps.
    #[serde(default)]
    pub splits: Vec<Split>,
}

/// The refused messages, oldest first.
#[derive(Debug, Deserialize)]
struct Messages {
    /// The messages.
    messages: Vec<Message>,
}

/// The refused messages of the identity feed, oldest first; `None` when the
/// CDR runs no identity feed.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)), the CDR's words for
/// an admin group that is off (`405`); [`ViewerError::Failed`] for an answer
/// that does not read.
pub async fn messages() -> Result<Option<Vec<Message>>, ViewerError> {
    Ok(read::<Messages>("admin/pmir/messages")
        .await?
        .map(|listed| listed.messages))
}

/// The path of the action `action` on the message `message_id`.
#[must_use]
pub fn message_path(message_id: &str, action: &str) -> String {
    format!(
        "admin/pmir/messages/{}/{action}",
        urlencoding::encode(message_id)
    )
}

/// Puts the refused message `message_id` back at its arrival position.
///
/// # Errors
/// The errors of a CDR call, the CDR's words for an unknown message (`404`)
/// and one that is not refused or was overtaken (`409`).
pub async fn retry_message(message_id: &str) -> Result<(), ViewerError> {
    post(&message_path(message_id, "retry"), String::new()).await
}

/// Settles the refused message `message_id` with `decision` (`discard` or
/// `lift-splits`) and `reason`.
///
/// # Errors
/// The errors of [`retry_message`], and the CDR's words for a decision or a
/// reason it refuses (`400`).
pub async fn settle_message(
    message_id: &str,
    decision: &str,
    reason: &str,
) -> Result<(), ViewerError> {
    post(
        &message_path(message_id, "settle"),
        serde_json::json!({ "decision": decision, "reason": reason }).to_string(),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::{Delivery, ParkedRef, message_path};

    /// The documented delivery view reads.
    #[test]
    fn the_delivery_view_reads() {
        let view: Delivery = serde_json::from_str(
            r#"{"cursors":[{"sink":"fhir_feed","chain":"system","through_id":42,"behind":0,"configured":true}],"parked":[{"sink":"syslog","chain":"access_log","record_id":7,"reason":"refused","parked_at":"2026-11-16T09:15:01Z","retry":false}]}"#,
        )
        .expect("the view reads");
        assert_eq!(view.parked[0].record_id, 7);
    }

    /// A parked record goes by its sink, its log and its numeric id.
    #[test]
    fn a_parked_record_names_itself() {
        let parked = ParkedRef {
            sink: "syslog".to_owned(),
            chain: "access_log".to_owned(),
            record_id: "7".to_owned(),
        };
        assert_eq!(
            parked.body(),
            r#"{"sink":"syslog","chain":"access_log","record_id":7}"#
        );
        assert_eq!(
            message_path("a/b", "retry"),
            "admin/pmir/messages/a%2Fb/retry"
        );
    }
}
