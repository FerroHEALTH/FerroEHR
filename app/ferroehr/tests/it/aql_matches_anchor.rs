// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A `matches` over a value list enters through the index its equality uses
//! and answers the rows it answered before (#6084).
//!
//! AQL QUERY master03 §matches: a value list holds when the leaf matches one
//! of its items, the items being in an "OR" relationship. The engine gives a
//! top-level `matches` over strings the anchor an equality with each member
//! would have, with the members as a list, when every member could anchor
//! its equality. The test itself is unchanged, so the anchor may only remove
//! work. The tests hold which lists get the anchor, that the anchored
//! statement answers what the unanchored one does, a `TERMINOLOGY('expand')`
//! operand, the operand forms the grammar refuses, and that the plan enters
//! through the index. No openEHR spec governs access paths: our own design.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use ferroehr::aql::error::{AqlError, AqlFeatureError};
use ferroehr::aql::ir::{ParamValue, Params};
use ferroehr::aql::lineage::ArchetypeLineage;
use ferroehr::aql::sql::SqlCtx;
use ferroehr::config::profile::SpecProfile;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;
use serde_json::{Value, json};
use sqlx::Executor as _;

use crate::aql_range_anchor::{
    FROM_OBSERVATION, date_time, event_holding, lowered, rows, rows_fetched, timed_composition,
};
use crate::fixtures::{create_ehr, uv};
use crate::storage_plans::{aql_sql_with, assert_lookup_through, explain_aql, render, scans};
use crate::typed_body::typed;

/// The path from the OBSERVATION `o` to the one ELEMENT of an event.
const ELEMENT: &str = "o/data[at0001]/events[at0002]/data[at0003]/items[at0004]";

/// How many value anchors `sql` carries: the test reads the leaf through the
/// typed columns, so only an anchor calls either function.
fn anchors(sql: &str) -> usize {
    sql.matches("node_search_text(").count() + sql.matches("node_code(").count()
}

/// The parameters the lists below bind.
fn params() -> Params {
    Params::default()
        .with("a", ParamValue::Str("alpha".to_owned()))
        .with("b", ParamValue::Str("beta".to_owned()))
        .with("n", ParamValue::Int(3))
}

/// A list of strings on a text leaf or on a code is anchored once, as a
/// membership test, against literals and parameters alike.
#[test]
fn a_value_list_of_strings_is_anchored() {
    for (leaf, list, function) in [
        (
            "value/value",
            "{'1950-01-01', '1950-01-02'}",
            "node_search_text(",
        ),
        ("value/value", "{$a, 'beta', $b}", "node_search_text("),
        ("value/id", "{'alpha', '19500101'}", "node_search_text("),
        ("value/units", "{'mm[Hg]', 'kg'}", "node_search_text("),
        (
            "value/defining_code/code_string",
            "{'at0008', '8', $a}",
            "node_code(",
        ),
        (
            "value/symbol/defining_code/code_string",
            "{'at0008', 'at0009'}",
            "node_code(",
        ),
    ] {
        let filter = format!("{ELEMENT}/{leaf} matches {list}");
        let sql = lowered(
            &format!("SELECT c/uid/value {FROM_OBSERVATION} WHERE {filter}"),
            &params(),
        );
        assert_eq!(anchors(&sql), 1, "{filter}: one anchor\n{sql}");
        assert!(
            sql.contains(&format!("{function}\"va")) && sql.contains(r#""data") IN ("#),
            "{filter}: the anchor reads the leaf's ELEMENT rows by membership\n{sql}"
        );
    }
}

/// On the `value` member a text in plain decimal form also reads the number
/// of an ordinal or a scale (`ext.node_number`), since that value's search
/// text is its symbol; a text in any other form equals no number and reads
/// the search text alone (#6090).
#[test]
fn a_number_shaped_member_also_reads_the_number_of_an_ordinal_or_a_scale() {
    let value = format!("{ELEMENT}/value/value");
    for (filter, numbers) in [
        (format!("{value} matches {{'1950-01-01', '19500101'}}"), 1),
        (format!("{value} = '19500101'"), 1),
        (format!("{value} matches {{'3.5', '-2', 'alpha'}}"), 1),
        (
            format!("{value} matches {{'1950-01-01', '1e5', '+3', '.5'}}"),
            0,
        ),
        (
            format!("{ELEMENT}/value/id matches {{'alpha', '19500101'}}"),
            0,
        ),
        (format!("{ELEMENT}/value/symbol/value = '3'"), 0),
    ] {
        let sql = lowered(
            &format!("SELECT c/uid/value {FROM_OBSERVATION} WHERE {filter}"),
            &params(),
        );
        assert_eq!(anchors(&sql), 1, "{filter}: one anchor\n{sql}");
        assert_eq!(
            sql.matches("node_number(").count(),
            numbers,
            "{filter}: the anchor reads the number only for a plain decimal member\n{sql}"
        );
    }
}

/// A list with a member no index holds, a conjunct under `OR` or `NOT`, and a
/// leaf no index reads imply no anchor.
#[test]
fn only_a_list_every_member_of_which_anchors_is_anchored() {
    let value = format!("{ELEMENT}/value/value");
    for filter in [
        format!("{value} matches {{'alpha', 3}}"),
        format!("{value} matches {{'alpha', $n}}"),
        format!("{value} matches {{'alpha'}} OR {value} matches {{'beta'}}"),
        format!("NOT {value} matches {{'alpha', 'beta'}}"),
        format!("{ELEMENT}/value matches {{'alpha', 'beta'}}"),
        format!("{ELEMENT}/value/defining_code/terminology_id/value matches {{'local', 'snomed'}}"),
    ] {
        let sql = lowered(
            &format!("SELECT c/uid/value {FROM_OBSERVATION} WHERE {filter}"),
            &params(),
        );
        assert_eq!(anchors(&sql), 0, "{filter}: no anchor\n{sql}");
    }
}

/// A `CODE_PHRASE` of the terminology `local`.
fn code_phrase(code: &str) -> Value {
    json!({
        "_type": "CODE_PHRASE",
        "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" },
        "code_string": code
    })
}

/// A `DV_CODED_TEXT` of `code` reading `value`.
fn coded_text(value: &str, code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": value, "defining_code": code_phrase(code) })
}

/// The values the membership store commits: the shapes the typed columns
/// hold, the same with a member that keeps them in the fragment, and values
/// whose `value` is a number or a Boolean.
fn listed_values() -> Vec<Value> {
    let mut values = Vec::new();
    for text in ["1950-01-01", "19500101", "alpha", "beta", "Alpha", "alpha "] {
        values.push(json!({ "_type": "DV_TEXT", "value": text }));
        values.push(json!({ "_type": "DV_TEXT", "value": text, "formatting": "plain" }));
        values.push(coded_text(text, "at0008"));
        values.push(json!({ "_type": "DV_IDENTIFIER", "id": text }));
        values.push(json!({
            "_type": "DV_ORDINAL", "value": 3, "symbol": coded_text(text, "at0009")
        }));
    }
    for date in ["1950-01-01", "19500101", "1950-01-02"] {
        values.push(json!({ "_type": "DV_DATE", "value": date }));
    }
    values.push(json!({
        "_type": "DV_ORDINAL", "value": 19_500_101, "symbol": coded_text("ordinal", "at0010")
    }));
    values.push(json!({ "_type": "DV_SCALE", "value": 3.5, "symbol": coded_text("beta", "8") }));
    values.push(json!({ "_type": "DV_QUANTITY", "magnitude": 8.0, "units": "mm[Hg]" }));
    values.push(json!({ "_type": "DV_BOOLEAN", "value": true }));
    values
}

/// Every list answers the same rows with its anchor and without it, over the
/// typed columns and the fragment alike, flat and streamed.
#[tokio::test]
async fn an_anchored_value_list_answers_the_rows_the_unanchored_one_does() {
    let (_db, svc) = crate::fixtures::service().await;
    let values = listed_values();
    for (index, value) in values.iter().enumerate() {
        let ehr = create_ehr(&svc).await;
        let other = &values[(index + 7) % values.len()];
        let body = timed_composition(
            &format!("value {index}"),
            &date_time("2026-03-01T10:00:00Z", false),
            &[
                event_holding("2026-03-01T10:00:00Z", false, false, value),
                event_holding("2026-03-01T11:00:00Z", false, true, other),
            ],
        );
        svc.create_composition(ehr, uv(&body, "249", None))
            .await
            .unwrap_or_else(|e| panic!("commit {value}: {e:?}"));
    }
    let bound = BTreeMap::from([
        ("a".to_owned(), json!("alpha")),
        ("b".to_owned(), json!("beta")),
    ]);
    let mut partial = 0_usize;
    for (leaf, lists) in [
        (
            "value/value",
            vec![
                "{'1950-01-01', '1950-01-02'}",
                "{'1950-01-01', '19500101'}",
                "{$a, $b}",
                "{'alpha', 'Alpha', 'absent'}",
                "{'true', 'beta'}",
                "{'3', 'alpha'}",
                "{'3.5', 'absent'}",
                "{'3.50', '03', '-0', '19500101.0'}",
                "{'1e5', '19500101'}",
            ],
        ),
        ("value/id", vec!["{'alpha', '19500101'}", "{$a, 'alpha '}"]),
        ("value/units", vec!["{'mm[Hg]', 'alpha'}"]),
        ("value/symbol/value", vec!["{'alpha', 'beta'}"]),
        (
            "value/defining_code/code_string",
            vec!["{'at0008', 'absent'}", "{$a, 'at0010'}"],
        ),
        (
            "value/symbol/defining_code/code_string",
            vec!["{'at0009', '8'}", "{'at0010', 'absent'}"],
        ),
    ] {
        let leaf = format!("{ELEMENT}/{leaf}");
        let all = rows(
            &svc,
            &format!("SELECT c/uid/value, {leaf} {FROM_OBSERVATION}"),
            BTreeMap::new(),
        )
        .await
        .len();
        for list in lists {
            let test = format!("{leaf} matches {list}");
            let anchored = format!("SELECT c/uid/value, {leaf} {FROM_OBSERVATION} WHERE {test}");
            let plain =
                format!("SELECT c/uid/value, {leaf} {FROM_OBSERVATION} WHERE {test} OR {test}");
            assert_eq!(anchors(&lowered(&plain, &params())), 0, "{plain}");
            let with = rows(&svc, &anchored, bound.clone()).await;
            let without = rows(&svc, &plain, bound.clone()).await;
            assert_eq!(with, without, "{anchored}: the anchor changes no row");
            let streamed = rows_fetched(&svc, &anchored, bound.clone(), Some(10_000)).await;
            assert_eq!(streamed, without, "{anchored}: nor does it when it streams");
            partial += usize::from(!with.is_empty() && with.len() < all);
        }
    }
    assert!(
        partial >= 10,
        "the lists split the store: {partial} partial answers"
    );
}

/// A canned terminology: every expansion is the same codes.
struct Expansion(Vec<String>);

#[async_trait]
impl ferroehr::aql::terminology::TerminologyExpander for Expansion {
    async fn expand(&self, _api: &str, _uri: &str) -> Result<Vec<String>, AqlError> {
        Ok(self.0.clone())
    }

    async fn boolean_operation(&self, _op: &str, _api: &str, _uri: &str) -> Result<bool, AqlError> {
        Ok(false)
    }

    async fn expand_uri(&self, _uri: &str) -> Result<Vec<String>, AqlError> {
        Ok(self.0.clone())
    }
}

/// The SQL of a parsed statement over the population.
fn population_sql(ast: &openehr_query::ast::SelectQuery) -> String {
    let params = Params::default();
    let ir = ferroehr::aql::plan(ast, &params, SpecProfile::default()).expect("the AQL plans");
    let ctx = SqlCtx {
        system_id: "matches.anchor.test".to_owned(),
        ehr_ids: Vec::new(),
        ehr_scope_kind: ferroehr::aql::sql::EhrScopeKind::Named,
        ehr_access: ferroehr::access_eval::condition::AccessCondition::default(),
        limit: None,
        offset: None,
        archetype_lineage: Arc::new(ArchetypeLineage::default()),
        policy_parties: ferroehr::aql::sql::PolicyParties::Served,
        access_settings: ferroehr::aql::sql::EhrAccessReaders::Every,
        own_records: None,
    };
    ferroehr::aql::sql::build(&ir, &params, &ctx)
        .expect("the AQL lowers to SQL")
        .sql
        .to_string()
}

/// A `TERMINOLOGY('expand')` operand, alone or in a list, matches the codes
/// it expands to (QUERY master03 §TERMINOLOGY), through the code anchor; an
/// empty expansion matches nothing and takes no anchor.
#[tokio::test]
async fn an_expanded_terminology_operand_is_anchored_as_its_codes() {
    let code = format!("{ELEMENT}/value/defining_code/code_string");
    for (operand, codes, expected) in [
        (
            "TERMINOLOGY('expand', 'openehr', 'g')",
            vec!["at0008", "at0009"],
            1,
        ),
        (
            "{'at0007', TERMINOLOGY('expand', 'openehr', 'g')}",
            vec!["at0008"],
            1,
        ),
        ("TERMINOLOGY('expand', 'openehr', 'g')", vec![], 0),
    ] {
        let aql = format!("SELECT c/uid/value {FROM_OBSERVATION} WHERE {code} matches {operand}");
        let mut ast = openehr_query::parser::parse_str(&aql).expect("the AQL parses");
        let expander = Expansion(codes.iter().map(|c| (*c).to_owned()).collect());
        ferroehr::aql::terminology::expand_matches(&mut ast, &expander)
            .await
            .expect("the operand expands");
        let sql = population_sql(&ast);
        assert_eq!(anchors(&sql), expected, "{operand} over {codes:?}\n{sql}");
    }
}

/// The interval and the regular-expression forms of the cADL constraint the
/// AQL grammar leaves out of a `matches` operand are refused, and a regular
/// expression in a node predicate is a typed reject.
#[test]
fn an_interval_or_a_regex_operand_is_refused() {
    let magnitude = format!("{ELEMENT}/value/magnitude");
    for operand in ["{|>=5.0|}", "{|1.0..5.0|}", "{/alpha.*/}"] {
        let aql =
            format!("SELECT c/uid/value {FROM_OBSERVATION} WHERE {magnitude} matches {operand}");
        assert!(
            openehr_query::parser::parse_str(&aql).is_err(),
            "{operand} is no matchesOperand (QUERY AqlParser.g4)"
        );
    }
    let aql = "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c \
               CONTAINS OBSERVATION o[name/value matches {/Timed.*/}]";
    let ast = openehr_query::parser::parse_str(aql).expect("a node predicate regex parses");
    let refused = ferroehr::aql::plan(&ast, &Params::default(), SpecProfile::default());
    assert!(
        matches!(
            refused,
            Err(AqlError::Feature(AqlFeatureError::RegexNodePredicate))
        ),
        "a regex node predicate is a typed reject: {refused:?}"
    );
}

/// A PERSON whose details hold a date of birth, the PERSON of #6084.
pub(crate) fn person_born(name: &str, birth: &str) -> Value {
    json!({
        "_type": "PERSON",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                              "value": "openEHR-DEMOGRAPHIC-PERSON.person.v1" },
            "rm_version": "1.1.0" },
        "name": { "_type": "DV_TEXT", "value": name },
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "archetype_node_id": "at0002",
            "name": { "_type": "DV_TEXT", "value": "legal name" },
            "details": {
                "_type": "ITEM_TREE",
                "archetype_node_id": "at0003",
                "name": { "_type": "DV_TEXT", "value": "structure" },
                "items": [{
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0004",
                    "name": { "_type": "DV_TEXT", "value": "family" },
                    "value": { "_type": "DV_TEXT", "value": name }
                }]
            }
        }],
        "details": {
            "_type": "ITEM_TREE",
            "archetype_node_id": "at0000",
            "name": { "_type": "DV_TEXT", "value": "details" },
            "items": [{
                "_type": "CLUSTER",
                "archetype_node_id": "at0001",
                "name": { "_type": "DV_TEXT", "value": "birth" },
                "items": [{
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0010",
                    "name": { "_type": "DV_TEXT", "value": "date of birth" },
                    "value": { "_type": "DV_DATE", "value": birth }
                }]
            }]
        }
    })
}

/// A lookup of a person by a list of dates of birth reads the one person
/// through `idx_node_hot_value_search`, on the flat shape and the streaming
/// one, as the equality does, and reads the store without its anchor.
#[tokio::test]
async fn a_lookup_by_a_list_of_dates_of_birth_reads_the_one_person() {
    const PERSONS: u32 = 200;
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let pools = ferroehr::db::domain::DomainPools::from_shared(&pool);
    let svc = FerroEhrService::new(&pools);
    for index in 0..PERSONS {
        let body = person_born(
            &format!("person {index}"),
            &format!("{}-01-01", 1800 + index),
        );
        svc.party_create(PartyKind::Person, WriteRequest::new(typed(&body)))
            .await
            .expect("create a person");
    }
    pools
        .party
        .execute("ANALYZE")
        .await
        .expect("give the planner statistics over the seeded parties");

    let birth = "p/details/items[at0001]/items[at0010]/value/value";
    let found = format!("{}-01-01", 1800 + PERSONS.div_euclid(2));
    let lookup = |test: &str| format!("SELECT p/uid/value FROM PERSON p WHERE {test}");
    let list = format!("{birth} matches {{'{found}', '1700-01-01', $absent}}");
    let params = Params::default().with("absent", ParamValue::Str("1700-01-02".to_owned()));
    for (name, limit) in [("the lookup", None), ("the lookup with a limit", Some(10))] {
        let prepared = aql_sql_with(&lookup(&list), &params, Vec::new(), limit);
        let plan = explain_aql(&pools.party, prepared).await;
        assert_lookup_through(name, &plan, PERSONS, "idx_node_hot_value_search");
    }
    let unanchored = lookup(&format!("{list} OR {list}"));
    let plan = explain_aql(
        &pools.party,
        aql_sql_with(&unanchored, &params, Vec::new(), None),
    )
    .await;
    let buffers = plan
        .pointer("/0/Plan/Shared Hit Blocks")
        .and_then(Value::as_f64)
        .expect("the root plan node reports its buffers");
    assert!(
        buffers > f64::from(PERSONS),
        "the same lookup without its anchor reads the store: {buffers} buffers"
    );
}

/// The lookup of Cadasto/FerroHEALTH-backlog#51 by its exact list,
/// `{'19500101', '1950-01-01'}`, reads the one person whose date of birth is
/// an ordinal of that number through `idx_node_hot_value_number` and the text
/// members through `idx_node_hot_value_search`, and answers the rows the
/// same lookup answers without its anchor (#6090).
#[tokio::test]
async fn the_reported_list_reads_an_ordinal_through_the_number_index() {
    const PERSONS: u32 = 100;
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let pools = ferroehr::db::domain::DomainPools::from_shared(&pool);
    let svc = FerroEhrService::new(&pools);
    for index in 0..PERSONS {
        let body = person_born(
            &format!("person {index}"),
            &format!("{}-01-01", 1800 + index),
        );
        svc.party_create(PartyKind::Person, WriteRequest::new(typed(&body)))
            .await
            .expect("create a person");
    }
    let mut ordinal = person_born("ordinal", "1800-01-01");
    ordinal["details"]["items"][0]["items"][0]["value"] = json!({
        "_type": "DV_ORDINAL", "value": 19_500_101, "symbol": coded_text("born", "at0011")
    });
    svc.party_create(PartyKind::Person, WriteRequest::new(typed(&ordinal)))
        .await
        .expect("create the person with an ordinal date of birth");
    pools
        .party
        .execute("ANALYZE")
        .await
        .expect("give the planner statistics over the seeded parties");

    let birth = "p/details/items[at0001]/items[at0010]/value/value";
    let list = format!("{birth} matches {{'19500101', '1950-01-01'}}");
    let lookup = |test: &str| format!("SELECT p/name/value FROM PERSON p WHERE {test}");
    let params = Params::default();
    let plan = explain_aql(
        &pools.party,
        aql_sql_with(&lookup(&list), &params, Vec::new(), None),
    )
    .await;
    assert_lookup_through(
        "the reported lookup",
        &plan,
        PERSONS,
        "idx_node_hot_value_search",
    );
    // The two indexes meet under one BitmapOr, whose second scan
    // `scans` reports under no heap node of its own.
    let indexes = plan.to_string();
    assert!(
        indexes.contains(r#""Index Name":"idx_node_hot_value_number""#),
        "the number-shaped member enters through the number index:\n{}",
        render(&scans(&plan))
    );
    let anchored = rows(&svc, &lookup(&list), BTreeMap::new()).await;
    let unanchored = rows(&svc, &lookup(&format!("{list} OR {list}")), BTreeMap::new()).await;
    assert_eq!(anchored.len(), 1, "one person: {anchored:?}");
    assert!(anchored[0].contains("ordinal"), "{anchored:?}");
    assert_eq!(anchored, unanchored, "the anchor changes no row");
}
