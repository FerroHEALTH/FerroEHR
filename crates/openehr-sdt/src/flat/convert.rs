// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The public conversion entry points: canonical-JSON COMPOSITION ⇄ the
//! FLAT and STRUCTURED wire forms, and the pure FLAT ⇄ STRUCTURED
//! transforms.
//!
//! Wire semantics: ITS-REST `simplified_formats/master04-basic_concepts.adoc`
//! (§Format variants, §Conversion Between Formats); media types
//! (`master02 §MIME Types`): `application/openehr.wt.flat+json`,
//! `application/openehr.wt.structured+json`. Every conversion runs through
//! the shared simplified tree ([`crate::flat::sim`]), so the two wire variants
//! cannot drift from each other.
//!
//! `now` on the building directions supplies the `ctx/time` default
//! (`master04 §Context`: "defaults to the current server time (now())") —
//! injected by the caller so conversions stay deterministic under test.

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

use serde_json::{Map, Value};

use crate::flat::build;
use crate::flat::error::FlatError;
use crate::flat::flatten;
use crate::flat::sim::flat;
use crate::flat::sim::structured;
use crate::flat::webtemplate::model::WebTemplate;

/// FLAT document → canonical-JSON COMPOSITION.
///
/// # Errors
/// Path-syntax, unknown-identifier, datum, and context errors per
/// [`FlatError`].
pub fn composition_from_flat(
    doc: &Map<String, Value>,
    wt: &WebTemplate,
    now: &str,
) -> Result<Value, FlatError> {
    let tree = flat::parse_flat(doc)?;
    build::build_composition(&tree, wt, now)
}

/// FLAT document → canonical-JSON COMPOSITION for a SUBMITTED body: the
/// plain conversion plus the master04 §Validation input-side checks
/// (mandatory context fields, resolvable coded bindings).
///
/// The commit seams use this entry; [`composition_from_flat`] stays the
/// unchecked projection (round-trips, fragments, response rendering).
///
/// # Errors
/// Everything [`composition_from_flat`] refuses, plus
/// [`FlatError::MissingContext`] and [`FlatError::CodeNotInValueSet`].
pub fn submitted_composition_from_flat(
    doc: &Map<String, Value>,
    wt: &WebTemplate,
    now: &str,
) -> Result<Value, FlatError> {
    let tree = flat::parse_flat(doc)?;
    let built = build::build_composition(&tree, wt, now)?;
    require_input_valid(doc, &tree, wt)?;
    Ok(built)
}

/// The master04 §Validation input-side checks a SUBMITTED simplified body
/// runs: mandatory context fields and resolvable coded bindings — refusing
/// here names the actual defect instead of letting an RM-invalid build fail
/// downstream at the strict canonical reader. Runs AFTER the build so the
/// build's own syntax-class refusals (`|other` conflicts, malformed datums)
/// keep precedence over these semantic ones.
fn require_input_valid(
    doc: &Map<String, Value>,
    tree: &crate::flat::sim::SimDocument,
    wt: &WebTemplate,
) -> Result<(), FlatError> {
    if let Some(m) = crate::flat::validation::validate_context(tree).first() {
        let field = if m.path.ends_with("territory") {
            "territory"
        } else {
            "language"
        };
        return Err(FlatError::MissingContext(field));
    }
    if let Some((path, code)) = crate::flat::validation::unresolvable_coded_leaves(doc, wt)
        .into_iter()
        .next()
    {
        return Err(FlatError::CodeNotInValueSet { path, code });
    }
    Ok(())
}

/// Canonical-JSON COMPOSITION → FLAT document.
///
/// # Errors
/// [`FlatError::Conversion`] when `composition` is not a JSON object.
pub fn composition_to_flat(
    composition: &Value,
    wt: &WebTemplate,
) -> Result<Map<String, Value>, FlatError> {
    let tree = flatten::flatten_composition(composition, wt)?;
    Ok(flat::emit_flat(&tree))
}

/// STRUCTURED document → canonical-JSON COMPOSITION.
///
/// # Errors
/// Structure, unknown-identifier, datum, and context errors per
/// [`FlatError`].
pub fn composition_from_structured(
    doc: &Value,
    wt: &WebTemplate,
    now: &str,
) -> Result<Value, FlatError> {
    let tree = structured::parse_structured(doc)?;
    build::build_composition(&tree, wt, now)
}

/// STRUCTURED document → canonical-JSON COMPOSITION for a SUBMITTED body —
/// the STRUCTURED twin of [`submitted_composition_from_flat`].
///
/// # Errors
/// Everything [`composition_from_structured`] refuses, plus
/// [`FlatError::MissingContext`] and [`FlatError::CodeNotInValueSet`].
pub fn submitted_composition_from_structured(
    doc: &Value,
    wt: &WebTemplate,
    now: &str,
) -> Result<Value, FlatError> {
    let tree = structured::parse_structured(doc)?;
    let built = build::build_composition(&tree, wt, now)?;
    // The input checks are defined over the FLAT key form (master04's
    // canonical spelling); the pure tree→FLAT projection supplies it.
    let flat_view = flat::emit_flat(&tree);
    require_input_valid(&flat_view, &tree, wt)?;
    Ok(built)
}

/// Canonical-JSON COMPOSITION → STRUCTURED document.
///
/// # Errors
/// [`FlatError::Conversion`] when `composition` is not a JSON object.
pub fn composition_to_structured(
    composition: &Value,
    wt: &WebTemplate,
) -> Result<Value, FlatError> {
    let tree = flatten::flatten_composition(composition, wt)?;
    Ok(structured::emit_structured(&tree))
}

/// FLAT → STRUCTURED, the pure template-free transform
/// (`master04 §Flat to Structured`).
///
/// # Errors
/// [`FlatError::MalformedPath`] on an invalid key.
pub fn flat_to_structured(doc: &Map<String, Value>) -> Result<Value, FlatError> {
    Ok(structured::emit_structured(&flat::parse_flat(doc)?))
}

/// STRUCTURED → FLAT, the pure template-free transform
/// (`master04 §Structured to Flat`).
///
/// # Errors
/// [`FlatError::Conversion`]/[`FlatError::MalformedPath`] on an invalid
/// document.
pub fn structured_to_flat(doc: &Value) -> Result<Map<String, Value>, FlatError> {
    Ok(flat::emit_flat(&structured::parse_structured(doc)?))
}
