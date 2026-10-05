// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Reference-model conformance for the OPT 1.4 pass (T5, T7 + the RM checks).
//!
//! The AOM2 RM-conformance rules (`AOM2/master08-validation.adoc` lines 70–75,
//! `AOM2/master04.5-constraint_model-class_definitions.adoc`) require "a
//! computational representation of the reference model"; we use the
//! BMM-generated static RM model (`openehr_rm::v1_2::model`) — the same spec-pinned
//! oracle the AQL planner uses. Rules:
//!
//! - **VCORM** — an object-constraint type name must exist in the RM
//!   (`master04.5` line 325).
//! - **VCARM** — a constrained attribute name must exist on its RM type
//!   (`master08` line 126).
//! - **VCAM** — a container constraint may not sit on a single-valued RM
//!   attribute (`master08` line 132).
//! - **VCAEX** — an attribute's existence, if set, must not widen the RM's
//!   (`master08` line 129).
//! - **VCACA** — a container attribute's cardinality must be the same as, or
//!   narrower than, the RM's (`master08` line 74, `master04.5` line 162).
//! - **VACMCO / VCOC** — the mandatory children must fit the container
//!   cardinality (`master04.5` line 159, restating cADL VCOC
//!   `ADL1.4/master05-cadl.adoc` line 324).

use std::collections::BTreeSet;
use std::sync::LazyLock;

use openehr_its::opt14::types::{CAttribute, CObject, Cardinality, Intervalofinteger};
use openehr_rm::v1_2::model;

use super::interval::{iv_lower, iv_upper};
use super::{NodeView, RuleViolation};

/// LOCATABLE meta attributes tolerated on any RM class (see the NOTE in
/// [`check_attribute`]: the constraint binds to a serialized meta field the
/// canonical form of a PATHABLE-only node still carries) — the class's OWN
/// attribute set, read from the BMM-generated static
/// RM model, never a hand-kept list: a LOCATABLE attribute added or renamed by
/// a spec-pin bump follows automatically. PATHABLE's inherited members
/// (`parent`, which the flattened model also reports) are excluded, since the
/// tolerance exists precisely for classes the RM derives from PATHABLE alone.
static LOCATABLE_META_ATTRS: LazyLock<BTreeSet<&str>> = LazyLock::new(|| {
    let pathable: BTreeSet<&'static str> = model::attributes("PATHABLE").map(|a| a.name).collect();
    model::attributes("LOCATABLE")
        .map(|a| a.name)
        .filter(|name| !pathable.contains(name))
        .collect()
});

/// The `(class, attribute)` pairs tolerated for prior-art OPT compatibility:
/// only the ones the generated RM model cannot answer, each with the released
/// text that says why it is absent from the model. Everything derivable is
/// derived instead, an attribute the RM declares under the British orthography
/// being matched by [`is_us_orthography_of_rm_attribute`].
///
/// - `EVENT.offset` / `POINT_EVENT.offset` / `INTERVAL_EVENT.offset` — a
///   FUNCTION, not an attribute, in RM 1.2.0:
///   `UML/classes/org.openehr.rm.data_structures.event.adoc` lists `offset ():
///   DV_DURATION` under §Functions ("computed as time.diff(parent.origin)").
/// - `DV_PROPORTION.is_integral` — likewise a §Functions member
///   (`UML/classes/org.openehr.rm.data_types.dv_proportion.adoc`:
///   `is_integral (): Boolean`).
/// - `ITEM_TABLE.rotated` — declared by NO released RM class this pin carries
///   (`UML/classes/org.openehr.rm.data_structures.item_table.adoc` §Attributes
///   declares `rows` alone); an RM 1.0.x-era attribute that later releases
///   dropped.
///
/// The generated RM model carries classes and attributes only, so none of these
/// five resolves from it. All appear in widely-deployed OPT 1.4 artifacts (the
/// vendored RIPPLE, `clinical_content` and Better corpus templates), which the
/// AOM2 VCARM rule would otherwise refuse wholesale.
const LEGACY_RM_ATTRS: &[(&str, &str)] = &[
    ("EVENT", "offset"),
    ("POINT_EVENT", "offset"),
    ("INTERVAL_EVENT", "offset"),
    ("DV_PROPORTION", "is_integral"),
    ("ITEM_TABLE", "rotated"),
];

/// `true` when `attr_name` is the US orthography of an attribute `parent_rm`
/// really declares — `ELEMENT.null_flavor` for the RM's `null_flavour`
/// (`UML/classes/org.openehr.rm.data_structures.element.adoc` §Attributes),
/// the spelling archetype tooling emits.
///
/// Derived from the BMM-generated static RM model rather than hand-listed, so
/// the tolerance is exactly as wide as the model: if a pin bump renames or
/// removes the British-spelled attribute, the US spelling stops being tolerated
/// with it, and a new `-our` attribute is covered without an edit here.
fn is_us_orthography_of_rm_attribute(parent_rm: &str, attr_name: &str) -> bool {
    attr_name
        .strip_suffix("or")
        .is_some_and(|stem| model::attribute(parent_rm, &format!("{stem}our")).is_some())
}

// ─── VCORM (object constraint type existence) ───────────────────────────────────

/// VCORM: "object constraint type name existence: a type name introducing an
/// object constraint block must be defined in the underlying information model."
/// (`AOM2/master04.5-constraint_model-class_definitions.adoc` line 325.)
pub(super) fn check_object_type(rm_type: &str, node_id: &str) -> Result<(), RuleViolation> {
    // Strip any generic argument (`DV_INTERVAL<DV_QUANTITY>` → `DV_INTERVAL`);
    // the static model keys on the bare class name.
    let bare = rm_type.split('<').next().unwrap_or(rm_type).trim();
    if bare.is_empty() {
        return Err(RuleViolation::new(
            "VCORM",
            format!("object node '{node_id}' has an empty rm_type_name"),
        ));
    }
    if model::class(bare).is_none() {
        return Err(RuleViolation::new(
            "VCORM",
            format!(
                "type '{rm_type}' (object node '{node_id}') is not defined in the reference model"
            ),
        ));
    }
    Ok(())
}

// ─── VCARM / VCAM / VCAEX / VCACA (attribute conformance) ────────────────────────

/// The RM-conformance checks on a constrained attribute: VCARM (the attribute
/// exists on the RM type), and — once the RM attribute is resolved — VCAM
/// (multiplicity) and VCAEX (existence). Fires only when the enclosing object's
/// RM type is known to the static model; an unknown parent means we cannot
/// judge its attributes (and VCORM already flagged the parent if it was bogus).
pub(super) fn check_attribute(
    attr: &CAttribute,
    attr_name: &str,
    parent_rm: &str,
    existence: &Intervalofinteger,
) -> Result<(), RuleViolation> {
    if model::class(parent_rm).is_none() {
        return Ok(());
    }
    match model::attribute(parent_rm, attr_name) {
        // The shape is OBSERVED in archie/openEHR-SDK-generated OPTs, which
        // model every constrainable node as Locatable — prior art, not authority.
        // NOTE: a LOCATABLE meta attribute constrained on a PATHABLE-only class
        // (e.g. ISM_TRANSITION) binds to a field that node's canonical
        // serialization carries, so it has a referent and is not a VCARM breach.
        None if LOCATABLE_META_ATTRS.contains(attr_name)
            || is_us_orthography_of_rm_attribute(parent_rm, attr_name)
            || LEGACY_RM_ATTRS.contains(&(parent_rm, attr_name)) =>
        {
            Ok(())
        }
        None => Err(RuleViolation::new(
            // VCARM: attribute name reference model validity (AOM2 line 126).
            "VCARM",
            format!("attribute '{attr_name}' is not defined in reference-model type '{parent_rm}'"),
        )),
        Some(rm_attr) => rm_conformance(attr, attr_name, parent_rm, existence, rm_attr),
    }
}

/// The AOM2 RM-conformance rules that apply to a `C_ATTRIBUTE` once its RM
/// attribute has been resolved: VCAM (multiplicity) and VCAEX (existence).
fn rm_conformance(
    attr: &CAttribute,
    attr_name: &str,
    parent_rm: &str,
    existence: &Intervalofinteger,
    rm_attr: &model::RmAttribute,
) -> Result<(), RuleViolation> {
    let rm_is_multiple = !matches!(rm_attr.container, model::Container::None);

    // VCAM: "archetype attribute reference model multiplicity conformance: the
    // multiplicity … of an attribute must conform to that of the corresponding
    // attribute in the underlying information model." (line 132.) A container
    // (`C_MULTIPLE_ATTRIBUTE`) constraint on a single-valued RM attribute cannot
    // conform.
    let arch_is_multiple = matches!(attr, CAttribute::CMultipleAttribute(_));
    if arch_is_multiple && !rm_is_multiple {
        return Err(RuleViolation::new(
            "VCAM",
            format!(
                "attribute '{attr_name}' on '{parent_rm}' is constrained as a container \
                 (C_MULTIPLE_ATTRIBUTE) but is single-valued in the reference model"
            ),
        ));
    }

    // VCAEX: "archetype attribute reference model existence conformance: the
    // existence of an attribute, if set, must conform, i.e. be the same or
    // narrower, to the existence … in the underlying information model."
    // (line 129.) The RM existence upper bound is always 1; the RM lower bound
    // is 1 for a mandatory attribute and 0 otherwise. Allowing absence (`{0..}`)
    // on an RM-mandatory attribute *widens* it — the one enforceable violation.
    if rm_attr.is_mandatory && iv_lower(existence) == 0 && !existence.lower_unbounded {
        return Err(RuleViolation::new(
            "VCAEX",
            format!(
                "attribute '{attr_name}' on '{parent_rm}' has existence lower bound 0 but the \
                 attribute is mandatory (existence lower bound 1) in the reference model"
            ),
        ));
    }

    // VCACA: "archetype attribute reference model cardinality conformance: the
    // cardinality of an attribute must conform, i.e. be the same or narrower, to
    // the cardinality of the corresponding attribute in the underlying
    // information model." (line 162.)
    if let CAttribute::CMultipleAttribute(multiple) = attr {
        check_rm_cardinality(&multiple.cardinality, attr_name, parent_rm, rm_attr)?;
    }

    Ok(())
}

/// VCACA's numeric arm: the archetype cardinality interval must be CONTAINED in
/// the RM's declared container cardinality — "the same or narrower"
/// (`AOM2/master04.5-constraint_model-class_definitions.adoc` line 162).
///
/// The RM bounds come from the BMM-generated static model
/// ([`openehr_rm::v1_2::model::RmAttribute::cardinality`], the BMM `cardinality` of a
/// container attribute); an attribute the BMM leaves unconstrained has no RM
/// interval to conform to and is skipped. Containment is the ordinary interval
/// rule in both directions:
///
/// - the archetype LOWER bound may not fall below the RM's — e.g. `CLUSTER.items`
///   is `List<ITEM> [1..*]`
///   (`RM/docs/UML/classes/org.openehr.rm.data_structures.cluster.adoc`
///   §Attributes), so an OPT stating `items cardinality {0..*}` widens the RM by
///   admitting an empty CLUSTER the RM forbids;
/// - the archetype UPPER bound may not exceed the RM's (an unbounded archetype
///   upper against a finite RM upper is the widest case of that).
///
/// The fully-open `{0..*}` is read as "no cardinality override" rather than as
/// a widening — see the in-body citations.
///
/// NOTE: only the numeric part is judged; `ADL1.4/master05-cadl.adoc` line 268
/// hedges the `ordered`/`unordered`/`unique` half ("How such constraints are
/// evaluated in practice may depend somewhat on knowledge of the software
/// system"), and that hedge is about container semantics rather than the
/// membership range.
fn check_rm_cardinality(
    card: &Cardinality,
    attr_name: &str,
    parent_rm: &str,
    rm_attr: &model::RmAttribute,
) -> Result<(), RuleViolation> {
    let Some(rm_card) = rm_attr.cardinality else {
        return Ok(());
    };
    // The fully-open interval states NO cardinality override and defers to the
    // RM, so it can never widen it. Two spec facts force that reading of an OPT
    // 1.4 artefact: AOM2 types `C_ATTRIBUTE.cardinality [0..1]` and says of its
    // sibling "Only set if it overrides the underlying reference model or parent
    // archetype" (`c_attribute.adoc` §Attributes), while AOM 1.4 types it
    // MANDATORY (`c_multiple_attribute.adoc`); and cADL's §"'Any' Constraints"
    // fixes `{*}` as deference to the RM (`AM/docs/ADL1.4/master05-cadl.adoc`).
    // Every STATED interval (a finite bound on either side) is judged below.
    if iv_lower(&card.interval) == 0 && iv_upper(&card.interval).is_none() {
        return Ok(());
    }
    let arch_lower = iv_lower(&card.interval);
    let widens_lower = i64::from(arch_lower) < i64::from(rm_card.lower);
    let widens_upper = match (iv_upper(&card.interval), rm_card.upper) {
        // An RM-unbounded upper cannot be exceeded.
        (_, None) => false,
        // An unbounded archetype upper against a finite RM upper widens it.
        (None, Some(_)) => true,
        (Some(arch_upper), Some(rm_upper)) => i64::from(arch_upper) > i64::from(rm_upper),
    };
    if widens_lower || widens_upper {
        let rm_upper = rm_card
            .upper
            .map_or_else(|| "*".to_owned(), |u| u.to_string());
        let arch_upper = iv_upper(&card.interval).map_or_else(|| "*".to_owned(), |u| u.to_string());
        return Err(RuleViolation::new(
            "VCACA",
            format!(
                "attribute '{attr_name}' on '{parent_rm}' has cardinality \
                 {{{arch_lower}..{arch_upper}}}, which is wider than the reference \
                 model's {{{}..{rm_upper}}} — an archetype cardinality must be the \
                 same or narrower",
                rm_card.lower
            ),
        ));
    }
    Ok(())
}

// ─── VACMCO / VCOC (occurrences vs cardinality) ─────────────────────────────────

/// VCOC / VACMCO: "it must be possible for … one instance of every mandatory
/// child object … to be included within the cardinality range."
/// (`AOM2/master04.5-constraint_model-class_definitions.adoc` line 159, restating cADL VCOC,
/// `ADL1.4/master05-cadl.adoc` line 324.) The sum of the children's occurrence
/// *lower* bounds is the count that MUST appear; it cannot exceed a finite
/// cardinality upper bound. (The maximum-side of the literal cADL wording is
/// intentionally *not* enforced: a single-membership container with several
/// alternative child blocks — each `occurrences 0..1` — is a legal openEHR
/// pattern whose occurrence-maxima sum exceeds the cardinality, cADL
/// §Single-valued/alternative blocks.)
pub(super) fn check_cardinality_occurrences(
    attr_name: &str,
    parent_rm: &str,
    card: &Cardinality,
    children: &[CObject],
) -> Result<(), RuleViolation> {
    let Some(card_upper) = iv_upper(&card.interval) else {
        return Ok(()); // open cardinality upper bound: any number of children fits.
    };
    let required: i64 = children
        .iter()
        .map(|c| i64::from(iv_lower(NodeView::of(c).occurrences)))
        .sum();
    if required > i64::from(card_upper) {
        return Err(RuleViolation::new(
            "VACMCO",
            format!(
                "attribute '{attr_name}' on '{parent_rm}': the sum of the child occurrences \
                 lower bounds ({required}) exceeds the cardinality upper bound ({card_upper}), \
                 so the mandatory children cannot fit"
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The VCARM tolerance set, unchanged by the derivation: the LOCATABLE meta
    /// attributes and the US orthography come from the generated RM model, the
    /// five function/removed spellings from the adjudicated table, and nothing
    /// else is tolerated.
    #[test]
    fn vcarm_tolerance_set_is_exactly_the_adjudicated_one() {
        // Derived from the model: the US spelling of an attribute the RM class
        // really declares (`ELEMENT.null_flavour`).
        assert!(is_us_orthography_of_rm_attribute("ELEMENT", "null_flavor"));
        assert!(model::attribute("ELEMENT", "null_flavour").is_some());
        // …and only where the British-spelled attribute actually exists.
        assert!(!is_us_orthography_of_rm_attribute("CLUSTER", "null_flavor"));
        assert!(!is_us_orthography_of_rm_attribute("ELEMENT", "colour"));

        // Derived from the model: LOCATABLE's own attributes, not PATHABLE's.
        assert!(LOCATABLE_META_ATTRS.contains("archetype_node_id"));
        assert!(LOCATABLE_META_ATTRS.contains("name"));
        assert!(
            !LOCATABLE_META_ATTRS.contains("parent"),
            "PATHABLE's inherited member is excluded — the tolerance exists for \
             PATHABLE-only classes"
        );

        // Adjudicated: the RM declares these as FUNCTIONS (or not at all), so
        // the attribute model cannot answer for them.
        for (class, attr) in LEGACY_RM_ATTRS {
            assert!(
                model::attribute(class, attr).is_none(),
                "{class}.{attr} is in the model after all — derive it instead of \
                 listing it"
            );
        }
        assert_eq!(
            LEGACY_RM_ATTRS,
            [
                ("EVENT", "offset"),
                ("POINT_EVENT", "offset"),
                ("INTERVAL_EVENT", "offset"),
                ("DV_PROPORTION", "is_integral"),
                ("ITEM_TABLE", "rotated"),
            ],
            "the hand-listed set only shrinks: an entry the generated model can \
             answer must be derived, never listed"
        );
    }
}
