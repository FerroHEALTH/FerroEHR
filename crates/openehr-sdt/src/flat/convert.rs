// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! COMPOSITION ⇄ FLAT and STRUCTURED (ITS-REST
//! `simplified_formats/master04-basic_concepts.adoc`), and FLAT ⇄ STRUCTURED,
//! all through the simplified tree ([`crate::flat::sim`]). The `rm_*` entry
//! points work on the typed COMPOSITION (#6003); the canonical-JSON ones are
//! unchecked projections. `now` is the `ctx/time` default (`master04
//! §Context`).

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

use openehr_rm::prelude::Composition;
use openehr_rm::v1_2::access;
use serde_json::{Map, Value};

use crate::flat::build::{self, BuildOptions};
use crate::flat::error::FlatError;
use crate::flat::flatten;
use crate::flat::sim::flat;
use crate::flat::sim::structured;
use crate::flat::typed;
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
    let built = build::composition_tree(&tree, wt, now, &BuildOptions::for_template(wt))?;
    Ok(typed::render(&*built))
}

/// FLAT document → typed COMPOSITION.
///
/// # Errors
/// As [`composition_from_flat`].
pub fn rm_from_flat(
    doc: &Map<String, Value>,
    wt: &WebTemplate,
    now: &str,
) -> Result<Composition, FlatError> {
    let tree = flat::parse_flat(doc)?;
    build::build_composition(&tree, wt, now)
}

/// FLAT document → canonical-JSON COMPOSITION for a submitted body, with the
/// master04 §Validation input checks.
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
    Ok(typed::render(&*submitted_tree(&tree, wt, now)?))
}

/// FLAT document → typed COMPOSITION for a submitted body.
///
/// # Errors
/// As [`submitted_composition_from_flat`].
pub fn submitted_rm_from_flat(
    doc: &Map<String, Value>,
    wt: &WebTemplate,
    now: &str,
) -> Result<Composition, FlatError> {
    submitted_rm_from_tree(&flat::parse_flat(doc)?, wt, now)
}

/// A parsed FLAT or STRUCTURED document → typed COMPOSITION for a submitted
/// body, over a tree read from the wire text ([`flat::parse_flat_text`]).
///
/// # Errors
/// As [`submitted_composition_from_flat`].
pub fn submitted_rm_from_tree(
    tree: &crate::flat::sim::SimDocument,
    wt: &WebTemplate,
    now: &str,
) -> Result<Composition, FlatError> {
    Ok(access::finish(submitted_tree(tree, wt, now)?)?)
}

/// The tree a submitted body builds, the master04 §Validation input checks
/// run after the build's syntax refusals and before the RM's own.
///
/// # Errors
/// The build's refusals, [`FlatError::MissingContext`] and
/// [`FlatError::CodeNotInValueSet`].
fn submitted_tree(
    tree: &crate::flat::sim::SimDocument,
    wt: &WebTemplate,
    now: &str,
) -> Result<typed::Obj, FlatError> {
    match build::composition_tree(tree, wt, now, &BuildOptions::for_template(wt)) {
        Ok(built) => {
            require_input_valid(tree, wt)?;
            Ok(built)
        }
        Err(e @ (FlatError::Rm(_) | FlatError::RawDecode { .. })) => {
            require_input_valid(tree, wt)?;
            Err(e)
        }
        Err(e) => Err(e),
    }
}

/// The master04 §Validation input checks of a submitted body: mandatory
/// context fields and resolvable coded bindings.
fn require_input_valid(
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
    if let Some((path, code)) = crate::flat::validation::unresolvable_coded_leaves(tree, wt)
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

/// Typed COMPOSITION → FLAT JSON text, the bytes `serde_json::to_string`
/// writes for [`rm_to_flat`].
///
/// # Errors
/// [`FlatError::Serialize`], which a FLAT datum never raises.
pub fn rm_to_flat_text(composition: &Composition, wt: &WebTemplate) -> Result<String, FlatError> {
    Ok(flat::write_flat(&flatten::flatten_typed(composition, wt))?)
}

/// Typed COMPOSITION → FLAT document, read with no canonical-JSON value.
#[must_use]
pub fn rm_to_flat(composition: &Composition, wt: &WebTemplate) -> Map<String, Value> {
    flat::emit_flat(&flatten::flatten_typed(composition, wt))
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
    let built = build::composition_tree(&tree, wt, now, &BuildOptions::for_template(wt))?;
    Ok(typed::render(&*built))
}

/// STRUCTURED document → typed COMPOSITION.
///
/// # Errors
/// As [`composition_from_structured`].
pub fn rm_from_structured(
    doc: &Value,
    wt: &WebTemplate,
    now: &str,
) -> Result<Composition, FlatError> {
    let tree = structured::parse_structured(doc)?;
    build::build_composition(&tree, wt, now)
}

/// STRUCTURED document → canonical-JSON COMPOSITION for a submitted body.
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
    Ok(typed::render(&*submitted_tree(&tree, wt, now)?))
}

/// STRUCTURED document → typed COMPOSITION for a submitted body.
///
/// # Errors
/// As [`submitted_composition_from_structured`].
pub fn submitted_rm_from_structured(
    doc: &Value,
    wt: &WebTemplate,
    now: &str,
) -> Result<Composition, FlatError> {
    submitted_rm_from_tree(&structured::parse_structured(doc)?, wt, now)
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

/// Typed COMPOSITION → STRUCTURED document, read with no canonical-JSON
/// value.
#[must_use]
pub fn rm_to_structured(composition: &Composition, wt: &WebTemplate) -> Value {
    structured::emit_structured(&flatten::flatten_typed(composition, wt))
}

/// FLAT document of a party template → canonical-JSON party.
///
/// # Errors
/// Path-syntax errors, plus everything [`build::build_party`] refuses.
pub fn party_from_flat(
    doc: &Map<String, Value>,
    wt: &WebTemplate,
    options: &BuildOptions<'_>,
) -> Result<Value, FlatError> {
    let tree = flat::parse_flat(doc)?;
    build::build_party(&tree, wt, options)
}

/// STRUCTURED document of a party template → canonical-JSON party.
///
/// # Errors
/// Structure errors, plus everything [`build::build_party`] refuses.
pub fn party_from_structured(
    doc: &Value,
    wt: &WebTemplate,
    options: &BuildOptions<'_>,
) -> Result<Value, FlatError> {
    let tree = structured::parse_structured(doc)?;
    build::build_party(&tree, wt, options)
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
