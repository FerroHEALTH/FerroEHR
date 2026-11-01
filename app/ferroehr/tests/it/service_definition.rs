// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end tests for the Definitions native API against a real
//! `PostgreSQL` 18 (shared testkit harness): ADL 1.4 source archetypes, OPTs
//! (delegated to `template_store`), and registered queries. Driven through the
//! service methods exactly as a protocol adapter would call them.
//!
//! Requires Docker. Each test owns its container (`Drop` removes it).

#![expect(
    clippy::panic,
    clippy::string_slice,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use ferroehr::service::FerroEhrService;
use ferroehr::service::definition::types::TemplateListFilter;
use ferroehr::service::error::ServiceError;
use ferroehr::service::status::{ServiceStatus, StatusError};

use ferroehr::service::list::Page;

use crate::adl2_fixture::adl2_source;

fn fixture(rel: &str) -> String {
    let path = format!(
        "{}/../../corpus/fixtures/service/{rel}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

const ARCHETYPE_REL: &str = "knowledge/archetypes/openEHR-EHR-COMPOSITION.prescription.v1.adl";
const ARCHETYPE_ID: &str = "openEHR-EHR-COMPOSITION.prescription.v1";

const REVISION_HISTORY_ARCHETYPE_REL: &str =
    "knowledge/archetypes/openEHR-EHR-OBSERVATION.revision_history.v1.adl";
const REVISION_HISTORY_ARCHETYPE_ID: &str = "openEHR-EHR-OBSERVATION.revision_history.v1";

const OPT_REL: &str = "knowledge/IDCR Allergies List.v0.opt";
const OPT_TEMPLATE_ID: &str = "IDCR Allergies List.v0";

// ── ADL 1.4 archetypes ───────────────────────────────────────────────────────

#[tokio::test]
async fn archetype_upload_get_list_replace_delete() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let adl = fixture(ARCHETYPE_REL);

    // Precondition: not present yet.
    assert!(!svc.has_archetype(ARCHETYPE_ID.to_owned()).await.unwrap());
    assert_eq!(
        svc.list_archetypes_adl14(Page::all()).await.unwrap(),
        Vec::<String>::new()
    );

    // A source that is no archetype is refused and stores nothing.
    assert!(
        svc.upload_archetype("not an archetype".to_owned())
            .await
            .is_err()
    );

    // upload → the archetype is present.
    svc.upload_archetype(adl.clone()).await.expect("upload");
    assert!(svc.has_archetype(ARCHETYPE_ID.to_owned()).await.unwrap());
    assert_eq!(
        svc.list_archetypes_adl14(Page::all()).await.unwrap().len(),
        1
    );

    // get returns the source verbatim.
    let got = svc
        .get_archetype(ARCHETYPE_ID.to_owned())
        .await
        .expect("get");
    assert_eq!(got, adl, "stored ADL source is byte-identical");

    // list names the stored archetype.
    let list = svc.list_archetypes_adl14(Page::all()).await.unwrap();
    assert_eq!(list, vec![ARCHETYPE_ID.to_owned()]);

    // upload_archetype replaces if the id already exists (spec: "replace it").
    let replacement = format!("{adl}\n-- replaced\n");
    svc.upload_archetype(replacement.clone())
        .await
        .expect("replace");
    assert_eq!(
        svc.list_archetypes_adl14(Page::all()).await.unwrap().len(),
        1,
        "replace, not insert"
    );
    assert_eq!(
        svc.get_archetype(ARCHETYPE_ID.to_owned()).await.unwrap(),
        replacement,
        "replacement overwrote the source"
    );

    // delete → the archetype is gone.
    svc.delete_archetype(ARCHETYPE_ID.to_owned())
        .await
        .expect("delete");
    assert!(!svc.has_archetype(ARCHETYPE_ID.to_owned()).await.unwrap());
    assert_eq!(
        svc.list_archetypes_adl14(Page::all()).await.unwrap(),
        Vec::<String>::new()
    );
}

/// An ADL 1.4 archetype carrying the optional `revision_history` section
/// (`docs/specs/openehr/AM/docs/ADL1.4/master08-adl.adoc` §Revision History
/// Section — "It is optional, and is included at the end of the archetype")
/// is a spec-valid 1.4 source: it must validate and upload like any other.
///
/// The section has no landing field on the assembled AOM2 artefact by upstream
/// decision — `AM/docs/ADL2/master01-preface.adoc` §Changes from ADL 1.4
/// removed it "since the AOM2 uses the openEHR Base Types version of the
/// Resource package" (SPECAM-61) — but the upload stores the 1.4 *source*
/// verbatim, so nothing is lost on the round trip.
#[tokio::test]
async fn archetype_with_revision_history_section_uploads() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let adl = fixture(REVISION_HISTORY_ARCHETYPE_REL);

    svc.upload_archetype(adl.clone())
        .await
        .expect("a 1.4 archetype with a revision_history section is valid and uploads");
    assert!(
        svc.has_archetype(REVISION_HISTORY_ARCHETYPE_ID.to_owned())
            .await
            .unwrap()
    );
    assert_eq!(
        svc.get_archetype(REVISION_HISTORY_ARCHETYPE_ID.to_owned())
            .await
            .expect("get"),
        adl,
        "the revision_history section survives storage verbatim"
    );
}

#[tokio::test]
async fn archetype_errors() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    // Invalid ADL → 422 reporting invalid_archetype.
    let bad = svc
        .upload_archetype("this is not an archetype".to_owned())
        .await
        .expect_err("invalid archetype rejected");
    assert!(
        matches!(
            bad,
            StatusError {
                status: ServiceStatus::InvalidArchetype,
                ..
            }
        ),
        "got {bad:?}"
    );

    // get / delete of an absent archetype → 404.
    let missing = svc
        .get_archetype("openEHR-EHR-OBSERVATION.absent.v1".to_owned())
        .await
        .expect_err("absent archetype");
    assert!(
        matches!(
            missing,
            StatusError {
                status: ServiceStatus::ArtefactDoesNotExist,
                ..
            }
        ),
        "got {missing:?}"
    );
    let del_missing = svc
        .delete_archetype("openEHR-EHR-OBSERVATION.absent.v1".to_owned())
        .await
        .expect_err("delete absent");
    assert!(
        matches!(
            del_missing,
            StatusError {
                status: ServiceStatus::ArtefactDoesNotExist,
                ..
            }
        ),
        "got {del_missing:?}"
    );
}

#[tokio::test]
async fn archetype_semantic_validity_runs_the_14_engine() {
    // The 1.4 upload runs the real `openehr-adl` engine (judged as 1.4), not a
    // structural probe: a source that PARSES but violates an ADL 1.4 / AOM 1.4
    // rule is rejected. ADL1.4 master08 §Validity Rules VARDT — the topmost
    // definition typename must match the RM class of the archetype id.
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let adl = fixture(ARCHETYPE_REL);

    // Break VARDT: the id is a COMPOSITION, the definition root now claims
    // OBSERVATION.
    let bad = adl.replacen("COMPOSITION[at0000]", "OBSERVATION[at0000]", 1);
    let err = svc
        .upload_archetype(bad)
        .await
        .expect_err("VARDT-violating upload rejected");
    // The rule-code mnemonic is carried through as the validation detail.
    assert!(
        matches!(
            err,
            StatusError {
                status: ServiceStatus::InvalidArchetype,
                ..
            }
        ),
        "got {err:?}"
    );
    assert!(
        err.message.contains("VARDT"),
        "the 422 detail must name the offending rule code, got {:?}",
        err.message
    );
    // Nothing was stored (validation gates the write).
    assert_eq!(
        svc.list_archetypes_adl14(Page::all()).await.unwrap(),
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn adl14_convert_to_adl2_migration_round_trip() {
    // The in-CDR 1.4 → ADL 2 migration capability: a stored 1.4 archetype
    // converts to ADL2 source that validates through the full ADL2 pipeline
    // (the same service path a native ADL2 upload takes). No openEHR spec
    // governs 1.4 → 2 conversion — our own design/extension.
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let adl = fixture(ARCHETYPE_REL);

    svc.upload_archetype(adl)
        .await
        .expect("upload 1.4 archetype");

    let conversion = svc
        .adl14_convert_to_adl2(ARCHETYPE_ID.to_owned())
        .await
        .expect("convert stored 1.4 archetype to ADL2");
    assert_eq!(conversion.roots.len(), 1, "one archetype, one root");
    assert_eq!(
        conversion.structure,
        Vec::new(),
        "an archetype fills no slot"
    );
    let adl2 = conversion.roots[0].adl2.clone();
    assert!(
        adl2.contains("archetype") && adl2.contains("COMPOSITION"),
        "converted output is ADL2 source, got: {}",
        &adl2[..adl2.len().min(120)]
    );

    // The converted artefact validates + stores through the full ADL2 engine
    // (parse → AOM2 phase 1 → RM conformance) — the migration produces a
    // spec-valid ADL2 archetype.
    svc.template_adl2_upload(adl2)
        .await
        .expect("the converted ADL2 artefact validates through the ADL2 pipeline");

    // Converting an absent archetype is a 404.
    let missing = svc
        .adl14_convert_to_adl2("openEHR-EHR-OBSERVATION.absent.v1".to_owned())
        .await
        .expect_err("convert absent archetype");
    assert!(
        matches!(
            missing,
            StatusError {
                status: ServiceStatus::ArtefactDoesNotExist,
                ..
            }
        ),
        "got {missing:?}"
    );
}

// ── ADL 1.4 OPTs (on template_store, keyed by template_id on the wire) ─────

/// The `template_id` of every summary a wire template list returns.
fn listed_template_ids(list: &[serde_json::Value]) -> Vec<String> {
    list.iter()
        .map(|t| {
            t["template_id"]
                .as_str()
                .unwrap_or_else(|| panic!("a template summary names its template_id: {t}"))
                .to_owned()
        })
        .collect()
}

#[tokio::test]
async fn opt_upload_get_list_filter_delete() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let xml = fixture(OPT_REL);

    // An XML document that is no OPT is refused and stores nothing.
    assert!(
        svc.template_adl14_upload("<not-a-template/>".to_owned())
            .await
            .is_err()
    );
    assert_eq!(
        svc.template_adl14_list(TemplateListFilter::default(), Page::all())
            .await
            .unwrap(),
        Vec::<serde_json::Value>::new()
    );

    // upload (valid OPT).
    svc.template_adl14_upload(xml.clone())
        .await
        .expect("upload opt");
    let listed = svc
        .template_adl14_list(TemplateListFilter::default(), Page::all())
        .await
        .unwrap();
    assert_eq!(
        listed_template_ids(&listed),
        vec![OPT_TEMPLATE_ID.to_owned()]
    );

    // get by the wire address returns the stored XML verbatim.
    let got = svc
        .template_adl14_get(OPT_TEMPLATE_ID.to_owned())
        .await
        .expect("get opt");
    assert_eq!(got, xml, "stored OPT XML is byte-identical");

    // A template_id pattern matches the stored template.
    let matched = svc
        .template_adl14_list(
            TemplateListFilter {
                template_id: Some("IDCR*Allergies*".to_owned()),
                ..TemplateListFilter::default()
            },
            Page::all(),
        )
        .await
        .unwrap();
    assert_eq!(
        listed_template_ids(&matched),
        vec![OPT_TEMPLATE_ID.to_owned()]
    );

    // Re-uploading the same template_id conflicts (wire's CNF-tested rule wins).
    let conflict = svc
        .template_adl14_upload(xml.clone())
        .await
        .expect_err("duplicate opt");
    assert!(
        matches!(
            conflict,
            StatusError {
                status: ServiceStatus::Conflict,
                ..
            }
        ),
        "got {conflict:?}"
    );

    // delete (present, then removed).
    svc.admin_template_delete(OPT_TEMPLATE_ID.to_owned())
        .await
        .expect("delete opt");
    assert!(
        svc.template_adl14_get(OPT_TEMPLATE_ID.to_owned())
            .await
            .is_err()
    );
    assert_eq!(
        svc.template_adl14_list(TemplateListFilter::default(), Page::all())
            .await
            .unwrap(),
        Vec::<serde_json::Value>::new()
    );
}

/// The template delete refuses (409) while a committed version still
/// references the template, so a physical delete never orphans clinical
/// data. The refusal is our own integrity design. Pointing an existing
/// `version` at the template exercises the FK-reference guard directly
/// (lighter than a full validated commit, which the guard does not need).
#[tokio::test]
async fn opt_delete_refuses_while_referenced() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    svc.template_adl14_upload(fixture(OPT_REL))
        .await
        .expect("upload opt");

    // Reference the template from a committed version row.
    let ehr: uuid::Uuid = Box::pin(svc.create_ehr_meta(None, None))
        .await
        .map(|(id, _)| id)
        .expect("ehr")
        .into();
    let referenced = sqlx::query("UPDATE version SET template_id = $1 WHERE ehr_id = $2")
        .bind(OPT_TEMPLATE_ID)
        .bind(ehr)
        .execute(&pool)
        .await
        .expect("reference template")
        .rows_affected();
    assert!(
        referenced >= 1,
        "a version row must now reference the template"
    );

    // Referenced → refused with the friendly 409 naming the count.
    let res = svc.admin_template_delete(OPT_TEMPLATE_ID.to_owned()).await;
    match res {
        Err(StatusError {
            status: ServiceStatus::Conflict,
            ref message,
            ..
        }) => {
            assert!(
                message.contains(&format!("referenced by {referenced} committed version")),
                "the 409 names the reference count ({referenced}), got {message:?}"
            );
        }
        other => panic!("referenced template must be refused (409), got {other:?}"),
    }
    assert!(
        svc.template_adl14_get(OPT_TEMPLATE_ID.to_owned())
            .await
            .is_ok(),
        "a refused delete leaves the template in place"
    );

    // Dereference → the delete succeeds (OPT removed).
    sqlx::query("UPDATE version SET template_id = NULL WHERE ehr_id = $1")
        .bind(ehr)
        .execute(&pool)
        .await
        .expect("dereference template");
    svc.admin_template_delete(OPT_TEMPLATE_ID.to_owned())
        .await
        .expect("delete opt");
    assert!(
        svc.template_adl14_get(OPT_TEMPLATE_ID.to_owned())
            .await
            .is_err()
    );
}

/// The ITS-REST `definition_template_adl1.4_list` filter + pagination params
/// (`operations/definition_template_adl1.4_list.yaml`;
/// `parameters/query/filter_template_id.yaml` — "supports wildcards `*`").
/// Two OPTs are uploaded through the
/// wire-shaped `DefinitionAdapter`; the list must honour the `template_id` glob
/// and the `offset`/`fetch` window rather than returning the full set.
#[tokio::test]
async fn template_adl14_list_filters_and_paginates() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    // Two templates: "IDCR Allergies List.v0" and "IDCR Problem List.v1".
    svc.template_adl14_upload(fixture(OPT_REL))
        .await
        .expect("upload allergies");
    svc.template_adl14_upload(fixture("knowledge/IDCR Problem List.v1.opt"))
        .await
        .expect("upload problem");

    let template_ids = |list: &[serde_json::Value]| -> Vec<String> {
        list.iter()
            .map(|t| t["template_id"].as_str().unwrap_or_default().to_owned())
            .collect()
    };

    // Empty filter + Page::all → the full set (both templates).
    let all = svc
        .template_adl14_list(TemplateListFilter::default(), Page::all())
        .await
        .expect("list all");
    assert_eq!(all.len(), 2, "both templates without a filter");

    // template_id glob "IDCR Allergies*" → only the allergies template.
    let filtered = svc
        .template_adl14_list(
            TemplateListFilter {
                template_id: Some("IDCR Allergies*".to_owned()),
                ..TemplateListFilter::default()
            },
            Page::all(),
        )
        .await
        .expect("list filtered");
    assert_eq!(
        template_ids(&filtered),
        vec![OPT_TEMPLATE_ID.to_owned()],
        "glob matches only the allergies template"
    );

    // A glob matching neither → empty.
    let none = svc
        .template_adl14_list(
            TemplateListFilter {
                template_id: Some("does-not-exist*".to_owned()),
                ..TemplateListFilter::default()
            },
            Page::all(),
        )
        .await
        .expect("list empty");
    assert!(none.is_empty(), "no template matches the glob");

    // offset=1 fetch=1 over the sorted set (Allergies < Problem) → the second.
    let paged = svc
        .template_adl14_list(
            TemplateListFilter::default(),
            Page {
                item_offset: Some(1),
                items_to_fetch: Some(1),
            },
        )
        .await
        .expect("list paged");
    assert_eq!(
        template_ids(&paged),
        vec!["IDCR Problem List.v1".to_owned()],
        "offset=1 fetch=1 yields the second sorted template"
    );

    // The optional ITS-REST TemplateMetadata.version is derived from the
    // template_id's `.vN` axis (spec: "taken from template_id") — never stored.
    // "IDCR Allergies List.v0" → "0", "IDCR Problem List.v1" → "1".
    for descriptor in &all {
        let tid = descriptor["template_id"].as_str().expect("template_id");
        let expected = tid
            .rsplit_once(".v")
            .map(|(_, v)| v)
            .expect("id carries .vN");
        assert_eq!(
            descriptor["version"].as_str(),
            Some(expected),
            "version derived from {tid}"
        );
    }
}

/// The `version`-absent listing collapses a versioned template family to its
/// latest `.vN` axis; `version=*` lists every stored version. The ITS-REST
/// docs text is silent on template-list version filtering, so the RELEASED
/// OAS grounds the behaviour: "Filter by version (e.g. `1.2.*` or use `*`
/// for all versions), taken from `template_id`; if missing, then only the
/// latest version will be returned"
/// (`specifications/parameters/query/filter_version.yaml`).
#[tokio::test]
async fn template_adl14_list_absent_version_collapses_to_latest() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    // Two versions of the one family: the v0 fixture, and a v9 sibling made
    // by re-versioning its template_id (the store keys by template_id, so
    // this is a distinct stored template of the same base identity).
    svc.template_adl14_upload(fixture(OPT_REL))
        .await
        .expect("upload v0");
    svc.template_adl14_upload(
        fixture(OPT_REL).replace("IDCR Allergies List.v0", "IDCR Allergies List.v9"),
    )
    .await
    .expect("upload v9");

    let template_ids = |list: &[serde_json::Value]| -> Vec<String> {
        list.iter()
            .map(|t| t["template_id"].as_str().unwrap_or_default().to_owned())
            .collect()
    };

    // `version` absent → only the latest version of the family.
    let latest = svc
        .template_adl14_list(TemplateListFilter::default(), Page::all())
        .await
        .expect("list latest");
    assert_eq!(
        template_ids(&latest),
        vec!["IDCR Allergies List.v9".to_owned()],
        "the absent-version listing carries only the latest version"
    );

    // `version=*` → every stored version.
    let all = svc
        .template_adl14_list(
            TemplateListFilter {
                version: Some("*".to_owned()),
                ..TemplateListFilter::default()
            },
            Page::all(),
        )
        .await
        .expect("list all versions");
    assert_eq!(
        template_ids(&all),
        vec![
            "IDCR Allergies List.v0".to_owned(),
            "IDCR Allergies List.v9".to_owned(),
        ],
        "`version=*` lists every stored version"
    );
}

/// The wire `query_type` formalism: a non-AQL formalism is an honest
/// *unsupported-formalism* reject (a distinct `precondition_violation`/`400`,
/// per `operations/definition_query_store.yaml`'s `200/400` set +
/// `parameters/query/query_type.yaml`), not a blanket "invalid AQL"; an AQL
/// formalism stores.
#[tokio::test]
async fn query_store_rejects_non_aql_formalism() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    let aql = "SELECT c FROM EHR e CONTAINS COMPOSITION c".to_owned();

    // A non-AQL formalism → unsupported-formalism reject.
    let err = svc
        .query_store(
            "org.example::q1".to_owned(),
            Some("1.0.0".to_owned()),
            "SQL".to_owned(),
            aql.clone(),
        )
        .await
        .expect_err("non-AQL formalism must be rejected");
    assert!(
        matches!(
            err,
            StatusError {
                status: ServiceStatus::PreconditionViolation,
                ..
            }
        ),
        "non-AQL is precondition_violation, got {err:?}"
    );

    // The AQL formalism (case-insensitive) stores.
    svc.query_store(
        "org.example::q1".to_owned(),
        Some("1.0.0".to_owned()),
        "aql".to_owned(),
        aql,
    )
    .await
    .expect("AQL formalism stores");
}

/// The stored-query name grammar per ITS-REST query `Qualified_query_name`:
/// the namespace is optional (`[{namespace}::]{query-name}`; `my_compositions`
/// is a listed valid example), the query-name character set `[a-zA-Z0-9_.-]`
/// admits dots and hyphens, and the query-name `aql` is reserved
/// (case-insensitive, §NOTE).
#[tokio::test]
async fn query_store_name_grammar_and_reserved_name() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    let aql = "SELECT c FROM EHR e CONTAINS COMPOSITION c".to_owned();

    // Namespace-less names — plain and dotted — store and read back.
    for name in ["ward_dashboard_probe", "cnf.ward_dashboard-probe"] {
        svc.query_store(name.to_owned(), None, "AQL".to_owned(), aql.clone())
            .await
            .unwrap_or_else(|e| panic!("namespace-less name `{name}` must store: {e:?}"));
        let listed = svc
            .query_list(name.to_owned())
            .await
            .unwrap_or_else(|e| panic!("`{name}` must be retrievable: {e:?}"));
        // The descriptor carries the ONE canonical identity — the
        // `misc::`-assumed qualified name (our own naming design);
        // the bare-name LIST still finds it (the pattern's misc composition).
        let canonical = format!("misc::{name}");
        assert!(
            listed
                .iter()
                .any(|d| d.get("name").and_then(|v| v.as_str()) == Some(canonical.as_str())),
            "the stored descriptor carries the canonical misc-qualified name: {listed:?}"
        );
    }

    // The reserved query-name `aql` is rejected case-insensitively, with or
    // without a namespace (Qualified_query_name §NOTE).
    for name in ["aql", "AQL", "org.openehr::aql", "org.openehr::AQL"] {
        let err = svc
            .query_store(name.to_owned(), None, "AQL".to_owned(), aql.clone())
            .await
            .expect_err("the reserved query-name `aql` must be rejected");
        assert!(
            matches!(
                err,
                StatusError {
                    status: ServiceStatus::PreconditionViolation,
                    ..
                }
            ),
            "`{name}` rejects as precondition_violation (wire 400), got {err:?}"
        );
    }

    // A three-part name's MIDDLE `aql` is the formalism segment, not the
    // query-name — never reserved.
    svc.query_store(
        "task_planning::aql::chemotherapy_plans".to_owned(),
        None,
        "AQL".to_owned(),
        aql,
    )
    .await
    .expect("a three-part name with the `aql` formalism segment stores");
}

#[tokio::test]
async fn opt_errors() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    // Invalid OPT → 422 reporting invalid_template.
    let bad = svc
        .template_adl14_upload("<not-a-template/>".to_owned())
        .await
        .expect_err("invalid opt");
    assert!(
        matches!(
            bad,
            StatusError {
                status: ServiceStatus::InvalidTemplate,
                ..
            }
        ),
        "got {bad:?}"
    );

    // An absent template_id → get/delete 404.
    let absent = "IDCR Absent.v0".to_owned();
    let get_missing = svc
        .template_adl14_get(absent.clone())
        .await
        .expect_err("absent opt");
    assert!(
        matches!(
            get_missing,
            StatusError {
                status: ServiceStatus::TemplateDoesNotExist,
                ..
            }
        ),
        "got {get_missing:?}"
    );
    let del_missing = svc
        .admin_template_delete(absent)
        .await
        .expect_err("delete absent opt");
    assert!(
        matches!(
            del_missing,
            StatusError {
                status: ServiceStatus::TemplateDoesNotExist,
                ..
            }
        ),
        "got {del_missing:?}"
    );
}

// ── ADL2 artefacts (on adl2_artefact, HRID-keyed) ────────────────────────────

const ADL2_OPT_HRID: &str = "openEHR-EHR-COMPOSITION.t_clinical_info.v1.0.0";
const ADL2_ARCH_HRID: &str = "openEHR-EHR-OBSERVATION.bp.v1.0.0";
const ADL2_TMPL_HRID: &str = "openEHR-EHR-COMPOSITION.t_vitals.v2.0.0";

#[tokio::test]
async fn adl2_upload_get_list_by_kind_delete() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    // Preconditions: empty, and a source that is no ADL2 is refused.
    assert_eq!(svc.artefacts_count().await.unwrap(), 0);
    let adl2_opt = adl2_source(
        "operational_template",
        ADL2_OPT_HRID,
        Some("openEHR-EHR-COMPOSITION.discharge.v1"),
    );
    let adl2_arch = adl2_source("archetype", ADL2_ARCH_HRID, None);
    let adl2_tmpl = adl2_source("template", ADL2_TMPL_HRID, None);
    assert!(
        svc.template_adl2_upload("this is not adl2".to_owned())
            .await
            .is_err()
    );
    assert_eq!(svc.artefacts_count().await.unwrap(), 0);

    // upload (OPT) → the artefact is present (keyed by ARCHETYPE_HRID).
    assert!(!svc.has_artefact(ADL2_OPT_HRID.to_owned()).await.unwrap());
    svc.template_adl2_upload(adl2_opt.clone())
        .await
        .expect("upload opt");
    assert!(svc.has_artefact(ADL2_OPT_HRID.to_owned()).await.unwrap());

    // get returns the source verbatim.
    assert_eq!(
        svc.template_adl2_source(ADL2_OPT_HRID.to_owned(), None)
            .await
            .unwrap()
            .payload,
        adl2_opt,
        "stored ADL2 source is byte-identical"
    );

    // Upload one artefact of each other kind.
    svc.template_adl2_upload(adl2_arch.clone())
        .await
        .expect("upload archetype");
    svc.template_adl2_upload(adl2_tmpl.clone())
        .await
        .expect("upload template");

    // Counts: all artefacts, and the archetype kind.
    assert_eq!(svc.artefacts_count().await.unwrap(), 3);
    assert_eq!(svc.archetypes_count_adl2().await.unwrap(), 1);

    // list_artefacts = all HRIDs; the archetype list and the template list
    // partition them.
    let all = svc.list_artefacts(Page::all()).await.unwrap();
    assert_eq!(all.len(), 3);
    assert!(all.contains(&ADL2_OPT_HRID.to_owned()));
    assert_eq!(
        svc.list_archetypes_adl2(Page::all()).await.unwrap(),
        vec![ADL2_ARCH_HRID.to_owned()]
    );
    let mut templates = listed_template_ids(
        &svc.template_adl2_list(TemplateListFilter::default(), Page::all())
            .await
            .unwrap(),
    );
    templates.sort();
    assert_eq!(
        templates,
        vec![ADL2_OPT_HRID.to_owned(), ADL2_TMPL_HRID.to_owned()]
    );

    // A second upload of a stored HRID conflicts and leaves the source as it
    // was.
    let replacement = format!("{adl2_opt}-- replaced\n");
    assert!(
        matches!(
            svc.template_adl2_upload(replacement).await,
            Err(ServiceError::Conflict(_))
        ),
        "a stored HRID is never replaced"
    );
    assert_eq!(svc.artefacts_count().await.unwrap(), 3);
    assert_eq!(
        svc.template_adl2_source(ADL2_OPT_HRID.to_owned(), None)
            .await
            .unwrap()
            .payload,
        adl2_opt
    );

    // delete_artefact → Post: gone.
    svc.delete_artefact(ADL2_OPT_HRID.to_owned())
        .await
        .expect("delete");
    assert!(!svc.has_artefact(ADL2_OPT_HRID.to_owned()).await.unwrap());
    assert_eq!(svc.artefacts_count().await.unwrap(), 2);
}

#[tokio::test]
async fn adl2_errors() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    // Unparseable ADL2 (unrecognised header) is *syntactically invalid
    // content* — the released 400 branch (ITS-REST `responses/400.yaml`:
    // "could not be parsed or is invalid (e.g. ... syntactically invalid ...
    // content)"), not the semantic 422 that AOM2 validation-phase failures
    // carry.
    let bad = StatusError::from(
        svc.template_adl2_upload("concept\nopenEHR-EHR-OBSERVATION.bp.v1.0.0".to_owned())
            .await
            .expect_err("invalid artefact rejected"),
    );
    assert!(
        matches!(
            bad,
            StatusError {
                status: ServiceStatus::PreconditionViolation,
                ..
            }
        ),
        "got {bad:?}"
    );

    // Recognised header but a malformed HRID fails the grammar → 400 too.
    let bad_hrid = StatusError::from(
        svc.template_adl2_upload("archetype\nnot-an-hrid".to_owned())
            .await
            .expect_err("malformed hrid rejected"),
    );
    assert!(
        matches!(
            bad_hrid,
            StatusError {
                status: ServiceStatus::PreconditionViolation,
                ..
            }
        ),
        "got {bad_hrid:?}"
    );

    // get / delete of an absent artefact → 404.
    let missing = StatusError::from(
        svc.template_adl2_source("openEHR-EHR-OBSERVATION.absent.v1.0.0".to_owned(), None)
            .await
            .expect_err("absent artefact"),
    );
    assert!(
        matches!(
            missing,
            StatusError {
                status: ServiceStatus::TemplateDoesNotExist,
                ..
            }
        ),
        "got {missing:?}"
    );
    let del_missing = svc
        .delete_artefact("openEHR-EHR-OBSERVATION.absent.v1.0.0".to_owned())
        .await
        .expect_err("delete absent");
    assert!(
        matches!(
            del_missing,
            StatusError {
                status: ServiceStatus::ArtefactDoesNotExist,
                ..
            }
        ),
        "got {del_missing:?}"
    );
}

// ── registered queries ───────────────────────────────────────────────────────

/// The `name` of every stored query a wire query list returns.
fn listed_query_names(list: &[serde_json::Value]) -> Vec<String> {
    list.iter()
        .map(|q| {
            q["name"]
                .as_str()
                .unwrap_or_else(|| panic!("a stored query names itself: {q}"))
                .to_owned()
        })
        .collect()
}

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "one full registered-query lifecycle exercised end to end on a \
              single container"
)]
async fn query_store_get_list_delete() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let good = "SELECT c FROM COMPOSITION c";
    let precondition = |e: &StatusError| e.status == ServiceStatus::PreconditionViolation;

    // The formalism: AQL in any case and at major version 1 stores; any other
    // formalism, or a text that is no AQL, is refused and stores nothing.
    for (name, formalism) in [("f::lower", "aql"), ("f::major", "AQL::1")] {
        svc.query_store(name.to_owned(), None, formalism.to_owned(), good.to_owned())
            .await
            .unwrap_or_else(|e| panic!("{formalism} stores: {e:?}"));
        svc.admin_query_delete(name.to_owned(), "1.0.0".to_owned())
            .await
            .expect("delete the formalism probe");
    }
    let cql = svc
        .query_store("f::cql".to_owned(), None, "cql".to_owned(), good.to_owned())
        .await
        .expect_err("cql is no supported formalism");
    assert!(precondition(&cql), "got {cql:?}");
    let not_aql = svc
        .query_store(
            "f::not_aql".to_owned(),
            None,
            "AQL".to_owned(),
            "this is not aql".to_owned(),
        )
        .await
        .expect_err("a text that is no AQL is refused");
    assert!(precondition(&not_aql), "got {not_aql:?}");
    assert_eq!(
        svc.query_list(String::new()).await.unwrap(),
        Vec::<serde_json::Value>::new()
    );

    // store under an explicit qualified name.
    let version = svc
        .query_store(
            "ehr::over_50".to_owned(),
            None,
            "AQL".to_owned(),
            good.to_owned(),
        )
        .await
        .expect("store named");
    let named = svc
        .query_version_get("ehr::over_50".to_owned(), version.clone())
        .await
        .expect("the named query is registered");
    assert_eq!(named["name"], "ehr::over_50");
    assert_eq!(named["q"], good);
    // A bare name gets the "misc" namespace applied.
    svc.query_store(
        "bare_name".to_owned(),
        None,
        "aql".to_owned(),
        good.to_owned(),
    )
    .await
    .expect("store bare");
    for name in ["bare_name", "misc::bare_name"] {
        assert!(
            svc.query_version_get(name.to_owned(), "1".to_owned())
                .await
                .is_ok(),
            "{name} addresses the bare-name store"
        );
    }

    // ── ONE canonical key across every surface (bare name → the "misc"
    // namespace) ─────────────────────────────────────────────────────────────
    // The WIRE store of a bare name must land on the SAME row the by-name wire
    // GET, the list, and the admin delete address — the former asymmetry
    // keyed the wire store under ('', name) and made the query
    // undeletable/unfindable under the name it was created with.
    svc.query_store(
        "wire_bare".to_owned(),
        Some("1.0.0".to_owned()),
        "AQL".to_owned(),
        good.to_owned(),
    )
    .await
    .expect("wire store of a bare name");
    assert!(
        svc.query_version_get("wire_bare".to_owned(), "1.0.0".to_owned())
            .await
            .is_ok(),
        "the bare name addresses the wire-stored bare name"
    );
    let fetched = svc
        .query_version_get("misc::wire_bare".to_owned(), "1.0.0".to_owned())
        .await
        .expect("qualified wire GET resolves the bare-name store");
    assert_eq!(fetched["version"], "1.0.0");
    svc.admin_query_delete("wire_bare".to_owned(), "1.0.0".to_owned())
        .await
        .expect("admin delete addresses the same canonical row by the bare name");
    assert!(
        svc.query_version_get("wire_bare".to_owned(), "1.0.0".to_owned())
            .await
            .is_err(),
        "deleted everywhere"
    );

    // The list holds the two stored queries; a name prefix narrows it.
    let mut all = listed_query_names(&svc.query_list(String::new()).await.unwrap());
    all.sort();
    assert_eq!(all, vec!["ehr::over_50", "misc::bare_name"]);
    assert_eq!(
        listed_query_names(&svc.query_list("ehr::".to_owned()).await.unwrap()),
        vec!["ehr::over_50"]
    );

    // delete → the query is gone.
    svc.admin_query_delete("ehr::over_50".to_owned(), version.clone())
        .await
        .expect("delete");
    assert!(
        svc.query_version_get("ehr::over_50".to_owned(), version.clone())
            .await
            .is_err()
    );
    assert_eq!(svc.query_list(String::new()).await.unwrap().len(), 1);
    // Deleting an absent query → 404.
    let del_missing = svc
        .admin_query_delete("ehr::over_50".to_owned(), version)
        .await
        .expect_err("delete absent");
    assert!(
        matches!(
            del_missing,
            StatusError {
                status: ServiceStatus::ArtefactDoesNotExist,
                ..
            }
        ),
        "got {del_missing:?}"
    );
}

/// The REST adapter surface for ADL2 template upload: a duplicate HRID is a
/// conflict (`409_template_already_exists`, `definition-codegen.openapi.yaml`
/// `/definition/template/adl2` POST) and invalid source is a 400.
#[tokio::test]
async fn adl2_template_upload_wire_conflicts_on_duplicate() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    let tmpl = adl2_source("template", "openEHR-EHR-COMPOSITION.t_wire.v1.0.0", None);
    let hrid = svc
        .template_adl2_upload(tmpl.clone())
        .await
        .expect("first upload");
    assert_eq!(hrid, "openEHR-EHR-COMPOSITION.t_wire.v1.0.0");

    let dup = svc
        .template_adl2_upload(tmpl.clone())
        .await
        .expect_err("duplicate template id conflicts on the wire surface");
    assert!(
        matches!(&dup, ServiceError::Conflict(e) if e.message.contains("already exists")),
        "got {dup:?}"
    );

    // A source that fails the ADL2 grammar (missing mandatory sections →
    // S-codes) is *syntactically invalid content* — the released 400 branch
    // declared on the upload (ITS-REST `responses/400.yaml`), not the
    // semantic 422 that AOM2 validation-phase failures (V-codes) carry.
    let bad = svc
        .template_adl2_upload(
            "template (adl_version=2.0.6)\nopenEHR-EHR-COMPOSITION.t_bad.v1\n".to_owned(),
        )
        .await
        .expect_err("invalid source rejected");
    assert!(
        matches!(&bad, ServiceError::BadRequest(e) if e.message.contains("syntactically invalid")),
        "got {bad:?}"
    );
}

/// An ADL 2 operational template whose included OBSERVATION archetype reuses
/// the template's node ids `id2` and `id3` (a synthetic fixture).
const OPT2_REUSED_IDS: &str = "operational_template (adl_version=2.0.6; rm_release=1.0.2; generated)
    openEHR-EHR-COMPOSITION.t_archetype_scope.v1.0.0

language
    original_language = <[ISO_639-1::en]>

description
    original_author = <
        [\"name\"] = <\"Veredictum contributors\">
    >
    lifecycle_state = <\"unmanaged\">
    details = <
        [\"en\"] = <
            language = <[ISO_639-1::en]>
            purpose = <\"A minimal operational template for conformance testing\">
        >
    >

definition
    COMPOSITION[id1] matches {
        category matches {
            DV_CODED_TEXT[id2] matches {
                defining_code matches {[at1]}
            }
        }
        content cardinality matches {0..*; unordered} matches {
            OBSERVATION[id3, openEHR-EHR-OBSERVATION.cnf_minimal.v1.0.0] occurrences matches {0..*} matches {
                data matches {
                    HISTORY[id2] matches {
                        events cardinality matches {1..*; unordered} matches {
                            EVENT[id3] occurrences matches {0..*} matches {
                                data matches {
                                    ITEM_TREE[id4] matches {
                                        items cardinality matches {0..*; unordered} matches {
                                            ELEMENT[id5] occurrences matches {0..*} matches {
                                                value matches {
                                                    DV_TEXT[id6]
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

terminology
    term_definitions = <
        [\"en\"] = <
            [\"id1\"] = <
                text = <\"Minimal\">
                description = <\"A minimal event composition\">
            >
            [\"id3\"] = <
                text = <\"Minimal observation\">
                description = <\"The observation the composition holds\">
            >
            [\"at1\"] = <
                text = <\"event\">
                description = <\"The openEHR composition category event\">
            >
        >
    >
    term_bindings = <
        [\"openehr\"] = <
            [\"at1\"] = <http://openehr.org/id/433>
        >
    >

component_terminologies
    [\"openEHR-EHR-OBSERVATION.cnf_minimal.v1.0.0\"] = <
        term_definitions = <
            [\"en\"] = <
                [\"id1\"] = <
                    text = <\"Minimal observation\">
                    description = <\"An observation with one text element\">
                >
                [\"id3\"] = <
                    text = <\"Any event\">
                    description = <\"Any event\">
                >
                [\"id5\"] = <
                    text = <\"Value\">
                    description = <\"A text value\">
                >
            >
        >
    >
";

/// An included archetype's node ids are its own: AOM2 `master04.5` VCOSU holds
/// "within the archetype", and OPT2 `master03-opt_raw` keeps each constituent's
/// terminology under `component_terminologies`, so the upload is accepted
/// (`201`); a node id repeated inside the included archetype is still `422`.
#[tokio::test]
async fn adl2_template_upload_accepts_node_ids_reused_by_an_included_archetype() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    let hrid = svc
        .template_adl2_upload(OPT2_REUSED_IDS.to_owned())
        .await
        .expect("an included archetype may reuse the template's node ids");
    assert_eq!(hrid, "openEHR-EHR-COMPOSITION.t_archetype_scope.v1.0.0");

    let duplicate = OPT2_REUSED_IDS
        .replace("t_archetype_scope", "t_archetype_scope_dup")
        .replace("ITEM_TREE[id4]", "ITEM_TREE[id2]");
    let refused = svc
        .template_adl2_upload(duplicate)
        .await
        .expect_err("a node id repeated inside one archetype is refused");
    assert!(
        matches!(&refused, ServiceError::ValidationFailed(errors)
            if errors.iter().any(|e| format!("{e:?}").contains("VCOSU"))),
        "got {refused:?}"
    );
}

// ── ADL2/OPT2 FLAT parity: commit-path resolver fallback (#269) ───────────────

/// The runtime `WebTemplate` resolver (`web_template`/`web_template_for`) falls
/// back to the ADL2/OPT2 store when a template id is not an ADL 1.4 template, so a
/// FLAT/STRUCTURED composition **commit** keyed to an ADL2-registered template
/// resolves and is validated — the ADL2 twin of the OPT 1.4 commit path. Before
/// #269 this 422'd "operational template not known" (the resolver read only the
/// OPT 1.4 `template_store`).
#[tokio::test]
async fn adl2_template_resolves_on_the_commit_path_and_validates() {
    use serde_json::Value;

    const HRID: &str = "openEHR-EHR-COMPOSITION.commit_resolver.v1.0.0";

    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    let opt = adl2_source("operational_template", HRID, None);
    svc.template_adl2_upload(opt)
        .await
        .expect("upload ADL2 OPT");

    // Gap 2: the runtime resolver used by the FLAT/STRUCTURED commit path now
    // resolves the ADL2 template (before #269: `ServiceError::Unprocessable`).
    let wt = svc
        .web_template(HRID)
        .await
        .expect("the ADL2 template resolves on the commit path");
    assert_eq!(wt.template_id, HRID);
    assert_eq!(
        wt.sem_ver.as_deref(),
        Some("1.0.0"),
        "resolved through the v2_4 (OPT2) front end, which carries semVer"
    );

    // A composition declaring that template validates end-to-end through the
    // commit path.
    let comp = openehr_sdt::flat::example::example_composition(
        &wt,
        openehr_sdt::flat::example::DetailLevel::Complete,
    )
    .expect("the example builds");
    assert_eq!(
        comp.pointer("/archetype_details/template_id/value")
            .and_then(Value::as_str),
        Some(HRID),
        "the example declares the ADL2 template id"
    );
    let ehr = Box::pin(svc.create_ehr_meta(None, None))
        .await
        .expect("create an EHR")
        .0;
    svc.create_composition(ehr, crate::fixtures::uv(&comp, "249", None))
        .await
        .expect("a self-consistent instance validates clean against the ADL2 template");
}

/// The full service seam for a template-with-filler: upload the archetype +
/// the `template` that `use_archetype`-fills it, then assert the projected
/// commit-path `WebTemplate` CONTAINS the filler's flattened subtree — under
/// the SLOT-LEVEL name the template's own terminology defines (OPT2 master03:
/// `create_opt` inlines every filler; RM `composition.entry.adoc` §Invariants
/// `Is_archetype_root` makes the fill the only conformant way to put an ENTRY
/// under content). Regression for the filler-root rubric resolving in the
/// component terminology (mislabeling the node from the constituent's
/// unrelated same-numbered id code).
#[tokio::test]
async fn adl2_template_with_filler_projects_the_filled_web_template() {
    fn find<'a>(
        n: &'a openehr_sdt::flat::webtemplate::model::WebTemplateNode,
        id: &str,
    ) -> Option<&'a openehr_sdt::flat::webtemplate::model::WebTemplateNode> {
        if n.id == id {
            return Some(n);
        }
        n.children.iter().find_map(|c| find(c, id))
    }
    fn dump(
        n: &openehr_sdt::flat::webtemplate::model::WebTemplateNode,
        d: usize,
        out: &mut String,
    ) {
        use std::fmt::Write;
        let _ = writeln!(out, "{}{} [{}]", "  ".repeat(d), n.id, n.rm_type);
        for c in &n.children {
            dump(c, d + 1, out);
        }
    }
    const ARCH: &str = include_str!("../../../../corpus/fixtures/adl2/archetype/cnf_count_a.adls");
    const TMPL: &str = include_str!("../../../../corpus/fixtures/adl2/opt/flat_parity_a.adls");
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    svc.template_adl2_upload(ARCH.to_owned())
        .await
        .expect("upload the filler archetype");
    svc.template_adl2_upload(TMPL.to_owned())
        .await
        .expect("upload the template");

    let wt = svc
        .web_template("openEHR-EHR-COMPOSITION.cnf_adl2_flat_a.v1.0.0")
        .await
        .expect("the ADL2 template resolves on the commit path");
    let mut tree = String::new();
    dump(&wt.tree, 0, &mut tree);
    assert!(
        find(&wt.tree, "observation_one").is_some(),
        "the filled OBSERVATION must appear in the projected WebTemplate; tree:\n{tree}"
    );
    assert!(
        find(&wt.tree, "count_item").is_some(),
        "the filler's constrained leaf must appear in the projected WebTemplate"
    );
}
