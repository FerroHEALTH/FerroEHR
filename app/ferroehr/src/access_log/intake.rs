// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The intake of the access log: each entry written before the response that
//! reports its access, unchained, with every column the chain keeps (#5747).
//!
//! The writer supplies the row and shared parts of the entry's canonical
//! form, the content digest over the whole form, the columns the log derives
//! from the content and, when the entry is indexed, its ITI-81 index row, so
//! the chainer only computes the links
//! (`migrations/audit/0041_audit_intake.sql`). Each derived column is read
//! from the entry's ITS-JSON value at the path the database's own derive
//! reads (`audit.access_log_record_derive`), and the text is cast by the
//! database as that function casts it, so a chained record compares equal to
//! its derivation, which the verifier checks. No openEHR spec governs audit
//! storage: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "the stored parts and the derived columns are read from the entry's canonical \
              ITS-JSON value (#1694 boundary class: canonical form)"
)]

use std::collections::BTreeMap;

use jiff_sqlx::Timestamp;
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;
use openehr_rm::draft::access_log::access_log::access_log_link::AccessLogLink;
use openehr_rm::draft::access_log::invariants::check_entry;
use openehr_rm::v1_2::common::change_control::canonical_form_impl::canonical_json;
use serde_json::Value;
use sqlx::PgConnection;

use crate::access_log::chain::ALGORITHM;
use crate::access_log::store::{AccessLogError, canonical_form};
use crate::access_log::stored_form;
use crate::digest::sha256_hex;

/// The member of an entry's ITS-JSON form its canonical form leaves out.
const LINK: &str = "link";

/// The ITI-81 index row of an entry: what the search filters on and the
/// render policy its projection is written under.
#[derive(Debug, Clone)]
pub struct IndexRow {
    /// When the access happened.
    pub recorded_at: jiff::Timestamp,
    /// The DICOM `EventActionCode`.
    pub action: &'static str,
    /// The DICOM `EventOutcomeIndicator`.
    pub outcome: i16,
    /// The caller, when one is named.
    pub principal: Option<String>,
    /// The subject of care of the entry's EHR.
    pub patient_id: Option<String>,
    /// The object the access reached.
    pub resource_id: Option<String>,
    /// The request the access belongs to.
    pub request_id: Option<String>,
    /// The caller's network address.
    pub client_ip: Option<String>,
    /// The EHR of a classified access.
    pub ehr_id: Option<String>,
    /// The ids of the search tokens of the projection, as the text of an
    /// `integer[]`.
    pub search_tokens: String,
    /// The digest of the render policy.
    pub render_policy: Vec<u8>,
}

/// An entry as the intake keeps it.
#[derive(Debug)]
pub struct IntakeEntry {
    row: String,
    shared: Option<(String, String)>,
    content_digest: String,
    derived: Derived,
    index: Option<IndexRow>,
}

/// The columns the log derives from an entry's content, as text the database
/// casts.
#[derive(Debug, Default)]
struct Derived {
    record_type: Option<String>,
    uid: Option<String>,
    time_recorded: Option<String>,
    ehr_id: Option<String>,
    action_terminology: Option<String>,
    action_code: Option<String>,
    result_terminology: Option<String>,
    result_code: Option<String>,
    party_id: Option<String>,
    retain_until: Option<String>,
}

impl IntakeEntry {
    /// `entry` as the intake keeps it, with `index` when it is indexed.
    ///
    /// The entry's link is the chain's to decide, so it is replaced by the
    /// prospective link over the entry's content digest, and the entry is
    /// checked against the draft's invariants with it: an entry that breaks
    /// one never reaches the intake.
    ///
    /// # Errors
    /// [`AccessLogError::Canonical`] when the entry has no canonical form,
    /// and [`AccessLogError::Invalid`] when it breaks an invariant.
    pub fn of(entry: &mut AccessLogEntry, index: Option<IndexRow>) -> Result<Self, AccessLogError> {
        let intake = Self::stored(entry, index)?;
        entry.link = AccessLogLink {
            sequence: 1,
            algorithm: ALGORITHM.to_owned(),
            previous_digest: intake.content_digest.clone(),
            content_digest: intake.content_digest.clone(),
            digest: intake.content_digest.clone(),
            signature: None,
        };
        check_entry(entry)?;
        Ok(intake)
    }

    /// The stored parts of `entry`.
    fn stored(entry: &AccessLogEntry, index: Option<IndexRow>) -> Result<Self, AccessLogError> {
        let value = serde_json::to_value(entry).map_err(AccessLogError::Canonical)?;
        let Value::Object(mut members) = value else {
            let whole = canonical_form(entry)?;
            return Ok(Self {
                content_digest: sha256_hex(&whole),
                row: whole,
                shared: None,
                derived: Derived::default(),
                index,
            });
        };
        members.remove(LINK);
        let document = Value::Object(members);
        let whole = canonical_json(&document).map_err(AccessLogError::Canonical)?;
        let content_digest = sha256_hex(&whole);
        let derived = Derived::of(&document);
        let Value::Object(members) = document else {
            return Err(AccessLogError::Canonical(serde::de::Error::custom(
                "an entry serialises as an object",
            )));
        };
        let form = stored_form::split(members);
        let row = canonical_json(&form.row).map_err(AccessLogError::Canonical)?;
        let shared = form
            .shared
            .map(|shared| {
                canonical_json(&shared)
                    .map(|text| (sha256_hex(&text), text))
                    .map_err(AccessLogError::Canonical)
            })
            .transpose()?;
        Ok(Self {
            row,
            shared,
            content_digest,
            derived,
            index,
        })
    }
}

impl Derived {
    /// The derived columns of `document`, read at the paths
    /// `audit.access_log_record_derive` reads.
    fn of(document: &Value) -> Self {
        let text = |path: &str| {
            document
                .pointer(path)
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        Self {
            record_type: text("/_type"),
            uid: text("/uid/value"),
            time_recorded: text("/time_recorded/value"),
            ehr_id: text("/ehr_id/value"),
            action_terminology: text("/action/defining_code/terminology_id/value"),
            action_code: text("/action/defining_code/code_string"),
            result_terminology: text("/outcome/result/defining_code/terminology_id/value"),
            result_code: text("/outcome/result/defining_code/code_string"),
            party_id: text("/accessor/party/external_ref/id/value"),
            retain_until: text("/retention/retain_until/value"),
        }
    }
}

/// The intake rows of a batch, one element per entry, as the `unnest` of
/// [`INTAKE`] reads them.
#[derive(Debug, Default)]
struct Columns {
    record_types: Vec<Option<String>>,
    uids: Vec<Option<String>>,
    times: Vec<Option<String>>,
    contents: Vec<String>,
    shared: Vec<Option<String>>,
    digests: Vec<String>,
    ehr_ids: Vec<Option<String>>,
    party_ids: Vec<Option<String>>,
    retain_until: Vec<Option<String>>,
    action_terminologies: Vec<Option<String>>,
    action_codes: Vec<Option<String>>,
    result_terminologies: Vec<Option<String>>,
    result_codes: Vec<Option<String>>,
    recorded_at: Vec<Option<Timestamp>>,
    actions: Vec<Option<&'static str>>,
    outcomes: Vec<Option<i16>>,
    principals: Vec<Option<String>>,
    patient_ids: Vec<Option<String>>,
    resource_ids: Vec<Option<String>>,
    request_ids: Vec<Option<String>>,
    client_ips: Vec<Option<String>>,
    index_ehr_ids: Vec<Option<String>>,
    tokens: Vec<Option<String>>,
    policies: Vec<Option<Vec<u8>>>,
}

impl Columns {
    /// Adds `entry`'s row.
    fn push(&mut self, entry: IntakeEntry) {
        let IntakeEntry {
            row,
            shared,
            content_digest,
            derived,
            index,
        } = entry;
        self.record_types.push(derived.record_type);
        self.uids.push(derived.uid);
        self.times.push(derived.time_recorded);
        self.contents.push(row);
        self.shared.push(shared.map(|(digest, _)| digest));
        self.digests.push(content_digest);
        self.ehr_ids.push(derived.ehr_id);
        self.party_ids.push(derived.party_id);
        self.retain_until.push(derived.retain_until);
        self.action_terminologies.push(derived.action_terminology);
        self.action_codes.push(derived.action_code);
        self.result_terminologies.push(derived.result_terminology);
        self.result_codes.push(derived.result_code);
        let index = index.map(|index| {
            self.recorded_at
                .push(Some(Timestamp::from(index.recorded_at)));
            self.actions.push(Some(index.action));
            self.outcomes.push(Some(index.outcome));
            self.principals.push(index.principal);
            self.patient_ids.push(index.patient_id);
            self.resource_ids.push(index.resource_id);
            self.request_ids.push(index.request_id);
            self.client_ips.push(index.client_ip);
            self.index_ehr_ids.push(index.ehr_id);
            self.tokens.push(Some(index.search_tokens));
            self.policies.push(Some(index.render_policy));
        });
        if index.is_none() {
            self.recorded_at.push(None);
            self.actions.push(None);
            self.outcomes.push(None);
            self.principals.push(None);
            self.patient_ids.push(None);
            self.resource_ids.push(None);
            self.request_ids.push(None);
            self.client_ips.push(None);
            self.index_ehr_ids.push(None);
            self.tokens.push(None);
            self.policies.push(None);
        }
    }
}

/// The shared parts of a batch, then its intake rows in their order, in one
/// statement.
const INTAKE: &str = "WITH shared AS (INSERT INTO audit.access_log_shared (digest, content) \
     SELECT decode(t.digest, 'hex'), t.content FROM unnest($25::text[], $26::text[]) \
     AS t(digest, content) ON CONFLICT (digest) DO NOTHING) \
     INSERT INTO audit.access_log_intake (record_type, uid, time_recorded, \
     content, shared, content_digest, ehr_id, party_id, retain_until, action_terminology, \
     action_code, result_terminology, result_code, recorded_at, action, outcome, principal, \
     patient_id, resource_id, request_id, client_ip, index_ehr_id, search_tokens, render_policy) \
     SELECT u.c1, u.c2::uuid, u.c3::timestamptz, u.c4, decode(u.c5, 'hex'), decode(u.c6, 'hex'), \
     u.c7, u.c8, u.c9::date, u.c10, u.c11, u.c12, u.c13, u.c14, u.c15, u.c16, u.c17, u.c18, \
     u.c19, u.c20, u.c21, u.c22, u.c23::integer[], u.c24 \
     FROM UNNEST($1::text[], $2::text[], $3::text[], $4::text[], $5::text[], $6::text[], \
     $7::text[], $8::text[], $9::text[], $10::text[], $11::text[], $12::text[], $13::text[], \
     $14::timestamptz[], $15::text[], $16::smallint[], $17::text[], $18::text[], $19::text[], \
     $20::text[], $21::text[], $22::text[], $23::text[], $24::bytea[]) WITH ORDINALITY \
     AS u(c1, c2, c3, c4, c5, c6, c7, c8, c9, c10, c11, c12, c13, c14, c15, c16, c17, c18, c19, \
     c20, c21, c22, c23, c24, n) \
     ORDER BY u.n";

/// Write `entries` into the intake on `conn`, in their order, with the
/// shared parts they name, in one statement; the caller's transaction
/// commits them.
///
/// # Errors
/// The store errors when a statement fails.
pub async fn take_in(
    conn: &mut PgConnection,
    entries: Vec<IntakeEntry>,
) -> Result<(), AccessLogError> {
    if entries.is_empty() {
        return Ok(());
    }
    let mut shared = BTreeMap::new();
    let mut columns = Columns::default();
    for entry in entries {
        if let Some((digest, content)) = &entry.shared {
            shared.insert(digest.clone(), content.clone());
        }
        columns.push(entry);
    }
    let (keys, texts): (Vec<String>, Vec<String>) = shared.into_iter().unzip();
    let c = columns;
    sqlx::query(INTAKE)
        .bind(c.record_types)
        .bind(c.uids)
        .bind(c.times)
        .bind(c.contents)
        .bind(c.shared)
        .bind(c.digests)
        .bind(c.ehr_ids)
        .bind(c.party_ids)
        .bind(c.retain_until)
        .bind(c.action_terminologies)
        .bind(c.action_codes)
        .bind(c.result_terminologies)
        .bind(c.result_codes)
        .bind(c.recorded_at)
        .bind(c.actions)
        .bind(c.outcomes)
        .bind(c.principals)
        .bind(c.patient_ids)
        .bind(c.resource_ids)
        .bind(c.request_ids)
        .bind(c.client_ips)
        .bind(c.index_ehr_ids)
        .bind(c.tokens)
        .bind(c.policies)
        .bind(keys)
        .bind(texts)
        .execute(&mut *conn)
        .await?;
    Ok(())
}
