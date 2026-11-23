// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Reference-model validation of ADL2 archetypes.
//!
//! `AOM2/master08-validation.adoc` §Phase 2 → Validate Against Reference Model:
//! VCORM, VCORMT, VCARM, VCAEX, VCACA and VCAM, with VACSO and the interior-node
//! half of VATID, which hinge on the RM's single/multiple determination.
//!
//! The checks are generic over a [`RmModel`] (`master11-rm_adaptation.adoc`):
//! [`ProductionRmModel`] for openEHR RM 1.2.0, or a BMM-loaded model. Type names
//! match case-insensitively with whitespace ignored (`ADL2/master04.3` §Reference
//! Model Type Matching), through [`base_type_name`].

use std::collections::BTreeSet;

use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::constraint_model::c_attribute::CAttribute;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object_proxy::CComplexObjectProxy;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_base::prelude::Interval;
use openehr_rm::v1_2::model;

use super::catalogue::ValidationCode;
use super::rm_extended::{class_attribute, class_conforms};
use super::{ValidationIssue, push_issue};
use crate::adl14::quantity_property::{ADL2_REFUSAL_ADVICE, is_quantity_property};
use crate::aom::access::{
    child_occurrences, complex_attributes, complex_rm_type, object_node_id, object_rm_type,
};
use crate::aom::interval::{Bounds, bounds, display_bounds, finite_upper, point_value_i32};
use crate::artefact::{ArchetypeView, view};
use crate::odin::is_delimited_regex_trimmed;
use crate::parse::Dialect;
use crate::paths::{child_path, locate};
use openehr_am::v2_4::aom2::definitions::adl_code_definitions::AdlCodeDefinitionsData;

/// One attribute of an RM type, as reported by a [`RmModel`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RmAttr {
    /// The declared value type of the attribute with its generic arguments, a
    /// formal parameter resolved to its bound (`HISTORY.events` reads
    /// `EVENT<ITEM_STRUCTURE>`); VCORMT matches against it covariantly (`master04.2`
    /// §`Rm_type_name`).
    pub declared_type: String,
    /// True if the attribute is a container (multiply-valued) in the RM.
    pub is_multiple: bool,
    /// The RM existence of the attribute (presence of the value/container
    /// itself): `{1..1}` if mandatory, else `{0..1}`.
    pub existence: Bounds,
    /// The RM cardinality of a container attribute, if the model records it;
    /// `None` for a single-valued attribute or a model that does not carry
    /// cardinality (see [`ProductionRmModel`]).
    pub cardinality: Option<Bounds>,
}

/// The declared literal set of an RM enumeration type (`master04.2`
/// §Constraints on Enumeration Types), as reported by a [`RmModel`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RmEnum {
    /// The underlying primitive type of the enumeration.
    pub underlying: EnumUnderlying,
    /// The declared integer literal values (populated for [`EnumUnderlying::Integer`]).
    pub int_values: Vec<i64>,
    /// The declared string literal values (populated for [`EnumUnderlying::String`]).
    pub str_values: Vec<String>,
}

/// The underlying primitive of an RM enumeration (`master04.2`: "a distinct type
/// based on a primitive type, normally Integer or String").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnumUnderlying {
    /// Integer-based enumeration (constrained by a `C_INTEGER`).
    Integer,
    /// String-based enumeration (constrained by a `C_STRING`).
    String,
}

/// A reference model against which an archetype is validated
/// (`master11-rm_adaptation.adoc`).
///
/// Type-name arguments may be generic and match case-insensitively with
/// whitespace ignored (`master04.3` §Reference Model Type Matching);
/// implementors route through [`base_type_name`].
pub trait RmModel {
    /// A human-readable identity of the model (for notices/messages).
    fn name(&self) -> &str;

    /// True if `rm_type` is a type defined in the reference model.
    fn type_exists(&self, rm_type: &str) -> bool;

    /// Conformance of `sub` to `sup`: `Some(true)` if `sub` is the same type as,
    /// or a subtype of, `sup`; `Some(false)` if both types are known but `sub`
    /// does not conform; `None` if either type is unknown to the model (so the
    /// caller cannot decide — VCORM reports the unknown type instead).
    fn conforms(&self, sub: &str, sup: &str) -> Option<bool>;

    /// Resolve `attr`, stored or computed (a function with no arguments, AOM2
    /// `master04.2` §Attribute Nodes), on `rm_type` through inheritance, or
    /// `None` if the type has no such attribute (`master04.5` VCARM) or is unknown.
    fn attribute(&self, rm_type: &str, attr: &str) -> Option<RmAttr>;

    /// If `rm_type` names an enumeration type in this model, its declared
    /// literal set; else `None`. Enumeration-typed slots arise where the
    /// information model types a property directly as an enumeration class
    /// (`master04.2` §Constraints on Enumeration Types).
    fn enumeration(&self, rm_type: &str) -> Option<RmEnum>;
}

/// The base (outer) class name of a possibly-generic RM type name, trimmed
/// (`"HISTORY <ITEM_LIST>"` → `"HISTORY"`; `master04.3` §Reference Model Type
/// Matching).
#[must_use]
pub fn base_type_name(rm_type: &str) -> &str {
    rm_type.split('<').next().unwrap_or(rm_type).trim()
}

/// The generic argument type names of a generic RM type name, in order
/// (`"HISTORY<ITEM_LIST>"` → `["ITEM_LIST"]`; a non-generic name → empty).
#[must_use]
pub(crate) fn generic_arguments(rm_type: &str) -> Vec<String> {
    let Some(open) = rm_type.find('<') else {
        return Vec::new();
    };
    let Some(close) = rm_type.rfind('>') else {
        return Vec::new();
    };
    let Some(inner) = rm_type.get(open + 1..close) else {
        return Vec::new();
    };
    split_top_level_commas(inner)
        .into_iter()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Split a generic argument list on top-level commas (not inside a nested
/// `<...>`).
fn split_top_level_commas(inner: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut cur = String::new();
    for ch in inner.chars() {
        match ch {
            '<' => {
                depth += 1;
                cur.push(ch);
            }
            '>' => {
                depth = depth.saturating_sub(1);
                cur.push(ch);
            }
            ',' if depth == 0 => out.push(std::mem::take(&mut cur)),
            _ => cur.push(ch),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

/// The case-folded, whitespace-free form of an RM type name for matching
/// (`master04.3` §Reference Model Type Matching: case-insensitive, whitespace
/// ignored).
#[must_use]
pub(crate) fn normalise_type_name(rm_type: &str) -> String {
    rm_type
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_uppercase)
        .collect()
}

/// Whether a stated object type `child` conforms to the declared RM type
/// `declared`, covariantly on generic arguments (`master04.2` §`Rm_type_name`).
#[must_use]
pub(crate) fn type_conforms(rm: &dyn RmModel, child: &str, declared: &str) -> Option<bool> {
    match rm.conforms(child, declared) {
        Some(true) => {}
        other => return other,
    }
    let declared_args = generic_arguments(declared);
    if declared_args.is_empty() {
        return Some(true);
    }
    let child_args = generic_arguments(child);
    if child_args.is_empty() || child_args.len() != declared_args.len() {
        return Some(true);
    }
    for (c, d) in child_args.iter().zip(declared_args.iter()) {
        if type_conforms(rm, c, d) == Some(false) {
            return Some(false);
        }
    }
    Some(true)
}

/// The openEHR RM 1.2.0 reference model, the generated `openehr_rm::v1_2::model`.
///
/// NOTE: VCORMT matches a stated generic argument against the parameter's bound
/// (`master04.2` §`Rm_type_name`), an approximation that never fires falsely.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProductionRmModel;

impl RmModel for ProductionRmModel {
    #[expect(
        clippy::unnecessary_literal_bound,
        reason = "the `RmModel` trait returns `&str` because the ODIN-backed implementation borrows a stored `String`; narrowing this impl to `&'static str` would not match the trait"
    )]
    fn name(&self) -> &str {
        "openEHR RM 1.2.0"
    }

    fn type_exists(&self, rm_type: &str) -> bool {
        // Only the base (outer) type is required to exist; generic arguments
        // (e.g. `ITEM_LIST` in `HISTORY<ITEM_LIST>`) may be RM classes or
        // foundation types the model excludes, so requiring them would
        // false-positive VCORM (master04.3 §Reference Model Type Matching).
        production_class(base_type_name(rm_type)).is_some()
    }

    fn conforms(&self, sub: &str, sup: &str) -> Option<bool> {
        let child = production_class(base_type_name(sub))?;
        let ancestor = production_class(base_type_name(sup))?;
        Some(class_conforms(child, ancestor))
    }

    fn attribute(&self, rm_type: &str, attr: &str) -> Option<RmAttr> {
        class_attribute(production_class(base_type_name(rm_type))?, attr)
    }

    fn enumeration(&self, rm_type: &str) -> Option<RmEnum> {
        let base = base_type_name(rm_type);
        let e = model::enumeration(base).or_else(|| model::enumeration(&base.to_uppercase()))?;
        Some(rm_enum_of(e))
    }
}

/// Render a generated attribute's declared type into a full RM type-name string
/// including generic arguments (`"EVENT"` + `[ITEM_STRUCTURE]` →
/// `"EVENT<ITEM_STRUCTURE>"`), the form [`RmAttr::declared_type`] carries.
pub(super) fn render_declared_type(base: &str, params: &[model::RmTypeRef]) -> String {
    if params.is_empty() {
        return base.to_owned();
    }
    let inner = params
        .iter()
        .map(render_type_ref)
        .collect::<Vec<_>>()
        .join(",");
    format!("{base}<{inner}>")
}

fn render_type_ref(t: &model::RmTypeRef) -> String {
    render_declared_type(t.name, t.params)
}

/// A [`Bounds`] from a generated model [`model::Cardinality`], clamping the
/// (tiny) container bounds into `i32` (a value beyond `i32` widens to the
/// permissive default rather than panicking — cardinality bounds are small).
pub(super) fn cardinality_bounds(c: model::Cardinality) -> Bounds {
    Bounds::new(
        i32::try_from(c.lower).unwrap_or(0),
        c.upper.and_then(|u| i32::try_from(u).ok()),
    )
}

/// An [`RmEnum`] from a generated model [`model::RmEnumeration`].
fn rm_enum_of(e: &model::RmEnumeration) -> RmEnum {
    let underlying = if e.underlying_type.eq_ignore_ascii_case("STRING") {
        EnumUnderlying::String
    } else {
        EnumUnderlying::Integer
    };
    let mut int_values = Vec::new();
    let mut str_values = Vec::new();
    for lit in e.literals {
        match lit.value {
            model::EnumValue::Int(i) => int_values.push(i),
            model::EnumValue::Str(s) => str_values.push(s.to_owned()),
        }
    }
    RmEnum {
        underlying,
        int_values,
        str_values,
    }
}

/// Look up a class in the generated model by its normalised (case-insensitive)
/// name. The generated table keys are the exact spec class names (upper-case
/// `SCREAMING_SNAKE`), so an upper-cased lookup matches them.
pub(super) fn production_class(base: &str) -> Option<&'static model::RmClass> {
    // Fast path: exact spec name; fallback: case-fold to match `Section` etc.
    model::class(base).or_else(|| model::class(&base.to_uppercase()))
}

/// Whether the openEHR [`ProductionRmModel`] governs `archetype`, from the HRID
/// `rm_publisher` (`openehr`) and a package that is no test schema.
#[must_use]
pub fn production_model_governs(archetype: &Archetype) -> bool {
    production_model_governs_view(&view(archetype))
}

fn production_model_governs_view(v: &ArchetypeView<'_>) -> bool {
    let publisher = v.archetype_id.rm_publisher.to_ascii_lowercase();
    let package = v.archetype_id.rm_package.to_ascii_uppercase();
    // The openEHR RM 1.2.0 packages the generated model carries. `TEST_PKG` (the
    // AOM2 test schema) and the component packages not compiled into this build
    // (TASK_PLANNING, foreign publishers) are excluded.
    publisher == "openehr" && !matches!(package.as_str(), "TEST_PKG" | "TASK_PLANNING")
}

/// Validate `archetype` against `rm` for `dialect`.
///
/// Under [`Dialect::Adl2`]: VCORM, VCARM, VCORMT, VCAEX, VCACA, VCAM, VACSO and
/// the interior-node half of VATID (`master08` §Phase 2). Under
/// [`Dialect::Adl14`]: VUNT only, the one ADL 1.4 rule that needs the RM
/// (`ADL1.4/master05-cadl.adoc` §Internal References). Called directly, the
/// checks run without the phase gate of [`validate_source`](super::validate_source).
#[must_use]
pub fn validate_rm_conformance(
    archetype: &Archetype,
    rm: &dyn RmModel,
    dialect: Dialect,
) -> Vec<ValidationIssue> {
    let v = view(archetype);
    let mut scan = RmScan {
        rm,
        root: v.definition,
        defined: super::scope::defined_codes(v.terminology),
        components: v.components,
        is_specialised: v.is_specialised(),
        issues: Vec::new(),
    };
    let root_type = complex_rm_type(v.definition);
    // VCORM on the root object type (master04.5 §Validity Rules: `C_OBJECT`).
    if !root_type.is_empty() && !rm.type_exists(root_type) {
        push_issue(
            &mut scan.issues,
            ValidationCode::Vcorm,
            format!(
                "root object type {root_type:?} is not defined in the reference model ({})",
                rm.name()
            ),
            "/",
        );
    }
    scan.walk_complex("", root_type, v.definition);
    match dialect {
        Dialect::Adl2 => scan.issues,
        Dialect::Adl14 => scan
            .issues
            .into_iter()
            .filter(|i| i.code == ValidationCode::Vunt)
            .collect(),
    }
}

/// Mutable state threaded through the reference-model walk.
struct RmScan<'a> {
    rm: &'a dyn RmModel,
    /// The definition root, against which a `use_node` target path resolves (VUNT).
    root: &'a CComplexObject,
    /// The codes defined in scope (interior VATID); a constituent's below its root.
    defined: BTreeSet<&'a str>,
    components: super::scope::Components<'a>,
    is_specialised: bool,
    issues: Vec<ValidationIssue>,
}

impl RmScan<'_> {
    /// Walk a complex `child` in the terminology scope it opens ([`super::scope::enter`]).
    fn walk_child(&mut self, path: &str, rm_type: &str, child: &CObject) {
        if let CObject::CComplexObject(cco) = child {
            let outer = super::scope::enter(child, self.components, &mut self.defined);
            self.walk_complex(path, rm_type, cco);
            super::scope::leave(outer, &mut self.defined);
        }
    }

    /// Walk a complex object whose RM type is `rm_type`, checking every
    /// attribute and its child objects against the reference model.
    fn walk_complex(&mut self, path: &str, rm_type: &str, cco: &CComplexObject) {
        for attr in complex_attributes(cco) {
            // A differential-path attribute is checked at its resolved location by the
            // specialisation walk (master04.5 §C_ATTRIBUTE, VDIFP).
            if attr.differential_path.is_some() {
                continue;
            }
            let attr_path = format!("{path}/{}", attr.rm_attribute_name);
            let rm_attr = if rm_type.is_empty() {
                None
            } else {
                self.rm.attribute(rm_type, &attr.rm_attribute_name)
            };

            match rm_attr {
                None => {
                    // VCARM (master04.5 §Validity Rules: `C_ATTRIBUTE`); an unknown enclosing type
                    // is already VCORM above.
                    if !rm_type.is_empty() && self.rm.type_exists(rm_type) {
                        let name = &attr.rm_attribute_name;
                        let mut message = format!(
                            "attribute {name:?} is not defined on reference-model type {rm_type:?}"
                        );
                        if is_quantity_property(rm_type, name) {
                            message = format!("{message}: {ADL2_REFUSAL_ADVICE}");
                        }
                        push_issue(&mut self.issues, ValidationCode::Vcarm, message, &attr_path);
                    }
                    // The declared type is unknown, so no VCORMT/VCAEX/etc for
                    // this attribute; still descend for VCORM on the subtree.
                    self.walk_children_untyped(&attr_path, attr);
                }
                Some(rm_attr) => {
                    self.check_attribute(&attr_path, attr, &rm_attr);
                    self.walk_children(&attr_path, attr, &rm_attr);
                }
            }
        }
    }

    /// The attribute-level RM checks (VCAEX / VCAM / VCACA / VACSO).
    fn check_attribute(&mut self, attr_path: &str, attr: &CAttribute, rm_attr: &RmAttr) {
        self.check_existence_conformance(attr_path, attr, rm_attr);
        self.check_arity(attr_path, attr, rm_attr);
        self.check_cardinality_conformance(attr_path, attr, rm_attr);
        if !rm_attr.is_multiple {
            self.check_single_valued_child_occurrences(attr_path, attr);
        }
    }

    /// VCAEX: existence, if set, must conform to (be same-or-narrower than) the
    /// RM existence (master04.5 §Validity Rules: `C_ATTRIBUTE`).
    fn check_existence_conformance(
        &mut self,
        attr_path: &str,
        attr: &CAttribute,
        rm_attr: &RmAttr,
    ) {
        // NOTE: ADL 1.4 master05 §existence states a {1..1} default when
        // unstated — applied literally it fails published CKM content, so an
        // unstated existence deliberately defers to the RM (reported upstream).
        let Some(ex) = attr.existence.as_ref() else {
            return;
        };
        let arch = bounds(ex);
        if !rm_attr.existence.contains(arch) {
            push_issue(
                &mut self.issues,
                ValidationCode::Vcaex,
                format!(
                    "existence {} does not conform to the reference-model existence {}",
                    display_bounds(arch),
                    display_bounds(rm_attr.existence)
                ),
                attr_path,
            );
        }
    }

    /// VCAM: a cardinality on a single-valued RM attribute (master04.5 §Validity
    /// Rules: `C_ATTRIBUTE`).
    fn check_arity(&mut self, attr_path: &str, attr: &CAttribute, rm_attr: &RmAttr) {
        if attr.cardinality.is_some() && !rm_attr.is_multiple {
            push_issue(
                &mut self.issues,
                ValidationCode::Vcam,
                "a cardinality is stated but the reference-model attribute is single-valued",
                attr_path,
            );
        }
    }

    /// VCACA: cardinality must conform to the RM container cardinality
    /// (master04.5 §Validity Rules: `C_ATTRIBUTE`).
    fn check_cardinality_conformance(
        &mut self,
        attr_path: &str,
        attr: &CAttribute,
        rm_attr: &RmAttr,
    ) {
        if let (Some(card), true) = (attr.cardinality.as_ref(), rm_attr.is_multiple)
            && let Some(rm_card) = rm_attr.cardinality
        {
            let arch = bounds(&card.interval);
            if !rm_card.contains(arch) {
                push_issue(
                    &mut self.issues,
                    ValidationCode::Vcaca,
                    format!(
                        "cardinality {} does not conform to the reference-model cardinality {}",
                        display_bounds(arch),
                        display_bounds(rm_card)
                    ),
                    attr_path,
                );
            }
        }
    }

    /// VACSO: a child of a single-valued attribute cannot have occurrences above 1
    /// (master04.5 §Validity Rules: `C_ATTRIBUTE`), the RM deciding single-valued.
    fn check_single_valued_child_occurrences(&mut self, attr_path: &str, attr: &CAttribute) {
        for child in attr.children.iter().flatten() {
            if let Some(occ) = child_occurrences(child)
                && let Some(upper) = finite_upper(occ)
                && upper > 1
            {
                push_issue(
                    &mut self.issues,
                    ValidationCode::Vacso,
                    format!("child of single-valued attribute has occurrences upper {upper} > 1"),
                    attr_path,
                );
            }
        }
    }

    /// Walk the children of an attribute whose RM declared type is known,
    /// checking each child's type existence/conformance and recursing.
    fn walk_children(&mut self, attr_path: &str, attr: &CAttribute, rm_attr: &RmAttr) {
        for child in attr.children.iter().flatten() {
            let child_type = object_rm_type(child);
            let child_path = child_path(attr_path, object_node_id(child));

            if !child_type.is_empty() {
                self.check_child_type(&child_path, child_type, &rm_attr.declared_type);
            }

            // VCORMEN / VCORMENV / VCORMENU: a primitive constraint on an
            // enumeration-typed RM slot must use the enumeration's declared
            // literal values (master08 §Phase 2; master04.2 §Constraints on
            // Enumeration Types).
            if let Some(en) = self.rm.enumeration(&rm_attr.declared_type) {
                self.check_enumeration(&child_path, child, &en);
            }

            if rm_attr.is_multiple && !self.is_specialised {
                self.check_interior_node_id(&child_path, child);
            }

            if let CObject::CComplexObjectProxy(proxy) = child {
                self.check_proxy_type(&child_path, proxy);
            }

            self.walk_child(&child_path, child_type, child);
        }
    }

    /// VCORM / VCORMT: a child's type must exist in the RM and conform to the owning
    /// attribute's declared type (master04.5 §Validity Rules: `C_OBJECT`; master04.2
    /// §`Rm_type_name`).
    fn check_child_type(&mut self, child_path: &str, child_type: &str, declared_type: &str) {
        if !self.rm.type_exists(child_type) {
            push_issue(
                &mut self.issues,
                ValidationCode::Vcorm,
                format!(
                    "object type {child_type:?} is not defined in the reference model ({})",
                    self.rm.name()
                ),
                child_path,
            );
            return;
        }
        if type_conforms(self.rm, child_type, declared_type) == Some(false) {
            push_issue(
                &mut self.issues,
                ValidationCode::Vcormt,
                format!(
                    "object type {child_type:?} does not conform to the attribute's reference-model type {declared_type:?}"
                ),
                child_path,
            );
        }
    }

    /// Interior-node VATID: a node under a multiply-valued attribute must have its
    /// id-code defined in the archetype's own terminology (master07 §Overview).
    fn check_interior_node_id(&mut self, child_path: &str, child: &CObject) {
        let nid = object_node_id(child);
        if (AdlCodeDefinitionsData::is_id_code(nid) || AdlCodeDefinitionsData::is_at_code(nid))
            && !self.defined.contains(nid)
        {
            push_issue(
                &mut self.issues,
                ValidationCode::Vatid,
                format!("node id {nid:?} is not defined in the terminology"),
                child_path,
            );
        }
    }

    /// VUNT: the type named in a `use_node` must be the referenced node's RM type
    /// or a super-type of it (`AOM2/master04.5` §Validity Rules:
    /// `C_COMPLEX_OBJECT_PROXY`; `ADL1.4/master05-cadl.adoc` §Internal References).
    fn check_proxy_type(&mut self, path: &str, proxy: &CComplexObjectProxy) {
        let declared = normalise_type_name(&proxy.rm_type_name);
        if declared.is_empty() {
            return; // no declared type ⇒ nothing to compare (1.4 accepts none).
        }
        let Some(target) = locate(self.root, &proxy.target_path) else {
            return; // unresolvable target path ⇒ VUNP (phase 3), not VUNT.
        };
        let target_type = object_rm_type(target);
        if target_type.is_empty() {
            return;
        }
        // `conforms(sub, sup)`: the REFERENCED type must conform to the DECLARED
        // one (declared is the same type or an ancestor). `None` = a type unknown
        // to the model, already reported as VCORM — undecidable here.
        if type_conforms(self.rm, target_type, &proxy.rm_type_name) == Some(false) {
            push_issue(
                &mut self.issues,
                ValidationCode::Vunt,
                format!(
                    "use_node type {:?} is neither the same as nor a super-type of the referenced \
                     node's type {target_type:?} at {:?}",
                    proxy.rm_type_name, proxy.target_path
                ),
                path,
            );
        }
    }

    /// Walk the children of an attribute whose declared type is unknown (its
    /// name was not defined in the RM — VCARM). No conformance can be judged, so
    /// only VCORM (type existence) is checked, and the subtree is recursed with
    /// each child's own RM type.
    fn walk_children_untyped(&mut self, attr_path: &str, attr: &CAttribute) {
        for child in attr.children.iter().flatten() {
            let child_type = object_rm_type(child);
            let child_path = child_path(attr_path, object_node_id(child));
            if !child_type.is_empty() && !self.rm.type_exists(child_type) {
                push_issue(
                    &mut self.issues,
                    ValidationCode::Vcorm,
                    format!(
                        "object type {child_type:?} is not defined in the reference model ({})",
                        self.rm.name()
                    ),
                    &child_path,
                );
            }
            // VUNT does not depend on the owning attribute's declared type, so it
            // runs on this branch too.
            if let CObject::CComplexObjectProxy(proxy) = child {
                self.check_proxy_type(&child_path, proxy);
            }
            self.walk_child(&child_path, child_type, child);
        }
    }

    /// Validate a primitive constraint against an enumeration-typed RM slot:
    /// VCORMEN for a primitive-kind mismatch, VCORMENV/VCORMENU for an out-of-set
    /// integer or string value.
    ///
    /// NOTE: `master08` §Phase 2 glosses these codes in one line; the partition is
    /// our reading of `master04.2` §Constraints on Enumeration Types.
    fn check_enumeration(&mut self, path: &str, child: &CObject, en: &RmEnum) {
        match child {
            CObject::CInteger(c) => {
                if en.underlying == EnumUnderlying::String {
                    push_issue(
                        &mut self.issues,
                        ValidationCode::Vcormen,
                        "an integer constraint is stated on a string-based enumeration slot",
                        path,
                    );
                    return;
                }
                for v in integer_point_values(c.constraint.as_deref().unwrap_or_default()) {
                    if !en.int_values.contains(&i64::from(v)) {
                        push_issue(
                            &mut self.issues,
                            ValidationCode::Vcormenv,
                            format!(
                                "integer value {v} is not a declared literal of the enumeration"
                            ),
                            path,
                        );
                    }
                }
            }
            CObject::CString(c) => {
                if en.underlying == EnumUnderlying::Integer {
                    push_issue(
                        &mut self.issues,
                        ValidationCode::Vcormen,
                        "a string constraint is stated on an integer-based enumeration slot",
                        path,
                    );
                    return;
                }
                for v in string_literal_values(c.constraint.as_deref().unwrap_or_default()) {
                    if !en.str_values.iter().any(|lit| lit == v) {
                        push_issue(
                            &mut self.issues,
                            ValidationCode::Vcormenu,
                            format!(
                                "string value {v:?} is not a declared literal of the enumeration"
                            ),
                            path,
                        );
                    }
                }
            }
            // Any other primitive kind cannot constrain an enumeration
            // (VCORMEN); complex objects / proxies / slots / terminology codes
            // are not primitive enumeration constraints and are left to the
            // other RM checks.
            CObject::CBoolean(_)
            | CObject::CReal(_)
            | CObject::CDate(_)
            | CObject::CTime(_)
            | CObject::CDateTime(_)
            | CObject::CDuration(_) => push_issue(
                &mut self.issues,
                ValidationCode::Vcormen,
                "a non-integer/string primitive constraint is stated on an enumeration slot",
                path,
            ),
            CObject::CComplexObject(_)
            | CObject::CComplexObjectProxy(_)
            | CObject::ArchetypeSlot(_)
            | CObject::CTerminologyCode(_) => {}
        }
    }
}

/// The point values of a `C_INTEGER` enumeration constraint; a range is not
/// enumerated, so it raises no VCORMENV (`master04.2` §Constraints on
/// Enumeration Types).
fn integer_point_values(constraint: &[Interval<i32>]) -> Vec<i32> {
    constraint.iter().filter_map(point_value_i32).collect()
}

/// The literal string values of a `C_STRING` enumeration constraint: the plain
/// entries, excluding regex forms (`/re/` or `^re^`), which cannot be a single
/// enumeration literal (`master04.2` §Constraints on Enumeration Types).
fn string_literal_values(constraint: &[String]) -> impl Iterator<Item = &str> {
    constraint
        .iter()
        .map(String::as_str)
        .filter(|s| !is_delimited_regex_trimmed(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assemble::parse_artefact;

    #[test]
    fn base_type_and_generics() {
        assert_eq!(base_type_name("HISTORY<ITEM_LIST>"), "HISTORY");
        assert_eq!(base_type_name("Interval <Quantity>"), "Interval");
        assert_eq!(base_type_name("OBSERVATION"), "OBSERVATION");
        assert_eq!(generic_arguments("HISTORY<ITEM_LIST>"), vec!["ITEM_LIST"]);
        assert_eq!(
            generic_arguments("Map<String, List<Item>>"),
            vec!["String", "List<Item>"]
        );
        assert_eq!(generic_arguments("OBSERVATION"), Vec::<String>::new());
        assert_eq!(
            normalise_type_name("Interval <Quantity>"),
            "INTERVAL<QUANTITY>"
        );
    }

    /// A `use_node` archetype built around `body`, for the VUNT pair below.
    fn cluster_with_proxy(proxy: &str) -> String {
        format!(
            "archetype (adl_version=2.0.5; rm_release=1.0.2)\n\
             \topenEHR-EHR-CLUSTER.vunt.v1.0.0\n\n\
             language\n\toriginal_language = <[ISO_639-1::en]>\n\n\
             description\n\tlifecycle_state = <\"draft\">\n\n\
             definition\n\
             \tCLUSTER[id1] matches {{\n\
             \t\titems matches {{\n\
             \t\t\tELEMENT[id2] matches {{*}}\n\
             \t\t\t{proxy}\n\
             \t\t}}\n\
             \t}}\n\n\
             terminology\n\tterm_definitions = <\n\
             \t\t[\"en\"] = <\n\
             \t\t\t[\"id1\"] = <text=<\"\"> description=<\"\">>\n\
             \t\t\t[\"id2\"] = <text=<\"\"> description=<\"\">>\n\
             \t\t\t[\"id5\"] = <text=<\"\"> description=<\"\">>\n\
             \t\t>\n\
             \t>\n"
        )
    }

    fn rm_codes(src: &str) -> Vec<ValidationCode> {
        let a = parse_artefact(src, Dialect::Adl2).expect("the fixture must parse");
        validate_rm_conformance(&a, &ProductionRmModel, Dialect::Adl2)
            .into_iter()
            .map(|i| i.code)
            .collect()
    }

    /// VUNT: `ADL1.4/master05-cadl.adoc` §Internal References L510-513 (same rule
    /// as `AOM2/master04.5` §`C_COMPLEX_OBJECT_PROXY` VUNT) — the `use_node` type
    /// must be the same as, or a super-type of, the referenced node's type.
    /// `CLUSTER` is a SIBLING of the referenced `ELEMENT`, not an ancestor.
    #[test]
    fn vunt_use_node_type_is_not_a_supertype_of_the_target() {
        let codes = rm_codes(&cluster_with_proxy("use_node CLUSTER[id5] /items[id2]"));
        assert!(
            codes.contains(&ValidationCode::Vunt),
            "expected VUNT, got {:?}",
            codes.iter().map(|c| c.mnemonic()).collect::<Vec<_>>()
        );
    }

    /// The accepting twin: master05 L510-513 blesses naming an ANCESTOR type ("a
    /// `use_node` reference to such a node can legally mention the parent type"), so
    /// `ITEM` over an `ELEMENT` target is clean — a check that only compared type
    /// names for equality would false-reject this.
    #[test]
    fn vunt_use_node_may_name_a_supertype_of_the_target() {
        for proxy in [
            "use_node ITEM[id5] /items[id2]",
            "use_node ELEMENT[id5] /items[id2]",
        ] {
            let codes = rm_codes(&cluster_with_proxy(proxy));
            assert!(
                !codes.contains(&ValidationCode::Vunt),
                "{proxy}: expected no VUNT, got {:?}",
                codes.iter().map(|c| c.mnemonic()).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn production_model_selection() {
        let ehr = parse_artefact(
            "archetype (adl_version=2.0.5; rm_release=1.0.2)\n\
             \topenEHR-EHR-OBSERVATION.x.v1.0.0\n\n\
             language\n\toriginal_language = <[ISO_639-1::en]>\n\n\
             description\n\tlifecycle_state = <\"draft\">\n\n\
             definition\n\tOBSERVATION[id1] matches {*}\n\n\
             terminology\n\tterm_definitions = <[\"en\"] = <[\"id1\"] = <text=<\"\"> description=<\"\">>>>\n",
            Dialect::Adl2,
        )
        .unwrap();
        assert!(production_model_governs(&ehr));
    }
}
