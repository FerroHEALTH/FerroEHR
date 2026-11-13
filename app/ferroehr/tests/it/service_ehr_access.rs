// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `access_directives` scheme of `EHR_ACCESS` end to end against a real
//! `PostgreSQL` 18 (shared testkit harness): who commits a new `EHR_ACCESS`
//! version, who reads it through AQL, an EHR Extract and the CONTRIBUTION
//! read, and the strict check of every written body.
//!
//! `EHR_ACCESS` is a mandatory, versioned RM object and the access-decision
//! authority (RM `org.openehr.rm.ehr.ehr_access.adoc` §`EHR_ACCESS` Class).
//! BASE Architecture Overview §Security Policy, "Access control of access
//! settings", names the gate-keeper; EHDS Art. 8 keeps a restriction
//! invisible to a provider. The scheme and the rules are our own design.

use ferroehr::error::ServiceError;
use ferroehr::ids::EhrId;
use ferroehr::party_binding::BindingMapping;
use ferroehr::party_binding::policy_mark::TERMINOLOGY;
use ferroehr::service::query::request::AqlQueryRequest;
use ferroehr::service::{DEFAULT_SYSTEM_ID, FerroEhrService};
use ferroehr::versioning::committer::CommitterIdentity;
use ferroehr::versioning::committer::bound::{BoundDetails, PortalParty};
use serde_json::{Value, json};

use crate::fixtures::change_type;

/// The `PERSON` of the subject's guardian, a gate-keeper.
const GUARDIAN: uuid::Uuid = uuid::Uuid::from_u128(0x0190_b3a2_5a3e_7c1d_9e2f_3b4c_5d6e_7f80);
/// The `PERSON` of a representative no gate-keeper names.
const STRANGER: uuid::Uuid = uuid::Uuid::from_u128(0x0190_b3a2_5a3e_7c1d_9e2f_3b4c_5d6e_7f81);

/// A caller of the running request.
#[derive(Clone)]
struct Caller {
    identity: Option<CommitterIdentity>,
    roles: Vec<String>,
}

/// A caller authenticated as `login`, a professional bound to no party.
fn identity(login: &str) -> CommitterIdentity {
    CommitterIdentity {
        subject: login.to_owned(),
        id_type: "basic",
        issuer: None,
        client: None,
        person: false,
        party: None,
        representative: false,
        bound: None,
        system: false,
        agent: None,
        external: None,
    }
}

fn professional() -> Caller {
    Caller {
        identity: Some(identity("doctor")),
        roles: vec!["USER".to_owned()],
    }
}

fn administrator() -> Caller {
    Caller {
        identity: Some(identity("root")),
        roles: vec!["ADMIN".to_owned()],
    }
}

/// The record's person through a patient-context credential.
fn subject() -> Caller {
    Caller {
        identity: Some(CommitterIdentity {
            person: true,
            ..identity("patient")
        }),
        roles: Vec::new(),
    }
}

/// A representative the server bound to the `PERSON` `person`.
fn representative(person: uuid::Uuid) -> Caller {
    let login = format!("rep-{person}");
    Caller {
        identity: Some(CommitterIdentity {
            person: true,
            representative: true,
            bound: Some(BoundDetails {
                namespace: "demographic".to_owned(),
                portal: Some(PortalParty {
                    person,
                    relationship: Some("10".to_owned()),
                    login_issuer: "ferroehr".to_owned(),
                    login: login.clone(),
                }),
                ..BoundDetails::default()
            }),
            ..identity(&login)
        }),
        roles: Vec::new(),
    }
}

/// The server itself: no request caller.
fn server() -> Caller {
    Caller {
        identity: None,
        roles: Vec::new(),
    }
}

/// Runs `fut` as `caller`.
async fn as_caller<F: Future>(caller: &Caller, fut: F) -> F::Output {
    Box::pin(ferroehr::system_log::access_context::with_roles(
        caller.roles.clone(),
        ferroehr::versioning::committer::with_committer(caller.identity.clone(), Box::pin(fut)),
    ))
    .await
}

/// A term of the access control groups.
fn term(code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": TERMINOLOGY },
                "code_string": code } })
}

/// Restricted settings whose gate-keepers are the subject and the guardian.
fn kept() -> Value {
    json!({
        "_type": "ACCESS_DIRECTIVE_SETTINGS",
        "default_access": term("restricted"),
        "gate_keepers": [
            { "_type": "PARTY_SELF" },
            { "_type": "PARTY_RELATED",
              "external_ref": { "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
                                "id": { "_type": "HIER_OBJECT_ID", "value": GUARDIAN.to_string() } },
              "relationship": { "_type": "DV_CODED_TEXT", "value": "guardian",
                  "defining_code": { "_type": "CODE_PHRASE",
                      "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                      "code_string": "10" } } }
        ]
    })
}

/// A service whose administrator role is `ADMIN`, and a fresh EHR.
async fn setup() -> (testkit::TestDb, FerroEhrService, EhrId) {
    let (db, svc) = crate::fixtures::service().await;
    let svc = svc.with_binding_mapping(BindingMapping::default().with_admin_role("ADMIN"));
    let ehr_id = crate::fixtures::create_ehr(&svc).await;
    (db, svc, ehr_id)
}

/// The placeholder committer naming `caller`'s login.
fn placeholder(caller: &Caller) -> Value {
    let login = caller
        .identity
        .as_ref()
        .map_or("system", |identity| identity.subject.as_str());
    json!({ "_type": "PARTY_IDENTIFIED",
            "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": "ferroehr", "id": login }] })
}

/// Commits, as `caller`, an `EHR_ACCESS` version carrying `settings` over the
/// current version `preceding`; the committed CONTRIBUTION's uid.
async fn write(
    svc: &FerroEhrService,
    ehr_id: EhrId,
    caller: &Caller,
    preceding: u32,
    settings: &Value,
) -> Result<String, ServiceError> {
    let ehr = svc.ehr_read(ehr_id).await.expect("ehr object").body;
    let access_vo = ehr["ehr_access"]["id"]["value"]
        .as_str()
        .expect("EHR.ehr_access.id.value")
        .to_owned();
    let audit = json!({ "change_type": change_type("251", "modification"),
                        "committer": placeholder(caller) });
    let archetype = "openEHR-EHR-EHR_ACCESS.access_directives.v1";
    let contribution = json!({
        "_type": "CONTRIBUTION",
        "versions": [ {
            "_type": "ORIGINAL_VERSION",
            "commit_audit": audit,
            "lifecycle_state": change_type("532", "complete"),
            "preceding_version_uid": {
                "value": format!("{access_vo}::{DEFAULT_SYSTEM_ID}::{preceding}")
            },
            "data": {
                "_type": "EHR_ACCESS",
                "archetype_node_id": archetype,
                "archetype_details": {
                    "_type": "ARCHETYPED",
                    "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
                    "rm_version": "1.2.0"
                },
                "name": { "_type": "DV_TEXT", "value": "EHR Access" },
                "settings": settings
            }
        } ],
        "audit": audit
    });
    let committed = as_caller(caller, svc.create_ehr_contribution(ehr_id, contribution)).await?;
    Ok(committed.body["uid"]["value"]
        .as_str()
        .expect("the contribution uid")
        .to_owned())
}

/// Asserts `outcome` is a `403`.
fn forbidden(outcome: &Result<String, ServiceError>, who: &str) {
    assert!(
        matches!(outcome, Err(ServiceError::Forbidden(_))),
        "{who} is refused 403: {outcome:?}"
    );
}

/// Every writer kind: the administrator and each gate-keeper the current
/// settings name commit; the subject before they are named, an unnamed
/// representative and a professional are refused `403`.
#[tokio::test]
async fn each_writer_kind_is_decided_by_the_gate_keepers() {
    let (_db, svc, ehr_id) = setup().await;
    forbidden(
        &write(&svc, ehr_id, &subject(), 1, &kept()).await,
        "the subject no settings name",
    );
    forbidden(
        &write(&svc, ehr_id, &professional(), 1, &kept()).await,
        "a professional",
    );
    write(&svc, ehr_id, &administrator(), 1, &kept())
        .await
        .expect("the administrator writes the first settings");
    write(&svc, ehr_id, &subject(), 2, &kept())
        .await
        .expect("the subject named PARTY_SELF writes");
    write(&svc, ehr_id, &representative(GUARDIAN), 3, &kept())
        .await
        .expect("the named guardian writes");
    forbidden(
        &write(&svc, ehr_id, &representative(STRANGER), 4, &kept()).await,
        "an unnamed representative",
    );
    forbidden(
        &write(&svc, ehr_id, &professional(), 4, &kept()).await,
        "a professional no gate-keeper names",
    );
    write(&svc, ehr_id, &server(), 4, &kept())
        .await
        .expect("the server's own write");
    let settings = svc
        .current_ehr_access_settings(ehr_id)
        .await
        .expect("the settings read");
    assert_eq!(
        (*settings).as_ref().map(|s| s.gate_keepers().len()),
        Some(2),
        "the committed settings are current"
    );
}

/// A body of the removed scheme, and one breaking an invariant, are refused
/// `400` naming the defect, even to the administrator.
#[tokio::test]
async fn a_body_of_the_old_scheme_or_breaking_an_invariant_is_refused() {
    let (_db, svc, ehr_id) = setup().await;
    let old = json!({ "_type": "FERROEHR_ACCESS_CONTROL_V1", "default_access": "restricted" });
    let refused = write(&svc, ehr_id, &administrator(), 1, &old).await;
    assert!(
        matches!(&refused, Err(ServiceError::BadRequest(r))
            if r.message.contains("FERROEHR_ACCESS_CONTROL_V1")),
        "the old scheme is a 400: {refused:?}"
    );
    let broken = json!({ "_type": "ACCESS_DIRECTIVE_SETTINGS", "default_access": term("closed") });
    let refused = write(&svc, ehr_id, &administrator(), 1, &broken).await;
    assert!(
        matches!(&refused, Err(ServiceError::BadRequest(r))
            if r.message.contains("Default_access_valid")),
        "a broken invariant is a 400: {refused:?}"
    );
}

/// How many `EHR_ACCESS` rows of `ehr_id` an AQL query answers `caller`.
async fn aql_rows(svc: &FerroEhrService, ehr_id: EhrId, caller: &Caller) -> usize {
    let aql =
        format!("SELECT a/uid/value FROM EHR e[ehr_id/value='{ehr_id}'] CONTAINS EHR_ACCESS a");
    let outcome = as_caller(
        caller,
        svc.execute_aql(&aql, None, &AqlQueryRequest::default()),
    )
    .await
    .expect("the query runs");
    outcome.result_set["rows"].as_array().map_or(0, Vec::len)
}

/// Whether an EHR Extract exported for `caller` carries the `EHR_ACCESS`.
async fn extract_carries(svc: &FerroEhrService, ehr_id: EhrId, caller: &Caller) -> bool {
    let extracts = as_caller(caller, svc.extract_ehrs(ehr_id))
        .await
        .expect("the export runs");
    extracts
        .iter()
        .any(|extract| extract.to_string().contains("X_VERSIONED_EHR_ACCESS"))
}

/// Every reader kind: the administrator, the subject and a gate-keeper read
/// the settings through AQL, an EHR Extract and the CONTRIBUTION; for a
/// professional and an unnamed representative they are absent, and the
/// CONTRIBUTION is unknown.
#[tokio::test]
async fn each_reader_kind_reads_or_finds_nothing() {
    let (_db, svc, ehr_id) = setup().await;
    let contribution = write(&svc, ehr_id, &administrator(), 1, &kept())
        .await
        .expect("the administrator writes");
    let contribution: uuid::Uuid = contribution.parse().expect("a uuid");
    for (who, caller, reads) in [
        ("the administrator", administrator(), true),
        ("the subject", subject(), true),
        ("the named guardian", representative(GUARDIAN), true),
        ("a professional", professional(), false),
        ("an unnamed representative", representative(STRANGER), false),
    ] {
        assert_eq!(
            aql_rows(&svc, ehr_id, &caller).await,
            usize::from(reads),
            "{who}: AQL"
        );
        assert_eq!(
            extract_carries(&svc, ehr_id, &caller).await,
            reads,
            "{who}: extract"
        );
        let read = as_caller(&caller, svc.ehr_contribution(ehr_id, contribution, true)).await;
        if reads {
            read.expect("the contribution reads");
        } else {
            assert!(
                matches!(read, Err(ServiceError::NotFound(_))),
                "{who}: the contribution is unknown: {read:?}"
            );
        }
    }
}

/// An EHR left without an `EHR_ACCESS` gets the default one at boot, and an
/// EHR that holds one is left alone.
#[tokio::test]
async fn a_missing_ehr_access_is_restored_at_boot() {
    let (db, svc, ehr_id) = setup().await;
    let other = crate::fixtures::create_ehr(&svc).await;
    let pool = db.pool();
    let vo: uuid::Uuid =
        sqlx::query_scalar("SELECT vo_id FROM vo_head WHERE ehr_id = $1 AND kind = 'EHR_ACCESS'")
            .bind(ehr_id)
            .fetch_one(&pool)
            .await
            .expect("the EHR_ACCESS head");
    for statement in [
        "DELETE FROM version WHERE vo_id = $1",
        "DELETE FROM vo_head WHERE vo_id = $1",
    ] {
        sqlx::query(statement)
            .bind(vo)
            .execute(&pool)
            .await
            .expect("drop the EHR_ACCESS");
    }
    assert_eq!(svc.restore_default_ehr_access().await.expect("restored"), 1);
    assert_eq!(
        svc.restore_default_ehr_access().await.expect("none left"),
        0
    );
    for ehr in [ehr_id, other] {
        let read = svc.ehr_read(ehr).await.expect("the EHR reads").body;
        assert!(
            read["ehr_access"]["id"]["value"].is_string(),
            "{ehr} holds an EHR_ACCESS"
        );
    }
}

/// The migration drops an `EHR_ACCESS` any version of which carries the
/// removed scheme, whole, and leaves every other EHR's alone; the boot then
/// restores the default.
#[tokio::test]
async fn the_migration_drops_an_ehr_access_of_the_removed_scheme() {
    let (db, svc, ehr_id) = setup().await;
    let other = crate::fixtures::create_ehr(&svc).await;
    let pool = db.pool();
    let head = |ehr: EhrId| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, uuid::Uuid>(
                "SELECT vo_id FROM vo_head WHERE ehr_id = $1 AND kind = 'EHR_ACCESS'",
            )
            .bind(ehr)
            .fetch_optional(&pool)
            .await
            .expect("the head reads")
        }
    };
    let old = head(ehr_id).await.expect("an EHR_ACCESS");
    sqlx::query(
        "UPDATE version SET body = jsonb_set(body::jsonb, '{settings}', \
         '{\"_type\": \"FERROEHR_ACCESS_CONTROL_V1\", \"default_access\": \"restricted\"}')::text \
         WHERE vo_id = $1",
    )
    .bind(old)
    .execute(&pool)
    .await
    .expect("store a body of the removed scheme");
    let kept = head(other).await;

    let mut tx = pool.begin().await.expect("a transaction");
    sqlx::raw_sql(include_str!(
        "../../migrations/clinical/0037_drop_access_control_v1.sql"
    ))
    .execute(&mut *tx)
    .await
    .expect("the migration runs");
    tx.commit().await.expect("the migration commits");

    assert_eq!(
        head(ehr_id).await,
        None,
        "the old scheme's EHR_ACCESS is dropped"
    );
    let versions: i64 = sqlx::query_scalar("SELECT count(*) FROM version WHERE vo_id = $1")
        .bind(old)
        .fetch_one(&pool)
        .await
        .expect("the versions count");
    assert_eq!(versions, 0, "with every version");
    assert_eq!(head(other).await, kept, "another EHR's EHR_ACCESS stays");
    assert_eq!(svc.restore_default_ehr_access().await.expect("restored"), 1);
    assert!(head(ehr_id).await.is_some(), "the default is restored");
}

/// The default `EHR_ACCESS` definition of the scheme is stored at boot when
/// absent and kept at every later boot.
#[tokio::test]
async fn the_default_definition_is_stored_at_boot_only_when_absent() {
    use ferroehr::service::demographic::default_definitions::Seeded;
    let (_db, svc, _ehr_id) = setup().await;
    let ids = [
        "openEHR-EHR-EHR_ACCESS.access_directives.v1.0.0",
        "openEHR-EHR-EHR_ACCESS.access_directives.v1.1.0",
    ];
    let first = svc
        .seed_default_definitions()
        .await
        .expect("the first boot seeds");
    let again = svc
        .seed_default_definitions()
        .await
        .expect("the next boot seeds");
    for id in ids {
        assert!(
            first.contains(&Seeded::Stored(id.to_owned())),
            "{id} is stored: {first:?}"
        );
        assert!(
            again.contains(&Seeded::Kept(format!("{id}.adls"))),
            "{id} is kept: {again:?}"
        );
    }
}

/// The ADL 2 errors of the shipped `v1.1.0` definition against `rm`.
fn errors_against(rm: &dyn openehr_adl::validate::rm::RmModel) -> Vec<String> {
    use openehr_adl::validate::bindings::NoTerminologyResolver;
    use openehr_adl::validate::catalogue::Severity;
    let (_, text) = ferroehr::service::ehr::access_settings::DEFINITIONS[1];
    openehr_adl::validate::validate_source(text, None, rm, &NoTerminologyResolver)
        .expect("the definition parses")
        .into_iter()
        .filter(|issue| issue.severity == Severity::Error)
        .map(|issue| format!("{}: {}", issue.code.mnemonic(), issue.message))
        .collect()
}

/// The `v1.1.0` definition constrains the settings to the draft class
/// `ACCESS_DIRECTIVE_SETTINGS` and its attributes, and validates against the
/// RM extended by the draft packages, where the class conforms to the RM's
/// `ACCESS_CONTROL_SETTINGS` (AOM2 `master11-rm_adaptation.adoc`).
#[test]
fn the_settings_definition_validates_against_the_rm_with_the_drafts() {
    let errors = errors_against(&ferroehr::draft_model::RM_WITH_DRAFTS);
    assert!(errors.is_empty(), "{errors:?}");
}

/// RM 1.2.0 alone has no `ACCESS_DIRECTIVE_SETTINGS`, so the validator
/// without the draft packages refuses the same definition by its draft
/// classes (`VCORM`, AOM2 `master04.3` §Reference Model Type Matching).
#[test]
fn the_settings_definition_is_refused_by_the_rm_without_the_drafts() {
    let errors = errors_against(&openehr_adl::validate::rm::ProductionRmModel);
    assert!(
        errors.iter().any(|e| e.starts_with("VCORM") && e.contains("ACCESS_DIRECTIVE_SETTINGS")),
        "{errors:?}"
    );
}
