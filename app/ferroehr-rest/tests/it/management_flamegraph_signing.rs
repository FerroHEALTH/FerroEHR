// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The CPU flamegraph of `/management/flamegraph` sampled while the server
//! signs (#5723).
//!
//! The profiler walks the frame-pointer chain and checks each read, so a sample
//! that interrupts SHA-512 inside an Ed25519 signature truncates a stack and
//! never ends the process. This test samples at the highest frequency the
//! endpoint admits while threads sign with an Ed25519 and an RSA-3072 key, and
//! asserts the window completes and its stacks reach the signer. No openEHR
//! spec governs the management surface: our own design.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ferroehr::config::management::ProfilingConfig;
use ferroehr::versioning::signature::config::{Mode, SigningConfig, VerifyOnRead};
use ferroehr::versioning::signature::signer::Signer;
use ferroehr_rest::extensions::management::flamegraph::{
    FlamegraphParams, ProfilerSlot, sample_flamegraph_svg,
};
use ferrohealth_openpgp_pq::encoding::{self, Encoding};
use ferrohealth_openpgp_pq::key::Key;

/// The data each signature is taken over, the size of a link digest.
const DIGEST: &[u8] = b"5e884898da28047151d0e56f8dc6292773603d0d6aabbdd62a11ef721d1542d8";

/// A signer over a freshly generated ML-DSA-65+Ed25519 `OpenPGP` key, written
/// to a file of the test's own.
fn signer(name: &str) -> Signer {
    let user_id = format!("flamegraph {name} <test@ferroehr.invalid>");
    let key = Key::generate(&[user_id.as_str()]).expect("a key");
    let armored = String::from_utf8(
        encoding::encode(std::slice::from_ref(key.signed()), Encoding::Armored)
            .expect("an armored key"),
    )
    .expect("armor is text");
    let path: PathBuf = std::env::temp_dir().join(format!(
        "ferroehr-flamegraph-{name}-{}.asc",
        std::process::id()
    ));
    std::fs::write(&path, armored).expect("the key file");
    Signer::from_config(&SigningConfig {
        enabled: true,
        mode: Mode::Pgp,
        key_file: Some(path),
        key_passphrase: None,
        key_passphrase_file: None,
        retired_key_files: Vec::new(),
        verify_on_read: Some(VerifyOnRead::Strict),
    })
    .expect("a signer")
}

#[tokio::test(flavor = "multi_thread")]
async fn the_flamegraph_samples_signing_at_its_highest_frequency_and_the_server_lives() {
    let signers = Arc::new([signer("first"), signer("second")]);
    let stop = Arc::new(AtomicBool::new(false));
    let workers: Vec<_> = (0..4_usize)
        .map(|n| {
            let (signers, stop) = (Arc::clone(&signers), Arc::clone(&stop));
            std::thread::spawn(move || {
                let mut signed = 0_u64;
                while !stop.load(Ordering::Relaxed) {
                    signers[n % 2].attest(DIGEST).expect("a signature");
                    signed += 1;
                }
                signed
            })
        })
        .collect();
    let limits = ProfilingConfig::default();
    let params = FlamegraphParams {
        seconds: Some(5),
        frequency: Some(limits.max_frequency),
    };
    let slot = ProfilerSlot::default();
    let mut reached_signer = false;
    for _ in 0..3 {
        let svg = sample_flamegraph_svg(&slot, limits, &params)
            .await
            .expect("the sample window completes");
        let svg = String::from_utf8(svg).expect("an SVG");
        assert!(svg.contains("<svg"), "a rendered flamegraph");
        reached_signer |= svg.contains("attest");
    }
    stop.store(true, Ordering::Relaxed);
    let signed: u64 = workers
        .into_iter()
        .map(|worker| worker.join().expect("a signing thread"))
        .sum();
    assert!(signed > 0, "the threads signed while the profiler sampled");
    assert!(
        reached_signer,
        "the sampled stacks walk from inside the signature up to the signer"
    );
}
