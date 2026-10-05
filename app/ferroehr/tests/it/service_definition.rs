// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! SM-2 end-to-end tests for the Definitions native API against a real
//! `PostgreSQL` 18 (shared testkit harness): ADL 1.4 source archetypes
//! (`I_DEFINITION_ADL14`), OPTs (delegated to `template_store`), and registered
//! queries (`I_DEFINITION_QUERY`). Driven through the SM service traits exactly
//! as a protocol adapter would call them; the SM pre/post-conditions are the
//! assertions.
//!
//! Requires Docker. Each test owns its container (`Drop` removes it).

#![expect(
    clippy::panic,
    clippy::string_slice,
    let_underscore_drop,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use ferroehr::service::FerroEhrService;
use ferroehr::service::definition::types::TemplateListFilter;
use ferroehr::service::error::ServiceError;
use ferroehr::service::status::{CallStatusType, SmError};

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

// ── ADL 1.4 archetypes (I_DEFINITION_ADL14) ──────────────────────────────────

#[tokio::test]
async fn archetype_upload_get_list_match_replace_delete() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let adl = fixture(ARCHETYPE_REL);

    // Precondition: not present yet.
    assert!(!svc.has_archetype(ARCHETYPE_ID.to_owned()).await.unwrap());
    assert_eq!(svc.archetypes_count_adl14().await.unwrap(), 0);

    // valid_archetype on good vs bad source.
    assert!(svc.valid_archetype(&adl).unwrap());
    assert!(!svc.valid_archetype("not an archetype").unwrap());

    // upload → Post_has_archetype.
    svc.upload_archetype(adl.clone()).await.expect("upload");
    assert!(svc.has_archetype(ARCHETYPE_ID.to_owned()).await.unwrap());
    assert_eq!(svc.archetypes_count_adl14().await.unwrap(), 1);

    // get returns the source verbatim.
    let got = svc
        .get_archetype(ARCHETYPE_ID.to_owned())
        .await
        .expect("get");
    assert_eq!(got, adl, "stored ADL source is byte-identical");

    // list + list_matching (regex).
    let list = svc.list_archetypes_adl14(Page::all()).await.unwrap();
    assert_eq!(list, vec![ARCHETYPE_ID.to_owned()]);
    let matched = svc
        .list_matching_archetypes("COMPOSITION\\.prescription".to_owned(), Page::all())
        .await
        .unwrap();
    assert_eq!(matched, vec![ARCHETYPE_ID.to_owned()]);
    let none = svc
        .list_matching_archetypes("OBSERVATION".to_owned(), Page::all())
        .await
        .unwrap();
    assert!(none.is_empty());

    // upload_archetype replaces if the id already exists (spec: "replace it").
    let replacement = format!("{adl}\n-- replaced\n");
    svc.upload_archetype(replacement.clone())
        .await
        .expect("replace");
    assert_eq!(
        svc.archetypes_count_adl14().await.unwrap(),
        1,
        "replace, not insert"
    );
    assert_eq!(
        svc.get_archetype(ARCHETYPE_ID.to_owned()).await.unwrap(),
        replacement,
        "replacement overwrote the source"
    );

    // delete → Post_archetype_removed.
    svc.delete_archetype(ARCHETYPE_ID.to_owned())
        .await
        .expect("delete");
    assert!(!svc.has_archetype(ARCHETYPE_ID.to_owned()).await.unwrap());
    assert_eq!(svc.archetypes_count_adl14().await.unwrap(), 0);
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

    assert!(
        svc.valid_archetype(&adl).unwrap(),
        "a 1.4 archetype with a revision_history section is valid"
    );
    svc.upload_archetype(adl.clone()).await.expect("upload");
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

    // Invalid ADL → 422 reporting invalid_archetype (i_definition_adl14.adoc
    // §upload_archetype .Errors).
    let bad = svc
        .upload_archetype("this is not an archetype".to_owned())
        .await
        .expect_err("invalid archetype rejected");
    assert!(
        matches!(
            bad,
            SmError {
                status: CallStatusType::InvalidArchetype,
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
            SmError {
                status: CallStatusType::ArtefactDoesNotExist,
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
            SmError {
                status: CallStatusType::ArtefactDoesNotExist,
                ..
            }
        ),
        "got {del_missing:?}"
    );

    // Uncompilable regex → 400 invalid_id_pattern (unbalanced group) — the
    // status `i_definition_adl14.adoc` §`list_matching_archetypes` declares
    // (.Errors: `invalid_id_pattern`), not the generic
    // `precondition_violation`.
    let bad_re = svc
        .list_matching_archetypes("(".to_owned(), Page::all())
        .await
        .expect_err("invalid pattern");
    assert!(
        matches!(
            bad_re,
            SmError {
                status: CallStatusType::InvalidIdPattern,
                ..
            }
        ),
        "got {bad_re:?}"
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
    assert!(
        !svc.valid_archetype(&bad).unwrap(),
        "a VARDT-violating 1.4 source must be invalid"
    );
    let err = svc
        .upload_archetype(bad)
        .await
        .expect_err("VARDT-violating upload rejected");
    // The rule-code mnemonic is carried through as the validation detail.
    assert!(
        matches!(
            err,
            SmError {
                status: CallStatusType::InvalidArchetype,
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
    assert_eq!(svc.archetypes_count_adl14().await.unwrap(), 0);
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

    let adl2 = svc
        .adl14_convert_to_adl2(ARCHETYPE_ID.to_owned())
        .await
        .expect("convert stored 1.4 archetype to ADL2");
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
            SmError {
                status: CallStatusType::ArtefactDoesNotExist,
                ..
            }
        ),
        "got {missing:?}"
    );
}

// ── ADL 1.4 OPTs (I_DEFINITION_ADL14; on template_store, UUID-keyed) ──────────

#[tokio::test]
async fn opt_upload_has_get_list_match_delete() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let xml = fixture(OPT_REL);

    // valid_opt on good vs bad XML.
    assert!(svc.valid_opt(&xml).unwrap());
    assert!(!svc.valid_opt("<not-a-template/>").unwrap());
    assert_eq!(svc.opts_count_adl14().await.unwrap(), 0);

    // upload_opt (Pre_valid).
    svc.upload_opt(xml.clone()).await.expect("upload opt");
    assert_eq!(svc.opts_count_adl14().await.unwrap(), 1);

    // list_opts yields the OPT's UUID; use it for has/get/delete (UUID-keyed).
    let opts = svc.list_opts_adl14(Page::all()).await.unwrap();
    assert_eq!(opts.len(), 1);
    let uuid = opts[0].clone();
    assert!(
        uuid::Uuid::parse_str(&uuid).is_ok(),
        "list_opts returns a UUID"
    );
    assert!(svc.has_opt(uuid.clone()).await.unwrap());
    let got = svc.get_opt(uuid.clone()).await.expect("get opt");
    assert_eq!(got, xml, "stored OPT XML is byte-identical");

    // list_matching_opts matches on the template_id (spec return-type defect —
    // we return template ids, per the NOTE).
    let matched = svc
        .list_matching_opts("IDCR.*Allergies".to_owned(), Page::all())
        .await
        .unwrap();
    assert_eq!(matched, vec![OPT_TEMPLATE_ID.to_owned()]);

    // Re-uploading the same template_id conflicts (wire's CNF-tested rule wins).
    let conflict = svc
        .upload_opt(xml.clone())
        .await
        .expect_err("duplicate opt");
    assert!(
        matches!(
            conflict,
            SmError {
                status: CallStatusType::Conflict,
                ..
            }
        ),
        "got {conflict:?}"
    );

    // delete_opt (Pre_has_opt / Post_opt_removed).
    svc.delete_opt(uuid.clone()).await.expect("delete opt");
    assert!(!svc.has_opt(uuid.clone()).await.unwrap());
    assert_eq!(svc.opts_count_adl14().await.unwrap(), 0);
}

/// The SM `delete_opt` path refuses (409) while a committed version still
/// references the template — the same integrity guard as the admin wire
/// delete, so a physical delete never orphans clinical data. The SM operation
/// itself (`i_definition_adl14.adoc` §`delete_opt`) defines only
/// `Pre_has_opt`/`invalid_template` and is silent here — the refusal is our
/// own integrity design. Pointing an existing `version` at the template
/// exercises the FK-reference guard directly (lighter than a full validated
/// commit, which the guard does not need).
#[tokio::test]
async fn opt_delete_refuses_while_referenced() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    svc.upload_opt(fixture(OPT_REL)).await.expect("upload opt");
    let opts = svc.list_opts_adl14(Page::all()).await.unwrap();
    assert_eq!(opts.len(), 1);
    let uuid = opts[0].clone();

    // Reference the template from a committed version row.
    let ehr: uuid::Uuid = svc.create_ehr(None).await.expect("ehr").into();
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
    let res = svc.delete_opt(uuid.clone()).await;
    match res {
        Err(SmError {
            status: CallStatusType::Conflict,
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
        svc.has_opt(uuid.clone()).await.unwrap(),
        "a refused delete leaves the template in place"
    );

    // Dereference → the delete succeeds (Post_opt_removed).
    sqlx::query("UPDATE version SET template_id = NULL WHERE ehr_id = $1")
        .bind(ehr)
        .execute(&pool)
        .await
        .expect("dereference template");
    svc.delete_opt(uuid.clone()).await.expect("delete opt");
    assert!(!svc.has_opt(uuid).await.unwrap());
}

/// The ITS-REST `definition_template_adl1.4_list` filter + pagination params
/// (`operations/definition_template_adl1.4_list.yaml`;
/// `parameters/query/filter_template_id.yaml` — "supports wildcards `*`";
/// `master02-overview.adoc` §List Handling). Two OPTs are uploaded through the
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
            SmError {
                status: CallStatusType::PreconditionViolation,
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
        // `misc::`-assumed qualified name (SM master04 §Registered Queries);
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
                SmError {
                    status: CallStatusType::PreconditionViolation,
                    ..
                }
            ),
            "`{name}` rejects as precondition_violation (wire 400), got {err:?}"
        );
    }

    // A three-part name's MIDDLE `aql` is the formalism segment (SM master04
    // §Registered Queries scheme 2), not the query-name — never reserved.
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

    // Invalid OPT → 422 reporting invalid_template (i_definition_adl14.adoc
    // §upload_opt .Errors).
    let bad = svc
        .upload_opt("<not-a-template/>".to_owned())
        .await
        .expect_err("invalid opt");
    assert!(
        matches!(
            bad,
            SmError {
                status: CallStatusType::InvalidTemplate,
                ..
            }
        ),
        "got {bad:?}"
    );

    // Unparseable UUID → 400.
    let bad_uuid = svc
        .has_opt("not-a-uuid".to_owned())
        .await
        .expect_err("bad uuid");
    assert!(
        matches!(
            bad_uuid,
            SmError {
                status: CallStatusType::PreconditionViolation,
                ..
            }
        ),
        "got {bad_uuid:?}"
    );

    // Absent (well-formed) UUID → get/delete 404.
    let absent = uuid::Uuid::now_v7().to_string();
    let get_missing = svc.get_opt(absent.clone()).await.expect_err("absent opt");
    assert!(
        matches!(
            get_missing,
            SmError {
                status: CallStatusType::TemplateDoesNotExist,
                ..
            }
        ),
        "got {get_missing:?}"
    );
    let del_missing = svc.delete_opt(absent).await.expect_err("delete absent opt");
    assert!(
        matches!(
            del_missing,
            SmError {
                status: CallStatusType::TemplateDoesNotExist,
                ..
            }
        ),
        "got {del_missing:?}"
    );
}

// ── ADL2 artefacts (I_DEFINITION_ADL2; on adl2_artefact, HRID-keyed) ──────────

const ADL2_OPT_HRID: &str = "openEHR-EHR-COMPOSITION.t_clinical_info.v1.0.0";
const ADL2_ARCH_HRID: &str = "openEHR-EHR-OBSERVATION.bp.v1.0.0";
const ADL2_TMPL_HRID: &str = "openEHR-EHR-COMPOSITION.t_vitals.v2.0.0";

#[tokio::test]
async fn adl2_upload_get_list_by_kind_match_replace_delete() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    // Preconditions: empty, and valid_artefact on good vs bad source.
    assert_eq!(svc.artefacts_count().await.unwrap(), 0);
    let adl2_opt = adl2_source(
        "operational_template",
        ADL2_OPT_HRID,
        Some("openEHR-EHR-COMPOSITION.discharge.v1"),
    );
    let adl2_arch = adl2_source("archetype", ADL2_ARCH_HRID, None);
    let adl2_tmpl = adl2_source("template", ADL2_TMPL_HRID, None);
    assert!(svc.valid_artefact(&adl2_opt).unwrap());
    assert!(!svc.valid_artefact("this is not adl2").unwrap());

    // upload_artefact (OPT) → Post_has_artefact (keyed by ARCHETYPE_HRID).
    assert!(!svc.has_artefact(ADL2_OPT_HRID.to_owned()).await.unwrap());
    svc.upload_artefact(adl2_opt.clone())
        .await
        .expect("upload opt");
    assert!(svc.has_artefact(ADL2_OPT_HRID.to_owned()).await.unwrap());

    // get_artefact returns the source verbatim.
    assert_eq!(
        svc.get_artefact(ADL2_OPT_HRID.to_owned()).await.unwrap(),
        adl2_opt,
        "stored ADL2 source is byte-identical"
    );

    // Upload one artefact of each other kind.
    svc.upload_artefact(adl2_arch.clone())
        .await
        .expect("upload archetype");
    svc.upload_artefact(adl2_tmpl.clone())
        .await
        .expect("upload template");

    // Counts by concrete type. `archetypes_count`/`opts_count` (and
    // `list_archetypes`/`list_opts`) are declared on both the ADL 1.4 and ADL2
    // Definitions traits, so calls on the concrete service are qualified.
    assert_eq!(svc.artefacts_count().await.unwrap(), 3);
    assert_eq!(svc.archetypes_count_adl2().await.unwrap(), 1);
    assert_eq!(svc.templates_count().await.unwrap(), 1);
    assert_eq!(svc.opts_count_adl2().await.unwrap(), 1);

    // list_artefacts = all HRIDs; per-kind lists partition them.
    let all = svc.list_artefacts(Page::all()).await.unwrap();
    assert_eq!(all.len(), 3);
    assert!(all.contains(&ADL2_OPT_HRID.to_owned()));
    assert_eq!(
        svc.list_archetypes_adl2(Page::all()).await.unwrap(),
        vec![ADL2_ARCH_HRID.to_owned()]
    );
    assert_eq!(
        svc.list_templates_adl2(Page::all()).await.unwrap(),
        vec![ADL2_TMPL_HRID.to_owned()]
    );
    assert_eq!(
        svc.list_opts_adl2(Page::all()).await.unwrap(),
        vec![ADL2_OPT_HRID.to_owned()]
    );

    // list_matching_artefacts (regex on the HRID).
    let obs = svc
        .list_matching_artefacts("OBSERVATION".to_owned(), Page::all())
        .await
        .unwrap();
    assert_eq!(obs, vec![ADL2_ARCH_HRID.to_owned()]);

    // upload_artefact replaces if the HRID already exists (spec: "replace it").
    let replacement = format!("{adl2_opt}-- replaced\n");
    svc.upload_artefact(replacement.clone())
        .await
        .expect("replace");
    assert_eq!(
        svc.artefacts_count().await.unwrap(),
        3,
        "replace, not insert"
    );
    assert_eq!(
        svc.get_artefact(ADL2_OPT_HRID.to_owned()).await.unwrap(),
        replacement
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
    let bad = svc
        .upload_artefact("concept\nopenEHR-EHR-OBSERVATION.bp.v1.0.0".to_owned())
        .await
        .expect_err("invalid artefact rejected");
    assert!(
        matches!(
            bad,
            SmError {
                status: CallStatusType::PreconditionViolation,
                ..
            }
        ),
        "got {bad:?}"
    );

    // Recognised header but a malformed HRID fails the grammar → 400 too.
    let bad_hrid = svc
        .upload_artefact("archetype\nnot-an-hrid".to_owned())
        .await
        .expect_err("malformed hrid rejected");
    assert!(
        matches!(
            bad_hrid,
            SmError {
                status: CallStatusType::PreconditionViolation,
                ..
            }
        ),
        "got {bad_hrid:?}"
    );

    // get / delete of an absent artefact → 404.
    let missing = svc
        .get_artefact("openEHR-EHR-OBSERVATION.absent.v1.0.0".to_owned())
        .await
        .expect_err("absent artefact");
    assert!(
        matches!(
            missing,
            SmError {
                status: CallStatusType::ArtefactDoesNotExist,
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
            SmError {
                status: CallStatusType::ArtefactDoesNotExist,
                ..
            }
        ),
        "got {del_missing:?}"
    );

    // Uncompilable regex → 400 invalid_id_pattern (`i_definition_adl2.adoc`
    // §`list_matching_artefacts` .Errors: `invalid_id_pattern`).
    let bad_re = svc
        .list_matching_artefacts("(".to_owned(), Page::all())
        .await
        .expect_err("invalid pattern");
    assert!(
        matches!(
            bad_re,
            SmError {
                status: CallStatusType::InvalidIdPattern,
                ..
            }
        ),
        "got {bad_re:?}"
    );
}

// ── registered queries (I_DEFINITION_QUERY) ──────────────────────────────────

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "one full I_DEFINITION_QUERY lifecycle exercised end to end on a \
              single container"
)]
async fn query_valid_store_list_match_delete() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let good = "SELECT c FROM COMPOSITION c";

    // valid_query: formalism equivalence + parse.
    assert!(svc.valid_query(good, "aql").unwrap());
    assert!(svc.valid_query(good, "AQL::1").unwrap());
    assert!(!svc.valid_query(good, "cql").unwrap());
    assert!(!svc.valid_query("this is not aql", "AQL").unwrap());
    assert_eq!(svc.queries_count().await.unwrap(), 0);

    // store_query with no name → a generated `misc::q_<uuid>` name.
    let desc = svc
        .store_query(good.to_owned(), "aql".to_owned(), None)
        .await
        .expect("store generated");
    assert!(
        desc.qualified_query_name.starts_with("misc::q_"),
        "generated name: {}",
        desc.qualified_query_name
    );
    assert_eq!(desc.formalism, "aql");
    assert_eq!(desc.source.as_deref(), Some(good));
    assert!(desc.version.is_some());
    // Post: the generated query is registered.
    assert!(
        svc.has_query(desc.qualified_query_name.clone())
            .await
            .unwrap()
    );

    // store_query with an explicit qualified name.
    let named = svc
        .store_query(
            good.to_owned(),
            "AQL".to_owned(),
            Some("ehr::over_50".to_owned()),
        )
        .await
        .expect("store named");
    assert_eq!(named.qualified_query_name, "ehr::over_50");
    assert!(svc.has_query("ehr::over_50".to_owned()).await.unwrap());
    // A bare name gets the "misc" namespace applied.
    let _ = svc
        .store_query(
            good.to_owned(),
            "aql".to_owned(),
            Some("bare_name".to_owned()),
        )
        .await
        .expect("store bare");
    assert!(svc.has_query("bare_name".to_owned()).await.unwrap());
    assert!(svc.has_query("misc::bare_name".to_owned()).await.unwrap());

    // ── ONE canonical key across every surface (SM master04 §Registered
    // Queries: bare name → the "misc" namespace) ─────────────────────────────
    // The WIRE store of a bare name must land on the SAME row the SM calls,
    // the by-name wire GET, the list, and the admin delete address — the
    // former asymmetry keyed the wire store under ('', name) and made the
    // query undeletable/unfindable under the name it was created with.
    svc.query_store(
        "wire_bare".to_owned(),
        Some("1.0.0".to_owned()),
        "AQL".to_owned(),
        good.to_owned(),
    )
    .await
    .expect("wire store of a bare name");
    assert!(
        svc.has_query("wire_bare".to_owned()).await.unwrap(),
        "SM has_query sees the wire-stored bare name"
    );
    assert!(
        svc.has_query("misc::wire_bare".to_owned()).await.unwrap(),
        "…under its canonical misc:: form too"
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
        !svc.has_query("wire_bare".to_owned()).await.unwrap(),
        "deleted everywhere"
    );

    // queries_count = distinct qualified names (3 stored).
    assert_eq!(svc.queries_count().await.unwrap(), 3);

    // list_queries has all three.
    let all = svc.list_queries(Page::all()).await.unwrap();
    assert_eq!(all.len(), 3);

    // list_matching_queries: id regex on the qualified name.
    let ehr_only = svc
        .list_matching_queries("^ehr::".to_owned(), None, Page::all())
        .await
        .unwrap();
    assert_eq!(ehr_only.len(), 1);
    assert_eq!(ehr_only[0].qualified_query_name, "ehr::over_50");
    // artefact pattern scans the source: `COMPOSITION` matches all.
    let by_artefact = svc
        .list_matching_queries(".*".to_owned(), Some("COMPOSITION".to_owned()), Page::all())
        .await
        .unwrap();
    assert_eq!(by_artefact.len(), 3);
    let by_artefact_none = svc
        .list_matching_queries(".*".to_owned(), Some("OBSERVATION".to_owned()), Page::all())
        .await
        .unwrap();
    assert!(by_artefact_none.is_empty());

    // delete_query removes by name → Post_query_deleted.
    svc.delete_query("ehr::over_50".to_owned())
        .await
        .expect("delete");
    assert!(!svc.has_query("ehr::over_50".to_owned()).await.unwrap());
    assert_eq!(svc.queries_count().await.unwrap(), 2);
    // Deleting an absent query → 404.
    let del_missing = svc
        .delete_query("ehr::over_50".to_owned())
        .await
        .expect_err("delete absent");
    assert!(
        matches!(
            del_missing,
            SmError {
                status: CallStatusType::ArtefactDoesNotExist,
                ..
            }
        ),
        "got {del_missing:?}"
    );

    // store_query on invalid AQL → 422 invalid_query.
    let bad = svc
        .store_query("nonsense".to_owned(), "aql".to_owned(), None)
        .await
        .expect_err("invalid query rejected");
    assert!(
        matches!(
            bad,
            SmError {
                status: CallStatusType::InvalidQuery,
                ..
            }
        ),
        "got {bad:?}"
    );

    // Bad id-pattern regex → 400 invalid_id_pattern
    // (`i_definition_query.adoc` §`list_matching_queries`).
    let bad_re = svc
        .list_matching_queries("(".to_owned(), None, Page::all())
        .await
        .expect_err("bad regex");
    assert!(
        matches!(
            bad_re,
            SmError {
                status: CallStatusType::InvalidIdPattern,
                ..
            }
        ),
        "got {bad_re:?}"
    );
}

#[tokio::test]
async fn query_store_set_not_implemented() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    // store_query_set is a spec TODO → 501 (trait default, NOTE).
    let err = svc.store_query_set(None).expect_err("not implemented");
    assert!(
        matches!(
            err,
            SmError {
                status: CallStatusType::NotImplemented,
                ..
            }
        ),
        "got {err:?}"
    );
}

/// The REST adapter surface for ADL2 template upload: a duplicate HRID is a
/// conflict (`409_template_already_exists`, `definition-codegen.openapi.yaml`
/// `/definition/template/adl2` POST) and invalid source is a 400 — while the
/// SM-native `upload_artefact` keeps replace semantics (SM master04
/// `i_definition_adl2.adoc`: "replace it").
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

    // The SM-native upload still replaces.
    svc.upload_artefact(tmpl).await.expect("native replace");

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

// ── ADL2/OPT2 FLAT parity: commit-path resolver fallback (#269) ───────────────

/// The runtime `WebTemplate` resolver (`web_template`/`web_template_for`) falls
/// back to the ADL2/OPT2 store when a template id is not an ADL 1.4 template, so a
/// FLAT/STRUCTURED composition **commit** keyed to an ADL2-registered template
/// resolves and is validated — the ADL2 twin of the OPT 1.4 commit path. Before
/// #269 this 422'd "operational template not known" (the resolver read only the
/// OPT 1.4 `template_store`).
///
/// Validation runs through the same choke point every commit uses
/// (`content_valid` → `validate_for_commit` → `web_template_for` →
/// `validate_archetype_conformance`), so this also exercises the `v2_4`
/// archetype-conformance capture end-to-end.
#[tokio::test]
async fn adl2_template_resolves_on_the_commit_path_and_validates() {
    use serde_json::Value;

    const HRID: &str = "openEHR-EHR-COMPOSITION.commit_resolver.v1.0.0";

    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    let opt = adl2_source("operational_template", HRID, None);
    svc.upload_artefact(opt).await.expect("upload ADL2 OPT");

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

    // A composition declaring that template validates end-to-end through the same
    // per-commit choke point.
    let mut comp = openehr_sdt::flat::example::example_composition(
        &wt,
        openehr_sdt::flat::example::DetailLevel::Complete,
    );
    assert_eq!(
        comp.pointer("/archetype_details/template_id/value")
            .and_then(Value::as_str),
        Some(HRID),
        "the example declares the ADL2 template id"
    );
    assert!(
        svc.definitions_valid(&comp)
            .await
            .expect("definitions_valid"),
        "the declared ADL2 template is now known to the definitions service"
    );
    assert!(
        svc.content_valid(&comp).await.expect("content_valid"),
        "a self-consistent instance validates clean against the ADL2 template"
    );

    // A structurally RM-broken instance is rejected — proving the validation
    // passes actually run against the ADL2-resolved template
    // (COMPOSITION.composer [1], RM common `composition.adoc`).
    comp.as_object_mut()
        .expect("composition object")
        .remove("composer");
    assert!(
        !svc.content_valid(&comp).await.expect("content_valid"),
        "an RM-invalid instance is rejected on the ADL2-resolved commit path"
    );
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

    svc.upload_artefact(ARCH.to_owned())
        .await
        .expect("upload the filler archetype");
    svc.upload_artefact(TMPL.to_owned())
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

// ── I_VALIDITY_CHECKER: definitions_valid over archetype ids (#2987) ──────────

/// `definitions_valid` resolves every declared definition identifier —
/// "archetype and template identifiers" (SM `i_validity_checker.adoc`
/// §`definitions_valid`) — against the stored repositories: archetype ids
/// against BOTH the ADL 1.4 store and the ADL 2 store, template ids through
/// the commit-path template lookup, collected from `archetype_details` at any
/// depth.
#[tokio::test]
async fn definitions_valid_resolves_archetype_ids_against_the_stored_repositories() {
    const ADL2_HRID: &str = "openEHR-EHR-SECTION.validity_known.v1.0.0";

    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    svc.upload_archetype(fixture(ARCHETYPE_REL))
        .await
        .expect("upload the ADL 1.4 archetype");
    svc.upload_artefact(adl2_source("archetype", ADL2_HRID, None))
        .await
        .expect("upload the ADL 2 archetype");

    // A root declaring the stored ADL 1.4 archetype and a nested node
    // declaring the stored ADL 2 archetype: every id resolves → true.
    let content = |root_id: &str, nested_id: &str| {
        serde_json::json!({
            "_type": "COMPOSITION",
            "archetype_details": { "archetype_id": { "value": root_id } },
            "content": [{
                "_type": "SECTION",
                "archetype_details": { "archetype_id": { "value": nested_id } },
            }],
        })
    };
    assert!(
        svc.definitions_valid(&content(ARCHETYPE_ID, ADL2_HRID))
            .await
            .expect("definitions_valid"),
        "ids known to the ADL 1.4 and ADL 2 repositories both resolve"
    );

    // An unknown ROOT archetype id refuses, even with the nested id known.
    assert!(
        !svc.definitions_valid(&content(
            "openEHR-EHR-COMPOSITION.never_stored.v1",
            ADL2_HRID
        ))
        .await
        .expect("definitions_valid"),
        "an archetype id neither repository holds is unknown"
    );

    // An unknown NESTED archetype id refuses too: collection recurses past the
    // root (`master03-archetyped_package.adoc` — every archetype root node
    // carries its own ARCHETYPED).
    assert!(
        !svc.definitions_valid(&content(
            ARCHETYPE_ID,
            "openEHR-EHR-SECTION.never_stored.v1"
        ))
        .await
        .expect("definitions_valid"),
        "an unknown archetype id below the root is still checked"
    );

    // An unknown template id refuses (the pre-#2987 check, retained).
    let mut with_template = content(ARCHETYPE_ID, ADL2_HRID);
    with_template["archetype_details"]["template_id"] =
        serde_json::json!({ "value": "never stored template" });
    assert!(
        !svc.definitions_valid(&with_template)
            .await
            .expect("definitions_valid"),
        "an unknown template id is unknown"
    );

    // Content declaring no definition identifiers uses none → true.
    assert!(
        svc.definitions_valid(&serde_json::json!({ "_type": "COMPOSITION" }))
            .await
            .expect("definitions_valid"),
        "no declared identifiers means nothing is unknown"
    );
}
