// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `PARTY.reverse_relationships`, the references a target party carries.
//!
//! They point at the `PARTY_RELATIONSHIP`s other parties hold inline, read
//! through the `party_relationship_target` index
//! ([`crate::storage::party_reverse`]).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): stored-content serving; the references join \
              the canonical party body the read serves"
)]

use openehr_base::prelude::{HierObjectId, LocatableRef, UidBasedId};
use serde_json::Value;

use crate::error::ServiceError;
use crate::ids::VoId;
use crate::service::FerroEhrService;
use crate::storage::party_reverse::ReverseRelationship;

/// One party's `reverse_relationships`: the references, and the source
/// parties whose current versions the read disclosed, for the access record.
#[derive(Debug, Clone, Default)]
pub struct ReverseRelationships {
    /// One canonical `LOCATABLE_REF` per relationship, in index order.
    pub references: Vec<Value>,
    /// The distinct source party container ids, in index order.
    pub sources: Vec<String>,
}

/// The path of one inline relationship in its source party's data, which
/// names that one relationship and no sibling.
///
/// BASE `architecture_overview/master11-paths.adoc` §Data Paths and
/// Uniqueness: a `uid` predicate is "the preferred method to achieve runtime
/// unique node identification", a `name/value` predicate is unique where the
/// names are, and the positional predicate is unique where the order of a
/// container is preserved, which the stored body preserves. The path takes
/// the first of the three that names one relationship: the `uid` when no
/// sibling shares it, else the archetype node id and the name when no
/// sibling shares both, else the position. No specification fixes which path
/// a `reverse_relationships` reference carries: our own design.
fn relationship_path(row: &ReverseRelationship) -> String {
    let quotable = |text: &&str| !text.contains('\'');
    let uid = row.uid.as_deref().filter(quotable);
    let name = row.name.as_deref().filter(quotable);
    match (uid, name, row.node_id.as_deref()) {
        (Some(uid), _, Some(node)) if row.uid_unique => {
            format!("/relationships[{node} and uid='{uid}']")
        }
        (Some(uid), _, None) if row.uid_unique => format!("/relationships[uid='{uid}']"),
        (_, Some(name), Some(node)) if row.name_unique => {
            format!("/relationships[{node} and name/value='{name}']")
        }
        _ => format!("/relationships[{}]", row.position),
    }
}

impl FerroEhrService {
    /// The `reverse_relationships` of a party in the RM 1.1.0 model: one
    /// `LOCATABLE_REF` per `PARTY_RELATIONSHIP` whose target is the party's
    /// version container, read from the current trunk version of each source
    /// party that holds one.
    ///
    /// RM 1.1.0 demographic §Party Relationships, which is not vendored
    /// (<https://specifications.openehr.org/releases/RM/Release-1.1.0/demographic.html#_party_relationships>):
    /// "each Party involved in a relationship includes it in its relationships
    /// list, if it is at the source end, or in the `reverse_relationships`
    /// list, if at the target end", and `reverse_relationships` "is by
    /// references". The reference names the source party's version container
    /// by `HIER_OBJECT_ID` (the same section, and the vendored RM 1.2.0
    /// `master02-demographic_package.adoc` §Party Relationships: relationships
    /// are "between _continuants_") and the relationship by its path in that
    /// party's data. RM 1.2.0 turns the attribute into a function (SPECRM-124,
    /// `master00-amendment_record.adoc`), so only a stable-profile read serves it.
    ///
    /// # Errors
    /// - [`ServiceError`] on a storage/database fault during the read.
    pub async fn party_reverse_relationships(
        &self,
        vo_id: VoId,
    ) -> Result<ReverseRelationships, ServiceError> {
        // NOTE: RM 1.1.0 `PARTY_RELATIONSHIP.Target_valid` negates this list (#1576,
        // upstream-confirmed; Veredictum report 1053); the package text and
        // `PARTY.Reverse_relationships_validity` both require it, and they decide.
        let mut conn = crate::db::acquire::acquire(&self.demographic_pool).await?;
        let rows = crate::storage::party_reverse::reverse_of(&mut conn, vo_id).await?;
        let mut out = ReverseRelationships::default();
        for row in &rows {
            out.references
                .push(openehr_its::json::to_canonical_value(&LocatableRef {
                    namespace: "local".to_owned(),
                    r#type: "PARTY_RELATIONSHIP".to_owned(),
                    id: UidBasedId::HierObjectId(HierObjectId::from(row.source_vo_id.0)),
                    path: Some(relationship_path(row)),
                }));
            let source = row.source_vo_id.to_string();
            if !out.sources.contains(&source) {
                out.sources.push(source);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(uid: Option<&str>, name: Option<&str>, unique: (bool, bool)) -> ReverseRelationship {
        ReverseRelationship {
            source_vo_id: VoId(uuid::Uuid::nil()),
            node_id: Some("at0001".to_owned()),
            name: name.map(str::to_owned),
            uid: uid.map(str::to_owned),
            position: 3,
            name_unique: unique.0,
            uid_unique: unique.1,
        }
    }

    /// The path takes a unique `uid`, else a unique name, else the position
    /// (BASE `master11-paths.adoc` §Data Paths and Uniqueness).
    #[test]
    fn the_path_takes_the_first_unique_predicate() {
        for (row, path) in [
            (
                row(Some("u-1"), Some("a"), (true, true)),
                "/relationships[at0001 and uid='u-1']",
            ),
            (
                row(Some("u-1"), Some("a"), (true, false)),
                "/relationships[at0001 and name/value='a']",
            ),
            (
                row(None, Some("a"), (true, false)),
                "/relationships[at0001 and name/value='a']",
            ),
            (row(None, Some("a"), (false, false)), "/relationships[3]"),
            (
                row(None, Some("o'clock"), (true, false)),
                "/relationships[3]",
            ),
            (row(Some("u'1"), None, (false, true)), "/relationships[3]"),
        ] {
            assert_eq!(relationship_path(&row), path, "{row:?}");
        }
    }
}
