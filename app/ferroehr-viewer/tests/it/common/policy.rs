// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The demo seed's access policy, put back in its first version for a journey
//! that reads a record on a justification.
//!
//! The seed's first policy version holds professionals to a care relationship
//! and its final version lifts that (`scripts/sandbox/seed/access/`), so a
//! journey over justified or emergency access writes the first version again
//! over the final one, runs, and writes the final one back whatever the
//! journey did. No openEHR specification governs the viewer's tests: our own
//! design.

use std::future::Future;

use futures::FutureExt as _;

/// The access policy route of the CDR at `cdr`, or its history.
fn policy_url(cdr: &str, history: bool) -> String {
    let base = format!("{cdr}/ferroehr/rest/extensions/v1/admin/access_policy");
    if history {
        format!("{base}/history")
    } else {
        base
    }
}

/// The version uid a weak or strong `ETag` names.
fn etag_uid(response: &reqwest::Response) -> String {
    response
        .headers()
        .get("etag")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim_start_matches("W/").trim_matches('"').to_owned())
        .expect("the policy names its version in its ETag")
}

/// The access policy in force at `cdr` and the version uid its `ETag` names.
async fn policy_in_force(
    http: &reqwest::Client,
    cdr: &str,
    admin: (&str, &str),
) -> (serde_json::Value, String) {
    let response = http
        .get(policy_url(cdr, false))
        .basic_auth(admin.0, Some(admin.1))
        .header("Accept", "application/json")
        .send()
        .await
        .expect("the CDR answers the policy read");
    assert!(
        response.status().is_success(),
        "the seed commits a policy: {}",
        response.status()
    );
    let etag = etag_uid(&response);
    (response.json().await.expect("the policy is JSON"), etag)
}

/// The seed's first policy version, the oldest of the history.
async fn first_policy(http: &reqwest::Client, cdr: &str, admin: (&str, &str)) -> serde_json::Value {
    let history: serde_json::Value = http
        .get(policy_url(cdr, true))
        .basic_auth(admin.0, Some(admin.1))
        .header("Accept", "application/json")
        .send()
        .await
        .expect("the CDR answers the policy history")
        .json()
        .await
        .expect("the policy history is JSON");
    history
        .as_array()
        .and_then(|versions| versions.last())
        .and_then(|version| version.get("data"))
        .cloned()
        .expect("the policy history holds the seed's first version")
}

/// Writes `policy` over the version `over` and answers the new version uid.
async fn write_policy(
    http: &reqwest::Client,
    cdr: &str,
    admin: (&str, &str),
    policy: &serde_json::Value,
    over: &str,
) -> String {
    let response = http
        .put(policy_url(cdr, false))
        .basic_auth(admin.0, Some(admin.1))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("If-Match", format!("\"{over}\""))
        .json(policy)
        .send()
        .await
        .expect("the CDR answers the policy write");
    assert!(
        response.status().is_success(),
        "the policy write is taken: {}",
        response.status()
    );
    etag_uid(&response)
}

/// Runs `journey` with the seed's first policy version in force at `cdr`,
/// then writes the final version back, whether the journey passed or not.
///
/// # Panics
/// When a policy read or write is refused, or when `journey` panics.
pub(crate) async fn with_first_policy(
    cdr: &str,
    admin: (&str, &str),
    journey: impl Future<Output = ()>,
) {
    let http = reqwest::Client::new();
    let (final_policy, in_force) = policy_in_force(&http, cdr, admin).await;
    let first = first_policy(&http, cdr, admin).await;
    let applied = write_policy(&http, cdr, admin, &first, &in_force).await;
    let outcome = std::panic::AssertUnwindSafe(journey).catch_unwind().await;
    write_policy(&http, cdr, admin, &final_policy, &applied).await;
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}
