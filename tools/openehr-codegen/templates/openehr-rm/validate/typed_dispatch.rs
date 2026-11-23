// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The typed dispatch tier: `_type` to the concrete RM type's [`Validate`] impl.
//!
//! An unnamed class falls through to
//! `openehr_its::json_codec::generated::structural`.

#![expect(
    clippy::disallowed_types,
    reason = "the wire-boundary validation reads the canonical JSON node before the typed decode \
              (#1694 boundary class)"
)]

use serde::de::DeserializeOwned;
use serde_json::Value;

use openehr_base::validate::{InvariantViolation, Validate};

/// Records a typed-deserialize failure as a violation: the commit path stores
/// the raw JSON, so this is where a non-conforming node is refused (ITS-REST
/// `422.yaml`, "converts, but does not validate").
pub fn record_type_mismatch(
    ty: &str,
    err: &dyn std::fmt::Display,
    out: &mut Vec<InvariantViolation>,
) {
    out.push(InvariantViolation::here(format!(
        "does not conform to RM type {ty}: {err}"
    )));
}

/// Deserializes `value` into `T`, rendering a failure as
/// `openehr_its::json::from_canonical_value` does; the path tracker runs only
/// on failure.
fn decode<T: DeserializeOwned>(value: &Value) -> Result<T, String> {
    match T::deserialize(value) {
        Ok(decoded) => Ok(decoded),
        Err(plain) => Err(match serde_path_to_error::deserialize::<_, T>(value) {
            Err(located) => render_path_error(&located),
            // A deterministic re-read cannot succeed; the path-less message stands.
            Ok(_) => plain.to_string(),
        }),
    }
}

/// Render a tracked read failure as `<message> (at $<path>)`, the shape
/// `openehr_its::json::JsonParseError` displays.
fn render_path_error(error: &serde_path_to_error::Error<serde_json::Error>) -> String {
    let path: Vec<String> = error
        .path()
        .iter()
        .map(|segment| match segment {
            serde_path_to_error::Segment::Seq { index } => format!("[{index}]"),
            serde_path_to_error::Segment::Map { key } => format!(".{key}"),
            serde_path_to_error::Segment::Enum { variant } => format!(".{variant}"),
            serde_path_to_error::Segment::Unknown => ".?".to_owned(),
        })
        .collect();
    let message = error.inner().to_string();
    if path.is_empty() {
        message
    } else {
        format!("{message} (at ${})", path.join(""))
    }
}

fn run<T: DeserializeOwned + Validate>(ty: &str, value: &Value, out: &mut Vec<InvariantViolation>) {
    match decode::<T>(value) {
        Ok(v) => v.validate_invariants(out),
        Err(e) => record_type_mismatch(ty, &e, out),
    }
}

/// [`run`] on a copy whose child node collections are emptied
/// ([`prune_child_nodes`]).
///
/// NOTE: no openEHR spec governs validation-pass mechanics: our own design,
/// for container types whose invariants read no child collection.
fn run_shallow<T: DeserializeOwned + Validate>(
    ty: &str,
    value: &Value,
    out: &mut Vec<InvariantViolation>,
) {
    match decode::<T>(&prune_child_nodes(value)) {
        Ok(v) => v.validate_invariants(out),
        Err(e) => record_type_mismatch(ty, &e, out),
    }
}

/// A copy of an RM node with nested node collections emptied, single nested
/// nodes kept and scalar arrays untouched.
fn prune_child_nodes(value: &Value) -> Value {
    let Value::Object(map) = value else {
        return value.clone();
    };
    let mut out = serde_json::Map::with_capacity(map.len());
    for (key, child) in map {
        let pruned = match child {
            // Keep one pruned member as the structural witness; an empty array stays.
            Value::Array(items) if items.iter().any(Value::is_object) => {
                Value::Array(items.first().map(prune_child_nodes).into_iter().collect())
            }
            // A single nested node is kept, its collections pruned.
            Value::Object(_) => prune_child_nodes(child),
            // Scalars and scalar arrays: keep verbatim.
            other => other.clone(),
        };
        out.insert(key.clone(), pruned);
    }
    Value::Object(out)
}

/// The `_type` to concrete-RM-type table, `false` for the fallthrough.
///
/// Covers the types carrying a non-terminology class invariant; the caller
/// closes out `LOCATABLE.Archetype_node_id_valid` with
/// [`super::locatable_node_id_violation`].
#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "a flat `_type` -> concrete-type dispatch table; the length is the size of the RM type set, not logic"
)]
pub fn dispatch_typed(ty: &str, value: &Value, out: &mut Vec<InvariantViolation>) -> bool {
    use openehr_base::v1_3::base_types::identification::archetype_id::ArchetypeId;
    use openehr_base::v1_3::base_types::identification::internet_id::InternetId;
    use openehr_base::v1_3::base_types::identification::iso_oid::IsoOid;
    use openehr_base::v1_3::base_types::identification::object_ref::ObjectRefData;
    use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
    use openehr_base::v1_3::base_types::identification::party_ref::PartyRef;
    use openehr_base::v1_3::base_types::identification::terminology_id::TerminologyId;
    use openehr_base::v1_3::base_types::identification::version_tree_id::VersionTreeId;

    use crate::v1_2::common::archetyped::archetyped::Archetyped;
    use crate::v1_2::common::archetyped::feeder_audit_details::FeederAuditDetails;
    use crate::v1_2::common::directory::folder::Folder;
    use crate::v1_2::common::generic::attestation::Attestation;
    use crate::v1_2::common::generic::audit_details::AuditDetailsData;
    use crate::v1_2::common::generic::party_identified::PartyIdentifiedData;
    use crate::v1_2::common::generic::party_related::PartyRelated;
    use crate::v1_2::common::tags::item_tag::ItemTag;
    use crate::v1_2::composition::composition::Composition;
    use crate::v1_2::composition::content::entry::action::Action;
    use crate::v1_2::composition::content::entry::activity::Activity;
    use crate::v1_2::composition::content::entry::admin_entry::AdminEntry;
    use crate::v1_2::composition::content::entry::evaluation::Evaluation;
    use crate::v1_2::composition::content::entry::instruction::Instruction;
    use crate::v1_2::composition::content::entry::instruction_details::InstructionDetails;
    use crate::v1_2::composition::content::entry::observation::Observation;
    use crate::v1_2::composition::content::navigation::section::Section;
    use crate::v1_2::composition::event_context::EventContext;
    use crate::v1_2::data_structures::history::history::History;
    use crate::v1_2::data_structures::history::interval_event::IntervalEvent;
    use crate::v1_2::data_structures::history::point_event::PointEvent;
    use crate::v1_2::data_structures::item_structure::item_table::ItemTable;
    use crate::v1_2::data_structures::representation::cluster::Cluster;
    use crate::v1_2::data_structures::representation::element::Element;
    use crate::v1_2::data_types::basic::dv_identifier::DvIdentifier;
    use crate::v1_2::data_types::encapsulated::dv_multimedia::DvMultimedia;
    use crate::v1_2::data_types::encapsulated::dv_parsable::DvParsable;
    use crate::v1_2::data_types::quantity::date_time::dv_date::DvDate;
    use crate::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
    use crate::v1_2::data_types::quantity::date_time::dv_duration::DvDuration;
    use crate::v1_2::data_types::quantity::date_time::dv_time::DvTime;
    use crate::v1_2::data_types::quantity::dv_count::DvCount;
    use crate::v1_2::data_types::quantity::dv_interval::DvInterval;
    use crate::v1_2::data_types::quantity::dv_ordered::DvOrdered;
    use crate::v1_2::data_types::quantity::dv_ordinal::DvOrdinal;
    use crate::v1_2::data_types::quantity::dv_proportion::DvProportion;
    use crate::v1_2::data_types::quantity::dv_quantity::DvQuantity;
    use crate::v1_2::data_types::quantity::dv_scale::DvScale;
    use crate::v1_2::data_types::quantity::reference_range::ReferenceRange;
    use crate::v1_2::data_types::text::code_phrase::CodePhrase;
    use crate::v1_2::data_types::text::dv_text::DvText;
    use crate::v1_2::data_types::text::term_mapping::TermMapping;
    use crate::v1_2::data_types::time_specification::dv_periodic_time_specification::DvPeriodicTimeSpecification;
    use crate::v1_2::data_types::uri::dv_ehr_uri::DvEhrUri;
    use crate::v1_2::data_types::uri::dv_uri::DvUriData;
    use crate::v1_2::integration::generic_entry::GenericEntry;

    match ty {
        // data_types
        "CODE_PHRASE" => run::<CodePhrase>(ty, value, out),
        // DV_TEXT and DV_CODED_TEXT share the DvText enum.
        "DV_TEXT" | "DV_CODED_TEXT" => run::<DvText>(ty, value, out),
        "DV_URI" => run::<DvUriData>(ty, value, out),
        "DV_EHR_URI" => run::<DvEhrUri>(ty, value, out),
        "DV_IDENTIFIER" => run::<DvIdentifier>(ty, value, out),
        "TERM_MAPPING" => run::<TermMapping>(ty, value, out),
        "DV_MULTIMEDIA" => run::<DvMultimedia>(ty, value, out),
        "DV_PROPORTION" => run::<DvProportion>(ty, value, out),
        "DV_QUANTITY" => run::<DvQuantity>(ty, value, out),
        "DV_COUNT" => run::<DvCount>(ty, value, out),
        "DV_DURATION" => run::<DvDuration>(ty, value, out),
        "DV_DATE" => run::<DvDate>(ty, value, out),
        "DV_TIME" => run::<DvTime>(ty, value, out),
        "DV_DATE_TIME" => run::<DvDateTime>(ty, value, out),
        "DV_ORDINAL" => run::<DvOrdinal>(ty, value, out),
        "DV_SCALE" => run::<DvScale>(ty, value, out),
        "DV_PARSABLE" => run::<DvParsable>(ty, value, out),
        "DV_PERIODIC_TIME_SPECIFICATION" => run::<DvPeriodicTimeSpecification>(ty, value, out),
        // A REFERENCE_RANGE node alone reads its limits at the bound of `T`.
        "REFERENCE_RANGE" => run::<ReferenceRange<DvOrdered>>(ty, value, out),
        // Typed DV_ORDERED limits, else `Value` limits.
        "DV_INTERVAL" => {
            if let Ok(v) = decode::<DvInterval<DvOrdered>>(value) {
                v.validate_invariants(out);
            } else {
                run::<DvInterval<Value>>(ty, value, out);
            }
        }
        // HISTORY and ITEM_TABLE read a child collection, so they decode fully.
        "ELEMENT" => run::<Element>(ty, value, out),
        "CLUSTER" => run_shallow::<Cluster>(ty, value, out),
        "HISTORY" => run::<History<Value>>(ty, value, out),
        "POINT_EVENT" => run_shallow::<PointEvent<Value>>(ty, value, out),
        "INTERVAL_EVENT" => run_shallow::<IntervalEvent<Value>>(ty, value, out),
        "ITEM_TABLE" => run::<ItemTable>(ty, value, out),
        // common
        "PARTY_IDENTIFIED" => run::<PartyIdentifiedData>(ty, value, out),
        "PARTY_RELATED" => run::<PartyRelated>(ty, value, out),
        "AUDIT_DETAILS" => run::<AuditDetailsData>(ty, value, out),
        "ATTESTATION" => run::<Attestation>(ty, value, out),
        "FEEDER_AUDIT_DETAILS" => run::<FeederAuditDetails>(ty, value, out),
        "ARCHETYPED" => run::<Archetyped>(ty, value, out),
        // Shallow: scalar-only invariants.
        "COMPOSITION" => run_shallow::<Composition>(ty, value, out),
        "EVENT_CONTEXT" => run_shallow::<EventContext>(ty, value, out),
        "ACTIVITY" => run_shallow::<Activity>(ty, value, out),
        "INSTRUCTION_DETAILS" => run::<InstructionDetails>(ty, value, out),
        "OBSERVATION" => run_shallow::<Observation>(ty, value, out),
        "INSTRUCTION" => run_shallow::<Instruction>(ty, value, out),
        "ACTION" => run_shallow::<Action>(ty, value, out),
        "EVALUATION" => run_shallow::<Evaluation>(ty, value, out),
        "ADMIN_ENTRY" => run_shallow::<AdminEntry>(ty, value, out),
        "GENERIC_ENTRY" => run_shallow::<GenericEntry>(ty, value, out),
        "SECTION" => run_shallow::<Section>(ty, value, out),
        "FOLDER" => run_shallow::<Folder>(ty, value, out),
        "ITEM_TAG" => run::<ItemTag>(ty, value, out),
        // RESOURCE_DESCRIPTION reads its `details` map, so it decodes fully.
        "RESOURCE_DESCRIPTION" => {
            run::<crate::v1_2::common::resource::resource_description::ResourceDescription>(
                ty, value, out,
            );
        }
        "RESOURCE_DESCRIPTION_ITEM" => {
            run::<crate::v1_2::common::resource::resource_description_item::ResourceDescriptionItem>(
                ty, value, out,
            );
        }
        "EXTRACT" => {
            run_shallow::<crate::v1_2::ehr_extract::common::extract::Extract>(ty, value, out);
        }
        "EXTRACT_UPDATE_SPEC" => {
            run::<crate::v1_2::ehr_extract::common::extract_update_spec::ExtractUpdateSpec>(
                ty, value, out,
            );
        }
        // base identification
        "OBJECT_REF" => run::<ObjectRefData>(ty, value, out),
        "PARTY_REF" => run::<PartyRef>(ty, value, out),
        "VERSION_TREE_ID" => run::<VersionTreeId>(ty, value, out),
        "OBJECT_VERSION_ID" => run::<ObjectVersionId>(ty, value, out),
        "ISO_OID" => run::<IsoOid>(ty, value, out),
        "ARCHETYPE_ID" => run::<ArchetypeId>(ty, value, out),
        "TERMINOLOGY_ID" => run::<TerminologyId>(ty, value, out),
        "INTERNET_ID" => run::<InternetId>(ty, value, out),
        // The caller runs the generated structural dispatch.
        _ => return false,
    }
    true
}
