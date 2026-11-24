// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The commit validation passes report every finding, in the order they emit
//! it, as the committed record holds it: the template examples and the corpus
//! COMPOSITIONs with their defective variants (#6453). A finding's text is
//! the refusal a client reads, so its order is part of the record.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use openehr_its::opt14;
use openehr_rm::prelude::Composition;
use openehr_sdt::flat::example::{DetailLevel, example_composition};
use openehr_sdt::flat::validation::{
    collect_constraint_binding_checks_typed, validate_archetype_conformance,
    validate_archetype_conformance_incomplete, validate_archetype_conformance_typed,
};
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use openehr_sdt::flat::webtemplate::model::WebTemplate;
use openehr_sdt::rm_instance::{ValidationMessage, validate_rm_and_terminology_typed};

use super::{typed_conformance_parity, typed_validation_parity};

/// Each source file's findings in emitted order, folded into a count and an
/// FNV-1a digest, so the record stays small and a change names its file.
#[derive(Default)]
struct Record {
    files: BTreeMap<String, (usize, u64)>,
    documents: usize,
    findings: usize,
}

/// The FNV-1a 64-bit offset basis.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// Folds `text` into the FNV-1a 64-bit digest `hash`.
fn fnv(hash: u64, text: &str) -> u64 {
    text.bytes().fold(hash, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

impl Record {
    fn fold(&mut self, file: &str, line: &str) {
        let entry = self.files.entry(file.to_owned()).or_insert((0, FNV_OFFSET));
        entry.0 += 1;
        entry.1 = fnv(fnv(entry.1, line), "\n");
    }

    fn report(&mut self, file: &str, label: &str, pass: &str, report: &[ValidationMessage]) {
        self.documents += 1;
        self.findings += report.len();
        for m in report {
            let line = format!("{label} [{pass}] {:?} {} | {}", m.kind, m.path, m.message);
            self.fold(file, &line);
        }
    }

    fn conformance(&mut self, file: &str, label: &str, typed: &Composition, wt: &WebTemplate) {
        let canonical = openehr_its::json::to_canonical_value(typed);
        let typed_report = validate_archetype_conformance_typed(typed, wt);
        self.report(file, label, "conformance", &typed_report);
        let json_report = validate_archetype_conformance(&canonical, wt);
        self.report(file, label, "conformance json", &json_report);
        let incomplete = validate_archetype_conformance_incomplete(&canonical, wt);
        self.report(file, label, "conformance incomplete", &incomplete);
        for check in collect_constraint_binding_checks_typed(typed, wt) {
            let line = format!(
                "{label} [binding] {} {} {} {}|{}",
                check.path,
                check.ac_code,
                check.query_uri,
                check.instance_terminology,
                check.instance_code
            );
            self.fold(file, &line);
        }
    }

    fn text(&self) -> String {
        let mut out = String::new();
        for (file, (count, hash)) in &self.files {
            let _ = writeln!(out, "{file}: {count} {hash:016x}");
        }
        out
    }
}

#[test]
fn the_validation_passes_report_what_the_record_holds() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let relative = |p: &str| {
        p.strip_prefix(root.to_string_lossy().as_ref())
            .unwrap_or(p)
            .trim_start_matches('/')
            .to_owned()
    };
    let mut record = Record::default();
    let mut templates: BTreeMap<String, WebTemplate> = BTreeMap::new();
    for path in typed_conformance_parity::opts() {
        let Ok(xml) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(opt) = opt14::from_xml(&xml) else {
            continue;
        };
        let Ok(wt) = build_web_template(&opt) else {
            continue;
        };
        let name = relative(&path.display().to_string());
        for level in [DetailLevel::Required, DetailLevel::Complete] {
            let Ok(example) = example_composition(&wt, level) else {
                continue;
            };
            for (label, variant) in typed_conformance_parity::variants(&example) {
                let Ok(typed) = openehr_its::json::from_canonical_value::<Composition>(&variant)
                else {
                    continue;
                };
                record.conformance(&name, &format!("{level:?} ({label})"), &typed, &wt);
            }
        }
        templates.entry(wt.template_id.clone()).or_insert(wt);
    }
    for (path, doc) in typed_validation_parity::compositions() {
        let name = relative(&path);
        for (label, variant) in typed_validation_parity::variants(&doc) {
            let Ok(typed) = openehr_its::json::from_canonical_value::<Composition>(&variant) else {
                continue;
            };
            let rm = validate_rm_and_terminology_typed(&typed);
            record.report(&name, label, "rm", &rm);
            let template = typed
                .archetype_details
                .as_ref()
                .and_then(|d| d.template_id.as_ref())
                .and_then(|id| templates.get(&id.value));
            if let Some(wt) = template {
                record.conformance(&name, label, &typed, wt);
            }
        }
    }
    assert!(
        record.documents > 2000,
        "read only {} documents",
        record.documents
    );
    assert!(record.findings > 1000, "only {} findings", record.findings);
    insta::assert_snapshot!(record.text());
}
