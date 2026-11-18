// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `VERSION.signature` a commit stores: a client-supplied one kept
//! verbatim, or the server's over the assembled version (RM common master06
//! §Digital Signature).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the signed version body is the canonical \
              form the commit interior carries (stored-content class)"
)]

use serde_json::Value;
use uuid::Uuid;

use super::ResolvedWrite;
use crate::error::ServiceError;
use crate::versioning::audit::AuditInput;
use crate::versioning::body::Body;
use crate::versioning::integrity::Signed;
use crate::versioning::{SigningCtx, integrity};

/// Derive the signature over the version body.
///
/// The signature input is the accepted body before decomposition, the value
/// the stored `version.body` text ([`super::canonical_body_text`]) serializes
/// and the node rows reassemble to, and the signature canonicalization is RFC
/// 8785 (key-order-insensitive), so it proves the stored and served bytes. A Null body is a logical delete
/// (Void body is signed, master06 §Logical Deletion).
///
/// # Errors
/// The signer's [`ServiceError`].
pub(super) fn body_and_signature(
    ctx: &SigningCtx<'_>,
    audit: &AuditInput,
    contribution_id: Uuid,
    r: &ResolvedWrite,
    at_committal_attestations: &[Value],
) -> Result<(Option<String>, bool), ServiceError> {
    let signs = r.client_signature.is_none() && ctx.signer.enabled();
    let body = match r.body.as_ref().filter(|_| signs) {
        None => Signed::Void,
        Some(Body::Canonical(value)) => Signed::Value(value),
        Some(typed) => Signed::Typed(typed),
    };
    version_signature(
        ctx,
        audit,
        contribution_id,
        r,
        body,
        at_committal_attestations,
    )
}

/// The signature stored with the version, and whether it is **client-supplied**
/// (foreign — stored verbatim, never re-verified at read; master06 §Digital
/// Signature) vs server-generated. A client-supplied signature is kept verbatim;
/// otherwise, with server signing enabled, the `body` is signed
/// — a logically deleted version signs Void (master06 §Logical Deletion), and a
/// content version signs the same served bytes reads return, so the digest
/// recomputes at read time. `at_committal_attestations` are the completed
/// `ATTESTATION`s committed with the version, which the signed form includes
/// (master06 §Digital Signature: "the entire Version object").
///
/// # Errors
/// [`ServiceError::Signing`] when the canonical form cannot be produced or the
/// signer fails.
fn version_signature(
    ctx: &SigningCtx<'_>,
    audit: &AuditInput,
    contribution_id: Uuid,
    r: &ResolvedWrite,
    body: Signed<'_>,
    at_committal_attestations: &[Value],
) -> Result<(Option<String>, bool), ServiceError> {
    if let Some(client) = &r.client_signature {
        return Ok((Some(client.clone()), true));
    }
    if !ctx.signer.enabled() {
        return Ok((None, false));
    }
    // The signed content is the SAME reassembled value the caller stores as
    // `version.body` — computed once, never re-reassembled here.
    let signature = integrity::sign_version(
        ctx,
        audit,
        r.time_committed,
        r.vo_id,
        r.tree,
        r.preceding_uid.as_deref(),
        &r.other_input_version_uids,
        contribution_id,
        &r.lifecycle,
        body,
        at_committal_attestations,
    )?;
    Ok((signature, false))
}
