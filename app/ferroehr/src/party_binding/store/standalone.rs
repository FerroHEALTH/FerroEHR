// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The side-table rows of a `PARTY_RELATIONSHIP` of its own: a
//! representation, named `represents`, with its basis, provider and writer
//! (#6079), and a care relationship, whose caller fact is about its source
//! (#6055), with the end it takes from the access policy (#6073). A
//! representation's row names its container, and the key row of the
//! `PERSON` it names as its source counts it. No openEHR spec governs the
//! side table: our own design.

use sqlx::PgConnection;
use uuid::Uuid;

use super::{DELETED, Scope, head_of, stored_heads, write_persons};
use crate::error::ServiceError;
use crate::party_binding::BindingMapping;
use crate::party_binding::derive::Facts;
use crate::party_binding::grants;
use crate::party_binding::grants::store::GrantHead;
use crate::party_binding::rows::{self, RepresentationRow};
use crate::storage::error::StorageError;
use crate::storage::party_reverse;
use crate::versioning::change::Committed;
use crate::versioning::{Kind, SigningCtx};

/// Judges the commit `committed` of a standalone `PARTY_RELATIONSHIP` holding
/// `facts` (`None` for a logical delete) against the protection rules under
/// `mapping` and, for a care relationship, its writer's grant, then brings
/// its reverse-relationship index and, at its trunk head, its representation
/// row and its care fact to it, on its commit transaction `tx`.
///
/// # Errors
/// The protection refusals, the care writer's `403`, and a storage failure.
pub(super) async fn committed(
    tx: &mut PgConnection,
    ctx: &SigningCtx<'_>,
    committed: &Committed,
    facts: Option<&Facts>,
) -> Result<(), ServiceError> {
    let mapping = ctx.binding;
    crate::party_binding::protect::check(tx, mapping, committed, facts).await?;
    crate::party_binding::protect::representation(tx, mapping, committed.vo_id, facts).await?;
    let changed = grants::caller::writer::check_care(tx, mapping, committed, facts).await?;
    let targets = facts.map_or(&[][..], |f| f.targets.as_slice());
    party_reverse::replace(tx, committed.vo_id, targets, committed.time_committed).await?;
    let (lifecycle_state, trunk_head) = head_of(tx, committed.vo_id).await?;
    if trunk_head == Some(committed.sys_version) {
        reindex(tx, mapping, committed.vo_id.0, facts).await?;
        let live = facts.filter(|_| lifecycle_state != DELETED);
        let judged = grants::caller::bounds::Head::from(committed);
        grants::caller::bounds::judge(tx, &ctx.access_policy, &judged, live, changed).await?;
        let head = GrantHead {
            vo_id: committed.vo_id.0,
            lifecycle_state: &lifecycle_state,
            facts,
        };
        grants::store::write(tx, Kind::PartyRelationship, head).await?;
    }
    Ok(())
}

/// Writes the representation row the standalone `PARTY_RELATIONSHIP`
/// `container` holds under `facts` (`None` for a deleted one), and brings the
/// rows of the `PERSON`s it names as its source, before and after, to their
/// heads, so a representative's key row counts it, on `tx`.
pub(super) async fn reindex(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    container: Uuid,
    facts: Option<&Facts>,
) -> Result<(), StorageError> {
    let before = rows::standalone_representative(tx, container).await?;
    let row = facts.and_then(|facts| {
        Some(RepresentationRow {
            representative: facts.representative?,
            representation: facts.representations.first()?.clone(),
        })
    });
    rows::write_standalone(tx, container, row.as_ref()).await?;
    if mapping.is_empty() {
        return Ok(());
    }
    let mut sources: Vec<Uuid> = before
        .into_iter()
        .chain(row.map(|row| row.representative))
        .collect();
    sources.sort_unstable();
    sources.dedup();
    if sources.is_empty() {
        return Ok(());
    }
    let persons = stored_heads(tx, mapping, Kind::Person, Some(&sources), None).await?;
    write_persons(tx, mapping, &persons, Scope::Heads { tombstones: false }).await
}

/// Writes the row of every standalone representation container that is not
/// deleted, on `tx`, before the `PERSON` rows that count them.
pub(super) async fn refill(tx: &mut PgConnection) -> Result<(), StorageError> {
    history(tx).await?;
    let mut after = None;
    loop {
        let page = stored_heads(
            tx,
            &BindingMapping::default(),
            Kind::PartyRelationship,
            None,
            after,
        )
        .await?;
        let Some(last) = page.last() else {
            break;
        };
        after = Some(last.vo_id);
        for head in page.iter().filter(|head| head.lifecycle_state != DELETED) {
            let Some(facts) = &head.facts else {
                continue;
            };
            reindex(tx, &BindingMapping::default(), head.vo_id, Some(facts)).await?;
        }
    }
    Ok(())
}

/// The versions [`history`] reads per page.
const HISTORY_PAGE: i64 = 500;

/// Writes a retired row for every representation any version of a
/// `PARTY_RELATIONSHIP` ever held, on `tx`, so a refill keeps
/// who once represented whom: a former representative is judged as one and
/// never as the patient (#5532). The live rows follow from the heads.
async fn history(tx: &mut PgConnection) -> Result<(), StorageError> {
    let mut after: (Uuid, i32) = (Uuid::nil(), -1);
    loop {
        let page: Vec<(Uuid, i32, String)> = sqlx::query_as(
            "SELECT v.vo_id, v.sys_version, v.body FROM version v \
             JOIN vo_head h ON h.tier = v.tier AND h.vo_id = v.vo_id \
             WHERE h.kind = 'PARTY_RELATIONSHIP' AND v.body IS NOT NULL \
               AND (v.vo_id, v.sys_version) > ($1, $2) \
               AND jsonb_path_exists(v.body::jsonb, \
                   '$.**?(@.code_string == \"represents\")') \
             ORDER BY v.vo_id, v.sys_version LIMIT $3",
        )
        .bind(after.0)
        .bind(after.1)
        .bind(HISTORY_PAGE)
        .fetch_all(&mut *tx)
        .await?;
        let Some((vo_id, sys_version, ..)) = page.last() else {
            return Ok(());
        };
        after = (*vo_id, *sys_version);
        let mut held: Vec<(RepresentationRow, Uuid)> = Vec::new();
        for (vo_id, _, body) in &page {
            let facts = super::parsed(Kind::PartyRelationship, body, &BindingMapping::default())?;
            let Some(representative) = facts.representative else {
                continue;
            };
            held.extend(facts.representations.iter().map(|representation| {
                let row = RepresentationRow {
                    representative,
                    representation: representation.clone(),
                };
                (row, *vo_id)
            }));
        }
        rows::write_history(tx, &held).await?;
    }
}
