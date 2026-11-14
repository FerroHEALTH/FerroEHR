// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access evaluator's condition in a query's one SQL statement, step by
//! step, against a real `PostgreSQL` 18 (shared testkit harness): the rows a
//! caller may not read never leave the database, so `COUNT`, `LIMIT` and
//! `OFFSET` see only the rows the caller reads.
//!
//! `EHR_ACCESS` is the access-decision authority of an EHR (RM ehr
//! `org.openehr.rm.ehr.ehr_access.adoc` §`EHR_ACCESS` Class); the decision
//! procedure and its condition are our own design (#5513).

use std::sync::Arc;

use ferroehr::access_eval::condition::{Asking, InForce, RequestAccess, with_request_access};
use ferroehr::access_eval::procedure::{Caller, Emergency};
use ferroehr::access_eval::scope::CategoryIndex;
use ferroehr::ids::{EhrId, VoId};
use ferroehr::party_binding::BindingMapping;
use ferroehr::party_binding::grants::caller::CallerFacts;
use ferroehr::party_binding::policy_mark::TERMINOLOGY;
use ferroehr::service::ehr::access_rule::SettingsCaller;
use ferroehr::service::query::request::{AqlQueryRequest, QueryOutcome};
use ferroehr::service::{DEFAULT_SYSTEM_ID, FerroEhrService};
use ferroehr::versioning::committer::CommitterIdentity;
use openehr_rm::draft::access_control::access_control::access_policy::AccessPolicy;
use openehr_rm::draft::access_control::terminology::{
    AccessDefault, CallerKind, ForeignCode, OperationClass,
};
use serde_json::{Value, json};

use crate::fixtures::{change_type, composition, create_ehr_for_party, uv};

/// The professional the tests ask as.
const NURSE: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f83";
/// The query every test pages through.
const COMPOSITIONS: &str =
    "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c ORDER BY c/uid/value";
/// The count of the same rows.
const COUNT: &str = "SELECT COUNT(c/uid/value) FROM EHR e CONTAINS COMPOSITION c";

/// Three EHRs, each with two compositions, the subject of each a `PERSON`.
struct World {
    _db: testkit::TestDb,
    svc: FerroEhrService,
    ehrs: Vec<(EhrId, uuid::Uuid, Vec<String>)>,
}

/// A term of the access control groups.
fn term(code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": TERMINOLOGY },
                "code_string": code } })
}

/// A coded value of `terminology`.
fn coded(terminology: &str, code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": terminology },
                "code_string": code } })
}

/// A directive of `kind` and `effect` with `extra` members.
fn directive(kind: &str, effect: &str, extra: &Value) -> Value {
    let mut d = json!({
        "_type": "ACCESS_DIRECTIVE",
        "name": { "_type": "DV_TEXT", "value": "Directive" },
        "archetype_node_id": "at0001",
        "kind": term(kind),
        "effect": term(effect),
        "emergency_override": false
    });
    if let (Some(d), Some(extra)) = (d.as_object_mut(), extra.as_object()) {
        d.extend(extra.clone());
    }
    d
}

/// An `OBJECT_REF` to the versioned composition `id`.
fn object_ref(id: &str) -> Value {
    json!({ "_type": "OBJECT_REF", "namespace": "local", "type": "VERSIONED_COMPOSITION",
            "id": { "_type": "HIER_OBJECT_ID", "value": id } })
}

/// The administrator, who commits the settings.
fn administrator() -> CommitterIdentity {
    CommitterIdentity {
        subject: "root".to_owned(),
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

impl World {
    /// The world, every EHR's settings absent.
    async fn new() -> Self {
        let (db, svc) = crate::fixtures::service().await;
        let svc = svc.with_binding_mapping(BindingMapping::default().with_admin_role("ADMIN"));
        let mut ehrs = Vec::new();
        for _ in 0..3 {
            let subject = uuid::Uuid::now_v7();
            let ehr_id = create_ehr_for_party(&svc, VoId(subject)).await;
            let mut objects = Vec::new();
            for name in ["first", "second"] {
                let written = svc
                    .create_composition(ehr_id, uv(&composition(name), "249", None))
                    .await
                    .expect("the composition commits");
                objects.push(written.vo_id.0.to_string());
            }
            objects.sort();
            ehrs.push((ehr_id, subject, objects));
        }
        Self { _db: db, svc, ehrs }
    }

    /// Commits `settings` as the current `EHR_ACCESS` of EHR `index`.
    async fn settings(&self, index: usize, settings: &Value) {
        let ehr_id = self.ehrs[index].0;
        let ehr = self.svc.ehr_read(ehr_id).await.expect("the EHR").body;
        let access_vo = ehr["ehr_access"]["id"]["value"]
            .as_str()
            .expect("EHR.ehr_access")
            .to_owned();
        let audit = json!({ "change_type": change_type("251", "modification"),
            "committer": { "_type": "PARTY_IDENTIFIED",
                "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": "ferroehr", "id": "root" }] } });
        let archetype = "openEHR-EHR-EHR_ACCESS.access_directives.v1";
        let contribution = json!({
            "_type": "CONTRIBUTION",
            "versions": [{
                "_type": "ORIGINAL_VERSION",
                "commit_audit": audit,
                "lifecycle_state": change_type("532", "complete"),
                "preceding_version_uid": { "value": format!("{access_vo}::{DEFAULT_SYSTEM_ID}::1") },
                "data": {
                    "_type": "EHR_ACCESS",
                    "archetype_node_id": archetype,
                    "archetype_details": { "_type": "ARCHETYPED",
                        "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
                        "rm_version": "1.2.0" },
                    "name": { "_type": "DV_TEXT", "value": "EHR Access" },
                    "settings": settings
                }
            }],
            "audit": audit
        });
        Box::pin(ferroehr::system_log::access_context::with_roles(
            vec!["ADMIN".to_owned()],
            ferroehr::versioning::committer::with_committer(
                Some(administrator()),
                Box::pin(self.svc.create_ehr_contribution(ehr_id, contribution)),
            ),
        ))
        .await
        .expect("the settings commit");
    }

    /// Every composition of EHR `index`.
    fn objects(&self, index: usize) -> Vec<String> {
        self.ehrs[index].2.clone()
    }

    /// Every composition, sorted.
    fn every(&self) -> Vec<String> {
        let mut all: Vec<String> = self.ehrs.iter().flat_map(|e| e.2.clone()).collect();
        all.sort();
        all
    }

    /// The subject of EHR `index`.
    fn subject(&self, index: usize) -> String {
        self.ehrs[index].1.to_string()
    }
}

/// What one query asks as.
struct Ask {
    caller: Caller,
    purpose: Option<&'static str>,
    policy: Option<AccessPolicy>,
    cared: Vec<String>,
    default: AccessDefault,
}

impl Ask {
    /// A professional bound to `NURSE`, asking with no purpose.
    fn professional() -> Self {
        Self::of(CallerKind::Professional)
    }

    /// A caller of `kind` bound to `NURSE`.
    fn of(kind: CallerKind) -> Self {
        Self {
            caller: Caller {
                kind,
                party: Some(NURSE.to_owned()),
                acts_for: Vec::new(),
                roles: Vec::new(),
                facts: CallerFacts::default(),
                administrator: false,
                settings: SettingsCaller::new(false, false, Some(NURSE.to_owned())),
            },
            purpose: None,
            policy: None,
            cared: Vec::new(),
            default: AccessDefault::Open,
        }
    }
}

/// The evaluator of a query `ask` runs.
fn request_of(ask: &Ask) -> Arc<RequestAccess> {
    Arc::new(RequestAccess::new(
        ask.caller.clone(),
        Asking {
            operation: OperationClass::Query,
            purpose: ask.purpose.map(str::to_owned),
            emergency: Emergency::default(),
        },
        InForce {
            policy: ask.policy.clone().map(Arc::new),
            categories: CategoryIndex::default(),
            server_default: ask.default,
            now: jiff::Timestamp::now(),
            grants: None,
        },
    ))
}

/// The outcome of `aql` run by `access` as `ask`, from `offset`, at most
/// `fetch`.
async fn run_query(
    world: &World,
    ask: &Ask,
    access: &Arc<RequestAccess>,
    aql: &str,
    page: Option<(i64, i64)>,
) -> QueryOutcome {
    let svc = &world.svc;
    let run = async {
        let index = svc
            .ehr_access_settings_index()
            .await
            .expect("the index reads");
        let request = AqlQueryRequest {
            ehr_access: access.condition(&index, ask.cared.clone()),
            offset: page.map(|(offset, _)| offset),
            fetch: page.map(|(_, fetch)| fetch),
            ..AqlQueryRequest::default()
        };
        Box::pin(svc.execute_aql(aql, None, &request))
            .await
            .expect("the query runs")
    };
    Box::pin(with_request_access(Arc::clone(access), run)).await
}

/// The rows of `aql` `ask` reads, from `offset`, at most `fetch`.
async fn rows(world: &World, ask: &Ask, aql: &str, page: Option<(i64, i64)>) -> Vec<Value> {
    let outcome = run_query(world, ask, &request_of(ask), aql, page).await;
    outcome.result_set["rows"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// The composition ids `ask` reads, each page of one row in turn, and checks
/// that they are `expected`, that one unpaged query reads the same, and that
/// `COUNT` counts them.
async fn assert_reads(world: &World, ask: &Ask, expected: &[String], case: &str) {
    let ids = |rows: Vec<Value>| -> Vec<String> {
        rows.iter()
            .filter_map(|row| row[0].as_str())
            .map(|uid| uid.split("::").next().unwrap_or_default().to_owned())
            .collect()
    };
    let whole = ids(rows(world, ask, COMPOSITIONS, None).await);
    assert_eq!(whole, expected, "{case}: the rows");
    let mut paged = Vec::new();
    for offset in 0..=i64::try_from(expected.len()).expect("a count") {
        paged.extend(ids(rows(world, ask, COMPOSITIONS, Some((offset, 1))).await));
    }
    assert_eq!(paged, expected, "{case}: one row per page");
    let counted = rows(world, ask, COUNT, None).await;
    assert_eq!(
        counted[0][0],
        json!(expected.len()),
        "{case}: the count counts only readable rows"
    );
}

/// Settings of `default` holding `directives` and `labels`.
fn settings(default: &str, directives: &[Value], labels: &[Value]) -> Value {
    let mut s = json!({ "_type": "ACCESS_DIRECTIVE_SETTINGS", "default_access": term(default) });
    if !directives.is_empty() {
        s["directives"] = json!(directives);
    }
    if !labels.is_empty() {
        s["labels"] = json!(labels);
    }
    s
}

/// `world`'s compositions less those of the EHRs `without` and the objects
/// `hidden`.
fn except(world: &World, without: &[usize], hidden: &[&String]) -> Vec<String> {
    let dropped: Vec<String> = without.iter().flat_map(|i| world.objects(*i)).collect();
    world
        .every()
        .into_iter()
        .filter(|id| !dropped.contains(id) && !hidden.contains(&id))
        .collect()
}

/// Step 1: an opted-out EHR contributes no row to an access service, and
/// every row to a professional.
#[tokio::test]
async fn step_1_an_opted_out_ehr_leaves_the_query() {
    let world = Box::pin(World::new()).await;
    world
        .settings(
            0,
            &settings("open", &[directive("opt_out", "deny", &json!({}))], &[]),
        )
        .await;
    let expected = except(&world, &[0], &[]);
    assert_reads(
        &world,
        &Ask::of(CallerKind::AccessService),
        &expected,
        "an access service",
    )
    .await;
    assert_reads(
        &world,
        &Ask::professional(),
        &world.every(),
        "a professional",
    )
    .await;
}

/// Step 2: a restricted object contributes no row to a professional, and an
/// exception naming the caller brings it back.
#[tokio::test]
async fn step_2_a_restricted_object_leaves_the_query() {
    let world = Box::pin(World::new()).await;
    let hidden = world.objects(1)[0].clone();
    let restriction = |exceptions: Option<Value>| {
        let mut d = directive(
            "restriction",
            "deny",
            &json!({ "data": [{ "objects": [object_ref(&hidden)] }] }),
        );
        if let Some(exceptions) = exceptions {
            d["exceptions"] = exceptions;
        }
        d
    };
    world
        .settings(1, &settings("open", &[restriction(None)], &[]))
        .await;
    let expected = except(&world, &[], &[&hidden]);
    assert_reads(&world, &Ask::professional(), &expected, "a professional").await;
    let world2 = Box::pin(World::new()).await;
    let hidden2 = world2.objects(1)[0].clone();
    let excepted = directive(
        "restriction",
        "deny",
        &json!({
        "data": [{ "objects": [object_ref(&hidden2)] }],
        "exceptions": [directive("consent", "permit", &json!({ "applies_to": [{ "party": {
            "_type": "PARTY_REF", "namespace": "local", "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": NURSE } } }] }))] }),
    );
    world2
        .settings(1, &settings("open", &[excepted], &[]))
        .await;
    assert_reads(
        &world2,
        &Ask::professional(),
        &world2.every(),
        "the excepted caller",
    )
    .await;
}

/// Step 3: a consent deny naming the caller removes its EHR from the query.
#[tokio::test]
async fn step_3_a_consent_deny_leaves_the_query() {
    let world = Box::pin(World::new()).await;
    let deny = directive(
        "consent",
        "deny",
        &json!({ "applies_to": [{ "party": {
        "_type": "PARTY_REF", "namespace": "local", "type": "PERSON",
        "id": { "_type": "HIER_OBJECT_ID", "value": NURSE } } }] }),
    );
    world.settings(2, &settings("open", &[deny], &[])).await;
    assert_reads(
        &world,
        &Ask::professional(),
        &except(&world, &[2], &[]),
        "the denied caller",
    )
    .await;
}

/// Step 4: a labelled object the caller is not cleared for contributes no
/// row.
#[tokio::test]
async fn step_4_a_sensitive_object_leaves_the_query() {
    let world = Box::pin(World::new()).await;
    let labelled = world.objects(0)[1].clone();
    let label = json!({ "_type": "ACCESS_LABEL", "target": object_ref(&labelled),
                        "sensitivities": [coded("2.16.840.1.113883.5.25", "HIV")] });
    world.settings(0, &settings("open", &[], &[label])).await;
    assert_reads(
        &world,
        &Ask::professional(),
        &except(&world, &[], &[&labelled]),
        "uncleared",
    )
    .await;
    let mut cleared = Ask::professional();
    cleared.caller.facts = cleared_for_hiv();
    assert_reads(&world, &cleared, &world.every(), "cleared").await;
}

/// A caller's facts clearing `HIV`.
fn cleared_for_hiv() -> CallerFacts {
    CallerFacts::new(
        vec![ForeignCode {
            terminology: "2.16.840.1.113883.5.25".to_owned(),
            code: "HIV".to_owned(),
        }],
        Vec::new(),
        Vec::new(),
    )
}

/// An `ACCESS_POLICY` with `members`.
fn policy(members: &Value) -> AccessPolicy {
    let mut p = json!({
        "_type": "ACCESS_POLICY",
        "name": { "_type": "DV_TEXT", "value": "Policy" },
        "archetype_node_id": "openEHR-EHR-ACCESS_POLICY.controller.v1",
        "controller": { "_type": "PARTY_REF", "namespace": "local", "type": "ORGANISATION",
                        "id": { "_type": "HIER_OBJECT_ID", "value": NURSE } },
        "care_relationship_required": false,
        "purpose_required": false
    });
    if let (Some(p), Some(members)) = (p.as_object_mut(), members.as_object()) {
        p.extend(members.clone());
    }
    openehr_its::json::from_canonical_value(&p).expect("the test policy reads")
}

/// Step 5: where the policy requires a care relationship, a professional
/// reads only the EHRs of the subjects in their care, with or without
/// settings.
#[tokio::test]
async fn step_5_only_the_ehrs_in_care_are_queried() {
    let world = Box::pin(World::new()).await;
    world.settings(1, &settings("open", &[], &[])).await;
    let mut ask = Ask::professional();
    ask.policy = Some(policy(&json!({ "care_relationship_required": true })));
    assert_reads(&world, &ask, &[], "no relationship").await;
    ask.cared = vec![world.subject(0), world.subject(1)];
    assert_reads(
        &world,
        &ask,
        &except(&world, &[2], &[]),
        "two subjects in care",
    )
    .await;
}

/// Step 6: where the policy requires a purpose, a query without one reads
/// nothing, and one the caller may declare reads everything.
#[tokio::test]
async fn step_6_a_missing_purpose_reads_nothing() {
    let world = Box::pin(World::new()).await;
    let mut ask = Ask::professional();
    ask.policy = Some(policy(&json!({ "purpose_required": true })));
    assert_reads(&world, &ask, &[], "no purpose").await;
    ask.purpose = Some("TREAT");
    assert_reads(&world, &ask, &[], "a purpose not permitted").await;
    ask.caller.facts = CallerFacts::new(
        Vec::new(),
        vec![ForeignCode {
            terminology: "2.16.840.1.113883.5.8".to_owned(),
            code: "TREAT".to_owned(),
        }],
        Vec::new(),
    );
    assert_reads(&world, &ask, &world.every(), "a permitted purpose").await;
}

/// Step 7: an EHR that requires consent contributes only the rows a permit
/// covers, and the server's restricted default withholds every EHR with no
/// settings.
#[tokio::test]
async fn step_7_the_default_disposition_bounds_the_query() {
    let world = Box::pin(World::new()).await;
    let permitted = world.objects(0)[0].clone();
    let permit = directive(
        "consent",
        "permit",
        &json!({
        "applies_to": [{ "caller_kind": term("professional") }],
        "data": [{ "objects": [object_ref(&permitted)] }] }),
    );
    world
        .settings(0, &settings("consent_required", &[permit], &[]))
        .await;
    world.settings(1, &settings("open", &[], &[])).await;
    let other = world.objects(0)[1].clone();
    assert_reads(
        &world,
        &Ask::professional(),
        &except(&world, &[], &[&other]),
        "consent required",
    )
    .await;
    let mut restricted = Ask::professional();
    restricted.default = AccessDefault::Restricted;
    let mut expected: Vec<String> = world.objects(1);
    expected.push(permitted);
    expected.sort();
    assert_reads(
        &world,
        &restricted,
        &expected,
        "a restricted server default",
    )
    .await;
}

/// An EHR a query reaches only through its condition is recorded with the
/// grounds a direct read of it records: the permit, with the uid of the
/// `EHR_ACCESS` version that carries it, read from the copied index (#5581).
#[tokio::test]
async fn a_query_served_ehr_names_the_grounds_of_a_direct_read() {
    let world = Box::pin(World::new()).await;
    let permit = directive(
        "consent",
        "permit",
        &json!({ "applies_to": [{ "caller_kind": term("professional") }],
                 "legal_grounds": [coded("urn:example:law", "gdpr-9-2-h")] }),
    );
    world
        .settings(0, &settings("consent_required", &[permit], &[]))
        .await;
    let (ask, ehr_id) = (Ask::professional(), world.ehrs[0].0);

    let queried = request_of(&ask);
    let outcome = run_query(&world, &ask, &queried, COMPOSITIONS, None).await;
    assert!(
        outcome
            .served_ehrs
            .iter()
            .any(|(served, _)| *served == ehr_id.0.to_string()),
        "the query serves the EHR"
    );
    assert!(
        queried.judgement(ehr_id).is_none(),
        "the query reaches the EHR only through its condition"
    );

    let direct = request_of(&ask);
    Box::pin(with_request_access(
        Arc::clone(&direct),
        world.svc.judge_request_ehrs(&[ehr_id]),
    ))
    .await
    .expect("the EHR is judged");

    let (queried, direct) = (
        queried.decision(Some(ehr_id)),
        direct.decision(Some(ehr_id)),
    );
    assert!(queried.refused.is_none(), "{queried:?}");
    assert_eq!(queried.grounds, direct.grounds);
    assert_eq!(queried.policies, direct.policies);
    let consent = queried
        .grounds
        .iter()
        .find(|ground| ground.kind.value == "consent")
        .expect("the permit is a ground");
    let version = consent.version.as_deref().expect("the EHR_ACCESS version");
    assert!(
        version.ends_with(&format!("::{DEFAULT_SYSTEM_ID}::2")),
        "{version}"
    );
    assert_eq!(queried.policies, [version]);
}
