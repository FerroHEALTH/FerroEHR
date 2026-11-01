// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The stored release an ADL2 archetype reference resolves to, with several
//! releases of one archetype stored, against a real `PostgreSQL` 18
//! (shared testkit harness).
//!
//! BASE Artefact Identification
//! (`BASE/docs/artefact_identification/master07-referencing.adoc`
//! §Referencing): an interface reference, the identifier down to its major
//! version, "will match the latest release available of that major version".

#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use ferroehr::service::FerroEhrService;
use ferroehr::service::error::ServiceError;

const FAMILY: &str = "openEHR-EHR-OBSERVATION.res_obs";
const TEMPLATE: &str = "openEHR-EHR-COMPOSITION.res_uses_v1.v1.0.0";

/// An ADL2 archetype of `FAMILY` at `version` whose root term names it.
fn release(version: &str) -> String {
    format!(
        "archetype (adl_version=2.0.6; rm_release=1.1.0)\n    {FAMILY}.v{version}\n\n\
         language\n    original_language = <[ISO_639-1::en]>\n\n\
         description\n    lifecycle_state = <\"published\">\n    details = <\n        \
         [\"en\"] = <\n            language = <[ISO_639-1::en]>\n        >\n    >\n\n\
         definition\n    OBSERVATION[id1] matches {{ *}}\n\n\
         terminology\n    term_definitions = <\n        [\"en\"] = <\n            \
         [\"id1\"] = <text = <\"Release {version}\"> description = <\"Root.\">>\n        \
         >\n    >\n"
    )
}

/// An ADL2 template `hrid` filling its content with `reference`.
fn template_using(hrid: &str, reference: &str) -> String {
    format!(
        "template (adl_version=2.0.6; rm_release=1.1.0)\n    {hrid}\n\n\
         language\n    original_language = <[ISO_639-1::en]>\n\n\
         description\n    lifecycle_state = <\"published\">\n    details = <\n        \
         [\"en\"] = <\n            language = <[ISO_639-1::en]>\n        >\n    >\n\n\
         definition\n    COMPOSITION[id1] matches {{\n        \
         content cardinality matches {{0..*; unordered}} matches {{\n            \
         use_archetype OBSERVATION[id2, {reference}]\n        }}\n    }}\n\n\
         terminology\n    term_definitions = <\n        [\"en\"] = <\n            \
         [\"id1\"] = <text = <\"Root\"> description = <\"Root.\">>\n            \
         [\"id2\"] = <text = <\"Filler\"> description = <\"Filler.\">>\n        >\n    >\n"
    )
}

/// A service over a fresh database holding `versions` of `FAMILY`, uploaded
/// in that order.
async fn store(versions: &[&str]) -> (testkit::TestDb, FerroEhrService) {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    for version in versions {
        svc.template_adl2_upload(&release(version))
            .await
            .unwrap_or_else(|e| panic!("upload {FAMILY}.v{version}: {e:?}"));
    }
    (db, svc)
}

/// The operational template of `TEMPLATE`, as canonical JSON.
async fn compiled(svc: &FerroEhrService) -> String {
    svc.template_adl2_opt_json(TEMPLATE.to_owned(), None)
        .await
        .expect("the template compiles")
        .payload
}

/// A template referencing `….v1` compiles against release 1.10.0, the newest
/// of major version 1 by number, never 1.9.0 nor 2.0.0, in either upload
/// order.
#[tokio::test]
async fn an_interface_reference_resolves_to_the_newest_release_of_its_major_version() {
    let ascending = ["1.0.0", "1.9.0", "1.10.0", "2.0.0"];
    let mut descending = ascending;
    descending.reverse();
    for order in [ascending, descending] {
        let (_db, svc) = store(&order).await;
        svc.template_adl2_upload(&template_using(TEMPLATE, &format!("{FAMILY}.v1")))
            .await
            .unwrap_or_else(|e| panic!("upload the template after {order:?}: {e:?}"));
        let opt = compiled(&svc).await;
        assert!(opt.contains("Release 1.10.0"), "{order:?}: {opt}");
        for other in ["Release 1.0.0", "Release 1.9.0", "Release 2.0.0"] {
            assert!(!opt.contains(other), "{order:?} resolved {other}: {opt}");
        }
    }
}

/// A reference to a major version no stored release carries resolves to
/// nothing, although other major versions of its archetype are stored.
#[tokio::test]
async fn a_reference_to_an_absent_major_version_is_refused() {
    let (_db, svc) = store(&["1.0.0", "2.0.0"]).await;
    let refused = svc
        .template_adl2_upload(&template_using(TEMPLATE, &format!("{FAMILY}.v3")))
        .await
        .expect_err("no release of major version 3 is stored");
    assert!(
        matches!(&refused, ServiceError::ValidationFailed(v) if v.iter().any(|v| v.path == "VARXR")),
        "got {refused:?}"
    );
}

/// A release stored beside its release candidate answers every read that
/// does not name the candidate in full, in either upload order; the full
/// candidate version reads the candidate (BASE `master07-referencing.adoc`
/// §Referencing: "a pre-release version is referenced by its full
/// identifier").
#[tokio::test]
async fn a_release_outranks_its_release_candidate_in_either_upload_order() {
    let release_id = format!("{FAMILY}.v1.0.0");
    let candidate_id = format!("{FAMILY}.v1.0.0-rc.1");
    for order in [["1.0.0", "1.0.0-rc.1"], ["1.0.0-rc.1", "1.0.0"]] {
        let (_db, svc) = store(&order).await;
        let reads: [(String, Option<&str>, &str); 7] = [
            (format!("{FAMILY}.v1"), None, &release_id),
            (format!("{FAMILY}.v1.0"), None, &release_id),
            (FAMILY.to_owned(), None, &release_id),
            (FAMILY.to_owned(), Some("1"), &release_id),
            (FAMILY.to_owned(), Some("1.0"), &release_id),
            (FAMILY.to_owned(), Some("1.0.0"), &release_id),
            (FAMILY.to_owned(), Some("1.0.0-rc.1"), &candidate_id),
        ];
        for (template_id, version, want) in reads {
            let read = svc
                .template_adl2_source(template_id.clone(), version.map(str::to_owned))
                .await
                .unwrap_or_else(|e| panic!("{order:?}: read {template_id} at {version:?}: {e:?}"));
            assert_eq!(read.hrid, want, "{order:?}: {template_id} at {version:?}");
        }
    }
}
