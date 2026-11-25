// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A specialised ADL 1.4 archetype converts to its ADL 2 differential form,
//! over a real PostgreSQL 19 (shared testkit harness).
//!
//! ADL 1.4 authors a specialised archetype flat (AOM2
//! `master03-archetype_package.adoc` §Structural Variants), while ADL 2 source
//! is differential, holding only the codes of its own level (`ADL2/master09.02`
//! §Differential and Flat Forms; AOM2 VTSD), so the conversion is reduced
//! against the conversion of the stored parent.

use std::path::Path;

use ferroehr::error::ServiceError;
use ferroehr::service::FerroEhrService;

const PARENT: &str = "openEHR-EHR-COMPOSITION.report.v1.adl";
const CHILD: &str = "openEHR-EHR-COMPOSITION.report-result.v1.adl";

fn ckm(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/archetypes/ckm/adl14")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Uploads `name`, converts it and stores the conversion.
async fn convert_and_store(svc: &FerroEhrService, name: &str) -> Result<String, ServiceError> {
    let id = svc
        .upload_archetype(ckm(name))
        .await
        .unwrap_or_else(|e| panic!("upload {name}: {e:?}"));
    let conversion = svc.adl14_convert_to_adl2(id).await?;
    let root = conversion.roots.first().expect("one converted root");
    svc.template_adl2_upload(&root.adl2).await
}

#[tokio::test]
async fn a_specialised_14_archetype_stores_as_its_differential_conversion() {
    let (_db, svc) = crate::fixtures::service().await;
    convert_and_store(&svc, PARENT)
        .await
        .expect("the parent's conversion stores");
    convert_and_store(&svc, CHILD)
        .await
        .expect("the specialised archetype's differential conversion stores");
}

#[tokio::test]
async fn a_specialised_14_archetype_without_its_parent_is_refused() {
    let (_db, svc) = crate::fixtures::service().await;
    let id = svc
        .upload_archetype(ckm(CHILD))
        .await
        .expect("the specialised archetype uploads");
    match svc.adl14_convert_to_adl2(id).await {
        Err(e @ ServiceError::Unprocessable { .. }) => {
            assert!(
                format!("{e:?}").contains("openEHR-EHR-COMPOSITION.report.v1"),
                "the refusal names the missing parent: {e:?}"
            );
        }
        other => panic!("a conversion without the stored parent is refused, got {other:?}"),
    }
}
