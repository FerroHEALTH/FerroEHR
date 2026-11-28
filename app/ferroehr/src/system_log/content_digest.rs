// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The content digest of a system audit record (`audit.audit_event`), form 8
//! (#6242, #6633).
//!
//! The writer computes it when it writes the record and stores it in
//! `content_digest`; the seal of the record's window covers it
//! ([`crate::system_log::seal`]), and the verifier computes it again from the
//! stored columns. An auditor recomputes it from a row alone:
//!
//! ```text
//! SHA-256( RFC 8785 ( ["ferroehr.audit_event.v8", { <column>: <value>, ... }] ) )
//! ```
//!
//! The object holds every column of `audit.audit_event` except `seq` (the
//! seal names it beside the digest), `content_digest` and `search_tokens` (a
//! search aid of store-internal ids), under its column name. Each value is
//! written by its type:
//!
//! - SQL `NULL` as JSON `null`, and a `boolean` as JSON `true` or `false`;
//! - `text` as a JSON string, and a `uuid` in its lower-case hyphenated form;
//! - `smallint` and `bigint` as their decimal text in a JSON string, so a
//!   64-bit value stays exact under RFC 8785's numbers;
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

use serde_json::Value;
use sha2::Digest as _;

use crate::digest::sha256;
use openehr_rm::v1_2::common::change_control::canonical_form_impl::canonical_json;
use openehr_rm::v1_2::common::change_control::canonical_serializer_impl::canonical_to_string;

/// The tag the digested array starts with.
pub const TAG: &str = "ferroehr.audit_event.v8";

/// The `digest_version` of a record whose digest has this form.
pub const VERSION: i16 = 8;

/// An instant as the digest writes it: UTC, six fractional digits, `Z`.
#[must_use]
pub fn instant(at: jiff::Timestamp) -> String {
    at.strftime(INSTANT).to_string()
}

/// `at` cut to the microsecond, the precision `timestamptz` keeps.
#[must_use]
pub fn to_microsecond(at: jiff::Timestamp) -> jiff::Timestamp {
    jiff::Timestamp::from_microsecond(at.as_microsecond()).unwrap_or(at)
}

/// The columns of one record, borrowed, as the digest writes them.
#[derive(Debug, Clone, Copy)]
pub struct Record<'a> {
    /// `id`.
    pub id: uuid::Uuid,
    /// `recorded_at`.
    pub recorded_at: jiff::Timestamp,
    /// `stored_at`.
    pub stored_at: jiff::Timestamp,
    /// `action`.
    pub action: &'a str,
    /// `outcome`.
    pub outcome: i16,
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
    pub result_count: Option<i64>,
    /// `request_id`.
    pub request_id: Option<&'a str>,
    /// `roles`.
    pub roles: Option<&'a Value>,
    /// `origins`.
    pub origins: Option<&'a Value>,
    /// `origin_count`.
    pub origin_count: Option<i64>,
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
    /// `justification`.
    pub justification: Option<&'a Value>,
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
    /// `render_policy`, its bytes.
    pub render_policy: &'a [u8],
    /// `binding`.
    pub binding: Option<&'a str>,
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
    pub digest_version: i16,
    /// `token_attributes`.
    pub token_attributes: Option<&'a Value>,
}

/// Why a content digest could not be computed.
#[derive(Debug, thiserror::Error)]
#[error("the record has no RFC 8785 form")]
pub struct Unwritable(#[source] serde_json::Error);

/// The content digest of `record`, its SHA-256, written member by member in
/// RFC 8785 order straight into the hash.
///
/// # Errors
/// [`Unwritable`] when a JSON value of the record holds a number with no
/// finite double form.
pub fn of(record: &Record<'_>) -> Result<[u8; 32], Unwritable> {
    let mut w = Members::open();
    // NOTE: RFC 8785 section 3.2.3, the members in the order of their names, which are ASCII.
    w.optional_text("acting_mode", record.acting_mode);
    w.text("action", record.action);
    w.json("actor_chain", record.actor_chain)?;
    w.json("agreement", record.agreement)?;
    w.optional_text("assurance_level", record.assurance_level);
    w.optional_text("assurance_value", record.assurance_value);
    w.optional_text("binding", record.binding);
    w.json("binding_detail", record.binding_detail)?;
    w.json("categories", record.categories)?;
    w.optional_text("category_basis", record.category_basis);
    w.json("category_content", record.category_content)?;
    w.json("category_evidence", record.category_evidence)?;
    w.optional_text("category_map_digest", record.category_map_digest);
    w.optional_text("client_id", record.client_id);
    w.optional_text("client_ip", record.client_ip);
    w.integer("digest_version", Some(i64::from(record.digest_version)));
    w.text("domain", record.domain);
    w.optional_text("ehr_id", record.ehr_id);
    w.text("event_code", record.event_code);
    w.json("external_fields", record.external_fields)?;
    w.json("facilities", record.facilities)?;
    w.uuid("id", record.id);
    w.optional_text("issuer", record.issuer);
    w.json("justification", record.justification)?;
    w.optional_text("legal_basis", record.legal_basis);
    w.optional_text("operation", record.operation);
    w.integer("origin_count", record.origin_count);
    w.json("origins", record.origins)?;
    w.integer("outcome", Some(i64::from(record.outcome)));
    w.json("party", record.party)?;
    w.json("party_groups", record.party_groups)?;
    w.optional_text("patient_id", record.patient_id);
    w.optional_text("permit", record.permit);
    w.optional_text("principal", record.principal);
    w.optional_text("professional_id", record.professional_id);
    w.json("provider", record.provider)?;
    w.optional_text("purpose", record.purpose);
    w.instant("recorded_at", record.recorded_at);
    w.hex("render_policy", record.render_policy);
    w.json("representation", record.representation)?;
    w.optional_text("request_id", record.request_id);
    w.text("resource_class", record.resource_class);
    w.optional_text("resource_id", record.resource_id);
    w.integer("result_count", record.result_count);
    w.optional_text("retention_ends", record.retention_ends);
    w.boolean("retention_stated", record.retention_stated);
    w.optional_text("review", record.review);
    w.json("roles", record.roles)?;
    w.instant("stored_at", record.stored_at);
    w.boolean("subject_lookup_failed", record.subject_lookup_failed);
    w.optional_text("subject_namespace", record.subject_namespace);
    w.json("summary", record.summary)?;
    w.json("token_attributes", record.token_attributes)?;
    w.optional_text("token_id", record.token_id);
    Ok(w.close())
}

/// The writer of the digested array: each member's name and value in their
/// RFC 8785 form, fed to the hash as they are written.
struct Members {
    hash: sha2::Sha256,
    first: bool,
}

impl Members {
    /// The hash with the array, its tag and the object opened.
    fn open() -> Self {
        let mut members = Self {
            hash: sha2::Sha256::new(),
            first: true,
        };
        members.put(b"[");
        members.string(TAG);
        members.put(b",{");
        members
    }

    /// The object and the array closed, and the digest.
    fn close(mut self) -> [u8; 32] {
        self.put(b"}]");
        self.hash.finalize().into()
    }

    fn put(&mut self, bytes: &[u8]) {
        self.hash.update(bytes);
    }

    /// The separator before a member, and its name.
    fn name(&mut self, name: &str) {
        if !std::mem::take(&mut self.first) {
            self.put(b",");
        }
        self.string(name);
        self.put(b":");
    }

    /// `text` as an RFC 8785 string (section 3.2.2.2): `"` and `\\` escaped,
    /// the control characters as their short escape or `\\u00xx`.
    fn string(&mut self, text: &str) {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        self.put(b"\"");
        let bytes = text.as_bytes();
        let mut start = 0;
        for (at, &byte) in bytes.iter().enumerate() {
            let short: &[u8] = match byte {
                b'"' => b"\\\"",
                b'\\' => b"\\\\",
                0x08 => b"\\b",
                0x0c => b"\\f",
                b'\n' => b"\\n",
                b'\r' => b"\\r",
                b'\t' => b"\\t",
                0x00..=0x1f => b"",
                _ => continue,
            };
            self.put(bytes.get(start..at).unwrap_or_default());
            if short.is_empty() {
                let high = HEX.get(usize::from(byte >> 4)).copied().unwrap_or(b'0');
                let low = HEX.get(usize::from(byte & 0x0f)).copied().unwrap_or(b'0');
                self.put(&[b'\\', b'u', b'0', b'0', high, low]);
            } else {
                self.put(short);
            }
            start = at.saturating_add(1);
        }
        self.put(bytes.get(start..).unwrap_or_default());
        self.put(b"\"");
    }

    fn text(&mut self, name: &str, value: &str) {
        self.name(name);
        self.string(value);
    }

    fn optional_text(&mut self, name: &str, value: Option<&str>) {
        if let Some(value) = value {
            self.text(name, value);
        } else {
            self.name(name);
            self.put(b"null");
        }
    }

    fn boolean(&mut self, name: &str, value: bool) {
        self.name(name);
        self.put(if value { b"true" } else { b"false" });
    }

    /// An integer as its decimal text in a string.
    fn integer(&mut self, name: &str, value: Option<i64>) {
        self.name(name);
        match value {
            Some(value) => {
                use std::fmt::Write as _;
                let mut digits = Stack::default();
                // NOTE: the buffer holds the 20 octets of any i64, so the write never fails.
                let _written = write!(digits, "{value}");
                self.string(digits.as_str());
            }
            None => self.put(b"null"),
        }
    }

    fn uuid(&mut self, name: &str, value: uuid::Uuid) {
        let mut text = [0_u8; uuid::fmt::Hyphenated::LENGTH];
        self.name(name);
        self.put(b"\"");
        self.put(value.hyphenated().encode_lower(&mut text).as_bytes());
        self.put(b"\"");
    }

    fn instant(&mut self, name: &str, value: jiff::Timestamp) {
        use std::fmt::Write as _;
        let mut text = Stack::default();
        // NOTE: the buffer holds the 27 octets of the instant's form, so the write never fails.
        let _written = write!(text, "{}", value.strftime(INSTANT));
        self.text(name, text.as_str());
    }

    /// Bytes as lower-case hexadecimal in a string.
    fn hex(&mut self, name: &str, value: &[u8]) {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        self.name(name);
        self.put(b"\"");
        for byte in value {
            let high = HEX.get(usize::from(byte >> 4)).copied().unwrap_or(b'0');
            let low = HEX.get(usize::from(byte & 0x0f)).copied().unwrap_or(b'0');
            self.put(&[high, low]);
        }
        self.put(b"\"");
    }

    /// A JSON value in its RFC 8785 form.
    fn json(&mut self, name: &str, value: Option<&Value>) -> Result<(), Unwritable> {
        self.name(name);
        match value {
            Some(value) => {
                let text = canonical_to_string(value).map_err(Unwritable)?;
                self.put(text.as_bytes());
            }
            None => self.put(b"null"),
        }
        Ok(())
    }
}

/// The `strftime` form of an instant: UTC, six fractional digits, `Z`.
const INSTANT: &str = "%Y-%m-%dT%H:%M:%S%.6fZ";

/// A short text written on the stack: an instant's form or an integer's.
#[derive(Default)]
struct Stack {
    bytes: [u8; 32],
    len: usize,
}

impl Stack {
    fn as_str(&self) -> &str {
        std::str::from_utf8(self.bytes.get(..self.len).unwrap_or_default()).unwrap_or_default()
    }
}

impl std::fmt::Write for Stack {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        let end = self.len.checked_add(text.len()).ok_or(std::fmt::Error)?;
        self.bytes
            .get_mut(self.len..end)
            .ok_or(std::fmt::Error)?
            .copy_from_slice(text.as_bytes());
        self.len = end;
        Ok(())
    }
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
     'justification', e.justification, \
     'retention_ends', to_char(e.retention_ends, 'YYYY-MM-DD'), \
     'retention_stated', e.retention_stated, 'permit', e.permit, 'issuer', e.issuer, \
     'client_id', e.client_id, 'ehr_id', e.ehr_id, 'subject_namespace', e.subject_namespace, \
     'subject_lookup_failed', e.subject_lookup_failed, 'review', e.review, \
     'category_content', e.category_content, \
     'render_policy', encode(e.render_policy, 'hex'), \
     'binding', e.binding) \
     || jsonb_build_object(\
     'party', e.party, 'party_groups', e.party_groups, 'provider', e.provider, \
     'representation', e.representation, 'external_fields', e.external_fields, \
     'agreement', e.agreement, 'actor_chain', e.actor_chain, 'facilities', e.facilities, \
     'binding_detail', e.binding_detail, 'summary', e.summary, \
     'digest_version', e.digest_version::text, 'token_attributes', e.token_attributes)";

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> jiff::Timestamp {
        text.parse().expect("an instant")
    }

    fn record<'a>(roles: &'a Value, stored_at: &str) -> Record<'a> {
        Record {
            id: "0193a3c4-0000-7000-8000-000000000001"
                .parse()
                .expect("a uuid"),
            recorded_at: at("2026-11-22T04:31:28.123456Z"),
            stored_at: at(stored_at),
            action: "R",
            outcome: 0,
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
            result_count: Some(9_007_199_254_740_993),
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
            justification: None,
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
            render_policy: &[0x00, 0xff],
            binding: None,
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
            digest_version: VERSION,
            token_attributes: None,
        }
    }

    /// The object [`STORED`] reads back for [`record`], written by hand.
    fn stored(roles: &Value) -> Value {
        let text = |value: &str| Value::String(value.to_owned());
        let members = [
            ("id", text("0193a3c4-0000-7000-8000-000000000001")),
            ("recorded_at", text("2026-11-22T04:31:28.123456Z")),
            ("stored_at", text("2026-11-22T04:31:28.200000Z")),
            ("action", text("R")),
            ("outcome", text("0")),
            ("event_code", text("110106")),
            ("operation", text("getTemplate")),
            ("principal", text("alice")),
            ("patient_id", Value::Null),
            ("resource_class", text("template")),
            ("resource_id", text("vital_signs.v1")),
            ("client_ip", text("192.0.2.7")),
            ("token_id", Value::Null),
            ("domain", text("system")),
            ("purpose", Value::Null),
            ("legal_basis", Value::Null),
            ("result_count", text("9007199254740993")),
            ("request_id", text("req-1")),
            ("roles", roles.clone()),
            ("origins", Value::Null),
            ("origin_count", Value::Null),
            ("categories", Value::Null),
            ("category_basis", Value::Null),
            ("category_evidence", Value::Null),
            ("category_map_digest", Value::Null),
            ("acting_mode", text("person")),
            ("assurance_level", Value::Null),
            ("assurance_value", Value::Null),
            ("professional_id", Value::Null),
            ("justification", Value::Null),
            ("retention_ends", text("2029-11-22")),
            ("retention_stated", Value::Bool(true)),
            ("permit", Value::Null),
            ("issuer", Value::Null),
            ("client_id", Value::Null),
            ("ehr_id", Value::Null),
            ("subject_namespace", Value::Null),
            ("subject_lookup_failed", Value::Bool(false)),
            ("review", Value::Null),
            ("category_content", Value::Null),
            ("render_policy", text("00ff")),
            ("binding", Value::Null),
            ("party", Value::Null),
            ("party_groups", Value::Null),
            ("provider", Value::Null),
            ("representation", Value::Null),
            ("external_fields", Value::Null),
            ("agreement", Value::Null),
            ("actor_chain", Value::Null),
            ("facilities", Value::Null),
            ("binding_detail", Value::Null),
            ("summary", Value::Null),
            ("digest_version", text("8")),
            ("token_attributes", Value::Null),
        ];
        assert_eq!(
            members.len(),
            54,
            "every column but seq, content_digest and search_tokens"
        );
        Value::Object(
            members
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value))
                .collect(),
        )
    }

    /// The digest of one fixed record, pinned so a change of the form is a
    /// deliberate new version.
    #[test]
    fn the_digest_of_a_fixed_record_is_pinned() {
        let roles = serde_json::json!(["nurse", "doctor"]);
        let digest = of(&record(&roles, "2026-11-22T04:31:28.2Z")).expect("a digest");
        assert_eq!(
            crate::digest::hex(&digest),
            "5571a2745b73257d1ce8cfb1b6e6459b66fb282ff6197d589ab4d157a0e3e8a3"
        );
    }

    /// The writer's form and the same record read back as JSON digest alike.
    #[test]
    fn the_written_and_the_stored_form_digest_alike() {
        let roles = serde_json::json!(["nurse", "doctor"]);
        assert_eq!(
            of(&record(&roles, "2026-11-22T04:31:28.2Z")).expect("a digest"),
            of_stored(stored(&roles)).expect("a digest")
        );
    }

    /// Text that needs escaping, text beyond ASCII and a JSON value whose
    /// members are out of order are written as RFC 8785 writes them.
    #[test]
    fn escaped_text_and_unordered_values_digest_as_their_canonical_form() {
        let roles = serde_json::json!({"z": [1.5, 1e21, -0.0], "a": "\u{e9}\u{1}\"\\\n/"});
        let mut written = record(&roles, "2026-11-22T04:31:28.2Z");
        written.principal = Some("\u{1f600} \"quoted\" \\ \u{8}\u{c}\n\r\t\u{1f}\u{7f}");
        let mut expected = stored(&roles);
        if let Some(members) = expected.as_object_mut() {
            members.insert(
                "principal".to_owned(),
                Value::String("\u{1f600} \"quoted\" \\ \u{8}\u{c}\n\r\t\u{1f}\u{7f}".to_owned()),
            );
        }
        assert_eq!(
            of(&written).expect("a digest"),
            of_stored(expected).expect("a digest")
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
}
