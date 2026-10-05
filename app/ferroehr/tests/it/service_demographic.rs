// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end DEMOGRAPHIC service tests against a real PostgreSQL 18
//! (shared testkit harness): the party CRUD + versioning + `VERSIONED_PARTY` +
//! contribution + tags lifecycle, driven through the `DemographicService`
//! envelope seam exactly as the REST layer calls it. Verifies the 0003 party
//! migration applies cleanly (the harness runs migrations) and that parties
//! version with no EHR scope.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use serde_json::{Value, json};
use sqlx::PgPool;

use crate::typed_body::typed;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::status::{CallStatusType, SmError};
use openehr_its::rest::generated::common::UpdateVersion;

/// The `uid.value` (`OBJECT_VERSION_ID`) of a versioned-object body.
fn ovid(v: &Value) -> &str {
    v["uid"]["value"].as_str().expect("uid.value")
}

/// The bare versioned-object UUID from an `OBJECT_VERSION_ID`.
fn vo_uuid(v: &Value) -> String {
    ovid(v).split("::").next().expect("vo uuid").to_owned()
}

/// The DB's current instant (`SELECT now()`) as a `jiff::Timestamp`. Time-travel
/// probes MUST anchor their reference instant on this server clock — the same
/// clock that stamps `version.committed_at` — never on the test-process wall
/// clock: a client/DB clock skew under parallel test load would race
/// the at-time read against the version validity intervals.
async fn db_now(pool: &PgPool) -> jiff::Timestamp {
    sqlx::query_scalar::<_, jiff_sqlx::Timestamp>("SELECT now()")
        .fetch_one(pool)
        .await
        .expect("db now()")
        .to_jiff()
}

fn person(name: &str) -> Value {
    json!({
        "_type": "PERSON",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-PERSON.person.v1" },
            "rm_version": "1.1.0" },
        "name": { "_type": "DV_TEXT", "value": name },
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "legal name" },
            "details": {
                "_type": "ITEM_TREE",
                "archetype_node_id": "at0002",
                "name": { "_type": "DV_TEXT", "value": "structure" },
                "items": [{
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0003",
                    "name": { "_type": "DV_TEXT", "value": "family" },
                    "value": { "_type": "DV_TEXT", "value": name }
                }]
            }
        }]
    })
}

fn organisation(name: &str) -> Value {
    json!({
        "_type": "ORGANISATION",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v1",
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v1" },
            "rm_version": "1.1.0" },
        "name": { "_type": "DV_TEXT", "value": name },
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "legal name" },
            "details": {
                "_type": "ITEM_TREE",
                "archetype_node_id": "at0002",
                "name": { "_type": "DV_TEXT", "value": "structure" },
                "items": [{
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0003",
                    "name": { "_type": "DV_TEXT", "value": "org name" },
                    "value": { "_type": "DV_TEXT", "value": name }
                }]
            }
        }]
    })
}

fn role(name: &str) -> Value {
    json!({
        "_type": "ROLE",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.role.v1",
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.role.v1" },
            "rm_version": "1.1.0" },
        "name": { "_type": "DV_TEXT", "value": name },
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": name },
            "details": {
                "_type": "ITEM_TREE",
                "archetype_node_id": "at0002",
                "name": { "_type": "DV_TEXT", "value": "structure" },
                "items": []
            }
        }],
        "performer": {
            "_type": "PARTY_REF",
            "namespace": "demographic",
            "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": "cccccccc-cccc-4ccc-8ccc-cccccccccccc" }
        }
        // No `capabilities`: a PRESENT list must be non-empty
        // (ROLE.Capabilities_valid, role.adoc) — absence is the valid way to
        // carry "no capabilities".
    })
}

/// The SM `UPDATE_VERSION` commit envelope for a bare-RM party write, built
/// from the wire shape (`commit_audit`, terminology-coded lifecycle/change).
/// The `change_type` matches the operation the envelope commits — `249` for a
/// first version, `251` when a `preceding_version_uid` names an existing one
/// (RM common master06 §Contributions; a client-supplied contradicting code
/// is rejected per ITS-REST overview §"openehr-version and
/// openehr-audit-details" + `AUDIT_DETAILS.Change_type_valid`).
fn uv<T: serde::de::DeserializeOwned>(data: &Value, preceding: Option<&str>) -> UpdateVersion<T> {
    let change = if preceding.is_some() { "251" } else { "249" };
    // `lifecycle_state` and `commit_audit.change_type` are `DV_CODED_TEXT` on
    // the released wire (ITS-REST `schemas/common/UpdateVersion.yaml` /
    // `UpdateAudit.yaml` both `$ref` `DvCodedText`), not the flat SM
    // `Terminology_code` spelling.
    let mut v = json!({
        "lifecycle_state": {
            "value": "complete",
            "defining_code": { "terminology_id": { "value": "openehr" }, "code_string": "532" }
        },
        "data": data,
        "commit_audit": {
            "change_type": {
                "value": "commit",
                "defining_code": { "terminology_id": { "value": "openehr" }, "code_string": change }
            },
            "committer": { "_type": "PARTY_IDENTIFIED", "name": "sm tester" }
        }
    });
    if let Some(p) = preceding {
        v["preceding_version_uid"] = json!({ "value": p });
    }
    openehr_its::json::from_canonical_value(&v).expect("UpdateVersion")
}

/// The literal SM `I_DEMOGRAPHIC_SERVICE` + `I_PARTY` calls (typed Uuid/version
/// arguments), not the wire seam: `create_party (UV_PARTY): UUID` →
/// `has_party`/`get_party`/`get_party_at_version` → `update_party` →
/// `delete_party`, checking the `Post_party_deleted: not has_party`
/// post-condition (`i_party.adoc`).
#[tokio::test]
async fn party_sm_calls_round_trip() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    // create_party(UV_PARTY) → the new VERSIONED_OBJECT's id.
    // Boxed: the typed `UPDATE_VERSION<PARTY>` argument rides the SM future
    // (clippy `large_futures`).
    let vo_id = Box::pin(svc.create_party(uv(&person("Jane"), None)))
        .await
        .expect("create_party");

    // has_party true; get_party returns the current PERSON.
    assert!(svc.has_party(vo_id).await.expect("has_party"), "live party");
    let got = svc.get_party(vo_id).await.expect("get_party");
    assert_eq!(got["_type"], "PERSON");
    assert_eq!(got["name"]["value"], "Jane");

    // The current version id — has_party_version_id + get_party_at_version.
    let v1 = got["uid"]["value"].as_str().expect("uid.value").to_owned();
    assert!(v1.ends_with("::1"), "first version, got {v1}");
    assert!(
        svc.has_party_version_id(v1.clone())
            .await
            .expect("has_party_version_id")
    );
    let by_ver = svc
        .get_party_at_version(v1.clone())
        .await
        .expect("get_party_at_version");
    assert_eq!(by_ver["_type"], "ORIGINAL_VERSION");
    // An unknown version id → false / object_version_does_not_exist.
    let bogus = format!("{vo_id}::ferroehr.local::9");
    assert!(!svc.has_party_version_id(bogus.clone()).await.expect("has"));
    let miss = svc.get_party_at_version(bogus).await;
    assert!(
        matches!(
            miss,
            Err(SmError {
                status: CallStatusType::ObjectVersionDoesNotExist,
                ..
            })
        ),
        "unknown version → object_version_does_not_exist, got {miss:?}"
    );

    // update_party(UV_PARTY with preceding_version_uid) → the new version uid.
    let v2 = Box::pin(svc.update_party(vo_id, uv(&person("Jane Roe"), Some(&v1))))
        .await
        .expect("update_party");
    assert!(v2.ends_with("::2"), "second version, got {v2}");
    let got2 = svc.get_party(vo_id).await.expect("get after update");
    assert_eq!(got2["name"]["value"], "Jane Roe");

    // delete_party → post-condition `not has_party`.
    let del = svc.delete_party(vo_id).await.expect("delete_party");
    assert!(!del.is_empty(), "delete returns the deleted version uid");
    assert!(
        !svc.has_party(vo_id).await.expect("has_party after delete"),
        "Post_party_deleted: not has_party"
    );
    // get_party on a deleted (no live current) party → versioned_object_does_not_exist.
    let after = svc.get_party(vo_id).await;
    assert!(
        matches!(
            after,
            Err(SmError {
                status: CallStatusType::VersionedObjectDoesNotExist,
                ..
            })
        ),
        "get_party after delete → 404, got {after:?}"
    );

    // A never-seen id: has_party false, get_party 404.
    let never = ferroehr::ids::VoId(uuid::Uuid::now_v7());
    assert!(!svc.has_party(never).await.expect("has_party unknown"));
    assert!(svc.get_party(never).await.is_err(), "unknown party → error");
}

#[tokio::test]
async fn person_lifecycle_end_to_end() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    // create → v1
    let created = svc
        .party_create(PartyKind::Person, typed(&person("Jane")), None)
        .await
        .expect("create person");
    assert_eq!(created.body["_type"], "PERSON");
    let ovid_v1 = ovid(&created.body).to_owned();
    let vo = vo_uuid(&created.body);
    assert!(ovid_v1.ends_with("::1"), "first version, got {ovid_v1}");
    assert_eq!(
        created.meta.as_ref().expect("meta").ehr_id,
        "",
        "a party has no EHR scope"
    );

    // get current (bare HIER_OBJECT_ID)
    let got = svc
        .party_get(PartyKind::Person, vo.clone(), None)
        .await
        .expect("get person");
    assert_eq!(got.body["name"]["value"], "Jane");

    // get by OBJECT_VERSION_ID (that specific version)
    let by_ovid = svc
        .party_get(PartyKind::Person, ovid_v1.clone(), None)
        .await
        .expect("get by ovid");
    assert_eq!(ovid(&by_ovid.body), ovid_v1);

    // time-travel: capture a time inside v1 FROM THE DB CLOCK, then update to
    // v2. The reference instant MUST come from the server clock (the clock that
    // stamps `version.committed_at`), never the test-process wall clock — a
    // client/DB clock skew under parallel-load testcontainers races the at-time
    // read against the version validity intervals (a real flake this fixes).
    let t_v1 = db_now(&pool).await;
    // A short gap so v2's commit transaction timestamp is strictly greater than
    // t_v1 even at microsecond resolution (both are now on the DB clock).
    tokio::time::sleep(std::time::Duration::from_millis(15)).await;

    // update (If-Match = current OVID) → v2
    let updated = svc
        .party_update(
            PartyKind::Person,
            vo.clone(),
            ovid_v1.clone(),
            typed(&person("Jane Roe")),
            None,
        )
        .await
        .expect("update person");
    let ovid_v2 = ovid(&updated.body).to_owned();
    assert!(ovid_v2.ends_with("::2"), "second version, got {ovid_v2}");

    // at-time read returns v1
    let at_v1 = svc
        .party_get(PartyKind::Person, vo.clone(), Some(t_v1.to_string()))
        .await
        .expect("at-time");
    assert_eq!(at_v1.body["name"]["value"], "Jane");

    // stale If-Match → precondition failed (412)
    let stale = svc
        .party_update(
            PartyKind::Person,
            vo.clone(),
            ovid_v1.clone(),
            typed(&person("stale")),
            None,
        )
        .await;
    assert!(
        matches!(
            stale,
            Err(SmError {
                status: CallStatusType::VersionMismatch,
                ..
            })
        ),
        "stale update, got {stale:?}"
    );

    // wrong-kind route → NotFound
    let wrong = svc.party_get(PartyKind::Role, vo.clone(), None).await;
    assert!(
        matches!(
            wrong,
            Err(SmError {
                status: CallStatusType::VersionedObjectDoesNotExist,
                ..
            })
        ),
        "person under role route is 404, got {wrong:?}"
    );

    // VERSIONED_PARTY + revision history + version-by-id
    let vp = svc
        .versioned_party_get(vo.clone())
        .await
        .expect("versioned_party");
    assert_eq!(vp.body["_type"], "VERSIONED_PARTY");
    let rh = svc
        .versioned_party_revision_history(vo.clone())
        .await
        .expect("revision_history");
    assert_eq!(rh.body["items"].as_array().expect("items").len(), 2);
    let ov = svc
        .versioned_party_version_get_by_id(vo.clone(), ovid_v1.clone())
        .await
        .expect("version by id");
    assert_eq!(ov.body["_type"], "ORIGINAL_VERSION");

    // delete (OBJECT_VERSION_ID on the path, no If-Match) → deleted;
    // subsequent get → 204 (Null)
    let deleted = svc
        .party_delete(PartyKind::Person, ovid_v2.clone(), None, None)
        .await
        .expect("delete");
    assert!(deleted.is_empty());
    let after = svc
        .party_get(PartyKind::Person, vo.clone(), None)
        .await
        .expect("get after delete");
    assert!(after.is_empty(), "deleted current read is 204 (Null body)");
}

/// Party write-path fix: the create/update representation is now
/// built **from the commit result** (never a post-commit re-read). It must be
/// byte-identical to a fresh read — the served body is
/// `inject_uid(reassemble(decompose(body)))` and the node codec round-trips
/// losslessly (RM common master06 §Committal: the written version identity +
/// content). Mirrors the EHR/DIRECTORY `write_responses_match_a_fresh_read`
/// gate; covers PERSON and the ORGANISATION sibling (same path, keyed by
/// `PartyKind`).
#[tokio::test]
async fn party_write_responses_match_a_fresh_read() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    // create → the built-from-commit body equals a fresh read.
    let created = svc
        .party_create(PartyKind::Person, typed(&person("Jane")), None)
        .await
        .expect("create person");
    let ovid_v1 = ovid(&created.body).to_owned();
    let vo = vo_uuid(&created.body);
    let fresh = svc
        .party_get(PartyKind::Person, vo.clone(), None)
        .await
        .expect("get person");
    assert_eq!(
        created.body, fresh.body,
        "create body built from the commit must equal a fresh read"
    );
    assert_eq!(
        created.meta.as_ref().expect("create meta").uid,
        fresh.meta.as_ref().expect("read meta").uid,
        "create uid == fresh read uid"
    );

    // update → the built-from-commit body equals a fresh read.
    let updated = svc
        .party_update(
            PartyKind::Person,
            vo.clone(),
            ovid_v1,
            typed(&person("Jane Roe")),
            None,
        )
        .await
        .expect("update person");
    let fresh_v2 = svc
        .party_get(PartyKind::Person, vo.clone(), None)
        .await
        .expect("get person v2");
    assert_eq!(
        updated.body, fresh_v2.body,
        "update body built from the commit must equal a fresh read"
    );
    assert_eq!(
        updated.meta.as_ref().expect("update meta").uid,
        fresh_v2.meta.as_ref().expect("read meta").uid,
    );

    // ORGANISATION sibling — the same create path keyed by PartyKind.
    let created_org = svc
        .party_create(PartyKind::Organisation, typed(&organisation("Acme")), None)
        .await
        .expect("create organisation");
    let vo_org = vo_uuid(&created_org.body);
    let fresh_org = svc
        .party_get(PartyKind::Organisation, vo_org, None)
        .await
        .expect("get organisation");
    assert_eq!(
        created_org.body, fresh_org.body,
        "organisation create body built from the commit must equal a fresh read"
    );
}

/// The versioned-party delete shape the DEM ECC cases drive (ECC-DEM-005/006 …):
/// the path is the **bare** versioned-object uid and the preceding version is
/// carried by `If-Match` (the `OBJECT_VERSION_ID`). Delete succeeds (`204`) and
/// the current read afterwards is a deleted/`204` (Null body).
#[tokio::test]
async fn person_delete_by_versioned_uid_with_if_match() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    let created = svc
        .party_create(PartyKind::Person, typed(&person("Jane")), None)
        .await
        .expect("create person");
    let ovid = ovid(&created.body).to_owned();
    let vo = vo_uuid(&created.body);

    // Bare versioned-object uid on the path, `If-Match` = the current OVID.
    let deleted = svc
        .party_delete(PartyKind::Person, vo.clone(), Some(ovid), None)
        .await
        .expect("delete by versioned uid + If-Match");
    assert!(deleted.is_empty());

    // The deleted current version reads back as 204 (Null body).
    let after = svc
        .party_get(PartyKind::Person, vo, None)
        .await
        .expect("get after delete");
    assert!(after.is_empty(), "deleted current read is 204 (Null body)");
}

#[tokio::test]
async fn role_create_and_get() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    let created = svc
        .party_create(PartyKind::Role, typed(&role("Clinician")), None)
        .await
        .expect("create role");
    assert_eq!(created.body["_type"], "ROLE");
    let vo = vo_uuid(&created.body);
    let got = svc
        .party_get(PartyKind::Role, vo, None)
        .await
        .expect("get role");
    assert_eq!(got.body["name"]["value"], "Clinician");
    assert_eq!(got.body["performer"]["type"], "PERSON");
}

/// The demographic CONTRIBUTION shares the commit audit rule of the EHR one
/// (#3550): `schemas/demographic/UpdateVersion.yaml` references the same
/// `schemas/common/UpdateAudit.yaml`, whose `required` lists `committer`, so a
/// member `commit_audit` without one is the 400 and is never filled from the
/// CONTRIBUTION audit.
#[tokio::test]
async fn demographic_contribution_member_without_committer_is_refused() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let coded = |code: &str, value: &str| {
        json!({
            "_type": "DV_CODED_TEXT", "value": value,
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": code
            }
        })
    };
    let err = svc
        .demographic_contribution_create(json!({
            "versions": [{
                "lifecycle_state": coded("532", "complete"),
                "commit_audit": { "change_type": coded("249", "creation") },
                "data": person("Unattributed")
            }],
            "audit": {
                "change_type": coded("249", "creation"),
                "committer": { "_type": "PARTY_IDENTIFIED", "name": "tester" }
            }
        }))
        .await
        .expect_err("a demographic member without a committer is refused");
    assert_eq!(
        err.status,
        CallStatusType::PreconditionViolation,
        "got {err:?}"
    );
    assert!(
        err.message
            .starts_with("versions[0]: commit_audit.committer is required"),
        "got {err:?}"
    );
}

#[tokio::test]
async fn demographic_contribution_multi_version() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    let body = json!({
        "_type": "CONTRIBUTION",
        "versions": [
            {
                "lifecycle_state": { "terminology_id": "openehr", "code_string": "532" },
                "_type": "ORIGINAL_VERSION",
                "commit_audit": {
                    "change_type": {
                        "_type": "DV_CODED_TEXT",
                        "value": "creation",
                        "defining_code": {
                            "_type": "CODE_PHRASE",
                            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                            "code_string": "249"
                        }
                    },
                    "committer": { "_type": "PARTY_IDENTIFIED", "name": "tester" }
                },
                "data": person("Alice")
            },
            {
                "lifecycle_state": { "terminology_id": "openehr", "code_string": "532" },
                "_type": "ORIGINAL_VERSION",
                "commit_audit": {
                    "change_type": {
                        "_type": "DV_CODED_TEXT",
                        "value": "creation",
                        "defining_code": {
                            "_type": "CODE_PHRASE",
                            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                            "code_string": "249"
                        }
                    },
                    "committer": { "_type": "PARTY_IDENTIFIED", "name": "tester" }
                },
                "data": role("Nurse")
            }
        ],
        "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } },
            "committer": { "_type": "PARTY_IDENTIFIED", "name": "tester" }
        }
    });

    let created = svc
        .demographic_contribution_create(body)
        .await
        .expect("create contribution");
    assert_eq!(created.body["_type"], "CONTRIBUTION");
    let uid = created.body["uid"]["value"]
        .as_str()
        .expect("uid")
        .to_owned();
    assert_eq!(
        created.body["versions"].as_array().expect("versions").len(),
        2
    );

    let got = svc
        .demographic_contribution_get(uid)
        .await
        .expect("get contribution");
    assert_eq!(got.body["versions"].as_array().expect("versions").len(), 2);
}

#[tokio::test]
async fn party_tags_crud() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    let created = svc
        .party_create(PartyKind::Person, typed(&person("Tagged")), None)
        .await
        .expect("create person");
    let vo = vo_uuid(&created.body);

    // PUT a tag collection
    let tags = svc
        .party_tags_update(
            PartyKind::Person,
            vo.clone(),
            vec![crate::item_tag_fixture::party_tag(
                "priority",
                Some("high"),
                None,
            )],
        )
        .await
        .expect("put tags");
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].key(), "priority");

    // GET tags on the party
    let got = svc
        .party_tags_get(PartyKind::Person, vo.clone())
        .await
        .expect("get tags");
    assert_eq!(got.len(), 1);

    // demographic tags (ehr-less scope) sees it
    let all = svc
        .demographic_tags_get(None, None, None)
        .await
        .expect("all demographic tags");
    assert_eq!(all.len(), 1);

    // DELETE the tag
    svc.party_tags_delete(PartyKind::Person, vo.clone(), "priority".to_owned())
        .await
        .expect("delete tag");
    let empty = svc
        .party_tags_get(PartyKind::Person, vo)
        .await
        .expect("get tags after delete");
    assert!(empty.is_empty());
}

/// The `If-Match` precondition compares the FULL `OBJECT_VERSION_ID`
/// case-insensitively: an `OBJECT_VERSION_ID` is a composite identifier, and
/// BASE `base_types` `master05-identification_package.adoc` §"Composite
/// Identifiers and Case" makes two identifiers "identical apart from case …
/// identify the same thing". A case-variant precondition therefore names the
/// current version and MUST NOT raise a `412`. The quoted wire form is accepted
/// at the same seam (RFC 9110 §8.8.3 — an entity-tag is a quoted string).
#[tokio::test]
async fn party_update_if_match_is_case_insensitive_and_quote_tolerant() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    let created = svc
        .party_create(PartyKind::Person, typed(&person("Jane")), None)
        .await
        .expect("create person");
    let ovid_v1 = ovid(&created.body).to_owned();
    let vo = vo_uuid(&created.body);

    // A case-variant of the current version_uid names the SAME version.
    let upper = ovid_v1.to_uppercase();
    assert_ne!(upper, ovid_v1, "the committed uid must have case to flip");
    let v2 = svc
        .party_update(
            PartyKind::Person,
            vo.clone(),
            upper,
            typed(&person("Jane Roe")),
            None,
        )
        .await
        .expect("BASE master05 §Composite Identifiers and Case: case-variant If-Match matches");
    let ovid_v2 = ovid(&v2.body).to_owned();
    assert!(ovid_v2.ends_with("::2"), "second version, got {ovid_v2}");

    // The quoted wire form of the now-current version is accepted verbatim.
    let v3 = svc
        .party_update(
            PartyKind::Person,
            vo.clone(),
            format!("\"{ovid_v2}\""),
            typed(&person("Jane Doe")),
            None,
        )
        .await
        .expect("quoted If-Match accepted");
    assert!(ovid(&v3.body).ends_with("::3"), "third version");

    // A stale full OVID is still the precondition failure (412).
    let stale = svc
        .party_update(
            PartyKind::Person,
            vo,
            ovid_v1,
            typed(&person("stale")),
            None,
        )
        .await;
    assert!(
        matches!(
            stale,
            Err(SmError {
                status: CallStatusType::VersionMismatch,
                ..
            })
        ),
        "a stale full OBJECT_VERSION_ID must still fail the precondition, got {stale:?}"
    );
}

/// A PARTY committed with an inline `relationships` list keeps that list
/// VERBATIM, and it stays disjoint from the service-managed relationship
/// containers.
///
/// RM demographic `docs/demographic/master02-demographic_package.adoc` §Party
/// Relationships (L44) makes the list compositional data of the source party —
/// "`PARTY_RELATIONSHIPs` are stored as part of the data of the `PARTY`
/// designated as the source. This means that the relationships attribute is by
/// value, while the `PARTY_RELATIONSHIP._source_` and `_target_` are
/// represented by references. The actual kind of reference is via the use of
/// `OBJECT_REFs` containing `HIER_OBJECT_IDs` to denote the Version container
/// of a Party, rather than `OBJECT_VERSION_IDs`" — and §Versioning Semantics
/// (L48) versions it with the party: "A Version of a `PARTY` includes all the
/// compositional parts, such as identities, contacts, Party relationships of
/// which it is the source."
///
/// The SM instead models a relationship as its OWN version container (all six
/// `I_PARTY_RELATIONSHIP` operations key on `a_versioned_party_rel_id`), which
/// this server realizes on its `versioned_party_relationship` surface. The two
/// representations are DISJOINT — committing this party mints no relationship
/// container, and no container write edits this list.
#[tokio::test]
async fn inline_relationships_are_stored_and_served_verbatim() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    let relationship = json!({
        "_type": "PARTY_RELATIONSHIP",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.relationship.v1",
        "name": { "_type": "DV_TEXT", "value": "inline relationship" },
        "source": { "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": "11111111-1111-4111-8111-111111111111" } },
        "target": { "_type": "PARTY_REF", "namespace": "demographic", "type": "ORGANISATION",
            "id": { "_type": "HIER_OBJECT_ID", "value": "22222222-2222-4222-8222-222222222222" } }
    });
    let mut body = person("Jane Related");
    body["relationships"] = json!([relationship.clone()]);

    // ACCEPTED: a by-value relationships list is ordinary PARTY content
    // (PARTY.Relationships_validity only forbids a PRESENT-EMPTY list).
    let created = svc
        .party_create(PartyKind::Person, typed(&body), None)
        .await
        .expect("a party carrying an inline relationships list is accepted");
    let vo = vo_uuid(&created.body);

    // SERVED VERBATIM: the read-back list is the committed one, unchanged,
    // unexpanded and un-repointed.
    let got = svc
        .party_get(PartyKind::Person, vo.clone(), None)
        .await
        .expect("read the party back");
    assert_eq!(
        got.body["relationships"],
        json!([relationship]),
        "the by-value relationships list must survive the commit byte-for-byte \
         (RM demographic master02 §Party Relationships / §Versioning Semantics)"
    );

    // DISJOINT: the inline relationship minted no relationship container, so
    // the party's own container id is not a relationship either.
    let not_a_relationship = svc.versioned_party_relationship_get(vo).await;
    assert!(
        matches!(
            not_a_relationship,
            Err(SmError {
                status: CallStatusType::VersionedObjectDoesNotExist,
                ..
            })
        ),
        "an inline by-value relationship must not create a relationship \
         container, got {not_a_relationship:?}"
    );
}

/// `PARTY.Relationships_validity`'s source-identity arm compares against the
/// party's VERSION CONTAINER id, so a round-tripped party (GET → edit → PUT)
/// carrying an inline `relationships` list commits.
///
/// RM demographic `docs/demographic/master02-demographic_package.adoc` §Party
/// Relationships (L44) requires the refs to be "`OBJECT_REFs` containing
/// `HIER_OBJECT_IDs` to denote the Version container of a Party, rather than
/// `OBJECT_VERSION_IDs`, which would denote particular versions" — while a
/// served party's `uid` is the three-part `OBJECT_VERSION_ID` of the version it
/// was read from (BASE `master05-identification_package.adoc` §Syntaxes:
/// `object_version_id = object_id, '::', creating_system_id, '::',
/// version_tree_id`). Comparing the container-scoped `source` against the whole
/// version id can never hold, so the two shapes here are BOTH pinned: the
/// container id accepts, a foreign container id still refuses.
#[tokio::test]
async fn inline_relationship_source_matches_the_version_container_not_the_version() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    let created = svc
        .party_create(PartyKind::Person, typed(&person("Jane Container")), None)
        .await
        .expect("create person");
    let vo = vo_uuid(&created.body);

    // The round trip: read the served body back — it carries the three-part
    // `uid` — and edit it in place, exactly as a client does.
    let served = svc
        .party_get(PartyKind::Person, vo.clone(), None)
        .await
        .expect("read the party back");
    let ovid_v1 = ovid(&served.body).to_owned();
    assert!(
        ovid_v1.split("::").count() == 3,
        "the served uid is the three-part OBJECT_VERSION_ID, got {ovid_v1}"
    );

    let relationship = |source: &str| {
        json!({
            "_type": "PARTY_RELATIONSHIP",
            "archetype_node_id": "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.relationship.v1",
            "name": { "_type": "DV_TEXT", "value": "inline relationship" },
            "source": { "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
                "id": { "_type": "HIER_OBJECT_ID", "value": source } },
            "target": { "_type": "PARTY_REF", "namespace": "demographic", "type": "ORGANISATION",
                "id": { "_type": "HIER_OBJECT_ID", "value": "22222222-2222-4222-8222-222222222222" } }
        })
    };

    // ACCEPTED: `source` names this party's version container.
    let mut body = served.body.clone();
    body["relationships"] = json!([relationship(&vo)]);
    let updated = svc
        .party_update(
            PartyKind::Person,
            vo.clone(),
            ovid_v1.clone(),
            typed(&body),
            None,
        )
        .await
        .expect(
            "a round-tripped party whose inline relationship names its own version \
             container must commit (RM demographic master02 §Party Relationships)",
        );
    let ovid_v2 = ovid(&updated.body).to_owned();
    assert!(ovid_v2.ends_with("::2"), "second version, got {ovid_v2}");

    // REFUSED (the arm is kept): `source` names another party's container.
    let mut foreign = svc
        .party_get(PartyKind::Person, vo.clone(), None)
        .await
        .expect("read v2 back")
        .body;
    foreign["relationships"] = json!([relationship("33333333-3333-4333-8333-333333333333")]);
    let refused = svc
        .party_update(
            PartyKind::Person,
            vo.clone(),
            ovid_v2,
            typed(&foreign),
            None,
        )
        .await;
    assert!(
        matches!(
            refused,
            Err(SmError {
                status: CallStatusType::ContentInvalid,
                ..
            })
        ),
        "an inline relationship sourced at another party is still invalid \
         (PARTY.Relationships_validity), got {refused:?}"
    );
}

/// The `owner_id` of an ehr-less demographic version container names the
/// SERVING SYSTEM, in the shape the published `VersionedParty` example uses.
///
/// `VERSIONED_OBJECT.owner_id` is `OBJECT_REF` 1..1, "Reference to object to
/// which this version container belongs, e.g. the id of the containing EHR or
/// other relevant owning entity" (RM common
/// `UML/classes/org.openehr.rm.common.versioned_object.adoc` §Attributes) — but
/// a demographic party has no containing EHR (RM demographic
/// `docs/demographic/master02-demographic_package.adoc` §Versioning Semantics:
/// "Every Party is stored in its own Version container"), so the first limb has
/// no referent and the second is read as the serving system. The one concrete
/// released shape is the `VersionedParty` example in the vendored ITS-REST OAS
/// (`crates/openehr-its/vendor/rest-oas/demographic-codegen.openapi.yaml`,
/// `components.schemas.VersionedParty.example`): an `OBJECT_REF` with
/// `namespace: local`, `type: SYSTEM` over a `HIER_OBJECT_ID`; the docs text
/// fixes nothing here, so the `id` is the configured system identifier,
/// cross-checked here against the value the commit audits carry.
#[tokio::test]
async fn versioned_party_owner_id_names_the_serving_system() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    let created = svc
        .party_create(PartyKind::Person, typed(&person("Jane")), None)
        .await
        .expect("create person");
    let vo = vo_uuid(&created.body);

    let vp = svc
        .versioned_party_get(vo.clone())
        .await
        .expect("versioned_party");
    assert_eq!(vp.body["owner_id"]["_type"], "OBJECT_REF");
    assert_eq!(vp.body["owner_id"]["namespace"], "local");
    assert_eq!(vp.body["owner_id"]["type"], "SYSTEM");
    assert_eq!(vp.body["owner_id"]["id"]["_type"], "HIER_OBJECT_ID");

    let rh = svc
        .versioned_party_revision_history(vo)
        .await
        .expect("revision_history");
    assert_eq!(
        vp.body["owner_id"]["id"]["value"], rh.body["items"][0]["audits"][0]["system_id"],
        "the container owner_id names the serving system — the same identifier \
         the commit audits carry"
    );
}
