// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The sandbox seed's archetype-library walk against the upload it drives.
//!
//! `scripts/sandbox/reseed.sh` offers the whole vendored 2013 CKM ADL 2 export
//! to `POST /definition/template/adl2` and counts each answer as stored or
//! refused against the pin in `scripts/sandbox/seed/manifest.json`. The status
//! it counts as a refusal is that pin's `refused_status`, so this test holds
//! the pin to the status the upload answers for a file the corpus gate
//! adjudicates as refused (`crates/openehr-adl/tests/it/ckm_archetype_packs.rs`).
use std::path::{Path, PathBuf};

use http::{StatusCode, header};
use serde_json::Value;

use crate::common;

const BASE: &str = "/ferroehr/rest/openehr/v1";

/// A library file the AOM2 validator refuses on VCARM: it constrains
/// `property` on `DV_QUANTITY`, which RM 1.2.0 does not define.
const REFUSED: &str = "openEHR-EHR-CLUSTER.ambient_oxygen.v1.0.0.adls";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The manifest's archetype-library pin.
fn library_pin() -> Value {
    let manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(repo_root().join("scripts/sandbox/seed/manifest.json"))
            .expect("read the sandbox seed manifest"),
    )
    .expect("the sandbox seed manifest is JSON");
    manifest["adl2"]["archetype_library"].clone()
}

/// The upload declares only 201, 400 and 409
/// (`operations/definition_template_adl2_upload.yaml`), so a library archetype
/// AOM2 refuses is a `400`, and the seed counts exactly that status as a
/// refusal (#5044: it counted `422`, and the v5.6.3 reseed stopped at the
/// first refused file).
#[tokio::test]
async fn the_seed_counts_a_refused_library_archetype_on_the_status_the_upload_answers() {
    let pin = library_pin();
    let root = pin["root"]
        .as_str()
        .expect("the manifest names the library root");
    let source = std::fs::read_to_string(repo_root().join(root).join("cluster").join(REFUSED))
        .expect("read the refused library archetype");

    let (_pg, app) = common::app(common::api_config(false)).await;
    let req = common::req("POST", format!("{BASE}/definition/template/adl2"))
        .header(header::CONTENT_TYPE, "text/plain")
        .prefer_representation()
        .body(source)
        .build();
    let (status, problem) = common::send_json(&app, req).await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    let errors = problem
        .get("errors")
        .and_then(Value::as_array)
        .expect("the problem carries its violations in errors");
    assert!(
        errors.iter().any(|e| e["path"].as_str() == Some("VCARM")),
        "{REFUSED} is refused on VCARM: {errors:?}"
    );
    assert_eq!(
        pin["refused_status"].as_u64(),
        Some(u64::from(status.as_u16())),
        "the manifest's refused_status, which reseed.sh counts as a library refusal, \
         is the status the upload answers for a refused archetype: {problem}"
    );
}
