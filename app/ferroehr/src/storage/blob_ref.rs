// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The multimedia blob reference index: which stored version references which
//! externalized blob.
//!
//! `DV_MULTIMEDIA` may carry its content by reference
//! (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_types.dv_multimedia.adoc`:
//! `uri` is a "URI reference to electronic information stored outside the
//! record as a file, database entry etc"), and the multimedia extension holds
//! those bytes in an object store. After a physical delete the collector has to
//! answer "does any surviving version still reference this blob"; the rows
//! written here answer it as an index probe over `blob_ref` instead of a scan
//! of every `node` row. No openEHR spec governs multimedia offload — our own
//! design/extension.
//!
//! The rows are written by the node write path, in the same transaction as the
//! nodes they describe, and the `blob_ref` foreign key into `version` carries
//! them across the archival tier move and removes them with the version, so
//! nothing else maintains them.

use sqlx::PgConnection;

use crate::ids::VoId;
use crate::storage::error::StorageError;

/// The URI scheme an externalized blob is addressed by
/// (`ferroehr_ext::multimedia::BlobStore::uri_for` writes
/// `s3://<bucket>/<digest>`).
///
/// The detection is scheme-driven rather than bucket-driven so the storage
/// layer needs no multimedia configuration and the index stays correct in a
/// build compiled without the extension. Recording a foreign `s3://` URI a
/// client supplied costs nothing: the collector only ever asks about URIs this
/// deployment minted, and an extra row can only keep a blob, never delete one.
pub(crate) const EXTERNAL_BLOB_SCHEME: &str = "s3://";

/// Record `uris` ([`crate::storage::render::blob_uris`]), the external blob
/// URIs version `sys_version` of `vo_id` references.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver/insert failure.
pub async fn record_uris(
    tx: &mut PgConnection,
    vo_id: VoId,
    sys_version: i32,
    uris: &[String],
) -> Result<(), StorageError> {
    let vo_ids = vec![vo_id.0; uris.len()];
    let sys_versions = vec![sys_version; uris.len()];
    let uris: Vec<&str> = uris.iter().map(String::as_str).collect();
    insert(tx, &vo_ids, &sys_versions, &uris).await
}

/// Insert one `blob_ref` row per index of the three aligned columns, with no
/// statement when they are empty.
async fn insert(
    tx: &mut PgConnection,
    vo_ids: &[uuid::Uuid],
    sys_versions: &[i32],
    uris: &[&str],
) -> Result<(), StorageError> {
    if uris.is_empty() {
        return Ok(());
    }
    // The version row is written by the same transaction and the tier defaults
    // to `hot` there, so the reference lands in the partition its version is
    // in. ON CONFLICT covers a replay of the same body into the same version.
    sqlx::query(
        "INSERT INTO blob_ref (vo_id, sys_version, uri) \
         SELECT * FROM unnest($1::uuid[], $2::int[], $3::text[]) \
         ON CONFLICT DO NOTHING",
    )
    .bind(vo_ids)
    .bind(sys_versions)
    .bind(uris)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    reason = "the test states the walk over a canonical fragment"
)]
mod tests {
    use serde_json::json;

    use crate::storage::render::blob_uris;
    use crate::storage::render::index::render;

    /// The walk finds an externalized reference at any depth and leaves inline
    /// content and foreign URIs alone.
    #[test]
    fn collects_externalized_references_only() {
        let fragment = json!({
            "_type": "ELEMENT",
            "value": {
                "_type": "DV_MULTIMEDIA",
                "uri": {"_type": "DV_URI", "value": "s3://blobs/aabbcc"},
                "thumbnail": {
                    "_type": "DV_MULTIMEDIA",
                    "uri": {"_type": "DV_URI", "value": "s3://blobs/ddeeff"}
                }
            },
            "other": [
                {"_type": "DV_MULTIMEDIA", "data": "AAEC"},
                {"_type": "DV_MULTIMEDIA",
                 "uri": {"_type": "DV_URI", "value": "https://example.test/scan.png"}}
            ]
        });
        assert_eq!(
            blob_uris(&render(&fragment).unwrap()),
            ["s3://blobs/aabbcc", "s3://blobs/ddeeff"]
        );
    }
}
