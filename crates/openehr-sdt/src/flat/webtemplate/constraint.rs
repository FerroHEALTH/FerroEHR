// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The template constraints a Web Template node captures beside its document,
//! read by the validation walk and the composition builder; none is serialized.

use crate::flat::validation::subtype;

/// An AOM 1.4 `C_ATTRIBUTE.existence` constraint on a single RM attribute
/// (`master04-constraint_model_package.adoc` §existence).
///
/// `path` is `{node aqlPath}/{rm_attribute_name}`; `max == -1` is unbounded.
#[derive(Debug, Clone)]
pub struct WebTemplateExistence {
    /// The existence lower bound; `>= 1` means the attribute is mandatory.
    pub min: i32,
    /// The existence upper bound; `-1` means unbounded.
    pub max: i32,
    /// Absolute archetype path of the constrained attribute (`{node aqlPath}/{rm_attribute_name}`).
    pub path: String,
}

/// A hoisted-wrapper type narrowing: instance nodes matched at `path` must
/// conform to `rm_type`, or to one of the same-path alternatives.
#[derive(Debug, Clone)]
pub struct WebTemplateSlot {
    /// Absolute archetype path of the hoisted wrapper constraint.
    pub path: String,
    /// The wrapper's constrained RM type (`ITEM_LIST`, `POINT_EVENT`, …; an
    /// abstract type such as `ITEM_STRUCTURE` admits every concrete subtype).
    pub rm_type: String,
    /// The hoisted wrapper's name, from the term definition of its node id (RM
    /// common `master03-archetyped_package.adoc` §"The LOCATABLE class").
    pub name: Option<String>,
}

/// A `C_CODE_PHRASE` code-list constraint on a coded RM attribute of a leaf;
/// `terminology` `None` means `local`.
#[derive(Debug, Clone)]
pub struct WebTemplateCodeList {
    /// The constrained RM attribute name (e.g. `media_type`).
    pub attr: String,
    /// The constrained terminology id; `None` means the archetype-`local` terminology.
    pub terminology: Option<String>,
    /// The allowed `code_string`s.
    pub codes: Vec<String>,
}

/// An archetype constraint binding in force on a coded leaf.
///
/// A `CONSTRAINT_REF` is resolved by a query into an external service (AOM 1.4
/// `master04-constraint_model_package.adoc` §Reference Objects;
/// `AM/docs/ADL1.4/master08-adl.adoc` §Constraint_bindings), so the web template
/// records what must be asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebTemplateConstraintBinding {
    /// The RM attribute of this leaf holding the coded value the binding
    /// constrains (`defining_code` for a `DV_CODED_TEXT`; empty when the node
    /// itself IS the `CODE_PHRASE` proxy).
    pub attr: String,
    /// The archetype constraint code (`ac0004`) the binding is keyed by.
    pub ac_code: String,
    /// The terminology the binding names (the `ConstraintBindingSet`
    /// `terminology`, e.g. `SNOMED-CT`, `LOINC`).
    pub terminology: String,
    /// The bound terminology-query URI whose result is the admissible value
    /// set.
    pub query_uri: String,
}

/// A structural stub for a constrained structural attribute.
///
/// The node-identified child the template constrains `data`, `description`,
/// `protocol` or `state` with, kept so the builder can fill the dropped
/// attribute with that identity (AOM 1.4 `master04-constraint_model_package.adoc`
/// §`Valid_value`).
#[derive(Debug, Clone)]
pub struct WebTemplateStructuralStub {
    /// The constrained RM attribute name (`data`, `description`, `protocol`,
    /// `state`).
    pub attr: String,
    /// Absolute archetype path of the constrained attribute
    /// (`{owner aqlPath}/{attr}`), so the stub stays addressable after the
    /// compactor re-homes it onto an ancestor.
    pub path: String,
    /// The constrained structural RM type (`ITEM_TREE`, `HISTORY`, …).
    pub rm_type: String,
    /// The constraint's archetype node id (`atNNNN`, or an archetype id at a root).
    pub node_id: String,
    /// The rubric text for `node_id` from the archetype `term_definitions`, when
    /// present.
    pub name: Option<String>,
}

/// A closed-archetype constraint on one attribute: an archetyped instance child
/// at `path` must match one of `allowed_ids` or an open slot in `slots`.
#[derive(Debug, Clone)]
pub struct WebTemplateClosedAttribute {
    /// Absolute archetype path of the constrained attribute (`{node aqlPath}/{attr}`).
    pub path: String,
    /// Fixed sibling identities (constraint `node_id`s at at-code level, or the
    /// `archetype_id` of a `C_ARCHETYPE_ROOT` / `ARCHETYPE_INTERNAL_REF`).
    pub allowed_ids: Vec<String>,
    /// The `rm_type_name` of each object constraint in `allowed_ids`, index
    /// for index: a child matching an id must also conform to that RM type.
    pub allowed_rm_types: Vec<String>,
    /// Open `ARCHETYPE_SLOT`s under this attribute.
    pub slots: Vec<WebTemplateArchetypeSlot>,
}

/// An open `ARCHETYPE_SLOT` constraint (AOM 1.4 `ARCHETYPE_SLOT`); `max == -1` is
/// unbounded.
#[derive(Debug, Clone)]
pub struct WebTemplateArchetypeSlot {
    /// The RM type a slot filler must conform to.
    pub rm_type: String,
    /// The minimum number of permitted fillers.
    pub min: i32,
    /// The maximum number of permitted fillers; `-1` means unbounded.
    pub max: i32,
    /// Archetype-id regexes a filler must match at least one of.
    pub includes: Vec<String>,
    /// Archetype-id regexes a filler must match none of.
    pub excludes: Vec<String>,
}

impl WebTemplateArchetypeSlot {
    /// Returns whether the slot admits a filler of RM type `rm_type` whose
    /// archetype id is `archetype_id` (AOM 1.4 `ARCHETYPE_SLOT`).
    ///
    /// Patterns are anchored at both ends; an empty `includes` admits every id, and
    /// a match-all exclude beside a non-empty `includes` is the ADL 1.4 closed-slot
    /// idiom. A slot [`Self::defect`] reports fails closed.
    #[must_use]
    pub fn admits(&self, rm_type: &str, archetype_id: &str) -> bool {
        if !rm_type_is_known(&self.rm_type) || !subtype::conforms(rm_type, &self.rm_type) {
            return false;
        }
        let included = self.includes.is_empty()
            || self
                .includes
                .iter()
                .any(|pattern| pattern_matches(pattern, archetype_id) == Some(true));
        included
            && !self.excludes.iter().any(|pattern| {
                let closed_idiom =
                    !self.includes.is_empty() && matches!(pattern.trim(), ".*" | ".+");
                !closed_idiom && pattern_matches(pattern, archetype_id) != Some(false)
            })
    }

    /// Returns the first defect that keeps the slot from being evaluated: a
    /// `rm_type` the RM does not define, or an include or exclude pattern
    /// neither `regex` nor `fancy-regex` compiles.
    #[must_use]
    pub fn defect(&self) -> Option<SlotDefect> {
        if !rm_type_is_known(&self.rm_type) {
            return Some(SlotDefect::UnknownRmType(self.rm_type.clone()));
        }
        self.includes
            .iter()
            .chain(&self.excludes)
            .find(|pattern| compile(pattern).is_none())
            .map(|pattern| SlotDefect::UnreadablePattern(pattern.clone()))
    }
}

/// Why an `ARCHETYPE_SLOT` cannot be evaluated.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SlotDefect {
    /// The slot's `rm_type` is no class of the RM (an empty one included).
    #[error("the slot type {0:?} is no class of the reference model")]
    UnknownRmType(String),
    /// An include or exclude pattern neither regex engine compiles.
    #[error("the archetype-id pattern {0:?} compiles in no regex engine")]
    UnreadablePattern(String),
}

/// Refuses a Web Template holding a constraint no engine can evaluate: a slot
/// [`WebTemplateArchetypeSlot::defect`] reports, or an uncompilable `C_STRING`
/// pattern.
pub(crate) fn check_evaluable(
    tree: &super::model::WebTemplateNode,
) -> Result<(), crate::flat::error::FlatError> {
    let mut stack = vec![tree];
    while let Some(node) = stack.pop() {
        for attribute in &node.closed_attributes {
            if let Some(defect) = attribute
                .slots
                .iter()
                .find_map(WebTemplateArchetypeSlot::defect)
            {
                return Err(crate::flat::error::FlatError::UnevaluableSlot {
                    path: attribute.path.clone(),
                    defect,
                });
            }
        }
        let unreadable = node
            .inputs
            .iter()
            .filter(|input| input.input_type == super::model::WebTemplateInputType::Text)
            .filter_map(|input| input.validation.as_ref()?.pattern.as_ref())
            .find(|pattern| compile(pattern).is_none());
        if let Some(pattern) = unreadable {
            return Err(crate::flat::error::FlatError::UnevaluablePattern {
                path: node.aql_path.clone(),
                pattern: pattern.clone(),
            });
        }
        stack.extend(node.children.iter().chain(&node.alternatives));
    }
    Ok(())
}

/// Whether `rm_type` names a class of the RM, generic parameters ignored.
fn rm_type_is_known(rm_type: &str) -> bool {
    openehr_rm::v1_2::model::class(subtype::base(rm_type)).is_some()
}

/// An archetype regex (a slot pattern or a `C_STRING` pattern) compiled, anchored at both ends, by `regex` or else by
/// `fancy-regex` (the ADL regex dialect is a subset of Perl, ADL 1.4
/// `master05-cadl.adoc` §Regular Expression); `None` when neither compiles it.
enum Compiled {
    Plain(regex::Regex),
    Fancy(fancy_regex::Regex),
}

fn compile(pattern: &str) -> Option<Compiled> {
    let body = pattern
        .strip_prefix('/')
        .and_then(|inner| inner.strip_suffix('/'))
        .unwrap_or(pattern);
    let anchored = format!("^(?:{body})$");
    regex::Regex::new(&anchored)
        .map(Compiled::Plain)
        .ok()
        .or_else(|| {
            // NOTE: ADL 1.4 `master05-cadl.adoc` §Regular Expression: a Perl pattern `regex` refuses goes to PCRE.
            fancy_regex::Regex::new(&anchored).map(Compiled::Fancy).ok()
        })
}

/// Whether `pattern` matches the whole of `value`; `None` when the pattern
/// cannot be evaluated.
pub(crate) fn pattern_matches(pattern: &str, value: &str) -> Option<bool> {
    match compile(pattern)? {
        Compiled::Plain(re) => Some(re.is_match(value)),
        Compiled::Fancy(re) => re.is_match(value).ok(),
    }
}
