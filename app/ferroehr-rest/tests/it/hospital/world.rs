// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The hospital world: the configuration of the controller's deployment, the
//! parties and clients it provisions, the patient's record and the policy,
//! each written through a public route.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Router;
use ferroehr::config::authz::AuthzConfig;
use ferroehr::config::binding::{PartyBindingConfig, ProviderId, Providers};
use ferroehr::config::scim::ScimConfig;
use ferroehr::party_binding::grants::operation::{GrantCeiling, Operation};
use ferroehr::party_binding::policy_mark::TERMINOLOGY;
use ferroehr::service::FerroEhrService;
use ferroehr::system_log::sender::AuditHandle;
use ferroehr::system_log::store::AuditStore;
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::access::authz::AuthzHandle;
use ferroehr_rest::extensions::management::Observability;
use ferrohealth_secret::secret_url::SecretUrl;
use ferrohealth_smart::config::{AuthSection, IssuerEntry};
use ferrohealth_smart::kind::IssuerKind;
use ferrohealth_smart::party::config::PartySection;
use ferrohealth_smart::profile::Preset;
use http::{HeaderMap, StatusCode, header};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::{self, BASE, EXT};

/// The hospital's Keycloak realm, the `persons` issuer of its staff.
pub(crate) const PERSONS: &str = "https://keycloak.sint-example.example/realms/hospital";
/// The hospital's own authorization server, the `machines` issuer of its
/// SCIM client and its system clients.
pub(crate) const MACHINES: &str = "https://machines.sint-example.example";
/// The patient portal, the `patients` issuer of the patient and her
/// representative.
pub(crate) const PATIENTS: &str = "https://portal.sint-example.example";
/// The audience of every token.
pub(crate) const AUDIENCE: &str = "ferroehr";
/// The identifier issuer of the staff's binding keys, which the hospital's
/// SCIM client provisions.
pub(crate) const LOGIN: &str = "urn:sint-example:keycloak";
/// The scope the SCIM clients' tokens carry.
pub(crate) const SCIM_SCOPE: &str = "ferroehr/scim";
/// The hospital's SCIM client, and the patient portal's.
pub(crate) const SCIM: &str = "scim-hospital";
pub(crate) const PORTAL_SCIM: &str = "scim-portal";
/// The terminology the hospital codes its roles in, which its SCIM client
/// sends as the values of a User's `roles` (RFC 7643 §4.1.2).
pub(crate) const ROLES: &str = "urn:sint-example:roles";
/// The header the default configuration reads the declared purpose from.
pub(crate) const PURPOSE: &str = "x-purpose-of-use";
/// The HL7 v3 `ActCode` confidentiality terminology: `R` restricted, `V`
/// very restricted.
pub(crate) const CONFIDENTIALITY: &str = "2.16.840.1.113883.5.25";
/// The HL7 v3 `PurposeOfUse` terminology.
pub(crate) const PURPOSE_OF_USE: &str = "2.16.840.1.113883.5.8";
/// The hospital's IT administrator, a member of staff whose Keycloak token
/// carries the administrator role.
pub(crate) const ROOT: &str = "root";
/// The identifier issuer of the providers' identifiers.
pub(crate) const PROVIDERS: &str = "urn:oid:2.999.7";
/// The identifier issuer of the lab analyser's device identifiers.
pub(crate) const DEVICES: &str = "urn:sint-example:devices";

/// The root archetypes of the record's compositions.
pub(crate) const ENCOUNTER: &str = "openEHR-EHR-COMPOSITION.encounter.v1";
pub(crate) const VITALS: &str = "openEHR-EHR-COMPOSITION.vital_signs.v1";
pub(crate) const MENTAL_HEALTH: &str = "openEHR-EHR-COMPOSITION.mental_health_note.v1";
/// The template the lab commits its results in.
pub(crate) const LAB_TEMPLATE: &str = "International Patient Summary";

/// One member of staff: the SCIM `userName`, the binding key, the roles
/// their token carries for the coarse RBAC gate, the code of the role SCIM
/// assigns them in the CDR (a `ROLE` whose `name` it is, #5621), the SCIM
/// Groups they join, and the clearances and purposes the controller's rules
/// give that role.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Staff {
    pub(crate) name: &'static str,
    pub(crate) display: &'static str,
    pub(crate) roles: &'static [&'static str],
    pub(crate) role: &'static str,
    pub(crate) groups: &'static [&'static str],
    pub(crate) clearances: &'static [&'static str],
    pub(crate) purposes: &'static [&'static str],
}

/// The staff of the scenario.
pub(crate) const STAFF: [Staff; 7] = [
    Staff {
        name: ROOT,
        display: "IT administrator",
        roles: &["USER", "ADMIN"],
        role: "it-administrator",
        groups: &[],
        clearances: &[],
        purposes: &["HOPERAT"],
    },
    Staff {
        name: "jansen",
        display: "Dr. Anna Jansen",
        roles: &["USER"],
        role: "cardiologist",
        groups: &["Cardiology"],
        clearances: &["R"],
        purposes: &["TREAT"],
    },
    Staff {
        name: "devries",
        display: "Bram de Vries",
        roles: &["USER"],
        role: "nurse",
        groups: &["Ward 3"],
        clearances: &[],
        purposes: &["TREAT"],
    },
    Staff {
        name: "smit",
        display: "Dr. Carla Smit",
        roles: &["USER"],
        role: "psychiatrist",
        groups: &["Psychiatry"],
        clearances: &["R", "V"],
        purposes: &["TREAT"],
    },
    Staff {
        name: "bakker",
        display: "Dr. Dirk Bakker",
        roles: &["USER", "EMERGENCY"],
        role: "emergency-physician",
        groups: &[],
        clearances: &[],
        purposes: &["TREAT", "ETREAT"],
    },
    Staff {
        name: "gina",
        display: "Gina Mulder",
        roles: &["USER"],
        role: "nurse",
        groups: &["Ward 3"],
        clearances: &[],
        purposes: &["TREAT"],
    },
    Staff {
        name: "hans",
        display: "Hans, privacy officer",
        roles: &["USER", "ADMIN"],
        role: "privacy-officer",
        groups: &[],
        clearances: &[],
        purposes: &["HOPERAT"],
    },
];

/// The care teams, each a SCIM Group the patient is under the care of.
pub(crate) const TEAMS: [&str; 3] = ["Cardiology", "Ward 3", "Psychiatry"];

/// What SCIM provisioned for one person: the User id, which is the
/// `PERSON`'s container.
#[derive(Debug, Clone)]
pub(crate) struct Provisioned {
    pub(crate) person: String,
}

/// One composition of the record: its versioned object and its version.
#[derive(Debug, Clone, Default)]
pub(crate) struct Doc {
    pub(crate) vo: String,
    pub(crate) version: String,
}

/// The assembled server and the ids of what the scenario wrote.
pub(crate) struct World {
    pub(crate) _db: testkit::TestDb,
    pub(crate) _drain: AuditHandle,
    pub(crate) pool: PgPool,
    pub(crate) app: Router,
    pub(crate) config: AppConfig,
    /// Each member of staff and the representative, by name.
    pub(crate) people: BTreeMap<&'static str, Provisioned>,
    /// Each system client's `AGENT`, by client id.
    pub(crate) agents: BTreeMap<&'static str, String>,
    /// Each care team's `GROUP`, by name.
    pub(crate) teams: BTreeMap<&'static str, String>,
    /// The regional `GROUP` the member provider belongs to.
    pub(crate) region: String,
    /// The patient's `PERSON` and her EHR.
    pub(crate) eva: String,
    pub(crate) ehr: String,
    /// Each of her care relationships, by team: its container and current
    /// version.
    pub(crate) stays: BTreeMap<&'static str, (String, String)>,
    /// Gina's own record, another patient's EHR.
    pub(crate) other_ehr: String,
    /// The compositions of the patient's record.
    pub(crate) encounter: Doc,
    pub(crate) vitals: Doc,
    pub(crate) mental: Doc,
    pub(crate) hiv: Doc,
    /// The contribution that committed the encounter.
    pub(crate) contribution: String,
    /// The current version of the controller's policy.
    pub(crate) policy: String,
    /// The current version of Eva's `EHR_ACCESS`.
    pub(crate) access: String,
}

/// A `DV_TEXT` of `value`.
pub(crate) fn text(value: &str) -> Value {
    json!({ "_type": "DV_TEXT", "value": value })
}

/// A `DV_CODED_TEXT` of `code` in `terminology`.
pub(crate) fn coded_in(terminology: &str, code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": terminology },
                "code_string": code } })
}

/// A code of the access control groups: an HL7 code where the group reuses
/// one (#5655), else a code of the draft `openehr_access` terminology.
pub(crate) fn term(code: &str) -> Value {
    use openehr_rm::draft::access_control::terminology::{
        TERMINOLOGY_ID_CONSENT_PROVISION_TYPE, TERMINOLOGY_ID_ROLE_CLASS,
        TERMINOLOGY_ID_SECURITY_ROLE_TYPE,
    };
    match code {
        "permit" | "deny" => coded_in(TERMINOLOGY_ID_CONSENT_PROVISION_TYPE, code),
        "MBR" => coded_in(TERMINOLOGY_ID_ROLE_CLASS, code),
        "datasubject" => coded_in(TERMINOLOGY_ID_SECURITY_ROLE_TYPE, code),
        _ => coded_in(TERMINOLOGY, code),
    }
}

/// A `PARTY_REF` of `kind` to `id`.
pub(crate) fn party_ref(kind: &str, id: &str) -> Value {
    json!({ "_type": "PARTY_REF", "namespace": "demographic", "type": kind,
            "id": { "_type": "HIER_OBJECT_ID", "value": id } })
}

/// The `ARCHETYPED` of the deployment's own `archetype`, with no template.
fn archetyped(archetype: &str) -> Value {
    json!({ "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
            "rm_version": "1.1.0" })
}

/// A name identity of `name`.
pub(crate) fn identities(name: &str) -> Value {
    json!([{ "_type": "PARTY_IDENTITY", "archetype_node_id": "at0001", "name": text("name"),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0002", "name": text("details"),
            "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0003", "name": text("name"),
                        "value": text(name) }] } }])
}

/// A credentials element named by the terminology's `code`, holding `value`.
pub(crate) fn named(code: &str, value: &Value) -> Value {
    json!({ "_type": "ELEMENT", "archetype_node_id": "at0030", "name": term(code),
            "value": value })
}

/// A capability granting `operation`, with the further credentials `items`.
pub(crate) fn capability(operation: &str, items: &[Value]) -> Value {
    let mut all = vec![json!({ "_type": "ELEMENT", "archetype_node_id": "at0021",
        "name": text("operation"), "value": term(operation) })];
    all.extend_from_slice(items);
    json!({
        "_type": "CAPABILITY", "archetype_node_id": "at0020", "name": text("grant"),
        "credentials": { "_type": "ITEM_TREE", "archetype_node_id": "at0022",
                         "name": text("terms"), "items": all }
    })
}

/// The deployment's configuration: the three issuers, the staff's logins
/// bound by the keys SCIM provisions, the patient portal's `patient/`
/// grants honoured, the break-glass role and the emergency purpose, and
/// `root` the first administrator.
pub(crate) fn config() -> AppConfig {
    let machines = IssuerEntry {
        issuer: SecretUrl::new(MACHINES),
        kind: Some(IssuerKind::Machines),
        profile: Preset::Rfc9068,
        audience: vec![AUDIENCE.to_owned()],
        jwks_file: common::jwks_file().display().to_string(),
        // NOTE: the test key signs RFC 9068 `at+jwt` tokens.
        token_types: Some(vec!["at+jwt".to_owned()]),
        ..IssuerEntry::default()
    };
    let persons = IssuerEntry {
        jwks_file: common::jwks_file().display().to_string(),
        ..common::entry(PERSONS, AUDIENCE)
    };
    let patients = IssuerEntry {
        kind: Some(IssuerKind::Patients),
        jwks_file: common::jwks_file().display().to_string(),
        ..common::entry(PATIENTS, AUDIENCE)
    };
    let staff = PartySection {
        issuer: SecretUrl::new(PERSONS),
        identifier_issuer: LOGIN.to_owned(),
        identifier_type: "login".to_owned(),
        group_claim: "groups".to_owned(),
        group_identifier_issuer: format!("{LOGIN}/groups"),
        ..PartySection::default()
    };
    let mut auth = common::auth(AuthSection {
        issuer: vec![machines, persons, patients],
        party: vec![staff],
        ..AuthSection::default()
    });
    auth.shared.enabled = true;
    let mut config = common::api_config(true);
    config.auth = auth;
    config.auth.scim = Some(ScimConfig::default());
    config.auth.smart.patient_issuers = vec![SecretUrl::new(PATIENTS)];
    config.public_root = Some(
        ferroehr::config::server::PublicRoot::new("https://cdr.sint-example.example", "")
            .expect("a public root"),
    );
    config.party_binding = PartyBindingConfig {
        provider: Some(Providers::One(ProviderId {
            issuer: PROVIDERS.to_owned(),
            id: "sint-example".to_owned(),
        })),
        ..PartyBindingConfig::default()
    };
    config.rbac.role_issuers = vec![SecretUrl::new(PERSONS)];
    config.rbac.emergency_access_role = Some("EMERGENCY".to_owned());
    config
}

/// A service over `pool` as `serve` installs it for `config`: the default
/// definitions seeded, the mapping installed, every operation honoured,
/// auditing to the local store through `audit` when given.
pub(crate) async fn replica(
    config: &AppConfig,
    pool: &PgPool,
    audit: Option<ferroehr::system_log::sender::AuditSender>,
) -> FerroEhrService {
    FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(pool))
        .seed_default_definitions()
        .await
        .expect("the boot seeds the default definitions");
    let sections = config.party_binding.sections();
    let mapping = sections
        .binding_mapping(&[LOGIN.to_owned(), PATIENTS.to_owned()], "ADMIN")
        .expect("the mapping compiles");
    let mut svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(pool))
        .with_mapped_party_templates(sections.mapped_templates())
        .with_binding_mapping(mapping)
        .with_grant_ceiling(GrantCeiling::new(Operation::ALL));
    if let Some(sender) = audit {
        svc = svc
            .with_audit(sender)
            .with_audit_store(AuditStore::new(pool.clone()));
    }
    let errors = svc
        .party_mapping_errors(&sections)
        .await
        .expect("the store answers");
    assert!(errors.is_empty(), "boot refuses the mapping: {errors:?}");
    svc.take_binding_lease(uuid::Uuid::now_v7())
        .await
        .expect("the side table is prepared");
    svc
}

/// The router `serve` builds over `svc` for `config`, with the RBAC gate on.
pub(crate) async fn router(config: &AppConfig, svc: &Arc<FerroEhrService>) -> Router {
    let mut authz = AuthzConfig {
        rbac: config.rbac.clone(),
        ..AuthzConfig::default()
    };
    authz.rbac.enabled = true;
    let handle = AuthzHandle::build(&authz, &config.api.base_path).map(Arc::new);
    common::build_full(
        config.clone(),
        Arc::clone(svc),
        handle,
        Observability::default(),
    )
    .await
}

// ── credentials ───────────────────────────────────────────────────────────────

/// The token's expiry, an hour from now.
fn exp() -> i64 {
    jiff::Timestamp::now().as_second() + 3600
}

/// The credential of the caller `name`: a member of staff's Keycloak token
/// (any other name a new member of staff's, holding `USER`),
/// the patient's or her representative's portal token naming her EHR
/// (`eva:<ehr>`, `frank:<ehr>`), or a system client's app-only token
/// (`client:<id>`).
pub(crate) fn credential(name: &str) -> String {
    if let Some(client) = name.strip_prefix("client:") {
        return common::bearer(&json!({
            "iss": MACHINES, "aud": AUDIENCE, "exp": exp(), "sub": client,
            "client_id": client,
            "scope": "system/composition-*.cruds system/aql-*.s system/template-*.r",
        }));
    }
    if let Some((who, ehr)) = name.split_once(':') {
        return common::bearer(&json!({
            "iss": PATIENTS, "aud": AUDIENCE, "exp": exp(), "sub": format!("{who}-login"),
            "ehrId": ehr, "scope": "patient/composition-*.cruds patient/aql-*.s",
            "roles": ["PATIENT"],
        }));
    }
    let (roles, groups): (&[&str], &[&str]) = STAFF
        .iter()
        .find(|staff| staff.name == name)
        .map_or((&["USER"], &[]), |staff| (staff.roles, staff.groups));
    let groups: Vec<String> = groups.iter().map(|team| external_id(team)).collect();
    common::bearer(&json!({
        "iss": PERSONS, "aud": AUDIENCE, "exp": exp(), "sub": name, "roles": roles,
        "groups": groups,
    }))
}

/// The `externalId` of the SCIM Group of `team`, which the staff's tokens
/// name in their `groups` claim.
pub(crate) fn external_id(team: &str) -> String {
    team.to_lowercase().replace(' ', "-")
}

// ── requests ──────────────────────────────────────────────────────────────────

/// What one request answers.
#[derive(Debug)]
pub(crate) struct Answer {
    pub(crate) status: StatusCode,
    pub(crate) headers: HeaderMap,
    pub(crate) body: String,
}

impl Answer {
    /// The body as JSON, `null` when it is none.
    pub(crate) fn json(&self) -> Value {
        // NOTE: our own test design, a body that is no JSON reads as null.
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }

    /// The `refusal` code of a problem body, `null` where it names none.
    pub(crate) fn refusal(&self) -> Value {
        self.json()["refusal"].clone()
    }

    /// The uid in the weak `ETag`.
    pub(crate) fn etag(&self) -> String {
        common::etag_uid(&self.headers)
    }

    /// The request id the server answered under.
    pub(crate) fn request_id(&self) -> String {
        common::header(&self.headers, "x-request-id")
            .expect("an x-request-id")
            .to_owned()
    }
}

/// One request of the scenario: who asks, what, under which purpose, with
/// which JSON body, scoped to which EHR (`openEHR-EHR-id`).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Ask<'a> {
    pub(crate) purpose: Option<&'a str>,
    pub(crate) body: Option<&'a Value>,
    pub(crate) scope: Option<&'a str>,
}

/// `method uri` as `caller`, as `ask` says, asking for the representation.
pub(crate) async fn ask(
    app: &Router,
    caller: &str,
    (method, uri): (&str, &str),
    ask: Ask<'_>,
) -> Answer {
    let mut req = common::req(method, uri)
        .auth(credential(caller))
        .accept("application/json")
        .prefer_representation();
    if let Some(purpose) = ask.purpose {
        req = req.header(PURPOSE, purpose);
    }
    if let Some(ehr) = ask.scope {
        req = req.header("openehr-ehr-id", ehr);
    }
    if let Some(body) = ask.body {
        req = req.json(body.to_string());
    }
    let (status, headers, body) = served(app, req.build()).await;
    Answer {
        status,
        headers,
        body,
    }
}

/// `req` answered by `app` on a task of its own, as `serve` polls each
/// request at the root of its connection's task: nested in the world's
/// setup, a debug build's stack held the setup's frames under every request
/// and overflowed in a composition's commit.
pub(crate) async fn served(
    app: &Router,
    req: http::Request<axum::body::Body>,
) -> (StatusCode, HeaderMap, String) {
    let app = app.clone();
    tokio::spawn(async move { common::send(&app, req).await })
        .await
        .expect("the request's task")
}

/// `method uri` as `caller`, declaring `purpose`, with a JSON `body`.
pub(crate) async fn call(
    app: &Router,
    caller: &str,
    method: &str,
    uri: &str,
    purpose: Option<&str>,
    body: Option<&Value>,
) -> Answer {
    let asked = Ask {
        purpose,
        body,
        scope: None,
    };
    ask(app, caller, (method, uri), asked).await
}

/// [`call`] with an `If-Match` of `version`.
pub(crate) async fn call_matching(
    app: &Router,
    caller: &str,
    (method, uri): (&str, &str),
    version: &str,
    body: &Value,
) -> Answer {
    let req = common::req(method, uri)
        .auth(credential(caller))
        .accept("application/json")
        .prefer_representation()
        .if_match(format!("\"{version}\""))
        .json(body.to_string());
    let (status, headers, body) = served(app, req.build()).await;
    Answer {
        status,
        headers,
        body,
    }
}

/// `method path` under `/scim/v2` as the SCIM client `client`, with `body`.
pub(crate) async fn scim(
    app: &Router,
    client: &str,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> (StatusCode, Value) {
    let token = common::bearer(&json!({
        "iss": MACHINES, "aud": AUDIENCE, "exp": exp(), "sub": client, "client_id": client,
        "scope": SCIM_SCOPE,
    }));
    let mut req = common::req(method, format!("/scim/v2{path}")).auth(&token);
    if let Some(body) = body {
        req = req
            .header(header::CONTENT_TYPE, "application/scim+json")
            .body(body.to_string());
    }
    let (status, _, body) = served(app, req.build()).await;
    (status, serde_json::from_str(&body).unwrap_or(Value::Null))
}

/// A SCIM `PatchOp` message of `operations` (RFC 7644 §3.5.2).
pub(crate) fn patch_op(operations: Value) -> Value {
    let mut message = json!({ "schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"] });
    message["Operations"] = operations;
    message
}

// ── the demographic API ───────────────────────────────────────────────────────

/// Creates `body` as a party of `kind` (the route segment) as `caller`; its
/// container and version.
pub(crate) async fn create_party(
    app: &Router,
    caller: &str,
    kind: &str,
    body: &Value,
) -> (String, String) {
    let answer = call(
        app,
        caller,
        "POST",
        &format!("{BASE}/demographic/{kind}"),
        None,
        Some(body),
    )
    .await;
    assert_eq!(
        answer.status,
        StatusCode::CREATED,
        "{caller} creates the {kind}: {}",
        answer.body
    );
    let version = answer.etag();
    (vo_of(&version), version)
}

/// The container of the version `uid`.
pub(crate) fn vo_of(uid: &str) -> String {
    uid.split("::").next().expect("a container").to_owned()
}

/// The current version and body of the party of `kind` `id`, as `root`
/// reads it.
pub(crate) async fn read_party(app: &Router, kind: &str, id: &str) -> (String, Value) {
    let answer = call(
        app,
        ROOT,
        "GET",
        &format!("{BASE}/demographic/{kind}/{id}"),
        None,
        None,
    )
    .await;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "{kind} {id}: {}",
        answer.body
    );
    (answer.etag(), answer.json())
}

/// Commits `body` as the next version of the party of `kind` `id` after
/// `version`, as `caller`; the answer.
pub(crate) async fn update_party(
    app: &Router,
    caller: &str,
    (kind, id, version): (&str, &str, &str),
    body: &Value,
) -> Answer {
    call_matching(
        app,
        caller,
        ("PUT", &format!("{BASE}/demographic/{kind}/{id}")),
        version,
        body,
    )
    .await
}

/// An `AGENT` carrying the client identifier `client` under
/// [`MACHINES`], acting for the `ORGANISATION` `provider` when given, and a
/// `ROLE` it performs holding `capabilities`, created by the administrator
/// `admin`; the `AGENT`.
pub(crate) async fn client(
    app: &Router,
    admin: &str,
    (client, provider): (&str, Option<&str>),
    capabilities: &[Value],
) -> String {
    let archetype = "openEHR-DEMOGRAPHIC-AGENT.integration_engine.v2";
    let mut agent = json!({
        "_type": "AGENT", "archetype_node_id": archetype,
        "archetype_details": archetyped(archetype), "name": text(client),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0010", "name": text("details"),
            "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0012", "name": text("client"),
                "value": { "_type": "DV_IDENTIFIER", "issuer": MACHINES, "id": client,
                           "type": "client_id" } }] },
        "identities": identities(client)
    });
    if let Some(provider) = provider {
        agent["relationships"] = json!([{
            "_type": "PARTY_RELATIONSHIP", "archetype_node_id": "at0040", "name": term("acts_for"),
            "source": party_ref("AGENT", "00000000-0000-4000-8000-000000000000"),
            "target": party_ref("ORGANISATION", provider)
        }]);
    }
    let id = if provider.is_some() {
        let mut bare = agent.clone();
        bare.as_object_mut()
            .expect("an object")
            .remove("relationships");
        let (id, version) = create_party(app, admin, "agent", &bare).await;
        agent["relationships"][0]["source"] = party_ref("AGENT", &id);
        let answer = update_party(app, admin, ("agent", &id, &version), &agent).await;
        assert_eq!(answer.status, StatusCode::OK, "acts for: {}", answer.body);
        id
    } else {
        create_party(app, admin, "agent", &agent).await.0
    };
    let archetype = "openEHR-DEMOGRAPHIC-ROLE.integration.v1";
    let role = json!({
        "_type": "ROLE", "archetype_node_id": archetype,
        "archetype_details": archetyped(archetype), "name": text("Integration"),
        "identities": identities("integration"),
        "performer": party_ref("AGENT", &id),
        "capabilities": capabilities
    });
    create_party(app, admin, "role", &role).await;
    id
}

/// A `GROUP` of the deployment's own archetype named `name`, created by
/// `root`.
pub(crate) async fn region(app: &Router, name: &str) -> String {
    let archetype = "openEHR-DEMOGRAPHIC-GROUP.region.v1";
    let body = json!({
        "_type": "GROUP", "archetype_node_id": archetype,
        "archetype_details": archetyped(archetype), "name": text(name),
        "identities": identities(name)
    });
    create_party(app, ROOT, "group", &body).await.0
}

/// A provider `ORGANISATION` identified by `id` under [`PROVIDERS`], a
/// member of each `GROUP` of `groups`, written by `root`: a membership of a
/// `GROUP` that belongs to no provider is an administrator's to write.
pub(crate) async fn provider(app: &Router, id: &str, groups: &[&str]) -> String {
    let archetype = "openEHR-DEMOGRAPHIC-ORGANISATION.hospital.v1";
    let mut organisation = json!({
        "_type": "ORGANISATION", "archetype_node_id": archetype,
        "archetype_details": archetyped(archetype), "name": text(id),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0001", "name": text("details"),
            "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0002", "name": text("provider"),
                "value": { "_type": "DV_IDENTIFIER", "issuer": PROVIDERS, "id": id,
                           "type": "healthcare-provider" } }] },
        "identities": identities(id)
    });
    let (organisation_id, version) = create_party(app, ROOT, "organisation", &organisation).await;
    if !groups.is_empty() {
        let memberships: Vec<Value> = groups
            .iter()
            .map(|group| {
                json!({ "_type": "PARTY_RELATIONSHIP", "archetype_node_id": "at0050",
                        "name": term("MBR"),
                        "source": party_ref("ORGANISATION", &organisation_id),
                        "target": party_ref("GROUP", group) })
            })
            .collect();
        organisation["relationships"] = Value::Array(memberships);
        let answer = update_party(
            app,
            ROOT,
            ("organisation", &organisation_id, &version),
            &organisation,
        )
        .await;
        assert_eq!(answer.status, StatusCode::OK, "membership: {}", answer.body);
    }
    organisation_id
}

/// A membership (`MBR`) of the `PERSON` `source` in the `GROUP` `team`,
/// from `from` to `until` (dates, each included; `None` unbounded).
pub(crate) fn membership(source: &str, team: &str, span: (&str, Option<&str>)) -> Value {
    json!({
        "_type": "PARTY_RELATIONSHIP",
        "archetype_node_id": "org.sint-example::openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.rostering.v1",
        "name": text("Rostering"),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0002", "name": text("details"),
            "items": [{ "_type": "ELEMENT", "archetype_node_id": "at0003", "name": text("kind"),
                        "value": term("MBR") }] },
        "time_validity": validity(span),
        "source": party_ref("PERSON", source),
        "target": party_ref("GROUP", team)
    })
}

/// The `DV_INTERVAL<DV_DATE>` from `from` to `until` (each included; `None`
/// unbounded).
fn validity((from, until): (&str, Option<&str>)) -> Value {
    let mut validity = json!({ "_type": "DV_INTERVAL",
        "lower": { "_type": "DV_DATE", "value": from },
        "lower_unbounded": false, "upper_unbounded": until.is_none(),
        "lower_included": true, "upper_included": until.is_some() });
    if let Some(until) = until {
        validity["upper"] = json!({ "_type": "DV_DATE", "value": until });
    }
    validity
}

/// A care relationship of its own: the `PERSON` `source` under the care of
/// the `GROUP` `team` in the `care_role` `role` of `HL7_ParticipationType`,
/// in the care of the hospital, from `from` to `until` (dates, each
/// included; `None` unbounded); draft access control package, Care
/// Relationships.
pub(crate) fn care(role: &str, source: &str, team: &str, span: (&str, Option<&str>)) -> Value {
    let named = |code: &str| {
        json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" },
                "code_string": code } })
    };
    json!({
        "_type": "PARTY_RELATIONSHIP",
        "archetype_node_id": "org.sint-example::openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.ward_stay.v1",
        "name": named("under_care_of"),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0002", "name": text("details"),
            "items": [
                { "_type": "ELEMENT", "archetype_node_id": "at0003", "name": named("care_role"),
                  "value": { "_type": "DV_CODED_TEXT", "value": role,
                    "defining_code": { "_type": "CODE_PHRASE",
                        "terminology_id": { "_type": "TERMINOLOGY_ID",
                            "value": "HL7_ParticipationType" },
                        "code_string": role } } },
                { "_type": "ELEMENT", "archetype_node_id": "at0004", "name": named("provider"),
                  "value": { "_type": "DV_IDENTIFIER", "issuer": PROVIDERS, "id": "sint-example",
                             "type": "ORGANISATION" } }] },
        "time_validity": validity(span),
        "source": party_ref("PERSON", source),
        "target": party_ref("GROUP", team)
    })
}

/// The patient's `PERSON` holding `relationships`.
pub(crate) fn patient_body(relationships: &[Value]) -> Value {
    let archetype = "openEHR-DEMOGRAPHIC-PERSON.person.v1";
    let mut body = json!({
        "_type": "PERSON", "archetype_node_id": archetype,
        "archetype_details": archetyped(archetype),
        "name": text("Eva Visser"),
        "identities": identities("Eva Visser")
    });
    if !relationships.is_empty() {
        body["relationships"] = Value::Array(relationships.to_vec());
    }
    body
}

/// Today in UTC, moved by `days`, as `YYYY-MM-DD`.
pub(crate) fn day(days: i64) -> String {
    jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date()
        .checked_add(jiff::Span::new().days(days))
        .expect("a date")
        .to_string()
}

/// The ADT system writes the patient's stays, each a team she is under the
/// care of from a date to a date (`None` open): a care relationship of its
/// own per team, created at `/demographic/party_relationship` the first time
/// and changed over its version after.
pub(crate) async fn admit(world: &mut World, stays: &[(&'static str, &str, Option<&str>)]) {
    for (team, from, until) in stays {
        let body = care("PART", &world.eva, &world.teams[team], (from, *until));
        let uri = format!("{EXT}/demographic/party_relationship");
        let answer = match world.stays.get(team) {
            Some((id, version)) => {
                call_matching(
                    &world.app,
                    "client:adt",
                    ("PUT", &format!("{uri}/{id}")),
                    version,
                    &body,
                )
                .await
            }
            None => call(&world.app, "client:adt", "POST", &uri, None, Some(&body)).await,
        };
        assert!(
            matches!(answer.status, StatusCode::OK | StatusCode::CREATED),
            "admission to {team}: {}",
            answer.body
        );
        let version = answer.etag();
        world.stays.insert(team, (vo_of(&version), version));
    }
}

/// `writer` writes the rostering of `name` on `teams` from `from` to
/// `until`: an `MBR` membership per team on their `PERSON`, beside
/// the memberships SCIM keeps there; the answer.
pub(crate) async fn roster(
    world: &World,
    writer: &str,
    name: &str,
    teams: &[&str],
    span: (&str, Option<&str>),
) -> Answer {
    let person = &world.people[name].person;
    let (version, mut body) = read_party(&world.app, "person", person).await;
    let mut relationships: Vec<Value> = body["relationships"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|relationship| relationship["details"]["items"][0]["value"] != term("MBR"))
        .collect();
    relationships.extend(
        teams
            .iter()
            .map(|team| membership(person, &world.teams[team], span)),
    );
    body["relationships"] = Value::Array(relationships);
    update_party(&world.app, writer, ("person", person, &version), &body).await
}
