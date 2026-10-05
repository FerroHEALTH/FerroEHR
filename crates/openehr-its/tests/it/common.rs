// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Shared corpus plumbing for the canonical-JSON gates (`fidelity.rs`,
//! `canonical_contract.rs`): the corpus walker and the single documented
//! exclusion list, so the gates can never drift apart on what counts as a
//! canonical RM root.

use std::fs;
use std::path::{Path, PathBuf};

/// Every `.json` under `tests/vendor/` **and** `tests/fixtures/twins/`, sorted
/// for determinism.
///
/// The twins directory carries the repo-authored VALID half of each adjudicated
/// defective vendored fixture (`fixture_twins.rs`). It joins the corpus walk on
/// purpose: excluding a defective vendored document without admitting its
/// corrected twin would silently narrow what these gates cover, and the twins
/// rule exists precisely so a spec-correct refusal costs no coverage.
pub(crate) fn corpus_files() -> Vec<PathBuf> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    let mut stack = vec![
        manifest.join("tests/vendor"),
        manifest.join("tests/fixtures/twins"),
    ];
    while let Some(dir) = stack.pop() {
        if let Ok(rd) = fs::read_dir(&dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "json") {
                    out.push(p);
                }
            }
        }
    }
    out.sort();
    out
}

/// The corpus-relative key of `path`: the path with its corpus root
/// (`tests/vendor/` or `tests/fixtures/`) stripped and separators normalized.
///
/// One derivation for every gate, so an absolute, machine-dependent path can
/// never reach a snapshot manifest or an exclusion lookup.
pub(crate) fn corpus_rel(path: &Path) -> String {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let rel = [
        manifest.join("tests/vendor"),
        manifest.join("tests/fixtures"),
    ]
    .iter()
    .find_map(|root| path.strip_prefix(root).ok())
    .unwrap_or(path);
    rel.display().to_string().replace('\\', "/")
}

/// The file a gate should actually read for the vendored fixture at `path`:
/// its repo-authored VALID TWIN when one exists, else `path` itself.
///
/// A gate that walks a vendored directory DIRECTLY (rather than through
/// [`corpus_files`] + [`excluded`]) still has to honour the twins adjudication,
/// or it re-reads a document the corpus gates already refused. Routing through
/// here keeps one substitution rule for every gate.
pub(crate) fn twinned(path: &Path) -> PathBuf {
    let Some(stem) = path.file_stem().and_then(std::ffi::OsStr::to_str) else {
        return path.to_path_buf();
    };
    let twin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/twins")
        .join(format!("{stem}.valid.json"));
    if twin.is_file() {
        twin
    } else {
        path.to_path_buf()
    }
}

/// The RM class a corpus document is declared under when its canonical JSON
/// omits the top-level `_type`.
///
/// ITS-JSON makes `_type` omittable exactly when the DECLARED slot is concrete
/// (`docs/specs/openehr/ITS-JSON/` — the canonical form self-tags only where the
/// static type is ambiguous), so a `_type`-less root is legal but carries its
/// class in the CONTEXT, not the bytes. The vendored corpus states that context
/// in its layout (`openehr_sdk/<kind>/canonical_json/…`), and a repo-authored
/// twin (`twins/<stem>.valid.json`) inherits it from the vendored document it
/// corrects. Both are DERIVED here — never a hand-maintained per-file table,
/// which is the drift this registry exists to end.
pub(crate) fn declared_root_type(path: &Path) -> Option<&'static str> {
    let rel = corpus_rel(path);
    if let Some(rest) = rel.strip_prefix("openehr_sdk/") {
        let kind = rest.split('/').next()?;
        return match kind {
            "composition" => Some("COMPOSITION"),
            "folder" => Some("FOLDER"),
            "ehr" => Some("EHR_STATUS"),
            "contribution" => Some("CONTRIBUTION"),
            "item_structure" => Some("ITEM_TREE"),
            _ => None,
        };
    }
    // A twin inherits the declared slot of the vendored document it corrects.
    let stem = rel.strip_prefix("twins/")?.strip_suffix(".valid.json")?;
    corpus_files()
        .into_iter()
        .find(|p| {
            corpus_rel(p).starts_with("openehr_sdk/")
                && p.file_stem().and_then(std::ffi::OsStr::to_str) == Some(stem)
        })
        .and_then(|p| declared_root_type(&p))
}

/// Corpus files that are **not** canonical RM 1.2 objects, with the reason each
/// is out of scope for the canonical gates. They are excluded, not silently
/// skipped — the gates still fail if a file *not* listed here fails to read.
/// (Same discipline as the AQL example-corpus exclusions.) Keyed by the
/// trailing path.
///
/// None of these reflect a defect in the generated types: they are either a
/// different serialization (raw-DB / ITS-REST request), deliberately invalid,
/// malformed, or RM-1.1-era data that omits fields RM 1.2 made mandatory.
pub(crate) fn excluded(name: &str) -> Option<&'static str> {
    // Normalize path separators for matching on any platform, and drop the
    // corpus-root prefix so keys are relative to the openEHR_SDK tree.
    let n = name.replace('\\', "/");
    let n = n.strip_prefix("openehr_sdk/").unwrap_or(&n);
    let reason = |r| Some(r);
    match n {
        // Path-keyed `content` map (ethercis raw-DB / flat form), not canonical.
        "composition/canonical_json/rawdb_composition.json"
        | "composition/canonical_json/composition_with_dvinterval_composite.json" => {
            reason("not canonical JSON: `content` is a path-keyed map (raw-DB/flat form)")
        }
        // The rest of the raw-DB family: EHRbase's decomposed row-per-locatable
        // DB shape (path keys `/name`, `/events`, `/$CLASS$`; one file is a bare
        // JSON array of rows). No RM class is expressible in it.
        "composition/canonical_json/rawdb_composition_history.json"
        | "composition/canonical_json/rawdb_composition_observation_event.json"
        | "composition/canonical_json/rawdb_composition_observation_event_item.json"
        | "composition/canonical_json/rawdb_returning_array.json" => {
            reason("not canonical JSON: EHRbase decomposed row-per-locatable DB shape")
        }
        // Legacy Jackson polymorphism: the discriminator is `@class`, not the
        // ITS-JSON `_type` key (`docs/specs/openehr/ITS-JSON/` — the canonical
        // form self-tags with `_type`).
        "composition/canonical_json/full_composition.json" => {
            reason("legacy Jackson `@class` discriminator, not the ITS-JSON `_type` form")
        }
        // ITS-REST CONTRIBUTION *request* DTOs: a `{versions, audit}` envelope
        // with no `_type`, where RM `CONTRIBUTION` is a `{uid, versions, audit}`
        // object whose `versions` is `Set<OBJECT_REF>`
        // (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.contribution.adoc`
        // §Attributes). They belong to the REST DTO layer, not the RM gate.
        "contribution/canonical_json/latest-contribution-one_entry-composition.json"
        | "contribution/canonical_json/latest-contribution-one_entry-composition-deletion.json"
        | "contribution/canonical_json/latest-contribution-one_entry-composition-modification.json"
        | "contribution/canonical_json/status.contribution.modification.json" => {
            reason("ITS-REST contribution request envelope (`{versions, audit}`, no `_type`)")
        }
        // Deliberately invalid fixture (`EVENT_CONTEXT_WRONG`) — a negative test.
        "composition/canonical_json/invalid.json" => {
            reason("deliberately invalid fixture (a wrong `_type`), a negative test")
        }
        // ITS-REST contribution *request* bodies: `versions` holds full
        // `ORIGINAL_VERSION`s, whereas RM `CONTRIBUTION.versions` is `Set<OBJECT_REF>`.
        // These belong to the REST DTO layer, not the RM canonical gate.
        "contribution/canonical_json/contribution-one_entry-composition.json"
        | "contribution/canonical_json/contribution-two_entries-composition.json" => reason(
            "ITS-REST contribution request shape (versions = ORIGINAL_VERSION, not OBJECT_REF)",
        ),
        // DEFECTIVE upstream fixture, adjudicated: two ENTRY nodes hoist
        // `feeder_system_audit` onto the ENTRY itself, while three sibling
        // nodes in the SAME document nest it correctly. That member belongs to
        // FEEDER_AUDIT (`…rm.common.feeder_audit.adoc` §Attributes), reachable
        // from a LOCATABLE only through `LOCATABLE.feeder_audit`. Kept vendored
        // verbatim as the INVALID twin; the corrected VALID twin and the
        // asserted refusal live in `fixture_twins.rs`.
        "composition/canonical_json/all_types_systematic_tests_feeder_audit.json" => reason(
            "defective fixture: two ENTRY nodes hoist FEEDER_AUDIT.feeder_system_audit onto \
             the ENTRY (valid twin in tests/fixtures/twins/, refusal in fixture_twins.rs)",
        ),
        // DEFECTIVE upstream fixture class, adjudicated (14 documents): a
        // PLACEHOLDER `OBJECT_VERSION_ID`
        // (`__THIS_SHOULD_BE_MODIFIED_BY_THE_TEST_::ehrbase.org::1`) whose
        // `object_id` matches none of the three `uid` productions of BASE
        // `base_types/master05-identification_package.adoc` §Syntaxes. Each is
        // kept vendored verbatim as the INVALID twin, with its corrected VALID
        // twin in `tests/fixtures/twins/`; asserted refusals in
        // `fixture_twins.rs`.
        "composition/canonical_json/alternative_types.json"
        | "composition/canonical_json/duration_tests.json"
        | "composition/canonical_json/laboratory_report.json"
        | "composition/canonical_json/laboratory_report_no_content.json"
        | "composition/canonical_json/minimal_admin.json"
        | "composition/canonical_json/minimal_evaluation_item_tree_name.json"
        | "composition/canonical_json/minimal_observation.json"
        | "composition/canonical_json/minimal_persistent.json"
        | "composition/canonical_json/nested.json"
        | "composition/canonical_json/obs_admin.json"
        | "composition/canonical_json/obs_admin_null_flavour.json"
        | "composition/canonical_json/obs_eva.json"
        | "composition/canonical_json/obs_inst.json"
        | "composition/canonical_json/time_series.json" => reason(
            "defective fixture: a placeholder OBJECT_VERSION_ID whose object_id is not a \
             legal uid (valid twin in tests/fixtures/twins/, refusal in fixture_twins.rs)",
        ),
        // DEFECTIVE upstream fixture, adjudicated: `composer.external_ref.id` is a
        // FHIR-style reference (`PractitionerRole/12345-mock`). `PARTY_REF.id` is
        // an `OBJECT_ID`; as a `HIER_OBJECT_ID` its root must be a `uid`, and no
        // `uid` production admits `/`
        // (`docs/specs/openehr/BASE/docs/base_types/master05-identification_package.adoc`
        // §Syntaxes). Valid twin in `tests/fixtures/twins/`.
        "composition/canonical_json/simple_composition_dvinterval.json" => reason(
            "defective fixture: a FHIR-style HIER_OBJECT_ID root that is not a uid \
             (valid twin in tests/fixtures/twins/, refusal in fixture_twins.rs)",
        ),
        // DEFECTIVE upstream fixture, adjudicated on TWO axes. (a) Three `uid`
        // slots are tagged `_type: OBJECT_VERSION_ID` while carrying a BARE
        // UUID; §Syntaxes gives `object_version_id` exactly three parts, so a
        // one-part value is a `HIER_OBJECT_ID`. (b) Two `OBJECT_REF.id` slots
        // are tagged `_type: OBJECT_REF_ID`, which names NO class in the
        // released specs — `OBJECT_REF.id` is declared `OBJECT_ID`
        // (`…base_types.object_ref.adoc` §Attributes). Valid twin in
        // `tests/fixtures/twins/` (both re-tagged, values kept).
        "folder/canonical_json/folder_with_items.json" => reason(
            "defective fixture: bare-UUID values tagged OBJECT_VERSION_ID, and OBJECT_REF.id \
             tagged with the non-existent class OBJECT_REF_ID (valid twin in \
             tests/fixtures/twins/)",
        ),
        // Malformed: a `DV_TEXT` whose value is under a `name` key instead of `value`.
        "folder/canonical_json/folder_without_duplicates.json" => {
            reason("malformed fixture: a DV_TEXT carries its text under `name` instead of `value`")
        }
        // RM-1.1-era EHRbase output that omits fields RM 1.2 makes mandatory on
        // LOCATABLE. Deserialization is strict here; leniency is a validation-layer
        // concern. Tracked as the RM 1.1↔1.2 divergence (docs/VERSIONS.md).
        "folder/canonical_json/simple_empty_folder.json" => {
            reason("RM 1.1-era: FOLDER omits mandatory LOCATABLE.archetype_node_id (RM 1.2)")
        }
        "item_structure/canonical_json/ehr_other_details.json" => {
            reason("RM 1.1-era: ITEM_TREE omits mandatory LOCATABLE.name (RM 1.2)")
        }
        // Defective per `dv_text.adoc` §Invariants Mappings_valid: a
        // present-but-empty `name.mappings` — refused at parse since #1730
        // (Option<NonEmptyVec>); the refusal twin is pinned in nonempty_wire.rs.
        "folder/canonical_json/flat_folder_insert.json" => reason(
            "defective: DV_TEXT.mappings present but empty (Mappings_valid; #1730 parse refusal)",
        ),
        // The repo-authored twin for `folder_with_items.json` corrects that
        // document's identifier defects, but the FOLDER it corrects also carries
        // an RM-1.1-era `details` ITEM_TREE with neither `name` nor
        // `archetype_node_id`, both mandatory on `LOCATABLE` in RM 1.2
        _ => None,
    }
}

/// The shared service fixture corpus (`corpus/fixtures/service`), resolved
/// from this crate's manifest directory.
pub(crate) fn service_corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/fixtures/service")
}
