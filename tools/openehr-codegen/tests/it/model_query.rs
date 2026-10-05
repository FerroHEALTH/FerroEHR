// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `model-query` report, tested over the **real** vendored BMM inputs
//! through `openehr_codegen::testsupport` — the same projection the CLI
//! subcommand prints.
//!
//! The golden rows below are the BMM's own statement of two RM classes, and
//! they are cross-checked against the normative UML tables
//! (`RM/docs/UML/classes/org.openehr.rm.common.folder.adoc` §FOLDER Class and
//! `…org.openehr.rm.common.locatable.adoc` §LOCATABLE Class): `FOLDER.items`,
//! `FOLDER.folders` and `FOLDER.details` are all `0..1`, the first two typed
//! `List<>`; `LOCATABLE` is abstract with `name`/`archetype_node_id` at `1..1`
//! and `uid`/`links`/`archetype_details`/`feeder_audit` at `0..1`. A BMM pin
//! bump that changes an existence, a cardinality, a declared type or the
//! emitted field shape therefore fails here.

use openehr_codegen::testsupport;

/// The report's header line, as its 13 column names.
const HEADER: [&str; 14] = [
    "component",
    "bmm",
    "package",
    "class",
    "abstract",
    "class_emission",
    "decl",
    "declared_on",
    "attribute",
    "bmm_type",
    "existence",
    "container",
    "cardinality",
    "emission",
];

/// `FOLDER` (RM 1.2.0), sorted by attribute name as the report sorts.
const FOLDER: [[&str; 14]; 3] = [
    [
        "rm",
        "openehr_rm_1.2.0",
        "common/directory",
        "FOLDER",
        "-",
        "struct",
        "2",
        "FOLDER",
        "details",
        "ITEM_STRUCTURE",
        "0..1",
        "-",
        "-",
        "Option<ItemStructure>",
    ],
    [
        "rm",
        "openehr_rm_1.2.0",
        "common/directory",
        "FOLDER",
        "-",
        "struct",
        "1",
        "FOLDER",
        "folders",
        "List<FOLDER>",
        "0..1",
        "List",
        "0..*",
        "Option<Vec<Folder>>",
    ],
    [
        "rm",
        "openehr_rm_1.2.0",
        "common/directory",
        "FOLDER",
        "-",
        "struct",
        "0",
        "FOLDER",
        "items",
        "List<OBJECT_REF>",
        "0..1",
        "List",
        "0..*",
        "Option<Vec<ObjectRef>>",
    ],
];

/// `LOCATABLE` (RM 1.2.0) — abstract, so the class emits an untagged enum over
/// its concrete descendants and these attributes flatten into each of them.
const LOCATABLE: [[&str; 14]; 6] = [
    [
        "rm",
        "openehr_rm_1.2.0",
        "common/archetyped",
        "LOCATABLE",
        "abstract",
        "enum",
        "4",
        "LOCATABLE",
        "archetype_details",
        "ARCHETYPED",
        "0..1",
        "-",
        "-",
        "Option<Archetyped>",
    ],
    [
        "rm",
        "openehr_rm_1.2.0",
        "common/archetyped",
        "LOCATABLE",
        "abstract",
        "enum",
        "1",
        "LOCATABLE",
        "archetype_node_id",
        "String",
        "1..1",
        "-",
        "-",
        "String",
    ],
    [
        "rm",
        "openehr_rm_1.2.0",
        "common/archetyped",
        "LOCATABLE",
        "abstract",
        "enum",
        "5",
        "LOCATABLE",
        "feeder_audit",
        "FEEDER_AUDIT",
        "0..1",
        "-",
        "-",
        "Option<FeederAudit>",
    ],
    [
        "rm",
        "openehr_rm_1.2.0",
        "common/archetyped",
        "LOCATABLE",
        "abstract",
        "enum",
        "3",
        "LOCATABLE",
        "links",
        "List<LINK>",
        "0..1",
        "List",
        "0..*",
        "Option<openehr_base::containers::NonEmptyVec<Link>>",
    ],
    [
        "rm",
        "openehr_rm_1.2.0",
        "common/archetyped",
        "LOCATABLE",
        "abstract",
        "enum",
        "0",
        "LOCATABLE",
        "name",
        "DV_TEXT",
        "1..1",
        "-",
        "-",
        "DvText",
    ],
    [
        "rm",
        "openehr_rm_1.2.0",
        "common/archetyped",
        "LOCATABLE",
        "abstract",
        "enum",
        "2",
        "LOCATABLE",
        "uid",
        "UID_BASED_ID",
        "0..1",
        "-",
        "-",
        "Option<UidBasedId>",
    ],
];

/// The TSV text a header plus these rows must render as. The RM crate emits
/// TWO generations (#1942: RM 1.1.0 released beside RM 1.2.0), and both
/// declare these classes with identical shapes, so the expected report is the
/// golden rows once per generation — `bmm` column first `openehr_rm_1.1.0`,
/// then the pinned `openehr_rm_1.2.0` rows verbatim (the report sorts by
/// (component, bmm, class, attribute)).
fn expected_tsv(rows: &[[&str; 14]]) -> String {
    let mut out = String::new();
    out.push_str(&HEADER.join("\t"));
    out.push('\n');
    for row in rows {
        let mut v1_1 = *row;
        v1_1[1] = "openehr_rm_1.1.0";
        out.push_str(&v1_1.join("\t"));
        out.push('\n');
    }
    for row in rows {
        out.push_str(&row.join("\t"));
        out.push('\n');
    }
    out
}

/// `FOLDER`'s reported rows are exactly what the vendored RM BMM states, with
/// the field shapes the emitter currently emits (`Option<Vec<T>>` for both
/// `0..1 List<>` attributes — absence and present-but-emptiness are distinct
/// model states — and `Option<T>` for the optional single one).
#[test]
fn folder_rows_match_the_vendored_bmm() {
    let report = testsupport::model_query(Some("rm"), Some("FOLDER"), None, "tsv")
        .expect("the RM composition loads");
    assert_eq!(report, expected_tsv(&FOLDER));
}

/// `LOCATABLE`'s reported rows, including its `abstract` marker and the `enum`
/// class shape that abstractness produces.
#[test]
fn locatable_rows_match_the_vendored_bmm() {
    let report = testsupport::model_query(Some("rm"), Some("LOCATABLE"), None, "tsv")
        .expect("the RM composition loads");
    assert_eq!(report, expected_tsv(&LOCATABLE));
}

/// An `--attribute` filter narrows to that one attribute of the class.
#[test]
fn an_attribute_filter_selects_one_row() {
    let report = testsupport::model_query(Some("rm"), Some("FOLDER"), Some("items"), "tsv")
        .expect("the RM composition loads");
    let items = FOLDER
        .iter()
        .filter(|r| r.get(8) == Some(&"items"))
        .copied()
        .collect::<Vec<_>>();
    assert_eq!(report, expected_tsv(&items));
}

/// The unfiltered report is byte-identical across runs in every format — the
/// property that makes it diffable across a BMM pin bump.
#[test]
fn the_report_is_deterministic() {
    for format in ["table", "tsv", "json"] {
        let first =
            testsupport::model_query(None, None, None, format).expect("every composition loads");
        let second =
            testsupport::model_query(None, None, None, format).expect("every composition loads");
        assert_eq!(first, second, "{format}: two runs differ");
    }
}

/// Non-vacuity: the unfiltered report covers every composition and thousands of
/// attributes, so a silently narrowed scope cannot pass the goldens above.
#[test]
fn the_report_covers_every_component() {
    let report =
        testsupport::model_query(None, None, None, "tsv").expect("every composition loads");
    let rows: Vec<&str> = report.lines().skip(1).collect();
    assert!(rows.len() > 1000, "only {} attribute rows", rows.len());
    for key in testsupport::crate_keys() {
        assert!(
            rows.iter().any(|r| r.starts_with(&format!("{key}\t"))),
            "component {key} contributed no row",
        );
    }
}

/// Every filter rejects an unknown value loudly, naming the valid ones.
#[test]
fn unknown_filter_values_are_rejected_with_the_valid_ones() {
    let component = testsupport::model_query(Some("ehr"), None, None, "tsv")
        .expect_err("`ehr` is not a composition key")
        .to_string();
    assert!(component.contains("unknown component"), "{component}");
    assert!(
        component.contains("base, rm, lang, am, term"),
        "{component}"
    );

    let class = testsupport::model_query(Some("rm"), Some("Folder"), None, "tsv")
        .expect_err("class names are the BMM's own spelling")
        .to_string();
    assert!(class.contains("unknown class"), "{class}");
    assert!(class.contains("FOLDER"), "{class}");

    let attribute = testsupport::model_query(Some("rm"), Some("FOLDER"), Some("nope"), "tsv")
        .expect_err("`nope` is not a FOLDER attribute")
        .to_string();
    assert!(attribute.contains("unknown attribute"), "{attribute}");
    assert!(attribute.contains("details, folders, items"), "{attribute}");

    let format = testsupport::model_query(None, None, None, "yaml")
        .expect_err("`yaml` is not a report format")
        .to_string();
    assert!(format.contains("table, tsv, json"), "{format}");
}

/// The flattened view reports every attribute a class CARRIES, not just the
/// ones it declares — the inheritance dimension.
///
/// `LOCATABLE.links` is declared once on the abstract `LOCATABLE`
/// (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.locatable.adoc`
/// §Attributes) and inherited by every archetyped RM class, so the declared
/// view reports it once PER GENERATION (both emitted RM generations declare
/// it) while the flattened view reports it once per carrier, each naming
/// `LOCATABLE` as the declaring class.
#[test]
fn the_flattened_view_reports_inherited_attributes_per_carrier() {
    let declared = testsupport::model_query_view(Some("rm"), None, Some("links"), "tsv", false)
        .expect("the rm composition loads");
    let flattened = testsupport::model_query_view(Some("rm"), None, Some("links"), "tsv", true)
        .expect("the rm composition loads");
    let rows = |report: &str| report.lines().skip(1).count();
    assert_eq!(
        rows(&declared),
        2,
        "links is declared exactly once per RM generation, on LOCATABLE"
    );
    assert!(
        rows(&flattened) > 1,
        "the flattened view must report every class that carries links, got {}",
        rows(&flattened)
    );
    for line in flattened.lines().skip(1) {
        let cells: Vec<&str> = line.split('\t').collect();
        assert_eq!(
            cells.get(7).copied(),
            Some("LOCATABLE"),
            "every carrier must name LOCATABLE as the declaring class: {line}"
        );
    }
}

/// The emission column of the flattened view is computed for the CARRYING
/// class, so a per-descendant divergence is visible rather than assumed absent.
/// `LOCATABLE.links` resolves identically everywhere; the assertion pins that
/// so a future emitter change that breaks the uniformity is caught here.
#[test]
fn a_uniformly_inherited_container_emits_one_shape_for_every_carrier() {
    let report = testsupport::model_query_view(Some("rm"), None, Some("links"), "tsv", true)
        .expect("the rm composition loads");
    let shapes: std::collections::BTreeSet<&str> = report
        .lines()
        .skip(1)
        .filter_map(|l| l.split('\t').nth(13))
        .collect();
    // NOTE: `Links_valid` (RM common `locatable.adoc` §Invariants) makes the
    // optional list non-empty-when-present, hence `Option<NonEmptyVec<_>>`.
    assert_eq!(
        shapes,
        ["Option<openehr_base::containers::NonEmptyVec<Link>>"]
            .into_iter()
            .collect(),
        "LOCATABLE.links must emit one shape for every carrier"
    );
}
