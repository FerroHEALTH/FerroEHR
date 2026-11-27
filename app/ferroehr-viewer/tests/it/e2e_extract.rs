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
//! End-to-end journey over EHR Extracts: an EHR the journey creates is
//! exported, and an extract another system wrote is imported under a fresh
//! id, which opens the new EHR (#6657).

use crate::common;

use common::{Harness, env, fill, login_basic_as};

/// A fresh UUID-shaped id from the clock, unique per run.
fn generated_uuid() -> String {
    let nanos = jiff::Timestamp::now().as_nanosecond().unsigned_abs();
    let low = u32::try_from(nanos & 0xffff_ffff).unwrap_or(1);
    let mid = u16::try_from((nanos >> 32) & 0xffff).unwrap_or(1);
    let node = u64::try_from((nanos >> 16) & 0xffff_ffff_ffff).unwrap_or(1);
    format!("{low:08x}-{mid:04x}-4000-8000-{node:012x}")
}

/// Export an EHR as an extract, and import another system's extract as a new
/// EHR.
///
/// The import is not the journey's own export: the CDR refuses an extract
/// whose committers are its own parties or logins unless a grant vouches for
/// the system that wrote it, and never for its own system.
#[tokio::test]
async fn an_exported_extract_imports_as_a_new_ehr() {
    let Some(cdr) = env("UI_E2E_CDR_URL") else {
        println!("SKIP: UI_E2E_CDR_URL unset (run scripts/ui-e2e.sh)");
        return;
    };
    let Some(h) = Harness::start("extract-export-import").await else {
        return;
    };
    // The seed grants the administrator import_ehr over EHRs with no subject.
    let user = env("UI_E2E_ADMIN_USER").unwrap_or_else(|| "ferroehr-admin".to_owned());
    let pass = env("UI_E2E_ADMIN_PASS").unwrap_or_else(|| "ferroehr".to_owned());
    login_basic_as(&h, &user, &pass).await;

    h.goto("/ehrs/new").await;
    common::submit(&h, "#editor-dock button[value='save']").await;
    h.wait_url_contains("/ehrs/").await;
    let url = h.current_url().await;
    let ehr_id = url
        .split("/ehrs/")
        .nth(1)
        .map(|rest| rest.split(['?', '/']).next().unwrap_or(rest).to_owned())
        .filter(|id| id.len() > 8)
        .expect("the create landed on /ehrs/{ehr_id}");
    h.wait_css(&format!("a[href='/download/extract/{ehr_id}']"))
        .await;

    let exported = reqwest::Client::new()
        .get(format!(
            "{cdr}/ferroehr/rest/extensions/v1/message/export/{ehr_id}"
        ))
        .basic_auth(&user, Some(&pass))
        .header("Accept", "application/json")
        .send()
        .await
        .expect("the CDR answers");
    let status = exported.status();
    let extract = exported.text().await.expect("the extract reads");
    assert!(
        status.is_success() && extract.contains(r#""_type":"EXTRACT""#),
        "the export answered {status}: {extract}"
    );

    let foreign = std::fs::read_to_string(format!(
        "{}/tests/fixtures/extract/foreign-ehr.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("the foreign extract fixture reads");
    let clone = generated_uuid();
    h.goto("/ehrs/import").await;
    h.wait_css("#source-text").await.paste(&foreign).await;
    fill(&h, "#x-ehr", &clone).await;
    common::submit(&h, "main button[name='intent'][value='import']").await;
    let target = format!("/ehrs/{clone}");
    if !common::poll_until(async || h.current_url().await.contains(&target)).await {
        let page = common::read_text(&h, "main").await.unwrap_or_default();
        panic!("the import never opened {target}; the page reads: {page}");
    }
    common::wait_text_contains(&h, ".fui-toast-text", "EHR imported").await;
    h.shot(1, "extract-imported").await;

    h.assert_console_clean(&[]).await;
    h.finish().await;
}
