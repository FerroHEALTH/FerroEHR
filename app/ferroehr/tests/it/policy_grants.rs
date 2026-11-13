// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The policy grants (#5435) over a real PostgreSQL 18, every party built on
//! a deployment's own archetypes with no template: a client resolves to the
//! `ACTOR` carrying its `client_id` identifier, its grants are the operation
//! codes of `ferroehr-operations` in the credentials of the `ROLE`s it
//! performs that hold today by the `ROLE`'s and the `CAPABILITY`'s
//! `time_validity` and now by the `not_before` and `not_after` elements,
//! `[authz.grants] allowed` refuses an operation above it at commit and never
//! honours a stored one, a change on one replica reaches another through its
//! notification, a refill rebuilds the rows from the versions, and an import
//! grant admits an import with no import role (#5438). No openEHR
//! spec gives a capability an authorization meaning: our own design.

use std::collections::HashMap;
use std::time::Duration;

use assert_fs::prelude::FileWriteStr as _;
use serde_json::{Value, json};

use crate::fixtures::{if_match, object, written_vo};
use ferroehr::config::FerroEhrConfig;
use ferroehr::error::ServiceError;
use ferroehr::ids::VoId;
use ferroehr::party_binding::grants::admission::{ADMIT_IDP, LOGIN_ISSUER};
use ferroehr::party_binding::grants::capabilities::{
    Actor, Capabilities, ClientState, with_capabilities,
};
use ferroehr::party_binding::grants::operation::{GrantCeiling, Operation};
use ferroehr::party_binding::policy_mark::TERMINOLOGY;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::message::import_policy::ImportPolicy;
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
        agent: None,
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

/// A `DV_CODED_TEXT` of the operation terminology.
fn operation_code(code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": TERMINOLOGY },
                "code_string": code } })
}

/// The `ARCHETYPED` of the deployment's own `archetype`, with no template.
fn archetyped(archetype: &str) -> Value {
    json!({ "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
            "rm_version": "1.1.0" })
}

/// A name identity of `name`.
fn identities(name: &str) -> Value {
    json!([{ "_type": "PARTY_IDENTITY", "archetype_node_id": "at0001", "name": text("name"),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0002", "name": text("details"),
            "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0003", "name": text("name"),
                        "value": text(name) }] } }])
}

/// An `AGENT` of the deployment's own archetype carrying the client
/// identifier `client_id` of [`ISSUER`] deep in its details.
fn agent(client_id: &str) -> Value {
    let archetype = "openEHR-DEMOGRAPHIC-AGENT.integration_engine.v2";
    json!({
        "_type": "AGENT", "archetype_node_id": archetype,
        "archetype_details": archetyped(archetype), "name": text("Integration engine"),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0010", "name": text("details"),
            "items": [{ "_type": "CLUSTER", "archetype_node_id": "at0011", "name": text("keys"),
                "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0012", "name": text("client"),
                    "value": { "_type": "DV_IDENTIFIER", "issuer": ISSUER, "id": client_id,
                               "type": "client_id" } }] }] },
        "identities": identities(client_id)
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

/// A credentials element whose name is the terminology's `code`, holding
/// `value`.
fn named(code: &str, value: &Value) -> Value {
    json!({ "_type": "ELEMENT", "archetype_node_id": "at0030", "name": operation_code(code),
            "value": value })
}

/// A capability granting `operations`, each an element of its own, with the
/// further credentials items `items` and its `time_validity`.
fn capability(operations: &[&str], items: &[Value], validity: Option<Value>) -> Value {
    let mut all: Vec<Value> = operations
        .iter()
        .map(|code| {
            json!({ "_type": "ELEMENT", "archetype_node_id": "at0021",
                            "name": text("operation"), "value": operation_code(code) })
        })
        .collect();
    all.extend_from_slice(items);
    let mut capability = json!({
        "_type": "CAPABILITY", "archetype_node_id": "at0020", "name": text("grant"),
        "credentials": { "_type": "ITEM_TREE", "archetype_node_id": "at0022", "name": text("terms"),
                         "items": all }
    });
    if let Some(validity) = validity {
        capability["time_validity"] = validity;
    }
    capability
}

/// The grant to commit compositions of `template`, relayed.
fn writes(template: &str, validity: Option<Value>) -> Value {
    capability(
        &["commit_composition"],
        &[
            named("template", &text(template)),
            named("origin", &operation_code("relay")),
        ],
        validity,
    )
}

/// A `ROLE` of the deployment's own archetype performed by `performer`,
/// holding `capabilities`, with its own `time_validity`.
fn role(performer: VoId, validity: Option<Value>, capabilities: &[Value]) -> Value {
    let archetype = "openEHR-DEMOGRAPHIC-ROLE.legacy_migration.v1";
    let mut role = json!({
        "_type": "ROLE", "archetype_node_id": archetype,
        "archetype_details": archetyped(archetype), "name": text("Legacy migration"),
        "identities": identities("legacy migration"),
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
        .flat_map(|grant| grant.credentials().texts("template"))
        .map(str::to_owned)
        .collect();
    out.sort();
    out
}

/// The capabilities of the client `client_id` on `svc`, after it read the
/// grant rows.
async fn of_client(svc: &FerroEhrService, client_id: &str) -> Capabilities {
    svc.grants().refresh().await.expect("the grants read");
    svc.grants().capabilities(
        Actor::Client {
            issuer: ISSUER,
            client_id,
        },
        jiff::Timestamp::now(),
    )
}

#[tokio::test]
async fn a_client_resolves_to_the_actor_carrying_its_identifier() {
    let (_db, svc) = service().await;
    let (lab, _) = create(&svc, PartyKind::Agent, &agent("lab-relay"))
        .await
        .expect("the AGENT commits");
    create(
        &svc,
        PartyKind::Role,
        &role(lab, None, &[writes("Lab.v1", None)]),
    )
    .await
    .expect("the ROLE commits");
    let granted = of_client(&svc, "lab-relay").await;
    let named = granted.client().expect("the client names its AGENT");
    assert_eq!(named.party, lab.0);
    assert_eq!(named.state, ClientState::Complete);
    assert!(granted.holds(Operation::CommitComposition));
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
            .capabilities(Actor::Client { issuer, client_id }, jiff::Timestamp::now());
        assert_eq!(none, Capabilities::default(), "{issuer} {client_id}");
    }
    let refused = create(&svc, PartyKind::Agent, &agent("lab-relay"))
        .await
        .expect_err("a second AGENT cannot name the client");
    assert!(matches!(refused, ServiceError::Conflict(_)), "{refused:?}");
}

#[tokio::test]
async fn a_deployments_own_archetypes_grant_by_operation_code() {
    let db = testkit::db().await.expect("testkit database");
    let svc = replica(&db.pool(), GrantCeiling::new(Operation::ALL)).await;
    let (migration, _) = create(&svc, PartyKind::Agent, &agent("legacy-migration"))
        .await
        .expect("the AGENT commits");
    let nested = json!({ "_type": "CLUSTER", "archetype_node_id": "at0040", "name": text("scope"),
        "items": [
            { "_type": "ELEMENT", "archetype_node_id": "at0041", "name": text("may"),
              "value": operation_code("export_ehr_extract") },
            named("subject_namespace", &text("nl.legacy-his.patients"))
        ] });
    let migrates = capability(&["import_ehr"], &[nested], None);
    let elsewhere = json!({ "_type": "ELEMENT", "archetype_node_id": "at0042",
        "name": text("operation"),
        "value": { "_type": "DV_CODED_TEXT", "value": "import_ehr_extract",
                   "defining_code": { "_type": "CODE_PHRASE",
                       "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" },
                       "code_string": "import_ehr_extract" } } });
    let other_terminology = capability(&[], &[elsewhere], None);
    create(
        &svc,
        PartyKind::Role,
        &role(migration, None, &[migrates, other_terminology]),
    )
    .await
    .expect("the ROLE commits");
    let granted = of_client(&svc, "legacy-migration").await;
    let mut operations: Vec<Operation> = granted
        .grants()
        .iter()
        .map(ferroehr::party_binding::grants::capabilities::Grant::operation)
        .collect();
    operations.sort();
    assert_eq!(
        operations,
        vec![Operation::ImportEhr, Operation::ExportEhrExtract],
        "only the codes of the operation terminology grant, wherever they sit"
    );
    let import = granted
        .of(Operation::ImportEhr)
        .next()
        .expect("the import grant");
    assert_eq!(
        import.credentials().texts("subject_namespace"),
        vec!["nl.legacy-his.patients"]
    );
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
            &role(lab, role_validity, &[writes(template, validity)]),
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
        let mut items = vec![
            named("template", &text(template)),
            named("origin", &operation_code("relay")),
        ];
        for (code, at) in [("not_before", before), ("not_after", after)] {
            if let Some(at) = at {
                items.push(named(
                    code,
                    &json!({ "_type": "DV_DATE_TIME", "value": at }),
                ));
            }
        }
        capability(&["commit_composition"], &items, None)
    };
    create(
        &svc,
        PartyKind::Role,
        &role(
            lab,
            None,
            &[
                bounded("Instant.current", Some(instant(-1)), Some(instant(1))),
                bounded("Instant.future", Some(instant(1)), None),
                bounded("Instant.expired", None, Some(instant(-1))),
            ],
        ),
    )
    .await
    .expect("the bounded ROLE commits");
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
async fn a_capability_change_and_an_inactive_client_apply_on_the_next_read() {
    let (_db, svc) = service().await;
    let (lab, lab_uid) = create(&svc, PartyKind::Agent, &agent("lab"))
        .await
        .expect("the AGENT commits");
    let (grant, uid) = create(
        &svc,
        PartyKind::Role,
        &role(lab, None, &[writes("Lab.v1", None)]),
    )
    .await
    .expect("the ROLE commits");
    assert_eq!(written(&of_client(&svc, "lab").await), vec!["Lab.v1"]);
    let ended = role(
        lab,
        Some(dates(None, Some(&day(-1)))),
        &[writes("Lab.v1", None)],
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
        inactive.client().map(|client| client.state),
        Some(ClientState::Inactive)
    );
}

#[tokio::test]
async fn an_operation_above_the_ceiling_is_refused_at_commit() {
    let (_db, svc) = service().await;
    let (lab, _) = create(&svc, PartyKind::Agent, &agent("lab"))
        .await
        .expect("the AGENT commits");
    for code in ["import_ehr", "import_everything"] {
        let refused = create(
            &svc,
            PartyKind::Role,
            &role(lab, None, &[capability(&[code], &[], None)]),
        )
        .await
        .expect_err("the default ceiling admits commit_composition only");
        assert!(matches!(refused, ServiceError::Forbidden(_)), "{refused:?}");
        let text = refused.to_string();
        assert!(
            text.contains(code) && text.contains("[authz.grants] allowed"),
            "{text}"
        );
    }
    create(
        &svc,
        PartyKind::Role,
        &role(lab, None, &[writes("Lab.v1", None)]),
    )
    .await
    .expect("commit_composition is in the default ceiling");
    assert!(
        of_client(&svc, "lab")
            .await
            .holds(Operation::CommitComposition)
    );
}

/// A grant is refused at its commit when it lacks the terms its operation is
/// checked by, the rules the removed `[[authz.system_client]]` boot checks
/// held (#5436): an origin, a device identifier issuer for a device, a
/// facility's issuer with one identifier or prefix, and a subject namespace
/// for an import or an export.
#[tokio::test]
async fn a_grant_without_the_terms_of_its_operation_is_refused_at_commit() {
    let db = testkit::db().await.expect("testkit database");
    let svc = replica(&db.pool(), GrantCeiling::new(Operation::ALL)).await;
    let (lab, _) = create(&svc, PartyKind::Agent, &agent("lab"))
        .await
        .expect("the AGENT commits");
    let template = named("template", &text("Lab.v1"));
    let origin = |code: &str| named("origin", &operation_code(code));
    let facility = |items: Vec<Value>| {
        json!({ "_type": "CLUSTER", "archetype_node_id": "at0031",
                "name": operation_code("facility"), "items": items })
    };
    let issuer = named("issuer", &text("urn:oid:2.999.1"));
    for (items, refusal) in [
        (vec![template.clone()], "needs an origin"),
        (
            vec![template.clone(), origin("import_ehr")],
            "needs an origin",
        ),
        (
            vec![template.clone(), origin("device")],
            "needs a device_identifier_issuer",
        ),
        (
            vec![
                template.clone(),
                origin("relay"),
                facility(vec![named("identifier", &text("ward-7"))]),
            ],
            "facility cluster",
        ),
        (
            vec![
                template.clone(),
                origin("relay"),
                facility(vec![
                    issuer.clone(),
                    named("identifier", &text("ward-7")),
                    named("identifier_prefix", &text("ward-")),
                ]),
            ],
            "facility cluster",
        ),
    ] {
        let refused = create(
            &svc,
            PartyKind::Role,
            &role(
                lab,
                None,
                &[capability(&["commit_composition"], &items, None)],
            ),
        )
        .await
        .expect_err("a grant without its terms");
        assert!(
            matches!(refused, ServiceError::Unprocessable { .. }),
            "{refused:?}"
        );
        assert!(format!("{refused:?}").contains(refusal), "{refused:?}");
    }
    for code in ["import_ehr", "import_ehr_extract", "export_ehr_extract"] {
        let refused = create(
            &svc,
            PartyKind::Role,
            &role(lab, None, &[capability(&[code], &[], None)]),
        )
        .await
        .expect_err("an import or export grant without a subject namespace");
        assert!(
            format!("{refused:?}").contains("names no subject_namespace"),
            "{code}: {refused:?}"
        );
    }
    let complete = capability(
        &["commit_composition"],
        &[
            template,
            origin("relay"),
            origin("device"),
            named("device_identifier_issuer", &text("urn:oid:2.999.9")),
            facility(vec![issuer, named("identifier_prefix", &text("ward-"))]),
        ],
        None,
    );
    let migrates = capability(
        &["import_ehr", "export_ehr_extract"],
        &[named("subject_namespace", &text("nl.legacy-his.patients"))],
        None,
    );
    create(
        &svc,
        PartyKind::Role,
        &role(lab, None, &[complete, migrates]),
    )
    .await
    .expect("a grant with its terms commits");
}

#[tokio::test]
async fn a_stored_operation_above_the_ceiling_is_never_honoured() {
    let db = testkit::db().await.expect("testkit database");
    let wide = replica(&db.pool(), GrantCeiling::new(Operation::ALL)).await;
    let narrow = replica(&db.pool(), GrantCeiling::default()).await;
    let (lab, _) = create(&wide, PartyKind::Agent, &agent("lab"))
        .await
        .expect("the AGENT commits");
    create(
        &wide,
        PartyKind::Role,
        &role(lab, None, &[capability(&["read_subject_audit"], &[], None)]),
    )
    .await
    .expect("the wide ceiling admits the privilege");
    assert!(
        of_client(&wide, "lab")
            .await
            .holds(Operation::ReadSubjectAudit)
    );
    assert_eq!(
        of_client(&narrow, "lab").await.grants(),
        [],
        "a replica whose ceiling does not admit the stored privilege never honours it"
    );
}

/// The token issuer of the persons whose first logins an admission admits.
const LOGIN: &str = "https://login.example.org/persons";

/// A capability granting `create_at_login` for [`LOGIN`] with the further
/// credentials `items`.
fn admits(items: &[Value]) -> Value {
    let mut all = vec![named(
        LOGIN_ISSUER,
        &json!({ "_type": "DV_URI", "value": LOGIN }),
    )];
    all.extend_from_slice(items);
    capability(&["create_at_login"], &all, None)
}

#[tokio::test]
async fn a_create_at_login_grant_admits_the_logins_of_the_issuer_it_names_per_read() {
    let db = testkit::db().await.expect("testkit database");
    let svc = replica(&db.pool(), GrantCeiling::new(Operation::ALL)).await;
    let (idp, _) = create(&svc, PartyKind::Agent, &agent("identity-provider"))
        .await
        .expect("the AGENT commits");
    let admission = role(
        idp,
        None,
        &[admits(&[named(ADMIT_IDP, &text("https://sts.example/"))])],
    );
    let (role_vo, role_uid) = create(&svc, PartyKind::Role, &admission)
        .await
        .expect("the admission commits");
    svc.grants().refresh().await.expect("the grants read");
    let now = jiff::Timestamp::now();
    let [admitted] = &*svc.grants().admissions(LOGIN, now) else {
        panic!("one admission for the issuer it names");
    };
    assert_eq!(admitted.idp(), ["https://sts.example/"]);
    assert!(!admitted.guests());
    assert_eq!(
        svc.grants().admissions("https://other.example", now),
        [],
        "an issuer no grant names is admitted by none"
    );
    assert_eq!(
        svc.grants()
            .capabilities(Actor::Party(idp.0), now)
            .grants()
            .len(),
        1,
        "the performer holds the grant like any other"
    );
    let ended = role(
        idp,
        Some(dates(None, Some(&day(-1)))),
        &[admits(&[named(ADMIT_IDP, &text("https://sts.example/"))])],
    );
    update(&svc, PartyKind::Role, (role_vo, &role_uid), &ended, "532")
        .await
        .expect("the admission ends");
    svc.grants().refresh().await.expect("the grants read");
    assert_eq!(
        svc.grants().admissions(LOGIN, jiff::Timestamp::now()),
        [],
        "the ended admission admits nothing on the next read"
    );
}

#[tokio::test]
async fn the_books_login_admission_admits_its_tenant_from_its_first_day() {
    let db = testkit::db().await.expect("testkit database");
    let svc = replica(&db.pool(), GrantCeiling::new(Operation::ALL)).await;
    let organisation = crate::policy_parties::plain(
        "ORGANISATION",
        "openEHR-DEMOGRAPHIC-ORGANISATION.hospital.v1",
        "Hospital",
    );
    let (provider, _) = create(&svc, PartyKind::Organisation, &organisation)
        .await
        .expect("the ORGANISATION commits");
    let admission = crate::policy_parties::body(
        "login-admission",
        &[(crate::policy_parties::PROVIDER, provider)],
    );
    create(&svc, PartyKind::Role, &admission)
        .await
        .expect("the book's admission commits");
    svc.grants().refresh().await.expect("the grants read");
    let tenant = "https://login.microsoftonline.com/0f1e2d3c-4b5a-4968-8776-655443322110/v2.0";
    let day_one: jiff::Timestamp = "2026-12-01T08:00:00Z".parse().expect("an instant");
    let [admitted] = &*svc.grants().admissions(tenant, day_one) else {
        panic!("the tenant is admitted from 1 December 2026");
    };
    assert_eq!(admitted.groups(), ["5f2a9c1e-0000-4000-8000-00000000c11a"]);
    assert_eq!(
        admitted.assurance(),
        Some(ferrohealth_smart::assurance::AssuranceLevel::Substantial)
    );
    let eve: jiff::Timestamp = "2026-11-30T23:00:00Z".parse().expect("an instant");
    assert_eq!(
        svc.grants().admissions(tenant, eve),
        [],
        "not before its first day"
    );
}

#[tokio::test]
async fn a_create_at_login_grant_admitting_every_login_or_above_the_ceiling_is_refused() {
    let db = testkit::db().await.expect("testkit database");
    let svc = replica(&db.pool(), GrantCeiling::new(Operation::ALL)).await;
    let (idp, _) = create(&svc, PartyKind::Agent, &agent("identity-provider"))
        .await
        .expect("the AGENT commits");
    let refused = create(&svc, PartyKind::Role, &role(idp, None, &[admits(&[])]))
        .await
        .expect_err("an admission without a condition admits every login");
    assert!(
        matches!(&refused, ServiceError::Unprocessable { violation, .. }
            if violation.detail().contains("declares no admission condition")),
        "{refused:?}"
    );
    let unnamed = capability(&["create_at_login"], &[named(ADMIT_IDP, &text("x"))], None);
    let refused = create(&svc, PartyKind::Role, &role(idp, None, &[unnamed]))
        .await
        .expect_err("an admission naming no issuer");
    assert!(
        matches!(&refused, ServiceError::Unprocessable { violation, .. }
            if violation.detail().contains("names no login issuer")),
        "{refused:?}"
    );
    let narrow = replica(&db.pool(), GrantCeiling::default()).await;
    let refused = create(
        &narrow,
        PartyKind::Role,
        &role(idp, None, &[admits(&[named(ADMIT_IDP, &text("x"))])]),
    )
    .await
    .expect_err("the default ceiling does not admit create_at_login");
    assert!(matches!(refused, ServiceError::Forbidden(_)), "{refused:?}");
    assert_eq!(svc.grants().admissions(LOGIN, jiff::Timestamp::now()), []);
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
        &role(lab, None, &[writes("Lab.v1", None)]),
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
        Actor::Client {
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
        &role(lab, None, &[writes("Lab.v1", None)]),
    )
    .await
    .expect("the ROLE commits");
    assert!(of_client(&svc, "lab").await.client().is_some());
    for statement in [
        "DELETE FROM party.binding_client",
        "DELETE FROM party.binding_capability",
    ] {
        sqlx::query(statement)
            .execute(&db.pool())
            .await
            .expect("the rows go");
    }
    assert!(
        of_client(&svc, "lab").await.client().is_some(),
        "the copy held stays"
    );
    svc.refill_binding(Duration::from_secs(60))
        .await
        .expect("the refill rebuilds the grants");
    let granted = of_client(&svc, "lab").await;
    assert_eq!(granted.client().map(|client| client.party), Some(lab.0));
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
        vec![Operation::CommitComposition]
    );
    assert_eq!(
        config("[authz.grants]\nallowed = [\"import_ehr\"]\n")
            .authz
            .grants
            .allowed,
        vec![Operation::ImportEhr]
    );
    let env = HashMap::from([(
        "FERROEHR__AUTHZ__GRANTS__ALLOWED".to_owned(),
        "export_ehr_extract,read_subject_audit".to_owned(),
    )]);
    let from_env = ferroehr::config::assemble(None, &env, &[]).expect("assembles");
    assert_eq!(
        from_env.authz.grants.allowed,
        vec![Operation::ExportEhrExtract, Operation::ReadSubjectAudit]
    );
    let file = assert_fs::NamedTempFile::new("ferroehr.toml").expect("temp file");
    file.write_str("[authz.grants]\nallowed = [\"import_everything\"]\n")
        .expect("write");
    ferroehr::config::assemble(Some(file.path()), &HashMap::new(), &[])
        .expect_err("an unknown operation is refused");
}

#[tokio::test]
async fn only_an_import_grant_admits_an_import() {
    let (_source_db, source) = crate::fixtures::service().await;
    let subject = ferroehr::service::ehr_index::types::SubjectRef {
        id: "pt-1".to_owned(),
        namespace: "nl.legacy-his.patients".to_owned(),
        r#type: "PERSON".to_owned(),
    };
    let ehr = crate::fixtures::create_ehr_subject(&source, &subject).await;
    let extracts = source.extract_ehrs(ehr).await.expect("export");
    let extract: openehr_rm::prelude::Extract =
        openehr_its::json::from_canonical_value(&extracts[0]).expect("an EXTRACT");
    let db = testkit::db().await.expect("testkit database");
    let source_system = crate::committer_binding::source_system_id(&extracts[0]);
    let target = replica(&db.pool(), GrantCeiling::new([Operation::ImportEhr]))
        .await
        .with_import_policy(ImportPolicy::new(vec![ISSUER.to_owned()]));
    let (migrator, _) = create(&target, PartyKind::Agent, &agent("migrator"))
        .await
        .expect("the AGENT commits");
    create(
        &target,
        PartyKind::Role,
        // NOTE: RM ehr_extract master09-semantics.adoc §Creation Semantics, FerroEHR's
        // own extract rewrites every reference namespace to `local`.
        &role(
            migrator,
            None,
            &[capability(
                &["import_ehr"],
                &[
                    crate::policy_fixtures::scope("creating_system", &source_system),
                    named("subject_namespace", &text("local")),
                ],
                None,
            )],
        ),
    )
    .await
    .expect("the ceiling admits import_ehr");
    target.grants().refresh().await.expect("the grants read");
    let granted = std::sync::Arc::new(
        target
            .grants()
            .capabilities(Actor::Party(migrator.0), jiff::Timestamp::now()),
    );
    assert!(granted.holds(Operation::ImportEhr));
    let person = CommitterIdentity {
        subject: "migrator".to_owned(),
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
    };
    let refused = ferroehr::system_log::access_context::with_roles(
        vec!["IMPORTER".to_owned()],
        with_committer(
            Some(person.clone()),
            Box::pin(target.import_ehr(None, extract.clone())),
        ),
    )
    .await
    .expect_err("a role is no grant");
    assert!(matches!(refused.flattened(), ServiceError::Forbidden(_)));
    with_capabilities(
        granted,
        with_committer(Some(person), Box::pin(target.import_ehr(None, extract))),
    )
    .await
    .expect("the import_ehr grant admits the import");
}

#[test]
fn the_privileged_roles_are_reserved_from_a_hold_roles_grant() {
    let config = config("[authz.rbac]\nemergency_access_role = \"Break_Glass\"\n");
    assert_eq!(
        config.authz.rbac.privileged_roles(),
        ["ADMIN", "BREAK_GLASS"]
    );
    let ceiling = config.authz.grants.ceiling(&config.authz.rbac);
    assert!(ceiling.reserves("admin") && ceiling.reserves("break_glass"));
    assert!(!ceiling.reserves("READONLY") && !ceiling.reserves("IMPORTER"));
    assert_eq!(
        ceiling.allowed().collect::<Vec<_>>(),
        vec![Operation::CommitComposition]
    );
}
