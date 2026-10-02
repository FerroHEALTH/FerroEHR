// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end composition-validation tests against a real PostgreSQL 18
//! (shared testkit harness): a COMPOSITION committed via the ITS-REST create/update
//! path is validated against its operational template *before* persistence.
//!
//! Oracle + fixtures: the vendored Apache-2.0 openEHR SDK corpus — the IPS
//! operational template (`openehr-its/tests/fixtures/sdk/ips.v0.opt`,
//! `template_id` "International Patient Summary") paired with its canonical-JSON
//! compositions (`openehr-its/tests/vendor/openehr_sdk/composition/…`):
//! `ips_canonical.json` (valid) and `ips_invalid.json` (out-of-range magnitudes
//! and coded values outside the value set). Same pairing the `openehr_sdt::flat`
//! validator's own corpus tests use (`openehr-its/tests/validation.rs`).
//!
//! Spec: openEHR ITS-REST 1.1.0 —
//! `docs/specs/openehr/ITS-REST/specifications/responses/422.yaml`
//! ("content could be converted to a COMPOSITION, but there are semantic
//! validation errors, such as the underlying template is not known or is not
//! validating the supplied COMPOSITION" → `422`). CNF cross-check:
//! `docs/specs/openehr/CNF/docs/platform_test_schedule/master07-func_tc_ehr_composition.adoc`
//! (`create_composition-event_bad_opt` → 422).

#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use serde_json::{Value, json};
use sqlx::PgPool;

use openehr_rm::prelude::PartyProxy;

use ferroehr::service::FerroEhrService;
use ferroehr::service::error::ServiceError;
use ferroehr::service::status::{CallStatusType, SmError};

use ferroehr::service::version_update::{change_type_coded, lifecycle_state_coded};
use openehr_its::rest::generated::common::{UpdateAudit, UpdateAuditData, UpdateVersion};

/// The SM `UPDATE_VERSION` commit envelope for a bare-RM write.
fn uv<T: serde::de::DeserializeOwned>(
    data: &Value,
    change_code: &str,
    preceding: Option<&str>,
) -> UpdateVersion<T> {
    UpdateVersion {
        preceding_version_uid: preceding.map(|p| p.parse().expect("OBJECT_VERSION_ID")),
        lifecycle_state: lifecycle_state_coded("532"),
        attestations: None,
        data: openehr_its::json::from_canonical_value(data)
            .expect("the fixture commit body decodes as its RM type"),
        commit_audit: UpdateAudit::UpdateAudit(UpdateAuditData {
            _type: None,
            system_id: None,
            change_type: change_type_coded(change_code),
            description: None,
            committer: openehr_its::json::from_canonical_value::<PartyProxy>(
                &json!({ "_type": "PARTY_IDENTIFIED", "name": "conformance tester" }),
            )
            .expect("committer"),
            additional_properties: std::collections::BTreeMap::new(),
        }),
        signature: None,
        additional_properties: std::collections::BTreeMap::new(),
    }
}

/// Read a workspace fixture relative to this crate's manifest dir.
fn fixture(rel: &str) -> String {
    let path = format!("{}/{rel}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

fn composition(name: &str) -> Value {
    let rel = format!(
        "../../crates/openehr-its/tests/vendor/openehr_sdk/composition/canonical_json/{name}"
    );
    serde_json::from_str(&fixture(&rel)).unwrap_or_else(|e| panic!("parse {name}: {e}"))
}

const IPS_OPT: &str = "../../crates/openehr-its/tests/fixtures/sdk/ips.v0.opt";

/// The `template_id` the IPS OPT declares (and its canonical compositions
/// carry in `archetype_details.template_id.value`).
const IPS_TEMPLATE_ID: &str = "International Patient Summary";

/// Count the persisted COMPOSITION versions (kind discriminator on `version`).
async fn composition_versions(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM version WHERE kind = 'COMPOSITION'")
        .fetch_one(pool)
        .await
        .expect("count compositions")
}

#[tokio::test]
async fn composition_validation_gates_persistence() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    // Ingest the IPS operational template (the validation target).
    svc.template_adl14_upload(fixture(IPS_OPT))
        .await
        .expect("upload IPS OPT");

    let ehr_id = svc.create_ehr(None).await.expect("create_ehr").to_string();
    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse::<uuid::Uuid>().expect("ehr uuid"));

    // ── valid composition → committed and retrievable ────────────────────────
    // NOTE: `create_composition` returns the new version_uid.
    let ovid = svc
        .create_composition(
            ehr_uuid,
            uv(&composition("ips_canonical.json"), "249", None),
        )
        .await
        .expect("valid composition accepted (201)")
        .version_uid();
    let vo_id = ovid.split("::").next().unwrap().to_owned();

    let fetched = svc
        .get_composition_latest(ehr_uuid, vo_id.parse().expect("vo uuid"))
        .await
        .expect("valid composition persisted");
    assert_eq!(
        fetched["uid"]["value"], ovid,
        "persisted composition round-trips"
    );
    assert_eq!(
        composition_versions(&pool).await,
        1,
        "exactly the one valid composition is stored"
    );

    // ── invalid composition → 422 with per-path violations, NOT persisted ─────
    let err = svc
        .create_composition(ehr_uuid, uv(&composition("ips_invalid.json"), "249", None))
        .await
        .expect_err("invalid composition rejected");
    // A well-formed body that fails RM/template validation is a 422 carrying the
    // per-path violations (`ServiceError::ValidationFailed`); the protocol adapter
    // (`ferroehr-rest`) renders the structured list into the 422 body. Here we
    // assert the variant + that the violations carry the RM paths.
    let ServiceError::ValidationFailed(violations) = &err else {
        panic!("expected content_invalid (422) with per-path violations, got {err:?}");
    };
    assert!(
        violations.iter().any(|v| v.path.contains('/')),
        "422 violations carry the RM-path-keyed entries: {violations:?}"
    );
    // Validation runs before the write transaction, so nothing was persisted.
    assert_eq!(
        composition_versions(&pool).await,
        1,
        "the rejected composition was not persisted"
    );

    // ── unknown template → 422 "template not known" ──────────────────────────
    let mut unknown = composition("ips_canonical.json");
    unknown["archetype_details"]["template_id"]["value"] =
        Value::String("no.such.template.v0".to_owned());
    let err = svc
        .create_composition(ehr_uuid, uv(&unknown, "249", None))
        .await
        .expect_err("unknown template rejected");
    // An unknown template is a 422 `ServiceError::Unprocessable`; the cause
    // still rides in the message.
    let ServiceError::Unprocessable { violation, .. } = &err else {
        panic!("expected content_invalid (422) for unknown template, got {err:?}");
    };
    assert!(
        violation.detail().contains("not known"),
        "422 detail names the cause: {violation}"
    );
    assert_eq!(
        composition_versions(&pool).await,
        1,
        "the unknown-template composition was not persisted"
    );
}

#[tokio::test]
async fn composition_update_is_validated() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    svc.template_adl14_upload(fixture(IPS_OPT))
        .await
        .expect("upload IPS OPT");
    let ehr_id = svc.create_ehr(None).await.expect("create_ehr").to_string();
    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse::<uuid::Uuid>().expect("ehr uuid"));

    // Seed a valid v1.
    let ovid_v1 = svc
        .create_composition(
            ehr_uuid,
            uv(&composition("ips_canonical.json"), "249", None),
        )
        .await
        .expect("valid v1")
        .version_uid();
    let vo_id = ovid_v1.split("::").next().unwrap().to_owned();
    let vo_uuid = vo_id.parse::<ferroehr::ids::VoId>().expect("vo uuid");

    // An update whose body fails template validation is rejected (422) and the
    // stored current version stays at v1.
    let err = svc
        .update_composition(
            ehr_uuid,
            vo_uuid,
            uv(&composition("ips_invalid.json"), "251", Some(&ovid_v1)),
        )
        .await
        .expect_err("invalid update rejected");
    // A failed-validation update is a 422 carrying the per-path violations.
    let ServiceError::ValidationFailed(violations) = &err else {
        panic!("expected content_invalid (422), got {err:?}");
    };
    assert!(
        !violations.is_empty(),
        "the 422 carries at least one per-path violation, got {err:?}"
    );

    let current = svc
        .get_composition_latest(ehr_uuid, vo_uuid)
        .await
        .expect("current still readable");
    assert_eq!(
        current["uid"]["value"], ovid_v1,
        "the rejected update did not advance the version"
    );
}

/// The direct COMPOSITION routes stamp `version.template_id` on every
/// version they commit, exactly like the CONTRIBUTION route — the promoted
/// column the ABAC template attribute (`template_of_version`) and the
/// template-delete guard both read.
///
/// The silent-unbind twin is pinned by the same assertions: were the stamp to
/// go back to `NULL`, `template_of_version` would answer `None` and a
/// template-scoped ABAC rule would stop binding without any error.
/// (`version.template_id` is our own promoted column — no openEHR spec
/// governs the storage mechanics.)
#[tokio::test]
async fn direct_route_commits_stamp_the_template_id() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    svc.template_adl14_upload(fixture(IPS_OPT))
        .await
        .expect("upload IPS OPT");
    let ehr_id = svc.create_ehr(None).await.expect("create_ehr").to_string();
    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse::<uuid::Uuid>().expect("ehr uuid"));

    // ── create (direct route) ────────────────────────────────────────────────
    let ovid_v1 = svc
        .create_composition(
            ehr_uuid,
            uv(&composition("ips_canonical.json"), "249", None),
        )
        .await
        .expect("valid v1")
        .version_uid();
    let vo_uuid = ovid_v1
        .split("::")
        .next()
        .expect("vo_id segment")
        .parse::<ferroehr::ids::VoId>()
        .expect("vo uuid");

    assert_eq!(
        svc.template_of_version(vo_uuid, None)
            .await
            .expect("resolve template of the current version")
            .as_deref(),
        Some(IPS_TEMPLATE_ID),
        "a direct-route create binds its template attribute"
    );

    // ── update (direct route) ────────────────────────────────────────────────
    let ovid_v2 = svc
        .update_composition(
            ehr_uuid,
            vo_uuid,
            uv(&composition("ips_canonical.json"), "251", Some(&ovid_v1)),
        )
        .await
        .expect("valid v2")
        .version_uid();
    assert_ne!(ovid_v1, ovid_v2, "the update advanced the version");
    assert_eq!(
        svc.template_of_version(vo_uuid, Some("2"))
            .await
            .expect("resolve template of version 2")
            .as_deref(),
        Some(IPS_TEMPLATE_ID),
        "a direct-route update stamps the template on the new version too"
    );

    // Every stored version row carries it — the cheap SQL prefilter the
    // template-delete guard counts.
    let stamped: i64 = sqlx::query_scalar("SELECT count(*) FROM version WHERE template_id = $1")
        .bind(IPS_TEMPLATE_ID)
        .fetch_one(&pool)
        .await
        .expect("count stamped versions");
    assert_eq!(stamped, 2, "both direct-route versions are stamped");
    let unstamped: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM version WHERE kind = 'COMPOSITION' AND template_id IS NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("count unstamped versions");
    assert_eq!(
        unstamped, 0,
        "no direct-route COMPOSITION version is left unbound"
    );
}

/// A version-item `commit_audit.change_type` as a coded openEHR audit change type.
fn coded_text_value(code: &str, value: &str) -> Value {
    json!({
        "_type": "DV_CODED_TEXT", "value": value,
        "defining_code": {
            "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
            "code_string": code
        }
    })
}

/// A `553|incomplete|` CONTRIBUTION carrying a single `creation` version.
fn incomplete_creation_contribution(data: &Value) -> Value {
    json!({
        "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } }, "committer": { "_type": "PARTY_IDENTIFIED", "name": "conformance tester" } }, "versions": [{
            "data": data,
            "commit_audit": { "change_type": coded_text_value("249", "creation") },
            "lifecycle_state": {
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": "553"
            }
        }]
    })
}

#[tokio::test]
async fn incomplete_lifecycle_relaxes_lower_bounds_but_not_wrongness() {
    // RM common master06 §"Incomplete Content": a `553|incomplete|` commit
    // treats existence/cardinality lower limits as zero ("data may be missing"),
    // while every wrongness check still applies ("but it may not be wrong").
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    svc.template_adl14_upload(fixture(IPS_OPT))
        .await
        .expect("upload IPS OPT");
    let ehr_uuid = svc.create_ehr(None).await.expect("create_ehr");

    // A composition missing its mandatory sections (content emptied): under the
    // IPS template each SECTION has occurrences min >= 1, so this is a pure
    // lower-bound (Required/Occurrences) violation with no wrongness. Missing
    // content is represented as ABSENT (Void) — a present-empty `content: []`
    // violates COMPOSITION.Content_valid (`content /= Void implies not
    // content.is_empty`, composition.adoc), an RM invariant that stays at full
    // strictness even under 553 ("data may be missing, but it may not be
    // wrong", RM common master06 §Incomplete Content).
    let mut missing = composition("ips_canonical.json");
    missing.as_object_mut().unwrap().remove("content");

    // Committed as `532|complete|` (the direct create path is always complete),
    // the missing-section lower bound is enforced → 422.
    let strict = svc
        .create_composition(ehr_uuid, uv(&missing.clone(), "249", None))
        .await;
    assert!(
        matches!(strict, Err(ServiceError::ValidationFailed(_))),
        "a complete commit must reject the missing mandatory sections, got {strict:?}"
    );
    assert_eq!(
        composition_versions(&pool).await,
        0,
        "the rejected complete commit persisted nothing"
    );

    // The identical body committed as `553|incomplete|` is accepted: the lower
    // limits are treated as zero.
    svc.create_ehr_contribution(ehr_uuid, incomplete_creation_contribution(&missing))
        .await
        .expect("an incomplete commit tolerates the missing mandatory sections");
    assert_eq!(
        composition_versions(&pool).await,
        1,
        "the incomplete commit persisted the composition"
    );

    // But an incomplete commit does NOT tolerate *wrong* data: ips_invalid has
    // out-of-range magnitudes / coded values outside the value set, which are
    // still rejected under 553 ("may be missing, but may not be wrong").
    let wrong = svc
        .create_ehr_contribution(
            ehr_uuid,
            incomplete_creation_contribution(&composition("ips_invalid.json")),
        )
        .await;
    assert!(
        matches!(
            wrong,
            Err(SmError {
                status: CallStatusType::ContentInvalid,
                ..
            })
        ),
        "an incomplete commit must still reject wrong (out-of-range/coded) data, got {wrong:?}"
    );
    assert_eq!(
        composition_versions(&pool).await,
        1,
        "the wrong incomplete commit persisted nothing"
    );
}

// ── WebTemplate cache: per-commit store-read elimination + invalidation ──────
// The commit path validates each COMPOSITION against its operational template's
// derived-runtime `WebTemplate`, resolved through `web_template_for`. That seam
// is cache-first: once a template is warm, subsequent commits are served from
// the in-memory `WebTemplate` cache and never re-read `template_store` (the
// profiled per-commit OPT read). No openEHR spec governs the cache; the spec
// blesses a compiled near-runtime form, the caching is our own design.

/// A second commit against an already-warm template is served from the cached
/// `WebTemplate` and does **not** re-read `template_store`. Proof by probe: after
/// warming the cache with one commit, the stored OPT content is corrupted in
/// place; a second commit still succeeds, which is only possible if the resolver
/// never re-read the (now-broken) stored content. (The `version.template_id`
/// FK forbids deleting the referenced row, so the probe corrupts content rather
/// than deleting — `UPDATE` is not a supported template mutation, only a probe.)
#[tokio::test]
async fn warm_template_is_served_from_cache_without_a_store_read() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    let desc = svc
        .template_adl14_upload(fixture(IPS_OPT))
        .await
        .expect("upload IPS OPT");
    let template_id = desc["template_id"]
        .as_str()
        .expect("template_id")
        .to_owned();

    let ehr_uuid = svc.create_ehr(None).await.expect("create_ehr");

    // First commit warms the cache (web_template_for: miss → build → cache).
    svc.create_composition(
        ehr_uuid,
        uv(&composition("ips_canonical.json"), "249", None),
    )
    .await
    .expect("first valid composition warms the cache");

    // Poison the stored OPT content: any later *store read* on the commit path
    // would build a broken WebTemplate and fail.
    let poisoned =
        sqlx::query("UPDATE template_store SET content = $1 WHERE lower(template_id) = lower($2)")
            .bind("<not-an-operational-template/>")
            .bind(&template_id)
            .execute(&pool)
            .await
            .expect("poison the stored OPT content")
            .rows_affected();
    assert_eq!(poisoned, 1, "exactly the one template row was poisoned");

    // Second commit against the same warm template still succeeds — served from
    // the cached WebTemplate, never re-reading the poisoned store content.
    svc.create_composition(
        ehr_uuid,
        uv(&composition("ips_canonical.json"), "249", None),
    )
    .await
    .expect("second commit is served from the warm cache, not the poisoned store");

    assert_eq!(
        composition_versions(&pool).await,
        2,
        "both compositions persisted"
    );
}

/// Deleting a template through the SM `delete_opt` path invalidates its cached
/// `WebTemplate`. Proof: warm the cache via the example endpoint (no committed
/// composition, so no `version` FK reference blocks the delete), delete the
/// OPT, then commit against it. With invalidation the resolver misses, re-reads
/// the store, finds nothing, and returns the clean "template not known" 422.
/// Were the entry NOT invalidated, the stale WebTemplate would validate the
/// commit and the `version.template_id` FK would then fail with a database
/// error — a different, wrong outcome. Asserting the 422 proves eviction.
#[tokio::test]
async fn deleting_a_template_invalidates_its_web_template_cache() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    let desc = svc
        .template_adl14_upload(fixture(IPS_OPT))
        .await
        .expect("upload IPS OPT");
    let template_id = desc["template_id"]
        .as_str()
        .expect("template_id")
        .to_owned();

    // Warm the WebTemplate cache without committing a composition (the example
    // endpoint builds and caches the same WebTemplate the commit path uses).
    let example = svc
        .template_adl14_example(template_id.clone(), Some("required".to_owned()), None)
        .await
        .expect("example warms the cache");
    assert_eq!(
        example.get("_type").and_then(Value::as_str),
        Some("COMPOSITION"),
        "example is a COMPOSITION"
    );

    // Delete the OPT through the real SM path (opt_delete), which invalidates
    // the cache entry.
    let opt_uuid = svc
        .list_opts_adl14(ferroehr::service::list::Page::all())
        .await
        .expect("list opts")
        .into_iter()
        .next()
        .expect("one stored OPT uuid");
    svc.delete_opt(opt_uuid).await.expect("delete opt");

    // A commit against the now-deleted template resolves to the clean
    // "template not known" 422 — only reachable if the stale WebTemplate was
    // evicted (otherwise the FK would fail with a database error instead).
    let ehr_uuid = svc.create_ehr(None).await.expect("create_ehr");
    let err = svc
        .create_composition(
            ehr_uuid,
            uv(&composition("ips_canonical.json"), "249", None),
        )
        .await
        .expect_err("commit against a deleted template is rejected");
    let ServiceError::Unprocessable { violation, .. } = &err else {
        panic!("expected content_invalid (422) for the deleted template, got {err:?}");
    };
    assert!(
        violation.detail().contains("not known"),
        "422 detail names the cause: {violation}"
    );
    assert_eq!(
        composition_versions(&pool).await,
        0,
        "nothing persisted against the deleted template"
    );
}

// ── RM data_structures §ELEMENT — Inv_null_flavour_indicated ─────────────────

/// Strip the `value` of the first ELEMENT that has one, leaving no
/// `null_flavour` behind — the RM-invalid shape
/// `Inv_null_flavour_indicated` forbids. Returns the RM path of the ELEMENT
/// that was emptied.
fn strip_first_element_value(v: &mut Value, path: &str) -> Option<String> {
    match v {
        Value::Object(map) => {
            if map.get("_type").and_then(Value::as_str) == Some("ELEMENT")
                && map.get("null_flavour").is_none()
                && map.remove("value").is_some()
            {
                return Some(path.to_owned());
            }
            let keys: Vec<String> = map.keys().cloned().collect();
            for key in keys {
                if let Some(child) = map.get_mut(&key)
                    && let Some(found) = strip_first_element_value(child, &format!("{path}/{key}"))
                {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => items
            .iter_mut()
            .enumerate()
            .find_map(|(i, item)| strip_first_element_value(item, &format!("{path}/{i}"))),
        _ => None,
    }
}

/// An ELEMENT carrying neither `value` nor `null_flavour` violates RM
/// `data_structures` §ELEMENT:
///
/// > `Inv_null_flavour_indicated`: `is_null() xor null_flavour = Void`
/// > (`RM/docs/UML/classes/org.openehr.rm.data_structures.element.adoc`)
///
/// The commit choke point runs the RM class-invariant pass unconditionally
/// (template-independent), so such a COMPOSITION is a `422` naming the
/// invariant — never a silent accept, and never a silent drop of the element
/// at the simplified-format seam. Nothing is persisted.
#[tokio::test]
async fn element_without_value_or_null_flavour_is_rejected_at_commit() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    svc.template_adl14_upload(fixture(IPS_OPT))
        .await
        .expect("upload IPS OPT");
    let ehr_uuid = svc.create_ehr(None).await.expect("create_ehr");

    let mut broken = composition("ips_canonical.json");
    let emptied = strip_first_element_value(&mut broken, "")
        .expect("the IPS composition carries a valued ELEMENT to empty");

    let err = svc
        .create_composition(ehr_uuid, uv(&broken, "249", None))
        .await
        .expect_err("an ELEMENT with neither value nor null_flavour must be rejected");
    let ServiceError::ValidationFailed(violations) = &err else {
        panic!("expected content_invalid (422) with per-path violations, got {err:?}");
    };
    assert!(
        violations
            .iter()
            .any(|v| v.message.contains("Inv_null_flavour_indicated")),
        "the 422 must name the violated RM invariant (emptied {emptied}): {violations:?}"
    );
    assert_eq!(
        composition_versions(&pool).await,
        0,
        "the RM-invalid composition was not persisted"
    );
}

/// The ADL2 twin of [`direct_route_commits_stamp_the_template_id`]: a
/// direct-route commit against an **ADL2-registered** operational template is
/// accepted and stamps `version.template_id` — the FK target is the
/// `template_ref` registry (the union of BOTH template dialects' wire
/// addresses, `clinical/0006_definitions.sql`), so the stamp is not refused as
/// a foreign
/// key violation the way a `template_store`-only FK would (the CNF
/// `SF-FLAT-adl2_commit` regression). The in-use delete guard then refuses the
/// ADL2 template's physical delete while the version references it (the same
/// never-orphan guard the OPT 1.4 delete carries; no openEHR spec governs the
/// storage mechanics — our own design; FLAT-over-ADL2 is grounded in the AM
/// side-by-side mandate, BASE `architecture_overview/master05-package_structure.adoc`).
#[tokio::test]
async fn direct_route_commit_against_an_adl2_template_stamps_and_guards() {
    const ADL2_TEMPLATE_ID: &str = "openEHR-EHR-COMPOSITION.cnf_adl2_flat_a.v1.0.0";
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    // The CNF `SF-FLAT-adl2_commit` provisioning pair + FLAT body, read from
    // the committed catalogue corpus (golden vectors over hand-written
    // fixtures, `.claude/rules/testing.md` §Oracles).
    let artifacts = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/fixtures");
    let archetype = std::fs::read_to_string(artifacts.join("adl2/archetype/cnf_count_a.adls"))
        .expect("CNF count archetype source");
    let opt2 = std::fs::read_to_string(artifacts.join("adl2/opt/flat_parity_a.adls"))
        .expect("CNF FLAT-parity OPT2 source");
    svc.upload_artefact(archetype)
        .await
        .expect("upload the ADL2 archetype");
    svc.upload_artefact(opt2)
        .await
        .expect("upload the ADL2 operational template");

    // Convert the FLAT body exactly as the REST edge does (WebTemplate via the
    // dialect-resolving public seam, `openehr_sdt::flat` conversion).
    let wt = svc
        .web_template(ADL2_TEMPLATE_ID)
        .await
        .expect("the ADL2-backed WebTemplate resolves");
    let flat: serde_json::Map<String, Value> = serde_json::from_str(
        &std::fs::read_to_string(artifacts.join("sf/flat.adl2_valid.json"))
            .expect("CNF FLAT fixture"),
    )
    .expect("FLAT JSON");
    let composition =
        openehr_sdt::flat::convert::composition_from_flat(&flat, &wt, "2026-08-03T12:00:00Z")
            .expect("FLAT converts to a canonical COMPOSITION");

    let ehr_id = svc.create_ehr(None).await.expect("create_ehr").to_string();
    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse::<uuid::Uuid>().expect("ehr uuid"));
    let ovid = svc
        .create_composition(ehr_uuid, uv(&composition, "249", None))
        .await
        .expect("a commit against an ADL2-registered template is accepted")
        .version_uid();
    let vo_uuid = ovid
        .split("::")
        .next()
        .expect("vo_id segment")
        .parse::<ferroehr::ids::VoId>()
        .expect("vo uuid");
    assert_eq!(
        svc.template_of_version(vo_uuid, None)
            .await
            .expect("resolve template of the current version")
            .as_deref(),
        Some(ADL2_TEMPLATE_ID),
        "an ADL2-templated direct-route create binds its template attribute"
    );

    // In use → the physical delete refuses (the 409-class conflict status,
    // the same mapping every `ServiceError::Conflict` carries), naming the
    // reference count; the OPT2 row and the registry row both survive.
    let in_use = svc
        .delete_artefact(ADL2_TEMPLATE_ID.to_owned())
        .await
        .expect_err("an in-use ADL2 template must refuse physical deletion");
    assert!(
        matches!(
            &in_use,
            SmError {
                status: CallStatusType::Conflict,
                ..
            }
        ) && in_use
            .message
            .contains("still referenced by 1 committed version"),
        "got {in_use:?}"
    );
    let registered: i64 =
        sqlx::query_scalar("SELECT count(*) FROM template_ref WHERE template_id = $1")
            .bind(ADL2_TEMPLATE_ID)
            .fetch_one(&pool)
            .await
            .expect("count registry rows");
    assert_eq!(registered, 1, "the refused delete left the registry intact");
}
