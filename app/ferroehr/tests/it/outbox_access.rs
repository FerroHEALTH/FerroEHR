// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The outbox feed publishes only what the access evaluator lets the feed's
//! reader read, against a real `PostgreSQL` 18 (#5587).
//!
//! An event names the EHR and the contribution it announces, so the feed is
//! judged as the request that reads it, like any read of that EHR's data: a
//! directive that binds the reader and denies the data withholds the event,
//! and so does an access policy the reader does not meet. The read goes on
//! past what it leaves out, so a page is never cut short by it. No openEHR spec
//! governs eventing: our own design/extension.

use std::sync::Arc;

use ferroehr::access_eval::condition::{Asking, InForce, RequestAccess, with_request_access};
use ferroehr::access_eval::procedure::{Caller, Emergency};
use ferroehr::access_eval::scope::CategoryIndex;
use ferroehr::extensions::outbox::OutboxDomain;
use ferroehr::ids::EhrId;
use ferroehr::party_binding::grants::caller::CallerFacts;
use ferroehr::party_binding::policy_mark::TERMINOLOGY;
use ferroehr::service::FerroEhrService;
use ferroehr::service::ehr::access_rule::SettingsCaller;
use openehr_rm::draft::access_control::access_control::access_policy::AccessPolicy;
use openehr_rm::draft::access_control::terminology::{
    AccessDefault, CallerKind, ForeignCode, OperationClass,
};
use serde_json::{Value, json};

use crate::fixtures::{composition_by, uv};
use crate::policy_fixtures::{capability, granted, scope};

/// The reader the feed is granted to.
const READER: &str = "ferrobridge";
/// The controller the test access policy names.
const CONTROLLER: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f84";
/// The purpose of use the permitted reader declares.
const PURPOSE: &str = "HOPERAT";

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

/// An actor scope naming the `AGENT` `agent`.
fn naming_agent(agent: &str) -> Value {
    json!({ "applies_to": [{ "party": { "_type": "PARTY_REF", "namespace": "local",
        "type": "AGENT", "id": { "_type": "HIER_OBJECT_ID", "value": agent } } }] })
}

/// An `ACCESS_POLICY` that requires a declared purpose.
fn purpose_required() -> AccessPolicy {
    let policy = json!({
        "_type": "ACCESS_POLICY",
        "name": { "_type": "DV_TEXT", "value": "Policy" },
        "archetype_node_id": "openEHR-EHR-ACCESS_POLICY.controller.v1",
        "controller": { "_type": "PARTY_REF", "namespace": "local", "type": "ORGANISATION",
                        "id": { "_type": "HIER_OBJECT_ID", "value": CONTROLLER } },
        "care_relationship_required": false,
        "purpose_required": true
    });
    openehr_its::json::from_canonical_value(&policy).expect("the test policy reads")
}

/// The feed's reader as a request judges it.
struct Reader {
    kind: CallerKind,
    party: Option<String>,
    facts: CallerFacts,
    purpose: Option<&'static str>,
    policy: Option<AccessPolicy>,
}

impl Reader {
    /// A reader of `kind` bound to `party`, with no facts, purpose or policy.
    fn new(kind: CallerKind, party: Option<String>) -> Self {
        Self {
            kind,
            party,
            facts: CallerFacts::default(),
            purpose: None,
            policy: None,
        }
    }

    /// The evaluator of the request this reader reads the feed in.
    fn access(&self) -> Arc<RequestAccess> {
        let caller = Caller {
            kind: self.kind,
            party: self.party.clone(),
            acts_for: Vec::new(),
            roles: Vec::new(),
            facts: self.facts.clone(),
            administrator: false,
            settings: SettingsCaller::new(false, false, self.party.clone()),
        };
        let asking = Asking {
            operation: OperationClass::Read,
            purpose: self.purpose.map(str::to_owned),
            emergency: Emergency::default(),
        };
        let in_force = InForce {
            policy: self.policy.clone().map(Arc::new),
            categories: CategoryIndex::default(),
            server_default: AccessDefault::Open,
            now: jiff::Timestamp::now(),
            grants: None,
        };
        Arc::new(RequestAccess::new(caller, asking, in_force))
    }
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

/// The `seq` and `contribution_id` of every event `reader` is served after
/// the cursor, up to `limit`.
async fn served(svc: &FerroEhrService, reader: &Reader, limit: i64) -> Vec<(i64, Value)> {
    Box::pin(with_request_access(
        reader.access(),
        svc.outbox_events(OutboxDomain::Clinical, READER, limit),
    ))
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

/// A service whose change feed is granted to [`READER`] through a client
/// `AGENT`, whose id it returns.
async fn granted_feed(pool: &sqlx::PgPool) -> (FerroEhrService, String) {
    let svc = crate::policy_fixtures::granting(FerroEhrService::new(
        &ferroehr::db::domain::DomainPools::from_shared(pool),
    ))
    .await;
    let (agent, _) = granted(
        &svc,
        "feed-reader",
        &[capability("read_change_feed", &[scope("reader", READER)])],
    )
    .await;
    (svc, agent.0.to_string())
}

/// A restriction naming one composition withholds the event of its commit
/// from a system reader and no other; the read goes on past it, so a page of
/// one serves the next event; and a deny that binds only professionals
/// withholds nothing from it.
#[tokio::test]
async fn an_event_a_directive_denies_to_the_reader_is_not_published() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let (svc, _) = granted_feed(&pool).await;
    let system = Reader::new(CallerKind::System, None);

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
    let before = served(&svc, &system, 100).await;
    assert_eq!(before.len(), 3, "the EHR creation and both compositions");

    commit_settings(
        &svc,
        ehr,
        settings(&[deny("restriction", &object_scope(hidden.vo_id.0))]),
    )
    .await;
    let after = served(&svc, &system, 100).await;
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
    let page = served(&svc, &system, 1).await;
    assert_eq!(
        page.iter().map(|(_, c)| c).collect::<Vec<_>>(),
        [&shown_contribution],
        "a page of one goes on past the withheld event"
    );

    let professionals = json!({ "applies_to": [{ "caller_kind": term("professional") }] });
    commit_settings(&svc, ehr, settings(&[deny("consent", &professionals)])).await;
    let unbound = served(&svc, &system, 100).await;
    assert!(
        unbound.iter().any(|(_, c)| *c == hidden_contribution),
        "a deny that binds only professionals withholds nothing: {unbound:?}"
    );
}

/// A consent deny naming the reader's own `AGENT` withholds every event of
/// the EHR from that reader, and from no reader bound to another party.
#[tokio::test]
async fn a_reader_whose_binding_a_directive_denies_gets_no_event() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let (svc, agent) = granted_feed(&pool).await;
    let ehr = crate::fixtures::create_ehr(&svc).await;
    svc.create_composition(ehr, uv(&composition_by("note", "a clinician"), "249", None))
        .await
        .expect("a composition");
    commit_settings(
        &svc,
        ehr,
        settings(&[deny("consent", &naming_agent(&agent))]),
    )
    .await;

    let denied = Reader::new(CallerKind::AccessService, Some(agent));
    assert_eq!(
        served(&svc, &denied, 100).await,
        Vec::new(),
        "the denied reader is served no event of the EHR"
    );
    let other = Reader::new(
        CallerKind::AccessService,
        Some(uuid::Uuid::now_v7().to_string()),
    );
    assert_eq!(
        served(&svc, &other, 100).await.len(),
        3,
        "a reader bound to another party is served every event"
    );
}

/// Under an access policy that requires a purpose, a reader that declares a
/// purpose its capability permits is served the events, and one that
/// declares none, or one it may not declare, is served none.
#[tokio::test]
async fn under_purpose_required_only_a_reader_with_a_permitted_purpose_gets_events() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let (svc, agent) = granted_feed(&pool).await;
    let ehr = crate::fixtures::create_ehr(&svc).await;
    svc.create_composition(ehr, uv(&composition_by("note", "a clinician"), "249", None))
        .await
        .expect("a composition");

    let mut reader = Reader::new(CallerKind::System, Some(agent));
    reader.policy = Some(purpose_required());
    assert_eq!(
        served(&svc, &reader, 100).await,
        Vec::new(),
        "a reader that declares no purpose is served nothing"
    );
    reader.purpose = Some(PURPOSE);
    assert_eq!(
        served(&svc, &reader, 100).await,
        Vec::new(),
        "a purpose the reader may not declare serves nothing"
    );
    reader.facts = CallerFacts::new(
        Vec::new(),
        vec![ForeignCode {
            terminology: "2.16.840.1.113883.5.8".to_owned(),
            code: PURPOSE.to_owned(),
        }],
        Vec::new(),
    );
    assert_eq!(
        served(&svc, &reader, 100).await.len(),
        2,
        "a reader with a permitted purpose is served the EHR creation and the composition"
    );
}
