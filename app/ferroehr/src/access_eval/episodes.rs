// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Whether a read selects the episodes of care each object is placed in.
//!
//! An `ACCESS_DATA_SCOPE` may name episodes (draft access control, Episodes
//! of Care). A read selects the head row's episodes beside the other facts
//! the evaluator judges an object by only once this replica has read
//! settings or a policy that name one, so a deployment that names none runs
//! the statements it ran before. A copy or class read later that names one
//! turns the selection on for every later read; an object read without its
//! episodes is judged by the fact's polarity ([`crate::access_eval::scope`]).
//! No openEHR spec governs this: our own design (#6416).

use std::sync::atomic::{AtomicBool, Ordering};

use openehr_rm::draft::access_control::access_control::access_data_scope::AccessDataScope;
use openehr_rm::draft::access_control::access_control::access_directive::AccessDirective;
use openehr_rm::draft::access_control::access_control::access_directive_settings::AccessDirectiveSettings;
use openehr_rm::draft::access_control::access_control::access_policy::AccessPolicy;

/// Whether this replica has read settings or a policy naming an episode.
static NAMED: AtomicBool = AtomicBool::new(false);

/// Whether a read selects the episodes of the objects it reads.
#[must_use]
pub fn selected() -> bool {
    NAMED.load(Ordering::Acquire)
}

/// Notes `settings`, which this replica is about to judge by.
pub fn note_settings(settings: &AccessDirectiveSettings) {
    let directives = settings.directives.as_deref().unwrap_or_default();
    let exclusions = settings.exclusions.as_deref().unwrap_or_default();
    if directives.iter().chain(exclusions).any(directive_names) {
        NAMED.store(true, Ordering::Release);
    }
}

/// Notes `policy`, which this replica is about to judge by.
pub fn note_policy(policy: &AccessPolicy) {
    let classified = policy
        .classifications
        .as_deref()
        .unwrap_or_default()
        .iter()
        .any(|c| names(&c.data));
    let ruled = policy
        .rules
        .as_deref()
        .unwrap_or_default()
        .iter()
        .chain(policy.catalogue.as_deref().unwrap_or_default())
        .any(directive_names);
    if classified || ruled {
        NAMED.store(true, Ordering::Release);
    }
}

/// Whether `directive` or one of its exceptions names an episode.
fn directive_names(directive: &AccessDirective) -> bool {
    directive
        .data
        .as_deref()
        .unwrap_or_default()
        .iter()
        .any(names)
        || directive
            .exceptions
            .as_deref()
            .unwrap_or_default()
            .iter()
            .any(directive_names)
}

/// Whether `scope` names an episode.
fn names(scope: &AccessDataScope) -> bool {
    scope.episodes.is_some()
}
