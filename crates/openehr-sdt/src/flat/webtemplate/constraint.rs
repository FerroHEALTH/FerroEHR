// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The template constraints a Web Template node captures beside its document.
//!
//! The validation walk and the composition builder read them from a
//! [`super::model::WebTemplateNode`]. None of them is part of the Web Template
//! metadata document (`ITS-REST simplified_formats master04 §"Web Template
//! Metadata"`), so none is serialized.

use crate::flat::validation::subtype;

/// An AOM 1.4 `C_ATTRIBUTE.existence` constraint on a single RM attribute.
///
/// Existence is whether the attribute *field* is present at all — distinct from
/// `cardinality` = container membership and `occurrences` = per-object-block
/// count; AOM 1.4 `master04-constraint_model_package.adoc` §existence. Captured
/// for the validation walk only; not part of the Web Template metadata
/// document.
///
/// `path` is the absolute archetype path of the constrained attribute
/// (`{node aqlPath}/{rm_attribute_name}`); `min`/`max` are the existence bounds
/// (`max == -1` unbounded). A mandatory attribute has `min >= 1`.
#[derive(Debug, Clone)]
pub struct WebTemplateExistence {
    /// The existence lower bound; `>= 1` means the attribute is mandatory.
    pub min: i32,
    /// The existence upper bound; `-1` means unbounded.
    pub max: i32,
    /// Absolute archetype path of the constrained attribute (`{node aqlPath}/{rm_attribute_name}`).
    pub path: String,
}

/// A hoisted-wrapper type narrowing (validation-only, never serialized).
///
/// The instance node(s) matched at the absolute archetype `path` must conform to
/// `rm_type` (or to one of the types when several same-path alternatives were
/// hoisted — the walk groups by path).
#[derive(Debug, Clone)]
pub struct WebTemplateSlot {
    /// Absolute archetype path of the hoisted wrapper constraint.
    pub path: String,
    /// The wrapper's constrained RM type (`ITEM_LIST`, `POINT_EVENT`, …; an
    /// abstract type such as `ITEM_STRUCTURE` admits every concrete subtype).
    pub rm_type: String,
    /// The hoisted wrapper's name, the term definition of its node id, when
    /// the template gives one; a builder names the re-materialised wrapper
    /// with it (RM common `master03-archetyped_package.adoc` §"The LOCATABLE
    /// class").
    pub name: Option<String>,
}

/// A `C_CODE_PHRASE` code-list constraint on a coded RM attribute of a leaf.
///
/// Validation-only, never serialized: `attr` is the RM attribute name (e.g.
/// `media_type`), `terminology` the constrained terminology id (`None` =
/// `local`), `codes` the allowed `code_string`s.
#[derive(Debug, Clone)]
pub struct WebTemplateCodeList {
    /// The constrained RM attribute name (e.g. `media_type`).
    pub attr: String,
    /// The constrained terminology id; `None` means the archetype-`local` terminology.
    pub terminology: Option<String>,
    /// The allowed `code_string`s.
    pub codes: Vec<String>,
}

/// An archetype **constraint binding** in force on a coded leaf
/// (validation-only, never serialized).
///
/// A `CONSTRAINT_REF` under a coded attribute is "a proxy for a set of
/// constraints … expressed in the binding of the constraint reference (e.g.
/// 'ac0004') to a query … into an external service (e.g. a terminology
/// service)" (`AM/docs/AOM1.4/master04-constraint_model_package.adoc`
/// §Reference Objects); the ac-code → query mapping is the archetype ontology's
/// `constraint_bindings` (`AM/docs/ADL1.4/master08-adl.adoc`
/// §Constraint_bindings). Resolving the query is a terminology-server duty
/// (BASE `architecture_overview/master12-terminology.adoc` §"Binding
/// Terminology Value-sets to Archetypes"), so the web template only records
/// what must be asked.
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

/// A structural stub for a structural attribute (`data`, `description`,
/// `protocol`, `state`) of an ENTRY, EVENT or ACTIVITY node
/// (validation/synthesis-only, never serialized).
///
/// The template constrains the attribute `attr` (e.g. `description`) with a
/// node-identified structural child of RM type `rm_type` (`ITEM_TREE`,
/// `HISTORY`, …) and archetype node id `node_id` (`at0017`), whose rubric
/// `name` comes from the archetype `term_definitions`. When the attribute
/// carries no leaf content, the compacted web-template drops the wrapper, so
/// this record lets the composition builder synthesise the empty attribute
/// with the *constrained* identity — a value the closed-archetype walk admits
/// (AOM 1.4 `AM/docs/AOM1.4/master04-constraint_model_package.adoc`
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

/// A closed-archetype constraint on one attribute (validation-only, never
/// serialized).
///
/// Under the constrained attribute at absolute archetype `path`, an instance
/// child bearing an `archetype_node_id` is admissible iff it matches one of
/// `allowed_ids` (a fixed at-code / archetype-id sibling alternative) whose
/// RM type it conforms to **or**
/// an open `ARCHETYPE_SLOT` in `slots`. Any other archetyped child is an
/// "unexpected node" (closed-world rejection).
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

/// An open `ARCHETYPE_SLOT` constraint (AOM 1.4 `ARCHETYPE_SLOT`;
/// validation-only, never serialized).
///
/// A slot-filling instance object must conform to `rm_type`, match at least
/// one `includes` archetype-id regex, and match no `excludes` regex (a
/// blanket `.*` exclude is ignored when `includes` is non-empty — the ADL 1.4
/// closed-slot idiom; AOM 1.4 has no `is_closed`). `min`/`max` bound the
/// permitted number of fillers (`max == -1` unbounded).
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
    /// archetype id is `archetype_id` (AOM 1.4 `ARCHETYPE_SLOT`, `includes`
    /// and `excludes`).
    ///
    /// `rm_type` conforms to the slot's `rm_type` by the RM inheritance
    /// graph, generic parameters ignored. The id matches at least one
    /// `includes` pattern (an empty list admits every id) and no `excludes`
    /// pattern. Each pattern constrains the whole id: it is anchored at both
    /// ends, and the slashes ADL writes around it are dropped. A match-all
    /// exclude (`.*` or `.+`) beside a non-empty `includes` is the ADL 1.4
    /// closed-slot idiom and excludes nothing the includes admit.
    ///
    /// The decision fails closed on a slot [`Self::defect`] reports: a slot
    /// type the RM does not define admits nothing, an include no engine can
    /// evaluate matches nothing, and such an exclude refuses every id.
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

/// Refuses a Web Template that holds a slot [`WebTemplateArchetypeSlot::defect`]
/// reports, naming the attribute the slot stands under.
pub(crate) fn check_slots(
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
        stack.extend(node.children.iter().chain(&node.alternatives));
    }
    Ok(())
}

/// Whether `rm_type` names a class of the RM, generic parameters ignored.
fn rm_type_is_known(rm_type: &str) -> bool {
    openehr_rm::v1_2::model::class(subtype::base(rm_type)).is_some()
}

/// A slot pattern compiled, anchored at both ends, by `regex` or else by
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

/// Whether `pattern` matches the whole of `archetype_id`; `None` when the
/// pattern cannot be evaluated.
fn pattern_matches(pattern: &str, archetype_id: &str) -> Option<bool> {
    match compile(pattern)? {
        Compiled::Plain(re) => Some(re.is_match(archetype_id)),
        Compiled::Fancy(re) => re.is_match(archetype_id).ok(),
    }
}
