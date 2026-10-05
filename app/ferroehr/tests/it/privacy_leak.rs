// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The leak gate: no subject identifier other than the opaque pseudonym
//! reaches the contribution outbox, the IHE ATNA audit trail or the traces.
//!
//! No openEHR spec governs any of these three sinks — the outbox and the
//! traces are our own extensions and ATNA is IHE's, so the rule is ours:
//! GDPR Art. 4(5) makes the pseudonymisation worthless if the identity leaves
//! through a side channel (<https://eur-lex.europa.eu/eli/reg/2016/679/oj>).
//!
//! The tests are written so they FAIL on a regression rather than passing
//! vacuously: the fixture deliberately carries identifying values that the
//! policy must strip or refuse, and each sink is asserted to be non-empty
//! before it is scanned. Sinks the server itself authors — the outbox
//! envelope and the two audit renderings — are scanned with the same detectors
//! the write path uses ([`ferroehr::privacy::detect`]) plus the fixture's own
//! values; the trace records, which are dominated by another crate's
//! telemetry, are scanned for the fixture's values alone (the reason is at
//! that test).

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only reaches \
              `#[test]`-annotated functions, so it misses this module's fixture helpers; a \
              failing fixture must panic at the fixture (the Rust Book ch11)"
)]

use std::sync::Arc;

use serde_json::{Value, json};
use sqlx::Row;
use tracing_subscriber::layer::SubscriberExt as _;

use ferroehr::privacy::PrivacyPolicy;
use ferroehr::privacy::config::PrivacyConfig;
use ferroehr::service::FerroEhrService;
use ferroehr::system_log::event::{AuditEvent, EventActionCode, EventOutcome, ObjectClass};
use ferroehr::system_log::fhir;
use ferroehr::system_log::message::{AuditContext, AuditMessage};

use crate::fixtures::uv;

/// The pseudonym namespace this deployment declares, and the one opaque
/// subject identifier the clinical side is allowed to hold.
const PSEUDONYM_NS: &str = "urn:ferroehr:pseudonym";
const OPAQUE_SUBJECT: &str = "018f3c2a-7b41-7c2e-9a55-6d1e4f80b2c3";

/// Values a leak would carry, one per shipped jurisdiction plus a name. All
/// synthetic, each constructed by running a published algorithm forward.
const FORBIDDEN: &[&str] = &[
    "111222333",   // privacy-allow: synthetic, claimed by nl-bsn
    "15038545660", // claimed by no-fodselsnummer
    "9001011239",  // claimed by se-personnummer
    "9434767016",  // claimed by gb-nhs-number
    "010100A123D", // claimed by fi-hetu
    "Dr Sybrand Veenstra",
];

/// The one the write path is asked to store, so the refusal is exercised.
const SYNTHETIC_BSN: &str = "111222333"; // privacy-allow: a synthetic value, no register issues it

fn audit_context() -> AuditContext {
    AuditContext {
        source_id: "ferroehr".to_owned(),
        enterprise_site_id: "site-1".to_owned(),
        server_ip: "10.42.23.77".to_owned(),
        value_if_missing: "UNKNOWN".to_owned(),
    }
}

/// Refuses any of the fixture's deliberately-identifying strings.
///
/// This half of the gate is EXACT: the values are known up front, so only a
/// real leak can trigger it, and it therefore runs over every sink verbatim.
fn assert_no_forbidden_literal(sink: &str, text: &str) {
    for forbidden in FORBIDDEN {
        assert!(
            !text.contains(forbidden),
            "{sink} leaked {forbidden:?}: {text}"
        );
    }
}

/// Refuses any text a shipped identifier rule claims, and any of the fixture's
/// deliberately-identifying strings.
///
/// The opaque pseudonym is the ONE subject identifier that may appear. The
/// scanner's own ruleset is what does the looking, so a rule added for a new
/// jurisdiction widens this gate with no edit here.
fn assert_no_identifier_but_the_pseudonym(sink: &str, text: &str) {
    for rule in ferroehr::privacy::detect::built_in_rules() {
        assert!(
            !rule.matches(text),
            "{sink} carries a value the `{}` rule claims: {text}",
            rule.key
        );
    }
    assert_no_forbidden_literal(sink, text);
}

fn enforcing_service(db: &testkit::TestDb) -> FerroEhrService {
    let policy = PrivacyPolicy::compile(&PrivacyConfig {
        subject_namespaces: vec![PSEUDONYM_NS.to_owned()],
        ..PrivacyConfig::default()
    })
    .expect("the deployment policy compiles");
    FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_privacy(Arc::new(policy))
}

fn pseudonymised_status() -> openehr_rm::prelude::EhrStatus {
    openehr_its::json::from_canonical_value(&json!({
        "_type": "EHR_STATUS",
        "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                              "value": "openEHR-EHR-EHR_STATUS.generic.v1" },
            "rm_version": "1.2.0"
        },
        "name": { "_type": "DV_TEXT", "value": "EHR Status" },
        "subject": {
            "_type": "PARTY_SELF",
            "external_ref": {
                "_type": "PARTY_REF",
                "namespace": PSEUDONYM_NS,
                "type": "PERSON",
                "id": { "_type": "GENERIC_ID", "value": OPAQUE_SUBJECT, "scheme": "pseudonym" }
            }
        },
        "is_queryable": true,
        "is_modifiable": true
    }))
    .expect("the pseudonymised EHR_STATUS decodes")
}

/// A COMPOSITION whose composer is a bare `external_ref` — the form the policy
/// leaves standing.
fn pseudonymised_composition() -> Value {
    let mut composition = crate::fixtures::composition("leak gate");
    composition["composer"] = json!({
        "_type": "PARTY_IDENTIFIED",
        "external_ref": {
            "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": "9c1f0e52-2d84-4a6b-8f31-77a0c5be1d40" }
        }
    });
    composition
}

#[tokio::test]
async fn the_outbox_envelope_carries_no_subject_identifier() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = enforcing_service(&db);
    let ehr_id = svc
        .create_ehr(Some(pseudonymised_status()))
        .await
        .expect("create_ehr");
    svc.create_composition(ehr_id, uv(&pseudonymised_composition(), "249", None))
        .await
        .expect("create_composition");

    let rows = sqlx::query("SELECT envelope FROM event_outbox ORDER BY committed_at")
        .fetch_all(&pool)
        .await
        .expect("the outbox rows");
    assert!(
        !rows.is_empty(),
        "the gate must scan something: no outbox row was written"
    );
    for row in &rows {
        let envelope: Value = row.get("envelope");
        let text = serde_json::to_string(&envelope).expect("serialize the envelope");
        assert_no_identifier_but_the_pseudonym("the outbox envelope", &text);
        // The envelope names the EHR by its own id and never by its subject:
        // even the opaque pseudonym has no business on a broker topic.
        assert!(
            !text.contains(OPAQUE_SUBJECT),
            "the outbox envelope carries the subject pseudonym: {text}"
        );
        assert!(
            !text.contains("subject"),
            "the outbox envelope carries a subject key: {text}"
        );
    }
}

#[tokio::test]
async fn the_atna_records_carry_the_opaque_pseudonym_and_nothing_else() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = enforcing_service(&db);
    let ehr_id = svc
        .create_ehr(Some(pseudonymised_status()))
        .await
        .expect("create_ehr");

    // The Patient-Number participant is filled from `ehr.subject_id`, exactly
    // as the server's own background resolver reads it.
    let resolved: Option<String> = sqlx::query_scalar("SELECT subject_id FROM ehr WHERE id = $1")
        .bind(ehr_id)
        .fetch_one(&pool)
        .await
        .expect("the promoted subject key");
    assert_eq!(
        resolved.as_deref(),
        Some(OPAQUE_SUBJECT),
        "the audit trail's subject is the promoted column, so that is what must be opaque"
    );

    let mut event = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Ehr,
        EventOutcome::Success,
    );
    event.ehr_id = Some(ehr_id.to_string());
    event.object_id = Some(ehr_id.to_string());
    "clinician".clone_into(&mut event.user_id);

    let ctx = audit_context();
    let xml = AuditMessage::build(&event, &ctx, resolved.as_deref())
        .to_xml()
        .expect("the DICOM PS3.15 audit message renders");
    assert!(
        xml.contains(&format!("ParticipantObjectID=\"{OPAQUE_SUBJECT}\"")),
        "the Patient-Number participant must carry the pseudonym: {xml}"
    );
    assert_no_identifier_but_the_pseudonym("the ATNA audit message", &xml);

    let rendered = fhir::to_fhir(&event, &ctx, resolved.as_deref()).expect("the FHIR AuditEvent");
    let text = serde_json::to_string(&rendered).expect("serialize the AuditEvent");
    assert!(
        text.contains(OPAQUE_SUBJECT),
        "the BALP patient entity must carry the pseudonym: {text}"
    );
    assert_no_identifier_but_the_pseudonym("the FHIR AuditEvent", &text);
}

/// A `tracing` writer that collects everything the run emits into one buffer.
#[derive(Clone, Default)]
struct CapturedTraces(Arc<std::sync::Mutex<Vec<u8>>>);

impl CapturedTraces {
    fn text(&self) -> String {
        let bytes = self.0.lock().expect("the trace buffer lock");
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

impl std::io::Write for CapturedTraces {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .expect("the trace buffer lock")
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CapturedTraces {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_commit_traces_carry_no_subject_identifier() {
    let db = testkit::db().await.expect("testkit database");
    let svc = enforcing_service(&db);

    // Everything the commit says, at the loudest level a deployment can turn
    // on: span open/close records and every event, ours and the driver's.
    let captured = CapturedTraces::default();
    let subscriber = tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .with_span_events(tracing_subscriber::fmt::format::FmtSpan::FULL)
            .with_ansi(false)
            .with_writer(captured.clone()),
    );
    let guard = tracing::subscriber::set_default(subscriber);

    let ehr_id = svc
        .create_ehr(Some(pseudonymised_status()))
        .await
        .expect("create_ehr");
    svc.create_composition(ehr_id, uv(&pseudonymised_composition(), "249", None))
        .await
        .expect("create_composition");
    drop(guard);

    let text = captured.text();
    assert!(
        !text.trim().is_empty(),
        "the gate must scan something: the commit emitted no trace record"
    );

    // The exact scan, not the checksum ruleset, is this sink's instrument.
    // The records are dominated by the database driver's own telemetry, and
    // every rule publishes the rate at which random digit runs satisfy its
    // arithmetic (`detect::IdentifierRule::collision`, one in eleven for the
    // elfproef) — a stream of `elapsed_secs` values would be measured, not the
    // policy. The fixture's identifying values are known up front, so a
    // substring match catches the leak and nothing else can trigger it.
    assert_no_forbidden_literal("the commit traces", &text);
    assert!(
        !text.contains(OPAQUE_SUBJECT),
        "a trace record carries the subject pseudonym: {text}"
    );
}

#[tokio::test]
async fn an_identifying_subject_never_reaches_any_sink_because_the_write_is_refused() {
    // The other half of the gate: the sinks above are clean because the value
    // never enters. Both the write path and the database refuse it.
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = enforcing_service(&db);

    let identifying: openehr_rm::prelude::EhrStatus =
        openehr_its::json::from_canonical_value(&json!({
            "_type": "EHR_STATUS",
            "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
            "archetype_details": {
                "_type": "ARCHETYPED",
                "archetype_id": { "_type": "ARCHETYPE_ID",
                                  "value": "openEHR-EHR-EHR_STATUS.generic.v1" },
                "rm_version": "1.2.0"
            },
            "name": { "_type": "DV_TEXT", "value": "EHR Status" },
            "subject": {
                "_type": "PARTY_SELF",
                "external_ref": {
                    "_type": "PARTY_REF",
                    "namespace": PSEUDONYM_NS,
                    "type": "PERSON",
                    "id": { "_type": "GENERIC_ID", "value": SYNTHETIC_BSN, "scheme": "bsn" }
                }
            },
            "is_queryable": true,
            "is_modifiable": true
        }))
        .expect("the fixture decodes");

    svc.create_ehr(Some(identifying))
        .await
        .expect_err("the write path refuses a national identifier as the subject");

    let leaked: i64 = sqlx::query_scalar("SELECT count(*) FROM ehr WHERE subject_id = $1")
        .bind(SYNTHETIC_BSN)
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(leaked, 0, "the refused subject must not exist in storage");
}
