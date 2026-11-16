// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access log of the hospital scenario: Eva reads her own log in the
//! subject view, every access to her record shown and the professional who
//! made it withheld under the national provision the policy names (EHDS
//! Art. 9(1) and 9(3), `docs/law/eu/ehds/text.html`); Hans, the privacy
//! officer, reads everything in the full view; both reads are accesses of
//! their own, and both chains verify. The draft `org.openehr.rm.access_log`
//! 0.1.0 is the shape; the views are our own design (#5130, #5570).

use http::StatusCode;
use serde_json::{Value, json};

use crate::common::EXT;
use crate::hospital::abac::{answered, read};
use crate::hospital::build::world;
use crate::hospital::ledger::{Expect, Ledger};
use crate::hospital::world::{Answer, World, call};

/// The entries of a page of the log.
fn entries(answer: &Answer) -> Vec<Value> {
    answer.json()["entries"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// The entry of the request `request_id` in `page`, when it shows one.
fn shown(page: &[Value], uid_of: impl Fn(&Value) -> bool) -> Option<Value> {
    page.iter().find(|entry| uid_of(entry)).cloned()
}

/// Dr. Jansen's read of the vital signs, the access every view must show.
async fn jansens_read(world: &World, ledger: &mut Ledger) -> Answer {
    let answer = read(world, "jansen", &world.vitals, Some("TREAT")).await;
    answered(&answer, StatusCode::OK, None);
    ledger.note(
        world,
        "Dr. Jansen's read",
        &answer,
        Expect::granted(&["treatment relationship"]),
    );
    answer
}

/// Whether `entry` records a read of the vital signs by a person.
fn of_the_vitals(world: &World, entry: &Value) -> bool {
    entry["accessor"]["kind"]["value"] == "person"
        && entry["items"].as_array().is_some_and(|items| {
            items.iter().any(|item| {
                item["version_id"]["value"]
                    .as_str()
                    .is_some_and(|version| version.starts_with(&world.vitals.vo))
            })
        })
}

/// Eva reads her own log: Dr. Jansen's read is there with its time, action
/// and grounds, without the professional and the provider the policy
/// withholds, and without the full view's chain link and channel.
#[tokio::test]
async fn eva_reads_her_own_log_minus_what_article_9_3_withholds() {
    let world = Box::pin(world()).await;
    let mut ledger = Ledger::default();
    jansens_read(&world, &mut ledger).await;
    ledger.verify(&world).await;
    let eva = format!("eva:{}", world.ehr);
    let answer = call(
        &world.app,
        &eva,
        "GET",
        &format!("{EXT}/ehr/{}/access_log", world.ehr),
        None,
        None,
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    let page = entries(&answer);
    let entry = shown(&page, |entry| of_the_vitals(&world, entry))
        .unwrap_or_else(|| panic!("Dr. Jansen's read is shown: {page:#?}"));
    for withheld in ["party", "provider", "credential", "binding"] {
        assert!(
            entry["accessor"].get(withheld).is_none(),
            "{withheld}: {entry}"
        );
    }
    for withheld in ["link", "channel"] {
        assert!(entry.get(withheld).is_none(), "{withheld}: {entry}");
    }
    assert!(entry.get("time_recorded").is_some(), "{entry}");
    assert_eq!(entry["action"]["value"], "read", "{entry}");
    let other = call(
        &world.app,
        &eva,
        "GET",
        &format!("{EXT}/ehr/{}/access_log", world.other_ehr),
        None,
        None,
    )
    .await;
    assert_eq!(
        other.status,
        StatusCode::NOT_FOUND,
        "another's log: {}",
        other.body
    );
}

/// Hans reads everything: Dr. Jansen's read in Eva's log with the
/// professional, the provider and the chain link, and the review across
/// EHRs; each of his reads is an access recorded in turn.
#[tokio::test]
async fn hans_reads_everything_and_the_chains_verify() {
    let world = Box::pin(world()).await;
    let mut ledger = Ledger::default();
    let request = jansens_read(&world, &mut ledger).await.request_id();
    ledger.verify(&world).await;
    let answer = call(
        &world.app,
        "hans",
        "GET",
        &format!("{EXT}/ehr/{}/access_log", world.ehr),
        None,
        None,
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    let page = entries(&answer);
    let entry = shown(&page, |entry| {
        entry["channel"]["request_id"] == json!(request)
    })
    .unwrap_or_else(|| panic!("Dr. Jansen's read: {page:#?}"));
    let jansen = &world.people["jansen"].person;
    assert_eq!(
        entry["accessor"]["party"]["external_ref"]["id"]["value"],
        json!(jansen),
        "{entry}"
    );
    assert!(entry["accessor"].get("provider").is_some(), "{entry}");
    assert!(entry.get("link").is_some(), "{entry}");
    let review = call(
        &world.app,
        "hans",
        "GET",
        &format!("{EXT}/admin/access-log?ehr_id={}", world.ehr),
        None,
        None,
    )
    .await;
    assert_eq!(review.status, StatusCode::OK, "{}", review.body);
    assert!(
        entries(&review)
            .iter()
            .any(|entry| entry["channel"]["request_id"] == json!(request)),
        "the review shows it: {}",
        review.body
    );
    let refused = call(
        &world.app,
        "jansen",
        "GET",
        &format!("{EXT}/admin/access-log"),
        None,
        None,
    )
    .await;
    assert_eq!(
        refused.status,
        StatusCode::FORBIDDEN,
        "a clinician: {}",
        refused.body
    );
    Ledger::default().verify(&world).await;
}
