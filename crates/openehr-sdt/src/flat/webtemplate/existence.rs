// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The AOM 1.4 `C_ATTRIBUTE.existence` constraints of an OPT 1.4 node
//! (`master04-constraint_model_package.adoc` §existence): the mandatory
//! attributes the validation walk enforces, and the prohibited attributes the
//! in-context synthesis leaves out.

use openehr_its::opt14::types::{CAttribute, CObject};

use super::builder::{object_occurrences, occurrences};
use super::constraint::WebTemplateExistence;
use super::implicit::attribute_existence;
use super::inputs;

/// Capture the AOM 1.4 `C_ATTRIBUTE.existence` constraints of the mandatory
/// single-valued RM attributes of `co`, keyed by absolute archetype path.
///
/// Scope: existence lower `>= 1` with an object-valued constraint child; `name`
/// is excluded (master04 §"Field Identifiers"). A node-identified structural
/// child counts too, since compaction may drop it and existence is then the only
/// check.
///
/// NOTE: AOM 1.4 §existence defaults to `{1..1}`; the declared value is honoured.
pub(super) fn existence_constraints(co: &CObject, node_path: &str) -> Vec<WebTemplateExistence> {
    let mut out = Vec::new();
    for attr in inputs::attributes(co) {
        // A container with existence lower >= 1 must be present (AOM 1.4
        // `Existence_set`); cardinality governs its members.
        if let CAttribute::CMultipleAttribute(m) = attr {
            let (min, max) = occurrences(&m.existence);
            if min.unwrap_or(0) >= 1 {
                out.push(WebTemplateExistence {
                    min: min.unwrap_or(0),
                    max,
                    path: format!("{node_path}/{}", m.rm_attribute_name),
                });
            }
            continue;
        }
        let CAttribute::CSingleAttribute(s) = attr else {
            continue;
        };
        let (min, max) = occurrences(&s.existence);
        let min = min.unwrap_or(0);
        // An object-valued child excludes function and primitive constraints; a
        // childless mandatory attribute counts too (AOM 1.4 §existence).
        let object_valued = s.children.is_empty()
            || s.children
                .iter()
                .any(|c| !matches!(c, CObject::CPrimitiveObject(_)));
        if min >= 1 && s.rm_attribute_name != "name" && object_valued {
            out.push(WebTemplateExistence {
                min,
                max,
                path: format!("{node_path}/{}", s.rm_attribute_name),
            });
        }
    }
    out
}

/// The names of the attributes of `co` the template prohibits: existence
/// `0..0` (AOM 1.4 `master04-constraint_model_package.adoc` §existence), or
/// constraint objects that all carry occurrences `0..0`.
pub(super) fn prohibited_attributes(co: &CObject) -> Vec<String> {
    inputs::attributes(co)
        .iter()
        .filter(|attr| {
            let children = inputs::attribute_children(attr);
            occurrences(attribute_existence(attr)).1 == 0
                || (!children.is_empty()
                    && children
                        .iter()
                        .all(|c| occurrences(object_occurrences(c)).1 == 0))
        })
        .map(|attr| inputs::attribute_name(attr).to_owned())
        .collect()
}
