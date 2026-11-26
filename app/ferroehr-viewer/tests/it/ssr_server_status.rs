// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Server › Overview and the shell over the CDR's full status document: the
//! licence, the deployment gaps and the support period (#6654).

use crate::ssr_app::{Viewer, stub};

/// The full status document of a sandbox whose licence expired and whose
/// support period ended, with one gap open and one accepted by name.
fn full_status() -> String {
    serde_json::json!({
        "status": "UP",
        "server_version": "6.0.0",
        "openehr_rest_api_version": "1.1.0",
        "timestamp": "2026-11-26T10:00:00Z",
        "licence": {
            "state": "licensed",
            "use": "commercial",
            "licensee": "Example Hospital",
            "not_after": "2026-01-31",
            "configured_token": "in-force"
        },
        "deployment": {
            "profile": "sandbox",
            "gaps": ["shared_credential", "audit_off"],
            "accepted": ["audit_off"]
        },
        "support": {"release_date": "2020-11-01", "support_ends": "2025-11", "status": "ended"}
    })
    .to_string()
}

/// A session that reads the full document sees the licence, the support
/// period and every gap by its words on Overview, the two warnings, and the
/// shell's band naming the gaps.
#[tokio::test]
async fn overview_shows_the_licence_the_gaps_and_the_support_period() {
    let viewer = Viewer::start_with(vec![stub(
        "/management/status",
        "application/json",
        &full_status(),
    )])
    .await;
    let cookie = viewer.sign_in().await;
    let server = viewer.page(&cookie, "/server").await;
    assert_eq!(server.status, http::StatusCode::OK, "{}", server.body);
    let main = server.main();
    for words in [
        "Licence",
        "Example Hospital",
        "Commercial",
        "2026-01-31",
        "The licence expired on 2026-01-31.",
        "Support period",
        "2025-11",
        "The support period of this release ended in 2025-11: it receives no more fixes.",
        "the clinical and demographic domains share one database credential",
        "access is not recorded to a durable audit trail",
        "Accepted by name",
        "Open",
    ] {
        assert!(main.contains(words), "Overview shows {words}:\n{main}");
    }
    let dashboard = viewer.page(&cookie, "/").await;
    assert!(
        dashboard.body.contains(
            "2 production safeguards are off: the clinical and demographic domains share \
             one database credential; access is not recorded to a durable audit trail"
        ),
        "the band names the gaps:\n{}",
        dashboard.body
    );
}

/// A session the full document is withheld from sees the public profile,
/// a note saying so, and the band's rule alone.
#[tokio::test]
async fn a_withheld_full_status_keeps_the_public_profile() {
    let refused = serde_json::json!({
        "method": "GET",
        "path": "/management/status",
        "body": "",
        "status": 403,
        "headers": {"content-type": "application/json"},
        "response": "{\"error\":\"forbidden\"}",
    });
    let viewer = Viewer::start_with(vec![refused]).await;
    let cookie = viewer.sign_in().await;
    let server = viewer.page(&cookie, "/server").await;
    assert_eq!(server.status, http::StatusCode::OK, "{}", server.body);
    assert!(
        server.main().contains("Your account may not read the CDR"),
        "{}",
        server.main()
    );
    assert!(!server.main().contains("Licensee"), "{}", server.main());
    assert!(
        server
            .body
            .contains("This deployment must not hold real patient data."),
        "{}",
        server.body
    );
}
