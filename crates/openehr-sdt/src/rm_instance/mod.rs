// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! **RM-instance validation** — the template-independent passes over a
//! canonical-JSON RM tree, plus the report shape every validation surface of
//! this crate speaks.
//!
//! These checks are properties of the instance value alone, so they hold for
//! every COMPOSITION, EHR_STATUS, FOLDER or demographic PARTY tree whether or
//! not an operational template is referenced. Two collecting passes recurse the
//! whole instance, keying violations by RM instance path (`/content[0]/…`):
//!
//! 1. the RM-invariant pass runs each node's core class invariants plus the
//!    JSON-level per-node checks of [`openehr_rm::v1_2::validate`]. Being
//!    template-independent, it still checks nodes a template would fold away
//!    (ELEMENT / `ITEM_TREE` / HISTORY / EVENT);
//! 2. the [`terminology`] pass validates the RM-mandated openEHR
//!    terminology-group and code-set codes against
//!    [`openehr_rm::v1_2::validate::terminology`], backed by
//!    [`openehr_term::bundle`].
//!
//! The decisions live in `openehr-rm`; this module owns the WALK — recursing,
//! carrying the effective declared type down, and prefixing the absolute RM
//! path onto each node-relative violation. The third, template-driven pass
//! lives in [`crate::flat::validation`], and [`validate_composition`] composes
//! all three.

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

pub mod terminology;

use serde_json::Value;
use std::fmt::Write as _;

use openehr_rm::v1_2::model::declared_concrete_type;
use openehr_rm::v1_2::validate::{
    check_archetyped_valid, check_cluster_items_present, check_data_structure_shapes,
    check_mandatory_containers,
};

use crate::flat::webtemplate::model::WebTemplate;
use openehr_its::wire_validate::validate_rm_invariants_as;

/// A single validation violation, keyed by the RM path of the offending node
/// (a message + path + violation kind).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationMessage {
    /// The RM path to the offending node (RM instance path for RM-invariant /
    /// terminology violations, archetype `aqlPath` for archetype-conformance
    /// violations).
    pub path: String,
    /// A human-readable description of the violation.
    pub message: String,
    /// The violation category.
    pub kind: ValidationKind,
}

/// The category of a [`ValidationMessage`].
///
/// The vocabulary spans both validation layers: [`ValidationKind::Invariant`]
/// and [`ValidationKind::Terminology`] are what the RM-instance passes in this
/// module report; the remaining variants are the archetype-conformance
/// judgements of the template pass in [`crate::flat::validation`], which shares
/// this report shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationKind {
    /// An instance node's RM type does not conform to the constraint's type.
    WrongType,
    /// A mandatory node (`min >= 1`) is absent.
    Required,
    /// The number of matching nodes is outside the occurrences range.
    Occurrences,
    /// A container attribute's child count is outside its cardinality range.
    Cardinality,
    /// A numeric value is outside the constrained range.
    RangeError,
    /// A string value does not match the constrained pattern.
    PatternError,
    /// A coded value is not among the constrained coded options.
    CodedValue,
    /// A code is not valid in its RM-mandated openEHR terminology group.
    Terminology,
    /// An RM class invariant failed.
    Invariant,
    /// An instance node is not admitted by any sibling constraint or open slot
    /// under a closed (constrained) attribute (closed-world admission per the
    /// AOM2 direction — `AM/docs/AOM2/master04.2` `Rm_type_name` matching).
    Unexpected,
    /// Any other violation.
    Other,
}

/// Push one violation onto a report.
pub(crate) fn push(
    out: &mut Vec<ValidationMessage>,
    path: impl Into<String>,
    message: impl Into<String>,
    kind: ValidationKind,
) {
    out.push(ValidationMessage {
        path: path.into(),
        message: message.into(),
        kind,
    });
}

/// Normalize an RM instance path (empty → the root `/`).
pub(crate) fn norm_path(p: &str) -> String {
    if p.is_empty() {
        "/".to_owned()
    } else {
        p.to_owned()
    }
}

/// Validate a canonical-JSON COMPOSITION against its `WebTemplate`, RM
/// invariants, and the RM-mandated openEHR terminology.
///
/// Returns every violation found (the validator does not stop at the first
/// error); an empty result means the composition is valid to the extent this
/// validator checks.
///
/// The composed entry point: the two instance passes of this module followed by
/// the template-driven archetype-conformance pass
/// ([`crate::flat::validation::validate_archetype_conformance`]).
#[must_use]
pub fn validate_composition(composition: &Value, wt: &WebTemplate) -> Vec<ValidationMessage> {
    // Passes 1 + 2: the template-independent instance checks. The root's
    // declared type is COMPOSITION — an untagged root is legal canonical JSON
    // (the ITS-JSON schema requires `_type` only on polymorphic slots, and the
    // resource root is concretely COMPOSITION).
    let mut out = validate_rm_and_terminology(composition);
    // Pass 3: archetype conformance guided by the WebTemplate tree.
    out.extend(crate::flat::validation::validate_archetype_conformance(
        composition,
        wt,
    ));
    out
}

/// Validate only the **template-independent** passes: RM class invariants +
/// the RM-mandated openEHR terminology.
///
/// These hold for every RM instance whether or not an operational template is
/// referenced, so a COMPOSITION committed without a `template_id` still passes
/// them.
///
/// The COMPOSITION-rooted wrapper over [`validate_rm_and_terminology_as`],
/// where a caller committing another resource kind names its own root type.
#[must_use]
pub fn validate_rm_and_terminology(composition: &Value) -> Vec<ValidationMessage> {
    validate_rm_and_terminology_as(composition, "COMPOSITION")
}

/// [`validate_rm_and_terminology`] with the root node's declared RM type given.
///
/// The entry point for every non-COMPOSITION commit kind (`EHR_STATUS`,
/// `EHR_ACCESS`, `FOLDER`, the demographic PARTY types, …).
///
/// The invariants these passes run — `ARCHETYPED.Rm_version_valid`,
/// `LOCATABLE.Links_valid` / `Archetype_node_id_valid`, the `LINK` 1..1
/// attributes, `FEEDER_AUDIT_DETAILS.System_id_valid`
/// (`RM/docs/UML/classes/org.openehr.rm.common.*.adoc` §Invariants) — constrain
/// every node carrying the shape, so one walk serves every resource kind.
///
/// `declared` is the ROOT's effective RM type, for a root whose wire `_type` is
/// legitimately absent (canonical JSON requires `_type` only on polymorphic
/// slots). Every descendant dispatches from its own `_type` or its parent
/// attribute's concretely-declared type, and a tagged root overrides
/// `declared`.
#[must_use]
pub fn validate_rm_and_terminology_as(root: &Value, declared: &str) -> Vec<ValidationMessage> {
    validate_with(root, declared, LowerBounds::Enforced)
}

/// [`validate_rm_and_terminology_as`] for a `553|incomplete|` commit: the
/// mandatory-presence and cardinality-lower-bound layers are relaxed to zero,
/// every other layer runs at full strength.
///
/// RM common `master06-change_control_package.adoc` §Incomplete Content
/// (NOTE): "mandatory attributes may be absent. Concretely, single-valued
/// attributes may have null values and container attributes may be empty, even
/// though they may have minimum existence and cardinality respectively of one.
/// All other validity requirements must be satisfied. In other words, in an
/// `incomplete` commit, data may be missing, but it may not be wrong." — and
/// §Incomplete Content again, on the implementation form: incomplete data
/// "respects the same template and archetype(s), but with all existence and
/// cardinality lower limits set to zero".
///
/// Exactly three things change against the strict entry point:
/// [`openehr_rm::v1_2::validate::check_mandatory_containers`] and
/// [`openehr_rm::v1_2::validate::check_cluster_items_present`] do not run, and
/// the class-invariant tier goes through
/// [`openehr_its::wire_validate::validate_rm_invariants_relaxed_as`], which does not
/// drive typed construction of a node missing mandatory data. Everything else —
/// terminology, `LOCATABLE.Archetyped_valid`, the HISTORY shape duty, the
/// undeclared-member refusal, every value-level invariant — is untouched.
#[must_use]
pub fn validate_rm_and_terminology_incomplete_as(
    root: &Value,
    declared: &str,
) -> Vec<ValidationMessage> {
    validate_with(root, declared, LowerBounds::Relaxed)
}

/// Whether the mandatory-presence / cardinality-lower-bound layers of the
/// RM-instance passes are enforced or relaxed to zero (RM common master06
/// §Incomplete Content).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LowerBounds {
    /// The `532|complete|` reading: every lower bound is enforced.
    Enforced,
    /// The `553|incomplete|` reading: "all existence and cardinality lower
    /// limits set to zero".
    Relaxed,
}

/// The shared body of the two public entry points.
fn validate_with(root: &Value, declared: &str, bounds: LowerBounds) -> Vec<ValidationMessage> {
    let mut out = Vec::new();
    // The ROOT arm of the declared-type conformance rule. Never relaxed — RM
    // common master06 §Incomplete Content: "data may be missing, but it may
    // not be wrong".
    if let Some(wire) = root.get("_type").and_then(Value::as_str)
        && openehr_rm::v1_2::model::class(declared).is_some()
        && !openehr_rm::v1_2::model::is_a(wire, declared)
    {
        push(
            &mut out,
            "/".to_owned(),
            format!(
                "does not conform to RM type {declared}: the root claims `_type` \
                 {wire}, which is not a {declared}"
            ),
            ValidationKind::Invariant,
        );
    }
    // One reusable path buffer across both passes (each leaves it empty).
    let mut path = String::new();
    rm_invariant_pass(&mut out, root, &mut path, Some(declared), bounds);
    terminology::terminology_pass(&mut out, root, &mut path, Some(declared));
    out
}

/// Pass 1: recurse the whole instance, running each node's core RM class
/// invariants plus the JSON-level per-node checks of [`openehr_rm::v1_2::validate`],
/// keyed by the running RM instance path.
///
/// `path` is one reusable buffer pushed and truncated per recursion step, so
/// the full path string is materialized only when a violation is recorded
/// rather than allocated at every one of the ~1.5k nodes a commit visits.
///
/// `declared` is the parent attribute's declared RM type when concrete — the
/// effective type of a node whose wire `_type` is legitimately absent — so
/// untagged nodes like `COMPOSITION.context` still run their class invariants
/// ([`openehr_rm::v1_2::model::declared_concrete_type`]).
pub(crate) fn rm_invariant_pass(
    out: &mut Vec<ValidationMessage>,
    v: &Value,
    path: &mut String,
    declared: Option<&str>,
    bounds: LowerBounds,
) {
    let Some(obj) = v.as_object() else { return };
    let fields = NodeFields::of(obj);
    let (node_id, has_archetype_details, details_archetype_id) = (
        fields.node_id,
        fields.has_archetype_details,
        fields.details_archetype_id,
    );
    // The node's effective RM type: the wire tag, else the parent's
    // concretely-declared attribute type (untagged nodes are legal there).
    let ty = fields.ty.or(declared);
    let mut inv = Vec::new();
    if let Some(effective) = ty {
        // The core (fast/typed) RM invariants only — the terminology-backed
        // invariants are enforced by the dedicated terminology pass (its own
        // `ValidationKind::Terminology` rendering), so calling the core-only
        // entry here avoids double-reporting them.
        match bounds {
            LowerBounds::Enforced => validate_rm_invariants_as(effective, v, &mut inv),
            LowerBounds::Relaxed => {
                openehr_its::wire_validate::validate_rm_invariants_relaxed_as(
                    effective, v, &mut inv,
                );
            }
        }
        if bounds == LowerBounds::Enforced {
            // The orthogonal model-driven layer: mandatory-container lower
            // bounds (kept outside the core pair so the fast-vs-typed
            // equivalence property stays exact). Relaxed to zero on a
            // `553|incomplete|` commit (master06 §Incomplete Content).
            check_mandatory_containers(effective, v, &mut inv);
        }
    }
    // The JSON-level per-node checks (`openehr-rm`'s own value semantics):
    // they read the raw node, so an absent list and a present-but-empty one
    // are still distinguishable here. This walk only adapts their
    // node-relative violations onto the absolute RM instance path.
    inv.extend(check_archetyped_valid(
        node_id,
        has_archetype_details,
        details_archetype_id,
    ));
    if bounds == LowerBounds::Enforced {
        // `CLUSTER.items` presence is a mandatory-presence duty, so it relaxes
        // with the other lower bounds on a `553|incomplete|` commit; every
        // other data-structure shape duty (the HISTORY generic-parameter rule)
        // stays enforced below.
        inv.extend(check_cluster_items_present(obj, ty));
    }
    inv.extend(check_data_structure_shapes(obj, ty));
    for iv in inv {
        let p = if iv.path.is_empty() {
            norm_path(path)
        } else {
            format!("{}/{}", path.trim_end_matches('/'), iv.path)
        };
        push(out, p, iv.message, ValidationKind::Invariant);
    }
    for (k, val) in obj {
        if !k.starts_with('_') {
            descend_attribute(out, k, val, path, ty, bounds);
        }
    }
}

/// The node fields every per-node check reads, collected in ONE projection
/// pass so none of them pays a hashed map lookup.
///
/// This runs for every node of every commit; the only remaining per-node gets
/// are gated behind a matching `_type`.
struct NodeFields<'a> {
    ty: Option<&'a str>,
    node_id: Option<&'a str>,
    has_archetype_details: bool,
    details_archetype_id: Option<&'a str>,
}

impl<'a> NodeFields<'a> {
    fn of(obj: &'a serde_json::Map<String, Value>) -> Self {
        let mut fields = NodeFields {
            ty: None,
            node_id: None,
            has_archetype_details: false,
            details_archetype_id: None,
        };
        for (k, val) in obj {
            match k.as_str() {
                "_type" => fields.ty = val.as_str(),
                "archetype_node_id" => fields.node_id = val.as_str(),
                "archetype_details" => {
                    fields.has_archetype_details = !val.is_null();
                    fields.details_archetype_id = val
                        .get("archetype_id")
                        .and_then(|a| a.get("value"))
                        .and_then(Value::as_str);
                }
                _ => {}
            }
        }
        fields
    }
}

/// Descends into one attribute of a node, checking each member's declared slot
/// type on the way down.
fn descend_attribute(
    out: &mut Vec<ValidationMessage>,
    k: &str,
    val: &Value,
    path: &mut String,
    ty: Option<&str>,
    bounds: LowerBounds,
) {
    let child_declared = ty.and_then(|t| declared_concrete_type(t, k));
    match val {
        Value::Array(a) => {
            for (i, item) in a.iter().enumerate() {
                let base = path.len();
                let _ = write!(path, "/{k}[{i}]");
                if item.is_object() {
                    check_slot_type(out, ty, k, item, path);
                    rm_invariant_pass(out, item, path, child_declared, bounds);
                } else if let Some(iv) = ty.and_then(|parent| {
                    openehr_rm::v1_2::validate::check_slot_member_is_object(parent, k)
                }) {
                    // A scalar member of a class-typed list slot is the same
                    // positive contradiction a foreign `_type` is — its
                    // declared type is an RM class, which no JSON scalar can
                    // be.
                    push(out, norm_path(path), iv.message, ValidationKind::Invariant);
                }
                path.truncate(base);
            }
        }
        Value::Object(_) => {
            let base = path.len();
            let _ = write!(path, "/{k}");
            check_slot_type(out, ty, k, val, path);
            rm_invariant_pass(out, val, path, child_declared, bounds);
            path.truncate(base);
        }
        _ => {}
    }
}

/// The declared-slot-type conformance rule (the WRONGNESS half of slot typing,
/// never relaxed — RM common master06 §Incomplete Content): a TAGGED child
/// must claim the slot's declared type or a subtype.
///
/// One model-driven rule for single slots and list members alike; the
/// shallow-pruned typed dispatch cannot see it (each pruned member
/// re-dispatches on its own `_type`), so the walk owns it.
fn check_slot_type(
    out: &mut Vec<ValidationMessage>,
    parent_ty: Option<&str>,
    attribute: &str,
    item: &Value,
    path: &str,
) {
    if let (Some(parent), Some(wire)) = (parent_ty, item.get("_type").and_then(Value::as_str))
        && let Some(iv) =
            openehr_rm::v1_2::validate::check_declared_slot_type(parent, attribute, wire)
    {
        push(out, norm_path(path), iv.message, ValidationKind::Invariant);
    }
}

#[cfg(test)]
mod tests {
    //! Per-rule unit tests for the RM-instance passes, built on minimal
    //! hand-shaped instances so each rule is exercised in isolation. End-to-end
    //! corpus + public-seam tests live in `tests/it/`.

    use serde_json::{Value, json};

    use super::*;

    // ── Declared-slot-type conformance (the WRONGNESS half of slot typing) ───

    /// RM ehr `composition.adoc` §Attributes types `content`
    /// `List<CONTENT_ITEM>`, so a `DV_TEXT` member is a positive type
    /// contradiction and is refused.
    #[test]
    fn foreign_type_in_a_list_slot_rejected() {
        let inst = json!({
            "_type": "COMPOSITION",
            "content": [{ "_type": "DV_TEXT", "value": "not a content item" }]
        });
        let out = rm_only(&inst);
        assert!(
            out.iter().any(|m| m.path == "/content[0]"
                && m.message.contains("`content` is declared CONTENT_ITEM")),
            "the foreign list member must be refused at its own path: {out:?}"
        );
    }

    /// The same contradiction on a SINGLE-VALUED slot: `COMPOSITION.context` is
    /// declared `EVENT_CONTEXT`, so a tagged foreign object there is refused.
    #[test]
    fn foreign_type_in_a_single_slot_rejected() {
        let inst = json!({
            "_type": "COMPOSITION",
            "context": { "_type": "DV_TEXT", "value": "not a context" }
        });
        let out = rm_only(&inst);
        assert!(
            out.iter().any(|m| m.path == "/context"
                && m.message.contains("`context` is declared EVENT_CONTEXT")),
            "the foreign single member must be refused at its own path: {out:?}"
        );
    }

    /// A legitimate subtype in an abstract-declared slot stays accepted —
    /// strict means exact, in both directions (spec-adherence.md).
    #[test]
    fn subtype_in_an_abstract_slot_accepted() {
        let inst = json!({
            "_type": "COMPOSITION",
            "content": [{
                "_type": "OBSERVATION",
                "name": { "_type": "DV_TEXT", "value": "o" },
                "archetype_node_id": "at0001"
            }]
        });
        let out = rm_only(&inst);
        assert!(
            !out.iter()
                .any(|m| m.message.contains("is declared CONTENT_ITEM")),
            "an OBSERVATION is a CONTENT_ITEM — no slot violation: {out:?}"
        );
    }

    /// Type WRONGNESS never relaxes on a `553|incomplete|` commit — "data may
    /// be missing, but it may not be wrong" (RM common master06 §Incomplete
    /// Content).
    #[test]
    fn slot_type_conformance_survives_the_incomplete_relaxation() {
        let inst = json!({
            "_type": "COMPOSITION",
            "content": [{ "_type": "DV_TEXT", "value": "still wrong" }]
        });
        let mut out = Vec::new();
        rm_invariant_pass(
            &mut out,
            &inst,
            &mut String::new(),
            Some("COMPOSITION"),
            LowerBounds::Relaxed,
        );
        assert!(
            out.iter()
                .any(|m| m.message.contains("`content` is declared CONTENT_ITEM")),
            "the incomplete relaxation must not admit a wrong type: {out:?}"
        );
    }

    // ── LOCATABLE root identity + Links_valid (RM invariant pass) ─────────────

    /// Run only pass 1 (the RM class invariants over the whole instance).
    fn rm_only(instance: &Value) -> Vec<ValidationMessage> {
        let mut out = Vec::new();
        rm_invariant_pass(
            &mut out,
            instance,
            &mut String::new(),
            Some("COMPOSITION"),
            LowerBounds::Enforced,
        );
        out
    }

    /// A minimal archetype-root node carrying an ARCHETYPED block, used to
    /// isolate the `LOCATABLE` root/link rules from every other invariant.
    fn root_node(node_id: &str, archetype_id: &str) -> Value {
        json!({
            "_type": "SECTION",
            "name": { "_type": "DV_TEXT", "value": "s" },
            "archetype_node_id": node_id,
            "archetype_details": {
                "_type": "ARCHETYPED",
                "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype_id },
                "rm_version": "1.2.0"
            },
            "items": [{
                "_type": "OBSERVATION", "archetype_node_id": "at0001",
                "name": { "_type": "DV_TEXT", "value": "o" }
            }]
        })
    }

    /// `RM/docs/UML/classes/org.openehr.rm.common.locatable.adoc` §Attributes
    /// (`archetype_node_id`): "At an archetype root point, the value of this
    /// attribute is always the stringified form of the `archetype_id` found in
    /// the `archetype_details` object" — a root whose two archetype identities
    /// disagree is refused.
    #[test]
    fn archetype_root_node_id_mismatch_rejected() {
        let inst = root_node(
            "openEHR-EHR-SECTION.other.v1",
            "openEHR-EHR-SECTION.adhoc.v1",
        );
        let msgs = rm_only(&inst);
        assert!(
            msgs.iter().any(|m| m.kind == ValidationKind::Invariant
                && m.message.contains("LOCATABLE.archetype_node_id")),
            "expected a root node-id identity violation, got {msgs:?}"
        );
    }

    /// The accepting twin: the same root whose `archetype_node_id` IS the
    /// stringified `archetype_details.archetype_id` raises no identity finding.
    #[test]
    fn archetype_root_node_id_match_accepted() {
        let inst = root_node(
            "openEHR-EHR-SECTION.adhoc.v1",
            "openEHR-EHR-SECTION.adhoc.v1",
        );
        let msgs = rm_only(&inst);
        assert!(
            !msgs
                .iter()
                .any(|m| m.message.contains("LOCATABLE.archetype_node_id")),
            "a matching root must not violate the node-id identity rule: {msgs:?}"
        );
    }

    /// `locatable.adoc` §Invariants `Links_valid`: the shape is
    /// `Option<NonEmptyVec<LINK>>`, so a present-but-empty `links` refuses at
    /// the typed tier's decode, on any LOCATABLE node.
    #[test]
    fn links_present_but_empty_rejected() {
        let mut inst = root_node(
            "openEHR-EHR-SECTION.adhoc.v1",
            "openEHR-EHR-SECTION.adhoc.v1",
        );
        let obj = inst.as_object_mut().expect("root object");
        obj.remove("items"); // SECTION.items is 0..1; keep the fixture valid but for links
        obj.insert("links".into(), json!([]));
        let msgs = rm_only(&inst);
        assert!(
            msgs.iter()
                .any(|m| m.message.contains("links") && m.message.contains("at least one member")),
            "expected the links parse refusal, got {msgs:?}"
        );
    }

    /// The accepting twin: a non-empty `links` list satisfies `Links_valid`.
    #[test]
    fn links_non_empty_accepted() {
        let mut inst = root_node(
            "openEHR-EHR-SECTION.adhoc.v1",
            "openEHR-EHR-SECTION.adhoc.v1",
        );
        inst.as_object_mut().expect("root object").insert(
            "links".into(),
            json!([{
                "_type": "LINK",
                "meaning": { "_type": "DV_TEXT", "value": "follow up" },
                "type": { "_type": "DV_TEXT", "value": "issue" },
                "target": { "_type": "DV_EHR_URI", "value": "ehr://example/x" }
            }]),
        );
        let msgs = rm_only(&inst);
        assert!(
            !msgs.iter().any(|m| m.message.contains("Links_valid")),
            "a non-empty links list must not violate Links_valid: {msgs:?}"
        );
    }

    // ── validation-walk cost measurement (not a gate) ─────────────────────────

    /// Count the `_type`-bearing nodes reachable in `v` (the units both
    /// template-independent passes visit).
    fn count_type_nodes(v: &Value) -> usize {
        match v {
            Value::Object(obj) => {
                let self_count = usize::from(obj.contains_key("_type"));
                self_count
                    + obj
                        .iter()
                        .filter(|(k, _)| !k.starts_with('_'))
                        .map(|(_, val)| count_type_nodes(val))
                        .sum::<usize>()
            }
            Value::Array(a) => a.iter().map(count_type_nodes).sum(),
            _ => 0,
        }
    }

    /// Time `iters` runs of `f`, returning microseconds per run.
    fn time_pass(iters: u32, mut f: impl FnMut() -> usize) -> f64 {
        let start = std::time::Instant::now();
        let mut sink = 0usize;
        for _ in 0..iters {
            sink = sink.wrapping_add(f());
        }
        std::hint::black_box(sink);
        start.elapsed().as_secs_f64() * 1e6 / f64::from(iters)
    }

    /// MEASUREMENT (not a correctness gate): quantify the pre-tx template-
    /// independent validation walk over the populated IPS example (~1.5k `_type`
    /// nodes). The RM-invariant and terminology passes each traverse the whole
    /// instance independently; this splits and times them. Ignored by default
    /// (timing, not correctness); run:
    /// `cargo nextest run -p openehr-its --run-ignored all \
    ///   -E 'test(measure_ips_validation_walk_cost)' --no-capture`.
    #[test]
    #[ignore = "measurement, not a correctness gate — run with --run-ignored all"]
    fn measure_ips_validation_walk_cost() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../corpus/templates/ckm/international-patient-summary.example.json"
        );
        let comp: Value =
            serde_json::from_str(&std::fs::read_to_string(path).expect("read IPS example"))
                .expect("parse IPS example");
        let node_count = count_type_nodes(&comp);

        // Warm up (allocator, branch predictors, the lazily-initialized bundle).
        for _ in 0..5 {
            std::hint::black_box(validate_rm_and_terminology(&comp).len());
        }

        let iters = 50;
        let t_rm = time_pass(iters, || {
            let mut out = Vec::new();
            rm_invariant_pass(
                &mut out,
                &comp,
                &mut String::new(),
                Some("COMPOSITION"),
                LowerBounds::Enforced,
            );
            out.len()
        });
        let t_term = time_pass(iters, || {
            let mut out = Vec::new();
            terminology::terminology_pass(&mut out, &comp, &mut String::new(), Some("COMPOSITION"));
            out.len()
        });
        let t_both = time_pass(iters, || validate_rm_and_terminology(&comp).len());

        eprintln!("IPS validation walk cost ({node_count} _type nodes, {iters} iters):");
        eprintln!("  pass 1 rm_invariant_pass : {t_rm:>8.1} us/op");
        eprintln!("  pass 2 terminology_pass  : {t_term:>8.1} us/op");
        eprintln!("  combined (1+2)           : {t_both:>8.1} us/op");
    }

    /// MEASUREMENT (not a correctness gate): quantify the archetype-conformance
    /// **walk** (pass 3) over the populated IPS example against its OPT-built
    /// `WebTemplate`, plus the full [`validate_composition`]. Ignored by default
    /// (timing, not correctness); run:
    /// `cargo nextest run -p openehr-its --run-ignored all \
    ///   -E 'test(measure_ips_validation_full_cost)' --no-capture`.
    #[test]
    #[ignore = "measurement, not a correctness gate — run with --run-ignored all"]
    fn measure_ips_validation_full_cost() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/templates/ckm");
        let opt_xml = std::fs::read_to_string(format!("{dir}/international-patient-summary.opt"))
            .expect("read IPS OPT");
        let opt = openehr_its::opt14::from_xml(&opt_xml).expect("parse IPS OPT");
        let wt = crate::flat::webtemplate::builder::build_web_template(&opt)
            .expect("build IPS WebTemplate");
        let comp: Value = serde_json::from_str(
            &std::fs::read_to_string(format!("{dir}/international-patient-summary.example.json"))
                .expect("read IPS example"),
        )
        .expect("parse IPS example");
        let node_count = count_type_nodes(&comp);

        // Warm up (allocator, branch predictors, the lazily-initialized bundle).
        for _ in 0..5 {
            std::hint::black_box(validate_composition(&comp, &wt).len());
        }

        // Public entry points only, so this harness compiles unchanged.
        let iters = 50;
        let t_rmterm = time_pass(iters, || validate_rm_and_terminology(&comp).len());
        let t_walk = time_pass(iters, || {
            crate::flat::validation::validate_archetype_conformance(&comp, &wt).len()
        });
        let t_all = time_pass(iters, || validate_composition(&comp, &wt).len());

        eprintln!("IPS full validation cost ({node_count} _type nodes, {iters} iters):");
        eprintln!("  passes 1+2 rm+terminology      : {t_rmterm:>8.1} us/op");
        eprintln!("  pass 3 walk (archetype conf.)  : {t_walk:>8.1} us/op");
        eprintln!("  full validate_composition      : {t_all:>8.1} us/op");
    }
}
