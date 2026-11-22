// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A CONTRIBUTION of COMPOSITION members written in one guarded statement
//! (#6256): every member is resolved, signed and decomposed first, on the
//! placement its set's gate read found, and the CONTRIBUTION, every audit,
//! version, head and node row and the outbox row are written by one
//! statement that writes nothing unless what the gate read judged still
//! holds. One instant, the gate read's, stamps the whole set (RM common
//! master06 §Committal and Audits). No openEHR spec governs statement
//! batching: our own design.

use sqlx::PgConnection;
use uuid::Uuid;

use super::prepared::{Prepared, prepare};
use super::{Change, Committed, CommittedContribution, project, resolve_change};
use crate::error::ServiceError;
use crate::ids::EhrId;
use crate::storage::version_repo::guarded::{Guard, SetHead, SetMember, commit_set};
use crate::storage::version_repo::placement::Placement;
use crate::versioning::audit::AuditInput;
use crate::versioning::{SigningCtx, restriction};

/// One COMPOSITION member of a guarded CONTRIBUTION, with what its set's gate
/// read found of its object.
pub(crate) struct Member {
    /// The version's own `commit_audit`.
    pub(crate) audit: AuditInput,
    /// What the member commits.
    pub(crate) change: Change,
    /// The object's trunk tip and next ordinal (none for a create).
    pub(crate) placement: Option<Placement>,
    /// The object's head ordinal, which the statement requires unchanged.
    pub(crate) head: Option<i32>,
    /// What the gate read found of the EHR's and the object's marks.
    pub(crate) known: restriction::Known,
    /// What the version a logical delete deletes holds.
    pub(crate) deleted: Option<crate::system_log::categories::ContentIds>,
}

/// The CONTRIBUTION a guarded set writes.
pub(crate) struct Set<'a> {
    /// The owning EHR.
    pub(crate) ehr_id: EhrId,
    /// The client-supplied uid; `None` mints one.
    pub(crate) supplied_uid: Option<Uuid>,
    /// The CONTRIBUTION's own audit.
    pub(crate) audit: &'a AuditInput,
    /// The one instant the set is committed at, the gate read's `now()`.
    pub(crate) now: jiff::Timestamp,
    /// The policy and definition revisions the decision rested on.
    pub(crate) guard: Guard,
}

/// Write `set` and its `members` in one guarded statement.
///
/// # Errors
/// The per-member resolution refusals (restriction, lifecycle, content, the
/// access evaluator), the signing and decomposition errors, the party
/// reference refusals, and the [`commit_set`] errors: a moved requirement, a
/// CONTRIBUTION uid in use, and a version key a concurrent commit took first.
pub(crate) async fn commit_guarded_set(
    conn: &mut PgConnection,
    set: Set<'_>,
    members: Vec<Member>,
    ctx: &SigningCtx<'_>,
) -> Result<CommittedContribution, ServiceError> {
    let uid = set.supplied_uid.unwrap_or_else(|| ctx.stamp.mint());
    let mut audits = Vec::with_capacity(members.len());
    let mut prepared: Vec<(Prepared, _)> = Vec::with_capacity(members.len());
    let mut references = Vec::new();
    for (index, member) in members.into_iter().enumerate() {
        let known = restriction::Known {
            now: Some(set.now),
            ..member.known
        };
        let (mut resolved, access) = Box::pin(resolve_change(
            conn,
            (Some(set.ehr_id), ctx),
            &known,
            member.placement,
            member.change,
        ))
        .await?;
        references.extend(
            std::mem::take(&mut resolved.party_refs)
                .into_iter()
                .map(|r| (Some(index), r)),
        );
        let mut ready = prepare(ctx, &member.audit, uid, resolved)?;
        ready.committed.deleted = member.deleted;
        prepared.push((ready, (access, member.head)));
        audits.push(member.audit);
    }
    // NOTE: our own design (#6256), a COMPOSITION references parties outside its set, so
    // they are checked before the statement writes it.
    crate::versioning::party_refs::check(conn, &references).await?;
    let mut rows = Vec::with_capacity(prepared.len());
    for ((ready, (_, head)), audit) in prepared.iter().zip(&audits) {
        rows.push(SetMember {
            audit: audit.row(),
            version: ready.folded()?,
            expected_head: *head,
        });
    }
    let outbox = ctx.outbox_enabled.then(|| {
        let versions: Vec<_> = prepared
            .iter()
            .map(|(ready, _)| ready.committed.envelope_entry())
            .collect();
        crate::storage::version_repo::commit::outbox_envelope(
            uid,
            Some(set.ehr_id),
            set.now,
            &versions,
        )
    });
    let head = SetHead {
        id: uid,
        ehr_id: set.ehr_id,
        audit: set.audit.row(),
        time_committed: set.now,
    };
    let guard = Guard {
        outbox,
        ..set.guard
    };
    let time_committed = commit_set(conn, head, &rows, &guard).await?;
    drop(rows);
    let mut versions: Vec<Committed> = Vec::with_capacity(prepared.len());
    for (ready, (access, _)) in prepared {
        if ready.has_more() {
            ready.finish(conn).await?;
        }
        Box::pin(project::committed(
            conn,
            ctx,
            Some(set.ehr_id),
            &ready.committed,
            None,
            access,
        ))
        .await?;
        versions.push(ready.committed);
    }
    Ok(CommittedContribution {
        id: uid,
        time_committed,
        versions,
    })
}
