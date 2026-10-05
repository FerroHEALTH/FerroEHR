// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Breadth gate over the vendored real-world archetype packs — both dialects.
//!
//! Two packs, two sources, because the two dialects are published in
//! different places (`.claude/rules/vendored-corpora.md`):
//!
//! - `corpus/archetypes/ckm/adl14/` — every
//!   archetype the public openEHR CKM publishes, as **ADL 1.4**
//!   (`scripts/vendor/ckm-archetypes.sh`). CKM publishes no ADL 2 export.
//! - `corpus/archetypes/adl2/ckm-2013-12-09/` —
//!   upstream's own CKM export carrying `*.adls` (**ADL 2**) beside `*.adl`
//!   (ADL 1.4) twins of the same archetypes, pinned by commit
//!   (`scripts/vendor/adl2-archetypes.sh`).
//!
//! The claim here is deliberately narrow and total: every file in both packs
//! PARSES in its own dialect. That is the breadth net the hand-written
//! `adl14-cadl`/`adl14-dadl` trees cannot provide — real clinical archetypes
//! carry constraint spellings, translation blocks and terminology sections no
//! authored fixture reproduces. Deeper claims (phase-1 validation, flattening)
//! stay with the rule-code-keyed `adl2-reference` corpus, whose file names
//! encode the expected outcome.
//!
//! NOTE: no openEHR spec governs 1.4 tolerance — our own design
//! ([`openehr_adl::adl14`]). Outcomes here are pinned by the packs themselves.
//!
//! Corpus discipline: 100% exercised, adjudicated refusals only. A file our
//! reader rejects is listed in the pack's `ADJUDICATED` table with the reason;
//! it stays vendored so the refusal is pinned, and a reader that starts
//! accepting it fails this gate.
#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test assertions and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use std::path::{Path, PathBuf};

use openehr_adl::artefact::ArchetypeRepository;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::error::SyntaxErrorCode;
use openehr_adl::parse::Dialect;
use openehr_adl::validate::bindings::NoTerminologyResolver;
use openehr_adl::validate::catalogue::{Severity, ValidationCode};
use openehr_adl::validate::rm::{ProductionRmModel, production_model_governs};
use openehr_adl::validate::{validate_source, validate_source_integrity};

/// Archetypes the conformant reader MUST refuse, with the syntax code the
/// refusal must carry and the adjudication behind it.
///
/// These are NEGATIVE tests, not a skip list: the gate asserts the refusal
/// happens AND carries the stated code. A file that starts parsing, or that
/// refuses with a different code, fails the gate — so a silently loosened
/// reader cannot hide here, and neither can a mis-coded error.
type Refusal = (&'static str, SyntaxErrorCode, &'static str);

/// One adjudicated defect family in the live CKM library.
///
/// A duplicate sibling container key in the terminology ODIN — incomplete
/// authoring, not a language feature. `LANG` ODIN `master05-content.adoc`
/// §VDOBU: "object identifier uniqueness: sibling objects occurring within a
/// container attribute must be uniquely identified with respect to each
/// other."
///
/// (The former second family — EMPTY inline dADL domain blocks, 9 files — was
/// re-adjudicated to ACCEPTANCE under #1465: the dADL chapter's own grammar
/// admits the empty block and §Empty Sections allows it anywhere, so it lowers
/// to the open constraint; see `adl14/domain.rs` `lower_adl14_domain`.)
const ADJUDICATED_CKM_ADL14: &[Refusal] = &[
    // ── duplicate sibling container key in terminology ODIN (VDOBU) ───────
    (
        "openEHR-DEMOGRAPHIC-ITEM_TREE.person_details.v0.adl",
        SyntaxErrorCode::Sdinv,
        "terminology ODIN repeats the sibling container key [\"at0310\"] (VDOBU)",
    ),
];

/// The upstream paired pack's adjudicated refusals — the same VDOBU defect
/// family, in three `.adl` files of the 2013 CKM export. The `.adls` half of
/// the directory has none, which is why the expected count is derived per file
/// set ([`expected_refusals`]) rather than from this table's length.
const ADJUDICATED_PAIRS: &[Refusal] = &[
    (
        "openEHR-EHR-CLUSTER.palpation-external_ear.v1.adl",
        SyntaxErrorCode::Sdinv,
        "terminology ODIN repeats the sibling container key [\"at0019\"] (VDOBU)",
    ),
    (
        "openEHR-EHR-CLUSTER.palpation-joint.v1.adl",
        SyntaxErrorCode::Sdinv,
        "terminology ODIN repeats the sibling container key [\"at0019\"] (VDOBU)",
    ),
    (
        "openEHR-EHR-OBSERVATION.lab_test-immunology-ANA.v1.adl",
        SyntaxErrorCode::Sdinv,
        "terminology ODIN repeats the sibling container key [\"at0.97\"] (VDOBU)",
    ),
];

fn artifacts_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/archetypes")
}

/// Every file with `ext` under `dir`, recursively, sorted.
fn files_with_extension(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = match std::fs::read_dir(&d) {
            Ok(entries) => entries,
            Err(e) => panic!("read pack dir {}: {e}", d.display()),
        };
        for path in entries.flatten().map(|e| e.path()) {
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == ext) {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("?")
        .to_owned()
}

/// The outcome of parsing a whole pack.
struct PackOutcome {
    /// Files that failed to parse with no adjudication covering them.
    failures: Vec<String>,
    /// Adjudicated refusals that now parse, or refuse with the wrong code —
    /// either way the adjudication no longer describes reality.
    broken_adjudications: Vec<String>,
    /// Files that parsed clean.
    clean: usize,
    /// Adjudicated refusals that refused with exactly the expected code.
    refused: usize,
}

/// Parse every file in `files` under `dialect`, asserting the positive claim
/// (parses clean) for ordinary files and the NEGATIVE claim (refuses, with the
/// stated code) for adjudicated ones.
fn parse_all(files: &[PathBuf], dialect: Dialect, adjudicated: &[Refusal]) -> PackOutcome {
    let mut out = PackOutcome {
        failures: Vec::new(),
        broken_adjudications: Vec::new(),
        clean: 0,
        refused: 0,
    };
    for path in files {
        let name = file_name(path);
        let expected = adjudicated
            .iter()
            .find(|(file, _, _)| *file == name)
            .map(|(_, code, reason)| (*code, *reason));
        let src = std::fs::read_to_string(path).expect("read archetype source");
        match (parse_artefact(&src, dialect), expected) {
            // ordinary file, parses — the positive claim
            (Ok(_), None) => out.clean += 1,
            // adjudicated refusal that now parses — the adjudication is stale
            (Ok(_), Some((code, reason))) => out.broken_adjudications.push(format!(
                "{name}: expected refusal {code} ({reason}) but the file now PARSES — \
                 remove the adjudication if that is a genuine fix, or investigate a \
                 loosened reader"
            )),
            // ordinary file that fails — a real, unadjudicated defect
            (Err(errors), None) => out.failures.push(format!("{name}: {errors:?}")),
            // adjudicated refusal — the refusal must carry the stated code
            (Err(errors), Some((code, reason))) => {
                if errors.iter().any(|e| e.code == code) {
                    out.refused += 1;
                } else {
                    out.broken_adjudications.push(format!(
                        "{name}: expected refusal {code} ({reason}) but got {:?}",
                        errors.iter().map(|e| e.code).collect::<Vec<_>>()
                    ));
                }
            }
        }
    }
    out
}

/// How many adjudicated refusals actually belong to this file set (the pairs
/// table is shared by the `.adls` and `.adl` halves of one directory).
fn expected_refusals(files: &[PathBuf], adjudicated: &[Refusal]) -> usize {
    adjudicated
        .iter()
        .filter(|(name, _, _)| files.iter().any(|p| file_name(p) == *name))
        .count()
}

fn assert_pack(label: &str, out: &PackOutcome, total: usize, expected_refusals: usize) {
    assert!(
        out.broken_adjudications.is_empty(),
        "{label}: adjudications that no longer describe reality:\n{}",
        out.broken_adjudications.join("\n")
    );
    assert!(
        out.failures.is_empty(),
        "{label}: {} of {total} files failed to parse and are not adjudicated:\n{}",
        out.failures.len(),
        out.failures.join("\n")
    );
    // Every adjudicated refusal was reached and refused with its stated code —
    // the negative half of the claim, asserted rather than assumed.
    assert_eq!(
        out.refused, expected_refusals,
        "{label}: {} of {expected_refusals} adjudicated refusals were exercised",
        out.refused
    );
    assert_eq!(
        out.clean + out.refused,
        total,
        "{label}: accounting mismatch over the pack ({} clean + {} refused != {total})",
        out.clean,
        out.refused
    );
}

/// Every archetype of the full CKM library parses in the ADL 1.4 dialect.
#[test]
fn ckm_adl14_pack_parses() {
    let dir = artifacts_root().join("ckm/adl14");
    let files = files_with_extension(&dir, "adl");
    assert!(
        files.len() >= 900,
        "the CKM archetype pack is missing: found {} files in {} — re-run \
         scripts/vendor/ckm-archetypes.sh",
        files.len(),
        dir.display()
    );
    let out = parse_all(&files, Dialect::Adl14, ADJUDICATED_CKM_ADL14);
    assert_pack(
        "CKM ADL 1.4 pack",
        &out,
        files.len(),
        expected_refusals(&files, ADJUDICATED_CKM_ADL14),
    );
}

/// Every ADL 2 archetype of the upstream paired pack parses in the ADL 2
/// dialect.
#[test]
fn upstream_adl2_pack_parses() {
    let dir = artifacts_root().join("adl2/ckm-2013-12-09");
    let files = files_with_extension(&dir, "adls");
    assert!(
        files.len() >= 300,
        "the upstream ADL 2 pack is missing: found {} files in {} — re-run \
         scripts/vendor/adl2-archetypes.sh",
        files.len(),
        dir.display()
    );
    let out = parse_all(&files, Dialect::Adl2, ADJUDICATED_PAIRS);
    assert_pack(
        "upstream ADL 2 pack",
        &out,
        files.len(),
        expected_refusals(&files, ADJUDICATED_PAIRS),
    );
}

/// The ADL 1.4 twins shipped beside those ADL 2 files parse in the 1.4
/// dialect — the same archetype, both dialects, both readable.
#[test]
fn upstream_adl14_twins_parse() {
    let dir = artifacts_root().join("adl2/ckm-2013-12-09");
    let files = files_with_extension(&dir, "adl");
    assert!(
        files.len() >= 300,
        "the upstream ADL 1.4 twins are missing: found {} files in {}",
        files.len(),
        dir.display()
    );
    let out = parse_all(&files, Dialect::Adl14, ADJUDICATED_PAIRS);
    assert_pack(
        "upstream ADL 1.4 twins",
        &out,
        files.len(),
        expected_refusals(&files, ADJUDICATED_PAIRS),
    );
}

/// The RM resource-meta rows over the whole CKM 1.4 pack.
///
/// Enforcement of the RM common ch.8 invariants on 1.4 sources
/// (`validate::resource_meta`) was audited clean against the vendored
/// real-world library first, so it newly rejects no previously-accepted
/// archetype — this sweep IS that record. Only the resource-meta codes are
/// asserted: other validity findings on real-world content are the
/// rule-code-keyed corpus' territory.
#[test]
fn ckm_adl14_pack_is_resource_meta_clean() {
    let resource_codes = [
        "AUTHORED_RESOURCE.Original_language_valid",
        "AUTHORED_RESOURCE.Translations_valid",
        "AUTHORED_RESOURCE.Description_valid",
        "TRANSLATION_DETAILS.Language_valid",
        "RESOURCE_DESCRIPTION.Original_author_valid",
        "RESOURCE_DESCRIPTION.Lifecycle_state_valid",
        "RESOURCE_DESCRIPTION.Details_valid",
        "RESOURCE_DESCRIPTION_ITEM.Language_valid",
        "RESOURCE_DESCRIPTION_ITEM.Purpose_valid",
        "RESOURCE_DESCRIPTION_ITEM.Use_valid",
        "RESOURCE_DESCRIPTION_ITEM.misuse_valid",
    ];
    let dir = artifacts_root().join("ckm/adl14");
    let files = files_with_extension(&dir, "adl");
    assert!(files.len() >= 900);
    let mut offenders = Vec::new();
    for path in &files {
        let src = std::fs::read_to_string(path).expect("archetype is readable");
        let Ok(issues) = openehr_adl::validate::validate_adl14_source(&src, &ProductionRmModel)
        else {
            continue; // adjudicated parse refusals are ckm_adl14_pack_parses' claim
        };
        for issue in issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .filter(|i| resource_codes.contains(&i.code.mnemonic()))
        {
            offenders.push(format!(
                "{}: {} — {}",
                file_name(path),
                issue.code.mnemonic(),
                issue.message
            ));
        }
    }
    assert!(
        offenders.is_empty(),
        "CKM archetypes refused by the resource-meta pass (adjudicate, never silence):\n{}",
        offenders.join("\n")
    );
}

/// A vendored ADL 2 archetype the AOM2 validator MUST refuse, with the
/// validity code the refusal must carry and the adjudication behind it.
///
/// The validation-level twin of [`Refusal`]: these files parse, and the CDR's
/// own upload path (`POST /definition/template/adl2`) refuses them with `422`
/// on exactly these codes, so the sandbox seed's pinned accepted/refused split
/// has this table as its authority.
type ValidityRefusal = (&'static str, ValidationCode, &'static str);

/// The adjudicated validity refusals over the 2013 CKM ADL 2 export.
const ADJUDICATED_ADL2_VALIDITY: &[ValidityRefusal] = &[
    (
        "openEHR-DEMOGRAPHIC-PARTY_IDENTITY.person_name-individual_provider.v1.0.0.adls",
        ValidationCode::Vacdf,
        "constraint codes used in the definition are undefined in the terminology (master03 VACDF)",
    ),
    (
        "openEHR-DEMOGRAPHIC-PERSON.person-patient.v1.0.0.adls",
        ValidationCode::Vacdf,
        "constraint codes used in the definition are undefined in the terminology (master03 VACDF)",
    ),
    (
        "openEHR-EHR-CLUSTER.ambient_oxygen.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY (undefined in RM 1.2.0) and the RM function(s) `is_integral`, which VCARM admits but the vendored BMM does not declare (#3061)",
    ),
    (
        "openEHR-EHR-CLUSTER.anatomical_location-precise.v1.0.0.adls",
        ValidationCode::Vasid,
        "its parent openEHR-EHR-CLUSTER.anatomical_location.v1.0.0 is refused above, so no flat parent exists to validate against (master03 VASID)",
    ),
    (
        "openEHR-EHR-CLUSTER.anatomical_location.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-CLUSTER.auscultation-chest.v1.0.0.adls",
        ValidationCode::Vpov,
        "a redefined value-set member is not in the parent value set (master03 VPOV)",
    ),
    (
        "openEHR-EHR-CLUSTER.dimensions.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-CLUSTER.environmental_conditions.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-CLUSTER.exam-abdomen.v1.0.0.adls",
        ValidationCode::Vdssm,
        "a specialised slot restates the parent slot constraints instead of narrowing them (master03 VDSSM)",
    ),
    (
        "openEHR-EHR-CLUSTER.exam-uterine_cervix.v1.0.0.adls",
        ValidationCode::Valc,
        "declares language(s) es-cl absent from the flat parent (master03 VALC)",
    ),
    (
        "openEHR-EHR-CLUSTER.exam_pupils.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-CLUSTER.fluid.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-CLUSTER.health_event-poisoning.v1.0.0.adls",
        ValidationCode::Valc,
        "declares language(s) ar-sy, es-ar absent from the flat parent (master03 VALC)",
    ),
    (
        "openEHR-EHR-CLUSTER.inspection-skin-wound.v1.0.0.adls",
        ValidationCode::Vcosu,
        "an object node id recurs in the flat form (master04.5 VCOSU)",
    ),
    (
        "openEHR-EHR-CLUSTER.level_of_exertion.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-CLUSTER.lymph_node_metastases.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-CLUSTER.macroscopy_colorectal_carcinoma.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-CLUSTER.medication_amount.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-CLUSTER.menstrual_cycle.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-CLUSTER.microscopy_colorectal_carcinoma.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-CLUSTER.microscopy_melanoma.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY (undefined in RM 1.2.0) and the RM function(s) `is_integral`, which VCARM admits but the vendored BMM does not declare (#3061)",
    ),
    (
        "openEHR-EHR-CLUSTER.microscopy_prostate_carcinoma.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains the RM function(s) `is_integral`; VCARM admits computed attributes, but the vendored BMM declares no functions (#3061)",
    ),
    (
        "openEHR-EHR-CLUSTER.move-joint.v1.0.0.adls",
        ValidationCode::Vasid,
        "its parent openEHR-EHR-CLUSTER.move.v1.0.0 is refused above, so no flat parent exists to validate against (master03 VASID)",
    ),
    (
        "openEHR-EHR-CLUSTER.move-spine.v1.0.0.adls",
        ValidationCode::Vasid,
        "its parent openEHR-EHR-CLUSTER.move.v1.0.0 is refused above, so no flat parent exists to validate against (master03 VASID)",
    ),
    (
        "openEHR-EHR-CLUSTER.move.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-CLUSTER.physical_properties.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-CLUSTER.refraction_details.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-CLUSTER.specimen.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-CLUSTER.specimen_preparation.v1.0.0.adls",
        ValidationCode::Vcaca,
        "cardinality {0..1} narrows below the RM cardinality {1..*} of CLUSTER.items (master04.5 VCACA)",
    ),
    (
        "openEHR-EHR-CLUSTER.symptom-pain.v1.0.0.adls",
        ValidationCode::Valc,
        "declares language(s) es absent from the flat parent (master03 VALC)",
    ),
    (
        "openEHR-EHR-CLUSTER.timing.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-CLUSTER.tumour_resection_margins.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-CLUSTER.waveform.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-EVALUATION.goal.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-EVALUATION.pregnancy.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-EVALUATION.problem_diagnosis.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-INSTRUCTION.request-procedure.v1.0.0.adls",
        ValidationCode::Vdifp,
        "a differential path names an attribute the flat parent does not have (master08 VDIFP)",
    ),
    (
        "openEHR-EHR-INSTRUCTION.transfusion.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-ITEM_TREE.gas_administration.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-ITEM_TREE.intravenous_fluids.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-OBSERVATION.apgar.v1.0.0.adls",
        ValidationCode::Vttbk,
        "term binding keys name paths that do not exist in the archetype (master03 VTTBK)",
    ),
    (
        "openEHR-EHR-OBSERVATION.blood_pressure.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-OBSERVATION.bodily_output-defaecation.v1.0.0.adls",
        ValidationCode::Vdssid,
        "a specialised slot is renumbered instead of keeping the parent slot id (master03 VDSSID)",
    ),
    (
        "openEHR-EHR-OBSERVATION.bodily_output-urination.v1.0.0.adls",
        ValidationCode::Vdssid,
        "a specialised slot is renumbered instead of keeping the parent slot id (master03 VDSSID)",
    ),
    (
        "openEHR-EHR-OBSERVATION.body_mass_index.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-OBSERVATION.body_surface_area.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-OBSERVATION.body_temperature.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-OBSERVATION.body_weight-adjusted.v1.0.0.adls",
        ValidationCode::Vasid,
        "its parent openEHR-EHR-OBSERVATION.body_weight.v1.0.0 is refused above, so no flat parent exists to validate against (master03 VASID)",
    ),
    (
        "openEHR-EHR-OBSERVATION.body_weight-birth.v1.0.0.adls",
        ValidationCode::Vasid,
        "its parent openEHR-EHR-OBSERVATION.body_weight.v1.0.0 is refused above, so no flat parent exists to validate against (master03 VASID)",
    ),
    (
        "openEHR-EHR-OBSERVATION.body_weight.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-OBSERVATION.chest_expansion.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-OBSERVATION.demo.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY (undefined in RM 1.2.0) and the RM function(s) `is_integral`, `offset`, which VCARM admits but the vendored BMM does not declare (#3061)",
    ),
    (
        "openEHR-EHR-OBSERVATION.distraction_hearing_test.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-OBSERVATION.ecg.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-OBSERVATION.electroacoustic_hearing_test.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-OBSERVATION.faeces.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-OBSERVATION.fetal_heart-monitoring.v1.0.0.adls",
        ValidationCode::Vasid,
        "its parent openEHR-EHR-OBSERVATION.fetal_heart.v1.0.0 is refused above, so no flat parent exists to validate against (master03 VASID)",
    ),
    (
        "openEHR-EHR-OBSERVATION.fetal_heart.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-OBSERVATION.hearing_screening.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-OBSERVATION.height.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-OBSERVATION.indirect_oximetry.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY (undefined in RM 1.2.0) and the RM function(s) `is_integral`, which VCARM admits but the vendored BMM does not declare (#3061)",
    ),
    (
        "openEHR-EHR-OBSERVATION.infant_feeding.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-OBSERVATION.intraocular_pressure.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-OBSERVATION.intravascular_pressure-cvp.v1.0.0.adls",
        ValidationCode::Vasid,
        "its parent openEHR-EHR-OBSERVATION.intravascular_pressure.v1.0.0 is refused above, so no flat parent exists to validate against (master03 VASID)",
    ),
    (
        "openEHR-EHR-OBSERVATION.intravascular_pressure-jvp.v1.0.0.adls",
        ValidationCode::Vasid,
        "its parent openEHR-EHR-OBSERVATION.intravascular_pressure.v1.0.0 is refused above, so no flat parent exists to validate against (master03 VASID)",
    ),
    (
        "openEHR-EHR-OBSERVATION.intravascular_pressure.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-OBSERVATION.lab_test-blood_gases.v1.0.0.adls",
        ValidationCode::Valc,
        "declares language(s) es-ar absent from the flat parent (master03 VALC)",
    ),
    (
        "openEHR-EHR-OBSERVATION.lab_test-full_blood_count.v1.0.0.adls",
        ValidationCode::Vsonin,
        "new object node ids are not valid new ids at this specialisation level (master03 VSONIN)",
    ),
    (
        "openEHR-EHR-OBSERVATION.mantoux.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-OBSERVATION.menstruation.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-OBSERVATION.msfc_score.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-OBSERVATION.oral_fluid_intake.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-OBSERVATION.pathology_test-blood_glucose.v1.0.0.adls",
        ValidationCode::Vdifp,
        "a differential path names an attribute the flat parent does not have (master08 VDIFP)",
    ),
    (
        "openEHR-EHR-OBSERVATION.pathology_test-lipids.v1.0.0.adls",
        ValidationCode::Vdifp,
        "a differential path names an attribute the flat parent does not have (master08 VDIFP)",
    ),
    (
        "openEHR-EHR-OBSERVATION.pulmonary_function.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-OBSERVATION.pulse.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-OBSERVATION.respiration.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-OBSERVATION.substance_use-alcohol.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-OBSERVATION.substance_use-caffeine.v1.0.0.adls",
        ValidationCode::Vcosu,
        "an object node id recurs in the flat form (master04.5 VCOSU)",
    ),
    (
        "openEHR-EHR-OBSERVATION.temperature.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-OBSERVATION.tympanogram_226hz.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-OBSERVATION.tympanogram_hf.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY (undefined in RM 1.2.0) and the RM function(s) `is_integral`, which VCARM admits but the vendored BMM does not declare (#3061)",
    ),
    (
        "openEHR-EHR-OBSERVATION.urine_output.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-OBSERVATION.uterine_contractions.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-OBSERVATION.visual_acuity.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY (undefined in RM 1.2.0) and the RM function(s) `is_integral`, which VCARM admits but the vendored BMM does not declare (#3061)",
    ),
    (
        "openEHR-EHR-OBSERVATION.visual_field_measurement.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY (undefined in RM 1.2.0) and the RM function(s) `is_integral`, which VCARM admits but the vendored BMM does not declare (#3061)",
    ),
    (
        "openEHR-EHR-OBSERVATION.waist_hip.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-OBSERVATION.warble_tones_hearing_test.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
    (
        "openEHR-EHR-OBSERVATION.word_list_hearing_test.v1.0.0.adls",
        ValidationCode::Vcarm,
        "constrains `property` on DV_QUANTITY, which RM 1.2.0 does not define (the 2013 converter carried ADL 1.4 C_DV_QUANTITY.property into an attribute position)",
    ),
];

/// The outcome of validating a whole pack the way the CDR validates an upload.
struct ValidityOutcome {
    /// Files refused with no adjudication covering them (`name: CODE message`).
    failures: Vec<String>,
    /// Adjudications that no longer describe reality.
    broken_adjudications: Vec<String>,
    /// Files validating with no error-severity issue.
    clean: usize,
    /// Adjudicated refusals that carried exactly the expected code.
    refused: usize,
}

/// The specialisation depth an ADL 2 id states in its concept segment
/// (`CLUSTER.exam-abdomen` specialises `CLUSTER.exam`: one `-`, depth 1).
fn id_depth(name: &str) -> usize {
    name.split('.')
        .nth(1)
        .map_or(0, |concept| concept.matches('-').count())
}

/// The pack in upload order: parents before children (depth, then name) — the
/// order the sandbox seed uses, so a child always meets its stored parent.
fn parents_first(files: &[PathBuf]) -> Vec<PathBuf> {
    let mut ordered: Vec<PathBuf> = files.to_vec();
    ordered.sort_by_cached_key(|p| {
        let name = file_name(p);
        (id_depth(&name), name)
    });
    ordered
}

/// Validate every parseable `.adls` of the pack exactly as the CDR does on
/// upload, in upload order: the full AOM2 catalogue against the production RM
/// for an openEHR RM archetype, the integrity pass alone otherwise, each over a
/// repository holding only the archetypes ACCEPTED so far — a refused parent is
/// never a flat parent, and a child whose parent is absent is VASID.
fn validate_all(files: &[PathBuf], adjudicated: &[ValidityRefusal]) -> ValidityOutcome {
    let mut repo = ArchetypeRepository::new();
    let mut out = ValidityOutcome {
        failures: Vec::new(),
        broken_adjudications: Vec::new(),
        clean: 0,
        refused: 0,
    };
    for path in parents_first(files) {
        let name = file_name(&path);
        let src = std::fs::read_to_string(&path).expect("read archetype source");
        let Ok(archetype) = parse_artefact(&src, Dialect::Adl2) else {
            // Parse refusals are the parse-level table's business.
            continue;
        };
        let issues = if production_model_governs(&archetype) {
            validate_source(
                &src,
                Some(&repo),
                &ProductionRmModel,
                &NoTerminologyResolver,
            )
        } else {
            validate_source_integrity(&src, Dialect::Adl2, Some(&repo))
        }
        .expect("a parsed source validates without syntax errors");
        let errors: Vec<_> = issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .collect();
        let expected = adjudicated
            .iter()
            .find(|(file, _, _)| *file == name)
            .map(|(_, code, reason)| (*code, *reason));
        match (errors.is_empty(), expected) {
            (true, None) => {
                out.clean += 1;
                repo.insert(archetype);
            }
            (true, Some((code, reason))) => out.broken_adjudications.push(format!(
                "{name}: expected refusal {code} ({reason}) but the file now VALIDATES — \
                 remove the adjudication if that is a genuine fix, or investigate a \
                 loosened validator"
            )),
            (false, None) => out.failures.push(format!(
                "{name}: {}",
                errors
                    .iter()
                    .map(|i| {
                        let at = i
                            .path
                            .as_ref()
                            .map_or(String::new(), |p| format!(" (at {p})"));
                        format!("{} {}{at}", i.code, i.message)
                    })
                    .collect::<Vec<_>>()
                    .join(" | ")
            )),
            (false, Some((code, reason))) => {
                if errors.iter().any(|i| i.code == code) {
                    out.refused += 1;
                } else {
                    out.broken_adjudications.push(format!(
                        "{name}: expected refusal {code} ({reason}) but got {:?}",
                        errors.iter().map(|i| i.code).collect::<Vec<_>>()
                    ));
                }
            }
        }
    }
    out
}

/// Every parseable archetype of the 2013 CKM ADL 2 export validates as the CDR
/// validates an upload, or is refused on an adjudicated validity code.
#[test]
fn upstream_adl2_pack_validates_or_is_adjudicated() {
    let dir = artifacts_root().join("adl2/ckm-2013-12-09");
    let files = files_with_extension(&dir, "adls");
    assert!(!files.is_empty(), "no .adls files under {}", dir.display());
    let out = validate_all(&files, ADJUDICATED_ADL2_VALIDITY);
    assert!(
        out.broken_adjudications.is_empty(),
        "adl2 validity: adjudications that no longer describe reality:\n{}",
        out.broken_adjudications.join("\n")
    );
    assert!(
        out.failures.is_empty(),
        "adl2 validity: {} files are refused and not adjudicated:\n{}",
        out.failures.len(),
        out.failures.join("\n")
    );
    assert_eq!(
        out.refused,
        ADJUDICATED_ADL2_VALIDITY.len(),
        "adl2 validity: {} of {} adjudicated refusals were exercised",
        out.refused,
        ADJUDICATED_ADL2_VALIDITY.len()
    );
    assert_eq!(
        out.clean + out.refused,
        files.len() - expected_refusals(&files, ADJUDICATED_PAIRS),
        "adl2 validity: accounting mismatch over the parseable pack"
    );
}
