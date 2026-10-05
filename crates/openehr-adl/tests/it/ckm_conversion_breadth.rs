// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! ADL 1.4 → ADL 2 conversion, exercised across the whole real-world corpus.
//!
//! `adl14_conversion.rs` is the DEEP gate: a handful of paired fixtures whose
//! converted output is compared structurally against an expected `.adls`. This
//! is the BREADTH gate over the same converter: every ADL 1.4 archetype openEHR
//! actually publishes — 944 from the live CKM plus the 330 ADL 1.4 twins of
//! upstream's paired export — must survive the full pipeline
//!
//! ```text
//! 1.4 source → convert → print as ADL 2 → re-parse in the ADL 2 dialect
//! ```
//!
//! without error. The re-parse is what makes the claim meaningful: a converter
//! that emits something its own reader cannot read has produced nothing usable,
//! and printing is where a half-converted node code or a dangling terminology
//! reference actually surfaces.
//!
//! NOTE: no openEHR spec governs 1.4→2 conversion — the whole `adl14` pipeline
//! is our own design (archie is prior art only), so the claim asserted here is
//! self-consistency (convert → print → re-read), not conformance to a spec
//! clause. Structural comparison against upstream's own `.adls` conversion of
//! the same archetypes is a separate, stronger claim and is NOT made here (see
//! the pack provenance).
//!
//! Corpus discipline: 100% exercised, adjudicated failures only. A source the
//! pipeline cannot carry is listed in `ADJUDICATED` with the reason, and the
//! gate asserts it still fails — a negative test, not a skip.
#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test assertions and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use std::path::{Path, PathBuf};

use openehr_adl::adl14::convert::{ConvertConfig, parse_and_convert};
use openehr_adl::adl14::log::ConversionLog;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::parse::Dialect;
use openehr_adl::print::print;

/// Sources that cannot reach the end of the pipeline, with the reason.
///
/// The 13 archetypes that do not PARSE as 1.4 are excluded structurally rather
/// than listed here — they are adjudicated (with their expected refusal codes)
/// in `ckm_archetype_packs.rs`, and a source that never parses cannot be
/// converted. Listing them twice would double-book one adjudication.
const ADJUDICATED: &[(&str, &str)] = &[];

fn packs() -> Vec<(&'static str, PathBuf)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    vec![
        ("CKM ADL 1.4", root.join("archetypes/ckm/adl14")),
        (
            "upstream 1.4 twins",
            root.join("archetypes/adl2/ckm-2013-12-09"),
        ),
    ]
}

fn adl_sources(dir: &Path) -> Vec<PathBuf> {
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
            } else if path.extension().is_some_and(|e| e == "adl") {
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

/// Convert → print → re-parse. `Ok(())` when the whole pipeline holds.
fn round_trip(src: &str) -> Result<(), String> {
    let mut log = ConversionLog::default();
    let converted = parse_and_convert(src, &ConvertConfig::default(), &mut log)
        .map_err(|e| format!("convert failed: {e:?}"))?;
    let printed = print(&converted).map_err(|e| format!("print refused: {e}"))?;
    parse_artefact(&printed, Dialect::Adl2)
        .map(|_| ())
        .map_err(|errors| {
            // Name the offending printed line: a round-trip failure is a defect
            // in what the converter EMITTED, so the emitted text is the evidence.
            let detail = errors
                .iter()
                .map(|e| {
                    let line = printed
                        .lines()
                        .nth(e.line.saturating_sub(1))
                        .unwrap_or("<past end>")
                        .trim();
                    format!(
                        "{} at {}:{} near `{line}`: {}",
                        e.code, e.line, e.column, e.message
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            format!("the converted ADL 2 does not re-parse: {detail}")
        })
}

/// Every real-world ADL 1.4 archetype that PARSES also converts to ADL 2,
/// prints, and re-parses as ADL 2.
#[test]
fn every_parseable_real_world_archetype_converts_and_round_trips() {
    let mut failures = Vec::new();
    let mut stale = Vec::new();
    let mut converted = 0_usize;
    let mut unparseable = 0_usize;
    let mut total = 0_usize;

    for (label, dir) in packs() {
        let sources = adl_sources(&dir);
        assert!(
            sources.len() >= 300,
            "{label}: pack is missing ({} sources in {}) — re-run the vendor script",
            sources.len(),
            dir.display()
        );
        for path in &sources {
            total += 1;
            let name = file_name(path);
            let src = std::fs::read_to_string(path).expect("read archetype source");
            // A source that does not parse as 1.4 is already adjudicated in
            // `ckm_archetype_packs.rs`; conversion has nothing to work on.
            if parse_artefact(&src, Dialect::Adl14).is_err() {
                unparseable += 1;
                continue;
            }
            let expected = ADJUDICATED.iter().find(|(file, _)| *file == name);
            match (round_trip(&src), expected) {
                (Ok(()), None) => converted += 1,
                (Ok(()), Some((_, reason))) => stale.push(format!(
                    "{label}/{name}: adjudicated as unconvertible ({reason}) but the \
                     pipeline now holds — remove the entry"
                )),
                (Err(why), None) => failures.push(format!("{label}/{name}: {why}")),
                (Err(_), Some(_)) => converted += 0,
            }
        }
    }

    println!(
        "conversion breadth: {total} sources, {unparseable} unparseable (adjudicated \
         elsewhere), {converted} converted + printed + re-parsed"
    );

    assert!(
        stale.is_empty(),
        "adjudications that no longer describe reality:\n{}",
        stale.join("\n")
    );
    assert!(
        failures.is_empty(),
        "{} of {total} real-world 1.4 archetypes fail the convert → print → \
         re-parse pipeline and are not adjudicated:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(
        converted >= 1200,
        "expected the whole real-world 1.4 corpus to be exercised, converted only \
         {converted} of {total}"
    );
}

/// The #1466 partition shape, pinned structurally on the hand-written fixture
/// (`tests/corpus/adl14-cadl/openEHR-EHR-CLUSTER.heterogeneous_quantity_rows.v1.adl`,
/// the live `range_of_motion` shape): `list` rows constraining DIFFERENT
/// member sets partition into one `DV_QUANTITY` alternative per set —
/// ADL 2 documents tuple rows only with a constraint in every member
/// (`ADL2/master04.4` §Tuple Constraints), so the union-with-holes tuple the
/// converter used to emit (`[{"mm"}, {}]`) has no spec form. The printed text
/// must re-parse, carry no empty tuple item, and keep the `deg`↔range pairing
/// and the row-1-only `assumed_value` inside the tuple alternative.
#[test]
fn heterogeneous_rows_partition_into_alternatives() {
    let src = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/corpus/adl14-cadl/openEHR-EHR-CLUSTER.heterogeneous_quantity_rows.v1.adl"),
    )
    .expect("fixture exists");
    let mut log = ConversionLog::new();
    let converted = parse_and_convert(&src, &ConvertConfig::default(), &mut log)
        .expect("the heterogeneous-rows fixture converts");
    let printed = print(&converted).expect("print the converted artefact");
    assert!(
        !printed.contains("{}"),
        "no empty tuple member may be printed:\n{printed}"
    );
    // Two DV_QUANTITY alternatives: the tuple one (units+magnitude) and the
    // plain-units one (mm/cm merged, magnitude simply unconstrained).
    let alternatives = printed.matches("DV_QUANTITY[").count();
    assert_eq!(
        alternatives, 2,
        "expected exactly two DV_QUANTITY alternatives:\n{printed}"
    );
    assert!(
        printed.contains("[units, magnitude]"),
        "the tuple alternative keeps the co-constrained members:\n{printed}"
    );
    assert!(
        printed.contains("\"mm\", \"cm\"") || printed.contains("\"mm\",\"cm\""),
        "the units-only rows merge into one plain constraint:\n{printed}"
    );
    // The assumed_value satisfies row 1 only — it must sit inside the tuple
    // alternative (an assumed 90.0 on the magnitude member).
    assert!(
        printed.contains("; 90.0"),
        "the assumed magnitude lands on the tuple alternative:\n{printed}"
    );
    // And the whole printed artefact re-parses in the ADL 2 dialect.
    parse_artefact(&printed, Dialect::Adl2)
        .unwrap_or_else(|e| panic!("converted output must re-parse: {e:?}"));
}
