// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Path analysis + typing against the generated RM model
//! (`openehr_rm::v1_2::model`) — the AQL planner's oracle.
//!
//! The central operation is the **path split**: every identified path deterministically splits into
//!
//! 1. a sequence of **structure hops** — attribute steps whose resolved node is
//!    a structure root (its own `node` row per [`openehr_rm::v1_2::model::is_structure_root`],
//!    mirrored from `ferroehr::storage::codec`), ending at the deepest node-row
//!    anchor; and
//! 2. a **fragment suffix** — the first non-structure step onward, addressed
//!    inside the anchor node's `data` JSONB.
//!
//! The split is unambiguous because the node codec prunes *all* structure-typed
//! children out of a node's fragment: once a step resolves to a non-structure
//! type, no structure root can appear below it. Along the way the analyzer
//! resolves the candidate leaf [`TypeSet`] (abstract slots expanded to concrete
//! descendants), the multiplicity (any list/set step ⇒ multi-valued), and the
//! typed [`Coercion`].

use openehr_query::ast::{
    ArchetypePredicate,
    IdentifiedPath,
    NodeNameConstraint,
    NodePredicate,
    ObjectPath,
    PathPredicate,
    PathPredicateOperand,
    Primitive,
    StandardPredicate,
    // PathPart intentionally not imported: parts are walked via ObjectPath.
};
use openehr_query::lexer::CompOp;
use openehr_rm::v1_2::model;
use std::collections::HashMap;

use super::error::{AnalysisError, AqlError, AqlFeatureError};
use super::ir::{
    ArchetypeConstraint, Bind, Coercion, EhrField, FragmentStep, LeafPath, NameConstraint,
    NodeConstraint, PathTarget, SourceId, StdPredicate, StructureStep, TypeSet, TypedLit,
    VersionField, VersionMetaPredicate,
};

/// The variable → source bindings gathered from the FROM clause. Built by
/// [`super::lower`]; consumed here to resolve identified-path roots.
#[derive(Debug, Default)]
pub(crate) struct Bindings {
    vars: HashMap<String, Binding>,
    /// The ACTIVE `spec_profile` — path resolution refuses classes/attributes
    /// the selected released generation does not define (a conformant server
    /// of that generation would answer "unknown", and answering rows instead
    /// would silently overclaim the profile).
    pub(crate) profile: crate::config::profile::SpecProfile,
}

/// One FROM variable binding.
#[derive(Debug, Clone)]
pub(crate) struct Binding {
    pub source: SourceId,
    pub kind: BindingKind,
}

/// What a bound variable resolves to.
#[derive(Debug, Clone)]
pub(crate) enum BindingKind {
    /// An `EHR` variable.
    Ehr,
    /// An RM structure-class variable, carrying its concrete type set.
    Rm(TypeSet),
    /// A `VERSION` variable.
    Version,
}

impl Bindings {
    /// Variable names are not case-sensitive (QUERY master03
    /// §Variables/Syntax) — bindings key on the case-folded name.
    pub(crate) fn insert(&mut self, var: &str, binding: Binding) {
        self.vars.insert(var.to_ascii_lowercase(), binding);
    }

    pub(crate) fn contains(&self, var: &str) -> bool {
        self.vars.contains_key(&var.to_ascii_lowercase())
    }

    fn get(&self, var: &str) -> Option<&Binding> {
        self.vars.get(&var.to_ascii_lowercase())
    }
}

/// Resolve an identified path to a typed [`PathTarget`].
pub(crate) fn analyze_path(
    path: &IdentifiedPath,
    bindings: &Bindings,
) -> Result<PathTarget, AqlError> {
    let binding = bindings
        .get(&path.root)
        .ok_or_else(|| AnalysisError::UnknownVariable(path.root.clone()))?;
    let source = binding.source;
    match &binding.kind {
        BindingKind::Ehr => analyze_ehr_path(source, path, bindings.profile),
        BindingKind::Version => Ok(PathTarget::Version {
            source,
            field: resolve_version_field(path.path.as_ref())?,
        }),
        BindingKind::Rm(types) => {
            let root_predicate = path
                .predicate
                .as_ref()
                .map(resolve_node_predicate)
                .transpose()?;
            let leaf = analyze_rm_path(
                source,
                types,
                root_predicate,
                path.path.as_ref(),
                bindings.profile,
            )?;
            Ok(PathTarget::Data(Box::new(leaf)))
        }
    }
}

/// The RM-path split: walk `object_path` from `root_types`, classifying each
/// step as a structure hop or a fragment step.
fn analyze_rm_path(
    source: SourceId,
    root_types: &TypeSet,
    root_predicate: Option<NodeConstraint>,
    object_path: Option<&ObjectPath>,
    profile: crate::config::profile::SpecProfile,
) -> Result<LeafPath, AqlError> {
    let mut anchor: Vec<StructureStep> = Vec::new();
    let mut fragment: Vec<FragmentStep> = Vec::new();
    let mut in_fragment = false;
    let mut current = root_types.clone();
    // The candidate types of the *parent* of the final step, and the final
    // attribute name — used to coerce a `.../value` leaf under a temporal DV
    // parent to `Temporal` (its `value` is an ISO-8601 String; QUERY §Built-in
    // Types/Dates and Times).
    let mut parent_types = current.clone();
    let mut last_name: Option<String> = None;

    if let Some(op) = object_path {
        for part in &op.parts {
            parent_types = current.clone();
            last_name = Some(part.name.clone());
            let (step_types, is_multi) = resolve_attribute(&current, &part.name, profile)?;
            let predicate = part
                .predicate
                .as_ref()
                .map(resolve_node_predicate)
                .transpose()?;

            let is_hop = !in_fragment
                && !step_types.is_empty()
                && step_types
                    .names()
                    .iter()
                    .all(|t| model::is_structure_root(t));

            if is_hop {
                anchor.push(StructureStep {
                    attribute: part.name.clone(),
                    node_types: step_types.clone(),
                    predicate,
                    multi_valued: is_multi,
                });
            } else {
                in_fragment = true;
                fragment.push(FragmentStep {
                    name: part.name.clone(),
                    predicate,
                    multi_valued: is_multi,
                });
            }
            current = step_types;
        }
    }

    let coercion = if last_name.as_deref() == Some("value") && all_temporal(&parent_types) {
        Coercion::Temporal
    } else {
        classify(&current)
    };
    Ok(LeafPath {
        source,
        root_predicate,
        anchor,
        fragment,
        types: current,
        coercion,
    })
}

/// Resolve `attr` across every concrete type in `current`, returning the union
/// of concrete declared types and whether the attribute is a container on any
/// of them.
fn resolve_attribute(
    current: &TypeSet,
    attr: &str,
    profile: crate::config::profile::SpecProfile,
) -> Result<(TypeSet, bool), AqlError> {
    let mut resolved: Vec<String> = Vec::new();
    let mut multi = false;
    let mut any = false;
    let mut any_in_profile = false;
    for ty in current.names() {
        if let Some(a) = model::attribute(ty, attr) {
            any = true;
            if profile_defines_attribute(profile, ty, attr) {
                any_in_profile = true;
            }
            if matches!(a.container, model::Container::List | model::Container::Set) {
                multi = true;
            }
            resolved.extend(expand_type(a.declared_type));
        }
    }
    if !any {
        return Err(AnalysisError::UnresolvableAttribute {
            attribute: attr.to_owned(),
            on: describe_types(current),
        }
        .into());
    }
    // The attribute resolves in the development model but the ACTIVE released
    // generation defines it on none of the candidate classes: a conformant
    // server of that generation answers "unknown attribute", so this one does
    // too — planning it against the superset model would silently overclaim
    // the profile.
    if !any_in_profile {
        return Err(AnalysisError::AttributeNotInProfile {
            attribute: attr.to_owned(),
            on: describe_types(current),
            profile: profile.as_str(),
            generation: profile.rm().spec_version(),
        }
        .into());
    }
    Ok((TypeSet::new(resolved), multi))
}

/// Does the ACTIVE profile's RM generation define `attr` on `ty` (or on any
/// ancestor `ty` inherits it from)?
///
/// Only booleans cross this seam — the planner keeps working over the
/// current (superset) model's types, so no cross-generation type ever leaks.
fn profile_defines_attribute(
    profile: crate::config::profile::SpecProfile,
    ty: &str,
    attr: &str,
) -> bool {
    match profile {
        crate::config::profile::SpecProfile::Development => true,
        crate::config::profile::SpecProfile::Stable => {
            openehr_rm::v1_1::model::attribute(ty, attr).is_some()
        }
    }
}

/// Does the ACTIVE profile's RM generation define class `name`?
pub(crate) fn profile_defines_class(
    profile: crate::config::profile::SpecProfile,
    name: &str,
) -> bool {
    match profile {
        crate::config::profile::SpecProfile::Development => true,
        crate::config::profile::SpecProfile::Stable => {
            openehr_rm::v1_1::model::class(name).is_some()
        }
    }
}

/// Expand a declared type name to its concrete descendant set. A BASE
/// foundation type (a primitive, `Any`, an interval or ISO 8601 value) or a
/// name outside the model is kept verbatim as a singleton: it is a value
/// type, not substitutable RM content.
fn expand_type(name: &str) -> Vec<String> {
    if model::class(name).is_some() && !model::is_foundation_type(name) {
        model::descendants(name)
            .iter()
            .map(|s| (*s).to_owned())
            .collect()
    } else {
        vec![name.to_owned()]
    }
}

fn describe_types(types: &TypeSet) -> String {
    match types.names() {
        [only] => only.clone(),
        names => format!("any of {{{}}}", names.join(", ")),
    }
}

/// The leaf-value category used to pick a [`Coercion`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cat {
    Num,
    Temporal,
    Bool,
    Text,
}

/// Classify a single type name into a leaf category, or `None` for
/// unknown/structural types.
fn categorize(name: &str) -> Option<Cat> {
    match name {
        "Integer" | "Integer64" | "Real" | "Double" | "DV_QUANTITY" | "DV_COUNT" | "DV_ORDINAL"
        | "DV_SCALE" | "DV_PROPORTION" | "DV_AMOUNT" | "DV_QUANTIFIED" => Some(Cat::Num),
        "DV_DATE" | "DV_TIME" | "DV_DATE_TIME" | "DV_DURATION" | "DV_TEMPORAL" => {
            Some(Cat::Temporal)
        }
        "Boolean" | "DV_BOOLEAN" => Some(Cat::Bool),
        "String" | "DV_TEXT" | "DV_CODED_TEXT" | "DV_IDENTIFIER" | "DV_URI" | "DV_EHR_URI"
        | "DV_PARSABLE" | "CODE_PHRASE" | "TERM_MAPPING" => Some(Cat::Text),
        _ => None,
    }
}

/// Decide the [`Coercion`] for a candidate leaf type set: a single uniform
/// category maps to its coercion; anything mixed or unknown is
/// [`Coercion::Raw`] (a guarded runtime dispatch — never a silent wrong-type
/// comparison; design §Coercion table).
/// Whether every type in the set is a temporal DV type (a non-empty set of
/// `DV_DATE`/`DV_TIME`/`DV_DATE_TIME`/`DV_DURATION`/`DV_TEMPORAL`).
fn all_temporal(types: &TypeSet) -> bool {
    !types.is_empty()
        && types
            .names()
            .iter()
            .all(|t| categorize(t) == Some(Cat::Temporal))
}

fn classify(types: &TypeSet) -> Coercion {
    if types.is_empty() {
        return Coercion::Raw;
    }
    let mut cat: Option<Cat> = None;
    for name in types.names() {
        match categorize(name) {
            Some(c) if cat.is_none() => cat = Some(c),
            Some(c) if cat == Some(c) => {}
            _ => return Coercion::Raw,
        }
    }
    match cat {
        Some(Cat::Num) => Coercion::Magnitude,
        Some(Cat::Temporal) => Coercion::Temporal,
        Some(Cat::Bool) => Coercion::Boolean,
        Some(Cat::Text) => Coercion::Text,
        None => Coercion::Raw,
    }
}

// ── EHR / VERSION field resolution ───────────────────────────────────────────

/// Resolve an identified path rooted at an `EHR` variable. `e/ehr_status[/...]`
/// addresses the EHR's current `EHR_STATUS` versioned object (a *separate* VO —
/// RM 1.2.0 `EHR.ehr_status`), which is not a node on the EHR; the residual path
/// below `ehr_status` is analysed against `EHR_STATUS` and carried as a
/// [`PathTarget::EhrStatus`] the SQL package resolves via an engine-level join.
/// Every other EHR attribute (`ehr_id`, `time_created`, `system_id`) is a scalar
/// field resolved directly.
fn analyze_ehr_path(
    source: SourceId,
    path: &IdentifiedPath,
    profile: crate::config::profile::SpecProfile,
) -> Result<PathTarget, AqlError> {
    let heads_ehr_status = path
        .path
        .as_ref()
        .and_then(|op| op.parts.first())
        .is_some_and(|p| p.name == "ehr_status");
    if !heads_ehr_status {
        return Ok(PathTarget::Ehr {
            source,
            field: resolve_ehr_field(path.path.as_ref())?,
        });
    }
    // `EHR_STATUS` is a singleton VO per EHR, not a filterable node set, so a
    // predicate on the EHR variable (`e[...]/ehr_status`) or on `ehr_status`
    // itself (`e/ehr_status[...]`) is a typed reject (never a wrong result).
    if path.predicate.is_some() {
        return Err(AqlFeatureError::UnsupportedEhrStatusPath(
            "predicate on the EHR variable".to_owned(),
        )
        .into());
    }
    let Some(op) = path.path.as_ref() else {
        // Unreachable in practice: this branch is entered only for
        // `e/ehr_status…` heads, which always carry a path; reject typed
        // rather than panic.
        return Err(AqlFeatureError::UnsupportedEhrStatusPath(
            "missing ehr_status path".to_owned(),
        )
        .into());
    };
    if op
        .parts
        .first()
        .is_some_and(|part| part.predicate.is_some())
    {
        return Err(AqlFeatureError::UnsupportedEhrStatusPath(
            "predicate on ehr_status".to_owned(),
        )
        .into());
    }
    // The residual path below `ehr_status`, analysed against EHR_STATUS. Empty ⇒
    // the whole EHR_STATUS object.
    let mut rest = op.clone();
    rest.parts.remove(0);
    let rest_ref = if rest.parts.is_empty() {
        None
    } else {
        Some(&rest)
    };
    let ehr_status = TypeSet::new(vec!["EHR_STATUS".to_owned()]);
    let leaf = analyze_rm_path(source, &ehr_status, None, rest_ref, profile)?;
    Ok(PathTarget::EhrStatus(Box::new(leaf)))
}

fn resolve_ehr_field(object_path: Option<&ObjectPath>) -> Result<EhrField, AqlError> {
    let Some(op) = object_path else {
        return Ok(EhrField::Whole);
    };
    let head = op.parts.first().map(|p| p.name.as_str());
    match head {
        Some("ehr_id") => Ok(EhrField::EhrId),
        Some("system_id") => Ok(EhrField::SystemId),
        Some("time_created") => Ok(EhrField::TimeCreated),
        Some(other) => Err(AnalysisError::UnresolvableAttribute {
            attribute: other.to_owned(),
            on: "EHR".to_owned(),
        }
        .into()),
        None => Ok(EhrField::Whole),
    }
}

fn resolve_version_field(object_path: Option<&ObjectPath>) -> Result<VersionField, AqlError> {
    let Some(op) = object_path else {
        // A bare VERSION variable addresses its uid.
        return Ok(VersionField::Uid);
    };
    let parts: Vec<&str> = op.parts.iter().map(|p| p.name.as_str()).collect();
    version_field_from_parts(&parts)
}

fn version_field_from_parts(parts: &[&str]) -> Result<VersionField, AqlError> {
    match parts {
        ["uid", ..] => Ok(VersionField::Uid),
        // The two CODED version fields are sub-path-sensitive: the stored
        // representation is the numeric group code, the rubric renders from the
        // openEHR terminology group, and the terminology id is the constant
        // `openehr` — a flat mapping would silently never match.
        ["lifecycle_state", rest @ ..] => coded_version_field(
            rest,
            parts,
            VersionField::LifecycleState,
            VersionField::LifecycleStateRubric,
            VersionField::LifecycleStateTerminology,
        ),
        ["contribution", ..] => Ok(VersionField::ContributionId),
        ["commit_audit", rest @ ..] => match rest {
            ["time_committed", ..] => Ok(VersionField::TimeCommitted),
            ["system_id", ..] => Ok(VersionField::SystemId),
            ["change_type", rest2 @ ..] => coded_version_field(
                rest2,
                parts,
                VersionField::ChangeType,
                VersionField::ChangeTypeRubric,
                VersionField::ChangeTypeTerminology,
            ),
            ["committer", ..] => Ok(VersionField::Committer),
            // AUDIT_DETAILS.description is a DV_TEXT whose DV_CODED_TEXT
            // subtype carries a defining_code (`audit_details.adoc`
            // §Attributes), so each addressed representation resolves to its own
            // extraction.
            ["description", rest @ ..] => match rest {
                [] => Ok(VersionField::Description),
                ["value"] => Ok(VersionField::DescriptionValue),
                ["defining_code", "code_string"] => Ok(VersionField::DescriptionCode),
                ["defining_code", "terminology_id", "value"] => {
                    Ok(VersionField::DescriptionTerminology)
                }
                _ => Err(AqlFeatureError::UnsupportedVersionPredicate(parts.join("/")).into()),
            },
            _ => Err(AqlFeatureError::UnsupportedVersionPredicate(parts.join("/")).into()),
        },
        _ => Err(AqlFeatureError::UnsupportedVersionPredicate(parts.join("/")).into()),
    }
}

/// Resolve the sub-path of a coded (`DV_CODED_TEXT`) version field to the
/// representation it addresses; any other suffix — including the bare coded
/// object, which has no defined scalar comparison form — is a typed reject.
fn coded_version_field(
    suffix: &[&str],
    full: &[&str],
    code: VersionField,
    rubric: VersionField,
    terminology: VersionField,
) -> Result<VersionField, AqlError> {
    match suffix {
        ["defining_code", "code_string"] => Ok(code),
        ["value"] => Ok(rubric),
        ["defining_code", "terminology_id", "value"] => Ok(terminology),
        _ => Err(AqlFeatureError::UnsupportedVersionPredicate(full.join("/")).into()),
    }
}

/// Resolve a version standard predicate (`VERSION v[commit_audit/... op val]`).
/// Rejects branch (non-trunk) version ids explicitly.
pub(crate) fn resolve_version_predicate(
    sp: &StandardPredicate,
) -> Result<VersionMetaPredicate, AqlError> {
    let parts: Vec<&str> = sp.path.parts.iter().map(|p| p.name.as_str()).collect();
    let field = version_field_from_parts(&parts)?;
    let value = bind_from_operand(&sp.operand)?;
    if field == VersionField::Uid
        && let Bind::Literal(TypedLit::String(s)) = &value
        && is_branch_version_id(s)
    {
        return Err(AqlFeatureError::BranchVersionAddressing.into());
    }
    Ok(VersionMetaPredicate {
        field,
        op: sp.op,
        value,
    })
}

/// Whether an `OBJECT_VERSION_ID` string names a branch version. The format is
/// `object_id::creating_system_id::version_tree_id`; a trunk `version_tree_id` is
/// a plain integer, while a branch id contains dots (`1.1.1`).
fn is_branch_version_id(s: &str) -> bool {
    s.split("::")
        .nth(2)
        .is_some_and(|tree_id| tree_id.contains('.'))
}

// ── predicate resolution ─────────────────────────────────────────────────────

/// Resolve an EHR class-predicate `[ehr_id/value=$id]` to its field + value.
pub(crate) fn resolve_ehr_predicate(
    sp: &StandardPredicate,
) -> Result<(EhrField, CompOp, Bind), AqlError> {
    let parts: Vec<&str> = sp.path.parts.iter().map(|p| p.name.as_str()).collect();
    let field = match parts.as_slice() {
        ["ehr_id", ..] => EhrField::EhrId,
        ["system_id", ..] => EhrField::SystemId,
        ["time_created", ..] => EhrField::TimeCreated,
        _ => {
            return Err(AnalysisError::UnresolvableAttribute {
                attribute: parts.join("/"),
                on: "EHR".to_owned(),
            }
            .into());
        }
    };
    Ok((field, sp.op, bind_from_operand(&sp.operand)?))
}

/// Resolve an AST [`PathPredicate`] into a typed [`NodeConstraint`].
pub(crate) fn resolve_node_predicate(pred: &PathPredicate) -> Result<NodeConstraint, AqlError> {
    let mut out = NodeConstraint::default();
    apply_predicate(pred, &mut out)?;
    Ok(out)
}

fn apply_predicate(pred: &PathPredicate, out: &mut NodeConstraint) -> Result<(), AqlError> {
    match pred {
        PathPredicate::Standard(sp) => apply_standard(sp, out),
        PathPredicate::Archetype(ap) => {
            out.archetype = Some(match ap {
                ArchetypePredicate::Hrid(h) => ArchetypeConstraint::Archetype(h.clone()),
                ArchetypePredicate::Parameter(p) => ArchetypeConstraint::Param(param_name(p)),
            });
            Ok(())
        }
        PathPredicate::Node(np) => apply_node(np, out),
    }
}

fn apply_standard(sp: &StandardPredicate, out: &mut NodeConstraint) -> Result<(), AqlError> {
    let parts: Vec<String> = sp.path.parts.iter().map(|p| p.name.clone()).collect();
    let value = bind_from_operand(&sp.operand)?;
    // `name/value = <text|param>` is the common name shortcut.
    if sp.op == CompOp::Eq && parts == ["name", "value"] {
        out.name = Some(match value {
            Bind::Literal(TypedLit::String(s)) => NameConstraint::Value(s),
            Bind::Param(p) => NameConstraint::Param(p),
            other @ Bind::Literal(_) => {
                return Err(AnalysisError::TypeMismatch(format!(
                    "name/value must be a string or parameter, got {other:?}"
                ))
                .into());
            }
        });
        return Ok(());
    }
    // `archetype_node_id = <code|hrid>` is the standard-predicate form of the
    // archetype/node shortcut predicates (master03 §Archetype predicate). Which
    // of the two the operand is comes from the RM's own reading of
    // `LOCATABLE.archetype_node_id` (`openehr_rm::v1_2::paths`), never a leader
    // guess.
    if sp.op == CompOp::Eq
        && parts == ["archetype_node_id"]
        && let Bind::Literal(TypedLit::String(s)) = &value
    {
        out.archetype = Some(if openehr_rm::v1_2::paths::is_archetype_root_node_id(s) {
            ArchetypeConstraint::Archetype(s.clone())
        } else if openehr_rm::v1_2::paths::archetype_node_id_is_term_code(s) {
            ArchetypeConstraint::NodeCode(s.clone())
        } else {
            return Err(AnalysisError::MalformedArchetypeNodeId(s.clone()).into());
        });
        return Ok(());
    }
    out.standard.push(StdPredicate {
        path: parts,
        op: sp.op,
        value,
    });
    Ok(())
}

fn apply_node(np: &NodePredicate, out: &mut NodeConstraint) -> Result<(), AqlError> {
    match np {
        NodePredicate::Code { code, name } => {
            out.archetype = Some(ArchetypeConstraint::NodeCode(code.clone()));
            if let Some(n) = name {
                out.name = Some(resolve_node_name(n));
            }
            Ok(())
        }
        NodePredicate::Archetype { hrid, name } => {
            out.archetype = Some(ArchetypeConstraint::Archetype(hrid.clone()));
            if let Some(n) = name {
                out.name = Some(resolve_node_name(n));
            }
            Ok(())
        }
        NodePredicate::Parameter(p) => {
            out.archetype = Some(ArchetypeConstraint::Param(param_name(p)));
            Ok(())
        }
        NodePredicate::Standard(sp) => apply_standard(sp, out),
        NodePredicate::MatchesRegex { .. } => Err(AqlFeatureError::RegexNodePredicate.into()),
        NodePredicate::And(a, b) => {
            apply_node(a, out)?;
            apply_node(b, out)
        }
        NodePredicate::Or(_, _) => Err(AqlFeatureError::OrNodePredicate.into()),
    }
}

fn resolve_node_name(n: &NodeNameConstraint) -> NameConstraint {
    match n {
        NodeNameConstraint::String(s) => NameConstraint::Value(s.clone()),
        NodeNameConstraint::Parameter(p) => NameConstraint::Param(param_name(p)),
        NodeNameConstraint::TermCode(c) => parse_name_term_code(c),
        // A bare at/id-code as the name operand is a term from the archetype's
        // own terminology — the canonical expansion asserts
        // `terminology_id/value = 'local'` (QUERY master03 §Node predicate).
        NodeNameConstraint::Code(c) => NameConstraint::TermCode {
            terminology: "local".to_owned(),
            code: c.clone(),
        },
    }
}

/// Decompose a `terminology::code|value|` name term-code lexeme into its
/// matching parts (QUERY master03 §Node predicate): the part before `::` is
/// the terminology id (an optional `(version)` suffix stays part of it), the
/// part after is the code string, and a trailing `|value|` is informational
/// only — it takes no part in matching and is dropped.
fn parse_name_term_code(raw: &str) -> NameConstraint {
    let (terminology, rest) = match raw.split_once("::") {
        Some((t, r)) => (t, r),
        // The lexer guarantees `::` is present; keep a lossless fallback.
        None => ("local", raw),
    };
    let code = match rest.split_once('|') {
        Some((c, _informational)) => c,
        None => rest,
    };
    NameConstraint::TermCode {
        terminology: terminology.to_owned(),
        code: code.to_owned(),
    }
}

/// Convert a path-predicate operand to a [`Bind`]. A path operand (rare in a
/// standard predicate RHS) is rejected as a type mismatch — the SQL package
/// handles path-vs-path comparisons only in the WHERE clause, not in predicates.
fn bind_from_operand(operand: &PathPredicateOperand) -> Result<Bind, AqlError> {
    match operand {
        PathPredicateOperand::Primitive(p) => Ok(Bind::Literal(typed_lit(p))),
        PathPredicateOperand::Parameter(p) => Ok(Bind::Param(param_name(p))),
        PathPredicateOperand::Code(c) => Ok(Bind::Literal(TypedLit::String(c.clone()))),
        PathPredicateOperand::Path(op) => Err(AnalysisError::TypeMismatch(format!(
            "path operand `{}` is not supported in a predicate right-hand side",
            op.parts
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>()
                .join("/")
        ))
        .into()),
    }
}

/// Normalize an AST parameter token to its bare name (the lexer keeps the
/// leading `$`; the IR and [`super::ir::Params`] key on the name without it).
pub(crate) fn param_name(raw: &str) -> String {
    raw.strip_prefix('$').unwrap_or(raw).to_owned()
}

/// Map an AST [`Primitive`] to a typed literal (temporals stay strings until a
/// comparison context retypes them; QUERY §Built-in Types/Dates and Times).
pub(crate) fn typed_lit(p: &Primitive) -> TypedLit {
    match p {
        Primitive::String(s) => TypedLit::String(s.clone()),
        Primitive::Integer(i) => TypedLit::Integer(*i),
        Primitive::Real(r) => TypedLit::Real(*r),
        Primitive::Boolean(b) => TypedLit::Boolean(*b),
        Primitive::Null => TypedLit::Null,
    }
}
