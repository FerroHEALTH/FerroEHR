// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The public conversion entry points: COMPOSITION ⇄ the FLAT and
//! STRUCTURED wire forms, and the pure FLAT ⇄ STRUCTURED transforms.
//!
//! The `rm_*` entry points build and read the typed `openehr-rm`
//! COMPOSITION with no canonical-JSON value in between (#6003); the
//! commit and read seams use them. The canonical-JSON entry points are the
//! unchecked projections of the same conversions: the built tree rendered
//! before the RM's mandatory attributes are checked, and a read of a value
//! (the form a `553|incomplete|` commit keeps).
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

use openehr_rm::prelude::Composition;
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
    Ok(typed::render(&*submitted_tree(doc, &tree, wt, now)?))
}

/// FLAT document → typed COMPOSITION for a SUBMITTED body, the typed form of
/// [`submitted_composition_from_flat`] the commit seam takes.
///
/// # Errors
/// As [`submitted_composition_from_flat`].
pub fn submitted_rm_from_flat(
    doc: &Map<String, Value>,
    wt: &WebTemplate,
    now: &str,
) -> Result<Composition, FlatError> {
    let tree = flat::parse_flat(doc)?;
    typed::finish(submitted_tree(doc, &tree, wt, now)?)
}

/// The tree a SUBMITTED body builds, with the master04 §Validation input
/// checks run after the build's own syntax-class refusals and before any
/// refusal of the RM itself (an attribute or value the class refuses, a
/// `|raw` fragment the canonical reader refuses, a mandatory attribute left
/// unset), which are the template-mediated refusals a strict read of the
/// built instance raises last.
///
/// # Errors
/// The build's refusals, [`FlatError::MissingContext`] and
/// [`FlatError::CodeNotInValueSet`].
fn submitted_tree(
    flat_view: &Map<String, Value>,
    tree: &crate::flat::sim::SimDocument,
    wt: &WebTemplate,
    now: &str,
) -> Result<typed::Obj, FlatError> {
    match build::composition_tree(tree, wt, now, &BuildOptions::for_template(wt)) {
        Ok(built) => {
            require_input_valid(flat_view, tree, wt)?;
            Ok(built)
        }
        Err(e @ (FlatError::Rm(_) | FlatError::RawDecode { .. })) => {
            require_input_valid(flat_view, tree, wt)?;
            Err(e)
        }
        Err(e) => Err(e),
    }
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

/// Typed COMPOSITION → FLAT JSON text, read with no canonical-JSON value and
/// written with no intermediate map: the bytes `serde_json::to_string` writes
/// for [`rm_to_flat`].
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
    // The input checks are defined over the FLAT key form (master04's
    // canonical spelling); the pure tree→FLAT projection supplies it.
    let flat_view = flat::emit_flat(&tree);
    Ok(typed::render(&*submitted_tree(&flat_view, &tree, wt, now)?))
}

/// STRUCTURED document → typed COMPOSITION for a SUBMITTED body, the typed
/// form of [`submitted_composition_from_structured`] the commit seam takes.
///
/// # Errors
/// As [`submitted_composition_from_structured`].
pub fn submitted_rm_from_structured(
    doc: &Value,
    wt: &WebTemplate,
    now: &str,
) -> Result<Composition, FlatError> {
    let tree = structured::parse_structured(doc)?;
    // The input checks are defined over the FLAT key form (master04's
    // canonical spelling); the pure tree→FLAT projection supplies it.
    let flat_view = flat::emit_flat(&tree);
    typed::finish(submitted_tree(&flat_view, &tree, wt, now)?)
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

/// FLAT document of a party template → canonical-JSON party (`PERSON`,
/// `ORGANISATION`, `GROUP`, `AGENT` or `ROLE`), with the caller's
/// [`build::BuildOptions`].
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

/// STRUCTURED document of a party template → canonical-JSON party, the
/// STRUCTURED twin of [`party_from_flat`].
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
