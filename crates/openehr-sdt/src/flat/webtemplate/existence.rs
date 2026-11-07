// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The AOM 1.4 `C_ATTRIBUTE.existence` constraints of an OPT 1.4 node
//! (`master04-constraint_model_package.adoc` §existence): the mandatory
//! attributes the validation walk enforces, and the prohibited attributes the
//! in-context synthesis leaves out.

use openehr_its::opt14::types::{CAttribute, CObject};

use super::builder::{object_occurrences, occurrences};
use super::implicit::attribute_existence;
use super::inputs;
use super::model::WebTemplateExistence;

/// Capture the AOM 1.4 `C_ATTRIBUTE.existence` constraints for the mandatory
/// single-valued RM attributes of `co`, keyed by their absolute archetype path.
///
/// Scope: `C_SINGLE_ATTRIBUTE`s with an existence lower bound `>= 1` and at
/// least one object-valued constraint child (excluding pure function/primitive
/// constraints such as `is_integral`/`lower_included`, which never appear as
/// navigable instance attributes). `name` is excluded (it names the node —
/// master04 §"Field Identifiers" — and is matched by the archetype-node
/// predicate instead).
///
/// A node-identified structural child (e.g. `OBSERVATION.state → HISTORY[at0005]`,
/// `HISTORY.summary → ITEM_TREE[at0007]`) is captured here **too**, not only when
/// the child carries no `node_id`: existence and *occurrences* are orthogonal
/// (AOM 1.4 §existence — "indicates whether its target object exists or not, i.e.
/// is mandatory or not"), and a mandated structural wrapper with no leaf content
/// is dropped by master04 §"Level Removal" compaction, so it never becomes a
/// walkable node the occurrence check could visit — leaving existence as the only
/// enforcement that the mandatory attribute is present at all. When the child
/// does survive, existence and occurrences both fire only on absence (redundant,
/// never contradictory).
///
/// NOTE: AOM 1.4 (`master04-constraint_model_package.adoc` §existence) makes
/// existence "always required" with an unstated default of `{1..1}`; the OPT XML
/// always serialises it, and we honour the declared value (biasing toward
/// confident violations — an unstated/`{0..1}` existence is not enforced).
pub(super) fn existence_constraints(co: &CObject, node_path: &str) -> Vec<WebTemplateExistence> {
    let mut out = Vec::new();
    for attr in inputs::attributes(co) {
        // A CONTAINER attribute with existence lower >= 1 demands the
        // attribute's presence regardless of member cardinality (AOM 1.4
        // c_attribute `Existence_set`; cardinality then governs membership of
        // the present container — the two constraints are orthogonal).
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
        // Require one object-valued constraint child, which selects real
        // structural attributes and excludes function/primitive constraints
        // (`is_integral`, `lower_included`, …) that are never navigable instance
        // attributes. A CHILDLESS mandatory attribute also counts: AOM 1.4
        // `master04-constraint_model_package.adoc` §existence lets an archetype
        // demand presence without constraining the value.
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
