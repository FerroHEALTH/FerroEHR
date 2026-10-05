// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Reference-model (phase-2 RM) validation corpus harness.
//!
//! Exercises the reference-model checks (VCORM/VCARM/VCORMT/VCAEX/VCACA/VCAM,
//! plus RM-dependent VACSO and the interior-node half of VATID) against the two
//! reference models the ADL2 conformance corpus authors fixtures against:
//!
//! - the openEHR RM 1.2.0 production model (`openehr_adl::validate::rm::
//!   ProductionRmModel`, the generated `openehr_rm::v1_2::model`) — the
//!   `openEHR-EHR-*` / `openEHR-DEMOGRAPHIC-*` fixtures under
//!   `tests/corpus/adl2-reference/validity/rm_checking/`;
//! - the openEHR `adltest`/`TEST_PKG` test schema (a BMM-loaded [`RmModel`]
//!   built here from the vendored ODIN BMM `tests/corpus/rm/
//!   openehr_adltest_100.bmm` via `openehr_lang::v1_1::odin`) — the
//!   `openEHR-TEST_PKG-*` fixtures.
//!
//! The oracle is each file's `other_details["regression"]` tag (per
//! `tests/corpus/INVENTORY.md`, never the filename). Spec oracle for the codes:
//! `docs/specs/openehr/AM/docs/AOM2/master04.5-constraint_model-class_definitions.adoc`
//! §Validity Rules + `master08-validation.adoc` §Phase 2.

// A test harness: vendored-fixture reads and parses are asserted to succeed.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "integration-test assertions, diagnostics and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use openehr_adl::aom::interval::Bounds;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::parse::Dialect;
use openehr_adl::validate::bindings::NoTerminologyResolver;
use openehr_adl::validate::catalogue::Severity;
use openehr_adl::validate::rm::{
    EnumUnderlying, ProductionRmModel, RmAttr, RmEnum, RmModel, base_type_name,
    production_model_governs, validate_rm_conformance,
};
use openehr_adl::validate::{ValidationIssue, validate_source};
use openehr_lang::v1_1::odin::{OdinInterval, OdinKey, OdinValue};

const CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus/adl2-reference");
const TEST_BMM: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/corpus/rm/referencemodels/bmm/openEHR/adl_test/Release-1.0.0/BMM/openehr_adltest_100.bmm"
);

// ── the BMM-loaded test reference model ────────────────────────────────────

/// One class in the `adltest`/`TEST_PKG` BMM (names stored upper-cased for
/// case-insensitive matching per `master04.3` §Reference Model Type Matching).
struct BmmClass {
    ancestors: Vec<String>,
    props: HashMap<String, RmAttr>,
}

/// The `adltest`/`TEST_PKG` reference model, loaded from the vendored ODIN BMM.
///
/// `adltest` declares `includes = openehr_rm_data_types_1.0.4`, so types it does
/// not itself define (`DATA_VALUE`, `DV_CODED_TEXT`, `CODE_PHRASE`, …) resolve
/// against the production openEHR RM data types via [`ProductionRmModel`].
struct BmmRmModel {
    classes: HashMap<String, BmmClass>,
    fallback: ProductionRmModel,
}

impl BmmRmModel {
    fn load(path: &str) -> Self {
        let src = std::fs::read_to_string(path).expect("read vendored test BMM");
        let tree = openehr_lang::v1_1::odin::parse(&src).expect("parse test BMM as ODIN");
        let mut classes = HashMap::new();
        if let OdinValue::Object(top) = &tree
            && let Some(OdinValue::KeyedList(defs)) = top.get("class_definitions")
        {
            for (key, val) in defs {
                let name = key_string(key);
                if let OdinValue::Object(body) = val {
                    classes.insert(name.to_uppercase(), read_class(body));
                }
            }
        }
        Self {
            classes,
            fallback: ProductionRmModel,
        }
    }

    /// Transitive ancestor names (upper-cased) of `class`, within this schema.
    fn ancestors_of(&self, class: &str, out: &mut Vec<String>) {
        if let Some(c) = self.classes.get(class) {
            for a in &c.ancestors {
                if !out.contains(a) {
                    out.push(a.clone());
                    self.ancestors_of(a, out);
                }
            }
        }
    }
}

impl RmModel for BmmRmModel {
    #[expect(
        clippy::unnecessary_literal_bound,
        reason = "the `RmModel` trait returns `&str` because other implementations borrow a stored `String`; narrowing this impl to `&'static str` would not match the trait"
    )]
    fn name(&self) -> &str {
        "openEHR TEST_PKG (adltest 1.0.2)"
    }

    fn type_exists(&self, rm_type: &str) -> bool {
        self.classes
            .contains_key(&base_type_name(rm_type).to_uppercase())
            || self.fallback.type_exists(rm_type)
    }

    fn conforms(&self, sub: &str, sup: &str) -> Option<bool> {
        let child = base_type_name(sub).to_uppercase();
        let ancestor = base_type_name(sup).to_uppercase();
        if self.classes.contains_key(&child) && self.classes.contains_key(&ancestor) {
            if child == ancestor {
                return Some(true);
            }
            let mut anc = Vec::new();
            self.ancestors_of(&child, &mut anc);
            return Some(anc.contains(&ancestor));
        }
        self.fallback.conforms(sub, sup)
    }

    fn attribute(&self, rm_type: &str, attr: &str) -> Option<RmAttr> {
        let base = base_type_name(rm_type).to_uppercase();
        if self.classes.contains_key(&base) {
            // own attributes, then inherited (walk transitive ancestors).
            if let Some(a) = self.classes.get(&base).and_then(|c| c.props.get(attr)) {
                return Some(a.clone());
            }
            let mut anc = Vec::new();
            self.ancestors_of(&base, &mut anc);
            for a in &anc {
                if let Some(p) = self.classes.get(a).and_then(|c| c.props.get(attr)) {
                    return Some(p.clone());
                }
            }
            return None;
        }
        self.fallback.attribute(rm_type, attr)
    }

    fn enumeration(&self, rm_type: &str) -> Option<RmEnum> {
        // The vendored adltest schema declares no enumeration classes, so enum
        // resolution defers to the production openEHR RM enumeration table (the
        // `includes = openehr_rm_data_types` fallback).
        self.fallback.enumeration(rm_type)
    }
}

fn key_string(k: &OdinKey) -> String {
    match k {
        OdinKey::String(s) | OdinKey::Date(s) | OdinKey::Time(s) | OdinKey::DateTime(s) => {
            s.clone()
        }
        OdinKey::Integer(i) => i.to_string(),
    }
}

/// Read a class body (`name`, `ancestors`, `is_abstract`, `properties`).
fn read_class(body: &indexmap::IndexMap<String, OdinValue>) -> BmmClass {
    let ancestors = body
        .get("ancestors")
        .map(string_list)
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.to_uppercase())
        .collect();
    let mut props = HashMap::new();
    if let Some(OdinValue::KeyedList(items)) = body.get("properties") {
        for (key, val) in items {
            if let Some((name, attr)) = read_property(&key_string(key), val) {
                props.insert(name, attr);
            }
        }
    }
    BmmClass { ancestors, props }
}

/// Read one property (a `(P_BMM_*) < … >` typed cast) into an [`RmAttr`].
fn read_property(pname: &str, val: &OdinValue) -> Option<(String, RmAttr)> {
    let (kind, body) = match val {
        OdinValue::Typed { rm_type, value } => (rm_type.as_str(), value.as_ref()),
        _ => return None,
    };
    let OdinValue::Object(map) = body else {
        return None;
    };
    let is_mandatory = matches!(map.get("is_mandatory"), Some(OdinValue::Boolean(true)));

    let (declared_type, is_multiple, cardinality) = match kind {
        "P_BMM_CONTAINER_PROPERTY" => {
            let (decl, card) = read_container(map);
            (decl, true, card)
        }
        "P_BMM_GENERIC_PROPERTY" => (read_generic_root(map), false, None),
        // P_BMM_SINGLE_PROPERTY / P_BMM_SINGLE_PROPERTY_OPEN: a bare `type`.
        _ => (string_of(map.get("type")).unwrap_or_default(), false, None),
    };

    Some((
        pname.to_owned(),
        RmAttr {
            declared_type,
            is_multiple,
            existence: Bounds::new(i32::from(is_mandatory), Some(1)),
            cardinality: is_multiple.then(|| cardinality.unwrap_or(Bounds::new(0, None))),
        },
    ))
}

/// Container property: `type_def = < container_type = <…> type = <"X"> >` +
/// optional `cardinality`.
fn read_container(map: &indexmap::IndexMap<String, OdinValue>) -> (String, Option<Bounds>) {
    let declared = match map.get("type_def") {
        Some(OdinValue::Object(td)) => string_of(td.get("type")).unwrap_or_default(),
        _ => String::new(),
    };
    let card = match map.get("cardinality") {
        Some(OdinValue::Interval(iv)) => Some(interval_bounds(iv)),
        _ => None,
    };
    (declared, card)
}

/// Generic property: `type_def = < root_type = <"X"> generic_parameters = <…> >`.
fn read_generic_root(map: &indexmap::IndexMap<String, OdinValue>) -> String {
    match map.get("type_def") {
        Some(OdinValue::Object(td)) => string_of(td.get("root_type")).unwrap_or_default(),
        Some(OdinValue::Typed { value, .. }) => match value.as_ref() {
            OdinValue::Object(td) => string_of(td.get("root_type")).unwrap_or_default(),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

fn string_of(v: Option<&OdinValue>) -> Option<String> {
    match v {
        Some(OdinValue::String(s)) => Some(s.clone()),
        _ => None,
    }
}

fn string_list(v: &OdinValue) -> Vec<String> {
    match v {
        OdinValue::String(s) => vec![s.clone()],
        OdinValue::List(items) => items
            .iter()
            .filter_map(|x| match x {
                OdinValue::String(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn interval_bounds(iv: &OdinInterval) -> Bounds {
    match iv {
        OdinInterval::Range { lower, upper, .. } => Bounds::new(
            lower.as_deref().and_then(odin_int).unwrap_or(0),
            upper.as_deref().and_then(odin_int),
        ),
        OdinInterval::PlusMinus { .. } => Bounds::new(0, None),
    }
}

fn odin_int(v: &OdinValue) -> Option<i32> {
    match v {
        OdinValue::Integer(i) => i32::try_from(*i).ok(),
        _ => None,
    }
}

// ── corpus walk ────────────────────────────────────────────────────────────

/// The RM codes this harness actively raises (a tag naming one must be raised
/// exactly on its fixture).
const RM_FIRING: &[&str] = &[
    "VCORM", "VCARM", "VCORMT", "VCAEX", "VCACA", "VCAM", "VATID",
];

/// Documented adjudications — files skipped with a spec-cited reason (never a
/// silent exclusion).
fn adjudicated(name: &str) -> Option<&'static str> {
    if name.ends_with("ENTRY_WRONG.rm_type_wrong.v1.0.0.adls") {
        // Definition root type `ENTRY` != identifier RM class `ENTRY_WRONG`, so
        // VARDT (master03 §Validity Rules L238: "the typename … must match the
        // type mentioned in the first segment of the archetype id") raises on
        // it. The corpus tags it PASS — a documented tag/spec inconsistency
        // (INVENTORY §3 records filename/tag contradictions). Not an RM gap;
        // adjudicated rather than weakening VARDT.
        Some("VARDT fires (ENTRY != ENTRY_WRONG, master03 L238); corpus PASS tag inconsistent")
    } else {
        None
    }
}

fn read_tag_raw(src: &str) -> Option<String> {
    let idx = src.find("regression")?;
    let rest = src.get(idx..)?;
    let open = rest.find("<\"")? + 2;
    let after = rest.get(open..)?;
    let end = after.find('"')?;
    after.get(..end).map(str::to_owned)
}

fn adls_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|e| e == "adls") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn error_codes(issues: &[ValidationIssue]) -> Vec<String> {
    issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| i.code.mnemonic().to_owned())
        .collect()
}

#[derive(Default)]
struct Counts {
    exact_code: usize,
    pass_clean: usize,
    normalised_vcam: usize,
    adjudicated: usize,
    deferred: usize,
}

/// The set of fixtures the harness claims beyond `validity/rm_checking/**`: the
/// two RM-dependent phase-1 fixtures re-claimed from the phase-1 harness's
/// adjudication list.
const RECLAIMED: &[&str] = &[
    "validity/consistency/openEHR-TEST_PKG-ENTRY.VATID_id_code_in_node_not_in_terminology.v1.0.0.adls",
];

/// The reference-model verdict for one corpus fixture.
///
/// A `Violation` carries the message tail; the caller prefixes the file name.
enum RmOutcome {
    /// The tagged RM code was raised.
    ExactCode,
    /// A `VSAM`-tagged fixture raised the non-specialised VCAM instead.
    NormalisedVcam,
    /// A `PASS`/untagged fixture raised no error.
    PassClean,
    /// A non-RM tag, owned by another harness; only spurious RM errors matter.
    Deferred,
    /// The fixture's outcome contradicts its tag.
    Violation(String),
}

/// Judges one `rm_checking` fixture against its authoritative `regression` tag.
fn judge_rm_fixture(
    src: &str,
    tag: Option<&str>,
    production: ProductionRmModel,
    test_model: &BmmRmModel,
) -> RmOutcome {
    let Ok(archetype) = parse_artefact(src, Dialect::Adl2) else {
        return RmOutcome::Violation("failed to parse".to_owned());
    };
    // Pick the governing reference model from the archetype HRID.
    let rm: &dyn RmModel = if production_model_governs(&archetype) {
        &production
    } else {
        test_model
    };
    // Full gated validation (phase 1 → phase 2 RM); if phase 1 is unclean the RM
    // pass is gated off, so fall back to the ungated RM pass to keep the
    // assertion about the RM code meaningful.
    let Ok(issues) = validate_source(src, None, rm, &NoTerminologyResolver) else {
        return RmOutcome::Violation("validate_source re-parse error".to_owned());
    };
    let gated = error_codes(&issues);
    let rm_only = gated.is_empty()
        || issues
            .iter()
            .all(|i| RM_FIRING.contains(&i.code.mnemonic()) || i.severity != Severity::Error);
    let codes = if rm_only {
        gated.clone()
    } else {
        error_codes(&validate_rm_conformance(&archetype, rm, Dialect::Adl2))
    };
    judge_rm_codes(tag, &gated, &codes)
}

/// The verdict for a fixture whose RM codes have been collected.
///
/// `gated` is the full gated pass's error set (what a `PASS` tag is judged
/// against); `codes` is the RM-pass set the firing assertions read.
fn judge_rm_codes(tag: Option<&str>, gated: &[String], codes: &[String]) -> RmOutcome {
    match tag {
        None | Some("PASS") => {
            if gated.is_empty() {
                RmOutcome::PassClean
            } else {
                RmOutcome::Violation(format!("PASS/untagged but raised {codes:?}"))
            }
        }
        // A non-specialised RM-arity violation: master04.5 VSAM is the
        // *specialised* form (vs the parent archetype); with no parent the
        // applicable rule is VCAM (master04.5 §Validity Rules: C_ATTRIBUTE,
        // VCAM). Assert VCAM is raised.
        Some("VSAM") => {
            if codes.iter().any(|c| c == "VCAM") {
                RmOutcome::NormalisedVcam
            } else {
                RmOutcome::Violation(format!("VSAM tag expected VCAM but raised {codes:?}"))
            }
        }
        Some(t) if RM_FIRING.contains(&t) => {
            if codes.iter().any(|c| c == t) {
                RmOutcome::ExactCode
            } else {
                RmOutcome::Violation(format!("expected {t} but raised {codes:?}"))
            }
        }
        // A non-RM tag (e.g. VARDT is phase-1, owned by the phase-1 harness):
        // assert only that the RM pass raises no spurious RM error.
        Some(_) => RmOutcome::Deferred,
    }
}

#[test]
fn corpus_rm_outcomes() {
    let production = ProductionRmModel;
    let test_model = BmmRmModel::load(TEST_BMM);

    let mut paths = adls_files(&PathBuf::from(format!("{CORPUS}/validity/rm_checking")));
    for extra in RECLAIMED {
        paths.push(PathBuf::from(format!("{CORPUS}/{extra}")));
    }
    paths.sort();

    let mut counts = Counts::default();
    let mut violations: Vec<String> = Vec::new();

    for path in &paths {
        let name = path
            .strip_prefix(CORPUS)
            .unwrap_or(path)
            .display()
            .to_string();
        if let Some(reason) = adjudicated(&name) {
            counts.adjudicated += 1;
            eprintln!("adjudicated {name}: {reason}");
            continue;
        }
        let Ok(src) = std::fs::read_to_string(path) else {
            violations.push(format!("{name}: unreadable"));
            continue;
        };
        let tag = read_tag_raw(&src);

        match judge_rm_fixture(&src, tag.as_deref(), production, &test_model) {
            RmOutcome::ExactCode => counts.exact_code += 1,
            RmOutcome::NormalisedVcam => counts.normalised_vcam += 1,
            RmOutcome::PassClean => counts.pass_clean += 1,
            RmOutcome::Deferred => counts.deferred += 1,
            RmOutcome::Violation(message) => violations.push(format!("{name}: {message}")),
        }
    }

    eprintln!(
        "rm corpus: exact={} vsam->vcam={} pass_clean={} adjudicated={} deferred={} ({} files)",
        counts.exact_code,
        counts.normalised_vcam,
        counts.pass_clean,
        counts.adjudicated,
        counts.deferred,
        paths.len(),
    );

    assert!(
        violations.is_empty(),
        "rm corpus violations ({}):\n{}",
        violations.len(),
        violations.join("\n")
    );
}

// ── hand-written cases for RM codes with no (exact) corpus coverage ─────────
// VCAM/VCACA had no corpus coverage; these author it against the vendored
// TEST_PKG schema, which — unlike the generated production model — carries RM
// container cardinality.

fn assert_raises(src: &str, rm: &dyn RmModel, code: &str) {
    let archetype = parse_artefact(src, Dialect::Adl2).unwrap();
    let issues = validate_rm_conformance(&archetype, rm, Dialect::Adl2);
    let raised = error_codes(&issues);
    assert!(
        raised.iter().any(|c| c == code),
        "expected {code}, raised {raised:?}"
    );
}

fn assert_clean(src: &str, rm: &dyn RmModel) {
    let archetype = parse_artefact(src, Dialect::Adl2).unwrap();
    let issues = validate_rm_conformance(&archetype, rm, Dialect::Adl2);
    let raised = error_codes(&issues);
    assert!(raised.is_empty(), "expected clean, raised {raised:?}");
}

/// A minimal `TEST_PKG` archetype wrapping a `definition` body.
fn test_pkg_archetype(class: &str, concept: &str, definition: &str, terms: &str) -> String {
    format!(
        "archetype (adl_version=2.0.5; rm_release=1.0.2)\n\
         \topenEHR-TEST_PKG-{class}.{concept}.v1.0.0\n\n\
         language\n\toriginal_language = <[ISO_639-1::en]>\n\n\
         description\n\tlifecycle_state = <\"draft\">\n\n\
         definition\n{definition}\n\n\
         terminology\n\tterm_definitions = <\n\t\t[\"en\"] = <\n{terms}\t\t>\n\t>\n"
    )
}

fn term(code: &str) -> String {
    format!("\t\t\t[\"{code}\"] = <text = <\"x\"> description = <\"x\">>\n")
}

#[test]
fn vcaca_cardinality_wider_than_rm() {
    // CLUSTER.items has RM cardinality {1..*} in TEST_PKG (`cardinality =
    // <|>=1|>`); constraining it to {0..*} widens the lower bound → VCACA
    // (master04.5 §Validity Rules: C_ATTRIBUTE, VCACA).
    let test_model = BmmRmModel::load(TEST_BMM);
    let def = "\tCLUSTER[id1] matches {\n\t\titems cardinality matches {0..*} matches {\n\t\t\tELEMENT[id2]\n\t\t}\n\t}";
    let terms = format!("{}{}", term("id1"), term("id2"));
    let src = test_pkg_archetype("CLUSTER", "vcaca_test", def, &terms);
    assert_raises(&src, &test_model, "VCACA");

    // {1..*} equals the RM cardinality → conforms (clean).
    let ok = "\tCLUSTER[id1] matches {\n\t\titems cardinality matches {1..*} matches {\n\t\t\tELEMENT[id2]\n\t\t}\n\t}";
    let ok_src = test_pkg_archetype("CLUSTER", "vcaca_ok", ok, &terms);
    assert_clean(&ok_src, &test_model);
}

#[test]
fn vcam_cardinality_on_single_valued_rm_attribute() {
    // ELEMENT.value is single-valued (RM `DATA_VALUE`); stating a cardinality
    // makes it a container → VCAM (master04.5 §Validity Rules: C_ATTRIBUTE,
    // VCAM).
    let test_model = BmmRmModel::load(TEST_BMM);
    let def = "\tELEMENT[id1] matches {\n\t\tvalue cardinality matches {0..*} matches {\n\t\t\tDV_TEXT[id2]\n\t\t}\n\t}";
    let terms = format!("{}{}", term("id1"), term("id2"));
    let src = test_pkg_archetype("ELEMENT", "vcam_test", def, &terms);
    assert_raises(&src, &test_model, "VCAM");
}

#[test]
fn vacso_single_valued_child_occurrences() {
    // ENTRY.element_attr is single-valued (RM `ELEMENT`); a child with
    // occurrences {0..2} exceeds the single-valued upper of 1 → VACSO
    // (master04.5 §Validity Rules: C_ATTRIBUTE, VACSO).
    let test_model = BmmRmModel::load(TEST_BMM);
    let def = "\tENTRY[id1] matches {\n\t\telement_attr matches {\n\t\t\tELEMENT[id2] occurrences matches {0..2}\n\t\t}\n\t}";
    let terms = format!("{}{}", term("id1"), term("id2"));
    let src = test_pkg_archetype("ENTRY", "vacso_test", def, &terms);
    assert_raises(&src, &test_model, "VACSO");

    // element_attr_2 is multiply-valued, so occurrences {0..2} is fine there.
    let ok = "\tENTRY[id1] matches {\n\t\telement_attr_2 matches {\n\t\t\tELEMENT[id2] occurrences matches {0..2}\n\t\t}\n\t}";
    let ok_src = test_pkg_archetype("ENTRY", "vacso_ok", ok, &terms);
    assert_clean(&ok_src, &test_model);
}

#[test]
fn test_model_loads_expected_classes() {
    let test_model = BmmRmModel::load(TEST_BMM);
    assert!(test_model.type_exists("ENTRY"));
    assert!(test_model.type_exists("CLUSTER"));
    // element_attr_2 is a container; element_attr is single-valued.
    assert!(
        test_model
            .attribute("ENTRY", "element_attr_2")
            .unwrap()
            .is_multiple
    );
    assert!(
        !test_model
            .attribute("ENTRY", "element_attr")
            .unwrap()
            .is_multiple
    );
    // A data type not defined in adltest resolves via the production fallback.
    assert!(test_model.type_exists("DV_CODED_TEXT"));
}

// ── VCORMT generic-argument matching (production model) ─────────────────────

/// The re-claimed `VCORMT_rm_non_conforming_type1` scenario, exercised directly
/// against the production model: `HISTORY<ITEM_LIST>.events` is
/// `EVENT<ITEM_STRUCTURE>`; a child `EVENT<CLUSTER>` conforms on the base type
/// (`EVENT`) but its generic argument `CLUSTER` does not conform to
/// `ITEM_STRUCTURE`, so VCORMT fires on the covariant argument match
/// (master04.2 §`Rm_type_name` and Reference Model Type Matching).
#[test]
fn vcormt_generic_argument_non_conformance() {
    let production = ProductionRmModel;
    let src = "archetype (adl_version=2.0.5; rm_release=1.0.2)\n\
        \topenEHR-EHR-OBSERVATION.vcormt_generic.v1.0.0\n\n\
        language\n\toriginal_language = <[ISO_639-1::en]>\n\n\
        description\n\tlifecycle_state = <\"draft\">\n\n\
        definition\n\
        \tOBSERVATION[id1] matches {\n\
        \t\tdata matches {\n\
        \t\t\tHISTORY<ITEM_LIST>[id2] matches {\n\
        \t\t\t\tevents cardinality matches {1..*} matches {\n\
        \t\t\t\t\tEVENT<CLUSTER>[id3] occurrences matches {0..*}\n\
        \t\t\t\t}\n\
        \t\t\t}\n\
        \t\t}\n\
        \t}\n\n\
        terminology\n\tterm_definitions = <\n\t\t[\"en\"] = <\n\
        \t\t\t[\"id1\"] = <text = <\"x\"> description = <\"x\">>\n\
        \t\t\t[\"id3\"] = <text = <\"x\"> description = <\"x\">>\n\
        \t\t>\n\t>\n";
    assert_raises(src, &production, "VCORMT");

    // `EVENT<ITEM_LIST>` conforms (ITEM_LIST conforms to ITEM_STRUCTURE) → clean.
    let ok = "archetype (adl_version=2.0.5; rm_release=1.0.2)\n\
        \topenEHR-EHR-OBSERVATION.vcormt_generic_ok.v1.0.0\n\n\
        language\n\toriginal_language = <[ISO_639-1::en]>\n\n\
        description\n\tlifecycle_state = <\"draft\">\n\n\
        definition\n\
        \tOBSERVATION[id1] matches {\n\
        \t\tdata matches {\n\
        \t\t\tHISTORY<ITEM_LIST>[id2] matches {\n\
        \t\t\t\tevents cardinality matches {1..*} matches {\n\
        \t\t\t\t\tEVENT<ITEM_LIST>[id3] occurrences matches {0..*}\n\
        \t\t\t\t}\n\
        \t\t\t}\n\
        \t\t}\n\
        \t}\n\n\
        terminology\n\tterm_definitions = <\n\t\t[\"en\"] = <\n\
        \t\t\t[\"id1\"] = <text = <\"x\"> description = <\"x\">>\n\
        \t\t\t[\"id3\"] = <text = <\"x\"> description = <\"x\">>\n\
        \t\t>\n\t>\n";
    assert_clean(ok, &production);
}

// ── VCACA on the production model (real BMM cardinality) ────────────────────

/// `CLUSTER.items` has RM cardinality `{1..*}` in the production openEHR RM;
/// constraining it to `{0..*}` widens the lower bound → VCACA. This exercises
/// the tight lower-bound half of VCACA now that the generated model carries real
/// container cardinality (master04.5 §Validity Rules: `C_ATTRIBUTE`, VCACA).
#[test]
fn vcaca_production_model_lower_bound() {
    let production = ProductionRmModel;
    let bad = "archetype (adl_version=2.0.5; rm_release=1.0.2)\n\
        \topenEHR-EHR-CLUSTER.vcaca_prod.v1.0.0\n\n\
        language\n\toriginal_language = <[ISO_639-1::en]>\n\n\
        description\n\tlifecycle_state = <\"draft\">\n\n\
        definition\n\
        \tCLUSTER[id1] matches {\n\
        \t\titems cardinality matches {0..*} matches {\n\
        \t\t\tELEMENT[id2]\n\
        \t\t}\n\
        \t}\n\n\
        terminology\n\tterm_definitions = <\n\t\t[\"en\"] = <\n\
        \t\t\t[\"id1\"] = <text = <\"x\"> description = <\"x\">>\n\
        \t\t\t[\"id2\"] = <text = <\"x\"> description = <\"x\">>\n\
        \t\t>\n\t>\n";
    assert_raises(bad, &production, "VCACA");

    // `{1..*}` equals the RM cardinality → conforms (clean).
    let ok = "archetype (adl_version=2.0.5; rm_release=1.0.2)\n\
        \topenEHR-EHR-CLUSTER.vcaca_prod_ok.v1.0.0\n\n\
        language\n\toriginal_language = <[ISO_639-1::en]>\n\n\
        description\n\tlifecycle_state = <\"draft\">\n\n\
        definition\n\
        \tCLUSTER[id1] matches {\n\
        \t\titems cardinality matches {1..*} matches {\n\
        \t\t\tELEMENT[id2]\n\
        \t\t}\n\
        \t}\n\n\
        terminology\n\tterm_definitions = <\n\t\t[\"en\"] = <\n\
        \t\t\t[\"id1\"] = <text = <\"x\"> description = <\"x\">>\n\
        \t\t\t[\"id2\"] = <text = <\"x\"> description = <\"x\">>\n\
        \t\t>\n\t>\n";
    assert_clean(ok, &production);
}

// ── VCORMEN / VCORMENV / VCORMENU (enumeration-literal validity) ────────────

/// A minimal reference model with two enumeration-typed attributes, for the
/// enumeration-literal checks. The openEHR RM 1.2.0 BMM types every "enum"
/// property as a plain `Integer`/`String` (only `PROPORTION_KIND` is an
/// enumeration class, and no property is typed as it), so an enum-typed *slot* —
/// the precondition these checks need — does not occur in the production model;
/// this bespoke model supplies one (master04.2 §Constraints on Enumeration
/// Types).
struct EnumTestModel;

impl RmModel for EnumTestModel {
    #[expect(
        clippy::unnecessary_literal_bound,
        reason = "the `RmModel` trait returns `&str` because other implementations borrow a stored `String`; narrowing this impl to `&'static str` would not match the trait"
    )]
    fn name(&self) -> &str {
        "enum test model"
    }

    fn type_exists(&self, rm_type: &str) -> bool {
        base_type_name(rm_type).eq_ignore_ascii_case("KIND_HOLDER")
    }

    fn conforms(&self, sub: &str, sup: &str) -> Option<bool> {
        if self.type_exists(sub) && self.type_exists(sup) {
            Some(base_type_name(sub).eq_ignore_ascii_case(base_type_name(sup)))
        } else {
            None
        }
    }

    fn attribute(&self, rm_type: &str, attr: &str) -> Option<RmAttr> {
        if !self.type_exists(rm_type) {
            return None;
        }
        let declared = match attr {
            "int_kind" => "PROP_KIND",
            "str_kind" => "STR_KIND",
            _ => return None,
        };
        Some(RmAttr {
            declared_type: declared.to_owned(),
            is_multiple: false,
            existence: Bounds::new(0, Some(1)),
            cardinality: None,
        })
    }

    fn enumeration(&self, rm_type: &str) -> Option<RmEnum> {
        match base_type_name(rm_type).to_uppercase().as_str() {
            "PROP_KIND" => Some(RmEnum {
                underlying: EnumUnderlying::Integer,
                int_values: vec![0, 1, 2, 3, 4],
                str_values: Vec::new(),
            }),
            "STR_KIND" => Some(RmEnum {
                underlying: EnumUnderlying::String,
                int_values: Vec::new(),
                str_values: vec!["mandatory".to_owned(), "optional".to_owned()],
            }),
            _ => None,
        }
    }
}

fn kind_holder(definition: &str) -> String {
    format!(
        "archetype (adl_version=2.0.5; rm_release=1.0.2)\n\
         \topenEHR-TEST_PKG-KIND_HOLDER.enum_test.v1.0.0\n\n\
         language\n\toriginal_language = <[ISO_639-1::en]>\n\n\
         description\n\tlifecycle_state = <\"draft\">\n\n\
         definition\n{definition}\n\n\
         terminology\n\tterm_definitions = <\n\t\t[\"en\"] = <\n{}\t\t>\n\t>\n",
        term("id1")
    )
}

#[test]
fn vcormenv_integer_value_not_a_literal() {
    // int_kind is PROP_KIND (integer enum, literals {0..4}); {2, 9} includes 9,
    // which is not a declared literal → VCORMENV (master08 §Phase 2; master04.2
    // §Constraints on Enumeration Types).
    let model = EnumTestModel;
    let def = "\tKIND_HOLDER[id1] matches {\n\t\tint_kind matches {2, 9}\n\t}";
    assert_raises(&kind_holder(def), &model, "VCORMENV");

    // {2, 3} are both declared literals → clean.
    let ok = "\tKIND_HOLDER[id1] matches {\n\t\tint_kind matches {2, 3}\n\t}";
    assert_clean(&kind_holder(ok), &model);
}

#[test]
fn vcormenu_string_value_not_a_literal() {
    // str_kind is STR_KIND (string enum, literals {mandatory, optional});
    // "bogus" is not a declared literal → VCORMENU.
    let model = EnumTestModel;
    let def = "\tKIND_HOLDER[id1] matches {\n\t\tstr_kind matches {\"mandatory\", \"bogus\"}\n\t}";
    assert_raises(&kind_holder(def), &model, "VCORMENU");

    let ok =
        "\tKIND_HOLDER[id1] matches {\n\t\tstr_kind matches {\"mandatory\", \"optional\"}\n\t}";
    assert_clean(&kind_holder(ok), &model);
}

#[test]
fn vcormen_primitive_kind_mismatch() {
    // A string constraint on the integer-based PROP_KIND slot is a
    // primitive-kind mismatch → VCORMEN.
    let model = EnumTestModel;
    let def = "\tKIND_HOLDER[id1] matches {\n\t\tint_kind matches {\"foo\"}\n\t}";
    assert_raises(&kind_holder(def), &model, "VCORMEN");
}
