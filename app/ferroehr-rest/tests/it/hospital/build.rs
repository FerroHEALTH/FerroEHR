// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Builds the hospital world through its public routes: SCIM for the staff,
//! their care teams and the patient's representative, the Demographic API for
//! the clients, the providers, the professionals' clearances and the care
//! relationships, ITS-REST for the record and its `EHR_ACCESS`, and the admin
//! API for the controller's `ACCESS_POLICY`.

use std::collections::BTreeMap;
use std::sync::Arc;

use ferroehr::system_log::config::AuditConfig;
use ferroehr::system_log::sender::start;
use http::StatusCode;
use serde_json::{Value, json};

use crate::common::{self, BASE, EXT};
use crate::hospital::world::{
    CONFIDENTIALITY, DEVICES, Doc, ENCOUNTER, LAB_TEMPLATE, LOGIN, MENTAL_HEALTH, OPERATOR,
    PATIENTS, PORTAL_SCIM, PROVIDERS, Provisioned, ROOT, SCIM, SCIM_SCOPE, STAFF, Staff, TEAMS,
    VITALS, World, admit, call, capability, client, coded_in, config, create_party, day, named,
    party_ref, patient_body, provider, read_party, region, replica, roster, router, scim, term,
    text, update_party,
};

/// The extension FerroEHR's User attributes sit in.
const EXTENSION: &str = "urn:ietf:params:scim:schemas:extension:ferrohealth:2.0:User";

/// The world: every party, the record, the policy and the directives.
pub(crate) async fn world() -> World {
    let config = config();
    let (db, pool) = common::migrated_pool().await;
    let audit = AuditConfig {
        purpose_codes: vec![
            "TREAT".to_owned(),
            "ETREAT".to_owned(),
            "HOPERAT".to_owned(),
        ],
        emergency_purpose_codes: vec!["ETREAT".to_owned()],
        ..common::local_audit()
    };
    let (sender, drain) = start(audit, None, Some(pool.clone()))
        .await
        .expect("the audit sender");
    let svc = Arc::new(replica(&config, &pool, Some(sender)).await);
    let app = router(&config, &svc).await;
    client(&app, OPERATOR, (SCIM, None), &[scim_grant(LOGIN)]).await;
    let mut people = BTreeMap::new();
    people.insert(ROOT, user(&app, SCIM, ROOT, "IT administrator").await);
    clients(&app).await;
    for staff in STAFF.iter().filter(|staff| staff.name != ROOT) {
        people.insert(
            staff.name,
            user(&app, SCIM, staff.name, staff.display).await,
        );
    }
    people.insert(
        "frank",
        user(&app, PORTAL_SCIM, "frank-login", "Frank Visser").await,
    );
    let mut teams = BTreeMap::new();
    for team in TEAMS {
        let members: Vec<&str> = STAFF
            .iter()
            .filter(|staff| staff.groups.contains(&team))
            .map(|staff| people[staff.name].person.as_str())
            .collect();
        teams.insert(team, group(&app, team, &members).await);
    }
    for staff in STAFF {
        clear(&app, &people[staff.name], &staff).await;
    }
    let region = region(&app, "Regional health network").await;
    let hospital = provider(&app, "sint-example", &[]).await;
    let member = provider(&app, "member-clinic", &[&region]).await;
    let outside = provider(&app, "outside-clinic", &[]).await;
    let lab = provider(&app, "lab", &[]).await;
    system_clients(&app, (&member, &outside, &lab)).await;
    let (eva, eva_version) = create_party(&app, ROOT, "person", &patient_body(&[])).await;
    let ehr = ehr_of(&app, &eva).await;
    let other_ehr = ehr_of(&app, &people["gina"].person).await;
    let mut world = World {
        _db: db,
        _drain: drain,
        pool,
        svc,
        app,
        config,
        people,
        teams,
        region,
        eva,
        eva_version,
        ehr,
        other_ehr,
        encounter: Doc::default(),
        vitals: Doc::default(),
        mental: Doc::default(),
        hiv: Doc::default(),
        contribution: String::new(),
        policy: String::new(),
        access: String::new(),
    };
    content(&mut world).await;
    represent(&world).await;
    let admitted = day(-2);
    let stays: Vec<(&str, &str, Option<&str>)> = TEAMS
        .iter()
        .map(|team| (*team, admitted.as_str(), None))
        .collect();
    world.eva_version = admit(&world, &world.eva_version, &stays).await;
    for staff in STAFF.iter().filter(|staff| !staff.groups.is_empty()) {
        let answer = roster(
            &world,
            "client:adt",
            staff.name,
            staff.groups,
            (&day(-30), None),
        )
        .await;
        assert_eq!(
            answer.status,
            StatusCode::OK,
            "{}'s roster: {}",
            staff.name,
            answer.body
        );
    }
    world.policy = policy(&world.app, &hospital, None).await;
    directives(&mut world, None).await;
    world
}

/// The SCIM User `user_name` of `client`, keyed by its own name, and the
/// professional `ROLE` it performs.
async fn user(app: &axum::Router, client: &str, user_name: &str, display: &str) -> Provisioned {
    let body = json!({
        "schemas": ["urn:ietf:params:scim:schemas:core:2.0:User", EXTENSION],
        "userName": user_name,
        "externalId": user_name,
        "name": { "formatted": display },
        EXTENSION: { "loginIdentifier": user_name }
    });
    let (status, created) = scim(app, client, "POST", "/Users", Some(&body)).await;
    assert_eq!(status, StatusCode::CREATED, "{user_name}: {created}");
    let person = created["id"].as_str().expect("an id").to_owned();
    let (_, read) = read_party(app, "person", &person).await;
    let role = read["roles"][0]["id"]["value"]
        .as_str()
        .unwrap_or_else(|| panic!("{user_name} performs a ROLE: {read}"))
        .to_owned();
    Provisioned { person, role }
}

/// The SCIM Group `name` of the hospital's client with `members`; its id.
async fn group(app: &axum::Router, name: &str, members: &[&str]) -> String {
    let body = json!({
        "schemas": ["urn:ietf:params:scim:schemas:core:2.0:Group"],
        "displayName": name,
        "externalId": name.to_lowercase().replace(' ', "-"),
        "members": members.iter().map(|id| json!({ "value": id })).collect::<Vec<_>>(),
    });
    let (status, created) = scim(app, SCIM, "POST", "/Groups", Some(&body)).await;
    assert_eq!(status, StatusCode::CREATED, "{name}: {created}");
    created["id"].as_str().expect("an id").to_owned()
}

/// The capability carrying the clearances and the permitted purposes of
/// `staff`.
pub(crate) fn facts(staff: &Staff) -> Value {
    let mut items: Vec<Value> = staff
        .clearances
        .iter()
        .map(|code| named("sensitivity_clearance", &coded_in(CONFIDENTIALITY, code)))
        .collect();
    items.extend(staff.purposes.iter().map(|code| {
        named(
            "permitted_purpose",
            &coded_in(crate::hospital::world::PURPOSE_OF_USE, code),
        )
    }));
    capability("access_demographic", &items)
}

/// The capabilities of `staff`: their facts, and for `root` the grant to
/// write the controller's policy.
fn privileges(staff: &Staff) -> Vec<Value> {
    let mut held = vec![facts(staff)];
    if staff.name == ROOT {
        held.push(capability("write_access_policy", &[]));
    }
    held
}

/// `root` adds the clearances, purposes and grants of `staff` to the
/// professional `ROLE` SCIM provisioned for them.
async fn clear(app: &axum::Router, provisioned: &Provisioned, staff: &Staff) {
    let (version, mut role) = read_party(app, "role", &provisioned.role).await;
    role["capabilities"] = Value::Array(privileges(staff));
    let answer = update_party(app, ROOT, ("role", &provisioned.role, &version), &role).await;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "{}'s ROLE: {}",
        staff.name,
        answer.body
    );
}

/// The permitted purpose of treatment.
fn treatment() -> Value {
    named(
        "permitted_purpose",
        &coded_in(crate::hospital::world::PURPOSE_OF_USE, "TREAT"),
    )
}

/// The grant to provision the Users whose binding keys `binding_issuer`
/// issues.
fn scim_grant(binding_issuer: &str) -> Value {
    capability(
        "scim_provision",
        &[
            named("binding_issuer", &text(binding_issuer)),
            named("token_scope", &text(SCIM_SCOPE)),
        ],
    )
}

/// The portal's SCIM client and the ADT system, granted by `root`.
async fn clients(app: &axum::Router) {
    client(app, ROOT, (PORTAL_SCIM, None), &[scim_grant(PATIENTS)]).await;
    client(
        app,
        ROOT,
        ("adt", None),
        &[
            capability("write_care_relationship", &[]),
            capability("access_demographic", &[]),
        ],
    )
    .await;
}

/// The lab system, the regional exchange's clients of a member provider and
/// of a provider outside the region, and the access service.
async fn system_clients(app: &axum::Router, (member, outside, lab): (&str, &str, &str)) {
    let query = capability("query_system_wide", &[treatment()]);
    client(app, ROOT, ("hie-member", Some(member)), &[query.clone()]).await;
    client(app, ROOT, ("hie-outside", Some(outside)), &[query.clone()]).await;
    client(
        app,
        ROOT,
        ("access-service", Some(member)),
        &[query.clone(), capability("access_service", &[treatment()])],
    )
    .await;
    let commits = capability(
        "commit_composition",
        &[
            named("template", &text(LAB_TEMPLATE)),
            named("origin", &term("device")),
            named("device_identifier_issuer", &text(DEVICES)),
            json!({ "_type": "CLUSTER", "archetype_node_id": "at0031", "name": term("facility"),
                    "items": [named("issuer", &text(PROVIDERS)), named("identifier", &text("lab"))] }),
            treatment(),
        ],
    );
    client(app, ROOT, ("lab", Some(lab)), &[commits, query]).await;
}

/// `root` creates the EHR whose subject is the `PERSON` `person`; its id.
async fn ehr_of(app: &axum::Router, person: &str) -> String {
    let status = json!({
        "_type": "EHR_STATUS",
        "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
        "archetype_details": { "_type": "ARCHETYPED", "rm_version": "1.1.0",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-EHR_STATUS.generic.v1" } },
        "name": text("EHR Status"),
        "subject": { "_type": "PARTY_SELF", "external_ref": party_ref("PERSON", person) },
        "is_queryable": true,
        "is_modifiable": true
    });
    let answer = call(
        app,
        ROOT,
        "POST",
        &format!("{BASE}/ehr"),
        None,
        Some(&status),
    )
    .await;
    assert_eq!(answer.status, StatusCode::CREATED, "EHR: {}", answer.body);
    answer.json()["ehr_id"]["value"]
        .as_str()
        .expect("an ehr_id")
        .to_owned()
}

/// A templateless COMPOSITION rooted at `archetype`, composed by `composer`.
pub(crate) fn composition(archetype: &str, name: &str, composer: &str) -> Value {
    json!({
        "_type": "COMPOSITION",
        "archetype_node_id": archetype,
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
            "rm_version": "1.2.0" },
        "name": text(name),
        "language": { "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" },
            "code_string": "nl" },
        "territory": { "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_3166-1" },
            "code_string": "NL" },
        "category": coded_in("openehr", "433"),
        "composer": { "_type": "PARTY_IDENTIFIED", "name": composer }
    })
}

/// The lab analyser's result: the IPS composition of [`LAB_TEMPLATE`],
/// composed by the device, at the lab.
pub(crate) fn lab_result() -> Value {
    let mut result = common::ips_canonical_composition();
    if let Some(object) = result.as_object_mut() {
        object.remove("uid");
    }
    result["composer"] = json!({ "_type": "PARTY_IDENTIFIED", "name": "Lab analyser",
        "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": DEVICES, "id": "analyser-1" }] });
    result["feeder_audit"] = json!({ "_type": "FEEDER_AUDIT",
        "originating_system_audit": { "_type": "FEEDER_AUDIT_DETAILS", "system_id": "analyser-1" } });
    result["context"]["health_care_facility"] = json!({ "_type": "PARTY_IDENTIFIED",
        "name": "Lab",
        "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": PROVIDERS, "id": "lab" }] });
    result
}

/// Commits `body` into the world's EHR as `caller`; the document and the
/// answer's headers.
pub(crate) async fn commit(world: &World, caller: &str, body: &Value) -> Doc {
    let answer = call(
        &world.app,
        caller,
        "POST",
        &format!("{BASE}/ehr/{}/composition", world.ehr),
        Some("TREAT"),
        Some(body),
    )
    .await;
    assert_eq!(
        answer.status,
        StatusCode::CREATED,
        "{caller} commits: {}",
        answer.body
    );
    let version = answer.etag();
    Doc {
        vo: crate::hospital::world::vo_of(&version),
        version,
    }
}

/// The record: the cardiology encounter, the vital signs, the mental-health
/// note and the lab's HIV result, each committed by its author before the
/// policy holds them to a care relationship.
async fn content(world: &mut World) {
    let (status, body) = common::send_body(
        &world.app,
        common::req("POST", format!("{BASE}/definition/template/adl1.4"))
            .auth(crate::hospital::world::credential(ROOT))
            .header(http::header::CONTENT_TYPE, "application/xml")
            .body(common::ips_opt_xml())
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "the lab's template: {body}");
    world.encounter = commit(
        world,
        "jansen",
        &composition(ENCOUNTER, "Cardiology encounter", "Dr. Anna Jansen"),
    )
    .await;
    world.vitals = commit(
        world,
        "devries",
        &composition(VITALS, "Vital signs", "Bram de Vries"),
    )
    .await;
    world.mental = commit(
        world,
        "smit",
        &composition(MENTAL_HEALTH, "Mental health note", "Dr. Carla Smit"),
    )
    .await;
    world.hiv = commit(world, "client:lab", &lab_result()).await;
    let answer = call(
        &world.app,
        ROOT,
        "GET",
        &format!(
            "{BASE}/ehr/{}/versioned_composition/{}/version/{}",
            world.ehr, world.vitals.vo, world.vitals.version
        ),
        None,
        None,
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    world.contribution = answer.json()["contribution"]["id"]["value"]
        .as_str()
        .unwrap_or_else(|| panic!("a contribution: {}", answer.body))
        .to_owned();
    let folder = json!({
        "_type": "FOLDER", "name": text("Eva's folders"),
        "archetype_node_id": "openEHR-EHR-FOLDER.directory.v1",
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-FOLDER.directory.v1" },
            "rm_version": "1.1.0" }
    });
    let answer = call(
        &world.app,
        ROOT,
        "POST",
        &format!("{BASE}/ehr/{}/directory", world.ehr),
        None,
        Some(&folder),
    )
    .await;
    assert_eq!(
        answer.status,
        StatusCode::CREATED,
        "directory: {}",
        answer.body
    );
    let answer = call(
        &world.app,
        ROOT,
        "PUT",
        &format!(
            "{BASE}/ehr/{}/composition/{}/tags",
            world.ehr, world.vitals.vo
        ),
        None,
        Some(&json!([{ "key": "ward", "value": "3" }])),
    )
    .await;
    assert!(
        answer.status.is_success(),
        "tags: {} {}",
        answer.status,
        answer.body
    );
}

/// `root` records Frank's representation of Eva on his `PERSON`.
async fn represent(world: &World) {
    let frank = &world.people["frank"].person;
    let (version, mut body) = read_party(&world.app, "person", frank).await;
    let representation = json!({
        "_type": "PARTY_RELATIONSHIP",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1",
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                "value": "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1" },
            "rm_version": "1.1.0" },
        "name": { "_type": "DV_CODED_TEXT", "value": "Partner",
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" },
                "code_string": "at12" } },
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "id3", "name": text("Details"),
            "items": [{ "_type": "ELEMENT", "archetype_node_id": "id4",
                "name": text("Representation code"), "value": term("represents") }] },
        "source": party_ref("PERSON", frank),
        "target": party_ref("PERSON", &world.eva)
    });
    let mut relationships = body["relationships"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    relationships.push(representation);
    body["relationships"] = Value::Array(relationships);
    let answer = update_party(&world.app, ROOT, ("person", frank, &version), &body).await;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "representation: {}",
        answer.body
    );
}

/// The controller's policy of `controller`, written by `root` over
/// `preceding` (`None` creates it): a care relationship with a 14-day grace
/// and a purpose required, the mental-health note very restricted and the
/// lab's results restricted; its version.
pub(crate) async fn policy(
    app: &axum::Router,
    controller: &str,
    preceding: Option<&str>,
) -> String {
    let archetype = ferroehr::access_policy::ARCHETYPE;
    let classification = |name: &str, scope: Value, code: &str| {
        json!({ "_type": "ACCESS_CLASSIFICATION", "name": text(name),
                "archetype_node_id": "at0002",
                "data": scope,
                "sensitivities": [coded_in(CONFIDENTIALITY, code)] })
    };
    let body = json!({
        "_type": "ACCESS_POLICY",
        "name": text("Sint Example access policy"),
        "archetype_node_id": archetype,
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
            "rm_version": "1.2.0" },
        "controller": party_ref("ORGANISATION", controller),
        "classifications": [
            classification("Mental health", json!({ "_type": "ACCESS_DATA_SCOPE",
                "archetypes": [{ "_type": "ARCHETYPE_ID", "value": MENTAL_HEALTH }] }), "V"),
            classification("Lab results", json!({ "_type": "ACCESS_DATA_SCOPE",
                "templates": [LAB_TEMPLATE] }), "R"),
        ],
        "care_relationship_required": true,
        "relationship_grace": { "_type": "DV_DURATION", "value": "P14D" },
        "purpose_required": true
    });
    let mut req = common::req("PUT", format!("{EXT}/admin/access_policy"))
        .auth(crate::hospital::world::credential(ROOT))
        .json(body.to_string());
    req = match preceding {
        Some(version) => req.if_match(format!("\"{version}\"")),
        None => req.header(http::header::IF_NONE_MATCH, "*"),
    };
    let (status, headers, answer) = common::send(app, req.build()).await;
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "the policy: {status} {answer}"
    );
    common::etag_uid(&headers)
}

/// A directive of `kind` and `effect` with `extra` members.
pub(crate) fn directive(kind: &str, effect: &str, extra: &Value) -> Value {
    let mut d = json!({
        "_type": "ACCESS_DIRECTIVE",
        "name": text(kind),
        "archetype_node_id": "at0001",
        "kind": term(kind),
        "effect": term(effect),
        "emergency_override": false
    });
    if let (Some(d), Some(extra)) = (d.as_object_mut(), extra.as_object()) {
        d.extend(extra.clone());
    }
    d
}

/// An actor scope naming the party of `kind` `id`.
pub(crate) fn naming(kind: &str, id: &str) -> Value {
    json!([{ "_type": "ACCESS_ACTOR_SCOPE", "party": party_ref(kind, id) }])
}

/// A `DV_INTERVAL<DV_DATE_TIME>` from `from` days to `until` days from now.
pub(crate) fn validity(from: i64, until: i64) -> Value {
    let at = |days: i64| {
        jiff::Timestamp::now()
            .checked_add(jiff::SignedDuration::from_hours(days * 24))
            .expect("an instant")
            .to_string()
    };
    json!({ "_type": "DV_INTERVAL",
        "lower": { "_type": "DV_DATE_TIME", "value": at(from) },
        "upper": { "_type": "DV_DATE_TIME", "value": at(until) },
        "lower_unbounded": false, "upper_unbounded": false,
        "lower_included": true, "upper_included": true })
}

/// Eva's directives: the restriction against Gina, the mental-health
/// restriction with Dr. Smit's exception, the regional consent valid over
/// `consent` days (a year from yesterday by default), Frank's delegation, and
/// `extra` directives.
pub(crate) fn eva_directives(world: &World, consent: Option<(i64, i64)>, extra: &[Value]) -> Value {
    let (from, until) = consent.unwrap_or((-1, 365));
    let mut directives = vec![
        directive(
            "restriction",
            "deny",
            &json!({ "applies_to": naming("PERSON", &world.people["gina"].person) }),
        ),
        directive(
            "restriction",
            "deny",
            &json!({
                "emergency_override": true,
                "data": [{ "_type": "ACCESS_DATA_SCOPE",
                    "archetypes": [{ "_type": "ARCHETYPE_ID", "value": MENTAL_HEALTH }] }],
                "exceptions": [directive("consent", "permit",
                    &json!({ "applies_to": naming("PERSON", &world.people["smit"].person) }))]
            }),
        ),
        directive(
            "consent",
            "permit",
            &json!({
                "applies_to": naming("GROUP", &world.region),
                "purposes": [coded_in(crate::hospital::world::PURPOSE_OF_USE, "TREAT")],
                "time_validity": validity(from, until)
            }),
        ),
        directive(
            "delegation",
            "permit",
            &json!({ "applies_to": naming("PERSON", &world.people["frank"].person) }),
        ),
    ];
    directives.extend_from_slice(extra);
    json!({
        "_type": "ACCESS_DIRECTIVE_SETTINGS",
        "default_access": term("restricted"),
        "directives": directives
    })
}

/// Eva commits `settings` as the next `EHR_ACCESS` version of her EHR
/// through an ITS-REST CONTRIBUTION, as its subject; the answer.
pub(crate) async fn commit_settings(
    world: &World,
    settings: &Value,
) -> crate::hospital::world::Answer {
    let preceding = current_access(world).await;
    let committer = json!({ "_type": "PARTY_IDENTIFIED",
        "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": PATIENTS, "id": "eva-login" }] });
    let contribution = json!({
        "_type": "CONTRIBUTION",
        "versions": [{
            "_type": "ORIGINAL_VERSION",
            "commit_audit": { "_type": "AUDIT_DETAILS", "system_id": "ferroehr",
                "change_type": coded_in("openehr", "251"), "committer": committer },
            "lifecycle_state": coded_in("openehr", "532"),
            "preceding_version_uid": { "_type": "OBJECT_VERSION_ID", "value": preceding },
            "data": {
                "_type": "EHR_ACCESS",
                "archetype_node_id": "openEHR-EHR-EHR_ACCESS.generic.v1",
                "archetype_details": { "_type": "ARCHETYPED",
                    "archetype_id": { "_type": "ARCHETYPE_ID",
                                      "value": "openEHR-EHR-EHR_ACCESS.generic.v1" },
                    "rm_version": "1.2.0" },
                "name": text("EHR Access"),
                "settings": settings
            }
        }],
        "audit": { "_type": "AUDIT_DETAILS", "system_id": "ferroehr",
            "change_type": coded_in("openehr", "251"), "committer": committer }
    });
    call(
        &world.app,
        &format!("eva:{}", world.ehr),
        "POST",
        &format!("{BASE}/ehr/{}/contribution", world.ehr),
        None,
        Some(&contribution),
    )
    .await
}

/// Commits Eva's directives with the regional consent over `consent` days,
/// and notes the `EHR_ACCESS` version they are.
pub(crate) async fn directives(world: &mut World, consent: Option<(i64, i64)>) {
    let settings = eva_directives(world, consent, &[]);
    settle(world, &settings).await;
}

/// Commits `settings` as Eva's `EHR_ACCESS` and notes its version.
pub(crate) async fn settle(world: &mut World, settings: &Value) {
    let answer = commit_settings(world, settings).await;
    assert_eq!(
        answer.status,
        StatusCode::CREATED,
        "Eva's directives: {}",
        answer.body
    );
    world.access = current_access(world).await;
}

/// The current version of Eva's `EHR_ACCESS`, as she reads it.
async fn current_access(world: &World) -> String {
    let answer = call(
        &world.app,
        &format!("eva:{}", world.ehr),
        "GET",
        &format!("{EXT}/ehr/{}/ehr_access", world.ehr),
        None,
        None,
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "EHR_ACCESS: {}", answer.body);
    answer.etag()
}
