// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The policy grants (#5435) over a real PostgreSQL 18: a system client
//! resolves to its `AGENT` by `(issuer, client_id)`, its grants are the
//! capabilities of the policy `ROLE`s it performs that hold today by the
//! `ROLE`'s and the `CAPABILITY`'s `time_validity` and now by the
//! credentials' `Not before` and `Not after`, `[authz.grants] allowed` refuses
//! a capability above it at commit and never honours a stored one, a change
//! on one replica reaches another through its notification, and a refill
//! rebuilds the rows from the versions. No openEHR spec gives a capability an
//! authorization meaning: our own design.

use std::collections::HashMap;
use std::time::Duration;

use assert_fs::prelude::FileWriteStr as _;
use serde_json::{Value, json};

use crate::fixtures::{if_match, object, written_vo};
use ferroehr::config::FerroEhrConfig;
use ferroehr::error::ServiceError;
use ferroehr::ids::VoId;
use ferroehr::party_binding::grants::capabilities::{Actor, AgentState, Capabilities};
use ferroehr::party_binding::grants::kind::{GrantCeiling, GrantKind};
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;
use ferroehr::versioning::committer::{CommitterIdentity, with_committer};

/// The token issuer of the system clients.
const ISSUER: &str = "https://login.example.org/machines";
/// The administrator role of the tests.
const ADMIN: &str = "ADMIN";

/// The configuration of `toml`.
fn config(toml: &str) -> FerroEhrConfig {
    let file = assert_fs::NamedTempFile::new("ferroehr.toml").expect("temp file");
    file.write_str(toml).expect("write");
    ferroehr::config::assemble(Some(file.path()), &HashMap::new(), &[]).expect("assembles")
}

/// A replica over `pool` holding the policy definitions under an empty party
/// mapping, as the server installs it, honouring `ceiling`.
async fn replica(pool: &sqlx::PgPool, ceiling: GrantCeiling) -> FerroEhrService {
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(pool));
    svc.seed_default_definitions()
        .await
        .expect("the policy definitions are seeded");
    let config = config("");
    let sections = config.authz.party_binding.sections();
    let mapping = sections
        .binding_mapping(&[], ADMIN)
        .expect("the mapping compiles");
    svc.with_mapped_party_templates(sections.mapped_templates())
        .with_binding_mapping(mapping)
        .with_grant_ceiling(ceiling)
}

/// A replica over a fresh database under the default ceiling.
async fn service() -> (testkit::TestDb, FerroEhrService) {
    let db = testkit::db().await.expect("testkit database");
    let svc = replica(&db.pool(), GrantCeiling::default()).await;
    (db, svc)
}

/// Runs `fut` as an administrator.
async fn as_admin<F: Future>(fut: F) -> F::Output {
    let caller = CommitterIdentity {
        subject: "admin".to_owned(),
        id_type: "oauth2",
        issuer: Some("https://login.example".to_owned()),
        client: None,
        person: false,
        party: None,
        representative: false,
        bound: None,
        system: false,
        system_client: None,
        external: None,
    };
    ferroehr::system_log::access_context::with_roles(
        vec![ADMIN.to_owned()],
        with_committer(Some(caller), Box::pin(fut)),
    )
    .await
}

/// The write of `data` after `preceding`, committed in `lifecycle`.
fn version<T: serde::de::DeserializeOwned>(
    data: &Value,
    preceding: Option<&str>,
    lifecycle: &str,
) -> WriteRequest<T> {
    let mut write = WriteRequest::new(crate::typed_body::typed(data));
    write.lifecycle_state = Some(lifecycle.to_owned());
    write.preceding = preceding.map(|p| p.parse().expect("an OBJECT_VERSION_ID"));
    write
}

/// Creates `body` as a party of `kind` as an administrator.
async fn create(
    svc: &FerroEhrService,
    kind: PartyKind,
    body: &Value,
) -> Result<(VoId, String), ServiceError> {
    let created =
        as_admin(async { Box::pin(svc.party_create(kind, version(body, None, "532"))).await })
            .await?;
    let uid = created.meta.as_ref().expect("a version").uid.clone();
    Ok((written_vo(&created), uid))
}

/// Commits `body` as the next version of `vo` after `uid`, in `lifecycle`.
async fn update(
    svc: &FerroEhrService,
    kind: PartyKind,
    (vo, uid): (VoId, &str),
    body: &Value,
    lifecycle: &str,
) -> Result<String, ServiceError> {
    let updated = as_admin(async {
        Box::pin(svc.party_update(
            kind,
            object(vo.to_string()),
            if_match(uid),
            version(body, Some(uid), lifecycle),
        ))
        .await
    })
    .await?;
    Ok(updated.meta.expect("a version").uid)
}

fn text(value: &str) -> Value {
    json!({ "_type": "DV_TEXT", "value": value })
}

fn coded(value: &str, code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": value,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" },
                "code_string": code } })
}

fn archetyped(archetype: &str) -> Value {
    json!({ "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
            "template_id": { "_type": "TEMPLATE_ID", "value": format!("{archetype}.0.0") },
            "rm_version": "1.1.0" })
}

/// A system `AGENT` carrying the client identifier `client_id` of
/// [`ISSUER`].
fn agent(client_id: &str) -> Value {
    let archetype = "openEHR-DEMOGRAPHIC-AGENT.system.v1";
    json!({
        "_type": "AGENT", "archetype_node_id": archetype,
        "archetype_details": archetyped(archetype), "name": text("System agent"),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "id2", "name": text("Identifiers"),
            "items": [{ "_type": "ELEMENT", "archetype_node_id": "id3", "name": text("Identifier"),
                "value": { "_type": "DV_IDENTIFIER", "issuer": ISSUER, "id": client_id,
                           "type": "client_id" } }] },
        "identities": [{ "_type": "PARTY_IDENTITY", "archetype_node_id": "id4", "name": text("Name"),
            "details": { "_type": "ITEM_TREE", "archetype_node_id": "id5", "name": text("Items"),
                "items": [{ "_type": "ELEMENT", "archetype_node_id": "id6", "name": text("Name"),
                            "value": text(client_id) }] } }]
    })
}

/// A `DV_INTERVAL<DV_DATE>` from `lower` to `upper`, each `None` unbounded.
fn dates(lower: Option<&str>, upper: Option<&str>) -> Value {
    let mut interval = json!({ "_type": "DV_INTERVAL",
        "lower_unbounded": lower.is_none(), "upper_unbounded": upper.is_none(),
        "lower_included": lower.is_some(), "upper_included": upper.is_some() });
    if let Some(lower) = lower {
        interval["lower"] = json!({ "_type": "DV_DATE", "value": lower });
    }
    if let Some(upper) = upper {
        interval["upper"] = json!({ "_type": "DV_DATE", "value": upper });
    }
    interval
}

/// The day `days` from today in UTC, as `YYYY-MM-DD`.
fn day(days: i64) -> String {
    let today = jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date();
    today
        .checked_add(jiff::Span::new().days(days))
        .expect("a date")
        .to_string()
}

/// One capability: its kind as `(value, code)`, the credentials' items, and
/// its `time_validity`.
fn capability(kind: (&str, &str), items: &[Value], validity: Option<Value>) -> Value {
    let mut capability = json!({
        "_type": "CAPABILITY", "archetype_node_id": "id6", "name": coded(kind.0, kind.1),
        "credentials": { "_type": "ITEM_TREE", "archetype_node_id": "id8", "name": text("Grant"),
                         "items": items }
    });
    if let Some(validity) = validity {
        capability["time_validity"] = validity;
    }
    capability
}

/// A credentials element `node` named `name` holding `value`.
fn element(node: &str, name: &str, value: &Value) -> Value {
    json!({ "_type": "ELEMENT", "archetype_node_id": node, "name": text(name), "value": value })
}

/// The system client relay grant writing `template`.
fn relay(template: &str, validity: Option<Value>) -> Value {
    capability(
        ("Relay", "at10"),
        &[element("id9", "Template written", &text(template))],
        validity,
    )
}

/// A policy `ROLE` of `concept` performed by the `AGENT` `performer`,
/// holding `capabilities`, with its own `time_validity`.
fn role(concept: &str, performer: VoId, validity: Option<Value>, capabilities: &[Value]) -> Value {
    let archetype = format!("openEHR-DEMOGRAPHIC-ROLE.{concept}.v1");
    let mut role = json!({
        "_type": "ROLE", "archetype_node_id": archetype,
        "archetype_details": archetyped(&archetype), "name": text("Grant role"),
        "identities": [{ "_type": "PARTY_IDENTITY", "archetype_node_id": "id2", "name": text("Role name"),
            "details": { "_type": "ITEM_TREE", "archetype_node_id": "id3", "name": text("Name"),
                "items": [{ "_type": "ELEMENT", "archetype_node_id": "id4", "name": text("Full name"),
                            "value": text(concept) }] } }],
        "performer": { "_type": "PARTY_REF", "namespace": "demographic", "type": "AGENT",
                       "id": { "_type": "HIER_OBJECT_ID", "value": performer.to_string() } },
        "capabilities": capabilities
    });
    if let Some(validity) = validity {
        role["time_validity"] = validity;
    }
    role
}

/// The templates of the grants of `capabilities`, sorted.
fn written(capabilities: &Capabilities) -> Vec<String> {
    let mut out: Vec<String> = capabilities
        .grants()
        .iter()
        .flat_map(|grant| grant.credentials().texts("id9"))
        .map(str::to_owned)
        .collect();
    out.sort();
    out
}

/// The capabilities of the system client `client_id` on `svc`, after it read
/// the grant rows.
async fn of_client(svc: &FerroEhrService, client_id: &str) -> Capabilities {
    svc.grants().refresh().await.expect("the grants read");
    svc.grants().capabilities(
        Actor::System {
            issuer: ISSUER,
            client_id,
        },
        jiff::Timestamp::now(),
    )
}

#[tokio::test]
async fn a_system_client_resolves_to_its_agent_by_issuer_and_client() {
    let (_db, svc) = service().await;
    let (lab, _) = create(&svc, PartyKind::Agent, &agent("lab-relay"))
        .await
        .expect("the AGENT commits");
    create(
        &svc,
        PartyKind::Role,
        &role("system_client", lab, None, &[relay("Lab.v1", None)]),
    )
    .await
    .expect("the ROLE commits");
    let granted = of_client(&svc, "lab-relay").await;
    let named = granted.agent().expect("the client names its AGENT");
    assert_eq!(named.party, lab.0);
    assert_eq!(named.state, AgentState::Complete);
    assert!(granted.holds(GrantKind::SystemClientRelay));
    assert_eq!(written(&granted), vec!["Lab.v1".to_owned()]);
    assert_eq!(
        svc.grants()
            .capabilities(Actor::Party(lab.0), jiff::Timestamp::now())
            .grants()
            .len(),
        1,
        "the AGENT's party resolves to the same grants"
    );
    for (issuer, client_id) in [(ISSUER, "lab"), ("https://other.example", "lab-relay")] {
        let none = svc
            .grants()
            .capabilities(Actor::System { issuer, client_id }, jiff::Timestamp::now());
        assert_eq!(none, Capabilities::default(), "{issuer} {client_id}");
    }
    let refused = create(&svc, PartyKind::Agent, &agent("lab-relay"))
        .await
        .expect_err("a second AGENT cannot name the client");
    assert!(matches!(refused, ServiceError::Conflict(_)), "{refused:?}");
}

#[tokio::test]
async fn only_a_role_and_capability_valid_today_grant() {
    let (_db, svc) = service().await;
    let (lab, _) = create(&svc, PartyKind::Agent, &agent("lab"))
        .await
        .expect("the AGENT commits");
    let (past, next) = (day(-10), day(10));
    let (yesterday, tomorrow) = (day(-1), day(1));
    let cases = [
        (
            "Role.current",
            Some(dates(Some(&yesterday), Some(&tomorrow))),
            None,
        ),
        ("Role.open", None, None),
        ("Role.future", Some(dates(Some(&tomorrow), None)), None),
        (
            "Role.expired",
            Some(dates(Some(&past), Some(&yesterday))),
            None,
        ),
        (
            "Capability.current",
            None,
            Some(dates(Some(&day(0)), Some(&day(0)))),
        ),
        (
            "Capability.future",
            None,
            Some(dates(Some(&tomorrow), Some(&next))),
        ),
        (
            "Capability.expired",
            None,
            Some(dates(None, Some(&yesterday))),
        ),
    ];
    for (template, role_validity, validity) in cases {
        create(
            &svc,
            PartyKind::Role,
            &role(
                "system_client",
                lab,
                role_validity,
                &[relay(template, validity)],
            ),
        )
        .await
        .unwrap_or_else(|e| panic!("{template} commits: {e}"));
    }
    let now = jiff::Timestamp::now();
    let instant = |hours: i64| {
        now.checked_add(jiff::SignedDuration::from_hours(hours))
            .expect("an instant")
            .to_string()
    };
    let bounded = |template: &str, before: Option<String>, after: Option<String>| {
        let mut items = vec![element("id9", "Template written", &text(template))];
        for (node, name, at) in [("id15", "Not before", before), ("id16", "Not after", after)] {
            if let Some(at) = at {
                items.push(element(
                    node,
                    name,
                    &json!({ "_type": "DV_DATE_TIME", "value": at }),
                ));
            }
        }
        capability(("Relay", "at10"), &items, None)
    };
    for capability in [
        bounded("Instant.current", Some(instant(-1)), Some(instant(1))),
        bounded("Instant.future", Some(instant(1)), None),
        bounded("Instant.expired", None, Some(instant(-1))),
    ] {
        create(
            &svc,
            PartyKind::Role,
            &role("system_client", lab, None, &[capability]),
        )
        .await
        .expect("a bounded ROLE commits");
    }
    assert_eq!(
        written(&of_client(&svc, "lab").await),
        vec![
            "Capability.current".to_owned(),
            "Instant.current".to_owned(),
            "Role.current".to_owned(),
            "Role.open".to_owned(),
        ]
    );
}

#[tokio::test]
async fn a_capability_change_and_an_inactive_agent_apply_on_the_next_read() {
    let (_db, svc) = service().await;
    let (lab, lab_uid) = create(&svc, PartyKind::Agent, &agent("lab"))
        .await
        .expect("the AGENT commits");
    let (grant, uid) = create(
        &svc,
        PartyKind::Role,
        &role("system_client", lab, None, &[relay("Lab.v1", None)]),
    )
    .await
    .expect("the ROLE commits");
    assert_eq!(written(&of_client(&svc, "lab").await), vec!["Lab.v1"]);
    let ended = role(
        "system_client",
        lab,
        Some(dates(None, Some(&day(-1)))),
        &[relay("Lab.v1", None)],
    );
    update(&svc, PartyKind::Role, (grant, &uid), &ended, "532")
        .await
        .expect("the ROLE ends");
    assert_eq!(of_client(&svc, "lab").await.grants(), []);
    update(
        &svc,
        PartyKind::Agent,
        (lab, &lab_uid),
        &agent("lab"),
        "800",
    )
    .await
    .expect("the AGENT is set inactive");
    let inactive = of_client(&svc, "lab").await;
    assert_eq!(
        inactive.agent().map(|agent| agent.state),
        Some(AgentState::Inactive)
    );
}

#[tokio::test]
async fn a_capability_above_the_ceiling_is_refused_at_commit() {
    let (_db, svc) = service().await;
    let (lab, _) = create(&svc, PartyKind::Agent, &agent("lab"))
        .await
        .expect("the AGENT commits");
    let refused = create(
        &svc,
        PartyKind::Role,
        &role("privileged", lab, None, &[audit()]),
    )
    .await
    .expect_err("the default ceiling admits no privilege");
    assert!(matches!(refused, ServiceError::Forbidden(_)), "{refused:?}");
    let text = refused.to_string();
    assert!(
        text.contains("privileged.subject_audit") && text.contains("[authz.grants] allowed"),
        "{text}"
    );
    let both = [
        relay("Lab.v1", None),
        capability(("Device", "at11"), &[], None),
    ];
    create(
        &svc,
        PartyKind::Role,
        &role("system_client", lab, None, &both),
    )
    .await
    .expect("both system client kinds are in the default ceiling");
    let granted = of_client(&svc, "lab").await;
    assert!(granted.holds(GrantKind::SystemClientRelay));
    assert!(granted.holds(GrantKind::SystemClientDevice));
}

/// The subject audit privilege.
fn audit() -> Value {
    capability(
        ("Subject audit", "at10"),
        &[element("id11", "Authorisation reference", &text("CHG-1"))],
        None,
    )
}

#[tokio::test]
async fn a_stored_capability_above_the_ceiling_is_never_honoured() {
    let db = testkit::db().await.expect("testkit database");
    let wide = replica(&db.pool(), GrantCeiling::new(GrantKind::ALL)).await;
    let narrow = replica(&db.pool(), GrantCeiling::default()).await;
    let (lab, _) = create(&wide, PartyKind::Agent, &agent("lab"))
        .await
        .expect("the AGENT commits");
    create(
        &wide,
        PartyKind::Role,
        &role("privileged", lab, None, &[audit()]),
    )
    .await
    .expect("the wide ceiling admits the privilege");
    assert!(of_client(&wide, "lab").await.holds(GrantKind::SubjectAudit));
    assert_eq!(
        of_client(&narrow, "lab").await.grants(),
        [],
        "a replica whose ceiling does not admit the stored privilege never honours it"
    );
}

#[tokio::test]
async fn a_change_on_one_replica_reaches_another_through_its_notification() {
    let db = testkit::db().await.expect("testkit database");
    let writer = replica(&db.pool(), GrantCeiling::default()).await;
    let reader = replica(&db.pool(), GrantCeiling::default()).await;
    let listened = reader.grants().listened();
    listened.spawn_listener();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !listened.is_listening() {
        assert!(
            std::time::Instant::now() < deadline,
            "the listener connects"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    reader.grants().refresh_if_stale().await.expect("a read");
    let seen = listened.notifications();
    let (lab, _) = create(&writer, PartyKind::Agent, &agent("lab"))
        .await
        .expect("the AGENT commits");
    create(
        &writer,
        PartyKind::Role,
        &role("system_client", lab, None, &[relay("Lab.v1", None)]),
    )
    .await
    .expect("the ROLE commits");
    assert!(
        listened
            .notified_after(seen + 1, Duration::from_secs(10))
            .await,
        "both commits notify"
    );
    reader
        .grants()
        .refresh_if_stale()
        .await
        .expect("the notified read");
    let granted = reader.grants().capabilities(
        Actor::System {
            issuer: ISSUER,
            client_id: "lab",
        },
        jiff::Timestamp::now(),
    );
    assert_eq!(written(&granted), vec!["Lab.v1"]);
}

#[tokio::test]
async fn a_refill_rebuilds_the_grants_from_the_versions() {
    let (db, svc) = service().await;
    let (lab, _) = create(&svc, PartyKind::Agent, &agent("lab"))
        .await
        .expect("the AGENT commits");
    create(
        &svc,
        PartyKind::Role,
        &role("system_client", lab, None, &[relay("Lab.v1", None)]),
    )
    .await
    .expect("the ROLE commits");
    assert!(of_client(&svc, "lab").await.agent().is_some());
    for statement in [
        "DELETE FROM party.binding_agent",
        "DELETE FROM party.binding_capability",
    ] {
        sqlx::query(statement)
            .execute(&db.pool())
            .await
            .expect("the rows go");
    }
    assert!(
        of_client(&svc, "lab").await.agent().is_some(),
        "the copy held stays"
    );
    svc.refill_binding(Duration::from_secs(60))
        .await
        .expect("the unmapped refill rebuilds the grants");
    let granted = of_client(&svc, "lab").await;
    assert_eq!(granted.agent().map(|agent| agent.party), Some(lab.0));
    assert_eq!(written(&granted), vec!["Lab.v1"]);
}

#[tokio::test]
async fn a_store_that_cannot_answer_is_unavailable() {
    let (db, svc) = service().await;
    let closed = db.pool();
    closed.close().await;
    let svc = svc.with_demographic_pool(closed);
    assert!(svc.grants().refresh_if_stale().await.is_err());
    assert!(svc.read_grants().await.is_err());
}

#[test]
fn the_grant_ceiling_is_read_from_the_file_and_the_environment() {
    assert_eq!(
        config("").authz.grants.allowed,
        vec![GrantKind::SystemClientRelay, GrantKind::SystemClientDevice]
    );
    assert_eq!(
        config("[authz.grants]\nallowed = [\"privileged.import\"]\n")
            .authz
            .grants
            .allowed,
        vec![GrantKind::Import]
    );
    let env = HashMap::from([(
        "FERROEHR__AUTHZ__GRANTS__ALLOWED".to_owned(),
        "data_migration,privileged.subject_audit".to_owned(),
    )]);
    let from_env = ferroehr::config::assemble(None, &env, &[]).expect("assembles");
    assert_eq!(
        from_env.authz.grants.allowed,
        vec![GrantKind::DataMigration, GrantKind::SubjectAudit]
    );
    let file = assert_fs::NamedTempFile::new("ferroehr.toml").expect("temp file");
    file.write_str("[authz.grants]\nallowed = [\"privileged.everything\"]\n")
        .expect("write");
    ferroehr::config::assemble(Some(file.path()), &HashMap::new(), &[])
        .expect_err("an unknown kind is refused");
}
