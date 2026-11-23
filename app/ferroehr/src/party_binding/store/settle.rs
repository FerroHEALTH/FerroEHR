// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The settle of a party commit: the grant rows, the caller facts, the
//! record subjects, the patient identifiers and the binding rows of its new
//! head, written as ordered statements in its transaction (#6257).
//!
//! Every delete and retirement goes in one statement and every insert in the
//! next, so an identifier's retirement runs a statement before its insert
//! and a value that moves between rows never meets its old row in a unique
//! index. Each step of one statement writes rows no other step of it
//! writes. The facts stamp and the grant revision follow, each only when the
//! rows the two statements returned say its rows changed, and a `ROLE`
//! rewrites its performers' rows as a refill does. No openEHR spec governs
//! the side table or its SQL: our own design.

use sqlx::PgConnection;

use super::wanted::{GROUP_SELECTORS, PersonRows, group_rows, person_selectors, tombstones_of};
use super::{Head, Scope, professional, write_performers};
use crate::party_binding::BindingMapping;
use crate::party_binding::grants;
use crate::party_binding::rows::{self, Retired};
use crate::party_binding::step::{self, Tag};
use crate::storage::error::StorageError;
use crate::versioning::Kind;

/// Writes the rows of `head`, the new head of a party commit of `kind`, on
/// its commit transaction `tx`, as [`super::reindex_heads`] writes them for
/// one head.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure, the unique violations of
/// the selector indexes and of `uq_binding_client` included.
pub(super) async fn commit(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    kind: Kind,
    head: &Head,
) -> Result<(), StorageError> {
    let heads = std::slice::from_ref(head);
    let parties = [head.vo_id];
    let mapped = !mapping.is_empty();
    let persons = (mapped && kind == Kind::Person)
        .then(|| PersonRows::of(mapping, heads, Scope::Heads { tombstones: true }));
    let professional = match &persons {
        Some(rows) => professional(tx, mapping, &rows.listed).await?,
        None => Vec::new(),
    };
    let tombstones = match &persons {
        Some(_) => tombstones_of(mapping, heads),
        None => Vec::new(),
    };
    let groups = if mapped && kind == Kind::Group {
        group_rows(mapping, heads)
    } else {
        None
    };
    let (mut clear, mut fill) = grants::store::steps(kind, head.grant_head());
    if let Some(rows) = &persons {
        let (selectors, _) = person_selectors(Scope::Heads { tombstones: true });
        let identifiers = &rows.identifiers;
        clear.push(rows::retire_identifiers(
            &parties,
            selectors,
            identifiers,
            &professional,
        ));
        clear.push(rows::retire_memberships(&parties, &rows.memberships));
        fill.push(rows::insert_identifiers(
            &parties,
            selectors.0,
            identifiers,
            &professional,
        ));
        fill.push(rows::add_memberships(&parties, &rows.memberships));
        fill.push(rows::tombstones(&tombstones));
        fill.push(rows::revive_held(identifiers));
    }
    if let Some(identifiers) = &groups {
        let selectors = (&GROUP_SELECTORS[..], None);
        clear.push(rows::retire_identifiers(
            &parties,
            selectors,
            identifiers,
            &professional,
        ));
        fill.push(rows::insert_identifiers(
            &parties,
            selectors.0,
            identifiers,
            &professional,
        ));
    }
    let mut written = step::batch(tx, clear).await?;
    if persons.is_some() {
        let retired = step::parties(&written, Tag::Groups);
        fill.push(rows::revise_groups(Retired::Before(retired)));
    }
    written.extend(step::batch(tx, fill).await?);
    if persons.is_some() || groups.is_some() {
        crate::party_binding::live::note_change();
    }
    let mut stamped = grants::caller::store::changed(&written);
    if step::differ(&written, Tag::SubjectsGone, Tag::SubjectsCame) {
        stamped.push(head.vo_id);
    }
    grants::caller::store::finish(tx, stamped).await?;
    if written.iter().any(|row| row.tag == Tag::Grants) {
        grants::store::revise(tx).await?;
        grants::live::note_change();
    }
    if mapped && kind == Kind::Role {
        write_performers(tx, mapping, heads).await?;
    }
    Ok(())
}
