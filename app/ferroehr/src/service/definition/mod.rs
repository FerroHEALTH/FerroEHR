// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The DEFINITION component of the platform crate — the openEHR **Definition**
//! service seam (SM `master04-definition_package.adoc`).
//!
//! One file per SM interface, each carrying its domain logic **and** its
//! concrete `FerroEhrService` catalog methods:
//!
//! - `adl14` — `I_DEFINITION_ADL14` (`i_definition_adl14.adoc`): ADL 1.4
//!   source archetypes (keyed by `ARCHETYPE_ID`) + OPTs (keyed by `UUID`).
//! - `adl2`  — `I_DEFINITION_ADL2` (`i_definition_adl2.adoc`): ADL2 artefacts
//!   (keyed by `ARCHETYPE_HRID`).
//! - `lineage` — the archetype specialisation graph over those artefacts (their
//!   `specialize` edges), memoised for the AQL archetype predicate.
//! - `opt14_convert` — the OPT-1.4 → ADL2 decomposition front end (a
//!   service-only capability, no wire): reads a stored OPT's `opt14` model,
//!   decomposes it into one 1.4-shaped `v2_4` source per embedded archetype
//!   root, and runs each through the `openehr_adl::adl14` converter.
//! - `query` — `I_DEFINITION_QUERY` (`i_definition_query.adoc`) +
//!   `QUERY_DESCRIPTOR` + the stored-query CRUD. DEFINITION *owns* query
//!   registration (`master04` §Registered Queries); the Query service only
//!   resolves + executes it, so the stored-query store folds in here.
//! - `wire`  — the ITS-REST wire-shaped extension methods (rich
//!   template/query shapes the SM interfaces do not express). Their route
//!   wiring is the ITS-REST layer's concern; behaviour rides on the SM logic
//!   in the sibling files.
//! - [`types`] — the shared data shapes ([`types::TemplateListFilter`],
//!   [`types::QueryDescriptor`]) the REST adapter consumes.
//!
//! NOTE: the SM `I_DEFINITION_*` signatures take and return AOM object types,
//! but openEHR publishes no BMM meta-model for AOM instances, so the interchange
//! form is the artefact's serialization — ADL 1.4 / ADL2 source text and OPT 1.4
//! canonical XML — which is what these calls carry.
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
mod binding_uri;
pub(super) mod lineage;
mod opt14_convert;
mod query;
mod wire;

pub mod types;

use regex::Regex;

use crate::service::list::Page;
use crate::service::status::{CallStatusType, SmError};

use crate::service::error::ServiceError;

/// Apply an SM [`Page`] to an iterator: skip `offset`, take `limit` (`None` ⇒
/// all — `master02-overview.adoc` §List Handling).
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

/// Compile an id-pattern regex; an uncompilable pattern is `invalid_id_pattern`
/// (`400`).
///
/// NOTE: the SM spells these "PERL regular expression"; the
/// `regex` crate is RE2-class, so PERL backreferences / lookaround are
/// unsupported — a pattern using them fails to compile and surfaces as
/// `invalid_id_pattern`, the correct SM outcome for an unusable pattern (a
/// narrower accept envelope, never a wrong status).
pub(super) fn compile_pattern(pattern: &str) -> Result<Regex, ServiceError> {
    Regex::new(pattern).map_err(|e| {
        let message = format!("invalid id pattern: {e}");
        ServiceError::BadRequest(
            SmError::new(CallStatusType::InvalidIdPattern, message).with_source(e),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::compile_pattern;
    use crate::service::error::ServiceError;
    use crate::service::status::CallStatusType;

    /// An uncompilable id pattern reports the SM status
    /// `i_definition_adl14.adoc` §`list_matching_archetypes` declares
    /// (`.Errors`: `invalid_id_pattern`) and carries the `regex` compile
    /// failure as a walkable cause
    /// ([RFC 0201](https://rust-lang.github.io/rfcs/0201-error-chaining.html)).
    #[test]
    fn an_uncompilable_pattern_is_invalid_id_pattern_and_carries_its_cause() {
        use std::error::Error;

        let err = compile_pattern("(").expect_err("an unbalanced group must not compile");
        let ServiceError::BadRequest(sm) = &err else {
            panic!("an unusable pattern is a 400, got {err:?}");
        };
        assert_eq!(sm.status, CallStatusType::InvalidIdPattern);
        // Two hops: the refusal carries its `SmError`, which carries the
        // concrete compile failure.
        let hops = std::iter::successors(Error::source(&err), |e| Error::source(*e))
            .map(|e| e.downcast_ref::<regex::Error>().is_some())
            .collect::<Vec<_>>();
        assert!(
            hops.contains(&true),
            "walking the chain must reach the concrete regex::Error, got {err:?}"
        );
    }
}
