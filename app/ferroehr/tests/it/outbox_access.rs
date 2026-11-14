// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The outbox feed publishes only what the access evaluator lets an access
//! service of the system kind read, against a real `PostgreSQL` 18 (#5587).
//!
//! An event names the EHR and the contribution it announces, so an event a
//! directive denies to such a caller is never served, and the read goes on
//! past it so a page is never cut short by what it leaves out. No openEHR spec
//! governs eventing: our own design/extension.

use ferroehr::extensions::outbox::OutboxDomain;
use ferroehr::ids::EhrId;
use ferroehr::party_binding::policy_mark::TERMINOLOGY;
use ferroehr::service::FerroEhrService;
use serde_json::{Value, json};

use crate::fixtures::{composition_by, uv};

/// The reader the feed is granted to.
const READER: &str = "ferrobridge";

/// A term of the access control groups.
fn term(code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": TERMINOLOGY },
                "code_string": code } })
}

/// A deny directive of `kind` with `extra` members.
fn deny(kind: &str, extra: &Value) -> Value {
    let mut directive = json!({
        "_type": "ACCESS_DIRECTIVE",
        "name": { "_type": "DV_TEXT", "value": "Directive" },
        "archetype_node_id": "at0001",
        "kind": term(kind),
        "effect": term("deny"),
        "emergency_override": false
    });
    if let (Some(directive), Some(extra)) = (directive.as_object_mut(), extra.as_object()) {
        directive.extend(extra.clone());
    }
    directive
}

/// Open settings holding `directives`.
fn settings(directives: &[Value]) -> Value {
    json!({ "_type": "ACCESS_DIRECTIVE_SETTINGS", "default_access": term("open"),
            "directives": directives })
}

/// A data scope naming the versioned object `vo_id`.
fn object_scope(vo_id: uuid::Uuid) -> Value {
    json!({ "data": [{ "objects": [{ "_type": "OBJECT_REF", "namespace": "local",
        "type": "VERSIONED_COMPOSITION",
        "id": { "_type": "HIER_OBJECT_ID", "value": vo_id.to_string() } }] }] })
}

/// Commits `settings` as the EHR's next `EHR_ACCESS` version.
async fn commit_settings(svc: &FerroEhrService, ehr: EhrId, settings: Value) {
    let versions = svc
        .ehr_access_versions(ehr)
        .await
        .expect("the EHR_ACCESS versions");
    let (current, _) = versions.first().expect("a current EHR_ACCESS version");
    svc.commit_ehr_access_settings(ehr, current, settings)
        .await
        .expect("the settings commit");
}

/// The `contribution_id` of every event `svc` serves after the reader's
/// cursor, up to `limit`.
async fn served(svc: &FerroEhrService, limit: i64) -> Vec<(i64, Value)> {
    svc.outbox_events(OutboxDomain::Clinical, READER, limit)
        .await
        .expect("read the feed")
        .into_iter()
        .map(|event| {
            let contribution = event.envelope["contribution_id"].clone();
            (event.seq, contribution)
        })
        .collect()
}

/// The contribution of the outbox row announcing the composition `vo_id`.
async fn contribution_of(pool: &sqlx::PgPool, vo_id: uuid::Uuid) -> Value {
    let contribution: uuid::Uuid =
        sqlx::query_scalar("SELECT contribution_id FROM clinical.version WHERE vo_id = $1 LIMIT 1")
            .bind(vo_id)
            .fetch_one(pool)
            .await
            .expect("the composition's contribution");
    json!(contribution)
}

/// A restriction naming one composition withholds the event of its commit
/// from the feed and no other; the read goes on past it, so a page of one
/// serves the next event; and a deny that binds only professionals withholds
/// nothing, since the feed is judged as a system caller.
#[tokio::test]
async fn an_event_a_directive_denies_to_an_access_service_is_not_published() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = crate::policy_fixtures::granting(FerroEhrService::new(
        &ferroehr::db::domain::DomainPools::from_shared(&pool),
    ))
    .await;
    crate::policy_fixtures::grant_reader(&svc, READER).await;

    let ehr = crate::fixtures::create_ehr(&svc).await;
    let hidden = svc
        .create_composition(
            ehr,
            uv(&composition_by("hidden", "a clinician"), "249", None),
        )
        .await
        .expect("the composition the restriction names");
    let shown = svc
        .create_composition(
            ehr,
            uv(&composition_by("shown", "a clinician"), "249", None),
        )
        .await
        .expect("a composition no directive names");
    let hidden_contribution = contribution_of(&pool, hidden.vo_id.0).await;
    let shown_contribution = contribution_of(&pool, shown.vo_id.0).await;
    let before = served(&svc, 100).await;
    assert_eq!(before.len(), 3, "the EHR creation and both compositions");

    commit_settings(
        &svc,
        ehr,
        settings(&[deny("restriction", &object_scope(hidden.vo_id.0))]),
    )
    .await;
    let after = served(&svc, 100).await;
    let contributions: Vec<&Value> = after.iter().map(|(_, c)| c).collect();
    assert!(
        !contributions.contains(&&hidden_contribution),
        "the restricted composition's event is withheld: {after:?}"
    );
    assert!(
        contributions.contains(&&shown_contribution),
        "the other composition's event is served: {after:?}"
    );
    assert_eq!(
        after.len(),
        3,
        "the EHR creation, the other composition and the settings commit: {after:?}"
    );

    let (creation, _) = before.first().expect("the EHR creation");
    svc.outbox_acknowledge(OutboxDomain::Clinical, READER, *creation)
        .await
        .expect("acknowledge the EHR creation");
    let page = served(&svc, 1).await;
    assert_eq!(
        page.iter().map(|(_, c)| c).collect::<Vec<_>>(),
        [&shown_contribution],
        "a page of one goes on past the withheld event"
    );

    let professionals = json!({ "applies_to": [{ "caller_kind": term("professional") }] });
    commit_settings(&svc, ehr, settings(&[deny("consent", &professionals)])).await;
    let unbound = served(&svc, 100).await;
    assert!(
        unbound.iter().any(|(_, c)| *c == hidden_contribution),
        "a deny that binds only professionals withholds nothing: {unbound:?}"
    );
}
