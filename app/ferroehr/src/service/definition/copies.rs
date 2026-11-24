// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What a replica builds from the definition stores and keeps, held at the
//! definition revision it was built at (#5948).
//!
//! The Web Templates of the stored templates, the template ids no OPT 1.4
//! template is stored under, and the archetype lineage index live in one
//! generation, tagged with a `definition_revision` a request read.
//! Every request reads the newest revision in the one read of the policy
//! heads it already makes, and a newer revision replaces the generation with
//! an empty one before the request answers, so no answer rests on a
//! notification's delay or a time-to-live. A generation is installed only
//! after a read returned its revision, so whatever is built into it was read
//! from the store at that revision or later. No openEHR spec governs the
//! caches: our own design.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::{Arc, PoisonError, RwLock};

use openehr_sdt::flat::cache::WebTemplateCache;

use crate::service::definition::lineage::{ArchetypeLineageCache, archetype_lineage_cache};

/// The definition copies of one revision.
#[derive(Debug)]
pub(crate) struct Generation {
    /// The `definition_revision` the copies were built at or after.
    revision: i64,
    /// The Web Templates built from the stored templates.
    pub(crate) web_templates: WebTemplateCache,
    /// The archetype specialisation lineage AQL widens a predicate with.
    pub(crate) lineage: ArchetypeLineageCache,
    /// The canonical template ids the OPT 1.4 store held no template under
    /// at a read made at this revision or later, at most [`ABSENT`] (#6335).
    absent: RwLock<BTreeSet<String>>,
}

/// The most template ids a generation remembers as absent; past it, an
/// absent id is read again.
const ABSENT: usize = 1024;

impl Generation {
    /// An empty generation at `revision`.
    fn at(revision: i64) -> Self {
        Self {
            revision,
            web_templates: WebTemplateCache::default(),
            lineage: archetype_lineage_cache(),
            absent: RwLock::new(BTreeSet::new()),
        }
    }

    /// Whether a read at this revision found no OPT 1.4 template under the
    /// canonical id `key`.
    pub(crate) fn opt14_absent(&self, key: &str) -> bool {
        self.absent
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(key)
    }

    /// Remember that a read found no OPT 1.4 template under the canonical id
    /// `key`, while fewer than [`ABSENT`] ids are remembered.
    pub(crate) fn note_opt14_absent(&self, key: &str) {
        let mut absent = self.absent.write().unwrap_or_else(PoisonError::into_inner);
        if absent.len() < ABSENT {
            absent.insert(key.to_owned());
        }
    }
}

/// The generation of definition copies this replica answers from.
pub(crate) struct DefinitionCopies {
    current: RwLock<Arc<Generation>>,
}

impl fmt::Debug for DefinitionCopies {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DefinitionCopies")
            .field("revision", &self.current().revision)
            .finish_non_exhaustive()
    }
}

impl Default for DefinitionCopies {
    fn default() -> Self {
        // NOTE: our own design (#5948), the store's revision starts at 0 and only grows,
        // so copies built before the first read are dropped by any change.
        Self {
            current: RwLock::new(Arc::new(Generation::at(0))),
        }
    }
}

impl DefinitionCopies {
    /// The generation to look up and build into. A caller takes it once and
    /// uses it for the lookup and the build, so a build that read the store
    /// before a newer revision was installed lands in the replaced generation.
    pub(crate) fn current(&self) -> Arc<Generation> {
        Arc::clone(&self.current.read().unwrap_or_else(PoisonError::into_inner))
    }

    /// The current generation when it is the one of `revision`, `None` once a
    /// newer revision replaced it: a copy built from what the store held at
    /// `revision` is kept only in that revision's generation.
    pub(crate) fn at(&self, revision: i64) -> Option<Arc<Generation>> {
        let current = self.current();
        (current.revision == revision).then_some(current)
    }

    /// Whether the copies were built at or after `revision`, a
    /// `definition_revision` the request read (#6251).
    pub(crate) fn holds(&self, revision: i64) -> bool {
        revision <= self.current().revision
    }

    /// Bring the copies to `revision`, the newest `definition_revision` read
    /// from the store: a generation older than it is replaced by an empty
    /// one. The revision only grows, so a read made before another caller's
    /// never puts an older generation back.
    pub(crate) fn confirm(&self, revision: i64) {
        if revision <= self.current().revision {
            return;
        }
        let mut current = self.current.write().unwrap_or_else(PoisonError::into_inner);
        if revision > current.revision {
            *current = Arc::new(Generation::at(revision));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::DefinitionCopies;

    /// A newer revision replaces the generation, and an older or equal one
    /// keeps it (#5948).
    #[test]
    fn only_a_newer_revision_replaces_the_generation() {
        let copies = DefinitionCopies::default();
        let first = copies.current();
        copies.confirm(0);
        assert!(Arc::ptr_eq(&first, &copies.current()), "the same revision");
        copies.confirm(3);
        let third = copies.current();
        assert!(!Arc::ptr_eq(&first, &third), "a newer revision");
        copies.confirm(2);
        assert!(Arc::ptr_eq(&third, &copies.current()), "an older revision");
        assert!(copies.at(3).is_some(), "the current revision");
        assert!(copies.at(2).is_none(), "a replaced revision");
    }

    /// An absence is held by its generation only, and a newer revision
    /// forgets it (#6335).
    #[test]
    fn a_newer_revision_forgets_an_absence() {
        let copies = DefinitionCopies::default();
        copies.current().note_opt14_absent("care.v0");
        assert!(copies.current().opt14_absent("care.v0"), "held");
        assert!(
            !copies.current().opt14_absent("other.v0"),
            "only the id noted"
        );
        copies.confirm(1);
        assert!(!copies.current().opt14_absent("care.v0"), "forgotten");
    }

    /// A generation remembers at most [`super::ABSENT`] ids.
    #[test]
    fn the_absences_are_bounded() {
        let copies = DefinitionCopies::default();
        let generation = copies.current();
        for n in 0..=super::ABSENT {
            generation.note_opt14_absent(&format!("t{n}"));
        }
        assert!(generation.opt14_absent("t0"));
        assert!(!generation.opt14_absent(&format!("t{}", super::ABSENT)));
    }
}
