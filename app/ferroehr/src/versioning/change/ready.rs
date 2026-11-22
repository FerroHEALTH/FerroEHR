// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A guarded single-object write in two steps (#6256): resolved, signed and
//! decomposed first, so the outcome it commits is known, then committed in
//! one guarded statement. Between the two, the caller writes the write's
//! audit records under a reservation the statement inserts beside its rows,
//! so the records are durable before anything of the write is visible
//! (#5747, #5986) and the reservation commits with the write or not at all
//! (#5762). No openEHR spec governs how a commit is run against the
//! database: our own design.

use sqlx::PgConnection;
use uuid::Uuid;

use super::prepared::{Prepared, prepare};
use super::{
    Committed, ContributionCtx, Write, commit_folded, folded_shape, project, resolve_change,
    skipped_resend,
};
use crate::error::ServiceError;
use crate::ids::EhrId;
use crate::service::ehr::access_projection::AccessCommit;
use crate::storage::version_repo::guarded::Guard;
use crate::versioning::SigningCtx;
use crate::versioning::audit::AuditInput;

/// A guarded write resolved, signed and decomposed, before its statement.
pub(crate) struct Ready {
    /// The owning EHR.
    ehr_id: Option<EhrId>,
    /// The version's own `commit_audit`.
    audit: AuditInput,
    /// What the gate read judged.
    guard: Guard,
    /// What the write commits.
    form: Form,
}

/// What a [`Ready`] write commits.
enum Form {
    /// An identical re-send the opt-in rule answers with the existing
    /// version (#4415): no version is written.
    Skipped(Committed),
    /// A version, with what its projection and the party hooks read.
    Version {
        /// The version signed and decomposed.
        prepared: Box<Prepared>,
        /// The access projection the version moves.
        access: Option<AccessCommit>,
        /// What a party body holds that the binding hooks read.
        party_facts: Option<Box<crate::party_binding::derive::Facts>>,
    },
}

/// Resolve, sign and decompose the guarded `write` on `conn`, writing
/// nothing.
///
/// # Errors
/// The resolution refusals (restriction, placement, lifecycle, content, the
/// access evaluator, party references), and the signing and decomposition
/// errors.
pub(crate) async fn prepare_write(
    conn: &mut PgConnection,
    write: Write,
    guard: Guard,
    ctx: &SigningCtx<'_>,
) -> Result<Ready, ServiceError> {
    let Write {
        ehr_id,
        change,
        audit,
        known,
        placement,
        deleted,
        guard: _,
    } = write;
    let (mut resolved, access) = Box::pin(resolve_change(
        conn,
        (ehr_id, ctx),
        &known,
        placement,
        change,
    ))
    .await?;
    let references: Vec<_> = std::mem::take(&mut resolved.party_refs)
        .into_iter()
        .map(|r| (None, r))
        .collect();
    // NOTE: our own design (#6256), a version references parties outside its own write, so
    // they are checked before the statement writes it.
    crate::versioning::party_refs::check(conn, &references).await?;
    if ctx.skip_identical_resend
        && let Some(existing) = skipped_resend(&resolved, &audit)
    {
        return Ok(Ready {
            ehr_id,
            audit,
            guard,
            form: Form::Skipped(existing),
        });
    }
    let party_facts = resolved.party_facts.take();
    let mut prepared = prepare(ctx, &audit, ctx.stamp.mint(), resolved)?;
    prepared.committed.deleted = deleted;
    Ok(Ready {
        ehr_id,
        audit,
        guard,
        form: Form::Version {
            prepared: Box::new(prepared),
            access,
            party_facts,
        },
    })
}

impl Ready {
    /// The outcome the write commits: its version, commit instant and
    /// content, as its response and its audit records name them.
    pub(crate) fn committed(&self) -> &Committed {
        match &self.form {
            Form::Skipped(existing) => existing,
            Form::Version { prepared, .. } => &prepared.committed,
        }
    }

    /// Whether the write writes rows after its statement, which a
    /// transaction then holds with it: a FOLDER membership, accompanying
    /// attestations, blob references, or the access-delay hold of a new
    /// COMPOSITION.
    pub(crate) fn needs_transaction(&self, ctx: &SigningCtx<'_>) -> bool {
        match &self.form {
            Form::Skipped(_) => false,
            Form::Version {
                prepared, access, ..
            } => prepared.writes_after() || access.is_some() || ctx.access_delay.is_some(),
        }
    }

    /// Commit the write on `conn` in its one guarded statement, inserting
    /// `reservation`, the reservation of the records the caller holds, beside
    /// its rows; then what its statement leaves.
    ///
    /// # Errors
    /// The guarded statement's errors: a moved requirement
    /// ([`crate::storage::error::StorageError::GuardMoved`]), a version key a
    /// concurrent commit took first, and the storage errors of what follows.
    pub(crate) async fn commit(
        self,
        conn: &mut PgConnection,
        ctx: &SigningCtx<'_>,
        reservation: Option<Uuid>,
    ) -> Result<Committed, ServiceError> {
        let (prepared, access, party_facts) = match self.form {
            Form::Skipped(existing) => {
                if let Some(reservation) = reservation {
                    crate::system_log::reservation::reserve(conn, reservation).await?;
                }
                return Ok(existing);
            }
            Form::Version {
                prepared,
                access,
                party_facts,
            } => (prepared, access, party_facts),
        };
        let mut committed = prepared.committed.clone();
        let guard = Guard {
            reservation,
            ..self.guard
        };
        let statement = folded_shape(
            ContributionCtx::New(Some(guard)),
            prepared.contribution_id,
            prepared.folded()?,
            ctx,
            &committed,
        );
        committed.time_committed = commit_folded(conn, &self.audit.row(), statement)
            .await?
            .time_committed;
        if prepared.has_more() {
            prepared.finish(conn).await?;
        }
        Box::pin(project::committed(
            conn,
            ctx,
            self.ehr_id,
            &committed,
            party_facts.as_deref(),
            access,
        ))
        .await?;
        Ok(committed)
    }
}
