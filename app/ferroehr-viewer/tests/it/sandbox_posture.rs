// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The hosted sandbox's viewer posture (`deploy/hosted/ferroehr-viewer.sandbox.toml`)
//! read the way the box's viewer reads it at boot: the file under the
//! environment the viewer image and the box's compose file give the
//! container, through the same load and session check.
//!
//! The 5.6.0 release deployed a posture that still spelled the 5.5
//! `[session]` keys, and the viewer refused the start on the box (#4985);
//! nothing read the file before the box did.

#![expect(
    clippy::expect_used,
    reason = "the posture file is a committed input; one that cannot be read is a broken checkout, \
              not a test outcome"
)]

use std::collections::BTreeMap;
use std::path::Path;

use ferroehr_viewer::config::load_sources;
use ferrohealth_session::config::StoreKind;

/// The posture file as the ferroehr image carries it to the box.
fn posture() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../deploy/hosted/ferroehr-viewer.sandbox.toml");
    std::fs::read_to_string(&path).expect("deploy/hosted/ferroehr-viewer.sandbox.toml")
}

/// The container environment: `docker/viewer/Dockerfile`'s listen address
/// and the compose file's `FERROEHR_VIEWER_CONFIG`.
fn container_environment() -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "FERROEHR_VIEWER__SERVER__LISTEN".to_owned(),
            "0.0.0.0:3000".to_owned(),
        ),
        (
            "FERROEHR_VIEWER_CONFIG".to_owned(),
            "/etc/ferroehr/viewer.toml".to_owned(),
        ),
    ])
}

#[test]
fn the_sandbox_viewer_posture_starts_the_viewer() {
    let config = load_sources(Some(&posture()), &container_environment())
        .unwrap_or_else(|error| panic!("the sandbox viewer posture refuses the start: {error}"));

    assert_eq!(config.cdr.base_url, "https://sandbox.ferroehr.eu");
    assert_eq!(config.server.listen.to_string(), "0.0.0.0:3000");
    assert_eq!(
        config.session.store,
        StoreKind::Sealed,
        "the sandbox seals its sessions with the committed demo key (#2949)"
    );
    assert!(
        config.session.secure_cookie,
        "TLS terminates at Caddy, so the cookie is Secure"
    );
    assert!(
        config.login.notice.contains("ferroehr / ferroehr"),
        "the sign-in card names the demo credentials"
    );
}
