// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Surface A1: OPT 1.4 artefact validity (`I_DEFINITION_ADL14` upload).
//!
//! openEHR formalizes the validity rules a CDR applies to an uploaded artefact
//! in the AOM2 validation catalogue
//! (`docs/specs/openehr/AM/docs/AOM2/master08-validation.adoc`) and the AOM2
//! class-definition rule blocks
//! (`AM/docs/AOM2/master04.5-constraint_model-class_definitions.adoc`,
//! `AM/docs/AOM2/master07-terminology_package.adoc`). OPT 1.4 has no normative
//! prose chapter, so AOM2/08 is the oracle here, applied to the flattened OPT 1.4
//! tree (`openehr_its::opt14::types::OperationalTemplate`).
//!
//! This module owns the tree walk (T1: the `C_COMPLEX_OBJECT`/`C_ATTRIBUTE`
//! alternation) and the shared context; the per-kind rules live in sibling
//! modules along the AOM2/08 catalogue's own section axis:
//!
//! - `invariants` — AOM 1.4 constraint-model per-node-kind invariants
//!   (`Existence_set`, `Members_valid`, `Target_path_valid`, VARID/VARDT, VACDF,
//!   VDFAI, STCDC);
//! - `rm_conformance` — VCORM/VCARM/VCAEX/VCACA/VCAM + VACMCO over
//!   `openehr_rm::v1_2::model`;
//! - `primitive` — `C_PRIMITIVE`, temporal and duration patterns, the
//!   `C_DOMAIN_TYPE` assumed-value rules;
//! - `terminology` — VATID/VTTBK/VTCBK/VTLC plus code collection;
//! - `interval` — the BASE interval and multiplicity primitives.
//!
//! It does not run `valid_value`, which is instance-time (surface B in
//! [`crate::validation`]). Every violation is reported through
//! [`ServiceError::ValidationFailed`], the ITS-REST `422` carrying the AOM2 rule
//! code in `validationErrors[]`: an AOM2 rule violation is a semantic error on a
//! successfully parsed artefact (overview `Requests_and_responses.md` §HTTP
//! status codes), distinct from the syntactic `400` branch that owns
//! not-well-formed content.
//!
//! The check sequence is terminology sets first, then the walk, and per node
//! VCORM, VATID, per-kind invariants, recurse. It reports EVERY violation it
//! finds rather than the first, because the alternative made repairing a
//! template one upload per defect (#3129) — and a 1.7 MB operational template
//! is not something anyone re-uploads casually. The order above is the order
//! they are reported in.
//!
//! Two failures stop a subtree instead of accumulating: an object whose RM type
//! the model does not have, and an attribute its parent RM type does not
//! declare. Everything below either would be checked against a shape that does
//! not exist, so the violation carries a clause saying its subtree went
//! unchecked. See [`Findings`].

mod interval;
mod invariants;
mod primitive;
mod resource;
mod rm_conformance;
mod terminology;

use std::collections::HashSet;

use openehr_its::opt14::types::{CAttribute, CObject, Intervalofinteger, OperationalTemplate};

use crate::service::error::ServiceError;

/// One artefact-validity violation: the AOM2 rule code + a human detail.
struct RuleViolation {
    code: &'static str,
    detail: String,
}

impl RuleViolation {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

/// How many violations one refusal carries before it stops collecting.
///
/// Generous on purpose: a template a person is repairing has tens of defects at
/// worst, and the whole point of reporting them together is that the list is
/// the repair list. The cap exists for the artefact nobody is repairing — a
/// corrupted or machine-mangled upload where every node is wrong — so a `422`
/// body stays something a client can read. Reaching it is reported, never
/// silent.
const MAX_REPORTED_VIOLATIONS: usize = 200;

/// The violations found so far.
///
/// The AOM2 rules are properties of separate nodes, so one failure says nothing
/// about the next and the walk keeps going. Two failures are different: an
/// object whose RM type does not exist, and an attribute its parent RM type
/// does not declare. Everything below those is being checked against a shape
/// the model does not have, so their subtrees are pruned and the violation says
/// so rather than burying the real defect under its own consequences.
struct Findings {
    violations: Vec<RuleViolation>,
    truncated: bool,
}

impl Findings {
    fn new() -> Self {
        Self {
            violations: Vec::new(),
            truncated: false,
        }
    }

    /// Record `outcome` when it is a violation; returns whether it was one, so
    /// a caller that must prune its subtree can see it without re-checking.
    fn record(&mut self, outcome: Result<(), RuleViolation>) -> bool {
        let Err(violation) = outcome else {
            return false;
        };
        if self.violations.len() < MAX_REPORTED_VIOLATIONS {
            self.violations.push(violation);
        } else {
            self.truncated = true;
        }
        true
    }

    /// Record a violation whose subtree is being pruned, saying so in its own
    /// detail: a reader must not read the absence of nested violations as their
    /// absence from the artefact.
    fn record_pruned(&mut self, outcome: Result<(), RuleViolation>, what: &str) -> bool {
        let Err(mut violation) = outcome else {
            return false;
        };
        violation.detail = format!(
            "{}; the {what} below it was not checked, because every rule under it \
             would be checked against a shape the reference model does not have",
            violation.detail
        );
        self.record(Err(violation))
    }

    /// Record every violation in `outcomes`, for a rule that yields more than
    /// one at a time.
    fn record_all(&mut self, outcomes: Vec<RuleViolation>) {
        for violation in outcomes {
            self.record(Err(violation));
        }
    }

    fn is_empty(&self) -> bool {
        self.violations.is_empty()
    }
}

/// The per-walk validation context: the globally-collected code sets plus
/// whether the artefact declares any `constraint_definitions` at all (which
/// gates VACDF — see `invariants::check_constraint_ref`).
struct Ctx {
    defined_at: HashSet<String>,
    defined_ac: HashSet<String>,
    has_constraint_defs: bool,
}

/// The `C_OBJECT` fields every structural rule interrogates, extracted once
/// per node — the `opt14` `C_OBJECT` family is a closed 13-variant set, so one
/// exhaustive match here replaces a per-field accessor match per rule.
struct NodeView<'a> {
    rm_type: &'a str,
    node_id: &'a str,
    occurrences: &'a Intervalofinteger,
    /// Empty for the leaf kinds (only `C_ARCHETYPE_ROOT` / `C_COMPLEX_OBJECT`
    /// / `T_COMPLEX_OBJECT` carry attribute constraints).
    attributes: &'a [CAttribute],
}

impl<'a> NodeView<'a> {
    fn of(obj: &'a CObject) -> Self {
        match obj {
            CObject::ArchetypeInternalRef(o) => {
                Self::leaf(&o.rm_type_name, &o.node_id, &o.occurrences)
            }
            CObject::ArchetypeSlot(o) => Self::leaf(&o.rm_type_name, &o.node_id, &o.occurrences),
            CObject::ConstraintRef(o) => Self::leaf(&o.rm_type_name, &o.node_id, &o.occurrences),
            CObject::CArchetypeRoot(o) => Self {
                rm_type: &o.rm_type_name,
                node_id: &o.node_id,
                occurrences: &o.occurrences,
                attributes: &o.attributes,
            },
            CObject::CCodePhrase(o) => Self::leaf(&o.rm_type_name, &o.node_id, &o.occurrences),
            CObject::CCodeReference(o) => Self::leaf(&o.rm_type_name, &o.node_id, &o.occurrences),
            CObject::CComplexObject(o) => Self {
                rm_type: &o.rm_type_name,
                node_id: &o.node_id,
                occurrences: &o.occurrences,
                attributes: &o.attributes,
            },
            CObject::CDefinedObject(o) => Self::leaf(&o.rm_type_name, &o.node_id, &o.occurrences),
            CObject::CDvOrdinal(o) => Self::leaf(&o.rm_type_name, &o.node_id, &o.occurrences),
            CObject::CDvQuantity(o) => Self::leaf(&o.rm_type_name, &o.node_id, &o.occurrences),
            CObject::CDvState(o) => Self::leaf(&o.rm_type_name, &o.node_id, &o.occurrences),
            CObject::CPrimitiveObject(o) => Self::leaf(&o.rm_type_name, &o.node_id, &o.occurrences),
            CObject::TComplexObject(o) => Self {
                rm_type: &o.rm_type_name,
                node_id: &o.node_id,
                occurrences: &o.occurrences,
                attributes: &o.attributes,
            },
        }
    }

    fn leaf(rm_type: &'a str, node_id: &'a str, occurrences: &'a Intervalofinteger) -> Self {
        Self {
            rm_type,
            node_id,
            occurrences,
            attributes: &[],
        }
    }
}

/// The child objects constrained under a `C_ATTRIBUTE` (single- and
/// multiple-valued alike).
fn attribute_children(attr: &CAttribute) -> &[CObject] {
    match attr {
        CAttribute::CSingleAttribute(a) => &a.children,
        CAttribute::CMultipleAttribute(a) => &a.children,
    }
}

/// Validate an uploaded OPT 1.4 artefact against the AOM2/08 standalone-artefact
/// validity rules. Every violation found is returned as one `422` carrying the
/// AOM2 rule codes; a fully valid artefact returns `Ok`.
///
/// # Errors
///
/// [`ServiceError::ValidationFailed`] (→ ITS-REST `422` rendering the `Error`
/// object with one entry per violation in `validationErrors[]`, plus a
/// `TRUNCATED` entry when the artefact had more than
/// [`MAX_REPORTED_VIOLATIONS`]): an AOM2 rule violation is a semantic error on
/// a successfully parsed
/// artefact (the overview status table's `422` row,
/// `docs/specs/openehr/ITS-REST/specifications/docs/overview/
/// Requests_and_responses.md` §HTTP status codes; no template operation
/// declares `422`, so the semantic branch is register-adjudicated), never the
/// syntactic `400` branch of `responses/400.yaml`.
pub(super) fn validate_opt_artefact(opt: &OperationalTemplate) -> Result<(), ServiceError> {
    let findings = check(opt);
    if findings.is_empty() {
        return Ok(());
    }
    let mut errors: Vec<openehr_base::validate::InvariantViolation> = findings
        .violations
        .into_iter()
        .map(|v| openehr_base::validate::InvariantViolation::at(v.code.to_string(), v.detail))
        .collect();
    if findings.truncated {
        errors.push(openehr_base::validate::InvariantViolation::at(
            "TRUNCATED".to_owned(),
            format!(
                "this artefact has more than {MAX_REPORTED_VIOLATIONS} validity violations; \
                 the rest were not reported. Fix these and upload again to see what remains"
            ),
        ));
    }
    Err(ServiceError::ValidationFailed(errors))
}

fn check(opt: &OperationalTemplate) -> Findings {
    let mut f = Findings::new();
    // Terminology-side rules first (cheap; no tree recursion needed beyond code
    // collection).
    let ctx = Ctx {
        defined_at: terminology::collect_defined_at_codes(opt),
        defined_ac: terminology::collect_defined_ac_codes(opt),
        has_constraint_defs: terminology::flat_ontologies(opt)
            .iter()
            .any(|o| o.constraint_definitions.iter().any(|s| !s.items.is_empty())),
    };
    f.record(terminology::check_term_bindings(opt, &ctx.defined_at)); // VTTBK
    f.record(terminology::check_constraint_bindings(opt, &ctx.defined_ac)); // VTCBK
    f.record(terminology::check_language_consistency(opt)); // VTLC

    // RM-conformance + structural rules walk the flattened definition tree.
    // The root definition is a `C_ARCHETYPE_ROOT`; its `rm_type_name` is the
    // top RM type (VCORM).
    let root_rm = opt.definition.rm_type_name.as_str();
    let root_type_unknown = f.record_pruned(
        rm_conformance::check_object_type(root_rm, &opt.definition.node_id),
        "whole definition tree",
    );
    f.record(terminology::check_node_id(
        &opt.definition.node_id,
        &ctx.defined_at,
    )); // VATID (root)
    // VARID / VARDT on the root archetype id (ADL1.4 master08 lines 544/556).
    f.record(invariants::check_archetype_id(
        &opt.definition.archetype_id.value,
        root_rm,
    ));
    if !root_type_unknown {
        for attr in &opt.definition.attributes {
            walk_attribute(attr, root_rm, &ctx, &mut f);
        }
    }

    // The RM resource-package invariants over the OPT's own meta-data header
    // run last, so the codes a reader sees keep the order the catalogue walks
    // them in.
    f.record_all(resource::check_resource_meta(opt));
    f
}

// ─── tree walk (T1: C_COMPLEX_OBJECT / C_ATTRIBUTE alternation) ──────────────────

/// Recurse into one constrained attribute of an object whose RM type is
/// `parent_rm`.
fn walk_attribute(attr: &CAttribute, parent_rm: &str, ctx: &Ctx, f: &mut Findings) {
    let (attr_name, existence, children, cardinality) = match attr {
        CAttribute::CSingleAttribute(a) => (
            a.rm_attribute_name.as_str(),
            &a.existence,
            &a.children,
            None,
        ),
        CAttribute::CMultipleAttribute(a) => (
            a.rm_attribute_name.as_str(),
            &a.existence,
            &a.children,
            Some(&a.cardinality),
        ),
    };

    // An attribute the parent RM type does not declare makes every rule below
    // it meaningless, so its children are pruned and the violation says so.
    if f.record_pruned(
        invariants::check_attribute_name(attr_name, parent_rm),
        "constraint tree",
    ) {
        return;
    }
    f.record(invariants::check_existence_set(
        attr_name, parent_rm, existence,
    ));

    // A single-valued attribute (no cardinality) can hold at most one value.
    if cardinality.is_none() {
        f.record(invariants::check_members_valid(
            attr_name, parent_rm, children,
        ));
    }

    // RM-conformance checks (VCARM/VCAM/VCAEX) on the resolved RM attribute.
    f.record(rm_conformance::check_attribute(
        attr, attr_name, parent_rm, existence,
    ));

    // VACMCO / VCOC: occurrences-vs-cardinality (container attributes only).
    if let Some(card) = cardinality {
        f.record(rm_conformance::check_cardinality_occurrences(
            attr_name, parent_rm, card, children,
        ));
    }

    for child in children {
        walk_object(child, ctx, f);
    }
}

/// Check one child object node, then recurse into its own attributes.
fn walk_object(obj: &CObject, ctx: &Ctx, f: &mut Findings) {
    let view = NodeView::of(obj);

    // VCORM: object constraint type-name existence. A primitive-object node
    // carries a foundation primitive type name (STRING, INTEGER, …) which is
    // intentionally absent from the RM model, so it is exempt. A type the model
    // does not have prunes its own subtree, for the reason `Findings` records.
    if !matches!(obj, CObject::CPrimitiveObject(_))
        && f.record_pruned(
            rm_conformance::check_object_type(view.rm_type, view.node_id),
            "constraint tree",
        )
    {
        return;
    }

    // VATID: every at-code used as a node_id must be defined in terminology.
    f.record(terminology::check_node_id(view.node_id, &ctx.defined_at));

    // AOM 1.4 per-node-kind invariants (the constraint-model class files).
    match obj {
        CObject::CArchetypeRoot(root) => {
            // VARID / VARDT on every flattened slot-filler root.
            f.record(invariants::check_archetype_id(
                &root.archetype_id.value,
                &root.rm_type_name,
            ));
        }
        CObject::ArchetypeSlot(slot) => {
            f.record(invariants::check_slot(slot));
        }
        CObject::ArchetypeInternalRef(r) => {
            f.record(invariants::check_internal_ref(r));
        }
        CObject::ConstraintRef(r) => {
            f.record(invariants::check_constraint_ref(r, ctx));
        }
        CObject::CPrimitiveObject(p) => {
            if let Some(item) = &p.item {
                f.record(primitive::check_primitive(item, view.node_id));
            }
        }
        CObject::CCodePhrase(c) => {
            f.record_all(invariants::check_code_list(&c.code_list, view.node_id));
            f.record(primitive::check_assumed_code(
                c.assumed_value.as_ref(),
                &c.code_list,
                view.node_id,
            ));
        }
        CObject::CCodeReference(c) => {
            f.record_all(invariants::check_code_list(&c.code_list, view.node_id));
            f.record(primitive::check_assumed_code(
                c.assumed_value.as_ref(),
                &c.code_list,
                view.node_id,
            ));
        }
        CObject::CDvOrdinal(c) => {
            f.record(primitive::check_dv_ordinal(c, view.node_id));
        }
        CObject::CDvQuantity(c) => {
            f.record(primitive::check_dv_quantity(c, view.node_id));
        }
        _ => {}
    }

    // Recurse into a nested C_ARCHETYPE_ROOT's terminology scope-wise via the
    // global set already collected; structurally we just descend its attributes.
    for attr in view.attributes {
        walk_attribute(attr, view.rm_type, ctx, f);
    }
}
