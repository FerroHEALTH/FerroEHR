// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Regression guard for the CNF-runner corpus vitals template
//! (`corpus/templates/vitals.opt`, corpus alias
//! `cnf.opt.vitals`, `template_id` `cnf.vitals`).
//!
//! The Simplified-Formats (SF-*) CNF schedule cases commit FLAT/STRUCTURED
//! instances against `cnf.opt.vitals` using
//! `vitals/body_temperature:i/any_event:i/...` field identifiers, so the built
//! `WebTemplate` MUST expose exactly those node ids. This test lives in
//! `openehr_sdt::flat` — not the conformance instrument, which stays a
//! dependency-light deterministic crate of its own — because the web-template
//! builder + OPT 1.4 parser it exercises live here. Node ids follow ITS-REST `simplified_formats`
//! `master04-basic_concepts.adoc` §"Node ID Generation Rules"; leaf suffixes
//! follow `master05-rm_mapping.adoc` §`DV_QUANTITY` / §`DV_CODED_TEXT`; the open vs
//! closed coded value-set distinction follows master04 §"Open Value-Sets and the
//! `|other` Suffix" (open = an `ELEMENT.value` `DV_CODED_TEXT | DV_TEXT` choice,
//! AOM 1.4 `masterAppA`; closed = a `DV_CODED_TEXT` with a local `code_list`).

use std::path::PathBuf;

use openehr_its::opt14;
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use openehr_sdt::flat::webtemplate::model::WebTemplateNode;

fn vitals_opt() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/templates/vitals.opt")
}

fn collect<'a>(node: &'a WebTemplateNode, out: &mut Vec<&'a WebTemplateNode>) {
    out.push(node);
    for child in &node.children {
        collect(child, out);
    }
}

#[test]
fn vitals_template_exposes_the_sf_flat_paths() {
    let xml = std::fs::read_to_string(vitals_opt()).expect("read vitals.opt");
    let opt = opt14::from_xml(&xml).expect("parse vitals.opt");
    let wt = build_web_template(&opt).expect("build vitals web template");

    let mut nodes = Vec::new();
    collect(&wt.tree, &mut nodes);

    // Diagnostic dump: full_id | rm_type | min..max | input suffixes | list_open
    for n in &nodes {
        let suffixes: Vec<String> = n
            .inputs
            .iter()
            .map(|i| {
                format!(
                    "{}{}",
                    i.suffix.as_deref().unwrap_or("_"),
                    match i.list_open {
                        Some(true) => "(open)",
                        Some(false) => "(closed)",
                        None => "",
                    }
                )
            })
            .collect();
        println!(
            "{:<60} {:<16} {}..{:<3} inputs={:?} list={}",
            n.full_id,
            n.rm_type,
            n.min.map_or("_".to_owned(), |m| m.to_string()),
            n.max,
            suffixes,
            n.inputs.iter().map(|i| i.list.len()).sum::<usize>(),
        );
    }

    let by_id = |id: &str| -> &WebTemplateNode {
        nodes
            .iter()
            .find(|n| n.full_id == id)
            .unwrap_or_else(|| panic!("no web-template node with full_id {id:?}"))
    };
    let has_suffix =
        |n: &WebTemplateNode, s: &str| n.inputs.iter().any(|i| i.suffix.as_deref() == Some(s));

    // template_id must match the manifest alias + the update_composition-wrong_template reference.
    assert_eq!(wt.template_id, "cnf.vitals", "template_id");

    // Root: COMPOSITION "Vitals" -> WT id `vitals` (master04 §Node ID Generation Rules).
    assert_eq!(wt.tree.rm_type, "COMPOSITION");
    assert_eq!(wt.tree.full_id, "vitals", "WT root id");

    // OBSERVATION `body_temperature`, unbounded (body_temperature:i expand to
    // distinct OBSERVATIONs — SF-INDEX-semantics, master04 §Instance
    // Indexing; the SF-FLAT-reject_cardinality ground is the `temperature`
    // ELEMENT's 0..1 bound below, master04 §Validation).
    let obs = by_id("vitals/body_temperature");
    assert_eq!(obs.rm_type, "OBSERVATION");
    assert_eq!(obs.max, -1, "body_temperature is unbounded (0..*)");

    // EVENT `any_event`, unbounded (any_event:i expand to distinct EVENTs —
    // SF-INDEX-multi_event_commit, master04 §Instance Indexing).
    let ev = by_id("vitals/body_temperature/any_event");
    assert!(
        ev.rm_type.ends_with("EVENT"),
        "any_event rm_type: {}",
        ev.rm_type
    );
    assert_eq!(ev.max, -1, "any_event must be unbounded (max=-1)");

    // EVENT.time (master05 §POINT_EVENT/§EVENT; SF-INDEX-multi_event_commit reads
    // events[i]/time).
    by_id("vitals/body_temperature/any_event/time");

    // Temperature = DV_QUANTITY with |magnitude + |unit (master05 §DV_QUANTITY).
    let temp = by_id("vitals/body_temperature/any_event/temperature");
    assert_eq!(temp.rm_type, "DV_QUANTITY");
    assert!(
        has_suffix(temp, "magnitude") && has_suffix(temp, "unit"),
        "temperature needs |magnitude + |unit"
    );

    // Symptom = OPEN coded value-set (master04 §Open Value-Sets: DV_CODED_TEXT|DV_TEXT
    // choice) -> |code is list-open AND a |other free-text suffix is exposed.
    let symptom = by_id("vitals/body_temperature/any_event/symptom");
    assert_eq!(symptom.rm_type, "DV_CODED_TEXT");
    let symptom_code = symptom
        .inputs
        .iter()
        .find(|i| i.suffix.as_deref() == Some("code"))
        .expect("symptom |code input");
    assert_eq!(
        symptom_code.list_open,
        Some(true),
        "symptom must be an OPEN value-set"
    );
    assert!(
        !symptom_code.list.is_empty(),
        "symptom |code carries the recommended local codes"
    );
    assert!(
        has_suffix(symptom, "other"),
        "an OPEN coded leaf exposes |other (master04 §Open Value-Sets)"
    );

    // Body exposure = CLOSED coded value-set (local code_list, no DV_TEXT
    // alternative) -> |code not list-open, no |other suffix (|other MUST be
    // rejected on a closed list — SF-FLAT-reject_other_closed_list).
    let exposure = by_id("vitals/body_temperature/any_event/body_exposure");
    assert_eq!(exposure.rm_type, "DV_CODED_TEXT");
    let exposure_code = exposure
        .inputs
        .iter()
        .find(|i| i.suffix.as_deref() == Some("code"))
        .expect("body_exposure |code input");
    assert_ne!(
        exposure_code.list_open,
        Some(true),
        "body_exposure must be a CLOSED value-set"
    );
    assert!(
        !exposure_code.list.is_empty(),
        "body_exposure |code carries the closed local list"
    );
    assert!(
        !has_suffix(exposure, "other"),
        "a CLOSED coded leaf must not expose |other"
    );
}
