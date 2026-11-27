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
//! End-to-end journey over Server › Repository: the administrator dumps the
//! whole repository to an archive on the CDR's file system, loads it back
//! behind its confirmation, and reads that every record it already holds was
//! reported and skipped.
//!
//! The archive goes under `/tmp` of the CDR's container, the one path the
//! compose stack keeps writable (`docker-compose.yml`, `tmpfs`).

use crate::common;

use common::{Harness, fill, login_basic_as, main_text, submit, wait_main_contains};

#[tokio::test]
async fn admin_dumps_the_repository_and_loads_it_back() {
    let Some(h) = Harness::start("repository").await else {
        return;
    };
    let location = format!("/tmp/{}", common::fresh("ferroehr-e2e-dump"));
    let (user, pass) = common::admin_credentials();
    login_basic_as(&h, &user, &pass).await;

    common::follow_nav(&h, "/server").await;
    common::open_tab(&h, "repository").await;
    h.wait_url_contains("/server/repository").await;
    wait_main_contains(&h, "Location on the CDR").await;
    fill(&h, "#d-location", &location).await;
    h.shot(1, "dump").await;
    submit(&h, "main button[name='intent'][value='dump']").await;
    wait_main_contains(&h, "Every record moved.").await;
    h.shot(2, "dumped").await;

    fill(&h, "#l-location", &location).await;
    common::confirm_in_dialog(
        &h,
        "main button[commandfor='repository-load-confirm']",
        "#repository-load-confirm button[name='intent'][value='load']",
    )
    .await;
    wait_main_contains(&h, "Records not moved").await;
    let text = main_text(&h).await;
    assert!(
        text.contains("an EHR with this id already exists"),
        "a load into the repository it came from skips every EHR: {text}"
    );
    h.shot(3, "loaded").await;

    h.assert_console_clean(&["Failed to load resource"]).await;
    h.finish().await;
}
