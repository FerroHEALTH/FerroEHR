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
//! End-to-end journey over justified and emergency access: the clinician
//! opens a record outside their care relationships on a justification, ends
//! it, opens it again as an emergency, and the administrator finds the
//! emergency with its reason in the record's access log.
//!
//! It runs under the seed's first policy version, which holds professionals
//! to a care relationship ([`common::policy::with_first_policy`]).

use crate::common;

use common::{Harness, env, fill, login_basic_as, main_text, submit, wait_main_contains};

/// States a justification of `purpose` with `reason` on the status page of
/// `ehr` and waits for its notice.
async fn justify(h: &Harness, ehr: &str, purpose: &str, reason: &str) {
    h.goto(&format!("/ehrs/{ehr}/status")).await;
    wait_main_contains(h, "Open this record on a justification").await;
    common::choose(h, "#j-purpose", purpose).await;
    fill(h, "#j-reason", reason).await;
    submit(h, "form:has(#j-reason) button[type=submit]").await;
    wait_main_contains(
        h,
        &format!("You opened this record on a justification ({purpose})"),
    )
    .await;
    assert!(
        main_text(h).await.contains(reason),
        "the notice repeats the reason"
    );
}

#[tokio::test]
async fn a_clinician_justifies_then_declares_an_emergency_and_the_log_keeps_the_reason() {
    let Some(h) = Harness::start("justified").await else {
        return;
    };
    let (Some(cdr), Some(ehr), Some(clinician), Some(clinician_pass)) = (
        common::cdr_url("justified"),
        env("UI_E2E_JUSTIFIED_EHR_ID"),
        env("UI_E2E_BASIC_USER"),
        env("UI_E2E_BASIC_PASS"),
    ) else {
        println!("SKIP justified: the seeded clinician env is unset (run scripts/ui-e2e.sh)");
        h.finish().await;
        return;
    };
    let (user, pass) = common::admin_credentials();
    let justified = common::fresh("Second opinion requested by the ward (synthetic)");
    let emergency = common::fresh("Unconscious on arrival (synthetic)");
    Box::pin(common::policy::with_first_policy(
        &cdr,
        (&user, &pass),
        async {
            login_basic_as(&h, &clinician, &clinician_pass).await;
            justify(&h, &ehr, "TREAT", &justified).await;
            h.shot(1, "justified").await;
            submit(&h, "main form:has(input[value='end-justification']) button").await;
            wait_main_contains(&h, "Open this record on a justification").await;
            h.shot(2, "ended").await;
            justify(&h, &ehr, "ETREAT", &emergency).await;
            h.shot(3, "emergency").await;
            common::sign_out(&h).await;

            login_basic_as(&h, &user, &pass).await;
            h.goto(&format!("/ehrs/{ehr}/access-log?justification=emergency"))
                .await;
            wait_main_contains(&h, "Emergency access").await;
            h.wait_css("main a.fui-row-link[href*='/access-log/']")
                .await
                .click()
                .await;
            wait_main_contains(&h, "Reason stated").await;
            assert!(
                main_text(&h).await.contains(&emergency),
                "the emergency's entry keeps the reason the clinician stated"
            );
            h.shot(4, "access-log-entry").await;
        },
    ))
    .await;

    h.assert_console_clean(&["Failed to load resource"]).await;
    h.finish().await;
}
