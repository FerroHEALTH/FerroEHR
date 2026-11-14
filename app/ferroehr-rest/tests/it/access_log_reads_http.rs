// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access log over ITS-REST (#5130), end to end over the assembled router,
//! the audit drain and the local store, on a real `PostgreSQL` 18.
//!
//! A reviewer reads an EHR's log and the whole log in the full view; the
//! subject of care, signing in through the patient portal, and their parent,
//! as the actor of the child's token (#5446), read the subject view of the
//! child's EHR and nothing of any other; a professional the admin class does
//! not admit is refused; every read of the log writes its own entry; and an
//! EHDS Art. 9(3) restriction in the controller's access policy withholds the
//! accessor it names from the subject view alone, from the next request on
//! (#5570).
//! No openEHR spec governs the access log: our own design on the draft
//! `org.openehr.rm.access_log` 0.1.0.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use ferroehr::access_policy::ARCHETYPE;
use ferroehr::config::authz::AuthzConfig;
use ferroehr::service::FerroEhrService;
use ferroehr::service::admin::access_policy::PolicyPrecondition;
use ferroehr::system_log::store::AuditStore;
use ferroehr_rest::extensions::access::authz::AuthzHandle;
use ferroehr_rest::extensions::management::Observability;
use http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::{self, BASE, EXT};
use crate::portal_representatives_http::{
    CHILD_LOGIN, PARENT, PORTAL, child, config, representation, representative, token,
};

/// The role every portal token carries, so the clinical class admits it.
pub(crate) const PATIENT_ROLE: &str = "PATIENT";

/// The assembled server and what a test reads beside it.
pub(crate) struct Fixture {
    _db: testkit::TestDb,
    _drain: ferroehr::system_log::sender::AuditHandle,
    pub(crate) pool: PgPool,
    svc: Arc<FerroEhrService>,
    pub(crate) app: Router,
    /// The child's EHR, which the parent represents.
    pub(crate) ehr: String,
    /// Another person's EHR.
    pub(crate) elsewhere: String,
}

/// The server over a fresh database: the patient portal trusted with its
/// roles, the professional `dr` and the administrator `root`, the RBAC gate
/// on, and the audit drain writing the access log; `root` has read the
/// child's EHR once, so its log holds a professional's access.
pub(crate) async fn fixture() -> Fixture {
    let (db, pool) = common::migrated_pool().await;
    let (sender, drain) =
        ferroehr::system_log::sender::start(common::local_audit(), None, Some(pool.clone()))
            .await
            .expect("the audit sender");
    let svc = Arc::new(
        common::binding_service(&pool)
            .with_audit(sender)
            .with_audit_store(AuditStore::new(pool.clone()))
            .with_binding_mapping(crate::party_binding_http::mapping()),
    );
    let (subject, ehr) = child(&svc).await;
    let (_, elsewhere) = child(&svc).await;
    representative(
        &svc,
        PARENT,
        json!([representation(subject, subject, "254", None)]),
    )
    .await;
    let mut config = config();
    config.api.admin = true;
    config.rbac.role_issuers = vec![ferrohealth_secret::secret_url::SecretUrl::new(PORTAL)];
    common::with_basic_users(
        &mut config,
        &svc,
        &pool,
        &[("dr", &["USER"]), ("root", &["ADMIN"])],
    )
    .await;
    let mut authz = AuthzConfig::default();
    authz.rbac.enabled = true;
    let handle = AuthzHandle::build(&authz, &config.api.base_path, None).map(Arc::new);
    let app = common::build_full(config, Arc::clone(&svc), handle, Observability::default()).await;
    let fixture = Fixture {
        _db: db,
        _drain: drain,
        pool,
        svc,
        app,
        ehr,
        elsewhere,
    };
    let (status, body) = get(
        &fixture.app,
        &format!("{BASE}/ehr/{}", fixture.ehr),
        &root(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "root reads the EHR: {body}");
    stored(&fixture.pool, 1, |entry| {
        entry["ehr_id"]["value"] == json!(fixture.ehr)
            && entry["accessor"]["kind"]["value"] == json!("person")
    })
    .await;
    fixture
}

/// The administrator's credential.
pub(crate) fn root() -> String {
    common::basic_credential("root")
}

/// The child's own portal token.
fn patient(ehr: &str) -> String {
    token_with_role(None, ehr)
}

/// The parent's token, as the actor of the child's.
fn parent(ehr: &str) -> String {
    token_with_role(Some(PARENT), ehr)
}

/// A portal token for `ehr`, its actor `actor`, carrying [`PATIENT_ROLE`].
pub(crate) fn token_with_role(actor: Option<&str>, ehr: &str) -> String {
    let mut claims = json!({ "roles": [PATIENT_ROLE] });
    if let Some(actor) = actor {
        claims["act"] = json!({ "sub": actor });
    }
    token(CHILD_LOGIN, ehr, &claims)
}

/// The status and JSON body of `GET uri` as `credential`.
pub(crate) async fn get(app: &Router, uri: &str, credential: &str) -> (StatusCode, Value) {
    let (status, body) = common::send_body(
        app,
        common::req("GET", uri)
            .auth(credential)
            .accept("application/json")
            .build(),
    )
    .await;
    (status, serde_json::from_str(&body).unwrap_or(Value::Null))
}

/// The URI of `ehr`'s log, with `query`.
fn log_of(ehr: &str, query: &str) -> String {
    format!("{EXT}/ehr/{ehr}/access_log{query}")
}

/// The stored entries `keep` selects, as their canonical JSON, polled until
/// there are at least `expected`.
pub(crate) async fn stored(
    pool: &PgPool,
    expected: usize,
    keep: impl Fn(&Value) -> bool,
) -> Vec<Value> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT content FROM audit.access_log_record \
             WHERE record_type = 'ACCESS_LOG_ENTRY' ORDER BY chain_seq",
        )
        .fetch_all(pool)
        .await
        .expect("read the access log");
        let kept: Vec<Value> = rows
            .iter()
            .map(|row| serde_json::from_str(row).expect("an entry"))
            .filter(|entry| keep(entry))
            .collect();
        if kept.len() >= expected {
            return kept;
        }
        assert!(
            Instant::now() < deadline,
            "only {} of {expected} entries landed: {rows:#?}",
            kept.len()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Whether `entry` records a read of the access log by an accessor of
/// `kind`.
fn log_read_by(entry: &Value, kind: &str) -> bool {
    entry["accessor"]["kind"]["value"] == json!(kind)
        && entry["items"].as_array().is_some_and(|items| {
            items
                .iter()
                .any(|item| item["rm_type"] == json!("ACCESS_LOG_RECORD"))
        })
}

/// The entries of a page.
fn entries(page: &Value) -> &[Value] {
    page["entries"].as_array().map_or(&[], Vec::as_slice)
}

/// A reviewer reads an EHR's log and the whole log in the full view, each
/// record with its link, the links of a span of the chain, and no checkpoint
/// where none is stored.
#[tokio::test]
async fn a_reviewer_reads_the_full_view() {
    let fx = fixture().await;
    let (status, page) = get(&fx.app, &log_of(&fx.ehr, ""), &root()).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["offset"], json!(0), "{page}");
    let first = entries(&page).first().cloned().expect("an entry");
    assert!(first.get("link").is_some(), "the full view: {first}");
    assert!(first.get("channel").is_some(), "{first}");
    assert!(first["accessor"].get("binding").is_some(), "{first}");

    let uid = first["uid"]["value"].as_str().expect("a uid");
    let (status, one) = get(&fx.app, &log_of(&fx.ehr, &format!("/{uid}")), &root()).await;
    assert_eq!(status, StatusCode::OK, "{one}");
    assert_eq!(one["uid"], first["uid"]);

    let review = format!("{EXT}/admin/access-log?ehr_id={}&fetch=5", fx.ehr);
    let (status, page) = get(&fx.app, &review, &root()).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["fetch"], json!(5));
    assert!(!entries(&page).is_empty(), "{page}");
    assert!(
        entries(&page)
            .iter()
            .all(|entry| entry["ehr_id"]["value"] == json!(fx.ehr)),
        "{page}"
    );

    let sequence = first["link"]["sequence"].as_i64().expect("a sequence");
    let chain =
        format!("{EXT}/admin/access-log/chain?from_sequence={sequence}&to_sequence={sequence}");
    let (status, links) = get(&fx.app, &chain, &root()).await;
    assert_eq!(status, StatusCode::OK, "{links}");
    assert_eq!(links["links"][0]["uid"], first["uid"]["value"], "{links}");
    assert_eq!(links["links"][0]["link"], first["link"], "{links}");

    let (status, _) = get(
        &fx.app,
        &format!("{EXT}/admin/access-log/checkpoint"),
        &root(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "no access log is wired for sealing"
    );

    let unknown = log_of("00000000-0000-4000-8000-000000000000", "");
    assert_eq!(
        get(&fx.app, &unknown, &root()).await.0,
        StatusCode::NOT_FOUND
    );
    let malformed = format!("{EXT}/admin/access-log?fetch=0");
    assert_eq!(
        get(&fx.app, &malformed, &root()).await.0,
        StatusCode::BAD_REQUEST
    );
}

/// The subject reads the subject view of their own EHR's log and of nothing
/// else, and no parameter widens it.
#[tokio::test]
async fn the_subject_reads_only_their_own_log() {
    let fx = fixture().await;
    let credential = patient(&fx.ehr);
    let (status, page) = get(&fx.app, &log_of(&fx.ehr, ""), &credential).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let professional = entries(&page)
        .iter()
        .find(|entry| entry["accessor"]["kind"]["value"] == json!("person"))
        .cloned()
        .expect("the professional's read is shown");
    for withheld in ["link", "channel", "security_labels", "other_details"] {
        assert!(
            professional.get(withheld).is_none(),
            "{withheld}: {professional}"
        );
    }
    for withheld in ["credential", "delegation", "binding"] {
        assert!(
            professional["accessor"].get(withheld).is_none(),
            "{withheld}: {professional}"
        );
    }
    assert!(
        professional.get("time_recorded").is_some(),
        "{professional}"
    );
    assert!(professional.get("action").is_some(), "{professional}");

    let uid = professional["uid"]["value"].as_str().expect("a uid");
    let (status, one) = get(&fx.app, &log_of(&fx.ehr, &format!("/{uid}")), &credential).await;
    assert_eq!(status, StatusCode::OK, "{one}");
    assert!(one.get("link").is_none(), "the subject view: {one}");

    for (case, elsewhere) in [
        ("another EHR's log", log_of(&fx.elsewhere, "")),
        (
            "an entry under another EHR",
            log_of(&fx.elsewhere, &format!("/{uid}")),
        ),
        (
            "an unknown EHR",
            log_of("00000000-0000-4000-8000-000000000000", ""),
        ),
    ] {
        let (status, _) = get(&fx.app, &elsewhere, &credential).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{case}");
    }
    for widening in ["?issuer=x", "?view=full", "?ehr_id=x", "?party_id=x"] {
        let (status, body) = get(&fx.app, &log_of(&fx.ehr, widening), &credential).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{widening}: {body}");
    }
}

/// The parent reads the subject view of the child's log as the actor of the
/// child's token, and nothing of another EHR.
#[tokio::test]
async fn a_representative_reads_the_represented_subjects_log() {
    let fx = fixture().await;
    let credential = parent(&fx.ehr);
    let (status, page) = get(&fx.app, &log_of(&fx.ehr, ""), &credential).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(!entries(&page).is_empty(), "{page}");
    assert!(
        entries(&page)
            .iter()
            .all(|entry| entry.get("link").is_none()),
        "the subject view: {page}"
    );
    let (status, body) = get(&fx.app, &log_of(&fx.elsewhere, ""), &credential).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

/// A professional the admin class does not admit reads no EHR's log and not
/// the review.
#[tokio::test]
async fn another_caller_is_refused() {
    let fx = fixture().await;
    let dr = common::basic_credential("dr");
    let (status, body) = get(&fx.app, &log_of(&fx.ehr, ""), &dr).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, body) = get(&fx.app, &format!("{EXT}/admin/access-log"), &dr).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
}

/// Every read of the log writes an entry naming the log of the EHR it read:
/// the subject's own, which their default view leaves out and
/// `include_self=true` shows, and the reviewer's, which the subject sees.
#[tokio::test]
async fn each_read_of_the_log_is_recorded() {
    let fx = fixture().await;
    let credential = patient(&fx.ehr);
    let (status, _) = get(&fx.app, &log_of(&fx.ehr, ""), &credential).await;
    assert_eq!(status, StatusCode::OK);
    let own = stored(&fx.pool, 1, |entry| log_read_by(entry, "subject of care")).await;
    assert_eq!(own[0]["ehr_id"]["value"], json!(fx.ehr), "{}", own[0]);
    assert_eq!(own[0]["action"]["value"], json!("read"), "{}", own[0]);

    let (status, _) = get(&fx.app, &log_of(&fx.ehr, ""), &root()).await;
    assert_eq!(status, StatusCode::OK);
    stored(&fx.pool, 1, |entry| log_read_by(entry, "person")).await;
    let (status, _) = get(&fx.app, &format!("{EXT}/admin/access-log"), &root()).await;
    assert_eq!(status, StatusCode::OK);
    stored(&fx.pool, 1, |entry| {
        entry["operation"]["value"] == json!("admin_list_access_log")
    })
    .await;

    let (_, page) = get(&fx.app, &log_of(&fx.ehr, ""), &credential).await;
    assert!(
        !entries(&page)
            .iter()
            .any(|entry| log_read_by(entry, "subject of care")),
        "the subject's own reads are left out by default: {page}"
    );
    assert!(
        entries(&page)
            .iter()
            .any(|entry| log_read_by(entry, "person")),
        "the reviewer's read is shown: {page}"
    );
    let (_, page) = get(&fx.app, &log_of(&fx.ehr, "?include_self=true"), &credential).await;
    assert!(
        entries(&page)
            .iter()
            .any(|entry| log_read_by(entry, "subject of care")),
        "include_self shows them: {page}"
    );
}

/// A code of `terminology`, its rubric the code.
fn coded(terminology: &str, code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
        "defining_code": { "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": terminology },
            "code_string": code } })
}

/// An access policy over the default archetype; with `restricted`, it
/// withholds the party and the provider of a `person` accessor from the
/// subject view under a national provision.
fn policy(restricted: bool) -> Value {
    let mut policy = json!({
        "_type": "ACCESS_POLICY",
        "name": { "_type": "DV_TEXT", "value": "Access policy" },
        "archetype_node_id": ARCHETYPE,
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": ARCHETYPE },
            "rm_version": "1.2.0" },
        "controller": { "_type": "PARTY_REF", "namespace": "demographic", "type": "ORGANISATION",
            "id": { "_type": "HIER_OBJECT_ID", "value": "2c3d4e5f-6a7b-4c8d-9e0f-a1b2c3d4e5f7" } },
        "care_relationship_required": false,
        "purpose_required": false
    });
    if restricted {
        let control = openehr_rm::draft::access_control::terminology::TERMINOLOGY_ID_ACCESS_CONTROL;
        policy["log_restrictions"] = json!([{ "_type": "ACCESS_LOG_RESTRICTION",
            "name": { "_type": "DV_TEXT", "value": "Professional withheld" },
            "archetype_node_id": "id11",
            "accessor_kinds": [coded("openehr", "person")],
            "withheld": [coded(control, "accessor_party"), coded(control, "accessor_provider")],
            "legal_grounds": [coded("urn:example:national-law", "art-9-3")] }]);
    }
    policy
}

/// The `person` entries and the `subject of care` entries of the subject
/// view of the child's log, its own reads included.
async fn subject_view_by_kind(fx: &Fixture, credential: &str) -> (Vec<Value>, Vec<Value>) {
    let (status, page) = get(&fx.app, &log_of(&fx.ehr, "?include_self=true"), credential).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let of = |kind: &str| -> Vec<Value> {
        entries(&page)
            .iter()
            .filter(|entry| entry["accessor"]["kind"]["value"] == json!(kind))
            .cloned()
            .collect()
    };
    (of("person"), of("subject of care"))
}

/// A restriction the policy declares withholds the accessor's party and
/// provider from the subject view of a `person` accessor's entries, for the
/// subject and their representative, from the next request on, with the
/// policy written on another replica and its notification keeping this one
/// fresh; the subject's own entries and a reviewer's full view keep the
/// party, and a policy with no restriction withholds nothing again.
#[tokio::test]
async fn an_art_9_3_restriction_applies_to_the_subject_view_alone() {
    let fx = fixture().await;
    let credential = patient(&fx.ehr);
    let (_, _) = get(&fx.app, &log_of(&fx.ehr, ""), &credential).await;
    stored(&fx.pool, 1, |entry| log_read_by(entry, "subject of care")).await;
    let (people, own) = subject_view_by_kind(&fx, &credential).await;
    assert!(!people.is_empty() && !own.is_empty(), "{people:?} {own:?}");
    assert!(
        people
            .iter()
            .all(|entry| entry["accessor"].get("party").is_some()),
        "no restriction, no change: {people:?}"
    );

    let listened = fx.svc.access_policy().listened();
    listened.spawn_listener((*fx.pool.connect_options()).clone());
    let deadline = Instant::now() + Duration::from_secs(10);
    while !listened.is_listening() {
        assert!(Instant::now() < deadline, "the listener connects");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let seen = listened.notifications();
    let writer = common::binding_service(&fx.pool);
    let first = writer
        .put_access_policy(&PolicyPrecondition::Create, &policy(true))
        .await
        .expect("the restricting policy commits")
        .version_uid;
    assert!(
        listened.notified_after(seen, Duration::from_secs(10)).await,
        "the commit notifies"
    );

    for reader in [patient(&fx.ehr), parent(&fx.ehr)] {
        let (people, own) = subject_view_by_kind(&fx, &reader).await;
        assert!(!people.is_empty(), "{people:?}");
        for entry in &people {
            assert!(entry["accessor"].get("party").is_none(), "{entry}");
            assert!(entry["accessor"].get("provider").is_none(), "{entry}");
            assert_eq!(
                entry["accessor"]["kind"]["value"],
                json!("person"),
                "{entry}"
            );
            assert!(entry.get("time_recorded").is_some(), "{entry}");
        }
        assert!(
            own.iter()
                .all(|entry| entry["accessor"].get("party").is_some()),
            "another accessor kind keeps its party: {own:?}"
        );
        let uid = people[0]["uid"]["value"].as_str().expect("a uid");
        let (status, one) = get(&fx.app, &log_of(&fx.ehr, &format!("/{uid}")), &reader).await;
        assert_eq!(status, StatusCode::OK, "{one}");
        assert!(one["accessor"].get("party").is_none(), "{one}");
    }

    let (status, page) = get(&fx.app, &log_of(&fx.ehr, ""), &root()).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let reviewed: Vec<&Value> = entries(&page)
        .iter()
        .filter(|entry| entry["accessor"]["kind"]["value"] == json!("person"))
        .collect();
    assert!(!reviewed.is_empty(), "{page}");
    assert!(
        reviewed
            .iter()
            .all(|entry| entry["accessor"].get("party").is_some() && entry.get("link").is_some()),
        "the full view is unchanged: {page}"
    );

    fx.svc
        .put_access_policy(&PolicyPrecondition::Replace(first), &policy(false))
        .await
        .expect("the policy without a restriction commits");
    let (people, _) = subject_view_by_kind(&fx, &credential).await;
    assert!(
        people
            .iter()
            .all(|entry| entry["accessor"].get("party").is_some()),
        "the restriction is lifted on the next request: {people:?}"
    );
    let changes = stored(&fx.pool, 1, |entry| {
        entry["operation"]["value"] == json!("admin_access_policy_write")
    })
    .await;
    assert_eq!(
        changes.len(),
        1,
        "the write on this replica changed the log's meaning: {changes:?}"
    );
    assert_eq!(
        changes[0]["action"]["value"],
        json!("update"),
        "{}",
        changes[0]
    );
    assert!(changes[0].get("ehr_id").is_none(), "{}", changes[0]);
}
