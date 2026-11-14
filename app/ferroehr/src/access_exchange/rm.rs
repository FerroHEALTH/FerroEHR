// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The RM values every format builds directives from: coded terms of the
//! access control terminology, intervals of date and time, party references
//! and the `details` tree that keeps what a document says and no directive
//! attribute holds.

use openehr_base::containers::NonEmptyVec;
use openehr_rm::draft::access_control::access_control::access_directive::AccessDirective;
use openehr_rm::draft::access_control::terminology::{
    DirectiveKind, Effect, TERMINOLOGY_ID_ACCESS_CONTROL, Term,
};
use openehr_rm::v1_2::data_structures::item_structure::item_structure::ItemStructure;
use openehr_rm::v1_2::data_structures::item_structure::item_tree::ItemTree;
use openehr_rm::v1_2::data_structures::representation::element::Element;
use openehr_rm::v1_2::data_structures::representation::item::Item;
use openehr_rm::v1_2::data_types::basic::data_value::DataValue;
use openehr_rm::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
use openehr_rm::v1_2::data_types::quantity::dv_interval::DvInterval;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use openehr_rm::v1_2::data_types::text::dv_text::DvText;

use crate::access_log::entry::coded;
use crate::versioning::audit::{dv_date_time_text, dv_text};

/// The `archetype_node_id` an imported directive takes: the directive node of
/// the shipped `openEHR-EHR-EHR_ACCESS.access_directives.v1.0.0` archetype.
pub(crate) const DIRECTIVE_NODE: &str = "id5";

/// The node id of the `details` tree and of each element in it.
const DETAILS_NODE: &str = "at0000";

/// `values` as an optional non-empty list.
pub(crate) fn list<T>(values: Vec<T>) -> Option<NonEmptyVec<T>> {
    // NOTE: draft access control BMM, every optional list is absent or non-empty, so an
    // empty one is legitimately absent.
    NonEmptyVec::new(values).ok()
}

/// The members of an optional list.
pub(crate) fn members<T>(values: Option<&NonEmptyVec<T>>) -> &[T] {
    values.map_or(&[], |values| values)
}

/// `term` of the access control terminology as a `DV_CODED_TEXT`.
pub(crate) fn term<T: Term>(term: T) -> DvCodedText {
    coded(TERMINOLOGY_ID_ACCESS_CONTROL, term.code(), term.code())
}

/// The text of `text`, plain or coded.
pub(crate) fn text_of(text: &DvText) -> &str {
    match text {
        DvText::DvText(plain) => &plain.value,
        DvText::DvCodedText(coded) => &coded.value,
    }
}

/// The other effect.
pub(crate) const fn opposite(effect: Effect) -> Effect {
    match effect {
        Effect::Permit => Effect::Deny,
        Effect::Deny => Effect::Permit,
    }
}

/// A directive of `kind` and `effect` with nothing else set.
pub(crate) fn directive(kind: DirectiveKind, effect: Effect) -> AccessDirective {
    AccessDirective {
        name: dv_text(&format!("{} directive", kind.code())),
        archetype_node_id: DIRECTIVE_NODE.to_owned(),
        uid: None,
        links: None,
        archetype_details: None,
        feeder_audit: None,
        kind: term(kind),
        effect: term(effect),
        applies_to: None,
        operations: None,
        purposes: None,
        data: None,
        time_validity: None,
        emergency_override: false,
        exceptions: None,
        instructed_by: None,
        instructed: None,
        legal_grounds: None,
        evidence: None,
        identifiers: None,
        details: None,
    }
}

/// The interval from `start` to `end`, both included; an absent bound is
/// unbounded.
pub(crate) fn interval(start: Option<String>, end: Option<String>) -> DvInterval<DvDateTime> {
    DvInterval {
        lower_unbounded: start.is_none(),
        upper_unbounded: end.is_none(),
        lower_included: start.is_some(),
        upper_included: end.is_some(),
        lower: start.map(dv_date_time_text),
        upper: end.map(dv_date_time_text),
    }
}

/// The two bounds of `interval` as written, `None` where unbounded; `None`
/// for the whole interval when a bound is excluded, which a closed period
/// cannot state.
pub(crate) fn bounds(
    interval: &DvInterval<DvDateTime>,
) -> Option<(Option<String>, Option<String>)> {
    let bound = |unbounded: bool, at: Option<&DvDateTime>, included: bool| match at {
        Some(at) if !unbounded => included.then(|| Some(at.value.clone())),
        _ => Some(None),
    };
    Some((
        bound(
            interval.lower_unbounded,
            interval.lower.as_ref(),
            interval.lower_included,
        )?,
        bound(
            interval.upper_unbounded,
            interval.upper.as_ref(),
            interval.upper_included,
        )?,
    ))
}

/// The `details` tree keeping `kept`, each an element named by the document
/// element it renders; `None` when nothing is kept.
pub(crate) fn details(kept: &[(String, String)]) -> Option<ItemStructure> {
    if kept.is_empty() {
        return None;
    }
    let items = kept
        .iter()
        .map(|(element, rendering)| {
            Item::Element(Element {
                name: dv_text(element),
                archetype_node_id: DETAILS_NODE.to_owned(),
                uid: None,
                links: None,
                archetype_details: None,
                feeder_audit: None,
                null_flavour: None,
                value: Some(DataValue::DvText(dv_text(rendering))),
                null_reason: None,
            })
        })
        .collect();
    Some(ItemStructure::ItemTree(Box::new(ItemTree {
        name: dv_text("details"),
        archetype_node_id: DETAILS_NODE.to_owned(),
        uid: None,
        links: None,
        archetype_details: None,
        feeder_audit: None,
        items: Some(items),
    })))
}

/// Copies onto `target` the attributes `own`, the FerroEHR extension a
/// directive was written with, carries: every attribute no evaluator reads.
pub(crate) fn plumb(target: &mut AccessDirective, own: Option<AccessDirective>) {
    let Some(own) = own else {
        return;
    };
    target.name = own.name;
    target.archetype_node_id = own.archetype_node_id;
    target.uid = own.uid;
    target.links = own.links;
    target.archetype_details = own.archetype_details;
    target.feeder_audit = own.feeder_audit;
    target.instructed_by = own.instructed_by;
    target.instructed = own.instructed;
    target.legal_grounds = own.legal_grounds;
    target.evidence = own.evidence;
    target.identifiers = own.identifiers;
    target.details = own.details;
}
