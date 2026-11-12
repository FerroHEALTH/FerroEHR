// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! AQL over the demographic store leaves the policy parties out for a caller
//! who is no administrator (#5450), over a real PostgreSQL 18: a party whose
//! root is of a policy archetype or a specialisation of one, and a party
//! holding an acts-for relationship inline, contribute no row, no count and
//! no page, through the root and through `CONTAINS`. An administrator's query
//! returns them, and every other party stays anyone's to query. QUERY master03
//! §FROM gives semantics to the clinical repository only and no openEHR spec
//! governs who reads a party: our own design.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::policy_parties::{
    AGENT, PERSON, PROVIDER, as_admin, as_clinician, body, create, mapped, plain, referred,
    system_client_under, update,
};
use ferroehr::error::ServiceError;
use ferroehr::ids::VoId;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::query::request::AqlQueryRequest;

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

/// The policy parties and the ordinary ones of each served root class.
struct Fixture {
    /// The ordinary `uid/value`s, by root class.
    ordinary: [(&'static str, BTreeSet<String>); 3],
    /// The policy `uid/value`s, by root class.
    policy: [(&'static str, BTreeSet<String>); 3],
}

/// Commits, beside the ordinary parties, a system client `ROLE`, a
/// privileged `ROLE`, a `ROLE` of a specialised system client archetype, an
/// `ORGANISATION` and a `PERSON` holding an acts-for relationship inline,
/// each written by an administrator.
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

    let (agent, _) = as_admin(create(svc, PartyKind::Agent, &body("agent", &[])))
        .await
        .expect("an administrator creates the AGENT");
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
        ordinary: [
            ("ROLE", set(&[&nurse_uid])),
            ("PERSON", set(&[&person_uid])),
            ("ORGANISATION", set(&[&provider_uid])),
        ],
        policy: [
            ("ROLE", set(&[&system_client, &privileged, &hospital])),
            ("PERSON", set(&[&acting])),
            ("ORGANISATION", set(&[&network])),
        ],
    }
}

/// Per served root class, a caller who is no administrator reads every
/// ordinary party and no policy party, whole, counted and paged; an
/// administrator reads both.
#[tokio::test]
async fn a_demographic_query_leaves_the_policy_parties_out_for_a_non_administrator() {
    let (_db, svc) = mapped().await;
    let fixture = fixture(&svc).await;
    for ((class, ordinary), (_, policy)) in fixture.ordinary.iter().zip(&fixture.policy) {
        let seen = as_clinician(every_reading(&svc, class)).await;
        assert_eq!(&seen, ordinary, "a clinician's {class} query");
        let seen = as_admin(every_reading(&svc, class)).await;
        let both: BTreeSet<String> = ordinary.union(policy).cloned().collect();
        assert_eq!(seen, both, "an administrator's {class} query");
    }
}

/// A node reached through `CONTAINS` belongs to its root's version, so a
/// policy `ROLE`'s `CAPABILITY` is left out with it, and so is a policy
/// party read under `VERSION`.
#[tokio::test]
async fn a_contained_node_of_a_policy_party_is_left_out() {
    let (_db, svc) = mapped().await;
    let fixture = fixture(&svc).await;
    let [(_, roles), ..] = &fixture.policy;

    let contained = "SELECT r/uid/value, c/name/value FROM ROLE r CONTAINS CAPABILITY c";
    let clinician: Vec<String> = as_clinician(first_cells(&svc, contained)).await;
    assert_eq!(
        clinician,
        Vec::<String>::new(),
        "a clinician's capabilities"
    );
    let admin: BTreeSet<String> = as_admin(first_cells(&svc, contained))
        .await
        .into_iter()
        .collect();
    assert_eq!(&admin, roles, "an administrator's capabilities");

    let versioned = "SELECT r/uid/value FROM VERSION v CONTAINS ROLE r";
    let clinician: BTreeSet<String> = as_clinician(first_cells(&svc, versioned))
        .await
        .into_iter()
        .collect();
    let [(_, ordinary), ..] = &fixture.ordinary;
    assert_eq!(&clinician, ordinary, "a clinician's ROLE versions");
}

/// No caller queries `FROM AGENT`: the root is not served, so a system's
/// `AGENT` is never an AQL row.
#[tokio::test]
async fn the_agent_root_is_refused() {
    let (_db, svc) = mapped().await;
    as_admin(create(&svc, PartyKind::Agent, &body("agent", &[])))
        .await
        .expect("an administrator creates the AGENT");
    for refused in [
        as_clinician(rows(&svc, "SELECT a/uid/value FROM AGENT a")).await,
        as_admin(rows(&svc, "SELECT a/uid/value FROM AGENT a")).await,
        as_clinician(rows(&svc, "SELECT p/uid/value FROM PARTY p")).await,
    ] {
        let refused = refused.expect_err("AGENT is no served root");
        assert!(
            refused
                .to_string()
                .contains("demographic class this server does not query"),
            "{refused}"
        );
    }
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
/// relation the query does not read anyway: it tests the decomposed
/// archetype columns and the inline relationships of the root row.
#[tokio::test]
async fn the_filter_reads_no_relation_the_query_does_not_read() {
    use ferroehr::aql::sql::PolicyParties;

    let (db, svc) = mapped().await;
    fixture(&svc).await;
    for aql in [
        "SELECT x/uid/value FROM ROLE x ORDER BY x/uid/value LIMIT 2",
        "SELECT COUNT(x/uid/value) FROM PERSON x",
        "SELECT r/uid/value, c/name/value FROM ROLE r CONTAINS CAPABILITY c",
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
