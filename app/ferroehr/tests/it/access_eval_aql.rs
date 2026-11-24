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

use ferroehr::access_eval::care_tie::Tie;
use ferroehr::access_eval::condition::care::{Care, CaredEhr, CaredSubject};
use ferroehr::access_eval::condition::{
    AccessCondition, Asking, InForce, RequestAccess, with_request_access,
};
use ferroehr::access_eval::procedure::{Caller, Emergency, Outcome};
use ferroehr::access_eval::scope::{CategoryIndex, Pred};
use ferroehr::ids::{EhrId, VoId};
use ferroehr::party_binding::BindingMapping;
use ferroehr::party_binding::grants::caller::facts::CallerFacts;
use ferroehr::party_binding::policy_mark::TERMINOLOGY;
use ferroehr::service::ehr::access_rule::SettingsCaller;
use ferroehr::service::query::request::{AqlQueryRequest, QueryOutcome};
use ferroehr::service::{DEFAULT_SYSTEM_ID, FerroEhrService};
use ferroehr::storage::access_projection::EhrSubject;
use ferroehr::versioning::committer::CommitterIdentity;
use openehr_rm::draft::access_control::access_control::access_policy::AccessPolicy;
use openehr_rm::draft::access_control::terminology::care::CarePath;
use openehr_rm::draft::access_control::terminology::{
    AccessDefault, CallerKind, ForeignCode, OperationClass, Refusal,
};
use serde_json::{Value, json};

use crate::fixtures::{change_type, composition, create_ehr_for_party, uv};

/// The controller the test access policy names.
const CONTROLLER: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f83";
/// The query every test pages through.
const COMPOSITIONS: &str =
    "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c ORDER BY c/uid/value";
/// The count of the same rows.
const COUNT: &str = "SELECT COUNT(c/uid/value) FROM EHR e CONTAINS COMPOSITION c";
/// The same rows with no `EHR` source, whose root decides its own EHR
/// (#5782).
const UNLINKED: &str = "SELECT c/uid/value FROM COMPOSITION c ORDER BY c/uid/value";
/// The count of the same rows with no `EHR` source.
const UNLINKED_COUNT: &str = "SELECT COUNT(c/uid/value) FROM COMPOSITION c";

/// Three EHRs, each with two compositions, the subject of each a `PERSON`,
/// and the nurse, the `PERSON` the professional the tests ask as is bound to,
/// which a directive names only as a party of the store.
struct World {
    db: testkit::TestDb,
    svc: FerroEhrService,
    ehrs: Vec<(EhrId, uuid::Uuid, Vec<String>)>,
    nurse: String,
}

/// A term of the access control groups.
fn term(code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": TERMINOLOGY },
                "code_string": code } })
}

/// An `access effect`: a code of HL7 FHIR `HL7_ConsentProvisionType`.
fn provision(code: &str) -> Value {
    let mut value = term(code);
    value["defining_code"]["terminology_id"]["value"] = serde_json::json!(
        openehr_rm::draft::access_control::terminology::TERMINOLOGY_ID_CONSENT_PROVISION_TYPE
    );
    value
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
        "effect": provision(effect),
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
        local_issuer: DEFAULT_SYSTEM_ID.into(),
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
        let nurse = crate::fixtures::create_party(
            &svc,
            uv(
                &crate::admin_fixture::person(
                    "Nurse",
                    &json!([{ "_type": "ELEMENT", "archetype_node_id": "at0003",
                        "name": { "_type": "DV_TEXT", "value": "family" },
                        "value": { "_type": "DV_TEXT", "value": "Nurse" } }]),
                ),
                "249",
                None,
            ),
        )
        .await
        .expect("the nurse is created")
        .0
        .to_string();
        Self {
            db,
            svc,
            ehrs,
            nurse,
        }
    }

    /// Commits `settings` as the current `EHR_ACCESS` of EHR `index`.
    async fn settings(&self, index: usize, settings: &Value) {
        let ehr_id = self.ehrs[index].0;
        let ehr = openehr_its::json::to_canonical_value(
            &self.svc.ehr_read(ehr_id).await.expect("the EHR").body,
        );
        let access_vo = ehr["ehr_access"]["id"]["value"]
            .as_str()
            .expect("EHR.ehr_access")
            .to_owned();
        let audit = json!({ "change_type": change_type("251", "modification"),
            "committer": { "_type": "PARTY_IDENTIFIED",
                "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": DEFAULT_SYSTEM_ID, "id": "root" }] } });
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

    /// The subject reference of EHR `index`: its `PERSON` under the CDR's own
    /// namespace.
    fn subject(&self, index: usize) -> EhrSubject {
        EhrSubject::new(
            ferroehr::config::binding::PartyBindingConfig::default().namespace,
            self.ehrs[index].1.to_string(),
        )
    }
}

/// What one query asks as.
struct Ask {
    caller: Caller,
    purpose: Option<&'static str>,
    policy: Option<AccessPolicy>,
    cared: Vec<EhrSubject>,
    default: AccessDefault,
    emergency: Emergency,
}

impl Ask {
    /// A professional bound to the nurse of `world`, asking with no purpose.
    fn professional(world: &World) -> Self {
        Self::of(world, CallerKind::Professional)
    }

    /// The subject of `own`, through a patient context naming that EHR, or
    /// naming none.
    fn subject(world: &World, own: Option<EhrId>) -> Self {
        let mut ask = Self::of(world, CallerKind::Subject);
        ask.caller.party = None;
        ask.caller.own_ehrs = own.into_iter().collect();
        ask.caller.settings = SettingsCaller::new(false, true, None);
        ask
    }

    /// A caller of `kind` bound to the nurse of `world`.
    fn of(world: &World, kind: CallerKind) -> Self {
        Self {
            caller: Caller {
                kind,
                party: Some(world.nurse.clone()),
                own_ehrs: Vec::new(),
                patient_grants_only: false,
                acts_for: Vec::new(),
                facts: CallerFacts::default(),
                administrator: false,
                settings: SettingsCaller::new(false, false, Some(world.nurse.clone())),
                lacks_consent_reference: false,
            },
            purpose: None,
            policy: None,
            cared: Vec::new(),
            default: AccessDefault::Open,
            emergency: Emergency::default(),
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
            emergency: ask.emergency,
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

/// How a statement reads the caller's care in [`run_query`].
#[derive(Clone, Copy, Debug)]
enum CareForm {
    /// The subjects of every patient in care, bound into the statement.
    Census,
    /// The EHRs whose subjects are in care, as a pinned query binds them
    /// (#5699); for a query over every EHR the same answer when every cared
    /// EHR is named.
    Ehrs,
    /// The clinical care projection, its rows written for the subjects in
    /// care under one team, probed by the EHR rows' keyed digests (#5699).
    Projected,
}

/// Every care form, each of which must read the same rows.
const CARE_FORMS: [CareForm; 3] = [CareForm::Census, CareForm::Ehrs, CareForm::Projected];

/// The care team the projected care is written under.
const TEAM: uuid::Uuid = uuid::Uuid::from_u128(0x0199_0000_0000_7000_8000_0000_0000_0001);

/// The provider the projected care relationships are in the care of.
const PROVIDER: (&str, &str) = ("urn:example:providers", "hospital");

/// The namespace this CDR names its parties under, which the EHR rows'
/// subjects are digested in.
fn own() -> String {
    ferroehr::config::binding::PartyBindingConfig::default().namespace
}

/// Write the care projection of `cared`, each relationship from `from` until
/// `until`, under [`TEAM`], and every EHR row's digest: what the care
/// projector writes from the party domain.
#[expect(
    clippy::disallowed_methods,
    reason = "a test reads the database on a raw pool connection, outside any request"
)]
async fn project(
    world: &World,
    cared: &[(
        EhrSubject,
        Option<jiff::civil::Date>,
        Option<jiff::civil::Date>,
    )],
) {
    use ferroehr::storage::access_care::{self, CareRow, cared_key, subject_digest, subject_key};
    let pool = world.db.pool();
    let mut conn = pool.acquire().await.expect("a connection");
    let mut tx = sqlx::Connection::begin(&mut *conn)
        .await
        .expect("a transaction");
    let persons: Vec<uuid::Uuid> = world.ehrs.iter().map(|(_, person, _)| *person).collect();
    let rows: Vec<CareRow> = cared
        .iter()
        .map(|(subject, from, until)| CareRow {
            digest: subject_digest(&cared_key(subject)),
            person: subject.id.parse().unwrap_or(uuid::Uuid::nil()),
            carer: TEAM,
            carer_kind: ferroehr::party_binding::grants::caller::care_of::CarerKind::Group,
            provider_issuer: PROVIDER.0.to_owned(),
            provider_id: PROVIDER.1.to_owned(),
            from: *from,
            until: *until,
            writer: None,
            self_declared: false,
            role: ForeignCode {
                terminology: "HL7_ParticipationType".to_owned(),
                code: "PART".to_owned(),
            },
        })
        .collect();
    access_care::clear(&mut tx)
        .await
        .expect("the projection is cleared");
    access_care::replace(&mut tx, &persons, &rows)
        .await
        .expect("the rows are written");
    let undigested = access_care::undigested(&mut tx, 100)
        .await
        .expect("the EHRs");
    let digests: Vec<_> = undigested
        .into_iter()
        .map(|(ehr, subject)| {
            let digest = subject_digest(&subject_key(&own(), &subject));
            (ehr, subject, digest)
        })
        .collect();
    access_care::set_digests(&mut tx, &digests)
        .await
        .expect("the digests are written");
    tx.commit().await.expect("the projection commits");
}

/// The projected care probe of the caller on `day` with `grace`.
fn projected(day: jiff::civil::Date, grace: &str) -> Care {
    Care::Projected(ferroehr::access_eval::condition::care::ProjectedCare {
        principals: vec![TEAM].into(),
        direct: Vec::new().into(),
        provider_issuers: vec![PROVIDER.0.to_owned()].into(),
        provider_ids: vec![PROVIDER.1.to_owned()].into(),
        day,
        grace: grace.to_owned(),
        self_declared: false,
    })
}

/// The tie by which a caller of [`TEAM`] holds a subject the projection
/// writes under it: the team path, the role `PART`.
fn team_tie() -> Tie {
    Tie {
        path: CarePath::Team,
        role: ForeignCode {
            terminology: "HL7_ParticipationType".to_owned(),
            code: "PART".to_owned(),
        },
        grant: None,
    }
}

/// The care `ask` holds, in `form`.
fn care_of(world: &World, ask: &Ask, form: CareForm) -> Care {
    match form {
        CareForm::Projected => projected(jiff::Zoned::now().date(), "P0D"),
        CareForm::Census => Care::Census(
            ask.cared
                .iter()
                .map(|subject| CaredSubject {
                    subject: subject.clone(),
                    ties: vec![team_tie()].into(),
                })
                .collect(),
        ),
        CareForm::Ehrs => Care::Ehrs(
            world
                .ehrs
                .iter()
                .enumerate()
                .filter(|(index, _)| ask.cared.contains(&world.subject(*index)))
                .map(|(_, (ehr_id, _, _))| CaredEhr {
                    ehr: ehr_id.0,
                    ties: vec![team_tie()].into(),
                })
                .collect(),
        ),
    }
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
    run_query_as(world, ask, access, aql, page, CareForm::Census).await
}

/// [`run_query`] with the caller's care read in `form`.
async fn run_query_as(
    world: &World,
    ask: &Ask,
    access: &Arc<RequestAccess>,
    aql: &str,
    page: Option<(i64, i64)>,
    form: CareForm,
) -> QueryOutcome {
    let svc = &world.svc;
    let run = async {
        let classes = svc.ehr_access_classes().await.expect("the classes read");
        let request = AqlQueryRequest {
            ehr_access: ferroehr::service::query::request::QueryAccess::Given(
                access.condition(&classes, care_of(world, ask, form)),
            ),
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

/// The rows of `aql` `ask` reads, from `offset`, at most `fetch`, with the
/// same rows under every care form.
async fn rows(world: &World, ask: &Ask, aql: &str, page: Option<(i64, i64)>) -> Vec<Value> {
    let mut read: Option<Vec<Value>> = None;
    for form in CARE_FORMS {
        if matches!(form, CareForm::Projected) {
            let cared: Vec<_> = ask.cared.iter().map(|s| (s.clone(), None, None)).collect();
            project(world, &cared).await;
        }
        let outcome = run_query_as(world, ask, &request_of(ask), aql, page, form).await;
        let rows = outcome.result_set.to_value().unwrap()["rows"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        match &read {
            Some(first) => assert_eq!(&rows, first, "{form:?} reads what the census reads"),
            None => read = Some(rows),
        }
    }
    read.unwrap_or_default()
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
    let unlinked = ids(rows(world, ask, UNLINKED, None).await);
    assert_eq!(unlinked, expected, "{case}: the rows with no EHR source");
    let paged = ids(rows(world, ask, UNLINKED, Some((1, 2))).await);
    let window: Vec<String> = expected.iter().skip(1).take(2).cloned().collect();
    assert_eq!(paged, window, "{case}: a page with no EHR source");
    let counted = rows(world, ask, UNLINKED_COUNT, None).await;
    assert_eq!(
        counted[0][0],
        json!(expected.len()),
        "{case}: the count with no EHR source"
    );
    for (index, (ehr_id, _, objects)) in world.ehrs.iter().enumerate() {
        let of_ehr: Vec<String> = expected
            .iter()
            .filter(|id| objects.contains(id))
            .cloned()
            .collect();
        let pinned = pinned_to(COMPOSITIONS, *ehr_id);
        let whole = ids(rows(world, ask, &pinned, None).await);
        assert_eq!(whole, of_ehr, "{case}: the rows of EHR {index} pinned");
        let mut paged = Vec::new();
        for offset in 0..=i64::try_from(of_ehr.len()).expect("a count") {
            paged.extend(ids(rows(world, ask, &pinned, Some((offset, 1))).await));
        }
        assert_eq!(
            paged, of_ehr,
            "{case}: one row per page of EHR {index} pinned"
        );
        let counted = rows(world, ask, &pinned_to(COUNT, *ehr_id), None).await;
        assert_eq!(
            counted[0][0],
            json!(of_ehr.len()),
            "{case}: the count of EHR {index} pinned"
        );
    }
}

/// `aql`, a query over `EHR e`, pinned to `ehr_id` by its own `WHERE`
/// clause, which the service decides before the statement (#5746).
fn pinned_to(aql: &str, ehr_id: EhrId) -> String {
    let filter = format!(" WHERE e/ehr_id/value = '{ehr_id}'");
    match aql.find(" ORDER BY") {
        Some(at) => format!("{}{filter}{}", &aql[..at], &aql[at..]),
        None => format!("{aql}{filter}"),
    }
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
        &Ask::of(&world, CallerKind::AccessService),
        &expected,
        "an access service",
    )
    .await;
    assert_reads(
        &world,
        &Ask::professional(&world),
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
    assert_reads(
        &world,
        &Ask::professional(&world),
        &expected,
        "a professional",
    )
    .await;
    let world2 = Box::pin(World::new()).await;
    let hidden2 = world2.objects(1)[0].clone();
    let excepted = directive(
        "restriction",
        "deny",
        &json!({
        "data": [{ "objects": [object_ref(&hidden2)] }],
        "exceptions": [directive("consent", "permit", &json!({ "applies_to": [{ "party": {
            "_type": "PARTY_REF", "namespace": "local", "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": world2.nurse } } }] }))] }),
    );
    world2
        .settings(1, &settings("open", &[excepted], &[]))
        .await;
    assert_reads(
        &world2,
        &Ask::professional(&world2),
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
        "id": { "_type": "HIER_OBJECT_ID", "value": world.nurse } } }] }),
    );
    world.settings(2, &settings("open", &[deny], &[])).await;
    assert_reads(
        &world,
        &Ask::professional(&world),
        &except(&world, &[2], &[]),
        "the denied caller",
    )
    .await;
}

/// Step 6: a labelled object no permit names contributes no row; a permit
/// rule naming the caller's role clears it.
#[tokio::test]
async fn step_6_a_sensitive_object_leaves_the_query() {
    let world = Box::pin(World::new()).await;
    let labelled = world.objects(0)[1].clone();
    let label = json!({ "_type": "ACCESS_LABEL", "target": object_ref(&labelled),
                        "sensitivities": [coded("HL7_ActCode", "HIV")] });
    world.settings(0, &settings("open", &[], &[label])).await;
    assert_reads(
        &world,
        &Ask::professional(&world),
        &except(&world, &[], &[&labelled]),
        "uncleared",
    )
    .await;
    let mut cleared = Ask::professional(&world);
    cleared.policy = Some(policy(&json!({ "rules": [{
        "_type": "ACCESS_DIRECTIVE", "name": { "_type": "DV_TEXT", "value": "HIV specialists" },
        "archetype_node_id": "at0001", "kind": term("controller_rule"), "effect": provision("permit"),
        "applies_to": [{ "role": coded(ROLES, "HIV_SPECIALIST") }],
        "data": [{ "sensitivities": [coded("HL7_ActCode", "HIV")] }],
        "emergency_override": false }] })));
    assert_reads(
        &world,
        &cleared,
        &except(&world, &[], &[&labelled]),
        "another role",
    )
    .await;
    cleared.caller.facts = performing("HIV_SPECIALIST");
    assert_reads(&world, &cleared, &world.every(), "cleared by a rule").await;
}

/// The roles of the role code system [`ROLES`]: a caller performing `role`.
fn performing(role: &str) -> CallerFacts {
    CallerFacts::new(
        vec![ForeignCode {
            terminology: ROLES.to_owned(),
            code: role.to_owned(),
        }],
        Vec::new(),
    )
}

/// The jurisdiction's role code system.
const ROLES: &str = "urn:example:roles";

/// An `ACCESS_POLICY` with `members`.
fn policy(members: &Value) -> AccessPolicy {
    let mut p = json!({
        "_type": "ACCESS_POLICY",
        "name": { "_type": "DV_TEXT", "value": "Policy" },
        "archetype_node_id": "openEHR-EHR-ACCESS_POLICY.controller.v1",
        "controller": { "_type": "PARTY_REF", "namespace": "local", "type": "ORGANISATION",
                        "id": { "_type": "HIER_OBJECT_ID", "value": CONTROLLER } },
        "care_relationship_required": false,
        "purpose_required": false
    });
    if let (Some(p), Some(members)) = (p.as_object_mut(), members.as_object()) {
        p.extend(members.clone());
    }
    openehr_its::json::from_canonical_value(&p).expect("the test policy reads")
}

/// Step 7: where the policy requires a care relationship, a professional
/// reads only the EHRs of the subjects in their care, with or without
/// settings.
#[tokio::test]
async fn step_7_only_the_ehrs_in_care_are_queried() {
    let world = Box::pin(World::new()).await;
    world.settings(1, &settings("open", &[], &[])).await;
    let mut ask = Ask::professional(&world);
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

/// The care projection reads a relationship's dates as the evaluator reads
/// them (`CareOf::live_on`): begun, and open or ended less than the grace
/// before the day of the request (#5699).
#[tokio::test]
async fn the_projection_reads_the_dates_of_care_as_the_evaluator() {
    use ferroehr::party_binding::grants::caller::care_of::{CareOf, ProviderKey};
    use openehr_rm::draft::access_control::terminology::ForeignCode;
    let world = Box::pin(World::new()).await;
    world.settings(1, &settings("open", &[], &[])).await;
    let mut ask = Ask::professional(&world);
    ask.policy = Some(policy(&json!({ "care_relationship_required": true })));
    let access = request_of(&ask);
    let today = access.today().expect("the day in UTC");
    let day = |days: i64| {
        today
            .checked_add(jiff::Span::new().days(days))
            .expect("a date")
    };
    let cases: [(Option<jiff::civil::Date>, Option<jiff::civil::Date>, &str); 7] = [
        (None, None, "P14D"),
        (Some(day(1)), None, "P14D"),
        (Some(today), Some(day(1)), "P0D"),
        (None, Some(today), "P0D"),
        (None, Some(day(-13)), "P14D"),
        (None, Some(day(-14)), "P14D"),
        (None, Some(day(-1)), "P1M"),
    ];
    for (from, until, grace) in cases {
        let span: jiff::Span = grace.parse().expect("a grace");
        let role = ForeignCode {
            terminology: "HL7_ParticipationType".to_owned(),
            code: "PART".to_owned(),
        };
        let provider = ProviderKey::new(PROVIDER.0, PROVIDER.1);
        let live = CareOf::spanning(TEAM, (from, until), role, provider).live_on(today, span);
        project(&world, &[(world.subject(0), from, until)]).await;
        let run = async {
            let classes = world
                .svc
                .ehr_access_classes()
                .await
                .expect("the classes read");
            let request = AqlQueryRequest {
                ehr_access: ferroehr::service::query::request::QueryAccess::Given(
                    access.condition(&classes, projected(today, grace)),
                ),
                ..AqlQueryRequest::default()
            };
            Box::pin(world.svc.execute_aql(COMPOSITIONS, None, &request))
                .await
                .expect("the query runs")
        };
        let outcome = Box::pin(with_request_access(Arc::clone(&access), run)).await;
        let read: Vec<String> = outcome.result_set.to_value().unwrap()["rows"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|row| row[0].as_str())
            .map(|uid| uid.split("::").next().unwrap_or_default().to_owned())
            .collect();
        let expected = if live { world.objects(0) } else { Vec::new() };
        assert_eq!(
            read, expected,
            "from {from:?} until {until:?} with grace {grace}"
        );
    }
}

/// A care census far larger than the store reads exactly the EHRs whose
/// subjects it names, with or without settings, whatever else it holds: the
/// subjects bound once as a `jsonb` object match as the bound array matched
/// (#5645).
#[tokio::test]
async fn a_large_care_census_reads_exactly_the_ehrs_in_care() {
    let world = Box::pin(World::new()).await;
    world.settings(1, &settings("open", &[], &[])).await;
    let mut ask = Ask::professional(&world);
    ask.policy = Some(policy(&json!({ "care_relationship_required": true })));
    let others: Vec<EhrSubject> = (0..5000)
        .map(|_| {
            EhrSubject::new(
                ferroehr::config::binding::PartyBindingConfig::default().namespace,
                uuid::Uuid::now_v7().to_string(),
            )
        })
        .collect();
    ask.cared.clone_from(&others);
    assert_reads(&world, &ask, &[], "a census naming no subject of the store").await;
    let upper = world.subject(1);
    ask.cared
        .push(EhrSubject::new(upper.namespace, upper.id.to_uppercase()));
    assert_reads(&world, &ask, &[], "a subject named in another case").await;
    ask.cared
        .extend([world.subject(2), world.subject(0), world.subject(2)]);
    assert_reads(
        &world,
        &ask,
        &except(&world, &[1], &[]),
        "two subjects among five thousand, one named twice",
    )
    .await;
    ask.cared.push(world.subject(1));
    assert_reads(&world, &ask, &world.every(), "every subject in care").await;
}

/// Step 4: where the policy requires a purpose, a query without one reads
/// nothing, whatever rules exist; step 5: a rule limiting the purposes a
/// professional declares reads nothing for another purpose.
#[tokio::test]
async fn step_4_a_missing_purpose_reads_nothing() {
    let world = Box::pin(World::new()).await;
    let mut ask = Ask::professional(&world);
    ask.policy = Some(policy(&json!({ "purpose_required": true })));
    assert_reads(&world, &ask, &[], "no purpose").await;
    ask.purpose = Some("TREAT");
    assert_reads(&world, &ask, &world.every(), "a purpose").await;
    ask.policy = Some(policy(&json!({ "purpose_required": true, "rules": [{
        "_type": "ACCESS_DIRECTIVE", "name": { "_type": "DV_TEXT", "value": "Treatment only" },
        "archetype_node_id": "at0001", "kind": term("controller_rule"), "effect": provision("deny"),
        "applies_to": [{ "caller_kind": term("professional") }],
        "exceptions": [{
            "_type": "ACCESS_DIRECTIVE", "name": { "_type": "DV_TEXT", "value": "Treatment" },
            "archetype_node_id": "at0002", "kind": term("controller_rule"),
            "effect": provision("permit"),
            "purposes": [coded("HL7_ActReason", "TREAT")],
            "emergency_override": false }],
        "emergency_override": false }] })));
    assert_reads(&world, &ask, &world.every(), "a permitted purpose").await;
    ask.purpose = Some("HRESCH");
    assert_reads(&world, &ask, &[], "a purpose the rule denies").await;
    ask.purpose = None;
    assert_reads(&world, &ask, &[], "no purpose under the rule").await;
}

/// Step 8: an EHR that requires consent contributes only the rows a permit
/// covers, and the server's restricted default withholds every EHR with no
/// settings.
#[tokio::test]
async fn step_8_the_default_disposition_bounds_the_query() {
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
        &Ask::professional(&world),
        &except(&world, &[], &[&other]),
        "consent required",
    )
    .await;
    let mut restricted = Ask::professional(&world);
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

/// An EHR a query serves is recorded with the grounds a direct read of it
/// records: the permit, with the uid of the `EHR_ACCESS` version that carries
/// it, judged from the per-EHR copy once the query ran (#5581, #5588).
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
    let (ask, ehr_id) = (Ask::professional(&world), world.ehrs[0].0);

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
        queried.judgement(ehr_id).is_some(),
        "the query judges the EHR it served from the per-EHR copy"
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

/// A `PARTY_REF` to the `PERSON` `id`.
fn person(id: &str) -> Value {
    json!({ "_type": "PARTY_REF", "namespace": "local", "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": id } })
}

/// An opt-out in force from `from`.
fn opt_out_from(from: &str) -> Value {
    directive(
        "opt_out",
        "deny",
        &json!({ "time_validity": { "_type": "DV_INTERVAL",
            "lower": { "_type": "DV_DATE_TIME", "value": from },
            "lower_unbounded": false, "upper_unbounded": true,
            "lower_included": true, "upper_included": false } }),
    )
}

/// The number of settings classes the store holds.
async fn classes(world: &World) -> usize {
    world
        .svc
        .ehr_access_classes()
        .await
        .expect("the classes read")
        .classes
        .len()
}

/// Two opt-outs that differ only in when they start share one class, and
/// the query reads each EHR's own start: the one in force withholds its EHR
/// from an access service, the one that starts next year withholds nothing
/// (#5588).
#[tokio::test]
async fn a_shared_class_reads_each_ehrs_validity() {
    let world = Box::pin(World::new()).await;
    world
        .settings(
            0,
            &settings("open", &[opt_out_from("2020-01-01T00:00:00Z")], &[]),
        )
        .await;
    world
        .settings(
            1,
            &settings("open", &[opt_out_from("2999-01-01T00:00:00Z")], &[]),
        )
        .await;
    assert_eq!(classes(&world).await, 1, "one class for both opt-outs");
    assert_reads(
        &world,
        &Ask::of(&world, CallerKind::AccessService),
        &except(&world, &[0], &[]),
        "an access service",
    )
    .await;
}

/// Two consent denies that differ only in the party they name share one
/// class, and the query reads each EHR's own party: only the EHR whose deny
/// names the caller leaves its query (#5588).
#[tokio::test]
async fn a_shared_class_reads_each_ehrs_party() {
    let world = Box::pin(World::new()).await;
    let other = crate::fixtures::create_party(
        &world.svc,
        uv(
            &crate::admin_fixture::person(
                "Doctor",
                &json!([{ "_type": "ELEMENT", "archetype_node_id": "at0003",
                    "name": { "_type": "DV_TEXT", "value": "family" },
                    "value": { "_type": "DV_TEXT", "value": "Doctor" } }]),
            ),
            "249",
            None,
        ),
    )
    .await
    .expect("the doctor is created")
    .0
    .to_string();
    let nurse = world.nurse.clone();
    let deny = |party: &str| {
        directive(
            "consent",
            "deny",
            &json!({ "applies_to": [{ "party": person(party) }] }),
        )
    };
    world
        .settings(0, &settings("open", &[deny(&nurse)], &[]))
        .await;
    world
        .settings(1, &settings("open", &[deny(&other)], &[]))
        .await;
    assert_eq!(classes(&world).await, 1, "one class for both denies");
    assert_reads(
        &world,
        &Ask::professional(&world),
        &except(&world, &[0], &[]),
        "the nurse",
    )
    .await;
}

/// A label on one EHR's object and on another's share one class, and the
/// query reads each EHR's own labelled object (#5588).
#[tokio::test]
async fn a_shared_class_reads_each_ehrs_labels() {
    let world = Box::pin(World::new()).await;
    let label = |object: &str| {
        json!({ "_type": "ACCESS_LABEL", "target": object_ref(object),
                "sensitivities": [coded("HL7_ActCode", "HIV")] })
    };
    let (first, second) = (world.objects(0)[0].clone(), world.objects(2)[1].clone());
    world
        .settings(0, &settings("open", &[], &[label(&first)]))
        .await;
    world
        .settings(2, &settings("open", &[], &[label(&second)]))
        .await;
    assert_eq!(classes(&world).await, 1, "one class for both labels");
    assert_reads(
        &world,
        &Ask::professional(&world),
        &except(&world, &[], &[&first, &second]),
        "uncleared",
    )
    .await;
}

/// A query judged on classes read before a commit made a new one never
/// answers an EHR of the new class under what it held before, nor leaves it
/// out: the statement fails, the service reads the classes again and
/// answers every EHR as the classes in force decide it (#5588, #5794).
#[tokio::test]
async fn an_ehr_of_a_class_the_condition_does_not_hold_is_judged_again() {
    let world = Box::pin(World::new()).await;
    let ask = Ask::professional(&world);
    let access = request_of(&ask);
    let stale = Box::pin(with_request_access(Arc::clone(&access), async {
        access.condition(
            &world
                .svc
                .ehr_access_classes()
                .await
                .expect("the classes read"),
            Care::Census(Vec::new().into()),
        )
    }))
    .await;
    world.settings(1, &settings("open", &[], &[])).await;
    let request = AqlQueryRequest {
        ehr_access: ferroehr::service::query::request::QueryAccess::Given(stale),
        ..AqlQueryRequest::default()
    };
    let outcome = Box::pin(with_request_access(
        Arc::clone(&access),
        world.svc.execute_aql(COMPOSITIONS, None, &request),
    ))
    .await
    .expect("the query runs");
    let served: Vec<String> = outcome.result_set.to_value().unwrap()["rows"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|row| row[0].as_str())
        .map(|uid| uid.split("::").next().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(served, world.every(), "the new class is read again");
    assert_reads(&world, &ask, &world.every(), "the classes read again").await;
}

/// The first column of every row of `aql` `ask` reads.
async fn first_column(world: &World, ask: &Ask, aql: &str) -> Vec<Value> {
    rows(world, ask, aql, None)
        .await
        .iter()
        .map(|row| row[0].clone())
        .collect()
}

/// A subject caller's query reads only the rows of their own EHR, for every
/// query shape, including one with no versioned-object root, which reads the
/// `ehr` row alone; a professional reads every EHR (#5586).
#[tokio::test]
async fn a_patient_query_reads_only_their_own_ehr() {
    let world = Box::pin(World::new()).await;
    world
        .settings(
            1,
            &settings("open", &[directive("consent", "permit", &json!({}))], &[]),
        )
        .await;
    let (own, other) = (world.ehrs[0].0, world.ehrs[1].0);
    let patient = Ask::subject(&world, Some(own));
    let open = Ask::professional(&world);
    for aql in [
        "SELECT e/ehr_id/value FROM EHR e",
        "SELECT e/ehr_id/value, e/ehr_status/subject FROM EHR e",
        "SELECT e/ehr_id/value FROM EHR e CONTAINS EHR_STATUS s",
        "SELECT e/ehr_id/value FROM EHR e CONTAINS COMPOSITION c",
        "SELECT e/ehr_id/value, c/name/value FROM EHR e CONTAINS COMPOSITION c LIMIT 10",
        "SELECT e/ehr_id/value FROM EHR e CONTAINS (COMPOSITION c1 OR EHR_STATUS s1)",
    ] {
        let control = first_column(&world, &open, aql).await;
        assert!(
            control.contains(&json!(other.0.to_string())),
            "{aql}: with no patient context the other EHR is served: {control:?}"
        );
        let served = first_column(&world, &patient, aql).await;
        assert!(!served.is_empty(), "{aql}: the caller's EHR is served");
        assert!(
            served.iter().all(|id| *id == json!(own.0.to_string())),
            "{aql}: only the caller's EHR is served: {served:?}"
        );
    }
    let count = first_column(&world, &patient, "SELECT COUNT(*) FROM EHR e").await;
    assert_eq!(count, vec![json!(1)], "one EHR is in the caller's scope");
    assert_reads(&world, &patient, &world.objects(0), "the subject of EHR 0").await;
    let settled = Ask::subject(&world, Some(other));
    assert_reads(&world, &settled, &world.objects(1), "the subject of EHR 1").await;
}

/// A subject caller whose patient context names no EHR, or an EHR that does
/// not exist, reads nothing; a caller in the context of an EHR reads it
/// (#5586).
#[tokio::test]
async fn a_patient_query_outside_any_own_ehr_reads_nothing() {
    let world = Box::pin(World::new()).await;
    let aql = "SELECT e/ehr_id/value FROM EHR e";
    for (case, own) in [
        ("no EHR", None),
        (
            "an EHR that does not exist",
            Some(EhrId(uuid::Uuid::now_v7())),
        ),
    ] {
        let ask = Ask::subject(&world, own);
        assert_eq!(
            first_column(&world, &ask, aql).await,
            Vec::<Value>::new(),
            "{case}: the patient context names no stored EHR"
        );
        assert_reads(&world, &ask, &[], case).await;
    }
    let unique = world.ehrs[2].0;
    let served = first_column(&world, &Ask::subject(&world, Some(unique)), aql).await;
    assert_eq!(
        served,
        vec![json!(unique.0.to_string())],
        "a patient context reads its EHR"
    );
}

/// Two EHRs whose subjects share an id under two namespaces are two
/// patients: the subject of one reads exactly that EHR, whichever namespace
/// it is under (#5586).
#[tokio::test]
async fn a_shared_subject_id_names_no_other_ehr() {
    let world = Box::pin(World::new()).await;
    let shared = |namespace: &str| {
        ferroehr::service::ehr_index::types::SubjectRef::person("PATIENT-X", namespace)
    };
    let ehr_a = crate::fixtures::create_ehr_subject(&world.svc, &shared("test.patients")).await;
    let ehr_b = crate::fixtures::create_ehr_subject(&world.svc, &shared("other.patients")).await;
    let aql = "SELECT e/ehr_id/value FROM EHR e";
    for ehr in [ehr_a, ehr_b] {
        let served = first_column(&world, &Ask::subject(&world, Some(ehr)), aql).await;
        assert_eq!(served, vec![json!(ehr.0.to_string())]);
    }
}

/// A subject caller is refused every EHR an operation names that is not
/// their own, in its path, its body or its result, and admitted to their
/// own: the judgement the service reads where data leave the store (#5586).
#[tokio::test]
async fn a_patient_is_refused_every_ehr_but_their_own() {
    let world = Box::pin(World::new()).await;
    world
        .settings(
            2,
            &settings("open", &[directive("consent", "permit", &json!({}))], &[]),
        )
        .await;
    let own = world.ehrs[0].0;
    let ask = Ask::subject(&world, Some(own));
    let access = request_of(&ask);
    let ehrs: Vec<EhrId> = world.ehrs.iter().map(|(ehr, _, _)| *ehr).collect();
    Box::pin(with_request_access(
        Arc::clone(&access),
        world.svc.judge_request_ehrs(&ehrs),
    ))
    .await
    .expect("the EHRs are judged");
    let item = ferroehr::access_eval::scope::Item::Ehr;
    let refused = Outcome::Refused(Refusal::NotSubject);
    for ehr in &ehrs {
        let expected = if *ehr == own { Outcome::Admit } else { refused };
        assert_eq!(access.outcome(*ehr, &item), expected, "{ehr:?}");
    }
    let nobody = request_of(&Ask::subject(&world, None));
    Box::pin(with_request_access(
        Arc::clone(&nobody),
        world.svc.judge_request_ehrs(&ehrs),
    ))
    .await
    .expect("the EHRs are judged");
    for ehr in &ehrs {
        assert_eq!(
            nobody.outcome(*ehr, &item),
            refused,
            "a patient context naming no EHR reaches none"
        );
    }
}

/// The condition `access` applies to a query of the running request, with
/// its care unread, as the protocol adapter hands it to the service.
async fn unresolved(world: &World, access: &Arc<RequestAccess>) -> AccessCondition {
    let classes = world
        .svc
        .ehr_access_classes()
        .await
        .expect("the classes read");
    access.condition(&classes, Care::Unresolved)
}

/// A query pinned to one EHR is decided before its statement, from the
/// copy of that EHR's settings, so the statement carries only what the
/// verdict leaves to each object; a query over several EHRs keeps the class
/// condition (#5746).
#[tokio::test]
async fn a_query_pinned_to_one_ehr_is_decided_before_the_statement() {
    let world = Box::pin(World::new()).await;
    let hidden = world.objects(1)[0].clone();
    let restriction = directive(
        "restriction",
        "deny",
        &json!({ "data": [{ "objects": [object_ref(&hidden)] }] }),
    );
    world
        .settings(1, &settings("open", &[restriction], &[]))
        .await;
    let ask = Ask::professional(&world);
    let access = request_of(&ask);
    let (one, two) = (world.ehrs[1].0, world.ehrs[0].0);
    let svc = &world.svc;
    let (pinned, unpinned, several) = Box::pin(with_request_access(Arc::clone(&access), async {
        let condition = unresolved(&world, &access).await;
        let pin = |ehrs: Option<Vec<EhrId>>| {
            let condition = condition.clone();
            async move {
                Box::pin(svc.query_access_condition(&condition, ehrs.as_deref()))
                    .await
                    .expect("the condition reads")
            }
        };
        (
            pin(Some(vec![one])).await,
            pin(None).await,
            pin(Some(vec![one, two])).await,
        )
    }))
    .await;
    let decided = pinned.pinned().expect("the one EHR is decided");
    assert_eq!(decided.ehr, one.0);
    assert!(decided.settled, "EHR 1 has a settings row");
    assert_ne!(
        decided.admitted,
        Pred::True,
        "the restriction is left to each object"
    );
    assert_ne!(
        decided.admitted,
        Pred::False,
        "the rest of the EHR is admitted"
    );
    assert!(
        unpinned.pinned().is_none(),
        "a query over any EHR is not pinned"
    );
    assert!(
        several.pinned().is_none(),
        "a query over two EHRs is not pinned"
    );
    let request = AqlQueryRequest {
        ehr_access: ferroehr::service::query::request::QueryAccess::Given(pinned),
        ..AqlQueryRequest::default()
    };
    let outcome = Box::pin(with_request_access(
        Arc::clone(&access),
        svc.execute_aql(&pinned_to(COMPOSITIONS, one), None, &request),
    ))
    .await
    .expect("the query runs");
    let served: Vec<String> = outcome.result_set.to_value().unwrap()["rows"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|row| row[0].as_str())
        .map(|uid| uid.split("::").next().unwrap_or_default().to_owned())
        .collect();
    let expected: Vec<String> = world
        .objects(1)
        .into_iter()
        .filter(|id| *id != hidden)
        .collect();
    assert_eq!(served, expected, "the restricted object is withheld");
}

/// A verdict judged on a copy of the EHR's settings older than the row in
/// force never answers from that copy: the statement fails, the service
/// reads the copy again and answers what the row in force admits; a store
/// whose revision keeps moving past every copy is refused with `503` once
/// the wait has passed, never answered with no rows (#5746, #5794).
#[tokio::test]
async fn a_pinned_verdict_judged_on_an_older_copy_is_judged_again_or_refused() {
    let world = Box::pin(World::new()).await;
    world.settings(0, &settings("open", &[], &[])).await;
    let ask = Ask::professional(&world);
    let access = request_of(&ask);
    let ehr_id = world.ehrs[0].0;
    let svc = &world.svc;
    let aql = pinned_to(COMPOSITIONS, ehr_id);
    let run = |older: bool| {
        let access = Arc::clone(&access);
        let aql = aql.clone();
        let world = &world;
        async move {
            Box::pin(with_request_access(Arc::clone(&access), async {
                let condition = unresolved(world, &access)
                    .await
                    .with_care(Care::Ehrs(Vec::new().into()));
                let mut entry = svc.ehr_access_entry(ehr_id).await.expect("the copy reads");
                if older {
                    entry.revision.settings = entry.revision.settings.map(|ordinal| ordinal - 1);
                }
                let request = AqlQueryRequest {
                    ehr_access: ferroehr::service::query::request::QueryAccess::Given(access.pin(&condition, ehr_id, &entry)),
                    ..AqlQueryRequest::default()
                };
                assert!(
                    matches!(
                        &request.ehr_access,
                        ferroehr::service::query::request::QueryAccess::Given(c) if c.pinned().is_some()
                    ),
                    "the EHR is decided"
                );
                Box::pin(svc.execute_aql(&aql, None, &request)).await
            }))
            .await
        }
    };
    for older in [false, true] {
        let outcome = run(older).await.expect("the query runs");
        let served: Vec<String> = outcome.result_set.to_value().unwrap()["rows"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|row| row[0].as_str())
            .map(|uid| uid.split("::").next().unwrap_or_default().to_owned())
            .collect();
        assert_eq!(
            served,
            world.objects(0),
            "a copy older than the row: {older}"
        );
    }
    // NOTE: our own design (#5794, #5888), a store that moves past every read of the copy
    // is simulated by a revision that moves on every read of it.
    sqlx::raw_sql(
        "CREATE SEQUENCE clinical.moving_revision; \
         CREATE OR REPLACE VIEW clinical.access_revision_of AS \
         SELECT e.id AS ehr_id, json_build_array(nextval('clinical.moving_revision'), \
         e.subject_namespace, e.subject_id)::text AS revision FROM clinical.ehr e",
    )
    .execute(&world.db.pool())
    .await
    .expect("the revision moves on every read");
    let started = std::time::Instant::now();
    let refused = run(true)
        .await
        .expect_err("a copy that never catches up is refused");
    assert!(
        matches!(refused, ferroehr::error::ServiceError::AccessRefreshing(_)),
        "the refusal is the 503 of a refreshing access state: {refused:?}"
    );
    assert!(
        started.elapsed() >= std::time::Duration::from_millis(100),
        "the service asked again for the bounded wait first"
    );
}

/// Break-glass: where the policy requires a care relationship, the
/// break-glass role declaring an emergency purpose reads every EHR without
/// one, a restriction whose `emergency_override` holds lifted and a consent
/// deny without it still in force (EHDS Art. 11(5)); without the emergency
/// it reads nothing.
#[tokio::test]
async fn break_glass_reads_what_the_emergency_lifts() {
    let world = Box::pin(World::new()).await;
    let restricted = world.objects(1)[0].clone();
    let mut restriction = directive(
        "restriction",
        "deny",
        &json!({ "data": [{ "objects": [object_ref(&restricted)] }] }),
    );
    restriction["emergency_override"] = json!(true);
    world
        .settings(1, &settings("open", &[restriction], &[]))
        .await;
    let deny = directive(
        "consent",
        "deny",
        &json!({ "applies_to": [{ "party": {
        "_type": "PARTY_REF", "namespace": "local", "type": "PERSON",
        "id": { "_type": "HIER_OBJECT_ID", "value": world.nurse } } }] }),
    );
    world.settings(2, &settings("open", &[deny], &[])).await;
    let mut ask = Ask::professional(&world);
    ask.policy = Some(policy(&json!({ "care_relationship_required": true })));
    assert_reads(&world, &ask, &[], "no relationship and no emergency").await;
    ask.emergency = Emergency {
        declared: true,
        break_glass: true,
        opt_out_override: false,
    };
    assert_reads(&world, &ask, &except(&world, &[2], &[]), "break-glass").await;
}

/// The newest compositions, as the benchmark's population page asks for
/// them (#5782).
const NEWEST: &str = "SELECT c/uid/value, c/context/start_time/value FROM EHR e \
     CONTAINS COMPOSITION c ORDER BY c/context/start_time/value DESC, c/uid/value ASC";

/// The composition ids of the rows of `aql` `ask` reads, sorted.
async fn sorted_ids(world: &World, ask: &Ask, aql: &str) -> Vec<String> {
    let mut ids: Vec<String> = rows(world, ask, aql, None)
        .await
        .iter()
        .filter_map(|row| row[0].as_str())
        .map(|uid| uid.split("::").next().unwrap_or_default().to_owned())
        .collect();
    ids.sort();
    ids
}

/// The composition ids `access` serves for `aql` under `condition`, in row
/// order.
async fn served_under(
    world: &World,
    access: &Arc<RequestAccess>,
    condition: AccessCondition,
    aql: &str,
) -> Vec<String> {
    let request = AqlQueryRequest {
        ehr_access: ferroehr::service::query::request::QueryAccess::Given(condition),
        ..AqlQueryRequest::default()
    };
    let outcome = Box::pin(with_request_access(
        Arc::clone(access),
        world.svc.execute_aql(aql, None, &request),
    ))
    .await
    .expect("the query runs");
    outcome.result_set.to_value().unwrap()["rows"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|row| row[0].as_str())
        .map(|uid| uid.split("::").next().unwrap_or_default().to_owned())
        .collect()
}

/// Whether the condition of `ask` admits every item of every class it
/// knows, of an EHR held closed and of an EHR with no settings, so the
/// statement reads it as "the EHR's class is known" (#5782).
async fn admits_every_known(world: &World, ask: &Ask) -> bool {
    let access = request_of(ask);
    let classes = world.svc.ehr_access_classes().await.expect("the classes");
    let condition = Box::pin(with_request_access(Arc::clone(&access), async {
        access.condition(&classes, Care::Census(Vec::new().into()))
    }))
    .await;
    let groups = condition.groups().expect("a condition over the classes");
    groups.default.is_none()
        && groups.closed.admits_all()
        && groups.groups.iter().all(|(_, pair)| pair.admits_all())
}

/// A restriction scoped by root archetype withholds the objects whose trunk
/// head has that root, case-folded, read from the head row the commit
/// writes, and no object of another root (#5782).
#[tokio::test]
async fn an_archetype_scope_reads_the_root_of_the_trunk_head() {
    let world = Box::pin(World::new()).await;
    let restriction = |archetype: &str| {
        directive(
            "restriction",
            "deny",
            &json!({ "data": [{ "archetypes": [
                { "_type": "ARCHETYPE_ID", "value": archetype }] }] }),
        )
    };
    let mut report = composition("report");
    report["archetype_node_id"] = json!("openEHR-EHR-COMPOSITION.report.v1");
    report["archetype_details"]["archetype_id"]["value"] =
        json!("openEHR-EHR-COMPOSITION.report.v1");
    let reported = world
        .svc
        .create_composition(world.ehrs[1].0, uv(&report, "249", None))
        .await
        .expect("the report commits")
        .vo_id
        .0
        .to_string();
    let ask = Ask::professional(&world);
    let mut unrestricted = world.every();
    unrestricted.push(reported.clone());
    unrestricted.sort();
    assert_eq!(
        sorted_ids(&world, &ask, UNLINKED).await,
        unrestricted,
        "no scope withholds nothing"
    );
    world
        .settings(
            0,
            &settings(
                "open",
                &[restriction("openEHR-EHR-COMPOSITION.ENCOUNTER.v1")],
                &[],
            ),
        )
        .await;
    world
        .settings(
            1,
            &settings(
                "open",
                &[restriction("openEHR-EHR-COMPOSITION.report.v1")],
                &[],
            ),
        )
        .await;
    let expected = except(&world, &[0], &[]);
    assert!(!expected.contains(&reported), "the report is EHR 1's own");
    assert_reads(&world, &ask, &expected, "the encounters and the report").await;
}

/// The issuer of the hospital's episode identifiers.
const HOSPITAL: &str = "urn:example:hospital";

impl World {
    /// Commits to EHR `index` a composition placed in the hospital's episode
    /// `episode`, and returns its id.
    async fn place(&mut self, index: usize, episode: &str) -> String {
        let mut body = composition(episode);
        body["context"] = json!({ "_type": "EVENT_CONTEXT",
            "start_time": { "_type": "DV_DATE_TIME", "value": "2026-11-23T10:00:00Z" },
            "setting": { "_type": "DV_CODED_TEXT", "value": "secondary medical care",
                "defining_code": { "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                    "code_string": "232" } },
            "other_context": { "_type": "ITEM_TREE", "archetype_node_id": "at0001",
                "name": { "_type": "DV_TEXT", "value": "Tree" },
                "items": [{ "_type": "ELEMENT", "archetype_node_id": "at9001",
                    "name": { "_type": "DV_CODED_TEXT", "value": "episode",
                        "defining_code": { "_type": "CODE_PHRASE",
                            "terminology_id": { "_type": "TERMINOLOGY_ID",
                                "value": "openehr_access" },
                            "code_string": "episode" } },
                    "value": { "_type": "DV_IDENTIFIER", "issuer": HOSPITAL,
                        "id": episode } }] } });
        let (ehr_id, _, objects) = &mut self.ehrs[index];
        let id = self
            .svc
            .create_composition(*ehr_id, uv(&body, "249", None))
            .await
            .expect("the placed composition commits")
            .vo_id
            .0
            .to_string();
        objects.push(id.clone());
        objects.sort();
        id
    }

    /// Whether a named read of the composition `vo` of EHR `index` by `ask`
    /// is admitted.
    async fn reads(&self, ask: &Ask, index: usize, vo: &str) -> bool {
        use ferroehr::service::ehr::composition_read::JudgedRead;
        let access = request_of(ask);
        let ehr_id = self.ehrs[index].0;
        let vo = VoId(vo.parse().expect("a composition id"));
        let run = self.svc.composition_judged(&access, ehr_id, vo, None);
        match Box::pin(with_request_access(Arc::clone(&access), run)).await {
            JudgedRead::Admitted(read) => read.is_ok(),
            JudgedRead::Absent | JudgedRead::Refused { .. } => false,
            JudgedRead::Unheld(error) | JudgedRead::Unjudged(error) => {
                panic!("the read is judged: {error}")
            }
        }
    }
}

/// A restriction naming an episode withholds the compositions placed in it,
/// from a query and from a named read, and nothing else; two EHRs whose
/// restrictions name different episodes share one class, and the query
/// reads each EHR's own episode from its facts (#6416).
#[tokio::test]
async fn an_episode_scope_reads_the_episodes_of_the_trunk_head() {
    let mut world = Box::pin(World::new()).await;
    let dermatology = world.place(0, "D-4").await;
    let cardiology = world.place(0, "C-17").await;
    let elsewhere = world.place(1, "D-4").await;
    let ask = Ask::professional(&world);
    assert_reads(&world, &ask, &world.every(), "no scope").await;
    let restriction = |episode: &str| {
        directive(
            "restriction",
            "deny",
            &json!({ "data": [{ "episodes": [{ "_type": "DV_IDENTIFIER",
                "issuer": HOSPITAL, "id": episode }] }] }),
        )
    };
    world
        .settings(0, &settings("open", &[restriction("D-4")], &[]))
        .await;
    world
        .settings(1, &settings("open", &[restriction("C-17")], &[]))
        .await;
    assert_eq!(classes(&world).await, 1, "one class for both restrictions");
    let expected = except(&world, &[], &[&dermatology]);
    assert_reads(&world, &ask, &expected, "each EHR's own episode").await;
    assert!(
        !world.reads(&ask, 0, &dermatology).await,
        "the episode named"
    );
    assert!(world.reads(&ask, 0, &cardiology).await, "another episode");
    assert!(
        world.reads(&ask, 1, &elsewhere).await,
        "another EHR's scope"
    );
    let unplaced = world.objects(0)[0].clone();
    assert!(![&dermatology, &cardiology].contains(&&unplaced));
    assert!(
        world.reads(&ask, 0, &unplaced).await,
        "an unplaced composition"
    );
}

/// A caller every known class, an EHR held closed and an EHR with no
/// settings admit in full reads every EHR of a known class, by every query
/// shape, and an EHR of a class its condition does not hold once the
/// classes are read again (#5794); a caller
/// a class refuses keeps the full condition and reads without that class's
/// EHR (#5782).
#[tokio::test]
async fn a_condition_admitting_every_known_class_reads_as_the_full_one() {
    let world = Box::pin(World::new()).await;
    world
        .settings(
            0,
            &settings("open", &[directive("opt_out", "deny", &json!({}))], &[]),
        )
        .await;
    world.settings(1, &settings("restricted", &[], &[])).await;
    let mut administrator = Ask::professional(&world);
    administrator.caller.administrator = true;
    assert!(
        admits_every_known(&world, &administrator).await,
        "the administrator's condition admits every known class"
    );
    assert_reads(&world, &administrator, &world.every(), "the administrator").await;
    assert_eq!(
        sorted_ids(&world, &administrator, NEWEST).await,
        world.every(),
        "the administrator's newest compositions"
    );
    let professional = Ask::professional(&world);
    assert!(
        !admits_every_known(&world, &professional).await,
        "a class refuses the professional something"
    );
    let expected = except(&world, &[1], &[]);
    assert_reads(&world, &professional, &expected, "the professional").await;
    assert_eq!(
        sorted_ids(&world, &professional, NEWEST).await,
        expected,
        "the professional's newest compositions"
    );
    let access = request_of(&administrator);
    let stale = Box::pin(with_request_access(Arc::clone(&access), async {
        access.condition(
            &world
                .svc
                .ehr_access_classes()
                .await
                .expect("the classes read"),
            Care::Census(Vec::new().into()),
        )
    }))
    .await;
    world
        .settings(
            2,
            &settings("open", &[opt_out_from("2020-01-01T00:00:00Z")], &[]),
        )
        .await;
    for aql in [COMPOSITIONS, UNLINKED] {
        assert_eq!(
            served_under(&world, &access, stale.clone(), aql).await,
            world.every(),
            "the administrator's stale condition is judged again: {aql}"
        );
    }
    assert!(
        admits_every_known(&world, &administrator).await,
        "the classes read again"
    );
    assert_reads(
        &world,
        &administrator,
        &world.every(),
        "the classes read again",
    )
    .await;
}

/// The `EHR_ACCESS` of an EHR a pinned query reads is answered as the query
/// over every EHR answers it: to an administrator, who reads every EHR's
/// settings, and to a caller of no administrator role, whom no settings name
/// a gate-keeper, and who reads none (#5746).
#[tokio::test]
async fn a_pinned_query_reads_the_settings_the_caller_may_read() {
    use ferroehr::system_log::access_context::with_roles;
    use ferroehr::versioning::committer::with_committer;
    let world = Box::pin(World::new()).await;
    world.settings(0, &settings("open", &[], &[])).await;
    let aql = "SELECT a/uid/value FROM EHR e CONTAINS EHR_ACCESS a ORDER BY a/uid/value";
    let ask = Ask::professional(&world);
    for (role, each) in [("ADMIN", 1), ("USER", 0)] {
        let roles = || vec![role.to_owned()];
        let doctor = || {
            Some(CommitterIdentity {
                subject: "doctor".to_owned(),
                ..administrator()
            })
        };
        let every = Box::pin(with_roles(
            roles(),
            with_committer(doctor(), first_column(&world, &ask, aql)),
        ))
        .await;
        assert_eq!(every.len(), world.ehrs.len() * each, "{role}: the census");
        let mut pinned_all = Vec::new();
        for (index, (ehr_id, _, _)) in world.ehrs.iter().enumerate() {
            let pinned = Box::pin(with_roles(
                roles(),
                with_committer(
                    doctor(),
                    first_column(&world, &ask, &pinned_to(aql, *ehr_id)),
                ),
            ))
            .await;
            assert_eq!(pinned.len(), each, "{role}: EHR {index} pinned");
            pinned_all.extend(pinned);
        }
        pinned_all.sort_by_key(ToString::to_string);
        let mut every = every;
        every.sort_by_key(ToString::to_string);
        assert_eq!(
            pinned_all, every,
            "{role}: the pinned answers are the census's"
        );
    }
}
