// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end service tests for the ADMIN dump/load API against a real
//! `PostgreSQL` 18 (shared testkit harness).
//!
//! Spec: SM `I_ADMIN_DUMP_LOAD.export_ehrs`/`load_ehrs`
//! (`docs/specs/openehr/SM/docs/UML/classes/i_admin_dump_load.adoc`), with
//! `EXPORT_SPEC` (`export_spec.adoc`, `segment_split_size` kb) and
//! `DUMP_LOAD_FAIL_REPORT` (`dump_load_fail_report.adoc`). The acceptance
//! properties:
//!
//! 1. **Round-trip fidelity** — export N EHRs from a source repository, load the
//!    archive into a *fresh* database, and every EHR's content reads back
//!    byte-equal at the canonical JSON level (`EHR_STATUS` + directory), with the
//!    same version/row counts.
//! 2. **Duplicate-id failure path** — loading an EHR whose id already exists is
//!    recorded in a `DUMP_LOAD_FAIL_REPORT` (`dump_status = false`) and skipped,
//!    never a crash, and leaves the repository unchanged.
//! 3. **Promoted-column re-derivation** — the `ehr.subject_*` columns are a
//!    projection of `EHR_STATUS.subject` (RM ehr master04 §EHR Status), so the
//!    load re-derives them from the loaded status: a loaded EHR is found by the
//!    subject lookup (SM `I_EHR_SERVICE.get_ehrs_for_subject`,
//!    `operations/ehr_get_by_subject.yaml`) even when the archive's projection
//!    was stale, and a PARTIAL load carrying a subject this repository already
//!    holds is reported like a duplicate id, never silently duplicated
//!    (one EHR per subject).
//! 4. **Both `EXPORT_FORMAT` members** (`export_format.adoc`) — the
//!    `openehr_canonical_xml` archive externalizes each version payload as an
//!    `ORIGINAL_VERSION` document under the ITS-XML published `<version>` root
//!    (`its-xml-1.0.2-nsv1/ALL/Version.xsd`), and the round trip through
//!    storage-JSON → RM → canonical XML → RM → storage-JSON is byte-equal at
//!    the served-version level, in all three containers.

#![expect(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    let_underscore_drop,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use ferroehr::ids::EhrId;
use ferroehr::service::FerroEhrService;

use crate::admin_fixture::{
    archive_dir, count_for_ehr, repository, seed_full_ehr, truncate_to_half,
};
use crate::fixtures::{change_type, committer, composition, source_chain, uid, uv};
use crate::typed_body::typed;
use ferroehr::service::admin::types::{
    CompressionFormat, DumpLoadFailReport, ExportFormat, ExportSpec,
};
use ferroehr::service::ehr_index::types::SubjectRef;
use ferroehr::service::status::CallStatusType;
use openehr_its::rest::generated::common::UpdateVersion;

/// An `EXPORT_SPEC` in `logical_format`/`compression_format` whose
/// `segment_split_size` of one kilobyte forces several segment entries — the
/// segmenting path every container test wants exercised.
fn split_spec(
    logical_format: Option<ExportFormat>,
    compression_format: Option<CompressionFormat>,
) -> ExportSpec {
    ExportSpec {
        logical_format,
        compression_format,
        segment_split_size: 1,
    }
}

/// Assert that a dump/load operation reported no per-entity failure.
fn assert_clean(reports: &[DumpLoadFailReport], what: &str) {
    assert!(
        reports.is_empty(),
        "a clean {what} reports no failures, got {reports:?}"
    );
}

/// An `EHR_STATUS` whose `PARTY_SELF` subject carries an `external_ref` — RM
/// ehr master04 §EHR Status (the subject 0..1 identifies the EHR).
fn status_for_subject(subject_id: &str) -> Value {
    json!({
        "_type": "EHR_STATUS",
        "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                              "value": "openEHR-EHR-EHR_STATUS.generic.v1" },
            "rm_version": "1.2.0"
        },
        "name": { "_type": "DV_TEXT", "value": "EHR Status" },
        "subject": {
            "_type": "PARTY_SELF",
            "external_ref": {
                "_type": "PARTY_REF",
                "namespace": "patients",
                "type": "PERSON",
                "id": { "_type": "HIER_OBJECT_ID", "value": subject_id }
            }
        },
        "is_queryable": true,
        "is_modifiable": true
    })
}

/// The per-EHR row counts across the versioned-object tables (used to prove the
/// load reproduced the same storage shape).
async fn counts(pool: &PgPool, ehr_id: Uuid) -> (i64, i64, i64, i64) {
    (
        count_for_ehr(
            pool,
            "SELECT count(*) FROM version WHERE ehr_id = $1",
            ehr_id,
        )
        .await,
        count_for_ehr(pool, "SELECT count(*) FROM node WHERE ehr_id = $1", ehr_id).await,
        count_for_ehr(
            pool,
            "SELECT count(*) FROM contribution WHERE ehr_id = $1",
            ehr_id,
        )
        .await,
        count_for_ehr(
            pool,
            "SELECT count(*) FROM item_tag WHERE ehr_id = $1",
            ehr_id,
        )
        .await,
    )
}

/// Read the current `EHR_STATUS` and directory `FOLDER` of `ehr_id` as canonical
/// JSON, serialized in storage (jsonb-normalized) key order — the byte-equal
/// comparison surface.
async fn canonical_snapshot(svc: &FerroEhrService, ehr_id: EhrId) -> (String, String) {
    let status = svc
        .get_ehr_status_at_time(ehr_id, None)
        .await
        .expect("status read");
    let directory = svc
        .get_directory_at_time(ehr_id, None, None)
        .await
        .expect("directory read")
        .body;
    (
        serde_json::to_string(&status).expect("status json"),
        serde_json::to_string(&directory).expect("directory json"),
    )
}

#[tokio::test]
async fn export_then_load_into_fresh_db_round_trips_byte_equal() {
    let (_src_db, src_pool, source) = repository().await;
    let (_dst_db, dst_pool, target) = repository().await;

    // Seed two EHRs in the source repository.
    let ehr1 = seed_full_ehr(&source).await;
    let ehr2 = seed_full_ehr(&source).await;

    // Snapshot the source canonical content before export.
    let src1 = canonical_snapshot(&source, ehr1).await;
    let src2 = canonical_snapshot(&source, ehr2).await;
    let src_counts1 = counts(&src_pool, ehr1.into()).await;
    let src_counts2 = counts(&src_pool, ehr2.into()).await;

    // Export to a canonical-JSON archive (small segment size forces multiple
    // segment files, exercising the segmenting path).
    let dir = archive_dir();
    let export_reports = source
        .export_ehrs(dir.clone(), ExportSpec::canonical_json(1))
        .await
        .expect("export");
    assert_clean(&export_reports, "export");
    // The archive exists on disk (manifest + at least one segment).
    assert!(
        std::path::Path::new(&dir).join("manifest.json").exists(),
        "manifest written"
    );

    // Load into the fresh target repository.
    let load_reports = target.load_ehrs(dir.clone()).await.expect("load");
    assert_clean(&load_reports, "load into an empty repository");

    // Both EHRs read back byte-equal at the canonical JSON level.
    assert_eq!(
        canonical_snapshot(&target, ehr1).await,
        src1,
        "ehr1 content must round-trip byte-equal"
    );
    assert_eq!(
        canonical_snapshot(&target, ehr2).await,
        src2,
        "ehr2 content must round-trip byte-equal"
    );

    // And the storage shape (version/node/contribution/tag counts) matches.
    assert_eq!(counts(&dst_pool, ehr1.into()).await, src_counts1);
    assert_eq!(counts(&dst_pool, ehr2.into()).await, src_counts2);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn load_duplicate_ehr_ids_is_reported_not_fatal() {
    let (_src_db, _src_pool, source) = repository().await;
    let (_dst_db, dst_pool, target) = repository().await;

    let ehr1 = seed_full_ehr(&source).await;
    let ehr2 = seed_full_ehr(&source).await;

    let dir = archive_dir();
    source
        .export_ehrs(dir.clone(), ExportSpec::canonical_json(1024))
        .await
        .expect("export");

    // First load into the empty target: both EHRs land, no failures.
    let first = target.load_ehrs(dir.clone()).await.expect("first load");
    assert_clean(&first, "first load");
    let after_first = counts(&dst_pool, ehr1.into()).await;

    // Second load of the same archive: every EHR id now already exists, so each
    // is recorded in a DUMP_LOAD_FAIL_REPORT (dump_status = false) and skipped —
    // no crash, no duplicate rows.
    let second = target.load_ehrs(dir.clone()).await.expect("second load");
    assert_eq!(second.len(), 2, "both EHRs reported as duplicates");
    for report in &second {
        assert_eq!(report.entity_type, "EHR");
        assert!(
            !report.dump_status,
            "a duplicate load fails for that entity"
        );
        assert!(report.error.is_some(), "with an explanatory error");
    }
    let reported: std::collections::BTreeSet<String> =
        second.iter().map(|r| r.entity_id.clone()).collect();
    assert!(reported.contains(&ehr1.to_string()));
    assert!(reported.contains(&ehr2.to_string()));

    // The repository is unchanged by the failed re-load (idempotent skip).
    assert_eq!(
        counts(&dst_pool, ehr1.into()).await,
        after_first,
        "a duplicate re-load must not add or remove rows"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn load_promotes_the_subject_from_the_loaded_status() {
    // The promoted `ehr.subject_*` columns are only a projection of
    // `EHR_STATUS.subject` (RM ehr master04 §EHR Status), so the load re-derives
    // them from the loaded status instead of trusting the archived projection:
    // an archive whose projection is absent — the shape an EHR landed by a path
    // that never promoted has — still yields an EHR the subject lookup finds
    // (SM `I_EHR_SERVICE.get_ehrs_for_subject`).
    let (_src_db, src_pool, source) = repository().await;
    let (_dst_db, _dst_pool, target) = repository().await;

    let ehr = source
        .create_ehr(Some(typed(&status_for_subject("patient-dump-1"))))
        .await
        .expect("source EHR for the subject");

    // Fixture: clear the promoted projection, leaving the subject only in the
    // EHR_STATUS content — what an EHR landed without promotion looks like.
    sqlx::query("UPDATE ehr SET subject_id = NULL, subject_namespace = NULL WHERE id = $1")
        .bind(Uuid::from(ehr))
        .execute(&src_pool)
        .await
        .expect("clear the promoted subject columns");
    assert!(
        source
            .get_ehrs_for_subject(SubjectRef::person("patient-dump-1", "patients"))
            .await
            .expect("source lookup")
            .is_empty(),
        "fixture: the source no longer finds the EHR by subject"
    );

    let dir = archive_dir();
    source
        .export_ehrs(dir.clone(), ExportSpec::canonical_json(1024))
        .await
        .expect("export");
    let reports = target.load_ehrs(dir.clone()).await.expect("load");
    assert_clean(&reports, "load");

    let found = target
        .get_ehrs_for_subject(SubjectRef::person("patient-dump-1", "patients"))
        .await
        .expect("target lookup");
    assert_eq!(
        found.len(),
        1,
        "the loaded EHR must be found by the subject its EHR_STATUS names"
    );
    assert_eq!(found[0].ehr_id, ehr.to_string());

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn load_reports_an_ehr_whose_subject_the_repository_already_holds() {
    // A dump of a self-consistent repository cannot clash with itself, but a
    // PARTIAL load into a NON-EMPTY repository can: one EHR per subject (RM ehr
    // master04 §EHR Status). The clash is reported per record
    // (`DUMP_LOAD_FAIL_REPORT`, `dump_status = false`) and skipped exactly like
    // a duplicate EHR id — never fatal, never a silent duplicate — and the rest
    // of the archive still loads.
    let (_src_db, _src_pool, source) = repository().await;
    let (_dst_db, _dst_pool, target) = repository().await;

    let clashing = source
        .create_ehr(Some(typed(&status_for_subject("patient-dump-2"))))
        .await
        .expect("source EHR for the subject");
    let innocent = seed_full_ehr(&source).await;

    // The target already holds that subject under a DIFFERENT EHR.
    let owner = target
        .create_ehr(Some(typed(&status_for_subject("patient-dump-2"))))
        .await
        .expect("target EHR for the same subject");

    let dir = archive_dir();
    source
        .export_ehrs(dir.clone(), ExportSpec::canonical_json(1024))
        .await
        .expect("export");
    let reports = target.load_ehrs(dir.clone()).await.expect("load");

    assert_eq!(
        reports.len(),
        1,
        "only the subject clash fails, got {reports:?}"
    );
    assert_eq!(reports[0].entity_type, "EHR");
    assert_eq!(reports[0].entity_id, clashing.to_string());
    assert!(!reports[0].dump_status);
    let error = reports[0].error.as_deref().expect("an explanatory error");
    assert!(
        error.contains("patient-dump-2"),
        "the report must name the clashing subject, got: {error}"
    );

    // The clashing record was skipped whole; everything else loaded.
    assert!(
        !target.has_ehr(clashing).await.expect("has_ehr"),
        "a reported record must not be partially loaded"
    );
    assert!(
        target.has_ehr(innocent).await.expect("has_ehr"),
        "the rest of the archive still loads"
    );
    let found = target
        .get_ehrs_for_subject(SubjectRef::person("patient-dump-2", "patients"))
        .await
        .expect("target lookup");
    assert_eq!(found.len(), 1, "the subject still resolves to its holder");
    assert_eq!(found[0].ehr_id, owner.to_string());

    let _ = std::fs::remove_dir_all(&dir);
}

/// SM `compression_format.adoc` member `zip`: the archive is one `archive.zip`
/// container carrying the identical entry set, and `load_ehrs` — which is
/// passed no format (`i_admin_dump_load.adoc`: `load_ehrs(file_sys_loc)`) —
/// detects and reads it, round-tripping byte-equal.
#[tokio::test]
async fn zip_compressed_export_round_trips_through_the_detected_container() {
    let (_src_db, src_pool, source) = repository().await;
    let (_dst_db, dst_pool, target) = repository().await;

    let ehr = seed_full_ehr(&source).await;
    let src = canonical_snapshot(&source, ehr).await;
    let src_counts = counts(&src_pool, ehr.into()).await;

    let dir = archive_dir();
    let spec = split_spec(
        Some(ExportFormat::OpenehrCanonicalJson),
        Some(CompressionFormat::Zip),
    );
    let reports = source
        .export_ehrs(dir.clone(), spec)
        .await
        .expect("zip export");
    assert_clean(&reports, "export");

    // The packed container is the ONLY thing written — no loose manifest.
    let root = std::path::Path::new(&dir);
    assert!(root.join("archive.zip").is_file(), "archive.zip written");
    assert!(
        !root.join("manifest.json").exists(),
        "the packed form writes no loose manifest"
    );

    let load_reports = target.load_ehrs(dir.clone()).await.expect("zip load");
    assert_clean(&load_reports, "load into an empty repository");
    assert_eq!(
        canonical_snapshot(&target, ehr).await,
        src,
        "a zip round-trip must be byte-equal at the canonical JSON level"
    );
    assert_eq!(
        counts(&dst_pool, ehr.into()).await,
        src_counts,
        "the loaded target must hold the same row counts as the source"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// SM `compression_format.adoc` member `7z` (owner-approved 2026-07-29,
/// `sevenz-rust2`): the archive is one `archive.7z` container carrying the
/// identical entry set, and the format-less `load_ehrs` detects and reads it,
/// round-tripping byte-equal — the exact mirror of the `zip` sibling above.
#[tokio::test]
async fn sevenz_compressed_export_round_trips_through_the_detected_container() {
    let (_src_db, src_pool, source) = repository().await;
    let (_dst_db, dst_pool, target) = repository().await;

    let ehr = seed_full_ehr(&source).await;
    let src = canonical_snapshot(&source, ehr).await;
    let src_counts = counts(&src_pool, ehr.into()).await;

    let dir = archive_dir();
    let spec = split_spec(
        Some(ExportFormat::OpenehrCanonicalJson),
        Some(CompressionFormat::SevenZip),
    );
    let reports = source
        .export_ehrs(dir.clone(), spec)
        .await
        .expect("7z export");
    assert_clean(&reports, "export");

    // The packed container is the ONLY thing written — no loose manifest.
    let root = std::path::Path::new(&dir);
    assert!(root.join("archive.7z").is_file(), "archive.7z written");
    assert!(
        !root.join("manifest.json").exists(),
        "the packed form writes no loose manifest"
    );

    let load_reports = target.load_ehrs(dir.clone()).await.expect("7z load");
    assert_clean(&load_reports, "load into an empty repository");
    assert_eq!(
        canonical_snapshot(&target, ehr).await,
        src,
        "a 7z round-trip must be byte-equal at the canonical JSON level"
    );
    assert_eq!(
        counts(&dst_pool, ehr.into()).await,
        src_counts,
        "the loaded target must hold the same row counts as the source"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ── EXPORT_FORMAT.openehr_canonical_xml ──────────────────────────────────────
//
// The battery below covers the round trip for this member. The refusal branches
// are unrelated to the member set and keep their own tests — a value outside the
// enumeration never reaches this layer (the REST edge refuses it,
// `admin_extension_http.rs`) and a non-positive `segment_split_size` is proved
// below.

/// Every versioned-object kind an EHR-scoped dump can carry, in one EHR:
/// `EHR_STATUS` (two versions), a directory `FOLDER`, a `COMPOSITION` with two
/// versions, and a second `COMPOSITION` that is logically DELETED (a version
/// with no content at all — RM common master06 §Logical Deletion), plus an
/// item tag. Every version is server-signed by the default service config, so
/// the round trip also proves the payload survives the XML transit unchanged:
/// the served signature is a digest over the version's own canonical form.
///
/// Returns the EHR and the `OBJECT_VERSION_ID`s of every COMPOSITION version.
async fn seed_mixed_kind_ehr(svc: &FerroEhrService) -> (EhrId, Vec<String>) {
    let ehr = svc.create_ehr(None).await.expect("ehr");

    let mut status = svc
        .get_ehr_status_at_time(ehr, None)
        .await
        .expect("status get");
    let status_ovid = uid(&status).to_owned();
    let status_vo = status_ovid.split("::").next().unwrap().to_owned();
    status.as_object_mut().expect("status obj").remove("uid");
    svc.target_tags_replace(
        ehr,
        status_vo,
        "EHR_STATUS",
        vec![crate::item_tag_fixture::ehr_tag(
            "priority",
            Some("high"),
            None,
        )],
    )
    .await
    .expect("tag");

    svc.create_directory(
        ehr,
        uv(
            &json!({ "_type": "FOLDER", "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1", "name": { "_type": "DV_TEXT", "value": "root" } }),
            "249",
            None,
        ),
    )
    .await
    .expect("directory");

    let created = svc
        .create_composition(ehr, uv(&composition("encounter v1"), "249", None))
        .await
        .expect("create_composition")
        .version_uid();
    let vo: ferroehr::ids::VoId = created.split("::").next().unwrap().parse().expect("vo id");
    let updated = svc
        .update_composition(
            ehr,
            vo,
            uv(&composition("encounter v2"), "251", Some(&created)),
        )
        .await
        .expect("update_composition")
        .version_uid();

    // A second COMPOSITION, logically deleted: the deleted version stores no
    // node rows, so the archive must carry it with NO payload document.
    let doomed = svc
        .create_composition(ehr, uv(&composition("to be deleted"), "249", None))
        .await
        .expect("create_composition")
        .version_uid();
    let deleted = svc
        .delete_composition(ehr, &doomed.parse().expect("ovid"), None, None)
        .await
        .expect("delete_composition")
        .version_uid();

    // The EHR_STATUS' second version lands LAST: it deactivates the EHR
    // (`is_modifiable = false`, RM ehr master04 §EHR Active Status), which
    // forbids every content write above.
    status["is_modifiable"] = json!(false);
    svc.replace_ehr_status(ehr, uv(&status, "251", Some(&status_ovid)))
        .await
        .expect("status update");

    (ehr, vec![created, updated, doomed, deleted])
}

/// The full comparison surface of a mixed-kind EHR: the current `EHR_STATUS`
/// and directory (the shared [`canonical_snapshot`]) plus the complete served
/// `ORIGINAL_VERSION` of every COMPOSITION version — envelope, provenance,
/// signature and data.
async fn mixed_snapshot(svc: &FerroEhrService, ehr: EhrId, ovids: &[String]) -> Vec<String> {
    let (status, directory) = canonical_snapshot(svc, ehr).await;
    let mut out = vec![status, directory];
    for ovid in ovids {
        let ov = svc
            .composition_version_envelope(ehr, ovid.parse().expect("ovid"))
            .await
            .expect("composition ORIGINAL_VERSION");
        out.push(serde_json::to_string(&ov).expect("version json"));
    }
    out
}

/// Export/load one mixed-kind EHR as `openehr_canonical_xml` in `compression`
/// and assert the whole served surface is byte-equal afterwards.
async fn assert_xml_round_trip(compression: Option<CompressionFormat>, container: &str) {
    let (_src_db, src_pool, source) = repository().await;
    let (_dst_db, dst_pool, target) = repository().await;

    let (ehr, ovids) = seed_mixed_kind_ehr(&source).await;
    let src = mixed_snapshot(&source, ehr, &ovids).await;
    let src_counts = counts(&src_pool, ehr.into()).await;

    let dir = archive_dir();
    // The externalized version documents are per-document and never split.
    let spec = split_spec(Some(ExportFormat::OpenehrCanonicalXml), compression);
    let reports = source
        .export_ehrs(dir.clone(), spec)
        .await
        .expect("canonical-XML export");
    assert_clean(&reports, "export");

    let load_reports = target.load_ehrs(dir.clone()).await.expect("load");
    assert_clean(&load_reports, "load into an empty repository");

    assert_eq!(
        mixed_snapshot(&target, ehr, &ovids).await,
        src,
        "a canonical-XML round trip in the {container} container must be byte-equal at the \
         served-version level"
    );
    assert_eq!(
        counts(&dst_pool, ehr.into()).await,
        src_counts,
        "the loaded target must hold the same row counts as the source"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// `EXPORT_FORMAT.openehr_canonical_xml`, loose entries: storage-JSON → RM →
/// canonical XML → RM → storage-JSON is lossless over every versioned-object
/// kind an EHR owns.
#[tokio::test]
async fn canonical_xml_export_round_trips_byte_equal_over_a_mixed_kind_ehr() {
    Box::pin(assert_xml_round_trip(None, "loose")).await;
}

/// The logical format and the container are independent axes: the identical
/// entry set travels in `archive.zip`.
#[tokio::test]
async fn canonical_xml_export_round_trips_through_the_zip_container() {
    Box::pin(assert_xml_round_trip(Some(CompressionFormat::Zip), "zip")).await;
}

/// …and in `archive.7z`.
#[tokio::test]
async fn canonical_xml_export_round_trips_through_the_sevenz_container() {
    Box::pin(assert_xml_round_trip(
        Some(CompressionFormat::SevenZip),
        "7z",
    ))
    .await;
}

/// The archive's SHAPE, not just its round trip: the manifest records the
/// requested `EXPORT_FORMAT` member; every live version's payload is an
/// `ORIGINAL_VERSION` document under the ITS-XML published `<version>` root
/// (`its-xml-1.0.2-nsv1/ALL/Version.xsd` declares `<xs:element name="version"
/// type="VERSION"/>` over an ABSTRACT `VERSION`, so the instance must name its
/// concrete type with `xsi:type`); the skeleton carries no inline body; and a
/// logically-deleted version gets no document at all.
#[tokio::test]
async fn the_canonical_xml_archive_holds_original_version_documents_under_the_published_root() {
    let (_db, _pool, source) = repository().await;
    let (_ehr, ovids) = seed_mixed_kind_ehr(&source).await;

    let dir = archive_dir();
    source
        .export_ehrs(dir.clone(), ExportSpec::canonical_xml(1024))
        .await
        .expect("canonical-XML export");
    let root = std::path::Path::new(&dir);

    let manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("manifest.json")).expect("manifest"),
    )
    .expect("manifest json");
    assert_eq!(
        manifest["format"], "openehr_canonical_xml",
        "the manifest records the EXPORT_FORMAT member the export was written in"
    );

    // Every COMPOSITION version but the deleted one has a document; the
    // deleted one has none.
    let deleted = ovids.last().expect("the deleted version uid");
    for ovid in &ovids {
        let entry = root.join("versions").join(format!("{ovid}.xml"));
        if ovid == deleted {
            assert!(
                !entry.exists(),
                "a logically-deleted version stores no content, so it gets no document"
            );
            continue;
        }
        let xml = std::fs::read_to_string(&entry)
            .unwrap_or_else(|e| panic!("version document {}: {e}", entry.display()));
        assert!(xml.starts_with("<version "), "published root, got:\n{xml}");
        assert!(
            xml.contains(r#"xsi:type="ORIGINAL_VERSION""#),
            "the abstract VERSION root must name its concrete subtype, got:\n{xml}"
        );
        assert!(
            xml.contains(r#"xmlns="http://schemas.openehr.org/v1""#),
            "the archive always writes the released-STABLE nsv1 lineage, got:\n{xml}"
        );
        assert!(
            xml.contains("<commit_audit") && xml.contains("<uid"),
            "a complete ORIGINAL_VERSION envelope, got:\n{xml}"
        );
    }

    // The skeleton references the documents and inlines no payload.
    let segment = std::fs::read_to_string(root.join("segment-0000.json")).expect("segment");
    assert!(
        segment.contains("\"body_entry\":\"versions/"),
        "the skeleton references the externalized payloads"
    );
    assert!(
        !segment.contains("\"body\":"),
        "an openehr_canonical_xml skeleton carries no inline body"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// A corrupt version document is a PER-RECORD failure, not an operation
/// failure: the EHR it belongs to is reported in a `DUMP_LOAD_FAIL_REPORT`
/// (`dump_load_fail_report.adoc`) and skipped whole — nothing of it committed —
/// while the rest of the archive still loads. That is the same per-entity shape
/// the SM gives a duplicate EHR id ("import EHRs with duplicate EHR ids will
/// fail", `i_admin_dump_load.adoc`), which is what a payload belonging to
/// exactly one EHR warrants; a mangled MANIFEST or SEGMENT stays the
/// whole-operation `file_not_writable`, because neither belongs to one entity.
#[tokio::test]
async fn a_corrupt_version_document_reports_that_record_and_commits_nothing() {
    let (_src_db, _src_pool, source) = repository().await;
    let (_dst_db, dst_pool, target) = repository().await;

    let (spoiled, ovids) = seed_mixed_kind_ehr(&source).await;
    let intact = seed_full_ehr(&source).await;

    let dir = archive_dir();
    source
        .export_ehrs(dir.clone(), ExportSpec::canonical_xml(1024))
        .await
        .expect("canonical-XML export");

    // Truncate ONE document mid-element: readable bytes, unparseable XML. It
    // belongs to a known EHR, so the report is checked by identity.
    let victim = std::path::Path::new(&dir)
        .join("versions")
        .join(format!("{}.xml", ovids.first().expect("a version uid")));
    truncate_to_half(&victim);

    let reports = target.load_ehrs(dir.clone()).await.expect("load");
    assert_eq!(
        reports.len(),
        1,
        "only the record owning the corrupt document fails, got {reports:?}"
    );
    assert_eq!(reports[0].entity_type, "EHR");
    assert_eq!(reports[0].entity_id, spoiled.to_string());
    assert!(!reports[0].dump_status);
    assert!(
        reports[0]
            .error
            .as_deref()
            .expect("an explanatory error")
            .contains("versions/"),
        "the report must name the unreadable entry, got {:?}",
        reports[0].error
    );

    assert!(
        !ehr_row_exists(&dst_pool, spoiled.into()).await,
        "a reported record must not be partially loaded"
    );
    assert!(
        target.has_ehr(intact).await.expect("has_ehr"),
        "the rest of the archive still loads"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// `EXPORT_SPEC.segment_split_size` is `Integer [1..1]` in kb
/// (`export_spec.adoc`); a non-positive size names no segment size at all and
/// is a `precondition_violation`, distinct from the unrealized-member branch.
#[tokio::test]
async fn non_positive_segment_split_size_is_a_precondition_violation() {
    let (_db, _pool, service) = repository().await;

    let dir = archive_dir();
    let err = service
        .export_ehrs(
            dir.clone(),
            ExportSpec {
                logical_format: None,
                compression_format: None,
                segment_split_size: 0,
            },
        )
        .await
        .expect_err("a non-positive split size is refused");
    assert_eq!(err.status, CallStatusType::PreconditionViolation);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `load_ehrs` against a location holding neither container is the SM's own
/// `file_not_writable` (the only error `i_admin_dump_load.adoc` declares),
/// never a panic or a silent empty load.
#[tokio::test]
async fn load_from_a_location_with_no_archive_is_file_not_writable() {
    let (_db, _pool, service) = repository().await;

    let dir = archive_dir();
    let err = service
        .load_ehrs(dir.clone())
        .await
        .expect_err("no archive at the location");
    assert_eq!(err.status, CallStatusType::FileNotWritable);
    // The SERVER FILESYSTEM PATH never reaches the body: it is deployment
    // layout, and the caller supplied only the configured location name. The
    // path + the OS diagnostic ride the trace record instead.
    assert!(
        !err.message.contains(&dir),
        "the configured server path must not reach the wire body, got {err:?}"
    );
    assert!(
        !err.message.contains('/'),
        "no filesystem path fragment reaches the wire body, got {err:?}"
    );
}

/// Whether the target repository holds an EHR row — the no-partial-write proof
/// the corrupt-archive loads below assert.
async fn ehr_row_exists(pool: &PgPool, ehr_id: Uuid) -> bool {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM ehr WHERE id = $1)")
        .bind(ehr_id)
        .fetch_one(pool)
        .await
        .expect("ehr probe")
}

/// A CORRUPT archive is the same fact as an unreadable one: a `manifest.json`
/// that will not parse as this format leaves `file_sys_loc` holding no readable
/// archive, so the load is the SM's own `file_not_writable`
/// (`i_admin_dump_load.adoc` — the one error it declares) — never an
/// `exception` blaming the server for the caller's archive, and never a
/// partial load.
#[tokio::test]
async fn load_from_an_archive_with_a_mangled_manifest_is_file_not_writable() {
    let (_src_db, _src_pool, source) = repository().await;
    let (_dst_db, dst_pool, target) = repository().await;

    let ehr = seed_full_ehr(&source).await;
    let dir = archive_dir();
    let reports = source
        .export_ehrs(dir.clone(), ExportSpec::canonical_json(1024))
        .await
        .expect("export");
    assert!(reports.is_empty());

    // Truncate the manifest mid-object: readable bytes, unparseable JSON.
    let manifest = std::path::Path::new(&dir).join("manifest.json");
    truncate_to_half(&manifest);

    let err = target
        .load_ehrs(dir.clone())
        .await
        .expect_err("a mangled manifest is refused");
    assert_eq!(err.status, CallStatusType::FileNotWritable);
    // The caller's-archive defect NAMES THE ENTRY (that is what the caller can
    // act on) and nothing else: no server path, no serde offsets/field names.
    assert!(
        err.message.contains("manifest.json"),
        "the refusal names the offending archive entry, got {err:?}"
    );
    assert!(
        !err.message.contains(&dir),
        "the server path must not reach the wire body, got {err:?}"
    );
    assert!(
        !err.message.contains("column") && !err.message.contains("line "),
        "the serde diagnostic must not reach the wire body, got {err:?}"
    );
    assert!(
        !ehr_row_exists(&dst_pool, ehr.into()).await,
        "a refused load commits nothing"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// The packed half of the same law: a TRUNCATED `archive.zip` cannot be opened
/// as a container, which is `file_not_writable` with nothing loaded.
#[tokio::test]
async fn load_from_a_truncated_container_is_file_not_writable() {
    let (_src_db, _src_pool, source) = repository().await;
    let (_dst_db, dst_pool, target) = repository().await;

    let ehr = seed_full_ehr(&source).await;
    let dir = archive_dir();
    let spec = split_spec(
        Some(ExportFormat::OpenehrCanonicalJson),
        Some(CompressionFormat::Zip),
    );
    // The export step has failed once under parallel suite load with the
    // curated internal-database message (#3250); the chain names the cause.
    let reports = source
        .export_ehrs(dir.clone(), spec)
        .await
        .unwrap_or_else(|e| panic!("export: {}", source_chain(&e)));
    assert!(reports.is_empty());

    // Drop the ZIP central directory (the trailing bytes): the file still
    // exists, so detection picks it, but it is no longer an archive.
    let container = std::path::Path::new(&dir).join("archive.zip");
    truncate_to_half(&container);

    let err = target
        .load_ehrs(dir.clone())
        .await
        .expect_err("a truncated container is refused");
    assert_eq!(err.status, CallStatusType::FileNotWritable);
    assert!(
        !err.message.contains(&dir) && !err.message.contains("archive.zip"),
        "the container path must not reach the wire body, got {err:?}"
    );
    assert!(
        !ehr_row_exists(&dst_pool, ehr.into()).await,
        "a refused load commits nothing"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// The admin archive is lossless for an `IMPORTED_VERSION` too: a row committed by
/// an EHR-Extract import carries TWO acts — the local one on the wrapper and
/// the source system's inside the wrapped `ORIGINAL_VERSION` (RM common
/// master06 §Committal and Audits) — and a dump/load round trip must reproduce
/// both. Regression for #1679: before the wrapper existed there was only one
/// act to carry, so the archive had nothing to lose.
#[tokio::test]
async fn an_imported_version_round_trips_through_the_archive() {
    let (_src_db, _src_pool, source) = repository().await;
    let (_mid_db, _mid_pool, middle) = repository().await;
    let (_dst_db, _dst_pool, target) = repository().await;

    // Source → extract → import: the middle repository now holds imported rows.
    let ehr = seed_full_ehr(&source).await;
    let extract = {
        let mut extracts = source.extract_ehrs(ehr).await.expect("export_ehrs");
        openehr_its::json::from_canonical_value(&extracts.remove(0)).expect("EXTRACT")
    };
    middle.import_ehr(None, extract).await.expect("import_ehr");

    // Middle → archive → target.
    let dir = archive_dir();
    let export_reports = middle
        .export_ehrs(dir.clone(), ExportSpec::canonical_json(1024))
        .await
        .expect("export");
    assert_clean(&export_reports, "export");
    let load_reports = target.load_ehrs(dir.clone()).await.expect("load");
    assert_clean(&load_reports, "load into an empty repository");

    // The loaded copy serves the same IMPORTED_VERSION the middle repository
    // does — wrapper act and wrapped original alike.
    let status_vo = middle
        .versioned_ehr_status_response(ehr)
        .await
        .expect("VERSIONED_EHR_STATUS");
    let vo_id: ferroehr::ids::VoId = uid(&status_vo.body).parse().expect("container uid");
    let version = middle
        .ehr_status_revision_history(ehr)
        .await
        .expect("REVISION_HISTORY")["items"][0]["version_id"]["value"]
        .as_str()
        .expect("version id")
        .rsplit("::")
        .next()
        .expect("version_tree_id")
        .to_owned();

    let before = middle
        .ehr_status_version_envelope(ehr, vo_id, &version)
        .await
        .expect("pre-dump version");
    let after = target
        .ehr_status_version_envelope(ehr, vo_id, &version)
        .await
        .expect("post-load version");
    assert_eq!(before["_type"], json!("IMPORTED_VERSION"));
    assert_eq!(
        after, before,
        "the archive must reproduce the IMPORTED_VERSION wrapper AND its wrapped original"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// #1685: the archive carries the `vo_attestation` rows, so a restored version
/// keeps BOTH attestation classes — one committed WITH the version
/// (`UPDATE_VERSION.attestations`, inside the signed canonical form — RM
/// common master06 §Attestation "Signing content at committal" / §Digital
/// Signature) and one attached afterwards by a `666|attestation|`-only
/// CONTRIBUTION — and a strict `verify_on_read` of the restored version
/// passes, which it cannot if the at-committal attestation was dropped.
/// The at-committal CONTRIBUTION fixture of
/// [`attestations_round_trip_and_the_restored_signature_verifies`]: one
/// COMPOSITION version carrying an `UPDATE_ATTESTATION`.
fn attested_contribution() -> Value {
    json!({
        "_type": "CONTRIBUTION",
        "versions": [{
            "_type": "ORIGINAL_VERSION",
            "commit_audit": {
                "change_type": change_type("249", "creation"),
                "committer": committer("author")
            },
            "lifecycle_state": change_type("532", "complete"),
            "data": composition("attested encounter"),
            "attestations": [{
                "_type": "UPDATE_ATTESTATION",
                "change_type": change_type("666", "attestation"),
                "committer": committer("attesting clinician"),
                "reason": { "_type": "DV_TEXT", "value": "witnessed" },
                "is_pending": false,
                "proof": "proof-bytes"
            }]
        }],
        "audit": {
            "change_type": change_type("251", "modification"),
            "committer": committer("author")
        }
    })
}

/// The 666-only after-committal CONTRIBUTION attesting `ovid` — fixture of the
/// same test.
fn later_attest_contribution(ovid: &str) -> Value {
    json!({
        "versions": [{
            "preceding_version_uid": { "value": ovid },
            "lifecycle_state": change_type("532", "complete"),
            "commit_audit": {
                "change_type": change_type("666", "attestation"),
                "committer": committer("senior reviewer"),
                "reason": { "_type": "DV_TEXT", "value": "authorised" },
                "is_pending": false
            }
        }],
        "audit": {
            "change_type": change_type("666", "attestation"),
            "committer": committer("senior reviewer")
        }
    })
}

#[tokio::test]
async fn attestations_round_trip_and_the_restored_signature_verifies() {
    use ferroehr::versioning::signature::config::{Mode, SigningConfig, VerifyOnRead};
    use ferroehr::versioning::signature::signer::Signer;
    use std::sync::Arc;

    let config = SigningConfig {
        enabled: true,
        mode: Mode::Digest,
        key_path: None,
        key_passphrase: None,
        key_passphrase_file: None,
        retired_key_paths: Vec::new(),
        verify_on_read: Some(VerifyOnRead::Strict),
    };
    let (_src_db, src_pool, source) = repository().await;
    let source = source.with_signer(Arc::new(Signer::from_config(&config).expect("signer")));
    let (_dst_db, dst_pool, target) = repository().await;
    let target = target.with_signer(Arc::new(Signer::from_config(&config).expect("signer")));

    let ehr_id = source.create_ehr(None).await.expect("ehr");
    let contribution = attested_contribution();

    let created = source
        .create_ehr_contribution(ehr_id, contribution)
        .await
        .expect("contribution with an at-committal attestation");
    let ovid = created.body["versions"][0]["id"]["value"]
        .as_str()
        .expect("committed version uid")
        .to_owned();

    // (b) The same version attested AFTERWARDS by a 666-only CONTRIBUTION.
    let attest = later_attest_contribution(&ovid);
    source
        .create_ehr_contribution(ehr_id, attest)
        .await
        .expect("after-committal 666 attestation");

    let served_before = source
        .composition_version_envelope(ehr_id, ovid.parse().expect("ovid"))
        .await
        .expect("source read verifies");

    // Round trip.
    let dir = archive_dir();
    let export_reports = source
        .export_ehrs(dir.clone(), ExportSpec::canonical_json(1))
        .await
        .expect("export");
    assert_clean(&export_reports, "export");
    let load_reports = target.load_ehrs(dir.clone()).await.expect("load");
    assert_clean(&load_reports, "load");

    let row_counts = |pool: &PgPool| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM vo_attestation")
                .fetch_one(&pool)
                .await
                .expect("count")
        }
    };
    assert_eq!(
        row_counts(&dst_pool).await,
        row_counts(&src_pool).await,
        "every vo_attestation row is restored"
    );

    // The strict read is the integrity assertion: a restored version whose
    // at-committal attestation vanished cannot recompute its stored signature.
    let served_after = target
        .composition_version_envelope(ehr_id, ovid.parse().expect("ovid"))
        .await
        .expect("the restored version verifies under strict verify_on_read");
    let atts = served_after["attestations"]
        .as_array()
        .expect("restored attestations");
    assert_eq!(atts.len(), 2, "both attestation classes restore");
    assert_eq!(atts[0]["reason"]["value"], "witnessed");
    assert_eq!(atts[1]["reason"]["value"], "authorised");
    assert_eq!(
        served_after, served_before,
        "the restored served document is identical to the source's"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// An archived EHR and an archived standalone party both come back ARCHIVED:
/// the dumped `archived_at` and `archive_reason` survive, and the rows sit in
/// the cold partition rather than the hot one.
///
/// The two halves are one fact: a marker without the tier placement makes an
/// archived object queryable again, and the tier without the marker loses the
/// instant it was archived at. No openEHR spec governs archival — our own
/// design/extension.
#[tokio::test]
async fn an_archived_ehr_and_party_round_trip_with_their_marker_and_tier() {
    let (_src_db, src_pool, source) = repository().await;
    let (_dst_db, dst_pool, target) = repository().await;

    let ehr = seed_full_ehr(&source).await;
    let person_vo = Box::pin(source.create_party(wave_uv(&wave_person("Archie"))))
        .await
        .expect("create_party");
    source
        .archive_ehrs(vec![ehr.to_string()])
        .await
        .expect("archive the EHR");
    source
        .archive_parties(vec![person_vo.to_string()])
        .await
        .expect("archive the party");

    let src_ehr_marker = archive_marker(&src_pool, "clinical", ehr.0).await;
    let src_party_marker = archive_marker(&src_pool, "party", person_vo.0).await;
    assert!(src_ehr_marker.is_some(), "the EHR really is archived");
    assert!(src_party_marker.is_some(), "the party really is archived");

    let dir = archive_dir();
    assert_clean(
        &source
            .export_ehrs(dir.clone(), ExportSpec::canonical_json(64))
            .await
            .expect("export"),
        "export",
    );
    assert_clean(
        &target.load_ehrs(dir.clone()).await.expect("load"),
        "load into an empty repository",
    );

    assert_eq!(
        archive_marker(&dst_pool, "clinical", ehr.0).await,
        src_ehr_marker,
        "the EHR's archive marker and tier must survive the round trip"
    );
    assert_eq!(
        archive_marker(&dst_pool, "party", person_vo.0).await,
        src_party_marker,
        "the party's archive marker and tier must survive the round trip"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// The `(archived_at, archive_reason, tier)` of every archived versioned object
/// the id names in `schema`: the EHR's objects by `ehr_id`, a party by its own
/// `vo_id`. `None` when nothing under that id is archived.
///
/// The schema is named explicitly because the two pseudonymisation domains are
/// two schemas over one connection here, and only a pool's `search_path`
/// chooses between them.
async fn archive_marker(
    pool: &PgPool,
    schema: &'static str,
    id: Uuid,
) -> Option<Vec<(String, Option<String>, String)>> {
    // `schema` is one of two literals at the call sites, never input.
    let sql = format!(
        "SELECT archived_at::text, archive_reason, tier FROM {schema}.vo_head \
         WHERE (ehr_id = $1 OR vo_id = $1) AND archived_at IS NOT NULL \
         ORDER BY vo_id"
    );
    let rows: Vec<(String, Option<String>, String)> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_all(pool)
        .await
        .expect("archive markers");
    (!rows.is_empty()).then_some(rows)
}

/// An archive written before the storage rewrite is refused by VERSION, naming
/// the remedy, rather than failing somewhere inside the load on a column the
/// operator never heard of.
#[tokio::test]
async fn an_archive_from_before_the_storage_rewrite_is_refused_with_the_remedy() {
    let (_db, _pool, target) = repository().await;
    let dir = archive_dir();
    std::fs::create_dir_all(&dir).expect("archive dir");
    // A first-generation manifest: archive_version 1, and no segments, so the
    // refusal cannot be reached by an unrelated read failure.
    std::fs::write(
        std::path::Path::new(&dir).join("manifest.json"),
        br#"{"format":"openehr_canonical_json","archive_version":1,"segment_split_size_kb":64,
             "ehr_count":0,"segments":[]}"#,
    )
    .expect("write manifest");

    let error = target
        .load_ehrs(dir.clone())
        .await
        .expect_err("a pre-rewrite archive must be refused");
    let message = error.to_string();
    assert!(
        message.contains("archive_version 1") && message.contains("predates the storage rewrite"),
        "the refusal names the version it read and why: {message}"
    );
    assert!(
        message.contains("export it again"),
        "the refusal states the remedy: {message}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ── the demographic wave (issue #2457) ───────────────────────────────────────

/// A minimal PERSON body for the wave round-trip, its legal-name identity
/// carrying one family-name `ELEMENT`.
fn wave_person(name: &str) -> Value {
    crate::admin_fixture::person(
        name,
        &json!([{
            "_type": "ELEMENT",
            "archetype_node_id": "at0003",
            "name": { "_type": "DV_TEXT", "value": "family" },
            "value": { "_type": "DV_TEXT", "value": name }
        }]),
    )
}

/// A minimal `PARTY_RELATIONSHIP` body between two party containers, archetyped
/// like the PERSON containers the wave carries beside it.
fn wave_relationship(name: &str, source: &str, target: &str) -> Value {
    let mut relationship = crate::admin_fixture::party_relationship(name, source, target);
    relationship["archetype_details"] = json!({
        "_type": "ARCHETYPED",
        "archetype_id": { "_type": "ARCHETYPE_ID",
                          "value": "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.relationship.v1" },
        "rm_version": "1.1.0"
    });
    relationship
}

/// The wire-shaped commit envelope for a demographic create.
fn wave_uv<T: serde::de::DeserializeOwned>(data: &Value) -> UpdateVersion<T> {
    let v = json!({
        "lifecycle_state": {
            "value": "complete",
            "defining_code": { "terminology_id": { "value": "openehr" }, "code_string": "532" }
        },
        "data": data,
        "commit_audit": {
            "change_type": {
                "value": "creation",
                "defining_code": { "terminology_id": { "value": "openehr" }, "code_string": "249" }
            },
            "committer": { "_type": "PARTY_IDENTIFIED", "name": "wave tester" }
        }
    });
    openehr_its::json::from_canonical_value(&v).expect("UpdateVersion")
}

#[tokio::test]
async fn demographic_wave_round_trips_standalone_party_and_relationship() {
    let (_src_db, _src_pool, source) = repository().await;
    let (_dst_db, _dst_pool, target) = repository().await;

    // Seed an EHR (so the archive carries both waves), a standalone person and
    // a relationship whose source is that person's container.
    let ehr = seed_full_ehr(&source).await;
    let person_vo = Box::pin(source.create_party(wave_uv(&wave_person("Wava"))))
        .await
        .expect("create_party");
    let rel_vo = source
        .create_party_relationship(wave_uv(&wave_relationship(
            "parent-of",
            &person_vo.to_string(),
            "33333333-3333-4333-8333-333333333333",
        )))
        .await
        .expect("create_party_relationship");

    let src_person = source.get_party(person_vo).await.expect("source party");
    let src_rel = source
        .get_party_relationship(rel_vo)
        .await
        .expect("source relationship");
    let src_ehr_snapshot = canonical_snapshot(&source, ehr).await;

    let dir = archive_dir();
    let export_reports = source
        .export_ehrs(dir.clone(), ExportSpec::canonical_json(64))
        .await
        .expect("export");
    assert_clean(&export_reports, "export");

    let load_reports = target.load_ehrs(dir.clone()).await.expect("load");
    assert_clean(&load_reports, "load into an empty repository");

    // The EHR wave still round-trips, and the demographic wave restores the
    // standalone containers verbatim (the archive ⇒ repository identity the
    // module documents).
    assert_eq!(canonical_snapshot(&target, ehr).await, src_ehr_snapshot);
    assert_eq!(
        target.get_party(person_vo).await.expect("loaded party"),
        src_person,
        "the standalone PARTY must round-trip verbatim"
    );
    assert_eq!(
        target
            .get_party_relationship(rel_vo)
            .await
            .expect("loaded relationship"),
        src_rel,
        "the PARTY_RELATIONSHIP must round-trip verbatim"
    );

    // A second load reports the demographic duplicates per entity, not fatally.
    let dup_reports = target.load_ehrs(dir.clone()).await.expect("re-load");
    assert!(
        dup_reports
            .iter()
            .any(|r| r.entity_id == person_vo.to_string() && !r.dump_status),
        "the duplicate standalone party is reported: {dup_reports:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn demographic_wave_round_trips_in_canonical_xml() {
    let (_src_db, _src_pool, source) = repository().await;
    let (_dst_db, _dst_pool, target) = repository().await;

    let person_vo = Box::pin(source.create_party(wave_uv(&wave_person("Xara"))))
        .await
        .expect("create_party");
    let rel_vo = source
        .create_party_relationship(wave_uv(&wave_relationship(
            "cares-for",
            &person_vo.to_string(),
            "44444444-4444-4444-8444-444444444444",
        )))
        .await
        .expect("create_party_relationship");
    let src_person = source.get_party(person_vo).await.expect("source party");
    let src_rel = source
        .get_party_relationship(rel_vo)
        .await
        .expect("source relationship");

    let dir = archive_dir();
    let spec = split_spec(Some(ExportFormat::OpenehrCanonicalXml), None);
    let reports = source.export_ehrs(dir.clone(), spec).await.expect("export");
    assert_clean(&reports, "export");
    assert!(
        std::fs::read_dir(std::path::Path::new(&dir).join("versions"))
            .expect("versions dir")
            .count()
            >= 2,
        "the demographic versions are externalized as versions/*.xml"
    );

    let load_reports = target.load_ehrs(dir.clone()).await.expect("load");
    assert_clean(&load_reports, "load");

    assert_eq!(
        target.get_party(person_vo).await.expect("loaded party"),
        src_person,
        "the canonical-XML demographic wave must round-trip the PARTY verbatim"
    );
    assert_eq!(
        target
            .get_party_relationship(rel_vo)
            .await
            .expect("loaded relationship"),
        src_rel,
        "the canonical-XML demographic wave must round-trip the relationship verbatim"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn demographic_wave_round_trips_inside_a_7z_container() {
    let (_src_db, _src_pool, source) = repository().await;
    let (_dst_db, _dst_pool, target) = repository().await;

    let ehr = seed_full_ehr(&source).await;
    let person_vo = Box::pin(source.create_party(wave_uv(&wave_person("Zena"))))
        .await
        .expect("create_party");
    let rel_vo = source
        .create_party_relationship(wave_uv(&wave_relationship(
            "employs",
            &person_vo.to_string(),
            "55555555-5555-4555-8555-555555555555",
        )))
        .await
        .expect("create_party_relationship");
    let src_person = source.get_party(person_vo).await.expect("source party");
    let src_rel = source
        .get_party_relationship(rel_vo)
        .await
        .expect("source relationship");
    let src_ehr_snapshot = canonical_snapshot(&source, ehr).await;

    let dir = archive_dir();
    let spec = split_spec(None, Some(CompressionFormat::SevenZip));
    let reports = source.export_ehrs(dir.clone(), spec).await.expect("export");
    assert_clean(&reports, "export");
    let root = std::path::Path::new(&dir);
    assert!(root.join("archive.7z").is_file(), "archive.7z written");
    assert!(
        !root.join("manifest.json").exists(),
        "the compressed container holds every entry — no loose manifest"
    );

    let load_reports = target.load_ehrs(dir.clone()).await.expect("load");
    assert_clean(&load_reports, "load");

    assert_eq!(canonical_snapshot(&target, ehr).await, src_ehr_snapshot);
    assert_eq!(
        target.get_party(person_vo).await.expect("loaded party"),
        src_person,
        "the demographic wave must round-trip through the 7z container verbatim"
    );
    assert_eq!(
        target
            .get_party_relationship(rel_vo)
            .await
            .expect("loaded relationship"),
        src_rel,
        "the relationship must round-trip through the 7z container verbatim"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
