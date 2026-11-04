// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An ADL 2 upload that constrains `DV_QUANTITY.property` is refused, and the
//! refusal names the ADL 1.4 route that converts without it (#4964).
//!
//! No RM release defines `DV_QUANTITY.property`
//! (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_types.dv_quantity.adoc`),
//! and AOM 2 has no `C_DV_QUANTITY`
//! (`AM/docs/AOM2/master04.3-constraint_model-second_order.adoc` §Tuple Constraints),
//! so VCARM refuses it (`AM/docs/AOM2/master04.5-constraint_model-class_definitions.adoc`).

use std::path::PathBuf;

use ferroehr::error::ServiceError;
use ferroehr::error::refusal::Invalid;
use ferroehr::service::FerroEhrService;

const PACK: &str = "../../corpus/archetypes/adl2/ckm-2013-12-09/entry/observation";
const HRID: &str = "openEHR-EHR-OBSERVATION.height.v1.0.0";

fn pack_file(name: &str) -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(PACK)
            .join(name),
    )
    .expect("the vendored CKM archetype")
}

#[tokio::test]
async fn an_adl2_quantity_property_is_refused_with_the_adl14_route() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));

    let err = svc
        .template_adl2_upload(&pack_file("openEHR-EHR-OBSERVATION.height.v1.0.0.adls"))
        .await
        .expect_err("an ADL 2 constraint on DV_QUANTITY.property is refused")
        .flattened();
    assert!(
        matches!(
            &err,
            ServiceError::Unprocessable {
                kind: Invalid::Content,
                ..
            }
        ),
        "VCARM is a content-invalid failure: {err:?}"
    );
    let message = err.message();
    assert!(message.contains("VCARM"), "{message}");
    assert!(
        message.contains("ADL 1.4 archetype-profile attribute C_DV_QUANTITY.property")
            && message.contains("no ADL 2 or RM release defines")
            && message.contains("upload the archetype's ADL 1.4 source instead"),
        "the refusal says what `property` is and what to upload: {message}"
    );
    assert!(
        message.contains("/property)"),
        "the refusal names the path: {message}"
    );
    assert!(
        !crate::fixtures::artefact_stored(&db.pool(), HRID).await,
        "a refused artefact is not stored"
    );

    // The route the refusal names: the ADL 1.4 twin uploads and converts to
    // ADL 2 without the property.
    let id = svc
        .upload_archetype(pack_file("openEHR-EHR-OBSERVATION.height.v1.adl"))
        .await
        .expect("the ADL 1.4 source uploads");
    let conversion = svc
        .adl14_convert_to_adl2(id)
        .await
        .expect("the 1.4 source converts");
    let adl2 = &conversion.roots.first().expect("one converted root").adl2;
    assert!(
        adl2.contains("DV_QUANTITY") && !adl2.contains("property matches"),
        "the converted archetype constrains DV_QUANTITY without property"
    );
}
