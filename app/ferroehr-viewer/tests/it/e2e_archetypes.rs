// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

#![allow(
    clippy::panic,
    clippy::expect_used,
    reason = "a browser journey asserts by panicking, and the shared harness panics when a configured stack cannot be driven"
)]
#![allow(
    clippy::print_stdout,
    reason = "the skip-with-reason and progress lines ARE this suite's report"
)]
#![allow(
    unreachable_pub,
    dead_code,
    reason = "the shared `common` harness is compiled into every journey binary; each one drives a different subset of it"
)]
//! End-to-end journey over Archetypes: an ADL 1.4 archetype uploaded from a
//! file, listed, opened on its source and on its ADL 2 conversion (#6659).

use std::time::Duration;

use crate::common;

use common::{Harness, login_basic, poll_until_for, read_prop, submit, wait_main_contains};

/// The archetype the journey uploads, and the id its source declares.
const FIXTURE: &str = "adl14.medication.v1.adl";
const ARCHETYPE_ID: &str = "openEHR-EHR-INSTRUCTION.medication.v1";

/// Upload an archetype file, find it in the list, read its source and its
/// ADL 2 conversion.
#[tokio::test]
async fn an_uploaded_archetype_is_listed_and_converts() {
    let Some(h) = Harness::start("archetypes").await else {
        return;
    };
    login_basic(&h).await;
    let path = std::fs::canonicalize(format!(
        "{}/../../corpus/fixtures/archetypes/{FIXTURE}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("the archetype fixture exists");

    h.goto("/archetypes/upload").await;
    h.wait_css("#source-file")
        .await
        .send_keys(path.to_string_lossy().into_owned())
        .await;
    let read = poll_until_for(Duration::from_secs(15), async || {
        read_prop(&h, "#source-text", "value")
            .await
            .is_some_and(|value| !value.is_empty())
    })
    .await;
    assert!(read, "the chosen file never reached the source field");
    submit(&h, "main button[name='intent'][value='upload']").await;
    h.wait_url_contains(&format!("/archetypes/{ARCHETYPE_ID}"))
        .await;
    wait_main_contains(&h, "Medication order").await;
    h.shot(1, "archetype-source").await;

    h.goto("/archetypes").await;
    h.wait_css(&format!(
        "main a.fui-row-link[href='/archetypes/{ARCHETYPE_ID}']"
    ))
    .await
    .click()
    .await;
    h.wait_css("main a[href$='?tab=adl2']").await.click().await;
    wait_main_contains(&h, "adl_version=2").await;
    h.shot(2, "archetype-adl2").await;

    h.assert_console_clean(&[]).await;
    h.finish().await;
}
