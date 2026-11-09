// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The shared commit-fixture family the topic modules import.
//!
//! Every suite that drives a write through the service layer needs the same
//! pieces: a `PARTY_IDENTIFIED` committer, an `openehr` change-type
//! `DV_CODED_TEXT`, a minimal valid RM COMPOSITION, and the
//! `UPDATE_VERSION` envelope carrying them. They live here once, so the per-suite
//! copies cannot drift apart.
//!
//! A suite whose fixture identity is load-bearing (an assertion reads the
//! composer or committer name) takes the parameterized builder rather than a
//! private copy.

use serde_json::{Value, json};

use ferroehr::ids::{EhrId, VoId};
use ferroehr::service::demographic::precondition::Precondition;
use ferroehr::service::version_update::{WriteRequest, change_type_coded, lifecycle_state_coded};
use ferroehr::versioning::object_version_id::{
    TreeId, UidAddress, parse_uid_based_id, parse_version_uid,
};
use openehr_its::rest::generated::common::{UpdateAudit, UpdateAuditData, UpdateVersion};
use openehr_its::rest::stored_query::{
    QualifiedQueryName, QueryNamePattern, QueryVersion, VersionPattern,
};
use openehr_rm::prelude::PartyProxy;

/// The identity the shared fixtures write as — both the COMPOSITION `composer`
/// and the commit `committer`.
const TESTER: &str = "conformance tester";

/// Returns a `PARTY_IDENTIFIED` committer named `name`, as canonical JSON.
pub(crate) fn committer(name: &str) -> Value {
    json!({ "_type": "PARTY_IDENTIFIED", "name": name })
}

/// Returns a `PARTY_IDENTIFIED` committer named `name`, as the typed RM value.
pub(crate) fn committer_proxy(name: &str) -> PartyProxy {
    openehr_its::json::from_canonical_value(&committer(name)).expect("committer")
}

/// Returns the wire `DV_CODED_TEXT` naming `openehr` terminology `code` with
/// rubric `value`.
///
/// The `audit_change_type` group codes this carries are listed in RM common
/// `master06-change_control_package.adoc` §Contributions.
pub(crate) fn change_type(code: &str, value: &str) -> Value {
    json!({
        "_type": "DV_CODED_TEXT", "value": value,
        "defining_code": {
            "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
            "code_string": code
        }
    })
}

/// Returns a commit `audit`: the `openehr` change type `code` with rubric
/// `value`, committed by the `PARTY_IDENTIFIED` named `committer_name`.
pub(crate) fn commit_audit(code: &str, value: &str, committer_name: &str) -> Value {
    json!({ "change_type": change_type(code, value), "committer": committer(committer_name) })
}

/// Returns a wire `UPDATE_ATTESTATION` (`666|attestation|`) by the
/// `PARTY_IDENTIFIED` named `committer_name`, with the `reason` text.
pub(crate) fn attestation_by(committer_name: &str, reason: &str, is_pending: bool) -> Value {
    json!({
        "_type": "UPDATE_ATTESTATION",
        "change_type": change_type("666", "attestation"),
        "committer": committer(committer_name),
        "reason": { "_type": "DV_TEXT", "value": reason },
        "is_pending": is_pending
    })
}

/// A `version_lifecycle_state` member in the released wire shape of
/// `UPDATE_VERSION.lifecycle_state`, a `DV_CODED_TEXT` (ITS-REST
/// `schemas/ehr/UpdateVersion.yaml` → `data_types/DvCodedText.yaml`).
pub(crate) fn lifecycle(code: &str) -> Value {
    openehr_its::json::to_canonical_value(&lifecycle_state_coded(code))
}

/// Returns a minimal *valid* RM COMPOSITION named `name`, composed by
/// `composer`.
///
/// `language`, `territory`, `category` and `composer` are all `1..1` (RM ehr,
/// COMPOSITION class), so typed RM validation rejects a fixture without them.
/// No template is referenced, so the fixture needs no `template_store` row.
pub(crate) fn composition_by(name: &str, composer: &str) -> Value {
    json!({
        "_type": "COMPOSITION",
        "archetype_node_id": "openEHR-EHR-COMPOSITION.encounter.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": {
                "_type": "ARCHETYPE_ID",
                "value": "openEHR-EHR-COMPOSITION.encounter.v1"
            },
            "rm_version": "1.2.0"
        },
        "name": { "_type": "DV_TEXT", "value": name },
        "language": {
            "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" },
            "code_string": "en"
        },
        "territory": {
            "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_3166-1" },
            "code_string": "NL"
        },
        "category": {
            "_type": "DV_CODED_TEXT",
            "value": "event",
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": "433"
            }
        },
        "composer": { "_type": "PARTY_IDENTIFIED", "name": composer }
    })
}

/// Returns a minimal *valid* RM COMPOSITION named `name`.
pub(crate) fn composition(name: &str) -> Value {
    composition_by(name, TESTER)
}

/// A commit envelope a fixture builds as a wire `UPDATE_VERSION`: the
/// envelope itself for a CONTRIBUTION member, or the [`WriteRequest`] a direct
/// write takes.
pub(crate) trait Envelope<T> {
    /// The envelope of `version`.
    fn from_version(version: UpdateVersion<T>) -> Self;
}

impl<T> Envelope<T> for UpdateVersion<T> {
    fn from_version(version: UpdateVersion<T>) -> Self {
        version
    }
}

impl<T> Envelope<T> for WriteRequest<T> {
    fn from_version(version: UpdateVersion<T>) -> Self {
        WriteRequest {
            data: version.data.expect("a direct write carries content"),
            audit: Some(version.commit_audit),
            lifecycle_state: Some(version.lifecycle_state.defining_code.code_string)
                .filter(|code| !code.is_empty()),
            signature: version.signature,
            attestations: version.attestations.unwrap_or_default(),
            preceding: version.preceding_version_uid,
        }
    }
}

/// Returns the `UPDATE_VERSION` commit envelope for a bare-RM write.
///
/// `change_code` is the `openehr` `audit_change_type` code the version's audit
/// records (`249` creation, `251` modification, `523` deleted); `preceding`
/// names the version this one supersedes (RM common
/// `master06-change_control_package.adoc` §Contributions).
pub(crate) fn uv<T: serde::de::DeserializeOwned, R: Envelope<T>>(
    data: &Value,
    change_code: &str,
    preceding: Option<&str>,
) -> R {
    R::from_version({
        UpdateVersion {
            preceding_version_uid: preceding.map(|p| p.parse().expect("OBJECT_VERSION_ID")),
            lifecycle_state: lifecycle_state_coded("532"),
            attestations: None,
            data: openehr_its::json::from_canonical_value(data)
                .expect("the fixture commit body decodes as its RM type"),
            commit_audit: UpdateAudit::UpdateAudit(UpdateAuditData {
                _type: None,
                system_id: None,
                change_type: change_type_coded(change_code),
                description: None,
                committer: committer_proxy(TESTER),
                additional_properties: std::collections::BTreeMap::new(),
            }),
            signature: None,
            additional_properties: std::collections::BTreeMap::new(),
        }
    })
}

/// The committed version's `uid/value` string off a served body.
pub(crate) fn uid(v: &Value) -> &str {
    v["uid"]["value"].as_str().expect("uid.value")
}

/// The bare versioned-object UUID of an `OBJECT_VERSION_ID` — everything
/// before the first separator (`object_version_id = object_id, '::',
/// creating_system_id, '::', version_tree_id`; BASE
/// `base_types/master05-identification_package.adoc` §Syntaxes).
pub(crate) fn vo_of(ovid: &str) -> &str {
    ovid.split("::").next().expect("vo uuid")
}

/// The typed id of an EHR a fixture holds as its wire string.
pub(crate) fn ehr(raw: impl AsRef<str>) -> EhrId {
    EhrId(raw.as_ref().parse().expect("an EHR uuid"))
}

/// The address a wire `uid_based_id` names: a versioned object, or one of
/// its versions.
pub(crate) fn at(raw: impl AsRef<str>) -> UidAddress {
    parse_uid_based_id(raw.as_ref()).expect("a well-formed uid_based_id")
}

/// The versioned object a bare id or an `OBJECT_VERSION_ID` names.
pub(crate) fn object(raw: impl AsRef<str>) -> VoId {
    VoId(
        vo_of(raw.as_ref())
            .parse()
            .expect("a versioned-object uuid"),
    )
}

/// The version tree position an `OBJECT_VERSION_ID` names.
pub(crate) fn tree(raw: impl AsRef<str>) -> TreeId {
    parse_version_uid(raw.as_ref())
        .expect("an OBJECT_VERSION_ID")
        .1
}

/// The precondition of a mandatory `If-Match` naming `raw`.
pub(crate) fn if_match(raw: impl AsRef<str>) -> Precondition {
    Precondition::parse(Some(raw.as_ref()), None).expect("a well-formed If-Match")
}

/// The precondition of a delete addressed by the version uid `raw`, with an
/// optional `If-Match`.
pub(crate) fn delete_precondition(raw: impl AsRef<str>, if_match: Option<&str>) -> Precondition {
    Precondition::parse(if_match, at(raw).tree()).expect("a well-formed If-Match")
}

/// A minimal valid root FOLDER (RM ehr master04 §Folders).
pub(crate) fn folder(name: &str) -> Value {
    json!({
        "_type": "FOLDER",
        "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-FOLDER.generic.v1" },
            "rm_version": "1.1.0"
        },
        "name": { "_type": "DV_TEXT", "value": name }
    })
}

// ── separated-credential fixtures ────────────────────────────────────────────

/// A password for a throwaway login role, fresh per call.
///
/// The value is never a secret: the role lives as long as one test against an
/// ephemeral clone. It is generated rather than written down because a literal
/// here is indistinguishable, to a scanner and to a reader, from a credential
/// that does matter, and the repository's own rule is that a finding is fixed
/// rather than suppressed.
pub(crate) fn throwaway_password() -> String {
    format!("pw{}", uuid::Uuid::now_v7().simple())
}

/// Rewrite the userinfo of a testkit clone DSN so a test can connect to the
/// same database as a different login role (scheme/host/port/database
/// preserved).
pub(crate) fn with_role(base_url: &str, user: &str, password: &str) -> String {
    let (scheme, rest) = base_url.split_once("://").expect("dsn scheme");
    let host_and_path = rest.split_once('@').map_or(rest, |(_, tail)| tail);
    format!("{scheme}://{user}:{password}@{host_and_path}")
}

/// A login DSN for a throwaway role holding `domain_role` and nothing else.
///
/// Roles are cluster-global on the shared testkit server, so the login role is
/// named off the clone's database name and the testkit sweep reaps it. The DSN
/// form is what lets a test build a real `DbConfig` on a separated credential.
pub(crate) async fn dsn_as(db: &testkit::TestDb, suffix: &str, domain_role: &str) -> String {
    let login = format!("{}_{suffix}", db.name());
    let password = throwaway_password();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {login} LOGIN PASSWORD '{password}' IN ROLE {domain_role}"
    )))
    .execute(&db.pool())
    .await
    .expect("create the login role");
    with_role(db.url(), &login, &password)
}

/// Every link of an error's cause chain, outermost first, joined by ` <- `.
///
/// A curated service message says nothing about the driver fault behind it;
/// the chain does. A test that fails on a database step panics with this so
/// the cause is in the report rather than only on a trace nobody captured
/// (#3250).
pub(crate) fn source_chain(error: &dyn std::error::Error) -> String {
    let mut out = error.to_string();
    let mut next = error.source();
    while let Some(cause) = next {
        out.push_str(" <- ");
        out.push_str(&cause.to_string());
        next = cause.source();
    }
    out
}

/// The four domain pools of a co-located test database, and the layout that
/// describes it.
///
/// Every DB-backed test runs one testkit database with all four schemas in it,
/// which is the shared-DSN posture the boot gate reads as one credential.
pub(crate) fn shared_pools(pool: &sqlx::PgPool) -> ferroehr::db::domain::DomainPools {
    ferroehr::db::domain::DomainPools::from_shared(pool)
}

/// The domain layout of that same co-located database: four domains, one DSN.
pub(crate) fn shared_layout() -> ferroehr::db::domain::DomainLayout {
    ferroehr::db::domain::StorageConfig::default().layout(&ferroehr::db::DbConfig::default())
}

/// Sets `EHR_STATUS.is_queryable` of `ehr` to `false` through the canonical
/// status update, so the EHR leaves every population query (RM
/// `org.openehr.rm.ehr.ehr_status.adoc`: "True if this EHR should be included
/// in population queries").
pub(crate) async fn set_not_queryable(svc: &ferroehr::service::FerroEhrService, ehr: EhrId) {
    let mut body = svc
        .ehr_status(ehr, None)
        .await
        .map(|r| r.body)
        .expect("the current EHR_STATUS reads");
    let if_match = body["uid"]["value"]
        .as_str()
        .expect("the EHR_STATUS carries its uid")
        .to_owned();
    let status = body.as_object_mut().expect("an EHR_STATUS object");
    status.remove("uid");
    status.insert("is_queryable".to_owned(), json!(false));
    Box::pin(svc.replace_ehr_status(ehr, uv(&body, "251", Some(&if_match))))
        .await
        .map(|m| m.uid)
        .expect("the EHR_STATUS update to is_queryable = false commits");
}

/// Whether `ehr` exists, read through the `GET /ehr/{ehr_id}` service call: a
/// read refused as `ehr_id_does_not_exist` is `false`, any other failure fails
/// the test.
pub(crate) async fn ehr_exists(svc: &ferroehr::service::FerroEhrService, ehr: EhrId) -> bool {
    match svc.ehr_read(ehr).await {
        Ok(_) => true,
        Err(e) => {
            assert!(
                matches!(
                    e,
                    ferroehr::error::ServiceError::NotFound(ferroehr::error::refusal::Refusal {
                        kind: ferroehr::error::refusal::Missing::Ehr,
                        ..
                    })
                ),
                "the EHR read fails only as an unknown EHR: {e}"
            );
            false
        }
    }
}

/// The versioned object a write response names, read off its
/// `OBJECT_VERSION_ID`.
pub(crate) fn written_vo(response: &ferroehr::service::response::ServiceResponse) -> VoId {
    let uid = &response
        .meta
        .as_ref()
        .expect("a write answers its version")
        .uid;
    VoId(vo_of(uid).parse().expect("a uuid versioned object id"))
}

/// Creates the PARTY an `UPDATE_VERSION` envelope carries through the
/// `POST /demographic/{kind}` service call, routed by the body's RM type,
/// returning the new versioned object.
pub(crate) async fn create_party(
    svc: &ferroehr::service::FerroEhrService,
    version: WriteRequest<openehr_rm::prelude::Party>,
) -> Result<VoId, ferroehr::error::ServiceError> {
    use ferroehr::service::demographic::types::PartyKind;
    use openehr_rm::prelude::Party;
    let kind = match &version.data {
        Party::Agent(_) => PartyKind::Agent,
        Party::Group(_) => PartyKind::Group,
        Party::Organisation(_) => PartyKind::Organisation,
        Party::Person(_) => PartyKind::Person,
        Party::Role(_) => PartyKind::Role,
    };
    let created = Box::pin(svc.party_create(kind, version)).await?;
    Ok(written_vo(&created))
}

/// Creates the `PARTY_RELATIONSHIP` an `UPDATE_VERSION` envelope carries
/// through the service call the REST relationship create makes, returning the
/// new versioned object.
pub(crate) async fn create_relationship(
    svc: &ferroehr::service::FerroEhrService,
    version: WriteRequest<openehr_rm::prelude::PartyRelationship>,
) -> Result<VoId, ferroehr::error::ServiceError> {
    let created = svc.party_relationship_create(version).await?;
    Ok(written_vo(&created))
}

/// An `EHR_STATUS` whose subject is a `PARTY_SELF` with an `external_ref`
/// naming `id` in `namespace` as an object of class `class` (RM ehr master04
/// §EHR Status; BASE `OBJECT_REF`), written as `POST /ehr` receives it. The id
/// is a `GENERIC_ID` under the namespace as its scheme, so a business
/// identifier is as admissible as a UUID (BASE `GENERIC_ID`).
pub(crate) fn status_naming(
    id: &str,
    namespace: &str,
    class: &str,
) -> openehr_rm::prelude::EhrStatus {
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
                "namespace": namespace,
                "type": class,
                "id": { "_type": "GENERIC_ID", "value": id, "scheme": namespace }
            }
        },
        "is_queryable": true,
        "is_modifiable": true
    }))
    .expect("the subject-bearing EHR_STATUS decodes")
}

/// Creates, through the `POST /ehr` service call, the EHR whose
/// `EHR_STATUS.subject` names `subject`.
pub(crate) async fn create_ehr_subject(
    svc: &ferroehr::service::FerroEhrService,
    subject: &ferroehr::service::ehr_index::types::SubjectRef,
) -> EhrId {
    Box::pin(svc.create_ehr_meta(WriteRequest::new(Some(status_naming(
        &subject.id,
        &subject.namespace,
        &subject.r#type,
    )))))
    .await
    .expect("the EHR naming its subject is created")
    .0
}

/// Creates, through the `POST /ehr` service call, the EHR whose subject is the
/// party `party` (its versioned-object id, as a `PERSON`).
pub(crate) async fn create_ehr_for_party(
    svc: &ferroehr::service::FerroEhrService,
    party: VoId,
) -> EhrId {
    create_ehr_subject(
        svc,
        &ferroehr::service::ehr_index::types::SubjectRef::person(party.0.to_string(), "local"),
    )
    .await
}

/// The ids of the EHRs whose `EHR_STATUS.subject` names `id` in `namespace`,
/// read through the `GET /ehr?subject_id=…&subject_namespace=…` service call:
/// empty when the read answers `ehr_id_does_not_exist`, one id otherwise.
pub(crate) async fn subject_ehr_ids(
    svc: &ferroehr::service::FerroEhrService,
    id: &str,
    namespace: &str,
) -> Vec<String> {
    match svc.ehr_read_for_subject(id, namespace).await {
        Ok(read) => vec![
            read.body["ehr_id"]["value"]
                .as_str()
                .expect("the EHR carries its ehr_id")
                .to_owned(),
        ],
        Err(e) => {
            assert!(
                matches!(
                    e,
                    ferroehr::error::ServiceError::NotFound(ferroehr::error::refusal::Refusal {
                        kind: ferroehr::error::refusal::Missing::Ehr,
                        ..
                    })
                ),
                "the subject read fails only as an unknown subject: {e}"
            );
            Vec::new()
        }
    }
}

/// Every finding of the full tamper check, `audit.verify_audit_chain()`, run
/// on `pool` without moving the verification checkpoint.
pub(crate) async fn verify_chain(
    pool: &sqlx::PgPool,
) -> Result<Vec<ferroehr::system_log::store::AuditChainFinding>, sqlx::Error> {
    use sqlx::Row;
    sqlx::query(
        "SELECT chain_seq, record_id, recorded_at, finding \
         FROM audit.verify_audit_chain() ORDER BY chain_seq NULLS FIRST",
    )
    .fetch_all(pool)
    .await?
    .iter()
    .map(|row| {
        Ok(ferroehr::system_log::store::AuditChainFinding {
            chain_seq: row.try_get("chain_seq")?,
            record_id: row.try_get("record_id")?,
            recorded_at: row
                .try_get::<Option<jiff_sqlx::Timestamp>, _>("recorded_at")?
                .map(jiff_sqlx::Timestamp::to_jiff),
            finding: row.try_get("finding")?,
        })
    })
    .collect()
}

/// Whether an ADL 1.4 archetype with `id` is stored, read off the store; identity
/// is case-insensitive (BASE master05 §Composite Identifiers and Case).
pub(crate) async fn archetype_stored(pool: &sqlx::PgPool, id: &str) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM archetype_store WHERE lower(archetype_id) = lower($1))",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .expect("the archetype store answers")
}

/// Whether an ADL2 artefact with `ARCHETYPE_HRID` `hrid` is stored, read off the
/// store; HRID identity is case-insensitive (BASE master05 §Composite
/// Identifiers and Case).
pub(crate) async fn artefact_stored(pool: &sqlx::PgPool, hrid: &str) -> bool {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM adl2_artefact WHERE lower(hrid) = lower($1))")
        .bind(hrid)
        .fetch_one(pool)
        .await
        .expect("the artefact store answers")
}

/// A fresh, fully migrated database and the platform service over it, both
/// domains on its one pool. Hold the database for as long as the service.
pub(crate) async fn service() -> (testkit::TestDb, ferroehr::service::FerroEhrService) {
    let db = testkit::db().await.expect("testkit database");
    let svc = ferroehr::service::FerroEhrService::new(
        &ferroehr::db::domain::DomainPools::from_shared(&db.pool()),
    );
    (db, svc)
}

/// Create an EHR with no subject and no supplied id; return its id.
pub(crate) async fn create_ehr(svc: &ferroehr::service::FerroEhrService) -> EhrId {
    Box::pin(svc.create_ehr_meta(WriteRequest::new(None)))
        .await
        .map(|(id, _)| id)
        .expect("create_ehr")
}

/// An `EHR_STATUS` whose `PARTY_SELF` subject carries an `external_ref` — RM
/// ehr master04 §EHR Status (the subject 0..1 identifies the EHR).
pub(crate) fn status_for_subject(subject_id: &str) -> Value {
    json!({
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
                "namespace": "patients",
                "type": "PERSON",
                "id": { "_type": "HIER_OBJECT_ID", "value": subject_id }
            }
        },
        "is_queryable": true,
        "is_modifiable": true
    })
}

/// Build a client pointing at `base` with a short, test-friendly timeout.
pub(crate) fn provider(
    base: &str,
) -> ferroehr::service::terminology::fhir::FhirTerminologyProvider {
    use ferroehr::service::terminology::config::TerminologyServerConfig;
    use ferroehr::service::terminology::fhir::FhirTerminologyProvider;
    let cfg = TerminologyServerConfig {
        enabled: true,
        url: ferrohealth_secret::secret_url::SecretUrl::new(base),
        request_timeout_ms: 800,
        ..TerminologyServerConfig::default()
    };
    FhirTerminologyProvider::new(&cfg).expect("build provider")
}

pub(crate) fn plan_ok(q: &str) -> ferroehr::aql::ir::QueryIr {
    let ast = openehr_query::parser::parse_str(q)
        .unwrap_or_else(|e| panic!("parse failed for {q:?}: {e}"));
    ferroehr::aql::plan(
        &ast,
        &ferroehr::aql::ir::Params::default(),
        ferroehr::config::profile::SpecProfile::default(),
    )
    .unwrap_or_else(|e| panic!("plan failed for {q:?}: {e}"))
}

pub(crate) async fn setup() -> (testkit::TestDb, ferroehr::service::FerroEhrService, EhrId) {
    let (db, svc) = service().await;
    let ehr_id = create_ehr(&svc).await;
    (db, svc, ehr_id)
}

/// The base composition with its template stripped and a `DV_QUANTITY` leaf.
pub(crate) fn base_composition() -> Value {
    let path = format!(
        "{}/../../crates/openehr-its/tests/vendor/openehr_sdk/composition/canonical_json/minimal_observation.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(&path).expect("read fixture"))
        .expect("parse fixture")
}

/// The minimal observation composition named `name` with the reading
/// `magnitude`: content, so a shared tree holds more than its root.
pub(crate) fn observation_composition(name: &str, magnitude: f64) -> Value {
    let mut c = base_composition();
    if let Some(details) = c
        .get_mut("archetype_details")
        .and_then(Value::as_object_mut)
    {
        details.remove("template_id");
    }
    if let Some(root) = c.as_object_mut() {
        root.remove("uid");
    }
    c["name"] = serde_json::json!({ "_type": "DV_TEXT", "value": name });
    c["content"][0]["archetype_details"] = serde_json::json!({
        "_type": "ARCHETYPED",
        "archetype_id": {
            "_type": "ARCHETYPE_ID",
            "value": "openEHR-EHR-OBSERVATION.minimal.v1"
        },
        "rm_version": "1.1.0",
    });
    c["content"][0]["data"]["events"][0]["data"]["items"][0]["value"] =
        serde_json::json!({ "_type": "DV_QUANTITY", "magnitude": magnitude, "units": "mm[Hg]" });
    c
}

/// The DB's current instant (`SELECT now()`) as a `jiff::Timestamp`. Time-travel
/// probes MUST anchor their reference instant on this server clock — the same
/// clock that stamps `version.committed_at` — never on the test-process wall
/// clock: a client/DB clock skew under parallel test load would race
/// the at-time read against the version validity intervals.
pub(crate) async fn db_now(pool: &sqlx::PgPool) -> jiff::Timestamp {
    sqlx::query_scalar::<_, jiff_sqlx::Timestamp>("SELECT now()")
        .fetch_one(pool)
        .await
        .expect("db now()")
        .to_jiff()
}

/// The `OBJECT_VERSION_ID` of the first version listed in a served CONTRIBUTION.
pub(crate) fn first_version_uid(contribution: &Value) -> String {
    contribution["versions"][0]["id"]["value"]
        .as_str()
        .expect("versions[0].id.value")
        .to_owned()
}

/// The request of a delete that names `preceding` as the current version
/// (`If-Match`), committed under the server's own audit.
pub(crate) fn deleting(preceding: Option<openehr_base::prelude::ObjectVersionId>) -> WriteRequest {
    let mut request = WriteRequest::new(());
    request.preceding = preceding;
    request
}

/// Seeds an EHR for the extract tests: an `EHR_STATUS` in two versions
/// (create, then update), a directory `FOLDER` and the auto-created
/// `EHR_ACCESS`. Returns the EHR id and the `EHR_STATUS` versioned-object uid.
pub(crate) async fn seed_extract_ehr(svc: &ferroehr::service::FerroEhrService) -> (EhrId, String) {
    let ehr_uuid = create_ehr(svc).await;

    let mut status = svc
        .ehr_status(ehr_uuid, None)
        .await
        .map(|r| r.body)
        .expect("status get");
    let status_ovid = status["uid"]["value"].as_str().expect("uid").to_owned();
    let status_vo = vo_of(&status_ovid).to_owned();
    status.as_object_mut().expect("status obj").remove("uid");

    svc.create_directory(
        ehr_uuid,
        uv(
            &json!({
                "_type": "FOLDER",
                "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1",
                "archetype_details": {
                    "_type": "ARCHETYPED",
                    "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-FOLDER.generic.v1" },
                    "rm_version": "1.1.0"
                },
                "name": { "_type": "DV_TEXT", "value": "root" }
            }),
            "249",
            None,
        ),
    )
    .await
    .expect("directory");

    status["is_modifiable"] = json!(false);
    Box::pin(svc.replace_ehr_status(ehr_uuid, uv(&status, "251", Some(&status_ovid))))
        .await
        .map(|m| m.uid)
        .expect("status update");

    (ehr_uuid, status_vo)
}

/// The content item of an extract whose wrapped object has the given
/// `X_VERSIONED_*` `_type`.
pub(crate) fn find_by_xtype<'a>(extract: &'a Value, xtype: &str) -> Option<&'a Value> {
    extract["chapters"][0]["items"]
        .as_array()
        .expect("chapter items")
        .iter()
        .find(|it| it["item"]["_type"] == json!(xtype))
}

/// Upload one vendored CKM template and generate its committable example, as
/// the storage bench seeds it.
pub(crate) async fn ckm_example(svc: &ferroehr::service::FerroEhrService, template: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../corpus/templates/ckm/{template}.opt"));
    let xml = std::fs::read_to_string(path).expect("the vendored OPT is readable");
    let template_id = openehr_its::opt14::from_xml(&xml)
        .expect("the vendored OPT parses")
        .template_id
        .value;
    svc.template_adl14_upload(&xml)
        .await
        .expect("upload the template");
    svc.template_adl14_example(template_id, Some("medium".to_owned()), None)
        .await
        .expect("the template generates an example composition")
}

/// The stored-query name `text`, read by the ITS-REST grammar.
pub(crate) fn qn(text: &str) -> QualifiedQueryName {
    text.parse().expect("a valid qualified query name")
}

/// The stored-query list pattern `text`, read by the ITS-REST grammar.
pub(crate) fn query_pattern(text: &str) -> QueryNamePattern {
    text.parse().expect("a valid query name pattern")
}

/// The stored-query version pattern `text`.
pub(crate) fn version_pattern(text: &str) -> VersionPattern {
    text.parse().expect("a valid version pattern")
}

/// The exact stored-query version `text`.
pub(crate) fn query_version(text: &str) -> QueryVersion {
    text.parse().expect("a valid query version")
}
