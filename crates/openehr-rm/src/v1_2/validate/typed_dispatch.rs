// @generated-from-template templates/openehr-rm/validate/typed_dispatch.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! The **typed dispatch tier** of the RM class-invariant check (hand-written).
//!
//! This is the `_type` → concrete-RM-type table that deserializes a
//! canonical-JSON node into its class through the emitted canonical-JSON `serde`
//! impls (`crate::json_serde`, `openehr-codegen -- emit-json`) and runs that
//! class's [`Validate`] impl.
//!
//! It lives here, beside the fast path ([`super::try_fast_validate`]) and the
//! invariant cores, because every decision it makes is RM model semantics: the
//! set of classes carrying a class invariant, how deeply each is decoded before
//! its invariants are read, and the message a non-conforming node produces. The
//! spec types carry their own `serde` impls since the foundation-phase codec
//! rewrite, so no downstream codec crate is needed to drive them.
//!
//! What is NOT here, and cannot be: the fallthrough for every class this table
//! does not name. That is the GENERATED five-crate structural dispatch
//! (`openehr_its::json_codec::generated::structural`), which spans
//! `openehr-base`/`-rm`/`-am`/`-term`/`-lang` at once and therefore can only be
//! emitted downstream of all of them. [`dispatch_typed`] reports the
//! fallthrough as `false` and the wire-boundary entry point
//! (`openehr_its::wire_validate::validate_rm_value_typed`) runs the generated
//! dispatch for it, then closes out the inherited
//! `LOCATABLE.Archetype_node_id_valid` rule
//! ([`super::locatable_node_id_violation`]).
//!
//! # Scope
//!
//! The invariants themselves are the released class tables' own
//! (`docs/specs/openehr/RM/docs/UML/classes/*.adoc` §Invariants), realized in
//! the `*_impl.rs` siblings; this module only routes a node to the right
//! concrete type. A node that does not deserialize into its declared concrete
//! RM type surfaces `does not conform to RM type …` (see
//! [`record_type_mismatch`]).

#![expect(
    clippy::disallowed_types,
    reason = "the wire-boundary validation reads the canonical JSON node before the typed decode \
              (#1694 boundary class)"
)]

use serde::de::DeserializeOwned;
use serde_json::Value;

use openehr_base::validate::{InvariantViolation, Validate};

/// Record a typed-deserialize failure as a validation violation.
///
/// A node that does not deserialize into its declared concrete RM type is NOT
/// "caught by the codec/schema layer" on the commit path: the node codec stores
/// the raw canonical-JSON fragment verbatim (no openEHR spec governs the storage
/// mechanics — our own storage design) and the ITS-JSON schema is not enforced
/// at commit, so a missing mandatory attribute (e.g. `COMPOSITION.composer [1]`)
/// or a wrong nested type (e.g. an `EHR_STATUS.subject` that is not `PARTY_SELF`)
/// reaches here and nowhere else. Per ITS-REST `422.yaml` ("converts,
/// but does not validate") this is a validation failure — surface it. (The valid
/// corpus deserializes cleanly at every node, so this never rejects a valid
/// input; if it ever did, that would expose a codegen field-optionality bug to
/// fix in the emitter.)
///
/// Public because the two tiers that report the SAME refusal live on both sides
/// of the crate boundary: this module's typed decode, and the wire-boundary
/// layer's undeclared-key door plus the generated structural fallthrough. One
/// message shape, one source.
pub fn record_type_mismatch(
    ty: &str,
    err: &dyn std::fmt::Display,
    out: &mut Vec<InvariantViolation>,
) {
    out.push(InvariantViolation::here(format!(
        "does not conform to RM type {ty}: {err}"
    )));
}

/// Deserialize `value` into `T` through the emitted canonical-JSON `serde`
/// impls, rendering a failure exactly as the canonical-JSON entry point
/// `openehr_its::json::from_canonical_value` renders it: the `serde_json`
/// message, then ` (at $<path>)` when the path tracker located the offending
/// node.
///
/// Two-phase, for the same reason the entry point is: the happy path reads
/// WITHOUT the path tracker (`serde_path_to_error` wraps every key and value
/// seed, which costs a large fraction of total read time on this corpus), and a
/// failed read re-runs the same deterministic decode WITH the tracker purely to
/// build the diagnostic (<https://docs.rs/serde_path_to_error/0.1>).
fn decode<T: DeserializeOwned>(value: &Value) -> Result<T, String> {
    match T::deserialize(value) {
        Ok(decoded) => Ok(decoded),
        Err(plain) => Err(match serde_path_to_error::deserialize::<_, T>(value) {
            Err(located) => render_path_error(&located),
            // A deterministic re-read of the same value cannot succeed where the
            // first read failed; if it ever does, the original diagnostic still
            // describes the failure — just without its path.
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

/// Like [`run`], but deserialize `T` from a copy of `value` whose nested
/// RM-node child collections have been emptied ([`prune_child_nodes`]).
///
/// NOTE: no openEHR spec governs validation-pass mechanics — our own perf
/// design: for LOCATABLE containers whose own invariants never read a child
/// collection, the child arrays are emptied before deserialize, making the
/// per-node pass O(total nodes) instead of O(Σ subtree sizes); single-valued
/// attributes are KEPT, so mandatory-presence and type conformance are
/// unchanged, and collection-reading types (`HISTORY.events`,
/// `ITEM_TABLE.rows`) keep the full [`run`] deserialize.
///
/// NOTE: emptying a child collection moves an element's malformation report
/// from the ancestor's path to the element's own recursion step; only the
/// redundant ancestor-cascade reporting on already-invalid input narrows —
/// the valid path and every test-pinned rejection stay byte-identical, and
/// the ITS-JSON schema gate remains the exhaustive structural oracle (this
/// pass is the RM class-invariant check, not a schema validator).
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

/// A shallow copy of an RM node with every nested RM-node **collection** emptied,
/// recursing through single-valued nested nodes (which are kept, so the node's
/// own mandatory single attributes stay enforced on deserialize). Scalar arrays
/// (e.g. `DV_MULTIMEDIA.data` octets) are kept as-is. See [`run_shallow`] for why
/// this is sound for the structural container types.
fn prune_child_nodes(value: &Value) -> Value {
    let Value::Object(map) = value else {
        return value.clone();
    };
    let mut out = serde_json::Map::with_capacity(map.len());
    for (key, child) in map {
        let pruned = match child {
            // An array of RM nodes: children are validated individually by
            // the composition validator, so keep ONE pruned member as the
            // structural witness — an empty array stays empty so the decode
            // judges it (NonEmptyVec refuses `[]` on 1..* and on the
            // present-implies-non-empty fields; a plain optional accepts it).
            Value::Array(items) if items.iter().any(Value::is_object) => {
                Value::Array(items.first().map(prune_child_nodes).into_iter().collect())
            }
            // A single nested node: keep it (its presence is a structural
            // constraint this node's deserialize must still enforce), but recurse
            // to empty ITS child collections.
            Value::Object(_) => prune_child_nodes(child),
            // Scalars and scalar arrays: keep verbatim.
            other => other.clone(),
        };
        out.insert(key.clone(), pruned);
    }
    Value::Object(out)
}

/// The `_type` → concrete-RM-type table: deserialize the node into the class
/// its `_type` names and run that class's `Validate` impl.
///
/// Returns `true` when the class was handled here, `false` for the
/// fallthrough — the caller then runs the GENERATED five-crate structural
/// dispatch (`openehr_its::json_codec::generated::structural`), which cannot
/// live in this crate because it spans every spec crate at once.
///
/// Authoritative for every node it handles (the fast path
/// [`super::try_fast_validate`] may only *skip* it when its result is provably
/// identical); also the oracle the fast-path equivalence tests compare against.
///
/// The table covers the concrete types carrying a non-terminology class
/// invariant, which need a typed value to run it on. Every other class falls
/// through to the generated structural dispatch, which decodes the node and
/// discards it: the codec is the structural-conformance authority for the whole
/// emitted model, so a class with no invariant is still refused when it is
/// structurally defective.
///
/// `DV_INTERVAL` is dispatched with a `DvOrdered` element type so
/// `Limits_consistent` is reached, falling back to `serde_json::Value` when the
/// limits do not deserialize as typed `DV_ORDERED` values. `HISTORY`,
/// `POINT_EVENT` and `INTERVAL_EVENT` use `serde_json::Value` throughout, which
/// is enough for their own non-child invariants.
///
/// The inherited `LOCATABLE.Archetype_node_id_valid`
/// (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.locatable.adoc`
/// §Invariants) is closed out by the caller via
/// [`super::locatable_node_id_violation`] over the generated model's descendant
/// closure. The arms below realize it themselves, so it is appended only when
/// this node's own pass did not report it; the classes with no arm pick it up
/// from that closeout after the structural fallthrough.
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
        // DV_TEXT + DV_CODED_TEXT share the DvText enum (Valid_value /
        // Formatting_valid, dv_text.adoc; DV_CODED_TEXT adds the structural
        // defining_code 1..1 at deserialize).
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
        "REFERENCE_RANGE" => run::<ReferenceRange>(ty, value, out),
        // DV_INTERVAL: prefer the DV_ORDERED-typed element so the
        // Limits_consistent ordering invariant runs; fall back to
        // Value elements (boundary flags only) for non-DV_ORDERED payloads.
        "DV_INTERVAL" => {
            if let Ok(v) = decode::<DvInterval<DvOrdered>>(value) {
                v.validate_invariants(out);
            } else {
                run::<DvInterval<Value>>(ty, value, out);
            }
        }
        // data_structures. HISTORY and ITEM_TABLE keep the full deserialize —
        // their own invariants read a child collection (`events` / `rows`); the
        // rest are structural containers with scalar-only invariants, so they
        // deserialize shallowly (see `run_shallow`).
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
        // ehr / composition — structural containers (scalar-only invariants),
        // deserialized shallowly (see `run_shallow`). GENERIC_ENTRY's `data:
        // ITEM [1..1]` is a single-valued node, so `run_shallow` keeps it and
        // still enforces its presence.
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
        // authored-resource metadata + the EHR_EXTRACT request classes:
        // scalar-only own invariants, shallow where child collections are
        // large. RESOURCE_DESCRIPTION keeps the full decode — its
        // Details_valid reads the `details` map.
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
        // Every other class: not handled here. The caller runs the GENERATED
        // structural dispatch, which decodes the node into that class's own Rust
        // type and discards it, so the codec is the structural-conformance
        // authority for the whole emitted model instead of only the
        // invariant-bearing classes above.
        _ => return false,
    }
    true
}
