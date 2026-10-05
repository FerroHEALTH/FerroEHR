// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Phase-1 overlay topic: the `annotations` and `rm_overlay` sections.
//!
//! Rule texts:
//! `docs/specs/openehr/AM/docs/AOM2/master03-archetype_package.adoc` §Validity
//! Rules, VRANP (annotation path validity) and
//! `master06-rm_overlay.adoc` §Validity, VRMVP / VRMVAV (RM-visibility path +
//! alias validity).

use std::collections::BTreeSet;

use super::ValidationIssue;
use super::catalogue::ValidationCode;
use crate::artefact::ArchetypeView;
use crate::paths::{Resolution, has_node_id_predicate, resolve};

/// VRANP: each annotation path must be a valid archetype path or an RM path
/// valid for the root class (master03 §Validity Rules).
///
/// NOTE: only paths carrying a node-id predicate are resolved against the
/// archetype here; a pure reference-model path (no `[id…]` predicate) is a
/// reference-model question (`super::rm`).
pub(super) fn check_annotations(v: &ArchetypeView<'_>, issues: &mut Vec<ValidationIssue>) {
    let Some(annotations) = v.annotations else {
        return;
    };
    for paths in annotations.documentation.values() {
        for path in paths.keys() {
            if has_node_id_predicate(path) && resolve(v.definition, path) != Resolution::Found {
                issues.push(
                    ValidationIssue::new(
                        ValidationCode::Vranp,
                        format!("annotation path {path:?} is not valid in the archetype"),
                    )
                    .at_path(path.clone()),
                );
            }
        }
    }
}

/// VRMVP / VRMVAV: `rm_overlay` visibility path + alias validity (master06
/// §Validity). The path's node-id-predicated part must resolve; the alias must
/// be a defined at-code.
///
/// NOTE: the pure-RM tail of a visibility path is a reference-model concern
/// (`super::rm`).
pub(super) fn check_rm_overlay(v: &ArchetypeView<'_>, issues: &mut Vec<ValidationIssue>) {
    let Some(overlay) = v.rm_overlay else {
        return;
    };
    let Some(map) = overlay.rm_visibility.as_ref() else {
        return;
    };
    let defined: BTreeSet<&str> = v
        .terminology
        .term_definitions
        .values()
        .flat_map(|m| m.keys().map(String::as_str))
        .collect();
    for (path, vis) in map {
        if has_node_id_predicate(path) && resolve(v.definition, path) == Resolution::NotFound {
            issues.push(
                ValidationIssue::new(
                    ValidationCode::Vrmvp,
                    format!("rm_visibility path {path:?} is not valid in the archetype"),
                )
                .at_path(path.clone()),
            );
        }
        if let Some(alias) = vis.alias.as_ref() {
            let code = &alias.code_string;
            if !defined.contains(code.as_str()) {
                issues.push(ValidationIssue::new(
                    ValidationCode::Vrmvav,
                    format!("rm_visibility alias {code:?} is not a defined at-code"),
                ));
            }
        }
    }
}
