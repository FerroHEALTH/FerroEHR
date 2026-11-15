// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The selector identifiers every live `PERSON` head holds, bound or not.
//!
//! A fill holds each value to one party by them, and the pre-flight reports
//! them. No openEHR spec governs the side table: our own design.

use std::collections::BTreeMap;

use sqlx::{PgConnection, Row};
use uuid::Uuid;

use super::{DELETED, stored_heads};
use crate::party_binding::BindingMapping;
use crate::storage::error::StorageError;
use crate::versioning::Kind;

/// One selector identifier of a live `PERSON` head, as the unique indexes
/// compare it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct HeldSelector {
    /// `key`, `user_name` or `external_id`.
    pub selector: &'static str,
    /// The `DV_IDENTIFIER.issuer`; empty for a `userName`, which is unique
    /// across issuers.
    pub issuer: String,
    /// The compared value: the canonical key, the folded `userName`, or the
    /// `externalId`.
    pub value: String,
    /// The `PERSON`'s container.
    pub party_id: Uuid,
}

/// Every selector identifier of every live `PERSON` head of a mapped
/// template, whether or not it binds, on `tx`: what [`super::refill`] would hold
/// each value to one party by.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn held_selectors(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
) -> Result<Vec<HeldSelector>, StorageError> {
    let mut out = Vec::new();
    let mut after = None;
    loop {
        let page = stored_heads(tx, mapping, Kind::Person, None, after).await?;
        let Some(last) = page.last() else {
            break;
        };
        after = Some(last.vo_id);
        let start = out.len();
        let mut names = Vec::new();
        for head in &page {
            let Some(facts) = head
                .facts
                .as_ref()
                .filter(|_| head.lifecycle_state != DELETED)
            else {
                continue;
            };
            if !facts
                .template
                .as_ref()
                .is_some_and(|t| mapping.persons.contains_key(t))
            {
                continue;
            }
            for (selector, identifier) in &facts.identifiers {
                let (issuer, value) = match *selector {
                    "key" => (identifier.issuer.clone(), identifier.canonical()),
                    "user_name" => {
                        names.push(identifier.id.clone());
                        (String::new(), identifier.id.clone())
                    }
                    _ => (identifier.issuer.clone(), identifier.id.clone()),
                };
                out.push(HeldSelector {
                    selector,
                    issuer,
                    value,
                    party_id: head.vo_id,
                });
            }
        }
        let folded = fold(tx, &names).await?;
        for held in out
            .iter_mut()
            .skip(start)
            .filter(|h| h.selector == "user_name")
        {
            if let Some(value) = folded.get(&held.value) {
                held.value.clone_from(value);
            }
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// The fold of each of `values`, as `party.binding_fold` compares a
/// `userName`.
async fn fold(
    tx: &mut PgConnection,
    values: &[String],
) -> Result<BTreeMap<String, String>, StorageError> {
    if values.is_empty() {
        return Ok(BTreeMap::new());
    }
    let rows = sqlx::query(
        "SELECT DISTINCT v, party.binding_fold(v) AS folded FROM unnest($1::text[]) AS t(v)",
    )
    .bind(values)
    .fetch_all(&mut *tx)
    .await?;
    let mut out = BTreeMap::new();
    for row in rows {
        out.insert(row.try_get("v")?, row.try_get("folded")?);
    }
    Ok(out)
}
