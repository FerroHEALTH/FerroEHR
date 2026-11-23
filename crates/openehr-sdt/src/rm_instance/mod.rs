// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! RM-instance validation: the template-independent passes over a canonical-JSON
//! RM tree, and the report shape every validation surface of this crate speaks.
//!
//! The RM-invariant pass and the [`terminology`] pass key violations by RM
//! instance path; the decisions live in [`openehr_rm::v1_2::validate`], and this
//! module owns the walk. [`validate_composition`] adds the template pass of
//! [`crate::flat::validation`].

#![expect(
    clippy::disallowed_types,
    reason = "canonical JSON / Simplified Formats operate on the wire value by definition \
              (ITS-JSON; ITS-REST Simplified Formats) — serde_json::Value IS the subject matter \
              (#1694)"
)]

pub mod terminology;
mod walk;

use serde_json::Value;

use openehr_rm::v1_2::validate::{
    check_archetyped_valid, check_cluster_items_present, check_data_structure_shapes,
    check_mandatory_containers,
};

use crate::flat::webtemplate::model::WebTemplate;
use openehr_its::wire_validate::validate_rm_invariants_as;
use walk::{NodeFields, descend_attribute};

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

/// The category of a [`ValidationMessage`]; `Invariant` and `Terminology` come
/// from this module, the rest from [`crate::flat::validation`].
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
/// invariants, and the RM-mandated openEHR terminology, returning every
/// violation.
#[must_use]
pub fn validate_composition(composition: &Value, wt: &WebTemplate) -> Vec<ValidationMessage> {
    // The root's declared type is COMPOSITION: an untagged root is legal canonical
    // JSON.
    let mut out = validate_rm_and_terminology(composition, "COMPOSITION");
    // Pass 3: archetype conformance guided by the WebTemplate tree.
    out.extend(crate::flat::validation::validate_archetype_conformance(
        composition,
        wt,
    ));
    out
}

/// [`validate_composition`] over a typed COMPOSITION: the RM-instance passes
/// and the archetype-conformance pass, with no canonical-JSON value (#6002).
#[must_use]
pub fn validate_composition_typed(
    composition: &openehr_rm::v1_2::composition::composition::Composition,
    wt: &WebTemplate,
) -> Vec<ValidationMessage> {
    let mut out = validate_rm_and_terminology_typed(composition);
    out.extend(crate::flat::validation::validate_archetype_conformance_typed(composition, wt));
    out
}

/// Validate a canonical-JSON party against its `WebTemplate`, RM invariants,
/// and the RM-mandated openEHR terminology.
///
/// The template root must be a concrete party class (RM demographic
/// `master02-demographic_package.adoc` §Class Descriptions); otherwise one
/// [`ValidationKind::WrongType`] message at `/` is returned and no pass runs.
#[must_use]
pub fn validate_party(party: &Value, wt: &WebTemplate) -> Vec<ValidationMessage> {
    let root = wt
        .tree
        .rm_type
        .split('<')
        .next()
        .unwrap_or(&wt.tree.rm_type);
    if !crate::flat::build::PARTY_CLASSES.contains(&root) {
        let mut out = Vec::new();
        push(
            &mut out,
            "/",
            format!(
                "template {} is rooted at {root}, which is not a party class",
                wt.template_id
            ),
            ValidationKind::WrongType,
        );
        return out;
    }
    let mut out = validate_rm_and_terminology(party, root);
    out.extend(crate::flat::validation::validate_archetype_conformance(
        party, wt,
    ));
    out
}

/// Validate only the template-independent passes: RM class invariants and the
/// RM-mandated openEHR terminology, for every commit kind.
///
/// `declared` is the root's RM type when its `_type` is absent; a tagged root
/// overrides it.
#[must_use]
pub fn validate_rm_and_terminology(root: &Value, declared: &str) -> Vec<ValidationMessage> {
    validate_with(root, declared, LowerBounds::Enforced)
}

/// [`validate_rm_and_terminology`] over a typed RM resource, with the same
/// messages and paths in canonical attribute order.
///
/// A `553|incomplete|` commit cannot be read into strict typed RM and keeps
/// [`validate_rm_and_terminology_incomplete_as`].
#[must_use]
pub fn validate_rm_and_terminology_typed<T: openehr_rm::v1_2::validate::tree::Walk + ?Sized>(
    root: &T,
) -> Vec<ValidationMessage> {
    use openehr_rm::v1_2::validate::tree::{TreeKind, validate_tree};
    validate_tree(root)
        .into_iter()
        .map(|v| ValidationMessage {
            path: v.path,
            message: v.message,
            kind: match v.kind {
                TreeKind::Invariant => ValidationKind::Invariant,
                TreeKind::Terminology => ValidationKind::Terminology,
            },
        })
        .collect()
}

/// [`validate_rm_and_terminology`] for a `553|incomplete|` commit, with the
/// mandatory-presence and cardinality lower bounds relaxed to zero.
///
/// RM common `master06-change_control_package.adoc` §Incomplete Content:
/// "data may be missing, but it may not be wrong". The mandatory-container and
/// `CLUSTER.items` checks do not run, and the invariant tier goes through
/// [`openehr_its::wire_validate::validate_rm_invariants_relaxed_as`].
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

/// Pass 1: recurse the instance, running each node's core RM class invariants
/// and the per-node checks of [`openehr_rm::v1_2::validate`].
///
/// `path` is one reused buffer. `declared` is the parent attribute's concrete
/// declared type, so an untagged node still runs its invariants
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
        // Core invariants only; the terminology pass reports the terminology-backed ones.
        match bounds {
            LowerBounds::Enforced => validate_rm_invariants_as(effective, v, &mut inv),
            LowerBounds::Relaxed => {
                openehr_its::wire_validate::validate_rm_invariants_relaxed_as(
                    effective, v, &mut inv,
                );
            }
        }
        if bounds == LowerBounds::Enforced {
            // Mandatory-container lower bounds, relaxed for `553|incomplete|` (master06
            // §Incomplete Content).
            check_mandatory_containers(effective, v, &mut inv);
        }
    }
    inv.extend(check_archetyped_valid(
        node_id,
        has_archetype_details,
        details_archetype_id,
    ));
    if bounds == LowerBounds::Enforced {
        // `CLUSTER.items` presence relaxes with the other lower bounds.
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

    /// A root whose two archetype identities disagree is refused (RM
    /// `org.openehr.rm.common.locatable.adoc` §Attributes).
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

    /// Times the RM-invariant and terminology passes over the populated IPS example.
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
            std::hint::black_box(validate_rm_and_terminology(&comp, "COMPOSITION").len());
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
        let t_both = time_pass(iters, || {
            validate_rm_and_terminology(&comp, "COMPOSITION").len()
        });

        eprintln!("IPS validation walk cost ({node_count} _type nodes, {iters} iters):");
        eprintln!("  pass 1 rm_invariant_pass : {t_rm:>8.1} us/op");
        eprintln!("  pass 2 terminology_pass  : {t_term:>8.1} us/op");
        eprintln!("  combined (1+2)           : {t_both:>8.1} us/op");
    }

    /// Times the archetype-conformance walk and [`validate_composition`] over the
    /// populated IPS example.
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
        let t_rmterm = time_pass(iters, || {
            validate_rm_and_terminology(&comp, "COMPOSITION").len()
        });
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
