// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADL 1.4 dADL breadth regression corpus (`tests/corpus/adl14-dadl/`).
//!
//! Unlike every other tree under `tests/corpus/`, this one is **hand-written,
//! not vendored** (`tests/corpus/PROVENANCE.md` §`adl14-dadl/`): the vendored
//! `adl2-reference` library exercises only a fraction of the dADL leaf
//! grammar, so the forms `AM/docs/ADL1.4/master04-dadl` defines but no
//! vendored artefact uses would otherwise have no regression net.
//!
//! Every fixture states its expectation in its file name, the corpus
//! convention (`tests/corpus/INVENTORY.md`), and the accepting ones repeat it
//! in the `regression` tag. Expectations here are derived from the spec text
//! first-hand — the citation for each construct is in the fixture beside it,
//! and in the reader that implements it (`openehr_lang::v1_1::odin`).
//!
//! Both twins are kept for every refusal (`.claude/rules/testing.md`): a form
//! the reader must ACCEPT lives in the breadth fixture, and a form it must
//! REFUSE gets its own `SDINV_*` fixture, so a reader that turns lenient fails
//! here rather than silently mis-reading an archetype.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test assertions, diagnostics and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use std::path::{Path, PathBuf};

use openehr_adl::assemble::parse_artefact;
use openehr_adl::error::SyntaxErrorCode;
use openehr_adl::meta::regression_tag;
use openehr_adl::parse::Dialect;
use openehr_adl::source::parse_source;
use openehr_adl::validate::catalogue::Severity;
use openehr_adl::validate::validate_source_integrity;
use openehr_lang::v1_1::odin::{OdinKey, OdinValue};

/// What a fixture must do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// Parses in the 1.4 dialect and validates clean under the 1.4 phase-1
    /// subset.
    Pass,
    /// Refused at parse with this code.
    Refuse(SyntaxErrorCode),
}

/// Every fixture of the tree, with its expected outcome. The coverage gate
/// (`corpus_coverage.rs`) cross-checks this list against the filesystem.
const FIXTURES: &[(&str, Expect)] = &[
    ("openEHR-EHR-OBSERVATION.dadl_breadth.v1.adl", Expect::Pass),
    (
        "openEHR-EHR-OBSERVATION.revision_history.v1.adl",
        Expect::Pass,
    ),
    (
        "openEHR-EHR-OBSERVATION.SDINV_duplicate_sibling_attribute.v1.adl",
        Expect::Refuse(SyntaxErrorCode::Sdinv),
    ),
];

fn tree() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/adl14-dadl")
}

fn read(name: &str) -> String {
    let path = tree().join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn every_fixture_meets_its_declared_outcome() {
    for (name, expect) in FIXTURES {
        let src = read(name);
        match expect {
            Expect::Pass => {
                let archetype = parse_artefact(&src, Dialect::Adl14)
                    .unwrap_or_else(|e| panic!("{name} must parse, got {e:?}"));
                assert_eq!(
                    regression_tag(&archetype).as_deref(),
                    Some("PASS"),
                    "{name}: the in-file regression tag must agree with the table"
                );
                let issues = validate_source_integrity(&src, Dialect::Adl14, None)
                    .unwrap_or_else(|e| panic!("{name} must parse for validation, got {e:?}"));
                let errors: Vec<_> = issues
                    .iter()
                    .filter(|i| i.severity == Severity::Error)
                    .collect();
                assert!(errors.is_empty(), "{name} must validate clean: {errors:?}");
            }
            Expect::Refuse(code) => {
                let errs = parse_artefact(&src, Dialect::Adl14)
                    .expect_err(&format!("{name} must be refused"));
                assert!(
                    errs.iter().any(|e| e.code == *code),
                    "{name}: expected {code:?}, got {errs:?}"
                );
            }
        }
    }
}

/// The breadth fixture's newly-accepted leaf forms land as the right
/// [`OdinValue`]s, not merely "parses without error".
#[test]
fn breadth_fixture_leaf_values() {
    let src = read("openEHR-EHR-OBSERVATION.dadl_breadth.v1.adl");
    let art = parse_source(&src, Dialect::Adl2).expect("breadth fixture parses");
    let description = art.description.as_ref().expect("description section");
    let OdinValue::Object(desc) = description else {
        panic!("expected an object description section");
    };
    let OdinValue::KeyedList(entries) = desc.get("other_details").expect("other_details") else {
        panic!("expected a keyed other_details list");
    };
    let get = |key: &str| -> &OdinValue {
        entries
            .iter()
            .find(|(k, _)| matches!(k, OdinKey::String(s) if s == key))
            .map_or_else(|| panic!("other_details[{key:?}] missing"), |(_, v)| v)
    };

    // `<...>` empty sections (master04 §Empty Sections).
    assert_eq!(get("empty_leaf"), &OdinValue::Empty);
    assert_eq!(get("empty_block"), &OdinValue::Empty);
    assert_eq!(
        get("empty_cast"),
        &OdinValue::Typed {
            rm_type: "PENSION".to_owned(),
            value: Box::new(OdinValue::Empty),
        }
    );

    // Namespaced type casts (master04 §Adding Type Information): the package
    // path is preserved flat, exactly as authored, for both the lower-case and
    // the upper-case package spelling the chapter exemplifies, and on a
    // template parameter.
    let OdinValue::Typed { rm_type, value } = get("namespaced_cast_lc_package") else {
        panic!("expected a namespaced typed cast");
    };
    assert_eq!(rm_type, "org.openehr.rm.ehr.content.ENTRY");
    assert!(matches!(**value, OdinValue::Object(_)));
    let OdinValue::Typed { rm_type, .. } = get("namespaced_cast_uc_package") else {
        panic!("expected a namespaced typed cast");
    };
    assert_eq!(rm_type, "Core.Abstractions.Relationships.Relationship");
    let OdinValue::Typed { rm_type, .. } = get("namespaced_cast_generic") else {
        panic!("expected a namespaced generic typed cast");
    };
    assert_eq!(rm_type, "List<org.openehr.rm.ehr.content.ENTRY>");

    // Partial date/times (master04 §Partial Date/Times).
    assert_eq!(
        get("date_time_unknown_time"),
        &OdinValue::DateTime("2004-06-11T??:??:??".to_owned())
    );
    assert_eq!(
        get("date_time_unknown_month_day_time"),
        &OdinValue::DateTime("2004-??-??T??:??:??".to_owned())
    );

    // Integer exponent + case-insensitive booleans (master04 §Integer Data,
    // §Boolean Data).
    assert_eq!(get("integer_exponent"), &OdinValue::Integer(29_000_000));
    assert_eq!(get("boolean_upper"), &OdinValue::Boolean(true));
    assert_eq!(get("boolean_mixed"), &OdinValue::Boolean(false));

    // Local term codes as leaf values (master04 §Lists of Built-in Types).
    assert_eq!(
        get("local_code"),
        &OdinValue::TermCode("[at0200]".to_owned())
    );
    assert_eq!(
        get("local_code_specialised"),
        &OdinValue::TermCode("[at0010.2]".to_owned())
    );

    // Multi-line leader removal (master04 §String Data) and `&quot;` verbatim.
    assert_eq!(
        get("multi_line"),
        &OdinValue::String(
            "And now the STORM-BLAST came, and he\nWas tyrannous and strong :\nAnd chased us south along."
                .to_owned()
        )
    );
    assert_eq!(
        get("quoted_phrase"),
        &OdinValue::String("what one might call a &quot;phrase&quot;.".to_owned())
    );
}

/// Unbounded interval endpoints land as `None` bounds (master04 §Intervals of
/// Ordered Primitive Types: `infinity` / `-infinity` / `*`).
#[test]
fn breadth_fixture_unbounded_intervals() {
    let src = read("openEHR-EHR-OBSERVATION.dadl_breadth.v1.adl");
    let art = parse_source(&src, Dialect::Adl2).expect("breadth fixture parses");
    let OdinValue::Object(desc) = art.description.as_ref().expect("description") else {
        panic!("expected an object description section");
    };
    let OdinValue::KeyedList(entries) = desc.get("other_details").expect("other_details") else {
        panic!("expected a keyed other_details list");
    };
    let bounds = |key: &str| -> (bool, bool) {
        let value = entries
            .iter()
            .find(|(k, _)| matches!(k, OdinKey::String(s) if s == key))
            .map_or_else(|| panic!("other_details[{key:?}] missing"), |(_, v)| v);
        let OdinValue::Interval(openehr_lang::v1_1::odin::OdinInterval::Range {
            lower, upper, ..
        }) = value
        else {
            panic!("{key}: expected a range interval, got {value:?}");
        };
        (lower.is_some(), upper.is_some())
    };
    assert_eq!(bounds("interval_to_infinity"), (true, false));
    assert_eq!(bounds("interval_to_star"), (true, false));
    assert_eq!(bounds("interval_from_neg_infinity"), (false, true));
    assert_eq!(bounds("interval_int"), (true, true));
}

/// A `(TYPE)`-cast section value assembles exactly like the uncast form.
///
/// The cast of `AM/docs/ADL1.4/master04-dadl` §Adding Type Information (and
/// `LANG/docs/odin/master05-content` §Adding Type Information) is a
/// dynamic-binding hint for the parser, not part of the datum — every
/// assemble accessor reads through it. The breadth fixture writes its
/// `translations` block cast twice over (on the container and on the entry),
/// so a reader that stops at the cast produces NO translations at all.
#[test]
fn cast_section_values_assemble_transparently() {
    let src = read("openEHR-EHR-OBSERVATION.dadl_breadth.v1.adl");
    let archetype = parse_artefact(&src, Dialect::Adl14).expect("breadth fixture parses");
    let openehr_am::v2_4::aom2::archetype::archetype::Archetype::AuthoredArchetype(authored) =
        &archetype
    else {
        panic!("expected an authored archetype");
    };
    let openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetype::AuthoredArchetype(
        data,
    ) = authored.as_ref()
    else {
        panic!("expected a plain authored archetype");
    };
    let translations = data
        .translations
        .as_ref()
        .expect("the cast translations block must still assemble");
    let de = translations.get("de").expect("the `de` translation");
    assert_eq!(de.language.code_string, "de");
    assert_eq!(de.author.get("name").map(String::as_str), Some("Ein Autor"));
}

/// The ADL 1.4 `revision_history` section parses and is preserved on the
/// source artefact (`AM/docs/ADL1.4/master08-adl` §Revision History Section).
/// It has no AOM2 landing field by upstream decision —
/// `AM/docs/ADL2/master01-preface` §Changes from ADL 1.4 removed the section
/// "since the AOM2 uses the openEHR Base Types version of the Resource
/// package" (SPECAM-61) — so the assertion is that the 1.4 read model keeps
/// it, not that the assembled artefact grows a field.
#[test]
fn revision_history_section_is_read_and_preserved() {
    let src = read("openEHR-EHR-OBSERVATION.revision_history.v1.adl");
    let art = parse_source(&src, Dialect::Adl2).expect("revision_history fixture parses");
    let OdinValue::Object(section) = art
        .revision_history
        .as_ref()
        .expect("revision_history section retained")
    else {
        panic!("expected an object revision_history section");
    };
    let OdinValue::KeyedList(revisions) = section
        .get("revision_history")
        .expect("revision_history attribute")
    else {
        panic!("expected a keyed revision list");
    };
    assert_eq!(revisions.len(), 3);
    assert_eq!(revisions[0].0, OdinKey::String("1.57".to_owned()));
    let OdinValue::Object(first) = &revisions[0].1 else {
        panic!("expected an object revision entry");
    };
    assert_eq!(
        first.get("committer"),
        Some(&OdinValue::String("Miriam Hanoosh".to_owned()))
    );
    // The spec's own example writes the timestamp with a space instead of the
    // ISO `T` designator its own §Complete Date/Times mandates; it is accepted
    // and normalised (see the `openehr_lang::v1_1::odin` lexer NOTE).
    assert_eq!(
        first.get("time_committed"),
        Some(&OdinValue::DateTime("2004-11-02T09:31:04+1000".to_owned()))
    );
}
