// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An erased EHR leaves no commit audit naming its subject (#6395, GDPR
//! Art. 17): what can name the subject, a person's committer and a
//! client's description, is stored on the audit's own row, which the
//! erasure deletes, and the shared parts that stay name no subject.

use ferroehr::service::DEFAULT_SYSTEM_ID;
use ferroehr::service::ehr_index::types::SubjectRef;
use ferroehr::versioning::committer::{CommitterIdentity, with_committer};
use serde_json::{Value, json};

use crate::fixtures::{change_type, composition, lifecycle};

/// The subject id the EHR names, and the text that would leak it.
const SUBJECT: &str = "subject-6395-erasure";

/// A professional, or the record's person when `person`.
fn caller(person: bool) -> CommitterIdentity {
    CommitterIdentity {
        subject: if person { SUBJECT } else { "dr.jansen" }.to_owned(),
        id_type: "oauth2",
        issuer: Some("https://idp.example".to_owned()),
        local_issuer: DEFAULT_SYSTEM_ID.into(),
        client: None,
        person,
        party: None,
        representative: false,
        bound: None,
        system: false,
        agent: None,
        external: None,
    }
}

/// An `UPDATE_VERSION` of `data` by `committer` with the client's
/// `description`.
fn version<T: serde::de::DeserializeOwned, R: crate::fixtures::Envelope<T>>(
    data: &Value,
    committer: &Value,
    description: &str,
) -> R {
    R::from_version(
        openehr_its::json::from_canonical_value(&json!({
            "lifecycle_state": lifecycle("532"),
            "data": data,
            "commit_audit": {
                "change_type": change_type("249", "creation"),
                "committer": committer,
                "description": { "_type": "DV_TEXT", "value": description }
            }
        }))
        .expect("an UPDATE_VERSION"),
    )
}

/// The commit audit rows of either table whose text holds [`SUBJECT`].
async fn naming(pool: &sqlx::PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM commit_audit_facts f WHERE f::text LIKE '%' || $1 || '%') \
              + (SELECT count(*) FROM commit_audit_shared s WHERE s::text LIKE '%' || $1 || '%')",
    )
    .bind(SUBJECT)
    .fetch_one(pool)
    .await
    .expect("the audits read")
}

#[tokio::test]
async fn an_erased_ehr_leaves_no_audit_naming_its_subject() {
    let (db, svc) = crate::fixtures::service().await;
    let subject = SubjectRef::person(SUBJECT.to_owned(), "patients".to_owned());
    let ehr = crate::fixtures::create_ehr_subject(&svc, &subject).await;
    let professional = json!({
        "_type": "PARTY_IDENTIFIED",
        "name": "dr.jansen",
        "identifiers": [{ "issuer": "https://idp.example", "id": "dr.jansen", "type": "oauth2" }]
    });
    let write = svc.create_composition(
        ehr,
        version(
            &composition("a"),
            &professional,
            &format!("Reviewed with {SUBJECT}"),
        ),
    );
    with_committer(Some(caller(false)), Box::pin(write))
        .await
        .expect("the professional's commit");
    let mut body = composition("mine");
    body["composer"] = json!({ "_type": "PARTY_SELF" });
    let write = svc.create_composition(
        ehr,
        version(
            &body,
            &json!({ "_type": "PARTY_SELF" }),
            &format!("Written by {SUBJECT}"),
        ),
    );
    with_committer(Some(caller(true)), Box::pin(write))
        .await
        .expect("the person's commit");
    assert_eq!(naming(&db.pool()).await, 2, "both audits name the subject");

    svc.admin_ehr_delete(ehr).await.expect("the erasure");
    assert_eq!(
        naming(&db.pool()).await,
        0,
        "no commit audit names the erased subject"
    );
}
