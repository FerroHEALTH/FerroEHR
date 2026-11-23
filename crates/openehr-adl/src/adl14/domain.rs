// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! ADL 1.4 inline dADL domain lowering (converter front end).
//!
//! NOTE: no openEHR spec governs the 1.4→2 conversion: our own design (see
//! [`crate::adl14`]).
//!
//! `C_DV_QUANTITY` and `C_DV_ORDINAL` lower to a `C_COMPLEX_OBJECT` with an
//! attribute tuple (`AOM2/master04.4` §Second-Order Constraints), `C_CODE_PHRASE`
//! to a terminology-code constraint; [`crate::adl14::convert`] renumbers.

use openehr_am::v2_4::aom2::constraint_model::c_attribute::CAttribute;
use openehr_am::v2_4::aom2::constraint_model::c_attribute_tuple::CAttributeTuple;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::aom2::constraint_model::c_primitive_object::CPrimitiveObject;
use openehr_am::v2_4::aom2::constraint_model::c_primitive_tuple::CPrimitiveTuple;
use openehr_base::prelude::{Interval, ProperInterval, ProperIntervalData};
use openehr_lang::v1_1::odin::OdinValue;

use crate::aom::build::{
    ObjectNode, cattr_empty, cattr_single, complex_object, point_int, point_real,
    primitive_to_cobject,
};
use crate::aom::interval::{
    int_interval_contains, point_value_f64, point_value_i32, real_interval_contains,
};
use crate::codes::NodeCoding;
use crate::odin::{odin_kind, untyped};

pub(crate) fn is_adl14_domain_type(id: &str) -> bool {
    matches!(id, "C_DV_QUANTITY" | "C_DV_ORDINAL" | "C_CODE_PHRASE")
}

/// The parts an inline dADL `C_CODE_PHRASE` block constrains, as the 1.4
/// custom-syntax spelling `[terminology:: code, … ; assumed]` carries them.
pub(crate) struct CodePhraseParts {
    /// The `terminology_id`'s value (`"local"`, `"SNOMED-CT"`, …).
    pub(crate) terminology: String,
    /// The `code_list` members, in source order.
    pub(crate) codes: Vec<String>,
    /// The `assumed_value` `CODE_PHRASE`'s `code_string`, if the block has one.
    pub(crate) assumed: Option<String>,
}

/// Read an inline dADL `C_CODE_PHRASE` block into the parts of a
/// `C_TERMINOLOGY_CODE` constraint string.
///
/// `ADL1.4/master09-customising_adl.adoc` §Custom Syntax: the block and the
/// `[local:: at0039, at0040]` syntax "express exactly the same constraint"
/// (`AOM1.4/masterAppA-domain_extension.adoc` §`C_CODED_TEXT`).
///
/// # Errors
/// The human-readable defect when the block is not a `C_CODE_PHRASE` instance —
/// a missing/unreadable `terminology_id`, an absent or empty `code_list`, an
/// `assumed_value` that is not a `CODE_PHRASE` of the same terminology, or an
/// attribute the class does not define. The caller raises it as `SDINV`.
pub(crate) fn adl14_code_phrase_parts(odin: &OdinValue) -> Result<CodePhraseParts, String> {
    let OdinValue::Object(map) = untyped(odin) else {
        return Err("expecting an object with 'terminology_id' and 'code_list'".to_owned());
    };
    if let Some(unknown) = map
        .keys()
        .find(|k| !matches!(k.as_str(), "terminology_id" | "code_list" | "assumed_value"))
    {
        return Err(format!(
            "unknown attribute {unknown:?} (expecting 'terminology_id', 'code_list' or \
             'assumed_value')"
        ));
    }
    let Some(terminology) = map.get("terminology_id").map(untyped) else {
        return Err("missing 'terminology_id'".to_owned());
    };
    let terminology = terminology_id_value(terminology)
        .ok_or_else(|| "'terminology_id' is not a terminology identifier".to_owned())?;
    let Some(code_list) = map.get("code_list") else {
        // NOTE: `masterAppA-domain_extension.adoc` §C_CODED_TEXT makes `code_list`
        // optional, but an empty code set would narrow the constraint to nothing.
        return Err("missing 'code_list'".to_owned());
    };
    let codes = code_list_codes(code_list)?;
    if codes.is_empty() {
        return Err("'code_list' constrains no code".to_owned());
    }
    let assumed = match map.get("assumed_value").map(untyped) {
        None => None,
        Some(value) => {
            let (assumed_terminology, code) = code_phrase_instance(value)?;
            if assumed_terminology != terminology {
                return Err(format!(
                    "'assumed_value' names terminology {assumed_terminology:?}, but the \
                     constraint is on {terminology:?}"
                ));
            }
            Some(code)
        }
    };
    Ok(CodePhraseParts {
        terminology,
        codes,
        assumed,
    })
}

/// A `TERMINOLOGY_ID` dADL value → its `value` string, in either the nested
/// object spelling of `ADL1.4/master09-customising_adl.adoc` §Custom Syntax or
/// the plain-string spelling of `AOM1.4/masterAppA-domain_extension.adoc`
/// §`C_CODED_TEXT` (`terminology: String`).
fn terminology_id_value(v: &OdinValue) -> Option<String> {
    match v {
        OdinValue::String(s) => Some(s.clone()),
        OdinValue::Object(map) => match map.get("value").map(untyped) {
            Some(OdinValue::String(s)) => Some(s.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// A `code_list` dADL value → its code strings, keyed, list or lone string
/// (`LANG/docs/odin/master05-content.adoc` §Container Objects).
///
/// # Errors
/// The defect when a member is not a code string.
fn code_list_codes(v: &OdinValue) -> Result<Vec<String>, String> {
    let member = |m: &OdinValue| match untyped(m) {
        OdinValue::String(s) => Ok(s.clone()),
        other => Err(format!(
            "'code_list' member is not a code string ({})",
            odin_kind(other)
        )),
    };
    match untyped(v) {
        OdinValue::KeyedList(entries) => entries.iter().map(|(_, m)| member(m)).collect(),
        OdinValue::List(items) => items.iter().map(member).collect(),
        OdinValue::String(s) => Ok(vec![s.clone()]),
        OdinValue::Empty => Ok(Vec::new()),
        other => Err(format!("'code_list' is not a list ({})", odin_kind(other))),
    }
}

/// A `CODE_PHRASE` dADL instance → its `(terminology, code_string)`.
///
/// # Errors
/// The defect when the value is not a `CODE_PHRASE` instance.
fn code_phrase_instance(v: &OdinValue) -> Result<(String, String), String> {
    let OdinValue::Object(map) = untyped(v) else {
        return Err("'assumed_value' is not a CODE_PHRASE object".to_owned());
    };
    let terminology = map
        .get("terminology_id")
        .map(untyped)
        .and_then(terminology_id_value)
        .ok_or_else(|| "'assumed_value' has no readable 'terminology_id'".to_owned())?;
    let Some(OdinValue::String(code)) = map.get("code_string").map(untyped) else {
        return Err("'assumed_value' has no 'code_string'".to_owned());
    };
    Ok((terminology, code.clone()))
}

/// Why an inline dADL domain block could not be lowered (each maps to `SDINV`).
pub(crate) enum DomainLoweringError {
    /// An empty `<>` block, a bare scalar, or a type this lowering does not model.
    Empty,
    /// The block's `assumed_value` satisfies none of its `list` rows (the
    /// attribute names carried for the message).
    AssumedValueUnmatched(String),
}

/// One constrained-member-set partition of a domain block's `list` rows —
/// the constraints of one sibling alternative.
struct Partition {
    /// The partition's per-attribute plain constraints.
    attributes: Vec<CAttribute>,
    /// The partition's attribute tuple (co-constrained members).
    attribute_tuples: Vec<CAttributeTuple>,
}

/// Partition a domain block's `list` rows by the exact set of member names each
/// row constrains.
///
/// ADL 2 tuple rows constrain every member (`ADL2/master04.4` §Tuple
/// Constraints), so each partition becomes its own alternative: one member
/// merges into a plain constraint, several into a tuple. Order is first
/// appearance.
///
/// # Errors
/// [`DomainLoweringError::Empty`] when a row value the primitive lowering
/// cannot model is met — the whole block refuses rather than dropping a
/// constraint.
fn partition_list_rows(list: &OdinValue) -> Result<Vec<Partition>, DomainLoweringError> {
    let rows = domain_list_rows(list);
    group_rows_by_member_set(&rows)
        .into_iter()
        .map(|(names, group)| partition_of_group(&names, &group))
        .collect()
}

/// One `list` row: its member (name, value) pairs in source order.
type DomainRow = Vec<(String, OdinValue)>;

/// Groups list rows by their member-name SET, in first-seen order.
fn group_rows_by_member_set(rows: &[DomainRow]) -> Vec<(Vec<String>, Vec<&DomainRow>)> {
    let mut groups: Vec<(Vec<String>, Vec<&DomainRow>)> = Vec::new();
    for row in rows {
        let mut names: Vec<String> = Vec::new();
        for (k, _) in row {
            if !names.iter().any(|n| n == k) {
                names.push(k.clone());
            }
        }
        match groups.iter_mut().find(|(g, _)| same_member_set(g, &names)) {
            Some((_, members)) => members.push(row),
            None => groups.push((names, vec![row])),
        }
    }
    groups
}

/// Whether two member-name lists denote the same set.
fn same_member_set(a: &[String], b: &[String]) -> bool {
    a.len() == b.len() && a.iter().all(|n| b.iter().any(|m| m == n))
}

/// The alternative one co-varying row group contributes: a tuple, a merged
/// constraint, or the shared prefix alone for an empty row set.
///
/// # Errors
/// Propagates [`build_attribute_tuple`]'s refusal for a tuple group.
fn partition_of_group(
    names: &[String],
    rows: &[&DomainRow],
) -> Result<Partition, DomainLoweringError> {
    let mut attributes: Vec<CAttribute> = Vec::new();
    let mut attribute_tuples: Vec<CAttributeTuple> = Vec::new();
    if names.len() >= 2 {
        attribute_tuples.push(build_attribute_tuple(names, rows)?);
    } else if let Some(name) = names.first() {
        let values: Vec<CPrimitiveObject> = rows
            .iter()
            .filter_map(|row| row.iter().find(|(k, _)| k == name))
            .filter_map(|(_, v)| domain_value_to_primitive(name, v))
            .collect();
        if let Some(merged) = merge_primitives(values) {
            attributes.push(cattr_single(name, primitive_to_cobject(merged)));
        }
    }
    Ok(Partition {
        attributes,
        attribute_tuples,
    })
}

/// The `C_ATTRIBUTE_TUPLE` of one co-varying partition, one `C_PRIMITIVE_TUPLE`
/// per row.
///
/// # Errors
/// [`DomainLoweringError::Empty`] for a row value the primitive lowering cannot
/// model, or a row matching no member (`AOM2/master04.5` §`C_PRIMITIVE_TUPLE`).
fn build_attribute_tuple(
    names: &[String],
    rows: &[&Vec<(String, OdinValue)>],
) -> Result<CAttributeTuple, DomainLoweringError> {
    let members: Vec<CAttribute> = names.iter().map(|n| cattr_empty(n)).collect();
    let mut tuples: Vec<CPrimitiveTuple> = Vec::new();
    for row in rows {
        let mut prim_members = Vec::new();
        for n in names {
            let Some(v) = row
                .iter()
                .find(|(k, _)| k == n)
                .and_then(|(_, v)| domain_value_to_primitive(n, v))
            else {
                return Err(DomainLoweringError::Empty);
            };
            prim_members.push(v);
        }
        let Ok(prim_members) = openehr_base::containers::NonEmptyVec::new(prim_members) else {
            return Err(DomainLoweringError::Empty);
        };
        tuples.push(CPrimitiveTuple {
            members: prim_members,
        });
    }
    Ok(CAttributeTuple {
        members: openehr_base::containers::present(members),
        tuples: openehr_base::containers::present(tuples),
    })
}

/// Lower a parsed 1.4 inline dADL domain block into one
/// `DV_QUANTITY`/`DV_ORDINAL` alternative per partition of its `list` rows.
///
/// # Errors
/// [`DomainLoweringError`] for an empty/unusable block or an `assumed_value` that
/// matches no `list` row; the caller turns both into `SDINV`.
pub(crate) fn lower_adl14_domain(
    rm_type: &str,
    odin: &OdinValue,
) -> Result<Vec<CObject>, DomainLoweringError> {
    let map = match untyped(odin) {
        OdinValue::Object(map) if !map.is_empty() => map,
        // An empty block constrains only the type (`ADL1.4/master05-cadl.adoc` §Symbols
        // `V_C_DOMAIN_TYPE`).
        OdinValue::Empty | OdinValue::Object(_) => {
            let target_rm = match rm_type {
                "C_DV_ORDINAL" => "DV_ORDINAL",
                "C_DV_QUANTITY" => "DV_QUANTITY",
                _ => return Err(DomainLoweringError::Empty),
            };
            return Ok(vec![complex_object(
                target_rm.to_owned(),
                String::new(),
                Vec::new(),
                Vec::new(),
                None,
            )]);
        }
        // A bare scalar payload is not an empty section — nothing to lower.
        _ => return Err(DomainLoweringError::Empty),
    };
    let target_rm = match rm_type {
        "C_DV_ORDINAL" => "DV_ORDINAL",
        "C_DV_QUANTITY" => "DV_QUANTITY",
        // Unreachable: the parse site gates the type before calling in. Kept as a
        // typed refusal rather than a fallback so no other domain constrainer can
        // ever be silently lowered to the wrong RM type.
        _ => return Err(DomainLoweringError::Empty),
    };
    let mut attributes: Vec<CAttribute> = Vec::new();

    // `property = <[openehr::122]>` → a `property` at-code constraint (the
    // external code is rewritten to a synthesised at-code + binding by the
    // converter).
    if let Some(OdinValue::TermCode(tc)) = map.get("property").map(untyped) {
        let constraint = tc.trim_start_matches('[').trim_end_matches(']').to_owned();
        attributes.push(cattr_single(
            "property",
            CObject::CTerminologyCode(
                ObjectNode::inline("Terminology_code", NodeCoding::At)
                    .c_terminology_code(constraint, None, None),
            ),
        ));
    }

    // `list` rows partition into one alternative per constrained member set
    // — see [`partition_list_rows`].
    let mut partitions = match map.get("list") {
        Some(list) => partition_list_rows(list)?,
        None => Vec::new(),
    };
    if partitions.is_empty() {
        // No `list` (or an empty one): a single alternative carrying only the
        // row-independent constraints — the pre-partition shape.
        partitions.push(Partition {
            attributes: Vec::new(),
            attribute_tuples: Vec::new(),
        });
    }

    // Assemble one sibling alternative per partition: the row-independent
    // prefix (`property`, …) is row-independent, so every alternative carries
    // its own copy.
    let mut alternatives: Vec<(Vec<CAttribute>, Vec<CAttributeTuple>)> = partitions
        .into_iter()
        .map(|p| {
            let mut attrs = attributes.clone();
            attrs.extend(p.attributes);
            (attrs, p.attribute_tuples)
        })
        .collect();

    // The 1.4 `assumed_value` is an RM instance (AOM 1.4 `C_DV_QUANTITY`); AOM2 puts
    // it on primitives (`AOM2/master04.2` §Assumed_value), so it decomposes onto
    // the one alternative that admits it.
    if let Some(OdinValue::Object(assumed)) = map.get("assumed_value").map(untyped) {
        let mut placed = None;
        let mut first_err = None;
        for (idx, (attrs, tuples)) in alternatives.iter().enumerate() {
            let mut try_attrs = attrs.clone();
            let mut try_tuples = tuples.clone();
            match apply_domain_assumed_values(assumed, &mut try_attrs, &mut try_tuples) {
                Ok(()) => {
                    placed = Some((idx, try_attrs, try_tuples));
                    break;
                }
                Err(e) => first_err = first_err.or(Some(e)),
            }
        }
        match placed {
            Some((idx, attrs, tuples)) => {
                let Some(slot) = alternatives.get_mut(idx) else {
                    // Unreachable: `idx` came from enumerating this very Vec.
                    return Err(DomainLoweringError::Empty);
                };
                *slot = (attrs, tuples);
            }
            None => {
                return Err(
                    first_err.unwrap_or(DomainLoweringError::AssumedValueUnmatched(String::new()))
                );
            }
        }
    }

    Ok(alternatives
        .into_iter()
        .map(|(attrs, tuples)| {
            complex_object(target_rm.to_owned(), String::new(), attrs, tuples, None)
        })
        .collect())
}

/// Land the leaves of a domain block's `assumed_value` on the constraints, a
/// tuple member on the one row the whole combination satisfies (`AOM2/master04.3`
/// §Tuple Constraints).
///
/// NOTE: a leaf for an unconstrained attribute has no AOM2 carrier
/// (`AOM2/master04.2` §`Assumed_value`) and is dropped: our own design.
///
/// # Errors
/// [`DomainLoweringError::AssumedValueUnmatched`] when no tuple row admits the
/// assumed combination.
fn apply_domain_assumed_values(
    assumed: &indexmap::IndexMap<String, OdinValue>,
    attributes: &mut [CAttribute],
    attribute_tuples: &mut [CAttributeTuple],
) -> Result<(), DomainLoweringError> {
    // The assumed leaves, as the primitive shape the constraint side uses.
    let leaves: Vec<(String, CPrimitiveObject)> = assumed
        .iter()
        .filter_map(|(name, value)| {
            domain_value_to_primitive(name, value).map(|p| (name.clone(), p))
        })
        .collect();
    if leaves.is_empty() {
        return Ok(());
    }

    // Plain attributes first.
    for (name, leaf) in &leaves {
        if let Some(attr) = attributes.iter_mut().find(|a| &a.rm_attribute_name == name)
            && let Some(child) = attr.children.as_mut().and_then(|c| c.first_mut())
        {
            set_assumed_on_cobject(child, leaf);
        }
    }

    // Tuple members: pick the single row the whole combination satisfies.
    for tuple in attribute_tuples.iter_mut() {
        let positions: Vec<(usize, &CPrimitiveObject)> = tuple
            .members
            .iter()
            .flatten()
            .enumerate()
            .filter_map(|(idx, m)| {
                leaves
                    .iter()
                    .find(|(name, _)| name == &m.rm_attribute_name)
                    .map(|(_, leaf)| (idx, leaf))
            })
            .collect();
        if positions.is_empty() {
            continue;
        }
        let row = tuple.tuples.iter().flatten().position(|row| {
            positions.iter().all(|(idx, leaf)| {
                row.members
                    .get(*idx)
                    .is_some_and(|constraint| primitive_admits(constraint, leaf))
            })
        });
        let Some(row) = row else {
            let named = positions
                .iter()
                .filter_map(|(idx, _)| tuple.members.as_ref().and_then(|m| m.get(*idx)))
                .map(|m| m.rm_attribute_name.clone())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(DomainLoweringError::AssumedValueUnmatched(named));
        };
        for (idx, leaf) in positions {
            if let Some(row) = tuple.tuples.as_mut().and_then(|t| t.get_mut(row))
                && let Some(member) = row.members.get_mut(idx)
            {
                set_assumed_on_primitive(member, leaf);
            }
        }
    }
    Ok(())
}

/// Whether `constraint` admits `value`; `false` only where non-membership is
/// positively decidable.
fn primitive_admits(constraint: &CPrimitiveObject, value: &CPrimitiveObject) -> bool {
    match (constraint, value) {
        (CPrimitiveObject::CString(c), CPrimitiveObject::CString(v)) => {
            c.constraint.as_ref().is_none_or(Vec::is_empty)
                || v.constraint
                    .iter()
                    .flatten()
                    .all(|want| c.constraint.iter().flatten().any(|have| have == want))
        }
        (CPrimitiveObject::CReal(c), CPrimitiveObject::CReal(v)) => {
            c.constraint.as_ref().is_none_or(Vec::is_empty)
                || v.constraint
                    .iter()
                    .flatten()
                    .filter_map(point_value_f64)
                    .all(|p| {
                        c.constraint
                            .iter()
                            .flatten()
                            .any(|iv| real_interval_contains(iv, p))
                    })
        }
        (CPrimitiveObject::CInteger(c), CPrimitiveObject::CInteger(v)) => {
            c.constraint.as_ref().is_none_or(Vec::is_empty)
                || v.constraint
                    .iter()
                    .flatten()
                    .filter_map(point_value_i32)
                    .all(|p| {
                        c.constraint
                            .iter()
                            .flatten()
                            .any(|iv| int_interval_contains(iv, p))
                    })
        }
        _ => true,
    }
}

/// Set `leaf`'s single value as the `assumed_value` of the primitive `target`.
fn set_assumed_on_primitive(target: &mut CPrimitiveObject, leaf: &CPrimitiveObject) {
    match (target, leaf) {
        (CPrimitiveObject::CString(t), CPrimitiveObject::CString(l)) => {
            t.assumed_value = l.constraint.iter().flatten().next().cloned();
        }
        (CPrimitiveObject::CReal(t), CPrimitiveObject::CReal(l)) => {
            t.assumed_value = l
                .constraint
                .iter()
                .flatten()
                .next()
                .and_then(point_value_f64);
        }
        (CPrimitiveObject::CInteger(t), CPrimitiveObject::CInteger(l)) => {
            t.assumed_value = l
                .constraint
                .iter()
                .flatten()
                .next()
                .and_then(point_value_i32)
                .map(f64::from);
        }
        (CPrimitiveObject::CBoolean(t), CPrimitiveObject::CBoolean(l)) => {
            t.assumed_value = l.constraint.iter().flatten().next().copied();
        }
        // Kind mismatch (or a leaf kind the domain lowering never produces):
        // leave the constraint untouched rather than coerce across types.
        _ => {}
    }
}

/// Set `leaf`'s single value as the `assumed_value` of the primitive object
/// `target` wraps, if it is one.
fn set_assumed_on_cobject(target: &mut CObject, leaf: &CPrimitiveObject) {
    match target {
        CObject::CString(t) => {
            if let CPrimitiveObject::CString(l) = leaf {
                t.assumed_value = l.constraint.iter().flatten().next().cloned();
            }
        }
        CObject::CReal(t) => {
            if let CPrimitiveObject::CReal(l) = leaf {
                t.assumed_value = l
                    .constraint
                    .iter()
                    .flatten()
                    .next()
                    .and_then(point_value_f64);
            }
        }
        CObject::CInteger(t) => {
            if let CPrimitiveObject::CInteger(l) = leaf {
                t.assumed_value = l
                    .constraint
                    .iter()
                    .flatten()
                    .next()
                    .and_then(point_value_i32)
                    .map(f64::from);
            }
        }
        CObject::CBoolean(t) => {
            if let CPrimitiveObject::CBoolean(l) = leaf {
                t.assumed_value = l.constraint.iter().flatten().next().copied();
            }
        }
        _ => {}
    }
}

/// The `["1"] = <…> …` rows of a domain `list`, each an ordered
/// `(attribute, value)` vec. The corpus always uses a keyed list; a bare object
/// is treated as a single row.
fn domain_list_rows(list: &OdinValue) -> Vec<Vec<(String, OdinValue)>> {
    let row_of = |m: &indexmap::IndexMap<String, OdinValue>| -> Vec<(String, OdinValue)> {
        m.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    };
    match untyped(list) {
        OdinValue::KeyedList(entries) => entries
            .iter()
            .filter_map(|(_, v)| match untyped(v) {
                OdinValue::Object(m) => Some(row_of(m)),
                _ => None,
            })
            .collect(),
        OdinValue::Object(m) => vec![row_of(m)],
        _ => Vec::new(),
    }
}

/// An ODIN leaf value → a `C_PRIMITIVE_OBJECT` for a domain attribute. The
/// attribute name disambiguates integer-vs-real intervals (`precision` is
/// integral, `magnitude` real).
fn domain_value_to_primitive(attr: &str, v: &OdinValue) -> Option<CPrimitiveObject> {
    match untyped(v) {
        OdinValue::String(s) => Some(CPrimitiveObject::CString(
            ObjectNode::inline("String", NodeCoding::At).c_string(vec![s.clone()], None),
        )),
        OdinValue::Integer(i) => Some(CPrimitiveObject::CInteger(
            ObjectNode::inline("Integer", NodeCoding::At).c_integer(vec![point_int(*i)], None),
        )),
        OdinValue::Real(r) => Some(CPrimitiveObject::CReal(
            ObjectNode::inline("Real", NodeCoding::At).c_real(vec![point_real(*r)], None),
        )),
        OdinValue::Interval(iv) => {
            if attr == "precision" {
                Some(CPrimitiveObject::CInteger(
                    ObjectNode::inline("Integer", NodeCoding::At)
                        .c_integer(vec![odin_interval_to_int(iv)], None),
                ))
            } else {
                Some(CPrimitiveObject::CReal(
                    ObjectNode::inline("Real", NodeCoding::At)
                        .c_real(vec![odin_interval_to_real(iv)], None),
                ))
            }
        }
        OdinValue::List(items) => {
            let mut merged: Vec<CPrimitiveObject> = Vec::new();
            for it in items {
                if let Some(p) = domain_value_to_primitive(attr, it) {
                    merged.push(p);
                }
            }
            merge_primitives(merged)
        }
        _ => None,
    }
}

/// Merge same-typed primitive constraints into a single object holding the
/// union of their value lists.
fn merge_primitives(mut items: Vec<CPrimitiveObject>) -> Option<CPrimitiveObject> {
    if items.is_empty() {
        return None;
    }
    if items.len() == 1 {
        return items.pop();
    }
    let mut strings: Vec<String> = Vec::new();
    let mut reals: Vec<Interval<f64>> = Vec::new();
    let mut ints: Vec<Interval<i32>> = Vec::new();
    let mut kind = 0u8;
    for it in items {
        match it {
            CPrimitiveObject::CString(c) => {
                kind = 1;
                strings.extend(c.constraint.into_iter().flatten());
            }
            CPrimitiveObject::CReal(c) => {
                kind = 2;
                reals.extend(c.constraint.into_iter().flatten());
            }
            CPrimitiveObject::CInteger(c) => {
                kind = 3;
                ints.extend(c.constraint.into_iter().flatten());
            }
            other => return Some(other),
        }
    }
    Some(match kind {
        1 => CPrimitiveObject::CString(
            ObjectNode::inline("String", NodeCoding::At).c_string(strings, None),
        ),
        2 => {
            CPrimitiveObject::CReal(ObjectNode::inline("Real", NodeCoding::At).c_real(reals, None))
        }
        _ => CPrimitiveObject::CInteger(
            ObjectNode::inline("Integer", NodeCoding::At).c_integer(ints, None),
        ),
    })
}

fn odin_interval_to_real(iv: &openehr_lang::v1_1::odin::OdinInterval) -> Interval<f64> {
    let (lower, li, upper, ui) = odin_range_bounds(iv, odin_as_real, |r| r);
    proper_or_point_real(lower, li, upper, ui)
}

fn odin_interval_to_int(iv: &openehr_lang::v1_1::odin::OdinInterval) -> Interval<i32> {
    let (lower, li, upper, ui) =
        odin_range_bounds(iv, |v| odin_as_real(v).map(real_to_i32), real_to_i32);
    if lower == upper && lower.is_some() {
        return point_int(i64::from(lower.unwrap_or_default()));
    }
    Interval::ProperInterval(ProperInterval::ProperInterval(ProperIntervalData {
        lower,
        upper,
        lower_unbounded: lower.is_none(),
        upper_unbounded: upper.is_none(),
        lower_included: li,
        upper_included: ui,
    }))
}

fn proper_or_point_real(
    lower: Option<f64>,
    li: bool,
    upper: Option<f64>,
    ui: bool,
) -> Interval<f64> {
    if lower == upper && lower.is_some() {
        return point_real(lower.unwrap_or_default());
    }
    Interval::ProperInterval(ProperInterval::ProperInterval(ProperIntervalData {
        lower,
        upper,
        lower_unbounded: lower.is_none(),
        upper_unbounded: upper.is_none(),
        lower_included: li,
        upper_included: ui,
    }))
}

/// The `(lower, lower_included, upper, upper_included)` of an ODIN interval,
/// each endpoint through `conv`.
///
/// `|N +/- M|` is the closed `[N-M, N+M]` (`ADL1.4/master04-dadl` §Intervals of
/// Ordered Primitive Types); a non-numeric centre yields an unbounded interval.
fn odin_range_bounds<T>(
    iv: &openehr_lang::v1_1::odin::OdinInterval,
    conv: impl Fn(&OdinValue) -> Option<T>,
    from_real: impl Fn(f64) -> T,
) -> (Option<T>, bool, Option<T>, bool) {
    match iv {
        openehr_lang::v1_1::odin::OdinInterval::Range {
            lower,
            lower_included,
            upper,
            upper_included,
        } => (
            lower.as_deref().and_then(&conv),
            *lower_included,
            upper.as_deref().and_then(&conv),
            *upper_included,
        ),
        openehr_lang::v1_1::odin::OdinInterval::PlusMinus { centre, delta } => {
            match (odin_as_real(centre), odin_as_real(delta)) {
                (Some(c), Some(d)) => (Some(from_real(c - d)), true, Some(from_real(c + d)), true),
                _ => (None, true, None, true),
            }
        }
    }
}

#[expect(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    reason = "archetype domain-constraint magnitudes are small integers; f64 represents them exactly"
)]
fn odin_as_real(v: &OdinValue) -> Option<f64> {
    match v {
        OdinValue::Real(r) => Some(*r),
        OdinValue::Integer(i) => Some(*i as f64),
        _ => None,
    }
}

#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    reason = "the value is clamped to the i32 range on the very next line, so the cast cannot truncate"
)]
fn real_to_i32(r: f64) -> i32 {
    r.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

#[cfg(test)]
mod tests {
    use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
    use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
    use openehr_am::v2_4::aom2::constraint_model::c_primitive_object::CPrimitiveObject;
    use openehr_base::prelude::{Interval, ProperInterval};

    use crate::error::SyntaxErrorCode;
    use crate::parse::{Dialect, parse_definition_body};

    /// An inline `|N +/-M|` magnitude lowers to the closed `[N-M, N+M]`
    /// (`ADL1.4/master04-dadl` §Intervals of Ordered Primitive Types).
    #[test]
    fn adl14_plus_minus_domain_interval_lowers_to_both_bounds() {
        let cco = parse_definition_body(
            "OBSERVATION[at0000] matches {\n\
             value matches {\n\
             C_DV_QUANTITY <\n\
             list = <\n\
             [\"1\"] = <\n\
             magnitude = <|5.0 +/-0.5|>\n\
             >\n\
             >\n\
             >\n\
             }\n\
             }",
            Dialect::Adl14,
        )
        .expect("the 1.4 inline domain block must parse");
        let CComplexObject::CComplexObject(d) = &cco else {
            panic!("expected a plain complex object root");
        };
        let CObject::CComplexObject(CComplexObject::CComplexObject(quantity)) =
            &d.attributes.as_deref().unwrap_or_default()[0]
                .children
                .as_deref()
                .unwrap_or_default()[0]
        else {
            panic!("expected the lowered DV_QUANTITY object");
        };
        let magnitude = quantity
            .attributes
            .iter()
            .flatten()
            .find(|a| a.rm_attribute_name == "magnitude")
            .expect("magnitude attribute");
        let CObject::CReal(real) = &magnitude.children.as_deref().unwrap_or_default()[0] else {
            panic!("expected a C_REAL magnitude constraint");
        };
        let [Interval::ProperInterval(ProperInterval::ProperInterval(range))] =
            real.constraint.as_deref().unwrap_or_default()
        else {
            panic!("expected one proper interval, got {:?}", real.constraint);
        };
        assert_eq!(range.lower, Some(4.5));
        assert_eq!(range.upper, Some(5.5));
        assert!(range.lower_included);
        assert!(range.upper_included);
    }

    /// A domain block's `assumed_value` binds to the leaf constraints of the one
    /// tuple row it satisfies (`AOM2/master04.2` §`Assumed_value`, `master04.3`).
    #[test]
    fn adl14_domain_assumed_value_lands_on_the_matching_tuple_row() {
        let cco = parse_definition_body(
            "ELEMENT[at0000] matches {\n\
             value matches {\n\
             C_DV_QUANTITY <\n\
             assumed_value = <units = <\"C\"> precision = <0> magnitude = <8.0>>\n\
             list = <\n\
             [\"1\"] = <units = <\"C\"> magnitude = <|>=4.0|>>\n\
             [\"2\"] = <units = <\"F\"> magnitude = <|>=40.0|>>\n\
             >\n\
             >\n\
             }\n\
             }",
            Dialect::Adl14,
        )
        .expect("the 1.4 inline domain block must parse");
        let CComplexObject::CComplexObject(d) = &cco else {
            panic!("expected a plain complex object root");
        };
        let CObject::CComplexObject(CComplexObject::CComplexObject(quantity)) =
            &d.attributes.as_deref().unwrap_or_default()[0]
                .children
                .as_deref()
                .unwrap_or_default()[0]
        else {
            panic!("expected the lowered DV_QUANTITY object");
        };
        let tuple = &quantity.attribute_tuples.as_deref().unwrap_or_default()[0];
        assert_eq!(
            tuple
                .members
                .iter()
                .flatten()
                .map(|m| m.rm_attribute_name.as_str())
                .collect::<Vec<_>>(),
            ["units", "magnitude"]
        );
        // Row 0 (`"C"`, >=4.0) admits the assumed combination; row 1 (`"F"`,
        // >=40.0) does not and must be left untouched.
        let CPrimitiveObject::CString(units0) =
            &tuple.tuples.as_deref().unwrap_or_default()[0].members[0]
        else {
            panic!("units is a string constraint");
        };
        assert_eq!(units0.assumed_value.as_deref(), Some("C"));
        let CPrimitiveObject::CReal(magnitude0) =
            &tuple.tuples.as_deref().unwrap_or_default()[0].members[1]
        else {
            panic!("magnitude is a real constraint");
        };
        assert_eq!(magnitude0.assumed_value, Some(8.0));
        let CPrimitiveObject::CString(units1) =
            &tuple.tuples.as_deref().unwrap_or_default()[1].members[0]
        else {
            panic!("units is a string constraint");
        };
        assert_eq!(units1.assumed_value, None);
    }

    /// A single-attribute domain block merges its rows into one plain constraint,
    /// so the `assumed_value` lands directly on that leaf's
    /// `C_PRIMITIVE_OBJECT.assumed_value` (`AOM2/master04.2` §`Assumed_value`).
    #[test]
    fn adl14_domain_assumed_value_lands_on_a_plain_attribute() {
        let cco = parse_definition_body(
            "ELEMENT[at0000] matches {\n\
             value matches {\n\
             C_DV_QUANTITY <\n\
             assumed_value = <units = <\"F\">>\n\
             list = <\n\
             [\"1\"] = <units = <\"C\">>\n\
             [\"2\"] = <units = <\"F\">>\n\
             >\n\
             >\n\
             }\n\
             }",
            Dialect::Adl14,
        )
        .expect("the 1.4 inline domain block must parse");
        let CComplexObject::CComplexObject(d) = &cco else {
            panic!("expected a plain complex object root");
        };
        let CObject::CComplexObject(CComplexObject::CComplexObject(quantity)) =
            &d.attributes.as_deref().unwrap_or_default()[0]
                .children
                .as_deref()
                .unwrap_or_default()[0]
        else {
            panic!("expected the lowered DV_QUANTITY object");
        };
        let units = quantity
            .attributes
            .iter()
            .flatten()
            .find(|a| a.rm_attribute_name == "units")
            .expect("units attribute");
        let CObject::CString(c) = &units.children.as_deref().unwrap_or_default()[0] else {
            panic!("expected a C_STRING units constraint");
        };
        assert_eq!(c.constraint, Some(vec!["C".to_owned(), "F".to_owned()]));
        assert_eq!(c.assumed_value.as_deref(), Some("F"));
    }

    /// An assumed value satisfying no `list` row is refused: the 1.4 source states
    /// an assumed instance outside its own constraint, and no tuple row can carry
    /// it (`AOM2/master04.3` §Tuple Constraints).
    #[test]
    fn adl14_domain_assumed_value_outside_every_row_is_refused() {
        let errs = parse_definition_body(
            "ELEMENT[at0000] matches {\n\
             value matches {\n\
             C_DV_QUANTITY <\n\
             assumed_value = <units = <\"kPa\"> magnitude = <8.0>>\n\
             list = <\n\
             [\"1\"] = <units = <\"mm[Hg]\"> magnitude = <|>=0.0|>>\n\
             [\"2\"] = <units = <\"cm[H2O]\"> magnitude = <|>=0.0|>>\n\
             >\n\
             >\n\
             }\n\
             }",
            Dialect::Adl14,
        )
        .expect_err("an unmatched assumed value must be refused");
        assert!(
            errs.iter().any(|e| e.code == SyntaxErrorCode::Sdinv),
            "expected SDINV, got {:?}",
            errs.iter().map(|e| e.code).collect::<Vec<_>>()
        );
    }

    /// The `C_CODE_PHRASE` block of `master09` §Custom Syntax lowers to the very
    /// constraint the compact custom syntax produces — the section presents them
    /// as two spellings that "express exactly the same constraint".
    #[test]
    fn adl14_code_phrase_block_lowers_like_the_custom_syntax() {
        let block = parse_definition_body(
            "ELEMENT[at0000] matches {\n\
             value matches {\n\
             DV_CODED_TEXT matches {\n\
             defining_code matches {\n\
             C_CODE_PHRASE <\n\
             terminology_id = <value = <\"local\">>\n\
             code_list = <[\"1\"] = <\"at0039\"> [\"2\"] = <\"at0040\">>\n\
             >\n\
             }\n\
             }\n\
             }\n\
             }",
            Dialect::Adl14,
        )
        .expect("the dADL block lowers");
        let custom = parse_definition_body(
            "ELEMENT[at0000] matches {\n\
             value matches {\n\
             DV_CODED_TEXT matches {\n\
             defining_code matches {\n\
             [local:: at0039, at0040]\n\
             }\n\
             }\n\
             }\n\
             }",
            Dialect::Adl14,
        )
        .expect("the custom syntax parses");
        assert_eq!(block, custom);
    }
}
