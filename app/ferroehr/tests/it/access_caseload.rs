// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The caseload a query is narrowed to, end to end through the service and
//! the store (#6056): the caseload members `direct`, `team`, `all` and
//! `none` with `caseload_roles`, read from the care copy and from the
//! clinical care projection alike, with `COUNT` and paging seeing only the
//! narrowed rows, the decision still refusing what it refuses, and the
//! probe on its index (draft access control, Queries and the Caseload).

use std::sync::Arc;

use serde_json::{Value, json};

use crate::access_eval_examples::{
    MEMBER, access, care, care_of, party, person, policy, relate_as_adt, update_as_adt,
};
use crate::fixtures::{composition, create_ehr_for_party, uv};
use crate::policy_fixtures::{
    CARE_PROVIDER, care_provider, care_writer, create, granted, granting,
};
use ferroehr::access_eval::caseload::{Ask, Path};
use ferroehr::access_eval::condition::{RequestAccess, with_request_access};
use ferroehr::error::ServiceError;
use ferroehr::ids::{EhrId, VoId};
use ferroehr::party_binding::policy_mark::TERMINOLOGY;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::query::request::AqlQueryRequest;

/// The compositions a query reads.
const COMPOSITIONS: &str =
    "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c ORDER BY c/uid/value";
/// Their count.
const COUNT: &str = "SELECT COUNT(c/uid/value) FROM EHR e CONTAINS COMPOSITION c";

/// A direct care relationship from `source` to the `PERSON` `carer` in the
/// role `role` of `HL7_ParticipationType`.
fn direct(source: VoId, carer: VoId, role: &str) -> Value {
    let mut body = care_of(source, carer, "2026-11-01", Some("2026-12-31"));
    body["target"]["type"] = json!("PERSON");
    body["details"]["items"][0]["value"] = json!({ "_type": "DV_CODED_TEXT", "value": role,
        "defining_code": { "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "HL7_ParticipationType" },
            "code_string": role } });
    body
}

/// The ward: Dr D, a member of cardiology, is the responsible practitioner of
/// Mr A directly; Ms B is under cardiology; Mr C is under nobody's care.
struct Ward {
    db: testkit::TestDb,
    svc: FerroEhrService,
    doctor: VoId,
    /// The EHR and composition of Mr A, Ms B and Mr C.
    records: Vec<(EhrId, String)>,
}

impl Ward {
    async fn new() -> Self {
        let db = testkit::db().await.expect("testkit database");
        let svc = granting(FerroEhrService::new(
            &ferroehr::db::domain::DomainPools::from_shared(&db.pool()),
        ))
        .await;
        granted(&svc, "adt", &[care_writer()]).await;
        let provider = care_provider(&svc, CARE_PROVIDER).await;
        let group = party(
            "GROUP",
            "openEHR-DEMOGRAPHIC-GROUP.ward.v1",
            "Cardiology",
            &[],
        );
        let (cardiology, _) = crate::policy_fixtures::provider_team(&svc, provider, &group).await;
        let (doctor, uid) = create(&svc, PartyKind::Person, &person("Dr D", &[]))
            .await
            .expect("the doctor commits");
        let roster = person(
            "Dr D",
            &[care(MEMBER, doctor, cardiology, "2026-10-01", None)],
        );
        update_as_adt(&svc, (doctor, &uid), &roster).await;
        let mut records = Vec::new();
        for name in ["Mr A", "Ms B", "Mr C"] {
            let (patient, _) = create(&svc, PartyKind::Person, &person(name, &[]))
                .await
                .expect("the patient commits");
            match name {
                "Mr A" => drop(relate_as_adt(&svc, &direct(patient, doctor, "RESP")).await),
                "Ms B" => {
                    let stay = care_of(patient, cardiology, "2026-11-01", Some("2026-12-31"));
                    drop(relate_as_adt(&svc, &stay).await);
                }
                _ => {}
            }
            let ehr_id = create_ehr_for_party(&svc, patient).await;
            let written = svc
                .create_composition(ehr_id, uv(&composition(name), "249", None))
                .await
                .expect("the composition commits");
            records.push((ehr_id, written.vo_id.0.to_string()));
        }
        svc.grants().refresh().await.expect("the copy reads");
        Self {
            db,
            svc,
            doctor,
            records,
        }
    }

    /// Dr D's request access under an open policy with `members`.
    fn doctor(&self, members: &Value) -> Arc<RequestAccess> {
        access(
            &self.svc,
            self.doctor,
            policy(members),
            "2026-11-20T12:00:00Z".parse().expect("an instant"),
        )
    }

    /// The composition ids of the records at `indexes`, sorted.
    fn objects(&self, indexes: &[usize]) -> Vec<String> {
        let mut ids: Vec<String> = indexes
            .iter()
            .map(|index| self.records[*index].1.clone())
            .collect();
        ids.sort();
        ids
    }
}

/// The first column of `aql` run by `access` narrowed to `caseload`, from
/// `offset`, at most `fetch`.
async fn column(
    ward: &Ward,
    access: &Arc<RequestAccess>,
    aql: &str,
    caseload: Option<Ask>,
    page: Option<(i64, i64)>,
) -> Result<Vec<Value>, ServiceError> {
    let svc = &ward.svc;
    let run = async {
        let request = AqlQueryRequest {
            ehr_access: ferroehr::service::query::request::QueryAccess::OfRequest,
            offset: page.map(|(offset, _)| offset),
            fetch: page.map(|(_, fetch)| fetch),
            caseload,
            ..AqlQueryRequest::default()
        };
        let outcome = Box::pin(svc.execute_aql(aql, None, &request)).await?;
        Ok(outcome.result_set.to_value().unwrap()["rows"]
            .as_array()
            .map(|rows| rows.iter().map(|row| row[0].clone()).collect())
            .unwrap_or_default())
    };
    Box::pin(with_request_access(Arc::clone(access), run)).await
}

/// The composition ids of [`COMPOSITIONS`] as `column` reads them.
async fn objects(
    ward: &Ward,
    access: &Arc<RequestAccess>,
    caseload: Option<Ask>,
    page: Option<(i64, i64)>,
) -> Vec<String> {
    column(ward, access, COMPOSITIONS, caseload, page)
        .await
        .expect("the query runs")
        .iter()
        .filter_map(Value::as_str)
        .map(|uid| uid.split("::").next().unwrap_or_default().to_owned())
        .collect()
}

/// A caseload of `path` in `roles`, as a request asks for it.
fn ask(path: Path, roles: &[(Option<&str>, &str)]) -> Option<Ask> {
    let roles: Vec<String> = roles
        .iter()
        .map(|(terminology, code)| match terminology {
            Some(terminology) => format!("{terminology}::{code}"),
            None => (*code).to_owned(),
        })
        .collect();
    let path = match path {
        Path::Direct => "direct",
        Path::Team => "team",
        Path::All => "all",
    };
    ferroehr::access_eval::caseload::read(
        Some(path),
        (!roles.is_empty()).then_some(roles.as_slice()),
    )
    .expect("the caseload reads")
}

/// Each value narrows to its path and roles, read from the care copy and,
/// once the projector has run, from the projection alike; `COUNT` and every
/// page see the narrowed rows only.
#[tokio::test]
async fn each_caseload_value_narrows_and_counts_and_pages_agree() {
    let ward = Ward::new().await;
    let doctor = ward.doctor(&json!({}));
    let cases: Vec<(&str, Option<Ask>, Vec<usize>)> = vec![
        ("none", None, vec![0, 1, 2]),
        ("direct", ask(Path::Direct, &[]), vec![0]),
        ("team", ask(Path::Team, &[]), vec![1]),
        ("all", ask(Path::All, &[]), vec![0, 1]),
        (
            "direct in RESP",
            ask(Path::Direct, &[(None, "RESP")]),
            vec![0],
        ),
        (
            "all in HL7_ParticipationType::PART",
            ask(Path::All, &[(Some("HL7_ParticipationType"), "PART")]),
            vec![1],
        ),
        (
            "direct in ATND",
            ask(Path::Direct, &[(None, "ATND")]),
            vec![],
        ),
        (
            "all in local::RESP",
            ask(Path::All, &[(Some("local"), "RESP")]),
            vec![],
        ),
    ];
    for projected in [false, true] {
        if projected {
            ward.svc.project_care().await.expect("the projector runs");
            assert!(ward.svc.care_projection_current(), "the projection holds");
        }
        for (name, caseload, expected) in &cases {
            let case = format!("{name}, projected {projected}");
            let read = objects(&ward, &doctor, caseload.clone(), None).await;
            assert_eq!(read, ward.objects(expected), "{case}");
            let counted = column(&ward, &doctor, COUNT, caseload.clone(), None)
                .await
                .expect("the count runs");
            assert_eq!(counted, vec![json!(expected.len())], "{case}: the count");
            let mut paged = Vec::new();
            for offset in 0..=expected.len() {
                let offset = i64::try_from(offset).expect("an offset");
                paged.extend(objects(&ward, &doctor, caseload.clone(), Some((offset, 1))).await);
            }
            assert_eq!(paged, ward.objects(expected), "{case}: one row per page");
        }
    }
}

/// The caseload never widens: a record the decision refuses stays out under
/// `all`, and a query naming an EHR outside the caseload reads nothing of it.
#[tokio::test]
async fn the_caseload_never_widens_the_decision() {
    let ward = Ward::new().await;
    let deny = json!({ "_type": "ACCESS_DIRECTIVE",
        "name": { "_type": "DV_TEXT", "value": "No access to Mr A" },
        "archetype_node_id": "at0001",
        "kind": { "_type": "DV_CODED_TEXT", "value": "consent",
            "defining_code": { "terminology_id": { "value": TERMINOLOGY }, "code_string": "consent" } },
        "effect": { "_type": "DV_CODED_TEXT", "value": "deny",
            "defining_code": { "terminology_id": { "value": "HL7_ConsentProvisionType" },
                "code_string": "deny" } },
        "emergency_override": false,
        "applies_to": [{ "_type": "ACCESS_ACTOR_SCOPE",
            "party": { "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
                "id": { "_type": "HIER_OBJECT_ID", "value": ward.doctor.to_string() } } }] });
    let settings = json!({ "_type": "ACCESS_DIRECTIVE_SETTINGS",
        "default_access": { "_type": "DV_CODED_TEXT", "value": "open",
            "defining_code": { "terminology_id": { "value": TERMINOLOGY }, "code_string": "open" } },
        "directives": [deny] });
    let (mr_a, _) = &ward.records[0];
    let versions = ward
        .svc
        .ehr_access_versions(*mr_a)
        .await
        .expect("the versions read");
    ward.svc
        .commit_ehr_access_settings(*mr_a, &versions[0].0, settings)
        .await
        .expect("the controller commits the deny");
    let doctor = ward.doctor(&json!({}));
    assert_eq!(
        objects(&ward, &doctor, ask(Path::All, &[]), None).await,
        ward.objects(&[1]),
        "the deny holds under all"
    );
    let (mr_c, _) = &ward.records[2];
    let pinned = format!(
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c WHERE e/ehr_id/value = '{mr_c}'"
    );
    let read = column(&ward, &doctor, &pinned, ask(Path::All, &[]), None)
        .await
        .expect("the query runs");
    assert!(read.is_empty(), "an EHR outside the caseload: {read:?}");
}

/// A policy that turns the caseload off refuses it `400` naming the member.
#[tokio::test]
async fn a_policy_without_the_caseload_refuses_it() {
    let ward = Ward::new().await;
    let doctor = ward.doctor(&json!({ "caseload": false }));
    let refused = column(&ward, &doctor, COMPOSITIONS, ask(Path::Direct, &[]), None)
        .await
        .expect_err("the caseload is off");
    assert!(
        matches!(refused, ServiceError::BadRequest(_)),
        "{refused:?}"
    );
    assert!(refused.to_string().contains("caseload"), "{refused}");
    assert_eq!(
        objects(&ward, &doctor, None, None).await.len(),
        3,
        "none still reads"
    );
}

/// The caseload probe of the projection starts from the caller's rows of the
/// caseload index (#6188) inside the query's one statement, and never scans
/// the projection whole.
#[tokio::test]
async fn the_projected_caseload_probe_starts_from_the_caseload_index() {
    let ward = Ward::new().await;
    ward.svc.project_care().await.expect("the projector runs");
    let ask = ask(Path::All, &[]).expect("a caseload");
    let caseload = ferroehr::access_eval::caseload::Caseload::Projected {
        care: ferroehr::access_eval::condition::care::ProjectedCare {
            principals: vec![ward.doctor.0].into(),
            direct: vec![ward.doctor.0].into(),
            provider_issuers: vec![CARE_PROVIDER.0.to_owned()].into(),
            provider_ids: vec![CARE_PROVIDER.1.to_owned()].into(),
            day: jiff::civil::date(2026, 11, 20),
            grace: "P0D".to_owned(),
            self_declared: false,
        },
        needs: ask.needs(),
    };
    let params = ferroehr::aql::ir::Params::default();
    let ast = openehr_query::parser::parse_str(COUNT).expect("the query parses");
    let ir = ferroehr::aql::plan(
        &ast,
        &params,
        ferroehr::config::profile::SpecProfile::default(),
    )
    .expect("the query plans");
    let ctx = ferroehr::aql::sql::SqlCtx {
        system_id: "caseload.example.org".to_owned(),
        ehr_ids: Vec::new(),
        ehr_scope_kind: ferroehr::aql::sql::EhrScopeKind::Named,
        ehr_access: ferroehr::access_eval::condition::AccessCondition::default(),
        limit: None,
        offset: None,
        archetype_lineage: Arc::default(),
        policy_parties: ferroehr::aql::sql::readers::PolicyParties::Served,
        access_settings: ferroehr::aql::sql::readers::EhrAccessReaders::Every,
        own_records: None,
        caseload: Some(Arc::new(caseload)),
    };
    let prepared = ferroehr::aql::sql::build(&ir, &params, &ctx).expect("the AQL lowers");
    assert!(prepared.sql.contains("access_care"), "{}", prepared.sql);
    let clinical =
        ferroehr::db::domain_pool_from(&ward.db.pool(), ferroehr::db::domain::Domain::Clinical);
    let mut tx = ferroehr::db::acquire::acquire(&clinical)
        .await
        .expect("a connection");
    // NOTE: our own design, three EHRs would be read by a sequential scan, so the plan test
    // asks the planner for the shape it takes at the scale the index exists for.
    sqlx::query("SET enable_seqscan = off")
        .execute(&mut *tx)
        .await
        .expect("the planner setting");
    let plan: Value = sqlx::query_scalar_with(
        sqlx::AssertSqlSafe(format!("EXPLAIN (FORMAT JSON) {}", prepared.sql)),
        prepared.values,
    )
    .fetch_one(&mut *tx)
    .await
    .expect("explain the query");
    let text = plan.to_string();
    assert!(
        text.contains("ix_access_care_caseload"),
        "the probe starts from the caseload index: {text}"
    );
    assert!(
        !scans_whole(&plan, "access_care"),
        "the probe never scans the projection whole: {text}"
    );
}

/// Whether a node of the JSON plan `plan` is a sequential scan of `relation`.
fn scans_whole(plan: &Value, relation: &str) -> bool {
    match plan {
        Value::Object(node) => {
            let seq = node.get("Node Type").and_then(Value::as_str) == Some("Seq Scan")
                && node.get("Relation Name").and_then(Value::as_str) == Some(relation);
            seq || node.values().any(|child| scans_whole(child, relation))
        }
        Value::Array(nodes) => nodes.iter().any(|child| scans_whole(child, relation)),
        _ => false,
    }
}
