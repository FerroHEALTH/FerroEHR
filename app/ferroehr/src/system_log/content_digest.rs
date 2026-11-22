// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The content digest of a system audit record (`audit.audit_event`), form 6
//! (#6242).
//!
//! The writer computes it when it writes the record and stores it in
//! `content_digest`; the seal of the record's window covers it
//! ([`crate::system_log::seal`]), and the verifier computes it again from the
//! stored columns. An auditor recomputes it from a row alone:
//!
//! ```text
//! SHA-256( RFC 8785 ( ["ferroehr.audit_event.v6", { <column>: <value>, ... }] ) )
//! ```
//!
//! The object holds every column of `audit.audit_event` except `seq` (the
//! seal names it beside the digest) and `content_digest`, under its column
//! name. Each value is written by its type:
//!
//! - SQL `NULL` as JSON `null`, and a `boolean` as JSON `true` or `false`;
//! - `text` as a JSON string, and a `uuid` in its lower-case hyphenated form;
//! - `smallint` and `bigint` as their decimal text in a JSON string, so a
//!   64-bit value stays exact under RFC 8785's numbers;
//! - `integer[]` (`search_tokens`) as a JSON array of decimal strings;
//! - `timestamptz` as `YYYY-MM-DDTHH:MM:SS.ffffffZ`, UTC with exactly six
//!   fractional digits;
//! - `date` as `YYYY-MM-DD`, and `bytea` as lower-case hexadecimal;
//! - `jsonb` as its JSON value.
//!
//! The writer binds `recorded_at` and `stored_at` at microseconds, the
//! precision `timestamptz` keeps. No specification governs it: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 6, settled by #1885): the record's jsonb \
              columns are JSON values"
)]

use serde::Serialize;
use serde_json::Value;

use crate::digest::sha256;
use openehr_rm::v1_2::common::change_control::canonical_form_impl::canonical_json;
use openehr_rm::v1_2::common::change_control::canonical_serializer_impl::canonical_to_string;

/// The tag the digested array starts with.
pub const TAG: &str = "ferroehr.audit_event.v6";

/// The `digest_version` of a record whose digest has this form.
pub const VERSION: i16 = 6;

/// An instant as the digest writes it: UTC, six fractional digits, `Z`.
#[must_use]
pub fn instant(at: jiff::Timestamp) -> String {
    at.strftime("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}

/// `at` cut to the microsecond, the precision `timestamptz` keeps.
#[must_use]
pub fn to_microsecond(at: jiff::Timestamp) -> jiff::Timestamp {
    jiff::Timestamp::from_microsecond(at.as_microsecond()).unwrap_or(at)
}

/// The columns of one record, borrowed, as the digest writes them.
#[derive(Debug, Serialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the record's four boolean columns, each its own member of the digested object"
)]
pub struct Record<'a> {
    /// `id`.
    pub id: String,
    /// `recorded_at`.
    pub recorded_at: String,
    /// `stored_at`.
    pub stored_at: &'a str,
    /// `action`.
    pub action: &'a str,
    /// `outcome`.
    pub outcome: String,
    /// `event_code`.
    pub event_code: &'a str,
    /// `operation`.
    pub operation: Option<&'a str>,
    /// `principal`.
    pub principal: Option<&'a str>,
    /// `patient_id`.
    pub patient_id: Option<&'a str>,
    /// `resource_class`.
    pub resource_class: &'a str,
    /// `resource_id`.
    pub resource_id: Option<&'a str>,
    /// `client_ip`.
    pub client_ip: Option<&'a str>,
    /// `token_id`.
    pub token_id: Option<&'a str>,
    /// `domain`.
    pub domain: &'a str,
    /// `purpose`.
    pub purpose: Option<&'a str>,
    /// `legal_basis`.
    pub legal_basis: Option<&'a str>,
    /// `result_count`.
    pub result_count: Option<String>,
    /// `request_id`.
    pub request_id: Option<&'a str>,
    /// `roles`.
    pub roles: Option<&'a Value>,
    /// `origins`.
    pub origins: Option<&'a Value>,
    /// `origin_count`.
    pub origin_count: Option<String>,
    /// `categories`.
    pub categories: Option<&'a Value>,
    /// `category_basis`.
    pub category_basis: Option<&'a str>,
    /// `category_evidence`.
    pub category_evidence: Option<&'a Value>,
    /// `category_map_digest`.
    pub category_map_digest: Option<&'a str>,
    /// `acting_mode`.
    pub acting_mode: Option<&'a str>,
    /// `assurance_level`.
    pub assurance_level: Option<&'a str>,
    /// `assurance_value`.
    pub assurance_value: Option<&'a str>,
    /// `professional_id`.
    pub professional_id: Option<&'a str>,
    /// `emergency_access`.
    pub emergency_access: bool,
    /// `retention_ends`.
    pub retention_ends: Option<&'a str>,
    /// `retention_stated`.
    pub retention_stated: bool,
    /// `permit`.
    pub permit: Option<&'a str>,
    /// `issuer`.
    pub issuer: Option<&'a str>,
    /// `client_id`.
    pub client_id: Option<&'a str>,
    /// `ehr_id`.
    pub ehr_id: Option<&'a str>,
    /// `subject_namespace`.
    pub subject_namespace: Option<&'a str>,
    /// `subject_lookup_failed`.
    pub subject_lookup_failed: bool,
    /// `review`.
    pub review: Option<&'a str>,
    /// `category_content`.
    pub category_content: Option<&'a Value>,
    /// `render_policy`.
    pub render_policy: &'a str,
    /// `search_tokens`.
    pub search_tokens: Vec<&'a str>,
    /// `binding`.
    pub binding: Option<&'a str>,
    /// `guest`.
    pub guest: bool,
    /// `party`.
    pub party: Option<&'a Value>,
    /// `party_groups`.
    pub party_groups: Option<&'a Value>,
    /// `provider`.
    pub provider: Option<&'a Value>,
    /// `representation`.
    pub representation: Option<&'a Value>,
    /// `external_fields`.
    pub external_fields: Option<&'a Value>,
    /// `agreement`.
    pub agreement: Option<&'a Value>,
    /// `actor_chain`.
    pub actor_chain: Option<&'a Value>,
    /// `facilities`.
    pub facilities: Option<&'a Value>,
    /// `binding_detail`.
    pub binding_detail: Option<&'a Value>,
    /// `summary`.
    pub summary: Option<&'a Value>,
    /// `digest_version`.
    pub digest_version: String,
    /// `token_attributes`.
    pub token_attributes: Option<&'a Value>,
}

/// The ids of an `integer[]` written as its text, `{1,2,-3}`, each as its
/// decimal text.
#[must_use]
pub fn token_ids(text: &str) -> Vec<&str> {
    text.trim_start_matches('{')
        .trim_end_matches('}')
        .split(',')
        .filter(|id| !id.is_empty())
        .collect()
}

/// Why a content digest could not be computed.
#[derive(Debug, thiserror::Error)]
#[error("the record has no RFC 8785 form")]
pub struct Unwritable(#[source] serde_json::Error);

/// The content digest of `record`, its SHA-256.
///
/// # Errors
/// [`Unwritable`] when a JSON value of the record holds a number with no
/// finite double form.
pub fn of(record: &Record<'_>) -> Result<[u8; 32], Unwritable> {
    canonical_to_string(&(TAG, record))
        .map(|text| sha256(&text))
        .map_err(Unwritable)
}

/// The content digest of a record read back as the object [`STORED`]
/// selects.
///
/// # Errors
/// As [`of`].
pub fn of_stored(document: Value) -> Result<[u8; 32], Unwritable> {
    canonical_json(&Value::Array(vec![Value::String(TAG.to_owned()), document]))
        .map(|text| sha256(&text))
        .map_err(Unwritable)
}

/// The object [`of_stored`] digests, built from the row `e` of
/// `audit.audit_event` with every value written as this module documents.
pub const STORED: &str = "jsonb_build_object(\
     'id', e.id::text, \
     'recorded_at', to_char(e.recorded_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), \
     'stored_at', to_char(e.stored_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'), \
     'action', e.action, 'outcome', e.outcome::text, 'event_code', e.event_code, \
     'operation', e.operation, 'principal', e.principal, 'patient_id', e.patient_id, \
     'resource_class', e.resource_class, 'resource_id', e.resource_id, \
     'client_ip', e.client_ip, 'token_id', e.token_id, 'domain', e.domain, \
     'purpose', e.purpose, 'legal_basis', e.legal_basis, \
     'result_count', e.result_count::text, 'request_id', e.request_id, \
     'roles', e.roles, 'origins', e.origins, 'origin_count', e.origin_count::text, \
     'categories', e.categories, 'category_basis', e.category_basis, \
     'category_evidence', e.category_evidence, 'category_map_digest', e.category_map_digest) \
     || jsonb_build_object(\
     'acting_mode', e.acting_mode, 'assurance_level', e.assurance_level, \
     'assurance_value', e.assurance_value, 'professional_id', e.professional_id, \
     'emergency_access', e.emergency_access, \
     'retention_ends', to_char(e.retention_ends, 'YYYY-MM-DD'), \
     'retention_stated', e.retention_stated, 'permit', e.permit, 'issuer', e.issuer, \
     'client_id', e.client_id, 'ehr_id', e.ehr_id, 'subject_namespace', e.subject_namespace, \
     'subject_lookup_failed', e.subject_lookup_failed, 'review', e.review, \
     'category_content', e.category_content, \
     'render_policy', encode(e.render_policy, 'hex'), \
     'search_tokens', to_jsonb(e.search_tokens::text[]), \
     'binding', e.binding, 'guest', e.guest) \
     || jsonb_build_object(\
     'party', e.party, 'party_groups', e.party_groups, 'provider', e.provider, \
     'representation', e.representation, 'external_fields', e.external_fields, \
     'agreement', e.agreement, 'actor_chain', e.actor_chain, 'facilities', e.facilities, \
     'binding_detail', e.binding_detail, 'summary', e.summary, \
     'digest_version', e.digest_version::text, 'token_attributes', e.token_attributes)";

#[cfg(test)]
mod tests {
    use super::*;

    fn record<'a>(roles: &'a Value, stored_at: &'a str) -> Record<'a> {
        Record {
            id: "0193a3c4-0000-7000-8000-000000000001".to_owned(),
            recorded_at: "2026-11-22T04:31:28.123456Z".to_owned(),
            stored_at,
            action: "R",
            outcome: "0".to_owned(),
            event_code: "110106",
            operation: Some("getTemplate"),
            principal: Some("alice"),
            patient_id: None,
            resource_class: "template",
            resource_id: Some("vital_signs.v1"),
            client_ip: Some("192.0.2.7"),
            token_id: None,
            domain: "system",
            purpose: None,
            legal_basis: None,
            result_count: Some("9007199254740993".to_owned()),
            request_id: Some("req-1"),
            roles: Some(roles),
            origins: None,
            origin_count: None,
            categories: None,
            category_basis: None,
            category_evidence: None,
            category_map_digest: None,
            acting_mode: Some("person"),
            assurance_level: None,
            assurance_value: None,
            professional_id: None,
            emergency_access: false,
            retention_ends: Some("2029-11-22"),
            retention_stated: true,
            permit: None,
            issuer: None,
            client_id: None,
            ehr_id: None,
            subject_namespace: None,
            subject_lookup_failed: false,
            review: None,
            category_content: None,
            render_policy: "00ff",
            search_tokens: vec!["3", "-1"],
            binding: None,
            guest: false,
            party: None,
            party_groups: None,
            provider: None,
            representation: None,
            external_fields: None,
            agreement: None,
            actor_chain: None,
            facilities: None,
            binding_detail: None,
            summary: None,
            digest_version: VERSION.to_string(),
            token_attributes: None,
        }
    }

    /// The digest of one fixed record, pinned so a change of the form is a
    /// deliberate new version.
    #[test]
    fn the_digest_of_a_fixed_record_is_pinned() {
        let roles = serde_json::json!(["nurse", "doctor"]);
        let digest = of(&record(&roles, "2026-11-22T04:31:28.200000Z")).expect("a digest");
        assert_eq!(
            crate::digest::hex(&digest),
            "74cea798d83a2e263a69c5d25c94ba5fc8943ae4b89e8433627ff081e7b30168"
        );
    }

    /// The writer's form and the form of the same record read back as JSON
    /// digest alike.
    #[test]
    fn the_written_and_the_stored_form_digest_alike() {
        let roles = serde_json::json!(["nurse", "doctor"]);
        let record = record(&roles, "2026-11-22T04:31:28.200000Z");
        let stored = serde_json::to_value(&record).expect("a value");
        assert_eq!(
            of(&record).expect("a digest"),
            of_stored(stored).expect("a digest")
        );
    }

    #[test]
    fn an_instant_keeps_six_fractional_digits_in_utc() {
        let at: jiff::Timestamp = "2026-11-22T05:31:28.1234567+01:00"
            .parse()
            .expect("an instant");
        assert_eq!(instant(to_microsecond(at)), "2026-11-22T04:31:28.123456Z");
        let whole: jiff::Timestamp = "2026-11-22T04:31:28Z".parse().expect("an instant");
        assert_eq!(instant(whole), "2026-11-22T04:31:28.000000Z");
    }

    #[test]
    fn the_token_ids_are_read_from_the_array_text() {
        assert_eq!(token_ids("{1,22,-3}"), vec!["1", "22", "-3"]);
        assert_eq!(token_ids("{}"), Vec::<&str>::new());
    }
}
