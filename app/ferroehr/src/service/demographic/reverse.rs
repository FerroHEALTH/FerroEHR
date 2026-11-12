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
use crate::error::refusal::Missing;
use crate::ids::VoId;
use crate::service::FerroEhrService;
use crate::service::demographic::types::PartyKind;
use crate::storage::party_reverse::ReverseRelationship;
use crate::versioning::Kind;
use crate::versioning::read::demographic_current;

/// One party's `reverse_relationships`: the references, and the source
/// parties whose current versions the read disclosed, for the access record.
#[derive(Debug, Clone, Default)]
pub struct ReverseRelationships {
    /// One canonical `LOCATABLE_REF` per relationship, in index order.
    pub references: Vec<Value>,
    /// The distinct source party container ids, in index order.
    pub sources: Vec<String>,
}

/// The path of one inline relationship in its source party's data: the
/// `relationships` attribute narrowed by the relationship's archetype node id
/// and, when it can be quoted, its name.
///
/// No specification fixes which path a `reverse_relationships` reference
/// carries; this is our own design, in the archetype-path predicate form of
/// BASE `architecture_overview/master11-paths.adoc` §Paths.
fn relationship_path(row: &ReverseRelationship) -> String {
    match (row.node_id.as_deref(), row.name.as_deref()) {
        (Some(node), Some(name)) if !name.contains('\'') => {
            format!("/relationships[{node} and name/value='{name}']")
        }
        (Some(node), _) => format!("/relationships[{node}]"),
        (None, _) => "/relationships".to_owned(),
    }
}

impl FerroEhrService {
    /// The `reverse_relationships` of a party in the RM 1.1.0 model: one
    /// `LOCATABLE_REF` per `PARTY_RELATIONSHIP` whose target is the party's
    /// version container, as the relationships stand now when `at` is `None`,
    /// else as they stood at the instant `at`.
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
    /// `master00-amendment_record.adoc`), whose result is the same list.
    ///
    /// A read of a party version, by its id or at a time, passes that
    /// version's commit instant, so every representation of one version
    /// carries one list; no openEHR spec says how a derived list behaves on an
    /// old version, so that is our own design (#5473).
    ///
    /// # Errors
    /// - [`ServiceError`] on a storage/database fault during the read.
    pub async fn party_reverse_relationships(
        &self,
        vo_id: VoId,
        at: Option<jiff::Timestamp>,
    ) -> Result<ReverseRelationships, ServiceError> {
        // NOTE: RM 1.1.0 `PARTY_RELATIONSHIP.Target_valid` negates this list (#1576,
        // upstream-confirmed; Veredictum report 1053); the package text and
        // `PARTY.Reverse_relationships_validity` both require it, and they decide.
        let mut conn = crate::db::acquire::acquire(&self.demographic_pool).await?;
        let rows = crate::storage::party_reverse::reverse_of(&mut conn, vo_id, at).await?;
        let mut out = ReverseRelationships::default();
        for row in &rows {
            out.references
                .push(openehr_its::json::to_canonical_value(&LocatableRef {
                    namespace: "local".to_owned(),
                    r#type: "PARTY_RELATIONSHIP".to_owned(),
                    id: UidBasedId::HierObjectId(HierObjectId::from(row.source_vo_id.0)),
                    // A standalone container is the relationship itself, so
                    // the reference needs no path into it.
                    path: (!row.standalone).then(|| relationship_path(row)),
                }));
            let source = row.source_vo_id.to_string();
            if !out.sources.contains(&source) {
                out.sources.push(source);
            }
        }
        Ok(out)
    }

    /// The kind of the party whose version container is `vo_id`: what an
    /// address that names no kind, such as the `reverse_relationships()`
    /// extension route's, reads a party through.
    ///
    /// # Errors
    /// - [`ServiceError`] mapped from `404` — `vo_id` names no party (an
    ///   unknown id, a `PARTY_RELATIONSHIP` container, or a clinical object).
    /// - [`ServiceError`] on a storage/database fault during the read.
    pub async fn party_kind(&self, vo_id: VoId) -> Result<PartyKind, ServiceError> {
        let kind = demographic_current(&self.demographic_pool, vo_id)
            .await?
            .map(|current| current.kind);
        match kind {
            Some(Kind::Agent) => Ok(PartyKind::Agent),
            Some(Kind::Group) => Ok(PartyKind::Group),
            Some(Kind::Organisation) => Ok(PartyKind::Organisation),
            Some(Kind::Person) => Ok(PartyKind::Person),
            Some(Kind::Role) => Ok(PartyKind::Role),
            _ => Err(ServiceError::not_found(
                Missing::VersionedObject,
                format!("PARTY {vo_id}"),
            )),
        }
    }
}
