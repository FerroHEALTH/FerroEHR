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

use openehr_rm::v1_2::common::change_control::canonical_serializer_impl::CanonicalObject;
use openehr_rm::v1_2::common::change_control::version_impl::CanonicalFormError;
use serde_json::Value;
use uuid::Uuid;

use crate::error::ServiceError;
use crate::ids::VoId;
use crate::versioning::SigningCtx;
use crate::versioning::audit::AuditInput;
use crate::versioning::object_version_id::TreeId;
use crate::versioning::object_version_id::VersionIdError;
use crate::versioning::signature::config::VerifyOnRead;
use crate::versioning::signature::signer::Signer;
use crate::versioning::wire::{
    OriginalVersionParts, VersionMember, build_imported_version, contribution_ref,
    write_original_version,
};

/// Computes the server-generated `VERSION.signature` of a version about to be
/// persisted (RM common master06 §Digital Signature); a client-supplied
/// signature never reaches here ([`crate::versioning::change`]).
///
/// The `ORIGINAL_VERSION` is signed over its canonical form with `signature`
/// Void, written by [`write_original_version`] so commit and read bytes match.
/// The attestations committed with it are inside the signed form; one added
/// later falls outside it.
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
    other_input_version_uids: &[String],
    contribution_id: Uuid,
    lifecycle_state: &str,
    data: Signed<'_>,
    attestations: &[Value],
) -> Result<Option<String>, ServiceError> {
    if !ctx.signer.enabled() {
        return Ok(None);
    }
    let canonical = signed_form(
        OriginalVersionParts {
            creating_system_id: &ctx.system_id,
            vo_id,
            tree,
            preceding_version_uid: preceding_uid,
            // NOTE: RM common master06 §Version Merging commits a merge as a local
            // `ORIGINAL_VERSION`, so its merge provenance is signed with it.
            other_input_version_uids,
            contribution: contribution_ref(contribution_id),
            commit_audit: audit.typed(&time_committed),
            lifecycle_state,
            data: &Value::Null,
            attestations,
            signature: None,
        },
        data,
    )?
    .map_err(CanonicalFormError::Canonicalize)
    .map_err(ServiceError::SigningCanonical)?;
    let signature = ctx
        .signer
        .sign(&canonical)
        .map_err(ServiceError::SigningFailed)?;
    Ok(Some(signature))
}

/// The RFC 8785 form a commit signs: the envelope `parts` name, without a
/// `signature`, joined with `data` (RM common master06 §Digital Signature).
///
/// RFC 8785 §3.2.3 orders the members, so the envelope is written from its
/// RM values and the body is read off its rendering, and no canonical value is
/// built of either.
///
/// # Errors
/// The outer error is [`write_original_version`]'s; the inner one is the
/// canonical writer's.
pub(crate) fn signed_form<C: VersionMember, A: VersionMember>(
    parts: OriginalVersionParts<'_, C, A>,
    data: Signed<'_>,
) -> Result<Result<String, serde_json::Error>, VersionIdError> {
    let mut envelope = CanonicalObject::default();
    if let Err(error) = write_original_version(parts, &mut envelope)? {
        return Ok(Err(error));
    }
    if let Signed::Rendered(rendered) = data {
        let written = envelope.member_written("data", |out| {
            out.reserve(rendered.text().len());
            rendered.write_canonical(rendered.root(), out)
        });
        if let Err(error) = written {
            return Ok(Err(error));
        }
    }
    Ok(envelope.finish())
}

/// The `data` a signed version carries: none (a logical delete), or the
/// body's one rendering, whose RFC 8785 form is read off its index.
#[derive(Clone, Copy)]
pub(crate) enum Signed<'a> {
    /// Void data (RM common master06 §Logical Deletion).
    Void,
    /// The commit body as rendered for `version.body`.
    Rendered(&'a crate::storage::render::index::Rendered),
}

/// Computes the server-generated `VERSION.signature` of the
/// `IMPORTED_VERSION` an import is about to persist (RM common master06
/// §Digital Signature).
///
/// The wrapper is signed like a local commit, over the value
/// [`build_imported_version`] produces with its own `signature` Void and
/// `item` whole, the original's foreign signature inside it untouched
/// (§Copying). Attestations added after the import (§Attestation) fall
/// outside the signed form.
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
    let iv = build_imported_version(
        &openehr_its::json::to_canonical_value(&contribution_ref(contribution_id)),
        commit_audit,
        item,
        None,
    );
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
    crate::telemetry::conversion::record("value_to_jcs");
    let canonical =
        openehr_rm::v1_2::common::change_control::version_impl::canonical_form_of_json(value)
            .map_err(ServiceError::SigningCanonical)?;
    let signature = ctx
        .signer
        .sign(&canonical)
        .map_err(ServiceError::SigningFailed)?;
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
            .map_err(ServiceError::SigningCanonical)?;
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
