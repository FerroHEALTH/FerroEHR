// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

#![expect(clippy::panic, reason = "test assertions/diagnostics/fixtures")]
//! `WebTemplate` builder tests.
//!
//! * **smoke gate** — every `.opt` in the 91-file service corpus AND the full
//!   vendored Better `web-template-tests` set (63 templates) is exercised: each
//!   that the `openehr_its::opt14` parser can read MUST build a `WebTemplate`
//!   without panicking, and the tree root's `rmType` must round-trip the OPT's
//!   own root type. Templates that `opt14` cannot yet parse are reported as
//!   pre-existing parser gaps (not a WebTemplate-builder failure).
//! * **insta goldens** — deterministic snapshots for representative templates.
//! * **targeted assertions** — ports of Better `BuilderTest`/`CodedTextTest`/
//!   `OrdinalWebTemplateInputBuilder`-style checks (coded-text `|code` + list,
//!   quantity `|magnitude`+`|unit`, ordinal integers, min/max, snake-cased ids,
//!   archetype/`atNNNN` predicates, sibling-id de-duplication).

use std::path::{Path, PathBuf};

use openehr_its::opt14;
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use openehr_sdt::flat::webtemplate::model::{WebTemplateInputType, WebTemplateNode};

fn corpus_dir() -> PathBuf {
    crate::common::service_corpus_dir()
}

fn better_fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/better")
}

/// Parse an `OPT` and build a `WebTemplate`, returning any error as a string.
fn build_from_file(
    path: &Path,
) -> Result<openehr_sdt::flat::webtemplate::model::WebTemplate, String> {
    let xml = std::fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    let opt = opt14::from_xml(&xml).map_err(|e| format!("opt14 parse: {e}"))?;
    build_web_template(&opt).map_err(|e| format!("build: {e}"))
}

fn find_node<'a>(
    node: &'a WebTemplateNode,
    pred: &dyn Fn(&WebTemplateNode) -> bool,
) -> Option<&'a WebTemplateNode> {
    if pred(node) {
        return Some(node);
    }
    for child in &node.children {
        if let Some(found) = find_node(child, pred) {
            return Some(found);
        }
    }
    None
}

fn for_each_node(node: &WebTemplateNode, f: &mut dyn FnMut(&WebTemplateNode)) {
    f(node);
    for child in &node.children {
        for_each_node(child, f);
    }
}

/// Assert that every parent's child ids are unique (Better numeric-suffix dedup).
fn assert_unique_sibling_ids(node: &WebTemplateNode) {
    let mut seen = std::collections::HashSet::new();
    for child in &node.children {
        assert!(
            seen.insert(child.id.clone()),
            "duplicate sibling id {:?} under {:?}",
            child.id,
            node.id
        );
        assert_unique_sibling_ids(child);
    }
}

// ── smoke gate ───────────────────────────────────────────────────────────────

#[test]
fn every_opt_builds_a_web_template() {
    let corpus = crate::common::opt_files(&corpus_dir());
    let vendored = crate::common::opt_files(&better_fixtures_dir());
    let corpus_count = corpus.len();
    let vendored_count = vendored.len();

    assert!(
        corpus_count >= 90,
        "expected the ~91-file service corpus, found {corpus_count} (dir: {})",
        corpus_dir().display()
    );
    assert!(
        vendored_count >= 60,
        "expected the full vendored Better set (~63), found {vendored_count}"
    );

    let mut build_failures = Vec::new();
    // opt14 parser gaps — reported, not a WebTemplate-builder failure.
    let mut parse_skips = Vec::new();
    let mut built = 0usize;
    let mut composition_roots = 0usize;

    for path in corpus.iter().chain(vendored.iter()) {
        let xml = match std::fs::read_to_string(path) {
            Ok(x) => x,
            Err(e) => {
                build_failures.push(format!("{}: read: {e}", path.display()));
                continue;
            }
        };
        let opt = match opt14::from_xml(&xml) {
            Ok(o) => o,
            Err(e) => {
                parse_skips.push(format!("{}: {e}", path.display()));
                continue;
            }
        };
        let expected_root = opt.definition.rm_type_name.clone();
        match build_web_template(&opt) {
            Ok(wt) => {
                built += 1;
                if wt.tree.rm_type != expected_root {
                    build_failures.push(format!(
                        "{}: root rmType = {} (expected {}, the OPT root type)",
                        path.display(),
                        wt.tree.rm_type,
                        expected_root
                    ));
                }
                if wt.tree.rm_type == "COMPOSITION" {
                    composition_roots += 1;
                }
            }
            Err(e) => build_failures.push(format!("{}: {e}", path.display())),
        }
    }

    eprintln!(
        "web-template smoke: {corpus_count} corpus + {vendored_count} vendored | \
         built OK = {built} ({composition_roots} COMPOSITION-rooted), \
         opt14 parse-skipped = {}",
        parse_skips.len()
    );
    if !parse_skips.is_empty() {
        eprintln!(
            "opt14 could not parse {} template(s):\n  {}",
            parse_skips.len(),
            parse_skips.join("\n  ")
        );
    }

    // The gate: the WebTemplate builder must succeed on every parseable OPT.
    assert!(
        build_failures.is_empty(),
        "{} template(s) failed the WebTemplate builder:\n{}",
        build_failures.len(),
        build_failures.join("\n")
    );
    assert!(
        built >= 140,
        "expected most templates to build, only {built} did"
    );
    assert!(
        composition_roots >= 130,
        "expected most templates to be COMPOSITION-rooted, got {composition_roots}"
    );
}

// ── insta goldens ────────────────────────────────────────────────────────────

fn golden(name: &str) {
    let path = better_fixtures_dir().join(name);
    let wt = build_from_file(&path).unwrap_or_else(|e| panic!("build {name}: {e}"));
    // WebTemplate output is deterministic — no volatile fields to redact.
    // Duplicate ids take the spec suffix form (`name_1`, master04 §Node ID
    // Generation Rules); there is no vendor-quirk variant.
    let snapshot = name.replace(['.', ' '], "_");
    insta::assert_json_snapshot!(snapshot, wt);
}

#[test]
fn golden_demo_vitals() {
    golden("Demo Vitals.opt");
}

#[test]
fn golden_diagnosis() {
    golden("Diagnosis.opt");
}

#[test]
fn golden_medication_list() {
    golden("medication_list.opt");
}

// ── targeted assertions ──────────────────────────────────────────────────────

#[test]
fn demo_vitals_shapes() {
    let wt =
        build_from_file(&better_fixtures_dir().join("Demo Vitals.opt")).expect("build Demo Vitals");

    // Root is a COMPOSITION with the format version and a template id.
    assert_eq!(wt.tree.rm_type, "COMPOSITION");
    assert_eq!(wt.version, "2.3");
    assert!(!wt.template_id.is_empty());
    assert!(!wt.default_language.is_empty());
    assert!(wt.languages.contains(&wt.default_language));

    // Every node has a non-empty snake-cased id; occurrences are consistent
    // (max == -1 for unbounded, else >= min).
    let mut node_count = 0usize;
    for_each_node(&wt.tree, &mut |n| {
        node_count += 1;
        assert!(!n.id.is_empty(), "empty id on {}", n.aql_path);
        assert!(
            n.id.chars().all(|c| c.is_ascii_lowercase()
                || c.is_ascii_digit()
                || matches!(c, '_' | '.' | '-')),
            "non-sanitized id {:?}",
            n.id
        );
        if n.max != -1 {
            let min = n.min.unwrap_or(0);
            assert!(n.max >= min, "max {} < min {min} at {}", n.max, n.aql_path);
        }
    });
    assert!(
        node_count > 5,
        "expected a populated tree, got {node_count} nodes"
    );

    // A quantity leaf carries |magnitude (DECIMAL) + |unit inputs (|unit singular).
    let quantity = find_node(&wt.tree, &|n| {
        n.inputs
            .iter()
            .any(|i| i.suffix.as_deref() == Some("magnitude"))
    })
    .expect("a DV_QUANTITY node with a |magnitude input");
    let suffixes: Vec<&str> = quantity
        .inputs
        .iter()
        .filter_map(|i| i.suffix.as_deref())
        .collect();
    assert!(
        suffixes.contains(&"magnitude"),
        "quantity inputs: {suffixes:?}"
    );
    assert!(
        suffixes.contains(&"unit"),
        "quantity inputs (|unit singular): {suffixes:?}"
    );

    // aql paths carry archetype/atNNNN predicates.
    assert!(
        find_node(&wt.tree, &|n| n.aql_path.contains("[at")).is_some(),
        "expected an aqlPath with an [atNNNN] predicate"
    );
    assert!(
        find_node(&wt.tree, &|n| n.aql_path.contains("[openEHR-EHR-")).is_some(),
        "expected an aqlPath with an [openEHR-EHR-...] archetype predicate"
    );
}

#[test]
fn coded_text_has_code_input_and_list() {
    // A CODED_TEXT leaf (with a local coded list) has a single CODED_TEXT input
    // suffixed `code` carrying a non-empty coded list (Better `CodedTextTest`).
    let wt =
        build_from_file(&better_fixtures_dir().join("Diagnosis.opt")).expect("build Diagnosis");
    let coded = find_node(&wt.tree, &|n| {
        n.inputs.iter().any(|i| {
            i.suffix.as_deref() == Some("code")
                && matches!(i.input_type, WebTemplateInputType::CodedText)
                && !i.list.is_empty()
        })
    })
    .expect("a coded-text node with a |code input + coded list");

    let code_input = coded
        .inputs
        .iter()
        .find(|i| i.suffix.as_deref() == Some("code"))
        .expect("code input");
    assert!(
        !code_input.list.is_empty(),
        "coded list should be populated"
    );
    for cv in &code_input.list {
        assert!(!cv.value.is_empty(), "coded value has a code");
    }
}

#[test]
fn ordinal_values_carry_ordinal_integers() {
    // DV_ORDINAL → a single CODED_TEXT input whose list entries carry `ordinal`
    // integers (Better `OrdinalWebTemplateInputBuilder`).
    let wt = build_from_file(&better_fixtures_dir().join("Testing Template N.opt"))
        .expect("build Testing Template N");
    let ordinal = find_node(&wt.tree, &|n| {
        n.inputs
            .iter()
            .any(|i| i.list.iter().any(|cv| cv.ordinal.is_some()))
    });
    assert!(
        ordinal.is_some(),
        "expected at least one DV_ORDINAL node with ordinal-tagged coded values"
    );
}

#[test]
fn ids_are_deduplicated_within_a_parent() {
    let wt = build_from_file(&better_fixtures_dir().join("Testing Template N.opt"))
        .expect("build Testing Template N");
    assert_unique_sibling_ids(&wt.tree);
}

// ── existence ∧ occurrences (ADL 1.4 master05-cadl §Occurrences) ─────────────

/// A child of an OPTIONAL single attribute is optional whatever occurrences the
/// constraint object carries.
///
/// ADL 1.4 `AM/docs/ADL1.4/master05-cadl.adoc` §Occurrences: for the value of a
/// single-valued attribute the occurrences "can only be `0..1` or `1..1`, and
/// this is already defined by the attribute `existence`". `action test.opt`
/// carries the canonical shape — `ISM_TRANSITION.careflow_step` with
/// `existence {0..1}` over a `DV_CODED_TEXT` with `occurrences {1..1}` — and RM
/// declares that attribute `0..1`
/// (`RM/docs/UML/classes/org.openehr.rm.composition.ism_transition.adoc`;
/// Simplified Formats `master05-rm_mapping.adoc` §`ISM_TRANSITION` Required "no").
#[test]
fn optional_single_attribute_never_yields_a_mandatory_child() {
    let wt =
        build_from_file(&better_fixtures_dir().join("action test.opt")).expect("build action test");

    let mut checked = 0usize;
    for_each_node(&wt.tree, &mut |n| {
        if n.aql_path.ends_with("/ism_transition/careflow_step") {
            checked += 1;
            assert_eq!(
                n.min,
                Some(0),
                "careflow_step is existence 0..1, so its child cannot be mandatory at {}",
                n.aql_path
            );
        }
        // The mandatory sibling keeps its bound: `current_state` is
        // existence 1..1 over occurrences 1..1 (RM Required "yes").
        if n.aql_path.ends_with("/ism_transition/current_state") {
            checked += 1;
            assert_eq!(
                n.min,
                Some(1),
                "current_state is existence 1..1 and must stay mandatory at {}",
                n.aql_path
            );
        }
    });
    assert!(
        checked >= 2,
        "expected the ism_transition pair in the tree, saw {checked} nodes"
    );
}

// ── term bindings (node + coded-value level) ─────────────────────────────────

#[test]
fn ontology_term_bindings_populate_node_and_coded_value_bindings() {
    // A template whose archetype ontology carries `<term_bindings>` yields
    // Better-shaped `termBindings` both on nodes (matched by the node's
    // constraint node id — `WebTemplateBuilder.setTermBindings`) and on coded
    // values (matched by the option code — `CodePhraseWebTemplateInputBuilder`).
    let wt = build_from_file(&better_fixtures_dir().join("Across - Visual Acuity Report.opt"))
        .expect("build Across - Visual Acuity Report");

    // Node-level: a node bound to SNOMED-CT 422673001.
    let node_binding =
        find_node_binding(&wt, "422673001").expect("a node bound to SNOMED-CT 422673001");
    assert_eq!(node_binding.terminology_id, "SNOMED-CT");
    // Better JSON shape/order: `value` then `terminologyId`.
    assert_eq!(
        serde_json::to_string(&node_binding).expect("serialize binding"),
        r#"{"value":"422673001","terminologyId":"SNOMED-CT"}"#
    );

    // Coded-value-level: a coded option bound to SNOMED-CT 362503005.
    let cv_binding = find_coded_value_binding(&wt, "362503005")
        .expect("a coded value bound to SNOMED-CT 362503005");
    assert_eq!(cv_binding.terminology_id, "SNOMED-CT");

    assert_every_binding_well_formed(&wt);
}

/// The SNOMED-CT node-level binding carrying `value`, if the tree has one.
fn find_node_binding(
    wt: &openehr_sdt::flat::webtemplate::model::WebTemplate,
    value: &str,
) -> Option<openehr_sdt::flat::webtemplate::model::WebTemplateBindingCodedValue> {
    let mut found = None;
    for_each_node(&wt.tree, &mut |n| {
        if let Some(b) = n.term_bindings.get("SNOMED-CT")
            && b.value == value
        {
            found = Some(b.clone());
        }
    });
    found
}

/// The SNOMED-CT coded-value binding carrying `value`, if the tree has one.
fn find_coded_value_binding(
    wt: &openehr_sdt::flat::webtemplate::model::WebTemplate,
    value: &str,
) -> Option<openehr_sdt::flat::webtemplate::model::WebTemplateBindingCodedValue> {
    let mut found = None;
    for_each_node(&wt.tree, &mut |n| {
        for input in &n.inputs {
            for cv in &input.list {
                if let Some(b) = cv.term_bindings.get("SNOMED-CT")
                    && b.value == value
                {
                    found = Some(b.clone());
                }
            }
        }
    });
    found
}

/// Every emitted binding — node- or coded-value-level — is well-formed.
fn assert_every_binding_well_formed(wt: &openehr_sdt::flat::webtemplate::model::WebTemplate) {
    for_each_node(&wt.tree, &mut |n| {
        for (term, b) in &n.term_bindings {
            assert!(!term.is_empty(), "empty terminology key at {}", n.aql_path);
            assert!(
                !b.value.is_empty() && !b.terminology_id.is_empty(),
                "incomplete node binding at {}",
                n.aql_path
            );
        }
        for input in &n.inputs {
            for cv in &input.list {
                for (term, b) in &cv.term_bindings {
                    assert!(
                        !term.is_empty(),
                        "empty terminology key on a coded value at {}",
                        n.aql_path
                    );
                    assert!(
                        !b.value.is_empty() && !b.terminology_id.is_empty(),
                        "incomplete coded-value binding at {}",
                        n.aql_path
                    );
                }
            }
        }
    });
}

// ── multiple coded-text compaction ───────────────────────────────────────────

#[test]
fn multiple_coded_text_alternatives_compact_to_one_node() {
    // `action test.opt` constrains an ELEMENT value with two `DV_CODED_TEXT`
    // alternatives (a local coded list + an unconstrained one). Better's
    // `compactMultipleCodedTexts` merges them into ONE coded node carrying the
    // union of the coded values — not two polymorphic `value`/`value2` siblings.
    let wt =
        build_from_file(&better_fixtures_dir().join("action test.opt")).expect("build action test");

    let suffix = "items[at0061]/items[at0052]/value";
    let mut at_path: Vec<(String, String, Vec<String>)> = Vec::new();
    for_each_node(&wt.tree, &mut |n| {
        if n.aql_path.ends_with(suffix) {
            let codes: Vec<String> = n
                .inputs
                .iter()
                .find(|i| i.suffix.as_deref() == Some("code"))
                .map(|i| i.list.iter().map(|c| c.value.clone()).collect())
                .unwrap_or_default();
            at_path.push((n.rm_type.clone(), n.id.clone(), codes));
        }
    });

    // Exactly one node at the choice path (the two alternatives merged into one).
    assert_eq!(
        at_path.len(),
        1,
        "expected a single compacted coded node at .../{suffix}, got {at_path:?}"
    );
    let (rm_type, id, codes) = &at_path[0];
    assert_eq!(rm_type, "DV_CODED_TEXT");
    // The merged list is the union of the alternatives' codes (dedup, in order):
    // the constrained alternative's five codes; the unconstrained one adds none.
    let codes: Vec<&str> = codes.iter().map(String::as_str).collect();
    assert_eq!(
        codes,
        ["at0053", "at0054", "at0055", "at0056", "at0058"],
        "merged coded list"
    );
    // Not split into a polymorphic `value`/`value2` pair.
    assert_ne!(id, "coded_text_value");
    assert_ne!(id, "coded_text_value2");
}
// ── the PARTY_PROXY family (master05 §§PARTY_SELF, PARTY_IDENTIFIED, PARTY_RELATED) ──

/// The four party TEXT suffixes every `PARTY_PROXY`-family node carries — the
/// rows master05's three subtype tables share (`|id`/`|id_scheme`/
/// `|id_namespace`, plus `|name` on the two identified subtypes).
const PARTY_SUFFIXES: [&str; 4] = ["id", "id_scheme", "id_namespace", "name"];

fn party_suffixes(node: &WebTemplateNode) -> Vec<&str> {
    node.inputs
        .iter()
        .filter_map(|i| i.suffix.as_deref())
        .collect()
}

/// A slot an OPT narrows to `PARTY_RELATED` is a party LEAF, exactly like one
/// left at `PARTY_PROXY`/`PARTY_IDENTIFIED`: master05 gives all three subtypes
/// their own mapping table and they share the party suffix rows. The subtype's
/// extra `relationship` is a `DV_CODED_TEXT` SUB-PATH (master05 §"`PARTY_RELATED`
/// performer": "the `relationship` attribute is emitted as a sub-path under the
/// participation, with the standard `DV_CODED_TEXT` suffixes"), so it is a CHILD
/// of the party node — never a reason to demote the party to an inputless
/// container, which is the divergence this asserts against.
#[test]
fn party_related_narrowing_is_a_party_leaf_with_a_relationship_child() {
    let wt = build_from_file(&better_fixtures_dir().join("Test constrained subject.opt"))
        .expect("build Test constrained subject");
    let party = find_node(&wt.tree, &|n| n.rm_type == "PARTY_RELATED")
        .expect("the template narrows a party slot to PARTY_RELATED");

    assert_eq!(
        party_suffixes(party),
        PARTY_SUFFIXES,
        "a PARTY_RELATED node carries the shared party suffixes"
    );
    assert!(
        party
            .inputs
            .iter()
            .all(|i| matches!(i.input_type, WebTemplateInputType::Text)),
        "every party suffix is a TEXT input: {:?}",
        party.inputs
    );

    let relationship = party
        .children
        .iter()
        .find(|c| c.aql_path.ends_with("/relationship"))
        .expect("the narrowed relationship is a child sub-path of the party node");
    assert_eq!(
        relationship.rm_type, "DV_CODED_TEXT",
        "master05 §PARTY_RELATED types the relationship sub-path DV_CODED_TEXT"
    );
    assert!(
        relationship
            .inputs
            .iter()
            .any(|i| i.suffix.as_deref() == Some("code")),
        "the relationship child carries the DV_CODED_TEXT suffixes"
    );
}

/// The same rule as a standing invariant over every readable OPT in both
/// corpora: no node of the `PARTY_PROXY` family may come out inputless.
/// Constraining an attribute of a party — which is exactly what a
/// `PARTY_RELATED` narrowing does — must not cost the node its own suffixes.
#[test]
fn no_party_node_is_inputless() {
    let mut checked = 0_usize;
    let mut offenders: Vec<String> = Vec::new();
    let mut files: Vec<PathBuf> = crate::common::opt_files(&better_fixtures_dir());
    files.extend(crate::common::opt_files(&corpus_dir()));
    for path in files {
        let Ok(wt) = build_from_file(&path) else {
            continue; // parser gaps are reported by the smoke gate, not here
        };
        for_each_node(&wt.tree, &mut |n| {
            if !matches!(
                n.rm_type.as_str(),
                "PARTY_PROXY" | "PARTY_IDENTIFIED" | "PARTY_RELATED"
            ) {
                return;
            }
            checked += 1;
            if party_suffixes(n) != PARTY_SUFFIXES {
                offenders.push(format!(
                    "{}: {} at {} has inputs {:?}",
                    path.display(),
                    n.rm_type,
                    n.aql_path,
                    party_suffixes(n)
                ));
            }
        });
    }
    assert!(checked > 0, "expected the corpora to contain party nodes");
    assert!(
        offenders.is_empty(),
        "party nodes without the master05 party suffixes:\n{}",
        offenders.join("\n")
    );
}

/// Every OPT 1.4 constraint the archetype-conformance walk cannot evaluate is
/// REPORTED rather than dropped: the deployed corpus constrains computed RM
/// FUNCTIONS as if they were stored members (`EVENT.offset`,
/// `DV_PROPORTION.is_integral`) and misspells `null_flavour`, none of which a
/// conformant instance can carry. The skip is a template property, so it is
/// enumerable from the template alone.
#[test]
fn unenforceable_constraints_are_reported_not_dropped() {
    use openehr_sdt::flat::validation::{UnenforceableReason, unenforceable_existence_constraints};

    let mut seen: Vec<String> = Vec::new();
    for path in crate::common::opt_dirs()
        .iter()
        .flat_map(|dir| crate::common::opt_files(dir))
    {
        let Ok(wt) = build_from_file(&path) else {
            continue;
        };
        for skipped in unenforceable_existence_constraints(&wt) {
            assert_eq!(skipped.reason, UnenforceableReason::AttributeNotInRmModel);
            assert!(
                skipped.path.ends_with(&skipped.attribute),
                "the reported path must end with the reported attribute, got {} / {}",
                skipped.path,
                skipped.attribute
            );
            seen.push(skipped.attribute.clone());
        }
    }
    seen.sort();
    seen.dedup();
    assert!(
        !seen.is_empty(),
        "the vendored OPT corpus carries unenforceable existence constraints; \
         reporting none means the skip went silent again"
    );

    // Whatever the corpus contains, NOTHING reported may be an attribute the RM
    // actually declares — that would be a real constraint wrongly skipped.
    for attr in &seen {
        assert!(
            !openehr_rm::v1_2::model::classes()
                .any(|c| c.attributes.iter().any(|a| a.name == attr)),
            "'{attr}' IS declared by the RM: skipping it would drop an enforceable constraint"
        );
    }
}

/// The slot-pattern probe reads the assertion's EXPRESSION TREE, not its string
/// form: `ASSERTION.expression` is the "Root of expression tree" while
/// `string_expression` is only its optional "String form of expression"
/// (`AM UML/classes/org.openehr.am.aom14.assertion.adoc` §ASSERTION Class).
///
/// `IDCR Problem List.v1.opt` is the pin because none of its nine `<includes>`
/// carries a `string_expression` at all — a source-text scan sees zero of them
/// and leaves every slot open to any archetype.
#[test]
fn opt14_slot_patterns_come_from_the_assertion_tree() {
    let path = corpus_dir().join("knowledge/IDCR Problem List.v1.opt");
    let xml = std::fs::read_to_string(&path).expect("read the IDCR corpus OPT");
    assert!(
        !xml.contains("string_expression"),
        "this pin depends on the fixture carrying no string_expression; \
         re-pick the fixture if the vendored file changed"
    );
    let opt = opt14::from_xml(&xml).expect("the IDCR OPT parses");
    let wt = build_web_template(&opt).expect("build the IDCR web template");

    let includes = slot_includes(&wt.tree);
    assert!(
        includes.contains(
            &r"openEHR-EHR-EVALUATION\.problem_diagnosis(-[a-zA-Z0-9_]+)*\.v1".to_owned()
        ),
        "the at0002 problem/diagnosis slot must carry its archetype-id regex, got {includes:?}"
    );
    assert!(
        !includes.iter().any(String::is_empty),
        "no slot may carry an empty pattern, got {includes:?}"
    );
}

/// Every OPT the issue names as silently unconstrained must now constrain its
/// slots (ADL 1.4 `master05-cadl.adoc` §Archetype Slots: a slot's fillers are
/// exactly the archetypes its `include`/`exclude` assertions admit).
#[test]
fn corpus_slots_are_constrained_without_a_string_expression() {
    for name in [
        "knowledge/IDCR Problem List.v1.opt",
        "knowledge/IDCR Allergies List.v0.opt",
        "knowledge/Vital Signs Encounter (Composition).opt",
    ] {
        let wt = build_from_file(&corpus_dir().join(name)).expect("build the corpus OPT");
        let includes = slot_includes(&wt.tree);
        assert!(
            !includes.is_empty(),
            "{name}: every slot include went missing — slot fillers are unconstrained"
        );
    }
}

/// Every `ARCHETYPE_SLOT` include pattern reachable from a built `WebTemplate`.
fn slot_includes(node: &WebTemplateNode) -> Vec<String> {
    let mut out = Vec::new();
    for attr in &node.closed_attributes {
        for slot in &attr.slots {
            out.extend(slot.includes.iter().cloned());
        }
    }
    for child in &node.children {
        out.extend(slot_includes(child));
    }
    out
}
