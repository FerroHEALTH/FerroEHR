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

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this module's \
              fixture helpers; a failing fixture must panic at the fixture (the \
              Rust Book ch11)"
)]

use serde_json::{Value, json};

use ferroehr::service::version_update::{change_type_coded, lifecycle_state_coded};
use openehr_its::rest::generated::common::{UpdateAudit, UpdateAuditData, UpdateVersion};
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

/// Returns the `UPDATE_VERSION` commit envelope for a bare-RM write.
///
/// `change_code` is the `openehr` `audit_change_type` code the version's audit
/// records (`249` creation, `251` modification, `523` deleted); `preceding`
/// names the version this one supersedes (RM common
/// `master06-change_control_package.adoc` §Contributions).
pub(crate) fn uv<T: serde::de::DeserializeOwned>(
    data: &Value,
    change_code: &str,
    preceding: Option<&str>,
) -> UpdateVersion<T> {
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
pub(crate) async fn set_not_queryable(
    svc: &ferroehr::service::FerroEhrService,
    ehr: ferroehr::ids::EhrId,
) {
    let mut body = svc
        .ehr_status_at_time_response(ehr, None)
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
    Box::pin(svc.replace_ehr_status_meta(ehr, uv(&body, "251", Some(&if_match))))
        .await
        .map(|m| m.uid)
        .expect("the EHR_STATUS update to is_queryable = false commits");
}

/// Whether `ehr` exists, read through the `GET /ehr/{ehr_id}` service call: a
/// read refused as `ehr_id_does_not_exist` is `false`, any other failure fails
/// the test.
pub(crate) async fn ehr_exists(
    svc: &ferroehr::service::FerroEhrService,
    ehr: ferroehr::ids::EhrId,
) -> bool {
    match svc.ehr_read(ehr).await {
        Ok(_) => true,
        Err(e) => {
            assert!(
                matches!(
                    e,
                    ferroehr::service::error::ServiceError::NotFound(
                        ferroehr::service::refusal::Refusal {
                            kind: ferroehr::service::refusal::Missing::Ehr,
                            ..
                        }
                    )
                ),
                "the EHR read fails only as an unknown EHR: {e}"
            );
            false
        }
    }
}

/// The [`Committal`](ferroehr::service::version_update::Committal) an
/// `UPDATE_VERSION` envelope carries, as the wire's `openehr-version` and
/// `openehr-audit-details` headers carry it: the audit plus a stated
/// `lifecycle_state` code.
pub(crate) fn envelope_committal<T>(
    version: &UpdateVersion<T>,
) -> ferroehr::service::version_update::Committal {
    ferroehr::service::version_update::Committal {
        audit: version.commit_audit.clone(),
        lifecycle_state: Some(version.lifecycle_state.defining_code.code_string.clone())
            .filter(|code| !code.is_empty()),
    }
}

/// The versioned object a write response names, read off its
/// `OBJECT_VERSION_ID`.
pub(crate) fn written_vo(
    response: &ferroehr::service::response::ServiceResponse,
) -> ferroehr::ids::VoId {
    let uid = &response
        .meta
        .as_ref()
        .expect("a write answers its version")
        .uid;
    ferroehr::ids::VoId(vo_of(uid).parse().expect("a uuid versioned object id"))
}

/// Creates the PARTY an `UPDATE_VERSION` envelope carries through the
/// `POST /demographic/{kind}` service call, routed by the body's RM type,
/// returning the new versioned object.
pub(crate) async fn create_party(
    svc: &ferroehr::service::FerroEhrService,
    version: UpdateVersion<openehr_rm::prelude::Party>,
) -> Result<ferroehr::ids::VoId, ferroehr::service::error::ServiceError> {
    use ferroehr::service::demographic::types::PartyKind;
    use openehr_rm::prelude::Party;
    let committal = envelope_committal(&version);
    let body = version.data.expect("a create carries a PARTY");
    let kind = match &body {
        Party::Agent(_) => PartyKind::Agent,
        Party::Group(_) => PartyKind::Group,
        Party::Organisation(_) => PartyKind::Organisation,
        Party::Person(_) => PartyKind::Person,
        Party::Role(_) => PartyKind::Role,
    };
    let created = Box::pin(svc.party_create(kind, body, Some(committal))).await?;
    Ok(written_vo(&created))
}

/// Creates the `PARTY_RELATIONSHIP` an `UPDATE_VERSION` envelope carries
/// through the service call the REST relationship create makes, returning the
/// new versioned object.
pub(crate) async fn create_relationship(
    svc: &ferroehr::service::FerroEhrService,
    version: UpdateVersion<openehr_rm::prelude::PartyRelationship>,
) -> Result<ferroehr::ids::VoId, ferroehr::service::error::ServiceError> {
    let committal = envelope_committal(&version);
    let body = version.data.expect("a create carries a PARTY_RELATIONSHIP");
    let created = svc.party_relationship_create(body, Some(committal)).await?;
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
) -> ferroehr::ids::EhrId {
    Box::pin(svc.create_ehr_meta(
        Some(status_naming(
            &subject.id,
            &subject.namespace,
            &subject.r#type,
        )),
        None,
    ))
    .await
    .expect("the EHR naming its subject is created")
    .0
}

/// Creates, through the `POST /ehr` service call, the EHR whose subject is the
/// party `party` (its versioned-object id, as a `PERSON`).
pub(crate) async fn create_ehr_for_party(
    svc: &ferroehr::service::FerroEhrService,
    party: ferroehr::ids::VoId,
) -> ferroehr::ids::EhrId {
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
                    ferroehr::service::error::ServiceError::NotFound(
                        ferroehr::service::refusal::Refusal {
                            kind: ferroehr::service::refusal::Missing::Ehr,
                            ..
                        }
                    )
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
