// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The controller's bounds on writing a care relationship (#6073; draft
//! access control decision procedure, Writing Relationships): a relationship
//! written without an end takes the default period of the first
//! `relationship_validity` entry that covers it, kept beside it with the
//! policy version the write confirmed and never written into the committed
//! version; one past the maximum, and an open one no entry covers, are
//! refused `422`; an `ORGANISATION` target needs `team_targets` and is never
//! the controller. A later policy version moves no kept end, and a refill of
//! the caller facts keeps it. An archive load judges each relationship it
//! lands against the policy in force there (#6123), and an erasure takes the
//! kept end. A provider that names no `ORGANISATION` and a `covers_for` that
//! names no party of its type, or the target itself, are refused `422` (#6156).
//! Our own design over our own model.

use serde_json::{Value, json};

use crate::policy_fixtures::{
    CARE_PROVIDER, as_admin, care_provider, create, deaf, granting,
};
use ferroehr::db::domain::{Domain, DomainPools};
use ferroehr::error::ServiceError;
use ferroehr::error::refusal::Invalid;
use ferroehr::ids::VoId;
use ferroehr::party_binding::policy_mark::TERMINOLOGY;
use ferroehr::service::FerroEhrService;
use ferroehr::service::admin::access_policy::PolicyPrecondition;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;

/// The controller every policy of these tests names.
const CONTROLLER: &str = "2c3d4e5f-6a7b-4c8d-9e0f-a1b2c3d4e5f7";

fn text(value: &str) -> Value {
    json!({ "_type": "DV_TEXT", "value": value })
}

fn coded(terminology: &str, code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": terminology },
                "code_string": code } })
}

fn party_ref(kind: &str, id: &str) -> Value {
    json!({ "_type": "PARTY_REF", "namespace": "demographic", "type": kind,
            "id": { "_type": "HIER_OBJECT_ID", "value": id } })
}

/// A policy whose `relationship_validity` gives the role `ATND` a default of
/// `default` within `P180D`, with `extra` members.
fn policy(default: &str, extra: &Value) -> Value {
    let archetype = ferroehr::access_policy::ARCHETYPE;
    let mut body = json!({
        "_type": "ACCESS_POLICY",
        "name": text("Hospital access policy"),
        "archetype_node_id": archetype,
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
            "rm_version": "1.2.0" },
        "controller": party_ref("ORGANISATION", CONTROLLER),
        "care_relationship_required": true,
        "purpose_required": false,
        "relationship_validity": [{ "_type": "ACCESS_RELATIONSHIP_VALIDITY",
            "roles": [coded("HL7_ParticipationType", "ATND")],
            "default_period": { "_type": "DV_DURATION", "value": default },
            "max_period": { "_type": "DV_DURATION", "value": "P180D" } }]
    });
    if let (Some(body), Some(extra)) = (body.as_object_mut(), extra.as_object()) {
        body.extend(extra.clone());
    }
    body
}

/// A `DV_INTERVAL<DV_DATE>` from `lower` to `upper`, each included; an upper
/// of `None` is written without an end.
fn interval(lower: &str, upper: Option<&str>) -> Value {
    let mut interval = json!({ "_type": "DV_INTERVAL",
        "lower": { "_type": "DV_DATE", "value": lower },
        "lower_unbounded": false, "upper_unbounded": upper.is_none(),
        "lower_included": true, "upper_included": upper.is_some() });
    if let Some(upper) = upper {
        interval["upper"] = json!({ "_type": "DV_DATE", "value": upper });
    }
    interval
}

/// A care relationship of its own from `source` to the `target_type`
/// `target` in the role `role` over `validity`.
fn care_of(
    source: VoId,
    (target_type, target): (&str, &str),
    role: &str,
    validity: &Value,
) -> Value {
    json!({
        "_type": "PARTY_RELATIONSHIP",
        "archetype_node_id": "org.example::openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.ward_stay.v3",
        "name": coded(TERMINOLOGY, "under_care_of"),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0002", "name": text("details"),
            "items": [
                { "_type": "ELEMENT", "archetype_node_id": "at0003",
                  "name": coded(TERMINOLOGY, "care_role"),
                  "value": coded("HL7_ParticipationType", role) },
                { "_type": "ELEMENT", "archetype_node_id": "at0004",
                  "name": coded(TERMINOLOGY, "provider"),
                  "value": { "_type": "DV_IDENTIFIER", "issuer": "urn:example:providers",
                             "id": "hospital", "type": "ORGANISATION" } }] },
        "time_validity": validity,
        "source": party_ref("PERSON", &source.to_string()),
        "target": party_ref(target_type, target)
    })
}

/// A plain party of `rm_type` named `name`.
fn party(rm_type: &str, name: &str) -> Value {
    let archetype = format!("openEHR-DEMOGRAPHIC-{rm_type}.plain.v1");
    json!({
        "_type": rm_type, "archetype_node_id": archetype,
        "archetype_details": { "_type": "ARCHETYPED", "rm_version": "1.1.0",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype } },
        "name": text(name),
        "identities": [{ "_type": "PARTY_IDENTITY", "archetype_node_id": "at0001",
            "name": text("name"),
            "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0002", "name": text("name"),
                "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0003", "name": text("name"),
                            "value": text(name) }] } }]
    })
}

/// Commits the care relationship `body` as an administrator; its container
/// and version uid.
async fn relate(svc: &FerroEhrService, body: &Value) -> Result<(VoId, String), ServiceError> {
    let mut write = WriteRequest::new(crate::typed_body::typed(body));
    write.lifecycle_state = Some("532".to_owned());
    let created = as_admin(async { Box::pin(svc.party_relationship_create(write)).await }).await?;
    let uid = created.meta.as_ref().expect("a version").uid.clone();
    Ok((crate::fixtures::written_vo(&created), uid))
}

/// Asserts `result` is a `422` naming `path`: a relationship write answers
/// the flat content refusal whose detail opens with the path.
fn refused<T: std::fmt::Debug>(result: Result<T, ServiceError>, path: &str) {
    let error = result.expect_err("a 422");
    assert!(
        matches!(error, ServiceError::Unprocessable { .. }),
        "a 422 naming {path}, got {error:?}"
    );
    assert!(
        format!("{error:?}").contains(&format!("\"{path}: ")),
        "{path}: {error:?}"
    );
}

/// The end kept for the relationship `vo` and the policy version that set
/// it, and the upper bound of its caller fact.
async fn kept(pool: &sqlx::PgPool, vo: VoId) -> (Option<(String, String)>, Option<String>) {
    let end: Option<(String, String)> = sqlx::query_as(
        "SELECT effective_until::text, policy_version FROM party.binding_care_validity \
         WHERE vo_id = $1",
    )
    .bind(vo.0)
    .fetch_optional(pool)
    .await
    .expect("the side table reads");
    let upper: Option<Option<String>> = sqlx::query_scalar(
        "SELECT upper(validity)::text FROM party.binding_caller_fact \
         WHERE holder_id = $1 AND kind = 'under_care_of'",
    )
    .bind(vo.0)
    .fetch_optional(pool)
    .await
    .expect("the caller fact reads");
    (end, upper.flatten())
}

/// A replica over `pool`, as the server installs it.
async fn replica(pool: &sqlx::PgPool) -> FerroEhrService {
    granting(FerroEhrService::new(&DomainPools::from_shared(pool))).await
}

#[tokio::test]
async fn an_open_relationship_takes_the_default_period_of_the_policy_version_confirmed() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let writer = replica(&pool).await;
    let reader = replica(&pool).await;
    let _elsewhere = deaf(&[std::sync::Arc::clone(reader.access_policy().listened())]).await;
    let (patient, _) = create(&reader, PartyKind::Person, &party("PERSON", "Patient"))
        .await
        .expect("the patient");
    let (team, _) = create(&reader, PartyKind::Group, &party("GROUP", "Cardiology"))
        .await
        .expect("the team");
    let team = team.to_string();

    let open = care_of(
        patient,
        ("GROUP", &team),
        "ATND",
        &interval("2026-11-01", None),
    );
    refused(relate(&reader, &open).await, "/time_validity");

    let first = writer
        .put_access_policy(&PolicyPrecondition::Create, &policy("P30D", &json!({})))
        .await
        .expect("the first policy version")
        .version_uid;
    assert_eq!(
        reader.access_policy().listened().notifications(),
        0,
        "the reader heard nothing of the policy"
    );
    let (stay, version) = relate(&reader, &open).await.expect("an open stay commits");
    let (end, upper) = kept(&pool, stay).await;
    assert_eq!(
        end,
        Some(("2026-12-01".to_owned(), first.clone())),
        "its start plus P30D, set by the version the write confirmed"
    );
    assert_eq!(
        upper.as_deref(),
        Some("2026-12-01"),
        "the caller fact ends there"
    );
    let stored = reader
        .party_relationship_get(crate::fixtures::at(stay.to_string()), None)
        .await
        .expect("the stay reads");
    assert_eq!(
        stored.body["time_validity"], open["time_validity"],
        "the committed version stays as it was sent"
    );

    let second = writer
        .put_access_policy(
            &PolicyPrecondition::Replace(first.clone()),
            &policy("P60D", &json!({})),
        )
        .await
        .expect("the second policy version")
        .version_uid;
    let ferroehr_pool = DomainPools::from_shared(&pool);
    ferroehr::party_binding::fill::refill_grants(ferroehr_pool.get(Domain::Party))
        .await
        .expect("the caller facts refill");
    assert_eq!(
        kept(&pool, stay).await,
        (
            Some(("2026-12-01".to_owned(), first.clone())),
            Some("2026-12-01".to_owned())
        ),
        "a later policy version and a refill move no kept end"
    );
    let (later, _) = relate(
        &reader,
        &care_of(
            patient,
            ("GROUP", &team),
            "ATND",
            &interval("2026-11-10", None),
        ),
    )
    .await
    .expect("a later stay commits");
    assert_eq!(
        kept(&pool, later).await.0,
        Some(("2027-01-09".to_owned(), second)),
        "the write confirmed the second version, which no notification announced"
    );

    let mut closed = open.clone();
    closed["time_validity"] = interval("2026-11-01", Some("2026-11-20"));
    let mut write = WriteRequest::new(crate::typed_body::typed(&closed));
    write.lifecycle_state = Some("532".to_owned());
    write.preceding = Some(version.parse().expect("an OBJECT_VERSION_ID"));
    as_admin(async {
        Box::pin(reader.party_relationship_update(stay, crate::fixtures::if_match(&version), write))
            .await
    })
    .await
    .expect("the discharge commits");
    assert_eq!(
        kept(&pool, stay).await,
        (None, Some("2026-11-21".to_owned())),
        "a written end is the relationship's own"
    );
}

#[tokio::test]
async fn a_relationship_past_the_maximum_or_no_entry_covers_open_is_refused() {
    let (_db, svc) = crate::fixtures::service().await;
    let svc = granting(svc).await;
    svc.put_access_policy(&PolicyPrecondition::Create, &policy("P30D", &json!({})))
        .await
        .expect("the policy");
    let (patient, _) = create(&svc, PartyKind::Person, &party("PERSON", "Patient"))
        .await
        .expect("the patient");
    let (team, _) = create(&svc, PartyKind::Group, &party("GROUP", "Cardiology"))
        .await
        .expect("the team");
    let team = team.to_string();
    let long = care_of(
        patient,
        ("GROUP", &team),
        "ATND",
        &interval("2026-11-01", Some("2027-06-01")),
    );
    refused(relate(&svc, &long).await, "/time_validity");
    let uncovered = care_of(
        patient,
        ("GROUP", &team),
        "CON",
        &interval("2026-11-01", None),
    );
    refused(relate(&svc, &uncovered).await, "/time_validity");
    let ended = care_of(
        patient,
        ("GROUP", &team),
        "CON",
        &interval("2026-11-01", Some("2029-01-01")),
    );
    relate(&svc, &ended)
        .await
        .expect("a written end no entry covers stands");
}

#[tokio::test]
async fn an_organisation_target_needs_team_targets_and_is_never_the_controller() {
    let (_db, svc) = crate::fixtures::service().await;
    let svc = granting(svc).await;
    let (patient, _) = create(&svc, PartyKind::Person, &party("PERSON", "Patient"))
        .await
        .expect("the patient");
    let (ward, _) = create(
        &svc,
        PartyKind::Organisation,
        &party("ORGANISATION", "Ward 3"),
    )
    .await
    .expect("the unit");
    let ward = ward.to_string();
    let stay = |target: &str| {
        care_of(
            patient,
            ("ORGANISATION", target),
            "ATND",
            &interval("2026-11-01", Some("2026-11-20")),
        )
    };
    let first = svc
        .put_access_policy(&PolicyPrecondition::Create, &policy("P30D", &json!({})))
        .await
        .expect("the policy")
        .version_uid;
    refused(relate(&svc, &stay(&ward)).await, "/target");
    let units = json!({ "team_targets": [party_ref("ORGANISATION", &ward)] });
    svc.put_access_policy(&PolicyPrecondition::Replace(first), &policy("P30D", &units))
        .await
        .expect("the policy naming the unit");
    relate(&svc, &stay(&ward))
        .await
        .expect("a named unit is a target");
    refused(relate(&svc, &stay(CONTROLLER)).await, "/target");
}

#[tokio::test]
async fn an_archive_load_gives_an_open_relationship_the_end_of_the_policy_in_force() {
    let source_db = testkit::db().await.expect("testkit database");
    let source = replica(&source_db.pool()).await;
    source
        .put_access_policy(&PolicyPrecondition::Create, &policy("P30D", &json!({})))
        .await
        .expect("the source policy");
    let (patient, _) = create(&source, PartyKind::Person, &party("PERSON", "Patient"))
        .await
        .expect("the patient");
    let (team, _) = create(&source, PartyKind::Group, &party("GROUP", "Cardiology"))
        .await
        .expect("the team");
    let team = team.to_string();
    let open = care_of(
        patient,
        ("GROUP", &team),
        "ATND",
        &interval("2026-11-01", None),
    );
    let (stay, _) = relate(&source, &open).await.expect("an open stay commits");
    let dir = crate::admin_fixture::archive_dir();
    let exported = source
        .export_ehrs(
            dir.clone(),
            ferroehr::service::admin::types::ExportSpec::canonical_json(1),
        )
        .await
        .expect("the export");
    assert!(exported.iter().all(|r| r.dump_status), "{exported:?}");

    let target_db = testkit::db().await.expect("testkit database");
    let target_pool = target_db.pool();
    let target = replica(&target_pool).await;
    let in_force = target
        .put_access_policy(&PolicyPrecondition::Create, &policy("P60D", &json!({})))
        .await
        .expect("the target policy")
        .version_uid;
    let reports = target.load_ehrs(dir.clone()).await.expect("the load");
    assert!(reports.iter().all(|r| r.dump_status), "{reports:?}");
    assert_eq!(
        kept(&target_pool, stay).await,
        (
            Some(("2026-12-31".to_owned(), in_force)),
            Some("2026-12-31".to_owned())
        ),
        "the loaded stay takes its start plus the P60D of the policy in force, \
         and its caller fact ends there"
    );

    let bare_db = testkit::db().await.expect("testkit database");
    let bare_pool = bare_db.pool();
    let bare = replica(&bare_pool).await;
    let reports = bare.load_ehrs(dir.clone()).await.expect("the load");
    let refused: Vec<_> = reports.iter().filter(|r| !r.dump_status).collect();
    assert_eq!(refused.len(), 1, "{reports:?}");
    assert_eq!(refused[0].entity_id, stay.to_string());
    assert!(
        refused[0]
            .error
            .as_deref()
            .is_some_and(|e| e.starts_with("/time_validity: ")),
        "{refused:?}"
    );
    let landed: i64 = sqlx::query_scalar("SELECT count(*) FROM party.version WHERE vo_id = $1")
        .bind(stay.0)
        .fetch_one(&bare_pool)
        .await
        .expect("the versions count");
    assert_eq!(
        landed, 0,
        "an open stay no policy covers is not loaded, as no write takes it"
    );
    assert_eq!(kept(&bare_pool, stay).await, (None, None));
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn an_erasure_drops_the_kept_end() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = replica(&pool).await;
    svc.put_access_policy(&PolicyPrecondition::Create, &policy("P30D", &json!({})))
        .await
        .expect("the policy");
    let (patient, _) = create(&svc, PartyKind::Person, &party("PERSON", "Patient"))
        .await
        .expect("the patient");
    let (team, _) = create(&svc, PartyKind::Group, &party("GROUP", "Cardiology"))
        .await
        .expect("the team");
    let team = team.to_string();
    let open = care_of(
        patient,
        ("GROUP", &team),
        "ATND",
        &interval("2026-11-01", None),
    );
    let (stay, _) = relate(&svc, &open).await.expect("an open stay commits");
    assert!(kept(&pool, stay).await.0.is_some(), "the stay keeps an end");
    svc.physical_party_delete(patient)
        .await
        .expect("the patient is erased");
    assert_eq!(
        kept(&pool, stay).await,
        (None, None),
        "the erasure takes the stay, its caller fact and its kept end"
    );
}

/// The care relationship `body` with its provider's id set to `provider` and,
/// when given, the `covers_for` element `cover`.
fn with_elements(body: &Value, provider: &str, cover: Option<Value>) -> Value {
    let mut body = body.clone();
    let items = body["details"]["items"].as_array_mut().expect("items");
    items[1]["value"]["id"] = json!(provider);
    if let Some(cover) = cover {
        items.push(json!({ "_type": "ELEMENT", "archetype_node_id": "at0005",
                           "name": coded(TERMINOLOGY, "covers_for"), "value": cover }));
    }
    body
}

/// A `covers_for` naming the carer `id` of `kind` in the provider's form.
fn cover(kind: &str, id: &str) -> Value {
    json!({ "_type": "DV_IDENTIFIER", "issuer": "demographic", "id": id, "type": kind })
}

/// Commits `body` as a new version of the care relationship `vo` at `version`
/// as an administrator.
async fn amend(
    svc: &FerroEhrService,
    (vo, version): (VoId, &str),
    body: &Value,
) -> Result<(), ServiceError> {
    let mut write = WriteRequest::new(crate::typed_body::typed(body));
    write.lifecycle_state = Some("532".to_owned());
    write.preceding = Some(version.parse().expect("an OBJECT_VERSION_ID"));
    as_admin(async {
        Box::pin(svc.party_relationship_update(vo, crate::fixtures::if_match(version), write))
            .await
    })
    .await
    .map(|_| ())
}

/// Asserts `result` is the `422` of the party reference check at the
/// `covers_for` element.
fn refused_cover<T: std::fmt::Debug>(result: Result<T, ServiceError>) {
    match result {
        Err(ServiceError::Unprocessable {
            kind: Invalid::Content,
            violation,
        }) => assert!(
            violation
                .path()
                .is_some_and(|at| at.ends_with("PARTY_RELATIONSHIP.details.covers_for")),
            "{violation:?}"
        ),
        other => panic!("expected the 422 at covers_for, got {other:?}"),
    }
}

/// Asserts `result` is a `422` whose detail names the `covers_for` element.
fn refused_unread<T: std::fmt::Debug>(result: Result<T, ServiceError>, why: &str) {
    let error = result.expect_err("a 422");
    assert!(
        matches!(error, ServiceError::Unprocessable { .. }),
        "a 422, got {error:?}"
    );
    let detail = format!("{error:?}");
    assert!(
        detail.contains("covers_for") && detail.contains(why),
        "{why}: {detail}"
    );
}

#[tokio::test]
async fn a_provider_or_covers_for_naming_no_party_is_refused_on_create_and_update() {
    let (_db, svc) = crate::fixtures::service().await;
    let svc = granting(svc).await;
    svc.put_access_policy(&PolicyPrecondition::Create, &policy("P30D", &json!({})))
        .await
        .expect("the policy");
    care_provider(&svc, CARE_PROVIDER).await;
    let person = |name: &'static str| {
        let svc = &svc;
        async move {
            create(svc, PartyKind::Person, &party("PERSON", name))
                .await
                .expect("the person")
                .0
        }
    };
    let patient = person("Patient").await;
    let carer = person("Meijer").await.to_string();
    let covered = person("Jansen").await.to_string();
    let (team, _) = create(&svc, PartyKind::Group, &party("GROUP", "Cardiology"))
        .await
        .expect("the team");
    let team = team.to_string();
    let dangling = "0198f1f1-0000-7000-8000-00000000dead";
    let stay = care_of(
        patient,
        ("PERSON", &carer),
        "ATND",
        &interval("2026-11-01", Some("2026-11-20")),
    );
    let (hospital, covering) = (CARE_PROVIDER.1, Some(cover("PERSON", &covered)));

    refused(
        relate(&svc, &with_elements(&stay, "nowhere", None)).await,
        "/details",
    );
    refused_cover(relate(&svc, &with_elements(&stay, hospital, Some(cover("PERSON", dangling)))).await);
    refused_cover(relate(&svc, &with_elements(&stay, hospital, Some(cover("PERSON", &team)))).await);
    refused_unread(
        relate(&svc, &with_elements(&stay, hospital, Some(cover("PERSON", &carer)))).await,
        "names its target",
    );
    let untyped = json!({ "_type": "DV_IDENTIFIER", "issuer": "demographic", "id": covered });
    refused_unread(
        relate(&svc, &with_elements(&stay, hospital, Some(untyped))).await,
        "as the type",
    );

    let (vo, version) = relate(&svc, &with_elements(&stay, hospital, covering.clone()))
        .await
        .expect("a cover naming another held PERSON commits");
    refused(
        amend(&svc, (vo, &version), &with_elements(&stay, "nowhere", covering)).await,
        "/details",
    );
    refused_cover(
        amend(
            &svc,
            (vo, &version),
            &with_elements(&stay, hospital, Some(cover("ROLE", &covered))),
        )
        .await,
    );
    amend(&svc, (vo, &version), &with_elements(&stay, hospital, None))
        .await
        .expect("a cover can be dropped");
}
