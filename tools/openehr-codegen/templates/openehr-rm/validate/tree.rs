// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! RM-instance validation over a typed tree (#6002): the class invariants
//! (each class's [`Validate`] by [`super::typed_dispatch::dispatch_typed`]),
//! [`super::check_archetyped_valid`], the HISTORY shape duty and the
//! [`super::terminology::slots_for`] bindings, over the generated walk.
//!
//! What the reader refuses is not re-checked. Messages come in canonical
//! attribute order; no openEHR specification orders them: our own design.

use std::any::Any;
use std::fmt::Write as _;

use openehr_base::validate::{InvariantViolation, Validate};

use super::terminology::{Binding, slots_for};

/// One violation of the typed passes, keyed by the RM instance path of the
/// offending node (`/content[0]/…`, `/` for the root).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeViolation {
    /// The RM instance path.
    pub path: String,
    /// The violation text.
    pub message: String,
    /// Which pass found it.
    pub kind: TreeKind,
}

/// The pass a [`TreeViolation`] comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeKind {
    /// A class invariant or a per-node check.
    Invariant,
    /// An RM-mandated openEHR terminology binding.
    Terminology,
}

/// A typed RM value the passes walk: one node, then its attributes in
/// canonical order.
pub trait Walk {
    /// Visit this value and everything under it.
    fn walk(&self, w: &mut Walker);
}

/// A primitive holds no RM object: a generic class instantiated over one
/// (`INTERVAL<Integer>`) walks as a node with no attributes to visit.
macro_rules! leaf {
    ($($t:ty),*) => {$(
        impl Walk for $t {
            fn walk(&self, _w: &mut Walker) {}
        }
    )*};
}

leaf!(String, bool, char, u8, i32, i64, f32, f64, serde_json::Value);

/// The coded value a terminology slot reads: `(code_string, terminology_id)`.
pub trait Coded {
    /// The code and its terminology, or `None` for an uncoded value.
    fn coded(&self) -> Option<(&str, &str)>;
}

impl Coded for crate::v1_2::data_types::text::code_phrase::CodePhrase {
    fn coded(&self) -> Option<(&str, &str)> {
        Some((self.code_string.as_str(), self.terminology_id.value.as_str()))
    }
}

impl Coded for crate::v1_2::data_types::text::dv_coded_text::DvCodedText {
    fn coded(&self) -> Option<(&str, &str)> {
        self.defining_code.coded()
    }
}

impl Coded for crate::v1_2::data_types::text::dv_text::DvText {
    fn coded(&self) -> Option<(&str, &str)> {
        match self {
            Self::DvCodedText(coded) => coded.coded(),
            Self::DvText(_) => None,
        }
    }
}

impl<T: Coded> Coded for Box<T> {
    fn coded(&self) -> Option<(&str, &str)> {
        T::coded(self)
    }
}

impl<T: Coded> Coded for Option<T> {
    fn coded(&self) -> Option<(&str, &str)> {
        self.as_ref().and_then(Coded::coded)
    }
}

/// What a node tells the passes about itself.
pub struct Node<'a> {
    /// The node's RM class.
    pub rm_type: &'static str,
    /// The node, for the class-invariant table.
    pub value: &'a dyn Any,
    /// `archetype_node_id`, on a LOCATABLE.
    pub node_id: Option<&'a str>,
    /// Whether the node carries `archetype_details`.
    pub has_archetype_details: bool,
    /// `archetype_details.archetype_id.value`, when present.
    pub details_archetype_id: Option<&'a str>,
    /// The coded value of a named attribute, for the terminology slots.
    pub coded: &'a dyn Fn(&str) -> Option<(&'a str, &'a str)>,
}

impl std::fmt::Debug for Node<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Node")
            .field("rm_type", &self.rm_type)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pass {
    Invariants,
    Terminology,
}

/// The running state of one pass: the path buffer and the violations.
pub struct Walker {
    pass: Pass,
    path: String,
    out: Vec<TreeViolation>,
    /// The violations of [`validate_node`], which judges the root alone.
    root: Option<Vec<InvariantViolation>>,
    /// Whether the next node is the root [`validate_below`] leaves out.
    skip_root: bool,
}

impl std::fmt::Debug for Walker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Walker")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl Walker {
    /// Visit a single-valued attribute.
    pub fn attr<T: Walk + ?Sized>(&mut self, key: &str, value: &T) {
        if self.root.is_some() {
            return;
        }
        let base = self.path.len();
        self.path.push('/');
        self.path.push_str(key);
        value.walk(self);
        self.path.truncate(base);
    }

    /// Visit member `index` of a container attribute.
    pub fn member<T: Walk + ?Sized>(&mut self, key: &str, index: usize, value: &T) {
        if self.root.is_some() {
            return;
        }
        let base = self.path.len();
        let _ = write!(self.path, "/{key}[{index}]");
        value.walk(self);
        self.path.truncate(base);
    }

    /// Run this pass's checks on one node.
    pub fn node(&mut self, node: &Node<'_>) {
        if let Some(out) = &mut self.root {
            root_node(node, out);
            return;
        }
        if self.skip_root {
            self.skip_root = false;
            return;
        }
        match self.pass {
            Pass::Invariants => self.invariants(node),
            Pass::Terminology => self.terminology(node),
        }
    }

    fn invariants(&mut self, node: &Node<'_>) {
        let ty = node.rm_type;
        let mut inv = Vec::new();
        let routed = class_invariants(ty, node.value, &mut inv);
        debug_assert_ne!(
            routed,
            Some(false),
            "{ty} has a class-invariant arm whose type the typed node does not have"
        );
        if node.node_id == Some("") && crate::v1_2::model::descendants("LOCATABLE").contains(&ty) {
            let mut closeout = Vec::with_capacity(1);
            super::generated::archetype_node_id_core(ty, "", &mut closeout);
            for v in closeout {
                if !inv.contains(&v) {
                    inv.push(v);
                }
            }
        }
        inv.extend(super::check_archetyped_valid(
            node.node_id,
            node.has_archetype_details,
            node.details_archetype_id,
        ));
        if ty == "HISTORY" {
            history_shapes(node.value, &mut inv);
        }
        for iv in inv {
            let path = if iv.path.is_empty() {
                norm_path(&self.path)
            } else {
                format!("{}/{}", self.path.trim_end_matches('/'), iv.path)
            };
            self.out.push(TreeViolation {
                path,
                message: iv.message,
                kind: TreeKind::Invariant,
            });
        }
    }

    fn terminology(&mut self, node: &Node<'_>) {
        for slot in slots_for(node.rm_type) {
            let Some((code, terminology)) = (node.coded)(slot.field) else {
                continue;
            };
            let violated = match slot.binding {
                Binding::Group(group) => {
                    terminology == "openehr"
                        && !group.is_valid(openehr_term::bundle::openehr(), code)
                }
                Binding::CodeSet(set) => !set.is_valid(openehr_term::bundle::openehr(), code),
            };
            if violated {
                self.out.push(TreeViolation {
                    path: format!("{}/{}", self.path, slot.field),
                    message: format!(
                        "code '{code}' is not a valid {} (openEHR terminology)",
                        slot.binding.label()
                    ),
                    kind: TreeKind::Terminology,
                });
            }
        }
    }
}

/// Normalize an RM instance path (empty is the root `/`).
fn norm_path(p: &str) -> String {
    if p.is_empty() {
        "/".to_owned()
    } else {
        p.to_owned()
    }
}

/// Runs the template-independent passes over a typed RM resource.
#[must_use]
pub fn validate_tree<T: Walk + ?Sized>(root: &T) -> Vec<TreeViolation> {
    let mut w = Walker {
        pass: Pass::Invariants,
        path: String::new(),
        out: Vec::new(),
        root: None,
        skip_root: false,
    };
    root.walk(&mut w);
    w.pass = Pass::Terminology;
    root.walk(&mut w);
    w.out
}

/// [`validate_tree`] without the value itself, the complement of
/// [`validate_node`].
#[must_use]
pub fn validate_below<T: Walk + ?Sized>(value: &T) -> Vec<TreeViolation> {
    let mut w = Walker {
        pass: Pass::Invariants,
        path: String::new(),
        out: Vec::new(),
        root: None,
        skip_root: true,
    };
    value.walk(&mut w);
    w.pass = Pass::Terminology;
    w.skip_root = true;
    value.walk(&mut w);
    w.out
}

/// Judges one typed value by itself: its class invariants, then its
/// terminology bindings, as `validate_rm_value` reports them.
#[must_use]
pub fn validate_node<T: Walk + ?Sized>(value: &T) -> Vec<InvariantViolation> {
    let mut w = Walker {
        pass: Pass::Invariants,
        path: String::new(),
        out: Vec::new(),
        root: Some(Vec::new()),
        skip_root: false,
    };
    value.walk(&mut w);
    w.root.unwrap_or_default()
}

/// The checks of [`validate_node`] on its one node.
fn root_node(node: &Node<'_>, out: &mut Vec<InvariantViolation>) {
    let ty = node.rm_type;
    let before = out.len();
    class_invariants(ty, node.value, out);
    if node.node_id == Some("") && crate::v1_2::model::descendants("LOCATABLE").contains(&ty) {
        let mut closeout = Vec::with_capacity(1);
        super::generated::archetype_node_id_core(ty, "", &mut closeout);
        for v in closeout {
            if !out.get(before..).is_some_and(|own| own.contains(&v)) {
                out.push(v);
            }
        }
    }
    for slot in slots_for(ty) {
        let Some((code, terminology)) = (node.coded)(slot.field) else {
            continue;
        };
        let violated = match slot.binding {
            Binding::Group(group) => {
                terminology == "openehr" && !group.is_valid(openehr_term::bundle::openehr(), code)
            }
            Binding::CodeSet(set) => !set.is_valid(openehr_term::bundle::openehr(), code),
        };
        if violated {
            out.push(InvariantViolation::at(
                slot.field,
                format!("Invariant {} failed on type {ty}", slot.invariant),
            ));
        }
    }
}

/// The HISTORY duty over typed events.
fn history_shapes(value: &dyn Any, inv: &mut Vec<InvariantViolation>) {
    use crate::v1_2::data_structures::history::event::Event;
    use crate::v1_2::data_structures::history::history::History;
    use crate::v1_2::data_structures::item_structure::item_structure::ItemStructure;
    let Some(history) = value.downcast_ref::<History<ItemStructure>>() else {
        return;
    };
    let mut first: Option<&str> = None;
    for (i, event) in history.events.iter().flatten().enumerate() {
        let data = match event {
            Event::PointEvent(e) => &e.data,
            Event::IntervalEvent(e) => &e.data,
        };
        let data_ty = item_structure_type(data);
        match first {
            None => first = Some(data_ty),
            Some(locked) if locked != data_ty => inv.push(InvariantViolation::at(
                format!("events[{i}]"),
                format!(
                    "HISTORY events must all carry the same ITEM_STRUCTURE \
                     subtype in data — the history is HISTORY<{locked}> but \
                     this event carries {data_ty} (RM data_structures \
                     master06 §History)"
                ),
            )),
            Some(_) => {}
        }
    }
}

fn item_structure_type(
    data: &crate::v1_2::data_structures::item_structure::item_structure::ItemStructure,
) -> &'static str {
    use crate::v1_2::data_structures::item_structure::item_structure::ItemStructure;
    match data {
        ItemStructure::ItemList(_) => "ITEM_LIST",
        ItemStructure::ItemSingle(_) => "ITEM_SINGLE",
        ItemStructure::ItemTable(_) => "ITEM_TABLE",
        ItemStructure::ItemTree(_) => "ITEM_TREE",
    }
}

fn run<T: Validate + 'static>(value: &dyn Any, out: &mut Vec<InvariantViolation>) -> bool {
    match value.downcast_ref::<T>() {
        Some(v) => {
            v.validate_invariants(out);
            true
        }
        None => false,
    }
}

/// The class-invariant table of [`super::typed_dispatch::dispatch_typed`],
/// typed: `None` for a class without an arm.
#[expect(
    clippy::too_many_lines,
    reason = "a flat RM type -> concrete type table; the length is the size of the RM type set, not logic"
)]
fn class_invariants(
    ty: &str,
    value: &dyn Any,
    out: &mut Vec<InvariantViolation>,
) -> Option<bool> {
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
    use crate::v1_2::data_structures::item_structure::item_structure::ItemStructure;
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
    use crate::v1_2::data_types::text::dv_coded_text::DvCodedText;
    use crate::v1_2::data_types::text::dv_text::DvTextData;
    use crate::v1_2::data_types::text::term_mapping::TermMapping;
    use crate::v1_2::data_types::time_specification::dv_periodic_time_specification::DvPeriodicTimeSpecification;
    use crate::v1_2::data_types::uri::dv_ehr_uri::DvEhrUri;
    use crate::v1_2::data_types::uri::dv_uri::DvUriData;
    use crate::v1_2::integration::generic_entry::GenericEntry;

    macro_rules! ordered {
        ($generic:ident) => {
            run::<$generic<DvOrdered>>(value, out)
                || run::<$generic<DvQuantity>>(value, out)
                || run::<$generic<DvCount>>(value, out)
                || run::<$generic<DvProportion>>(value, out)
                || run::<$generic<DvOrdinal>>(value, out)
                || run::<$generic<DvScale>>(value, out)
                || run::<$generic<DvDate>>(value, out)
                || run::<$generic<DvTime>>(value, out)
                || run::<$generic<DvDateTime>>(value, out)
                || run::<$generic<DvDuration>>(value, out)
        };
    }
    Some(match ty {
        "CODE_PHRASE" => run::<CodePhrase>(value, out),
        "DV_TEXT" => run::<DvTextData>(value, out),
        "DV_CODED_TEXT" => run::<DvCodedText>(value, out),
        "DV_URI" => run::<DvUriData>(value, out),
        "DV_EHR_URI" => run::<DvEhrUri>(value, out),
        "DV_IDENTIFIER" => run::<DvIdentifier>(value, out),
        "TERM_MAPPING" => run::<TermMapping>(value, out),
        "DV_MULTIMEDIA" => run::<DvMultimedia>(value, out),
        "DV_PROPORTION" => run::<DvProportion>(value, out),
        "DV_QUANTITY" => run::<DvQuantity>(value, out),
        "DV_COUNT" => run::<DvCount>(value, out),
        "DV_DURATION" => run::<DvDuration>(value, out),
        "DV_DATE" => run::<DvDate>(value, out),
        "DV_TIME" => run::<DvTime>(value, out),
        "DV_DATE_TIME" => run::<DvDateTime>(value, out),
        "DV_ORDINAL" => run::<DvOrdinal>(value, out),
        "DV_SCALE" => run::<DvScale>(value, out),
        "DV_PARSABLE" => run::<DvParsable>(value, out),
        "DV_PERIODIC_TIME_SPECIFICATION" => run::<DvPeriodicTimeSpecification>(value, out),
        // The JSON tier reads the limits at the bound `DV_ORDERED`; a typed
        // tree holds each instantiation the RM declares, which orders the same.
        "REFERENCE_RANGE" => ordered!(ReferenceRange),
        "DV_INTERVAL" => ordered!(DvInterval),
        "ELEMENT" => run::<Element>(value, out),
        "CLUSTER" => run::<Cluster>(value, out),
        "HISTORY" => run::<History<ItemStructure>>(value, out),
        "POINT_EVENT" => run::<PointEvent<ItemStructure>>(value, out),
        "INTERVAL_EVENT" => run::<IntervalEvent<ItemStructure>>(value, out),
        "ITEM_TABLE" => run::<ItemTable>(value, out),
        "PARTY_IDENTIFIED" => run::<PartyIdentifiedData>(value, out),
        "PARTY_RELATED" => run::<PartyRelated>(value, out),
        "AUDIT_DETAILS" => run::<AuditDetailsData>(value, out),
        "ATTESTATION" => run::<Attestation>(value, out),
        "FEEDER_AUDIT_DETAILS" => run::<FeederAuditDetails>(value, out),
        "ARCHETYPED" => run::<Archetyped>(value, out),
        "COMPOSITION" => run::<Composition>(value, out),
        "EVENT_CONTEXT" => run::<EventContext>(value, out),
        "ACTIVITY" => run::<Activity>(value, out),
        "INSTRUCTION_DETAILS" => run::<InstructionDetails>(value, out),
        "OBSERVATION" => run::<Observation>(value, out),
        "INSTRUCTION" => run::<Instruction>(value, out),
        "ACTION" => run::<Action>(value, out),
        "EVALUATION" => run::<Evaluation>(value, out),
        "ADMIN_ENTRY" => run::<AdminEntry>(value, out),
        "GENERIC_ENTRY" => run::<GenericEntry>(value, out),
        "SECTION" => run::<Section>(value, out),
        "FOLDER" => run::<Folder>(value, out),
        "ITEM_TAG" => run::<ItemTag>(value, out),
        "RESOURCE_DESCRIPTION" => run::<
            crate::v1_2::common::resource::resource_description::ResourceDescription,
        >(value, out),
        "RESOURCE_DESCRIPTION_ITEM" => run::<
            crate::v1_2::common::resource::resource_description_item::ResourceDescriptionItem,
        >(value, out),
        "EXTRACT" => run::<crate::v1_2::ehr_extract::common::extract::Extract>(value, out),
        "EXTRACT_UPDATE_SPEC" => run::<
            crate::v1_2::ehr_extract::common::extract_update_spec::ExtractUpdateSpec,
        >(value, out),
        "OBJECT_REF" => run::<ObjectRefData>(value, out),
        "PARTY_REF" => run::<PartyRef>(value, out),
        "VERSION_TREE_ID" => run::<VersionTreeId>(value, out),
        "OBJECT_VERSION_ID" => run::<ObjectVersionId>(value, out),
        "ISO_OID" => run::<IsoOid>(value, out),
        "ARCHETYPE_ID" => run::<ArchetypeId>(value, out),
        "TERMINOLOGY_ID" => run::<TerminologyId>(value, out),
        "INTERNET_ID" => run::<InternetId>(value, out),
        _ => return None,
    })
}
