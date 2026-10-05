// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Digital-signature integrity at commit and read.
//!
//! Spec: RM common `master06-change_control_package.adoc` §Digital Signature +
//! BASE arch-overview `master07-security.adoc` §Integrity. The signature is
//! computed over the version's canonical serialized form with the signature
//! attribute Void, and is a stored fact carried with the data (for Extracts).
//! This module holds the *policy* — sign the assembled `ORIGINAL_VERSION` at
//! commit, and (optionally) recompute-and-compare at read; the primitives live
//! in [`signature`](super::signature).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 2): the serialized version envelope is the \
              signed artifact (RM common master06 §Digital Signature) — re-encoding breaks \
              verification"
)]

use serde_json::Value;
use uuid::Uuid;

use crate::ids::VoId;
use crate::service::error::ServiceError;
use crate::versioning::SigningCtx;
use crate::versioning::audit::AuditInput;
use crate::versioning::object_version_id::TreeId;
use crate::versioning::signature::config::VerifyOnRead;
use crate::versioning::signature::signer::Signer;
use crate::versioning::wire::{
    OriginalVersionParts, build_imported_version, build_original_version, contribution_ref,
};

/// Compute the server-generated `VERSION.signature` for a version about to be
/// persisted (RM common master06 §Digital Signature). The caller
/// ([`crate::versioning::change`]) decides client-vs-server first: a
/// client-supplied signature is stored verbatim and never reaches here.
///
/// When signing is enabled the fully-assembled `ORIGINAL_VERSION`, the exact
/// value that will later be served (built by the shared
/// [`build_original_version`] so commit-time and read-time bytes match,
/// including `other_input_version_uids` merge provenance), is signed over its
/// `canonical_form()` with the signature attribute Void.
///
/// The `attestations` committed with this version
/// (`UPDATE_VERSION.attestations`) are signed too: master06 §Digital Signature
/// serialises "the entire Version object (note that the signature attribute will
/// be Void at this point)", so `signature` is the only excluded attribute. An
/// attestation added afterwards post-dates the signature and is appended outside
/// it at read time.
///
/// # Errors
/// [`ServiceError::Signing`] when the canonical form cannot be produced or the
/// `OpenPGP` signer fails (digest signing is infallible).
#[expect(
    clippy::too_many_arguments,
    reason = "the parts of an ORIGINAL_VERSION plus the signing context; a \
              parameter struct would not read clearer at the call sites"
)]
pub(crate) fn sign_version(
    ctx: &SigningCtx<'_>,
    audit: &AuditInput,
    time_committed: jiff::Timestamp,
    vo_id: VoId,
    tree: TreeId,
    preceding_uid: Option<&str>,
    contribution_id: Uuid,
    lifecycle_state: &str,
    data: &Value,
    attestations: &[Value],
) -> Result<Option<String>, ServiceError> {
    if !ctx.signer.enabled() {
        return Ok(None);
    }
    // The envelope is built WITHOUT `data` (Null is skipped by the builder)
    // and the body joins the signed form through a shallow reference view in
    // `sign_canonical_with_data` — the commit path never deep-copies the body
    // into the envelope. RFC 8785 orders keys, so the join point is
    // byte-identical to a fully-assembled ORIGINAL_VERSION's canonical form.
    let ov = build_original_version(&OriginalVersionParts {
        creating_system_id: &ctx.system_id,
        vo_id,
        tree,
        preceding_version_uid: preceding_uid,
        // Merge provenance reaches storage only through the routes that carry a
        // FOREIGN `ORIGINAL_VERSION` verbatim — the EHR-Extract import and the
        // archive load — which build and sign their own rows.
        // NOTE: a locally committed version is never a MERGE (RM common master06
        // §Version Merging), and the released commit wire declares no merge shape
        // at all (ITS-REST `schemas/ehr/UpdateVersion.yaml`).
        other_input_version_uids: &[],
        contribution: &contribution_ref(contribution_id),
        commit_audit: &audit.canonical(&time_committed),
        lifecycle_state,
        data: &Value::Null,
        attestations,
        signature: None,
    })?;
    sign_canonical_with_data(ctx, &ov, data)
}

/// JCS-serialize and sign the envelope joined with its `data` through a
/// shallow reference view — the body is never deep-copied into the envelope.
///
/// `ov` carries neither `data` nor `signature` (the sign path builds it that
/// way), so the view is exactly the `canonical_form()` input; a Null `data`
/// (logical delete) stays absent, matching the builder's own skip.
///
/// # Errors
/// [`ServiceError::SigningCanonical`] / [`ServiceError::SigningFailed`] as
/// [`sign_canonical`].
fn sign_canonical_with_data(
    ctx: &SigningCtx<'_>,
    ov: &Value,
    data: &Value,
) -> Result<Option<String>, ServiceError> {
    let canonical =
        openehr_rm::v1_2::common::change_control::version_impl::canonical_form_of_json_with_data(
            ov, data,
        )
        .map_err(|e| ServiceError::SigningCanonical(e.to_string(), e))?;
    let signature = ctx
        .signer
        .sign(&canonical)
        .map_err(|e| ServiceError::SigningFailed(e.to_string(), e))?;
    Ok(Some(signature))
}

/// Compute the server-generated `VERSION.signature` of the `IMPORTED_VERSION`
/// an import act is about to persist (RM common master06 §Digital Signature:
/// "If the object to be serialised is an `IMPORTED_VERSION`, the process is the
/// same — all attributes of the object are serialised and then used to generate
/// a signature. The result will be that the `IMPORTED_VERSION` instance will
/// carry its own signature which signifies the act of importing and making
/// available locally an `ORIGINAL_VERSION` from another system").
///
/// The wrapper is signed exactly like a local commit — an import IS a local act
/// of committal (§Committal and Audits) — so this server signs it whenever
/// signing is enabled, and the wrapped original's own foreign signature rides
/// inside `item` untouched (§Copying: "the `ORIGINAL_VERSION` instance is never
/// modified"). The value signed is the one [`build_imported_version`] produces
/// with the wrapper's own `signature` Void — `item` included WHOLE, since "all
/// attributes of the object are serialised" and the received original's own
/// `attestations` are part of it. That is exactly what the read path rebuilds
/// before verifying; attestations added to the imported version AFTER the
/// import (§Attestation) are appended outside the signed form.
///
/// # Errors
/// [`ServiceError::Signing`] when the canonical form cannot be produced or the
/// `OpenPGP` signer fails.
pub(crate) fn sign_imported_version(
    ctx: &SigningCtx<'_>,
    contribution_id: Uuid,
    commit_audit: &Value,
    item: &Value,
) -> Result<Option<String>, ServiceError> {
    if !ctx.signer.enabled() {
        return Ok(None);
    }
    let iv = build_imported_version(&contribution_ref(contribution_id), commit_audit, item, None);
    sign_canonical(ctx, &iv)
}

/// Sign an assembled `VERSION` value over its `canonical_form()` (master06
/// §Digital Signature; `version.adoc` `canonical_form`: "all attributes except
/// signature").
///
/// # Errors
/// [`ServiceError::Signing`] when the canonical form cannot be produced or the
/// `OpenPGP` signer fails.
fn sign_canonical(ctx: &SigningCtx<'_>, value: &Value) -> Result<Option<String>, ServiceError> {
    let canonical =
        openehr_rm::v1_2::common::change_control::version_impl::canonical_form_of_json(value)
            .map_err(|e| ServiceError::SigningCanonical(e.to_string(), e))?;
    let signature = ctx
        .signer
        .sign(&canonical)
        .map_err(|e| ServiceError::SigningFailed(e.to_string(), e))?;
    Ok(Some(signature))
}

/// Read-time signature verification (RM common master06 §Digital Signature —
/// the signature verifies the served version against its recomputed
/// `canonical_form`). Applies ONLY to signatures **this server generated**: a
/// `client_supplied` signature is stored verbatim and never re-verified (the
/// author may have signed another agreed serialization we cannot recompute —
/// master06 §Digital Signature / §Copying). Otherwise a no-op when
/// `verify_on_read = off` or the version carries no signature. A `warn`
/// mismatch logs + meters (`version_signature_invalid_total`); a `strict`
/// mismatch is a 5xx integrity failure.
///
/// # Errors
/// [`ServiceError::Signing`] when `verify_on_read = strict` and the stored
/// server signature fails verification, or (in any non-`off` mode) when the
/// served version's canonical form cannot be recomputed.
pub(crate) fn verify_on_read(
    signer: &Signer,
    ov: &Value,
    signature: Option<&str>,
    client_supplied: bool,
) -> Result<(), ServiceError> {
    // Client-supplied signatures are foreign facts stored verbatim — never our
    // canonical form to recompute (master06 §Digital Signature).
    if client_supplied || signer.verify_on_read() == VerifyOnRead::Off {
        return Ok(());
    }
    let Some(signature) = signature else {
        return Ok(());
    };
    let canonical =
        openehr_rm::v1_2::common::change_control::version_impl::canonical_form_of_json(ov)
            .map_err(|e| ServiceError::SigningCanonical(e.to_string(), e))?;
    let verdict = signer.verify(&canonical, signature);
    if verdict.is_failure() {
        crate::telemetry::metrics::metrics()
            .version_signature_invalid
            .add(
                1,
                &[opentelemetry::KeyValue::new("verdict", verdict.label())],
            );
        tracing::error!(
            verdict = verdict.label(),
            "version signature failed verification (verify_on_read)"
        );
        if signer.verify_on_read() == VerifyOnRead::Strict {
            return Err(ServiceError::Signing(format!(
                "stored version signature does not verify ({})",
                verdict.label()
            )));
        }
    }
    Ok(())
}
