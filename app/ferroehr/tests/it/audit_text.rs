// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What the audit store serves, as text, for the tests that assert a value
//! never reaches the trail: the FHIR `AuditEvent` the store renders for each
//! record, and the columns migration 0017 added to render it from.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only reaches \
              `#[test]`-annotated functions, so it misses this helper module; a failed read \
              is the test failing (the Rust Book ch11)"
)]

use ferroehr::system_log::store::{AuditSearchFilter, AuditStore};
use sqlx::PgPool;

/// The columns migration 0017 added of every stored record, then every
/// record's served FHIR document as JSON text.
///
/// # Panics
/// When the trail cannot be read or a record cannot be rendered.
pub(crate) async fn audit_texts(pool: &PgPool) -> Vec<String> {
    let mut texts: Vec<String> = sqlx::query_scalar(
        "SELECT concat_ws(' ', issuer, client_id, ehr_id, subject_namespace, \
             category_content::text) FROM audit.audit_event ORDER BY chain_seq",
    )
    .fetch_all(pool)
    .await
    .expect("read the stored audit rows");
    let (_, documents) = AuditStore::new(pool.clone())
        .search(&AuditSearchFilter {
            count: 1000,
            ..AuditSearchFilter::default()
        })
        .await
        .expect("render the stored audit records");
    texts.extend(documents.iter().map(ToString::to_string));
    texts
}
