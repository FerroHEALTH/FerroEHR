// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! AQL over the demographic store leaves the policy parties out for a caller
//! who is no administrator (#5450), over a real PostgreSQL 18: a party with a
//! version holding a capability coded in FerroEHR's operation terminology,
//! an acts-for relationship or a client identifier, in any archetype, and an
//! acts-for `PARTY_RELATIONSHIP` of its own contribute no row, no count and
//! no page, under every root class (#5474), through `CONTAINS` and through
//! the join to an `EHR`, and keep doing so after an administrator turns them
//! into ordinary parties (#5485). An administrator's query returns them, and
//! every other party stays anyone's to query. QUERY master03 §FROM gives
//! semantics to the clinical repository only and no openEHR spec governs who
//! reads a party: our own design.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use crate::fixtures::at;
use crate::policy_parties::{
    AGENT, PERSON, PROVIDER, as_admin, as_clinician, assert_policy_refusal, body, create, delete,
    mapped, plain, referred, system_client_under, update,
};
use ferroehr::error::ServiceError;
use ferroehr::ids::VoId;
use ferroehr::party_binding::policy::ACTS_FOR_ARCHETYPE;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::query::request::AqlQueryRequest;
use ferroehr::service::version_update::WriteRequest;

/// The rows `aql` answers.
async fn rows(svc: &FerroEhrService, aql: &str) -> Result<Vec<Value>, ServiceError> {
    let outcome = svc
        .execute_aql(aql, None, &AqlQueryRequest::default())
        .await?;
    Ok(outcome.result_set["rows"]
        .as_array()
        .expect("a rows array")
        .clone())
}

/// The first cell of each row `aql` answers, as text.
async fn first_cells(svc: &FerroEhrService, aql: &str) -> Vec<String> {
    rows(svc, aql)
        .await
        .unwrap_or_else(|e| panic!("query {aql:?}: {e:?}"))
        .iter()
        .map(|row| row[0].as_str().expect("a text cell").to_owned())
        .collect()
}

/// The `uid/value` of each `class` root, read whole, read as a count, read
/// one full ordered page at a time and read under a `LIMIT` alone; the four
/// readings agree, and the set is answered.
async fn every_reading(svc: &FerroEhrService, class: &str) -> BTreeSet<String> {
    let all: BTreeSet<String> = first_cells(svc, &format!("SELECT x/uid/value FROM {class} x"))
        .await
        .into_iter()
        .collect();

    let counted = rows(svc, &format!("SELECT COUNT(x/uid/value) FROM {class} x"))
        .await
        .expect("the count");
    assert_eq!(counted, vec![json!([all.len()])], "the {class} count");

    let mut paged = BTreeSet::new();
    for offset in (0..=all.len()).step_by(2) {
        let page = first_cells(
            svc,
            &format!(
                "SELECT x/uid/value FROM {class} x ORDER BY x/uid/value LIMIT 2 OFFSET {offset}"
            ),
        )
        .await;
        // A page is full unless it is the last, so no row was dropped after paging.
        assert_eq!(
            page.len(),
            2.min(all.len() - offset),
            "the {class} page at {offset}: {page:?}"
        );
        paged.extend(page);
    }
    assert_eq!(paged, all, "the {class} pages");

    let streamed: BTreeSet<String> =
        first_cells(svc, &format!("SELECT x/uid/value FROM {class} x LIMIT 100"))
            .await
            .into_iter()
            .collect();
    assert_eq!(streamed, all, "the {class} rows under a LIMIT");
    all
}

/// The acts-for relationship of the book's example from `source`, of
/// `class`, to the provider `provider`.
fn acts_for(provider: VoId, source: VoId, class: &str) -> Value {
    let mut relationship =
        body("agent-acts-for", &[(PROVIDER, provider)])["relationships"][0].clone();
    relationship["source"]["type"] = json!(class);
    relationship["source"]["id"]["value"] = json!(source.to_string());
    relationship
}

/// The root classes a demographic query names, concrete and abstract.
const ROOTS: [&str; 8] = [
    "PERSON",
    "ORGANISATION",
    "GROUP",
    "AGENT",
    "ROLE",
    "ACTOR",
    "PARTY",
    "PARTY_RELATIONSHIP",
];

/// The concrete classes a root class covers (RM demographic
/// `master02-demographic_package.adoc` §Class Descriptions).
fn covered(class: &str) -> &'static [&'static str] {
    match class {
        "ACTOR" => &["PERSON", "ORGANISATION", "GROUP", "AGENT"],
        "PARTY" => &["PERSON", "ORGANISATION", "GROUP", "AGENT", "ROLE"],
        "PERSON" => &["PERSON"],
        "ORGANISATION" => &["ORGANISATION"],
        "GROUP" => &["GROUP"],
        "AGENT" => &["AGENT"],
        "ROLE" => &["ROLE"],
        "PARTY_RELATIONSHIP" => &["PARTY_RELATIONSHIP"],
        other => panic!("no root class {other}"),
    }
}

/// The policy parties and the ordinary ones, by concrete class.
struct Fixture {
    /// The ordinary `uid/value`s, by concrete class.
    ordinary: BTreeMap<&'static str, BTreeSet<String>>,
    /// The policy `uid/value`s, by concrete class.
    policy: BTreeMap<&'static str, BTreeSet<String>>,
    /// The acting `PERSON`, which holds an acts-for relationship inline.
    acting: VoId,
}

impl Fixture {
    /// The `uid/value`s of `side` under the root class `class`.
    fn under(side: &BTreeMap<&'static str, BTreeSet<String>>, class: &str) -> BTreeSet<String> {
        covered(class)
            .iter()
            .flat_map(|concrete| side.get(concrete).into_iter().flatten().cloned())
            .collect()
    }
}

/// A standalone `PARTY_RELATIONSHIP` from `source` to `target`, of the
/// archetype `archetype`.
fn standalone(
    archetype: &str,
    (source, source_class): (VoId, &str),
    (target, target_class): (VoId, &str),
) -> Value {
    json!({
        "_type": "PARTY_RELATIONSHIP",
        "archetype_node_id": archetype,
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
            "rm_version": "1.1.0" },
        "name": { "_type": "DV_TEXT", "value": "relationship" },
        "source": { "_type": "PARTY_REF", "namespace": "demographic", "type": source_class,
            "id": { "_type": "HIER_OBJECT_ID", "value": source.to_string() } },
        "target": { "_type": "PARTY_REF", "namespace": "demographic", "type": target_class,
            "id": { "_type": "HIER_OBJECT_ID", "value": target.to_string() } }
    })
}

/// Commits `body` as a standalone `PARTY_RELATIONSHIP` and returns its
/// `uid/value`.
async fn relate(svc: &FerroEhrService, body: &Value) -> Result<String, ServiceError> {
    let created =
        Box::pin(svc.party_relationship_create(WriteRequest::new(crate::typed_body::typed(body))))
            .await?;
    Ok(created.meta.expect("a version").uid)
}

/// Commits, beside the ordinary parties of every class, a system `AGENT`, a
/// system client `ROLE`, a privileged `ROLE`, a `ROLE` of a specialised
/// system client archetype, an `ORGANISATION` and a `PERSON` holding an
/// acts-for relationship inline, and an acts-for `PARTY_RELATIONSHIP` of its
/// own, each policy party written by an administrator.
async fn fixture(svc: &FerroEhrService) -> Fixture {
    let [
        (provider, provider_uid),
        (partner, partner_uid),
        (person, person_uid),
    ] = referred(svc).await;
    let mut nurse = plain("ROLE", "openEHR-DEMOGRAPHIC-ROLE.role.v0", "Nurse");
    nurse["performer"] = json!({ "_type": "PARTY_REF", "namespace": "demographic",
        "type": "PERSON", "id": { "_type": "HIER_OBJECT_ID", "value": person.to_string() } });
    let (_, nurse_uid) = as_clinician(create(svc, PartyKind::Role, &nurse))
        .await
        .expect("an ordinary ROLE");
    let (_, ward_uid) = as_clinician(create(
        svc,
        PartyKind::Group,
        &plain("GROUP", "openEHR-DEMOGRAPHIC-GROUP.group.v0", "Ward team"),
    ))
    .await
    .expect("an ordinary GROUP");
    let (_, analyser_uid) = as_clinician(create(
        svc,
        PartyKind::Agent,
        &plain(
            "AGENT",
            "openEHR-DEMOGRAPHIC-AGENT.device.v0",
            "Lab analyser",
        ),
    ))
    .await
    .expect("an ordinary AGENT");
    let employed = as_clinician(relate(
        svc,
        &standalone(
            "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.relationship.v0",
            (person, "PERSON"),
            (provider, "ORGANISATION"),
        ),
    ))
    .await
    .expect("an ordinary PARTY_RELATIONSHIP");

    let (agent, agent_uid) = as_admin(create(svc, PartyKind::Agent, &body("agent", &[])))
        .await
        .expect("an administrator creates the AGENT");
    let mut held_alone = acts_for(provider, agent, "AGENT");
    held_alone["archetype_details"] = json!({ "_type": "ARCHETYPED",
        "archetype_id": { "_type": "ARCHETYPE_ID", "value": ACTS_FOR_ARCHETYPE },
        "rm_version": "1.1.0" });
    let acts_for_standalone = as_admin(relate(svc, &held_alone))
        .await
        .expect("an administrator names the provider the AGENT acts for");
    let (_, system_client) = as_admin(create(
        svc,
        PartyKind::Role,
        &body("system-client-role", &[(AGENT, agent)]),
    ))
    .await
    .expect("an administrator grants the system client ROLE");
    let (_, privileged) = as_admin(create(
        svc,
        PartyKind::Role,
        &body("privileged-role", &[(PERSON, person)]),
    ))
    .await
    .expect("an administrator grants the privileged ROLE");

    let specialised = crate::adl2_fixture::adl2_source(
        "archetype",
        "openEHR-DEMOGRAPHIC-ROLE.system_client-hospital.v1.0.0",
        Some("openEHR-DEMOGRAPHIC-ROLE.system_client.v1"),
    );
    let template = svc
        .template_adl2_upload(&specialised)
        .await
        .expect("the deployment uploads a specialisation");
    let (_, hospital) = as_admin(create(
        svc,
        PartyKind::Role,
        &system_client_under(
            &template,
            "openEHR-DEMOGRAPHIC-ROLE.system_client-hospital.v1",
            agent,
        ),
    ))
    .await
    .expect("an administrator grants the specialised ROLE");

    let mut network = plain(
        "ORGANISATION",
        "openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v0",
        "Regional care network",
    );
    network["relationships"] = json!([acts_for(provider, partner, "ORGANISATION")]);
    let network = as_admin(update(
        svc,
        PartyKind::Organisation,
        partner,
        &partner_uid,
        &network,
    ))
    .await
    .expect("an administrator names the provider the network acts for");

    let (deputy, deputy_uid) = as_clinician(create(
        svc,
        PartyKind::Person,
        &plain(
            "PERSON",
            "openEHR-DEMOGRAPHIC-PERSON.person.v0",
            "A. de Vries",
        ),
    ))
    .await
    .expect("a person");
    let mut acting = plain(
        "PERSON",
        "openEHR-DEMOGRAPHIC-PERSON.person.v0",
        "A. de Vries",
    );
    acting["relationships"] = json!([acts_for(provider, deputy, "PERSON")]);
    let acting = as_admin(update(svc, PartyKind::Person, deputy, &deputy_uid, &acting))
        .await
        .expect("an administrator names the provider the person acts for");

    let set = |uids: &[&String]| uids.iter().map(|uid| (*uid).clone()).collect();
    Fixture {
        ordinary: BTreeMap::from([
            ("ROLE", set(&[&nurse_uid])),
            ("PERSON", set(&[&person_uid])),
            ("ORGANISATION", set(&[&provider_uid])),
            ("GROUP", set(&[&ward_uid])),
            ("AGENT", set(&[&analyser_uid])),
            ("PARTY_RELATIONSHIP", set(&[&employed])),
        ]),
        policy: BTreeMap::from([
            ("ROLE", set(&[&system_client, &privileged, &hospital])),
            ("PERSON", set(&[&acting])),
            ("ORGANISATION", set(&[&network])),
            ("AGENT", set(&[&agent_uid])),
            ("PARTY_RELATIONSHIP", set(&[&acts_for_standalone])),
        ]),
        acting: deputy,
    }
}

/// Per root class, concrete and abstract, a caller who is no administrator
/// reads every ordinary party and no policy party, whole, counted and paged;
/// an administrator reads both.
#[tokio::test]
async fn a_demographic_query_leaves_the_policy_parties_out_for_a_non_administrator() {
    let (_db, svc) = mapped().await;
    let fixture = fixture(&svc).await;
    for class in ROOTS {
        let ordinary = Fixture::under(&fixture.ordinary, class);
        let policy = Fixture::under(&fixture.policy, class);
        assert!(
            !ordinary.is_empty(),
            "the fixture holds an ordinary {class}"
        );
        let seen = as_clinician(every_reading(&svc, class)).await;
        assert_eq!(seen, ordinary, "a clinician's {class} query");
        let seen = as_admin(every_reading(&svc, class)).await;
        let both: BTreeSet<String> = ordinary.union(&policy).cloned().collect();
        assert_eq!(seen, both, "an administrator's {class} query");
    }
}

/// A node reached through `CONTAINS` belongs to its root's version, so a
/// policy `ROLE`'s `CAPABILITY` is left out with it, under the `ROLE` root
/// and under the abstract `PARTY`, and so is a policy party read under
/// `VERSION`.
#[tokio::test]
async fn a_contained_node_of_a_policy_party_is_left_out() {
    let (_db, svc) = mapped().await;
    let fixture = fixture(&svc).await;
    let roles = &fixture.policy["ROLE"];

    for root in ["ROLE", "PARTY"] {
        let contained =
            format!("SELECT r/uid/value, c/name/value FROM {root} r CONTAINS CAPABILITY c");
        let clinician: Vec<String> = as_clinician(first_cells(&svc, &contained)).await;
        assert_eq!(
            clinician,
            Vec::<String>::new(),
            "a clinician's capabilities under {root}"
        );
        let admin: BTreeSet<String> = as_admin(first_cells(&svc, &contained))
            .await
            .into_iter()
            .collect();
        assert_eq!(
            &admin, roles,
            "an administrator's capabilities under {root}"
        );
    }

    for class in ["ROLE", "AGENT", "PARTY", "PARTY_RELATIONSHIP"] {
        let versioned = format!("SELECT r/uid/value FROM VERSION v CONTAINS {class} r");
        let clinician: BTreeSet<String> = as_clinician(first_cells(&svc, &versioned))
            .await
            .into_iter()
            .collect();
        assert_eq!(
            clinician,
            Fixture::under(&fixture.ordinary, class),
            "a clinician's {class} versions"
        );
    }
}

/// `FROM AGENT` and `FROM PARTY` serve the ordinary parties and leave the
/// system's `AGENT` out for a caller who is no administrator; an
/// administrator's query returns it.
#[tokio::test]
async fn the_agent_and_party_roots_leave_the_system_agent_out() {
    let (_db, svc) = mapped().await;
    let (_, agent) = as_admin(create(&svc, PartyKind::Agent, &body("agent", &[])))
        .await
        .expect("an administrator creates the AGENT");
    for class in ["AGENT", "PARTY"] {
        let aql = format!("SELECT a/uid/value FROM {class} a");
        let clinician = as_clinician(first_cells(&svc, &aql)).await;
        assert!(
            !clinician.contains(&agent),
            "a clinician's {class} query leaves the system AGENT out: {clinician:?}"
        );
        let admin = as_admin(first_cells(&svc, &aql)).await;
        assert!(
            admin.contains(&agent),
            "an administrator's {class} query returns the system AGENT: {admin:?}"
        );
    }
}

/// The party half of a query rooted at the abstract `PARTY` that joins to the
/// `EHR` its party is the subject of leaves a policy party out: the `PERSON`
/// holding an acts-for relationship joins to its EHR for an administrator
/// only.
#[tokio::test]
async fn a_party_join_leaves_a_policy_party_out() {
    let (_db, svc) = mapped().await;
    let fixture = fixture(&svc).await;
    let ehr = crate::fixtures::create_ehr_for_party(&svc, fixture.acting).await;
    let aql = "SELECT p/uid/value, e/ehr_id/value FROM PARTY p CONTAINS EHR e";
    let clinician = as_clinician(rows(&svc, aql))
        .await
        .expect("a clinician's join");
    assert!(clinician.is_empty(), "a clinician's join: {clinician:?}");
    let admin = as_admin(rows(&svc, aql))
        .await
        .expect("an administrator's join");
    assert_eq!(
        admin,
        vec![json!([
            fixture.policy["PERSON"].first().expect("the acting PERSON"),
            ehr.to_string()
        ])],
        "an administrator's join"
    );
}

/// The relations every scan of `plan` reads, sorted.
fn relations(plan: &Value) -> Vec<String> {
    fn walk(node: &Value, out: &mut Vec<String>) {
        if let Some(relation) = node.get("Relation Name").and_then(Value::as_str) {
            out.push(relation.to_owned());
        }
        for child in node
            .get("Plans")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            walk(child, out);
        }
    }
    let mut out = Vec::new();
    walk(&plan[0]["Plan"], &mut out);
    out.sort_unstable();
    out
}

/// The plan of `aql` on the party pool of `db`, built for `policy_parties`.
async fn plan_of(
    db: &testkit::TestDb,
    aql: &str,
    policy_parties: ferroehr::aql::sql::PolicyParties,
) -> Value {
    use ferroehr::aql::sql::{EhrAccessScope, EhrScopeKind, SqlCtx};

    let ast = openehr_query::parser::parse_str(aql).expect("the AQL parses");
    let params = ferroehr::aql::ir::Params::default();
    let ir = ferroehr::aql::plan(
        &ast,
        &params,
        ferroehr::config::profile::SpecProfile::default(),
    )
    .expect("the AQL plans");
    let ctx = SqlCtx {
        system_id: "policy.parties.test".to_owned(),
        ehr_ids: Vec::new(),
        ehr_scope_kind: EhrScopeKind::Named,
        subject_scope: None,
        ehr_access: EhrAccessScope::Unrestricted,
        limit: ir.limit,
        offset: None,
        archetype_lineage: std::sync::Arc::default(),
        policy_parties,
    };
    let prepared = ferroehr::aql::sql::build(&ir, &params, &ctx).expect("the AQL lowers");
    let party = ferroehr::db::domain_pool_from(&db.pool(), ferroehr::db::domain::Domain::Party);
    sqlx::query_scalar_with(
        sqlx::AssertSqlSafe(format!("EXPLAIN (ANALYZE, FORMAT JSON) {}", prepared.sql)),
        prepared.values,
    )
    .fetch_one(&party)
    .await
    .expect("explain the query")
}

/// The filter adds no statement, so no transaction, to a query, and reads no
/// relation the query does not read anyway: it tests the policy mark of the
/// head row each root reaches by its primary key.
#[tokio::test]
async fn the_filter_reads_no_relation_the_query_does_not_read() {
    use ferroehr::aql::sql::PolicyParties;

    let (db, svc) = mapped().await;
    fixture(&svc).await;
    for aql in [
        "SELECT x/uid/value FROM ROLE x ORDER BY x/uid/value LIMIT 2",
        "SELECT COUNT(x/uid/value) FROM PERSON x",
        "SELECT r/uid/value, c/name/value FROM ROLE r CONTAINS CAPABILITY c",
        "SELECT x/uid/value FROM PARTY x ORDER BY x/uid/value LIMIT 2",
        "SELECT COUNT(x/uid/value) FROM ACTOR x",
        "SELECT x/uid/value FROM PARTY_RELATIONSHIP x",
    ] {
        let served = plan_of(&db, aql, PolicyParties::Served).await;
        let hidden = plan_of(&db, aql, PolicyParties::Hidden).await;
        assert_eq!(
            relations(&hidden),
            relations(&served),
            "{aql}: the hidden plan {hidden} against the served plan {served}"
        );
    }
}

/// The container of the version `uid`.
fn vo_of(uid: &str) -> VoId {
    uid.split("::")
        .next()
        .and_then(|vo| vo.parse().ok())
        .expect("an OBJECT_VERSION_ID")
}

/// The policy mark of the party `vo`: when one of its versions was first a
/// policy party.
async fn mark(db: &testkit::TestDb, vo: VoId) -> Option<String> {
    sqlx::query_scalar("SELECT policy_party_at::text FROM party.vo_head WHERE vo_id = $1")
        .bind(vo.0)
        .fetch_one(&db.pool())
        .await
        .expect("the head row")
}

/// Turns each policy party of `fixture` into an ordinary one, as an
/// administrator: the policy `ROLE`s into a plain `ROLE` of the same
/// performer, the system `AGENT` into a plain `AGENT`, and the `PERSON` and
/// the `ORGANISATION` holding an acts-for relationship inline into ones
/// holding none. Returns the uid of each ordinary version, by root class. The
/// acts-for `PARTY_RELATIONSHIP` of its own is no party and stays as it is.
async fn turn_ordinary(svc: &FerroEhrService, fixture: &Fixture) -> Vec<(PartyKind, String)> {
    let persons = &fixture.ordinary["PERSON"];
    let person = vo_of(persons.first().expect("the ordinary PERSON"));
    let mut turned = Vec::new();
    for (class, uids) in &fixture.policy {
        for uid in uids {
            let (kind, body) = match *class {
                "ROLE" => {
                    let mut role =
                        plain("ROLE", "openEHR-DEMOGRAPHIC-ROLE.role.v0", "Former grant");
                    role["performer"] = json!({ "_type": "PARTY_REF", "namespace": "demographic",
                        "type": "PERSON",
                        "id": { "_type": "HIER_OBJECT_ID", "value": person.to_string() } });
                    (PartyKind::Role, role)
                }
                "PERSON" => (
                    PartyKind::Person,
                    plain(
                        "PERSON",
                        "openEHR-DEMOGRAPHIC-PERSON.person.v0",
                        "A. de Vries",
                    ),
                ),
                "ORGANISATION" => (
                    PartyKind::Organisation,
                    plain(
                        "ORGANISATION",
                        "openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v0",
                        "Regional care network",
                    ),
                ),
                "AGENT" => (
                    PartyKind::Agent,
                    plain(
                        "AGENT",
                        "openEHR-DEMOGRAPHIC-AGENT.device.v0",
                        "Former system",
                    ),
                ),
                _ => continue,
            };
            let ordinary = as_admin(update(svc, kind, vo_of(uid), uid, &body))
                .await
                .unwrap_or_else(|e| panic!("an administrator turns a {class} ordinary: {e:?}"));
            turned.push((kind, ordinary));
        }
    }
    turned
}

/// A party an administrator turns from a policy party into an ordinary one
/// stays hidden from a caller who is no administrator, every version of it,
/// through the Demographic API and through AQL alike, per served root class;
/// an administrator reads every version (#5485).
#[tokio::test]
async fn a_former_policy_party_stays_hidden_on_both_read_paths() {
    let (_db, svc) = mapped().await;
    let fixture = fixture(&svc).await;
    let turned = turn_ordinary(&svc, &fixture).await;

    for class in ROOTS {
        let ordinary = &Fixture::under(&fixture.ordinary, class);
        let policy = Fixture::under(&fixture.policy, class);
        let seen = as_clinician(every_reading(&svc, class)).await;
        assert_eq!(&seen, ordinary, "a clinician's {class} query");
        let versions =
            format!("SELECT x/uid/value FROM VERSION v[ALL_VERSIONS] CONTAINS {class} x");
        let seen: BTreeSet<String> = as_clinician(first_cells(&svc, &versions))
            .await
            .into_iter()
            .collect();
        assert_eq!(&seen, ordinary, "a clinician's {class} versions");

        let containers: BTreeSet<VoId> = policy.iter().map(|uid| vo_of(uid)).collect();
        let seen = as_admin(every_reading(&svc, class)).await;
        let admin_containers: BTreeSet<VoId> = seen.iter().map(|uid| vo_of(uid)).collect();
        assert!(
            containers.is_subset(&admin_containers),
            "an administrator's {class} query: {seen:?}"
        );
        let seen: BTreeSet<VoId> = as_admin(first_cells(&svc, &versions))
            .await
            .iter()
            .map(|uid| vo_of(uid))
            .collect();
        assert!(
            containers.is_subset(&seen),
            "an administrator's {class} versions"
        );
    }

    for (kind, uid) in &turned {
        let vo = vo_of(uid);
        assert_policy_refusal(as_clinician(svc.party_get(*kind, at(vo.to_string()), None)).await);
        assert_policy_refusal(as_clinician(svc.party_get(*kind, at(uid), None)).await);
        assert_policy_refusal(as_clinician(svc.versioned_party_get(vo)).await);
        assert_policy_refusal(as_clinician(svc.versioned_party_revision_history(vo)).await);
        as_admin(svc.party_get(*kind, at(uid), None))
            .await
            .expect("an administrator reads the former policy party");
    }
}

/// The commit of a policy version marks its party in the commit's own
/// transaction; a later ordinary version and a deletion keep the mark; an
/// ordinary party carries none, and a refused commit leaves none (#5485).
#[tokio::test]
async fn the_mark_is_written_by_the_commit_and_never_cleared() {
    let (db, svc) = mapped().await;
    let fixture = fixture(&svc).await;
    let mut marks = Vec::new();
    for uids in fixture.policy.values() {
        for uid in uids {
            let vo = vo_of(uid);
            marks.push((vo, mark(&db, vo).await.expect("a policy party is marked")));
        }
    }
    for (class, uids) in &fixture.ordinary {
        for uid in uids {
            assert_eq!(mark(&db, vo_of(uid)).await, None, "an ordinary {class}");
        }
    }

    let nurses = &fixture.ordinary["ROLE"];
    let nurse = nurses.first().expect("the ordinary ROLE");
    let persons = &fixture.ordinary["PERSON"];
    let person = vo_of(persons.first().expect("the ordinary PERSON"));
    let refused = as_clinician(update(
        &svc,
        PartyKind::Role,
        vo_of(nurse),
        nurse,
        &body("privileged-role", &[(PERSON, person)]),
    ))
    .await;
    assert_policy_refusal(refused);
    assert_eq!(
        mark(&db, vo_of(nurse)).await,
        None,
        "a refused commit marks nothing"
    );

    let turned = turn_ordinary(&svc, &fixture).await;
    for (vo, at_commit) in &marks {
        assert_eq!(
            mark(&db, *vo).await,
            Some(at_commit.clone()),
            "{vo} after an ordinary version"
        );
    }
    for (kind, uid) in &turned {
        as_admin(delete(&svc, *kind, uid))
            .await
            .expect("an administrator deletes the former policy party");
    }
    for (vo, at_commit) in &marks {
        assert_eq!(
            mark(&db, *vo).await,
            Some(at_commit.clone()),
            "{vo} after its deletion"
        );
    }
}

/// `ferroehr party-binding refill` marks every party with a policy version
/// from the stored versions, the former policy parties included, and marks
/// no other (#5485).
#[tokio::test]
async fn the_refill_rebuilds_the_mark() {
    let (db, svc) = mapped().await;
    let fixture = fixture(&svc).await;
    turn_ordinary(&svc, &fixture).await;
    let pool = db.pool();
    sqlx::query("UPDATE party.vo_head SET policy_party_at = NULL")
        .execute(&pool)
        .await
        .expect("clear every mark");

    let party = ferroehr::db::domain_pool_from(&pool, ferroehr::db::domain::Domain::Party);
    ferroehr::party_binding::fill::refill(
        &party,
        &ferroehr::party_binding::BindingMapping::default().with_policy(),
        std::time::Duration::ZERO,
    )
    .await
    .expect("the refill runs");

    for (class, uids) in &fixture.policy {
        for uid in uids {
            assert!(mark(&db, vo_of(uid)).await.is_some(), "a policy {class}");
        }
    }
    for (class, uids) in &fixture.ordinary {
        for uid in uids {
            assert_eq!(mark(&db, vo_of(uid)).await, None, "an ordinary {class}");
        }
    }
}

/// Every scan of `plan` as `(node type, relation, index, conditions)`, the
/// conditions being its index condition and its filter.
fn scans(plan: &Value) -> Vec<(String, String, String, String)> {
    fn walk(node: &Value, out: &mut Vec<(String, String, String, String)>) {
        if let Some(relation) = node.get("Relation Name").and_then(Value::as_str) {
            let text = |key: &str| {
                node.get(key)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            };
            out.push((
                text("Node Type"),
                relation.to_owned(),
                text("Index Name"),
                format!("{} {}", text("Index Cond"), text("Filter")),
            ));
        }
        for child in node
            .get("Plans")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            walk(child, out);
        }
    }
    let mut out = Vec::new();
    walk(&plan[0]["Plan"], &mut out);
    out
}

/// Ordinary `ROLE`s the plan test seeds beside the fixture, so the head
/// relation is large enough that reading it whole costs the planner more
/// than probing it by key, as on any store in use.
const SEEDED_ROLES: usize = 120;

/// A caller who is no administrator gets the policy parties left out by one
/// predicate on the head row, which every root reaches through an index on
/// the head's key, never by a scan of the head relation; an administrator's
/// plan carries no such predicate (#5485).
#[tokio::test]
async fn the_mark_is_tested_on_the_head_row_reached_by_its_key() {
    use ferroehr::aql::sql::PolicyParties;

    let (db, svc) = mapped().await;
    let fixture = fixture(&svc).await;
    let persons = &fixture.ordinary["PERSON"];
    let person = vo_of(persons.first().expect("the ordinary PERSON"));
    as_clinician(async {
        for index in 0..SEEDED_ROLES {
            let mut role = plain(
                "ROLE",
                "openEHR-DEMOGRAPHIC-ROLE.role.v0",
                &format!("Nurse {index}"),
            );
            role["performer"] = json!({ "_type": "PARTY_REF", "namespace": "demographic",
                "type": "PERSON", "id": { "_type": "HIER_OBJECT_ID", "value": person.to_string() } });
            create(&svc, PartyKind::Role, &role)
                .await
                .expect("an ordinary ROLE");
        }
    })
    .await;
    sqlx::query("ANALYZE party.vo_head, party.version, party.node")
        .execute(&db.pool())
        .await
        .expect("analyze the party relations");

    // NOTE: a read of every ROLE reads every head, which a sequential scan does best.
    for (aql, keyed) in [
        (
            "SELECT x/uid/value FROM ROLE x ORDER BY x/uid/value LIMIT 2",
            true,
        ),
        ("SELECT x/uid/value FROM ROLE x LIMIT 2", true),
        ("SELECT COUNT(x/uid/value) FROM ROLE x", false),
        (
            "SELECT x/uid/value FROM VERSION v[ALL_VERSIONS] CONTAINS ROLE x",
            false,
        ),
        (
            "SELECT x/uid/value FROM VERSION v[ALL_VERSIONS] CONTAINS ROLE x LIMIT 2",
            true,
        ),
        (
            "SELECT r/uid/value, c/name/value FROM ROLE r CONTAINS CAPABILITY c",
            false,
        ),
        (
            "SELECT r/uid/value, c/name/value FROM ROLE r CONTAINS CAPABILITY c LIMIT 2",
            true,
        ),
    ] {
        let hidden = scans(&plan_of(&db, aql, PolicyParties::Hidden).await);
        let heads: Vec<_> = hidden.iter().filter(|scan| scan.1 == "vo_head").collect();
        assert!(!heads.is_empty(), "{aql}: no head row in {hidden:?}");
        for (node_type, _, index, conditions) in &heads {
            assert!(
                !keyed || (node_type.starts_with("Index") && !index.is_empty()),
                "{aql}: the head is reached by {node_type} in {hidden:?}"
            );
            assert!(
                conditions.contains("policy_party_at IS NULL"),
                "{aql}: the head scan tests no mark in {hidden:?}"
            );
        }
        let served = scans(&plan_of(&db, aql, PolicyParties::Served).await);
        assert!(
            served
                .iter()
                .all(|scan| !scan.3.contains("policy_party_at")),
            "{aql}: an administrator's plan tests the mark in {served:?}"
        );
    }
}

/// A `DV_CODED_TEXT` of FerroEHR's operation terminology.
fn operation(code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
        "defining_code": { "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" },
            "code_string": code } })
}

/// An `ELEMENT` named `name` holding `value`.
fn item(name: &str, value: &Value) -> Value {
    json!({ "_type": "ELEMENT", "archetype_node_id": "at0010",
        "name": { "_type": "DV_TEXT", "value": name }, "value": value })
}

/// A deployment models its own parties from its own archetypes, with no
/// FerroEHR template: a `ROLE` whose capability's credentials hold an
/// operation code, nested in a cluster, and a `PERSON` holding a
/// relationship whose details carry the acts-for code are policy parties,
/// written and read by an administrator only, through the Demographic API
/// and through AQL; the same `ROLE` with a code of another terminology is an
/// ordinary party (#5485).
#[tokio::test]
async fn a_deployments_own_archetypes_carrying_the_codes_are_policy_parties() {
    let (_db, svc) = mapped().await;
    let [(provider, _), _, (person, person_uid)] = referred(&svc).await;
    let migration = |code: Value| {
        let mut role = plain(
            "ROLE",
            "org.example::openEHR-DEMOGRAPHIC-ROLE.legacy_migration.v3",
            "Legacy migration",
        );
        role["performer"] = json!({ "_type": "PARTY_REF", "namespace": "demographic",
            "type": "PERSON", "id": { "_type": "HIER_OBJECT_ID", "value": person.to_string() } });
        role["capabilities"] = json!([{ "_type": "CAPABILITY", "archetype_node_id": "at0020",
            "name": { "_type": "DV_TEXT", "value": "Nightly import" },
            "credentials": { "_type": "ITEM_TREE", "archetype_node_id": "at0021",
                "name": { "_type": "DV_TEXT", "value": "Terms" },
                "items": [{ "_type": "CLUSTER", "archetype_node_id": "at0022",
                    "name": { "_type": "DV_TEXT", "value": "Scope" },
                    "items": [item("What", &code)] }] } }]);
        role
    };

    let mut granting = migration(operation("import_ehr"));
    granting["capabilities"][0]["credentials"]["items"][0]["items"]
        .as_array_mut()
        .expect("the scope's items")
        .push(json!({ "_type": "ELEMENT", "archetype_node_id": "at0011",
            "name": operation("subject_namespace"),
            "value": { "_type": "DV_TEXT", "value": "nl.legacy-his.patients" } }));
    assert_policy_refusal(as_clinician(create(&svc, PartyKind::Role, &granting)).await);
    let (_, grant_uid) = as_admin(create(&svc, PartyKind::Role, &granting))
        .await
        .expect("an administrator grants the deployment's own ROLE");

    let mut local = operation("import_ehr");
    local["defining_code"]["terminology_id"]["value"] = json!("local");
    let (_, ordinary_uid) = as_clinician(create(&svc, PartyKind::Role, &migration(local)))
        .await
        .expect("a code of another terminology grants nothing");

    let mut acting = plain(
        "PERSON",
        "openEHR-DEMOGRAPHIC-PERSON.person.v0",
        "J. Jansen",
    );
    acting["relationships"] = json!([{ "_type": "PARTY_RELATIONSHIP",
        "archetype_node_id": "org.example::openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.mandate.v2",
        "name": { "_type": "DV_TEXT", "value": "Mandate" },
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0030",
            "name": { "_type": "DV_TEXT", "value": "Mandate details" },
            "items": [item("Kind", &operation("acts_for"))] },
        "source": { "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": person.to_string() } },
        "target": { "_type": "PARTY_REF", "namespace": "demographic", "type": "ORGANISATION",
            "id": { "_type": "HIER_OBJECT_ID", "value": provider.to_string() } } }]);
    assert_policy_refusal(
        as_clinician(update(
            &svc,
            PartyKind::Person,
            person,
            &person_uid,
            &acting,
        ))
        .await,
    );
    let acting_uid = as_admin(update(
        &svc,
        PartyKind::Person,
        person,
        &person_uid,
        &acting,
    ))
    .await
    .expect("an administrator names the provider the person acts for");

    let roles: BTreeSet<String> = as_clinician(first_cells(&svc, "SELECT x/uid/value FROM ROLE x"))
        .await
        .into_iter()
        .collect();
    assert_eq!(
        roles,
        BTreeSet::from([ordinary_uid.clone()]),
        "a clinician's ROLEs"
    );
    let persons = as_clinician(first_cells(&svc, "SELECT x/uid/value FROM PERSON x")).await;
    assert!(
        !persons.contains(&acting_uid),
        "a clinician's PERSONs: {persons:?}"
    );
    for (kind, uid) in [
        (PartyKind::Role, &grant_uid),
        (PartyKind::Person, &acting_uid),
    ] {
        assert_policy_refusal(as_clinician(svc.party_get(kind, at(uid), None)).await);
        as_admin(svc.party_get(kind, at(uid), None))
            .await
            .expect("an administrator reads it");
    }
    as_clinician(svc.party_get(PartyKind::Role, at(&ordinary_uid), None))
        .await
        .expect("the ordinary ROLE stays anyone's to read");
    let admin: BTreeSet<String> = as_admin(first_cells(&svc, "SELECT x/uid/value FROM ROLE x"))
        .await
        .into_iter()
        .collect();
    assert_eq!(
        admin,
        BTreeSet::from([grant_uid, ordinary_uid]),
        "an administrator's ROLEs"
    );
}
