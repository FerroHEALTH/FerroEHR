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

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): stored canonical fragments — a typed \
              round-trip drops forward-compatible keys (the openEHR release strategy: minors are compatible supersets)"
)]

use serde_json::Value;
use sqlx::PgConnection;

use crate::ids::VoId;
use crate::storage::error::StorageError;
use crate::storage::row::NodeRow;

/// The URI scheme an externalized blob is addressed by
/// (`ferroehr_ext::multimedia::BlobStore::uri_for` writes
/// `s3://<bucket>/<digest>`).
///
/// The detection is scheme-driven rather than bucket-driven so the storage
/// layer needs no multimedia configuration and the index stays correct in a
/// build compiled without the extension. Recording a foreign `s3://` URI a
/// client supplied costs nothing: the collector only ever asks about URIs this
/// deployment minted, and an extra row can only keep a blob, never delete one.
const EXTERNAL_BLOB_SCHEME: &str = "s3://";

/// Record the external blob URIs the given versions' node rows reference.
///
/// Runs inside the caller's transaction and issues no statement when the rows
/// reference nothing, which is every commit in a deployment that stores no
/// multimedia by reference.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver/insert failure.
pub async fn record(
    tx: &mut PgConnection,
    versions: &[(VoId, i32, &[NodeRow])],
) -> Result<(), StorageError> {
    let mut vo_ids: Vec<uuid::Uuid> = Vec::new();
    let mut sys_versions: Vec<i32> = Vec::new();
    let mut uris: Vec<&str> = Vec::new();
    for (vo_id, sys_version, rows) in versions {
        let mut found: Vec<&str> = Vec::new();
        for row in *rows {
            collect_uris(&row.data, &mut found);
        }
        found.sort_unstable();
        found.dedup();
        for uri in found {
            vo_ids.push(vo_id.0);
            sys_versions.push(*sys_version);
            uris.push(uri);
        }
    }
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
    .bind(&vo_ids)
    .bind(&sys_versions)
    .bind(&uris)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Collect every externalized blob URI in one canonical fragment.
///
/// A `DV_MULTIMEDIA` holds its reference at `uri.value`, and a fragment may
/// carry several (an `ITEM_TREE` of images, a thumbnail beside its full-size
/// original), at any depth.
fn collect_uris<'a>(value: &'a Value, out: &mut Vec<&'a str>) {
    match value {
        Value::Object(map) => {
            if let Some(uri) = map
                .get("uri")
                .and_then(|u| u.get("value"))
                .and_then(Value::as_str)
                && uri.starts_with(EXTERNAL_BLOB_SCHEME)
            {
                out.push(uri);
            }
            for child in map.values() {
                collect_uris(child, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_uris(item, out);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::collect_uris;

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
        let mut found = Vec::new();
        collect_uris(&fragment, &mut found);
        found.sort_unstable();
        assert_eq!(found, ["s3://blobs/aabbcc", "s3://blobs/ddeeff"]);
    }
}
