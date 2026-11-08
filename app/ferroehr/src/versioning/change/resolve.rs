// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The three resolvers of a change: each turns a [`super::Change`] arm into
//! the [`super::ResolvedWrite`] the commit stores, on the commit transaction.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): commit bodies are the canonical \
              form the commit interior carries (stored-content class)"
)]

use serde_json::Value;
use sqlx::PgConnection;

use super::{
    ResolvedWrite, canonical_body_text, next_version, reject_deleted_with_data, resolve_lifecycle,
    retire_protected_identifiers, seal_protected_identifiers, stamp_version_uid,
    validate_transition,
};
use crate::error::ServiceError;
use crate::ids::{AllocatedVoId, EhrId, VoId};
use crate::storage::codec::decompose;
use crate::storage::row::NodeRow;
use crate::versioning::object_version_id::{TreeId, object_version_id};
use crate::versioning::{
    Kind, SigningCtx, attestation, external, lifecycle, person, profile, system,
};

/// A logically deleted version of `vo_id` after the open tip `expected` names.
#[expect(
    clippy::too_many_arguments,
    reason = "the fields of `Change::Delete` as it is destructured; a struct \
              for them would repeat the enum variant"
)]
pub(super) async fn resolve_delete(
    tx: &mut PgConnection,
    ctx: &SigningCtx<'_>,
    at: Placing,
    vo_id: VoId,
    kind: Kind,
    expected: Option<TreeId>,
    signature: Option<String>,
    other_input_version_uids: Vec<String>,
) -> Result<ResolvedWrite, ServiceError> {
    let next = next_version(
        tx,
        at.ehr_id,
        vo_id,
        kind,
        expected,
        &ctx.system_id,
        at.preplaced,
    )
    .await?;
    person::refuse_altering_others(tx, kind, vo_id, next.preceding_tree).await?;
    system::admit_delete(tx, kind, vo_id).await?;
    external::admit_delete()?;
    lifecycle::refuse_already_deleted(kind, vo_id, &next.preceding_lifecycle)?;
    retire_protected_identifiers(tx, ctx, kind, vo_id).await?;
    // A deleted version carries no data — its `ORIGINAL_VERSION.data` is
    // Void (master06 §Logical Deletion); the signature is over the
    // data-less version wrapper.
    Ok(ResolvedWrite {
        kind,
        vo_id,
        ehr_id: at.ehr_id,
        ordinal: next.ordinal,
        tree: next.tree,
        lifecycle: lifecycle::state::DELETED.to_owned(),
        preceding_uid: Some(next.preceding_uid),
        other_input_version_uids,
        template_id: None,
        client_signature: signature,
        rows: Vec::new(),
        canonical_text: None,
        content_digest: None,
        shared_from: None,
        preceding: None,
        // A deleted version's data is Void (master06 §Logical
        // Deletion): there is no body a generation could fail to
        // express.
        stable_compatible: true,
        origins: Vec::new(),
        attestations: Vec::new(),
        is_first_folder: false,
        time_committed: at.known_now.unwrap_or(next.now),
    })
}

/// Where a write lands: its EHR, the commit instant the caller read and the
/// placement it already holds.
pub(super) struct Placing {
    pub(super) ehr_id: Option<EhrId>,
    pub(super) known_now: Option<jiff::Timestamp>,
    pub(super) preplaced: Option<crate::storage::version_repo::placement::Placement>,
}

/// The content half of a create or a modify, as the change carries it.
pub(super) struct Content {
    pub(super) allocated: Option<AllocatedVoId>,
    pub(super) kind: Kind,
    pub(super) canonical: Value,
    pub(super) lifecycle_state: Option<String>,
    pub(super) template_id: Option<String>,
    pub(super) signature: Option<String>,
    pub(super) attestations: Vec<attestation::AttestationInput>,
    pub(super) other_input_version_uids: Vec<String>,
}

/// A body ready to store: its node rows, its canonical text, its
/// `stable_compatible` stamp and its origins.
struct Prepared {
    rows: Vec<NodeRow>,
    canonical_text: Option<String>,
    stable_compatible: bool,
    origins: Vec<String>,
    /// SHA-256 over the canonical body with the root `uid` removed
    /// ([`content_digest`]).
    content_digest: [u8; 32],
}

/// SHA-256 over the canonical ITS-JSON bytes of `canonical` with the root
/// `uid` removed, the digest a version's content is compared with its
/// predecessor's on (#4415).
///
/// NOTE: RM ehr `master05-composition_package.adoc` §Event Context makes
/// `context` content (a changed `start_time` or `setting` is a clinical
/// change), and `feeder_audit` is content the version carries, so only the
/// server-assigned `uid` leaves the digested bytes; the serializer is the one
/// that writes `version.body`, so equal bodies digest equal.
fn content_digest(canonical: &Value) -> Result<[u8; 32], ServiceError> {
    use sha2::Digest as _;
    let mut without_uid = canonical.clone();
    if let Some(root) = without_uid.as_object_mut() {
        root.shift_remove("uid");
    }
    let text = canonical_body_text(&without_uid)?;
    Ok(sha2::Sha256::digest(text.as_bytes()).into())
}

/// The lifecycle state a content write commits: the large inline
/// `DV_MULTIMEDIA` externalized first, so the stored, served and signed form is
/// the offloaded one, and the deleted state refused, because deleting the data
/// and setting the state are one act (master06 §Logical Deletion).
#[cfg_attr(
    not(feature = "multimedia"),
    expect(
        clippy::unused_async,
        reason = "the offload it awaits is the `multimedia` feature's; a slim build has none"
    )
)]
async fn content_lifecycle(
    ctx: &SigningCtx<'_>,
    canonical: &mut Value,
    lifecycle_state: Option<String>,
) -> Result<String, ServiceError> {
    #[cfg(feature = "multimedia")]
    if let Some(engine) = ctx.multimedia {
        engine
            .offload(canonical)
            .await
            .map_err(|e| ServiceError::internal("externalize multimedia", e))?;
    }
    #[cfg(not(feature = "multimedia"))]
    let _ = (ctx, &canonical);
    let lifecycle = resolve_lifecycle(lifecycle_state)?;
    reject_deleted_with_data(&lifecycle)?;
    Ok(lifecycle)
}

/// Seal the protected identifiers, stamp the version uid and decompose the
/// ACCEPTED body: the stored node rows reassemble to these bytes, and the
/// released-generation reader's answer is what a later `stable` deployment
/// reads back.
async fn prepare_body(
    tx: &mut PgConnection,
    ctx: &SigningCtx<'_>,
    kind: Kind,
    vo_id: VoId,
    tree: TreeId,
    mut canonical: Value,
) -> Result<Prepared, ServiceError> {
    seal_protected_identifiers(tx, ctx, kind, vo_id, &mut canonical).await?;
    stamp_version_uid(
        &mut canonical,
        &object_version_id(vo_id, &ctx.system_id, tree),
    )?;
    let stable_compatible = profile::stable_compatible(ctx.spec_profile, kind, &canonical);
    let origins = crate::versioning::origins::origins(&ctx.direct_origin, &canonical);
    let canonical_text = Some(canonical_body_text(&canonical)?);
    let content_digest = content_digest(&canonical)?;
    Ok(Prepared {
        rows: decompose(canonical)?,
        canonical_text,
        stable_compatible,
        origins,
        content_digest,
    })
}

/// The first version of a new object.
pub(super) async fn resolve_create(
    tx: &mut PgConnection,
    ctx: &SigningCtx<'_>,
    at: Placing,
    mut content: Content,
) -> Result<ResolvedWrite, ServiceError> {
    let lifecycle =
        content_lifecycle(ctx, &mut content.canonical, content.lifecycle_state.take()).await?;
    // a first version can only be `complete`/`incomplete`.
    validate_transition(None, &lifecycle)?;
    person::refuse_composing_for_others(content.kind, &content.canonical)?;
    system::admit_content(
        content.kind,
        &content.canonical,
        content.template_id.as_deref(),
    )?;
    external::admit_content(
        content.kind,
        &content.canonical,
        content.template_id.as_deref(),
    )?;
    let vo_id = content
        .allocated
        .take()
        .map_or_else(|| VoId::minted(ctx.stamp), |allocated| allocated.id());
    let tree = TreeId::trunk(1);
    let body = prepare_body(tx, ctx, content.kind, vo_id, tree, content.canonical).await?;
    let time_committed = match at.known_now {
        Some(ts) => ts,
        None => crate::storage::version_repo::placement::tx_now(tx).await?,
    };
    Ok(ResolvedWrite {
        kind: content.kind,
        vo_id,
        ehr_id: at.ehr_id,
        ordinal: 1,
        tree,
        lifecycle,
        preceding_uid: None,
        other_input_version_uids: content.other_input_version_uids,
        template_id: content.template_id,
        client_signature: content.signature,
        rows: body.rows,
        canonical_text: body.canonical_text,
        content_digest: Some(body.content_digest),
        shared_from: None,
        preceding: None,
        stable_compatible: body.stable_compatible,
        origins: body.origins,
        attestations: content.attestations,
        is_first_folder: content.kind == Kind::Folder && at.ehr_id.is_some(),
        time_committed,
    })
}

/// A new version of `vo_id` after the open tip `expected` names.
pub(super) async fn resolve_modify(
    tx: &mut PgConnection,
    ctx: &SigningCtx<'_>,
    at: Placing,
    vo_id: VoId,
    expected: Option<TreeId>,
    mut content: Content,
) -> Result<ResolvedWrite, ServiceError> {
    let kind = content.kind;
    let lifecycle =
        content_lifecycle(ctx, &mut content.canonical, content.lifecycle_state.take()).await?;
    let next = next_version(
        tx,
        at.ehr_id,
        vo_id,
        kind,
        expected,
        &ctx.system_id,
        at.preplaced,
    )
    .await?;
    person::refuse_composing_for_others(kind, &content.canonical)?;
    system::admit_content(kind, &content.canonical, content.template_id.as_deref())?;
    external::admit_content(kind, &content.canonical, content.template_id.as_deref())?;
    person::refuse_altering_others(tx, kind, vo_id, next.preceding_tree).await?;
    // the transition from the preceding version's state must be legal
    // (master06 §Version Lifecycle state machine).
    validate_transition(Some(&next.preceding_lifecycle), &lifecycle)?;
    let body = prepare_body(tx, ctx, kind, vo_id, next.tree, content.canonical).await?;
    // Content equal to the preceding tip's is stored once: the new version
    // reads the tip's rows (its own, or the ones it shares), and nothing of
    // the commit's own record is skipped (#4415).
    let shared_from = (next.preceding.content_digest.as_deref() == Some(&body.content_digest[..]))
        .then_some(next.preceding.content_key);
    Ok(ResolvedWrite {
        kind,
        vo_id,
        ehr_id: at.ehr_id,
        ordinal: next.ordinal,
        tree: next.tree,
        lifecycle,
        preceding_uid: Some(next.preceding_uid),
        other_input_version_uids: content.other_input_version_uids,
        template_id: content.template_id,
        client_signature: content.signature,
        rows: body.rows,
        canonical_text: body.canonical_text,
        content_digest: Some(body.content_digest),
        shared_from,
        preceding: Some(next.preceding),
        stable_compatible: body.stable_compatible,
        origins: body.origins,
        attestations: content.attestations,
        is_first_folder: false,
        time_committed: at.known_now.unwrap_or(next.now),
    })
}
