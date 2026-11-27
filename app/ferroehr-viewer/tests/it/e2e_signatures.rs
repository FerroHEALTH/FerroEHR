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
//! End-to-end journey over a version's signature and attestations: the
//! administrator opens the letter the seed's clinician attested and reads, on
//! its Versions tab, the kind of signature the CDR wrote and the attestation.
//!
//! The journeys' posture signs every version with the committed
//! ML-DSA-65+Ed25519 test key (`docker/viewer/e2e-env.yml`), and the seed's
//! clinician commits the letter with a signed attestation
//! (`scripts/sandbox/seed/records/attested-signed.json`).

use crate::common;

use common::{Harness, env, login_basic_as, main_text, submit_control, wait_main_contains};

/// The attested composition's name in the seed.
const LETTER: &str = "Cardiology outpatient letter";

#[tokio::test]
async fn admin_reads_a_versions_signature_and_its_attestation() {
    let Some(h) = Harness::start("signatures").await else {
        return;
    };
    let Some(ehr) = env("UI_E2E_SEEDED_EHR_ID") else {
        println!("SKIP signatures: UI_E2E_SEEDED_EHR_ID is unset (run scripts/ui-e2e.sh)");
        h.finish().await;
        return;
    };
    let (user, pass) = common::admin_credentials();
    login_basic_as(&h, &user, &pass).await;

    h.goto(&format!("/ehrs/{ehr}")).await;
    wait_main_contains(&h, LETTER).await;
    let row = h
        .wait_xpath(&format!(
            "//main//tr[contains(normalize-space(.), '{LETTER}')]//a[contains(@class, 'fui-row-link')]"
        ))
        .await;
    submit_control(&h, row).await;
    h.wait_url_contains("/compositions/").await;
    common::open_tab(&h, "versions").await;
    wait_main_contains(&h, "Attestations").await;
    let text = main_text(&h).await;
    for words in [
        "An OpenPGP signature, ML-DSA-65+Ed25519",
        "signed by the treating cardiologist",
    ] {
        assert!(
            text.contains(words),
            "the Versions tab shows {words:?}: {text}"
        );
    }
    h.shot(1, "signature-and-attestation").await;

    h.assert_console_clean(&["Failed to load resource"]).await;
    h.finish().await;
}
