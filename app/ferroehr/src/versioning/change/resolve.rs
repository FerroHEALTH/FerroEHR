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
    ResolvedWrite, next_version, reject_deleted_with_data, resolve_lifecycle, validate_transition,
    version_uid,
};
use crate::error::ServiceError;
use crate::ids::{AllocatedVoId, EhrId, VoId};
use crate::storage::identifier_repo;
use crate::storage::render::index::Rendered;
use crate::versioning::body::Body;
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
    system::admit_delete(kind)?;
    external::admit_delete(tx, kind, vo_id).await?;
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
        body: None,
        rendered: None,
        content_digest: None,
        party_facts: None,
        party_refs: Vec::new(),
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
    pub(super) body: Body,
    pub(super) lifecycle_state: Option<String>,
    pub(super) template_id: Option<String>,
    pub(super) signature: Option<String>,
    pub(super) attestations: Vec<attestation::AttestationInput>,
    pub(super) other_input_version_uids: Vec<String>,
}

/// A body ready to store: the body itself, its canonical text, its
/// `stable_compatible` stamp and its origins.
struct Prepared {
    /// The sealed, uid-stamped body.
    body: Body,
    /// Its one rendering: the `version.body` text and the index the node rows
    /// are read off.
    rendered: Rendered,
    stable_compatible: bool,
    origins: Vec<String>,
    /// SHA-256 over the canonical body with the root `uid` removed
    /// ([`content_digest`]).
    content_digest: [u8; 32],
    /// What a party body holds that the binding hooks read, derived before
    /// the body is decomposed (#5220); `None` for every other kind.
    party_facts: Option<Box<crate::party_binding::derive::Facts>>,
    /// The party references the body carries, judged after the commit's
    /// inserts ([`crate::versioning::party_refs`]).
    party_refs: Vec<crate::versioning::party_refs::Reference>,
}

/// The body bytes a point read serves verbatim (`version.body`) and their
/// content digest ([`crate::storage::render::content_digest`]), from one
/// rendering of the canonical value `canonical`.
///
/// # Errors
/// [`ServiceError`] when the value cannot serialize (a non-finite number,
/// unreachable for a codec-produced canonical value, refused loudly anyway).
pub(crate) fn canonical_text_and_digest(
    canonical: &Value,
) -> Result<(String, [u8; 32]), ServiceError> {
    let rendered = Body::render_canonical(canonical)?;
    let digest = crate::storage::render::content_digest(&rendered);
    let text = String::from_utf8(rendered.into_text())
        .map_err(|e| ServiceError::internal("serialize the canonical body", e))?;
    Ok((text, digest))
}

/// The lifecycle state a content write commits, with the deleted state refused,
/// because deleting the data and setting the state are one act (master06
/// §Logical Deletion). The body must arrive with its large inline
/// `DV_MULTIMEDIA` already externalized (`FerroEhrService::externalize`, run
/// before the transaction opens), so the stored, served and signed form is the
/// offloaded one and no upload waits under the transaction.
fn content_lifecycle(
    ctx: &SigningCtx<'_>,
    body: &Body,
    lifecycle_state: Option<String>,
) -> Result<String, ServiceError> {
    #[cfg(feature = "multimedia")]
    if let Some(engine) = ctx.multimedia {
        match body {
            Body::Canonical(canonical) => engine.ensure_externalized(canonical),
            typed => match typed.as_rm() {
                Some(rm) => engine.ensure_externalized_typed(rm),
                None => Ok(()),
            },
        }
        .map_err(|e| ServiceError::internal("externalize multimedia", e))?;
    }
    #[cfg(not(feature = "multimedia"))]
    let _ = (ctx, body);
    let lifecycle = resolve_lifecycle(lifecycle_state)?;
    reject_deleted_with_data(&lifecycle)?;
    Ok(lifecycle)
}

/// Seal the protected identifiers and stamp the version uid of the ACCEPTED
/// body: the node rows the write decomposes it into reassemble to these bytes, and the
/// released-generation reader's answer is what a later `stable` deployment
/// reads back.
async fn prepare_body(
    tx: &mut PgConnection,
    ctx: &SigningCtx<'_>,
    kind: Kind,
    vo_id: VoId,
    tree: TreeId,
    mut body: Body,
) -> Result<Prepared, ServiceError> {
    seal_protected_identifiers(tx, ctx, kind, vo_id, &mut body).await?;
    body.stamp_uid(&version_uid(&object_version_id(
        vo_id,
        &ctx.system_id,
        tree,
    ))?);
    let stable_compatible = match &body {
        Body::Canonical(canonical) => profile::stable_compatible(ctx.spec_profile, kind, canonical),
        typed => {
            ctx.spec_profile == crate::config::profile::SpecProfile::Stable
                || typed.expressible_in_released().unwrap_or(false)
        }
    };
    let rendered = body.render()?;
    let origins = ctx
        .direct_origin
        .stamp_fed(crate::storage::render::fed_origins(&rendered))?;
    let content_digest = crate::storage::render::content_digest(&rendered);
    // NOTE: the party hooks read the one rendering of the body, whichever
    // form it was committed as (#6003).
    let read = crate::versioning::tree::Indexed::root(&rendered);
    let party_facts = kind.is_demographic().then(|| {
        Box::new(crate::party_binding::derive::Facts::of(
            kind,
            read,
            ctx.binding,
        ))
    });
    let party_refs = crate::versioning::party_refs::of_body(kind, read);
    Ok(Prepared {
        body,
        rendered,
        stable_compatible,
        origins,
        content_digest,
        party_facts,
        party_refs,
    })
}

/// The first version of a new object.
pub(super) async fn resolve_create(
    tx: &mut PgConnection,
    ctx: &SigningCtx<'_>,
    at: Placing,
    mut content: Content,
) -> Result<ResolvedWrite, ServiceError> {
    let lifecycle = content_lifecycle(ctx, &content.body, content.lifecycle_state.take())?;
    // a first version can only be `complete`/`incomplete`.
    validate_transition(None, &lifecycle)?;
    person::refuse_composing_for_others(content.kind, &content.body)?;
    system::admit_content(content.kind, &content.body)?;
    external::admit_content(content.kind, &content.body, content.template_id.as_deref())?;
    let vo_id = content
        .allocated
        .take()
        .map_or_else(|| VoId::minted(ctx.stamp), |allocated| allocated.id());
    let tree = TreeId::trunk(1);
    let body = prepare_body(tx, ctx, content.kind, vo_id, tree, content.body).await?;
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
        body: Some(body.body),
        rendered: Some(body.rendered),
        content_digest: Some(body.content_digest),
        party_facts: body.party_facts,
        party_refs: body.party_refs,
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
    let lifecycle = content_lifecycle(ctx, &content.body, content.lifecycle_state.take())?;
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
    person::refuse_composing_for_others(kind, &content.body)?;
    system::admit_content(kind, &content.body)?;
    external::admit_content(kind, &content.body, content.template_id.as_deref())?;
    person::refuse_altering_others(tx, kind, vo_id, next.preceding_tree).await?;
    // the transition from the preceding version's state must be legal
    // (master06 §Version Lifecycle state machine).
    validate_transition(Some(&next.preceding_lifecycle), &lifecycle)?;
    let body = prepare_body(tx, ctx, kind, vo_id, next.tree, content.body).await?;
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
        body: Some(body.body),
        rendered: Some(body.rendered),
        content_digest: Some(body.content_digest),
        party_facts: body.party_facts,
        party_refs: body.party_refs,
        shared_from,
        preceding: Some(next.preceding),
        stable_compatible: body.stable_compatible,
        origins: body.origins,
        attestations: content.attestations,
        is_first_folder: false,
        time_committed: at.known_now.unwrap_or(next.now),
    })
}

/// Seal every protected national identifier of a demographic body into rows
/// of this version, on the commit transaction `tx`, before the body is
/// decomposed and signed.
///
/// Runs on the demographic kinds only: a party's identifiers are the point of
/// that domain, and clinical content carrying one is a different rule
/// altogether ([`crate::privacy`] refuses it outright rather than sealing it).
/// A demographic kind's commit transaction is always on the party domain's
/// pool, the one database that holds `national_identifier` in every layout,
/// so the sealed rows commit or roll back with the version that references
/// them and no compensating cleanup is needed.
///
/// Placed before `decompose` and the signature for the same reason the
/// multimedia offload is: the stored, signed and served body must be one form,
/// which is what lets a later read verify a signature against the bytes it
/// actually holds. No openEHR spec governs identifier protection — our own
/// design/extension.
///
/// # Errors
/// The [`ServiceError`] the store's refusal maps to: `409` for a
/// value another party holds, `422` for a body the store cannot hold, `500`
/// for a configuration or storage fault. The commit fails rather than
/// proceeding, because proceeding would store the body with the identifier
/// still in it.
async fn seal_protected_identifiers(
    tx: &mut PgConnection,
    ctx: &SigningCtx<'_>,
    kind: Kind,
    vo_id: VoId,
    body: &mut Body,
) -> Result<(), ServiceError> {
    let Some(engine) = ctx.identifiers else {
        return Ok(());
    };
    if !kind.is_demographic() {
        return Ok(());
    }
    engine.externalize(tx, vo_id.0, body).await?;
    Ok(())
}

/// Take a logically deleted party's identifiers out of resolution, on the
/// commit transaction `tx`.
///
/// A deleted version carries no data (RM common
/// `master06-change_control_package.adoc` §Logical Deletion), so the party
/// holds no identifier from it on; the earlier versions keep their rows. No
/// openEHR spec governs identifier protection — our own design/extension.
///
/// # Errors
/// The [`ServiceError`] a failed update maps to: a `500` for a storage fault.
async fn retire_protected_identifiers(
    tx: &mut PgConnection,
    ctx: &SigningCtx<'_>,
    kind: Kind,
    vo_id: VoId,
) -> Result<(), ServiceError> {
    if ctx.identifiers.is_none() || !kind.is_demographic() {
        return Ok(());
    }
    identifier_repo::retire_live(tx, vo_id).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::canonical_text_and_digest;

    /// The digest of the one-pass writer, written the way the two passes it
    /// replaced wrote it: the compact text with the root `uid` taken out.
    fn two_pass(canonical: &serde_json::Value) -> (String, [u8; 32]) {
        use sha2::Digest as _;
        let mut without = canonical.clone();
        if let Some(map) = without.as_object_mut() {
            map.shift_remove("uid");
        }
        let digested = serde_json::to_string(&without).unwrap();
        (
            serde_json::to_string(canonical).unwrap(),
            sha2::Sha256::digest(digested.as_bytes()).into(),
        )
    }

    #[test]
    fn one_pass_writes_the_text_and_digest_two_passes_wrote() {
        let uid = json!({ "_type": "OBJECT_VERSION_ID", "value": "a::b::1" });
        for canonical in [
            json!({ "_type": "COMPOSITION", "uid": uid, "name": { "value": "\"ä\"" } }),
            json!({ "uid": uid, "_type": "COMPOSITION", "language": 1.5 }),
            json!({ "_type": "COMPOSITION", "name": "n", "uid": uid }),
            json!({ "_type": "COMPOSITION" }),
            json!({ "uid": uid }),
            json!({}),
            json!(null),
        ] {
            assert_eq!(
                canonical_text_and_digest(&canonical).unwrap(),
                two_pass(&canonical),
                "{canonical}"
            );
        }
    }
}
