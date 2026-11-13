// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The side-table row of a standalone representation: a `PARTY_RELATIONSHIP`
//! of its own carrying the representation code, which is how a patient
//! authorises a proxy (#5446). Its row names its container, and the key row
//! of the `PERSON` it names as its source counts it. No openEHR spec governs
//! the side table: our own design.

use sqlx::PgConnection;
use uuid::Uuid;

use super::{DELETED, head_of, stored_heads, write_persons};
use crate::error::ServiceError;
use crate::party_binding::BindingMapping;
use crate::party_binding::derive::Facts;
use crate::party_binding::rows::{self, RepresentationRow};
use crate::storage::error::StorageError;
use crate::storage::party_reverse;
use crate::versioning::Kind;
use crate::versioning::change::Committed;

/// Judges the commit `committed` of a standalone `PARTY_RELATIONSHIP` holding
/// `facts` (`None` for a logical delete) against the protection rules under
/// `mapping`, then brings its reverse-relationship index and, at its trunk
/// head, its representation row to it, on its commit transaction `tx`.
///
/// # Errors
/// The protection refusals and a storage failure.
pub(super) async fn committed(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    committed: &Committed,
    facts: Option<&Facts>,
) -> Result<(), ServiceError> {
    crate::party_binding::protect::check(tx, mapping, committed, facts).await?;
    crate::party_binding::protect::representation(tx, mapping, committed.vo_id, facts).await?;
    let targets = facts.map_or(&[][..], |f| f.targets.as_slice());
    party_reverse::replace(tx, committed.vo_id, targets, committed.time_committed).await?;
    let (_, trunk_head) = head_of(tx, committed.vo_id).await?;
    if trunk_head == Some(committed.sys_version) {
        reindex(tx, mapping, committed.vo_id.0, facts).await?;
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
        let representation = facts.representations.first()?;
        Some(RepresentationRow {
            representative: facts.representative?,
            subject: representation.subject,
            code: representation.code.clone(),
            validity: representation.validity.clone(),
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
    write_persons(tx, mapping, &persons, false).await
}

/// Writes the row of every standalone representation container that is not
/// deleted, on `tx`, before the `PERSON` rows that count them.
pub(super) async fn refill(tx: &mut PgConnection) -> Result<(), StorageError> {
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
