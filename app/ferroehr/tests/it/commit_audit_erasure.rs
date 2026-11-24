// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An erased EHR leaves no commit audit naming its subject (#6395, GDPR
//! Art. 17): what can name the subject, a person's committer and a
//! client's description, is stored on the audit's own row, which the
//! erasure deletes, and the shared parts that stay name no subject.

use ferroehr::service::ehr_index::types::SubjectRef;
use ferroehr::versioning::committer::with_committer;
use serde_json::json;

use crate::commit_audit_golden::{matching, person, professional, version};
use crate::fixtures::composition;

/// The subject id the EHR names, and the text that would leak it.
const SUBJECT: &str = "subject-6395-erasure";

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
    let reviewed = format!("Reviewed with {SUBJECT}");
    let write = svc.create_composition(
        ehr,
        version(&composition("a"), "249", None, &matching(), Some(&reviewed)),
    );
    with_committer(Some(professional()), Box::pin(write))
        .await
        .expect("the professional's commit");
    let mut body = composition("mine");
    body["composer"] = json!({ "_type": "PARTY_SELF" });
    let written = format!("Written by {SUBJECT}");
    let write = svc.create_composition(
        ehr,
        version(
            &body,
            "249",
            None,
            &json!({ "_type": "PARTY_SELF" }),
            Some(&written),
        ),
    );
    with_committer(Some(person()), Box::pin(write))
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
