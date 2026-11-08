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
use crate::storage::codec::reassemble;
use crate::versioning::audit::AuditInput;
use crate::versioning::{SigningCtx, integrity};

/// Reassemble the version body from the node rows and derive the signature
/// over it.
///
/// The signature input is the reassembled value; the stored `version.body`
/// is the pre-decomposition text ([`super::canonical_body_text`]). The two are the
/// same VALUE by decompose/reassemble fidelity, and the signature
/// canonicalization is RFC 8785 (key-order-insensitive), so signing the
/// reassembled form still proves the stored and served bytes. Empty rows =
/// logical delete (Void body is signed — master06 §Logical Deletion).
///
/// # Errors
/// The reassembly [`ServiceError`] or the signer's failure.
pub(super) fn body_and_signature(
    ctx: &SigningCtx<'_>,
    audit: &AuditInput,
    contribution_id: Uuid,
    r: &ResolvedWrite,
    at_committal_attestations: &[Value],
) -> Result<(Option<String>, bool), ServiceError> {
    // Only a server signature reads the body: reassembling it for nothing
    // costs as much as the version is large (#5220).
    let signs = r.client_signature.is_none() && ctx.signer.enabled();
    let body = if r.rows.is_empty() || !signs {
        None
    } else {
        Some(reassemble(&r.rows)?)
    };
    version_signature(
        ctx,
        audit,
        contribution_id,
        r,
        body.as_ref(),
        at_committal_attestations,
    )
}

/// The signature stored with the version, and whether it is **client-supplied**
/// (foreign — stored verbatim, never re-verified at read; master06 §Digital
/// Signature) vs server-generated. A client-supplied signature is kept verbatim;
/// otherwise, with server signing enabled, the pre-reassembled `body` is signed
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
    body: Option<&Value>,
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
        body.unwrap_or(&Value::Null),
        at_committal_attestations,
    )?;
    Ok((signature, false))
}
