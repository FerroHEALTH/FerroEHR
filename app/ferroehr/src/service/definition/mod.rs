// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The DEFINITION component of the platform crate, behind the ITS-REST
//! Definition API.
//!
//! Our own design.
//!
//! One file per interface, each carrying its domain logic **and** its
//! concrete `FerroEhrService` catalog methods:
//!
//! - `adl14` — ADL 1.4 source archetypes (keyed by `ARCHETYPE_ID`) + OPTs
//!   (keyed by `UUID`).
//! - `adl2`  — ADL2 artefacts
//!   (keyed by `ARCHETYPE_HRID`).
//! - `lineage` — the archetype specialisation graph over those artefacts (their
//!   `specialize` edges), memoised for the AQL archetype predicate.
//! - `query` — query registration: the stored-query CRUD. DEFINITION *owns*
//!   query registration (`master04` §Registered Queries); the Query service
//!   only resolves + executes it, so the stored-query store folds in here.
//! - `wire`  — the ITS-REST wire-shaped extension methods (rich
//!   template/query shapes). Their route
//!   wiring is the ITS-REST layer's concern; behaviour rides on the logic
//!   in the sibling files.
//! - `version_selector` — the ITS-REST `version` path parameter: its SEMVER
//!   grammar, checked before any versioned read resolves, and its matching.
//! - [`types`] — the shared data shapes ([`types::TemplateListFilter`],
//!   [`types::Adl2Template`]) the REST adapter consumes.
//!
//! NOTE: ITS-REST Definition API exchanges the artefact's serialization (ADL
//! 1.4 / ADL2 source text and OPT 1.4 canonical XML), which is what these calls
//! carry.
//!
//! Identity is matched case-insensitively while storage is case-preserving (BASE
//! `master05-identification_package.adoc` §Composite Identifiers and Case: "two
//! identifiers identical apart from case are considered to be identical"): every
//! lookup, existence check and delete compares `lower(id) = lower($1)` and every
//! write first removes any case-variant of the same id in the same transaction.
//! This module is that single canonicalisation boundary for definition
//! artefacts.

mod adl14;
mod adl2;
mod adl2_dependents;
mod adl2_engine;
mod adl2_resolution;
mod adl2_terminology;
mod binding_uri;
pub(super) mod lineage;
mod query;
mod version_selector;
mod wire;

pub mod types;

use crate::service::list::Page;

/// Apply a [`Page`] to an iterator: skip `offset`, take `limit` (`None` ⇒
/// all).
pub(super) fn paginate<T>(items: impl Iterator<Item = T>, page: Page) -> Vec<T> {
    let offset = usize::try_from(page.offset()).unwrap_or(usize::MAX);
    let skipped = items.skip(offset);
    match page.limit() {
        Some(n) => skipped
            .take(usize::try_from(n).unwrap_or(usize::MAX))
            .collect(),
        None => skipped.collect(),
    }
}

/// A [`Page`] as `(offset, limit)` SQL bind values; a `None` limit binds SQL
/// `NULL` (`LIMIT NULL` = all rows in `PostgreSQL`).
pub(super) fn page_bounds(page: Page) -> (i64, Option<i64>) {
    let offset = i64::try_from(page.offset()).unwrap_or(i64::MAX);
    let limit = page.limit().and_then(|l| i64::try_from(l).ok());
    (offset, limit)
}
