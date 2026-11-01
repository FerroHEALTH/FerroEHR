// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! One repository holding several versions of one archetype, under two major
//! versions and two namespaces, each reference resolving to its own.
//!
//! Oracle: BASE `artefact_identification` `master07-referencing.adoc`
//! §Referencing. Two major versions of one archetype "denote different
//! logical archetypes"; an interface reference "will match the latest release
//! available of that major version"; "a pre-release version is referenced by
//! its full identifier"; and "a namespace is never a wildcard", a reference
//! without one denoting "an artefact in the same namespace as the referencing
//! artefact".

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test assertions and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use std::fmt::Write;

use openehr_adl::artefact::ArchetypeRepository;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::opt::create_opt;
use openehr_adl::parse::Dialect;
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetype;

const FAMILY: &str = "openEHR-EHR-OBSERVATION.repo_obs";

/// An ADL2 archetype `hrid` whose root term names it.
fn archetype(hrid: &str) -> Archetype {
    parse(&format!(
        "archetype (adl_version=2.0.6; rm_release=1.1.0)\n    {hrid}\n\n\
         language\n    original_language = <[ISO_639-1::en]>\n\n\
         description\n    lifecycle_state = <\"published\">\n    details = <\n        \
         [\"en\"] = <\n            language = <[ISO_639-1::en]>\n        >\n    >\n\n\
         definition\n    OBSERVATION[id1] matches {{ *}}\n\n\
         terminology\n    term_definitions = <\n        [\"en\"] = <\n            \
         [\"id1\"] = <text = <\"Held {hrid}\"> description = <\"Root.\">>\n        \
         >\n    >\n"
    ))
}

/// An ADL2 template `hrid` filling its content with each of `references`.
fn template_using(hrid: &str, references: &[&str]) -> Archetype {
    let mut fillers = String::new();
    let mut terms = String::new();
    for (i, reference) in references.iter().enumerate() {
        let code = i + 2;
        writeln!(
            fillers,
            "            use_archetype OBSERVATION[id{code}, {reference}]"
        )
        .expect("write to a String");
        writeln!(
            terms,
            "            [\"id{code}\"] = <text = <\"Filler {code}\"> description = <\"Filler.\">>"
        )
        .expect("write to a String");
    }
    parse(&format!(
        "template (adl_version=2.0.6; rm_release=1.1.0)\n    {hrid}\n\n\
         language\n    original_language = <[ISO_639-1::en]>\n\n\
         description\n    lifecycle_state = <\"published\">\n    details = <\n        \
         [\"en\"] = <\n            language = <[ISO_639-1::en]>\n        >\n    >\n\n\
         definition\n    COMPOSITION[id1] matches {{\n        \
         content cardinality matches {{0..*; unordered}} matches {{\n{fillers}        }}\n    }}\n\n\
         terminology\n    term_definitions = <\n        [\"en\"] = <\n            \
         [\"id1\"] = <text = <\"Root\"> description = <\"Root.\">>\n{terms}        >\n    >\n"
    ))
}

fn parse(src: &str) -> Archetype {
    parse_artefact(src, Dialect::Adl2).unwrap_or_else(|e| panic!("parse: {e:?}\n{src}"))
}

/// The physical id of a held archetype.
fn id_of(archetype: &Archetype) -> String {
    match archetype {
        Archetype::AuthoredArchetype(a) => match a.as_ref() {
            AuthoredArchetype::AuthoredArchetype(d) => d.archetype_id.physical_id(),
            other => panic!("not an archetype: {other:?}"),
        },
        Archetype::TemplateOverlay(o) => panic!("not an archetype: {o:?}"),
    }
}

/// A repository holding `ids`, inserted in that order.
fn holding(ids: &[String]) -> ArchetypeRepository {
    let mut repo = ArchetypeRepository::new();
    for id in ids {
        repo.insert(archetype(id));
    }
    repo
}

fn id(version: &str) -> String {
    format!("{FAMILY}.v{version}")
}

/// The physical id `reference` resolves to in `repo`.
fn resolved(repo: &ArchetypeRepository, reference: &str) -> Option<String> {
    repo.get(reference).map(id_of)
}

#[test]
fn two_major_versions_of_one_archetype_resolve_each_to_its_own() {
    let ids = [id("1.0.0"), id("1.10.0"), id("1.9.3"), id("2.0.0")];
    for order in [ids.to_vec(), ids.iter().rev().cloned().collect()] {
        let repo = holding(&order);
        assert_eq!(resolved(&repo, &id("1")), Some(id("1.10.0")), "{order:?}");
        assert_eq!(resolved(&repo, &id("2")), Some(id("2.0.0")), "{order:?}");
        assert_eq!(resolved(&repo, &id("3")), None, "{order:?}");
    }
}

#[test]
fn one_template_compiles_against_both_major_versions() {
    let repo = holding(&[id("1.0.0"), id("1.10.0"), id("2.0.0"), id("2.1.0")]);
    let template = template_using(
        "openEHR-EHR-COMPOSITION.repo_uses_both.v1.0.0",
        &[&id("1"), &id("2")],
    );
    let opt = create_opt(&template, &repo).expect("both fillers resolve");
    let text = format!("{opt:?}");
    for held in [id("1.10.0"), id("2.1.0")] {
        assert!(text.contains(&format!("Held {held}")), "{held} is inlined");
    }
    for other in [id("1.0.0"), id("2.0.0")] {
        assert!(
            !text.contains(&format!("Held {other}")),
            "{other} is not inlined"
        );
    }
}

#[test]
fn more_version_parts_narrow_the_match() {
    let repo = holding(&[id("1.2.0"), id("1.2.7"), id("1.3.0")]);
    assert_eq!(resolved(&repo, &id("1.2")), Some(id("1.2.7")));
    assert_eq!(resolved(&repo, &id("1.2.0")), Some(id("1.2.0")));
    assert_eq!(resolved(&repo, &id("1.4")), None);
}

#[test]
fn a_release_outranks_a_newer_release_candidate() {
    let repo = holding(&[id("1.3.0-rc.2"), id("1.2.0"), id("1.3.0-rc.10")]);
    assert_eq!(resolved(&repo, &id("1")), Some(id("1.2.0")));
    let candidates = holding(&[id("1.3.0-rc.2"), id("1.3.0-rc.10")]);
    assert_eq!(resolved(&candidates, &id("1")), Some(id("1.3.0-rc.10")));
}

#[test]
fn alpha_and_beta_versions_match_only_their_full_identifier() {
    let repo = holding(&[id("1.0.0-alpha"), id("1.0.0-beta.2")]);
    assert_eq!(resolved(&repo, &id("1")), None);
    assert_eq!(
        resolved(&repo, &id("1.0.0-beta.2")),
        Some(id("1.0.0-beta.2"))
    );
}

#[test]
fn the_family_is_case_insensitive_and_other_families_never_match() {
    let repo = holding(&[
        "openehr-ehr-observation.REPO_OBS.v1.0.0".to_owned(),
        format!("{FAMILY}2.v1.1.0"),
    ]);
    assert_eq!(
        resolved(&repo, &id("1")),
        Some("openehr-ehr-observation.REPO_OBS.v1.0.0".to_owned())
    );
}

#[test]
fn a_namespace_is_never_a_wildcard() {
    let repo = holding(&[id("1.0.0"), format!("org.example::{}", id("1.5.0"))]);
    assert_eq!(resolved(&repo, &id("1")), Some(id("1.0.0")));
    assert_eq!(
        resolved(&repo, &format!("org.example::{}", id("1"))),
        Some(format!("org.example::{}", id("1.5.0")))
    );
    assert_eq!(resolved(&repo, &format!("org.other::{}", id("1"))), None);
    assert_eq!(
        repo.resolve(&id("1"), Some("org.example")).map(id_of),
        Some(format!("org.example::{}", id("1.5.0"))),
        "a reference without a namespace takes the referrer's"
    );
    assert_eq!(
        repo.resolve(&format!("org.example::{}", id("1")), Some("org.other"))
            .map(id_of),
        Some(format!("org.example::{}", id("1.5.0"))),
        "a reference's own namespace wins over the referrer's"
    );
}

#[test]
fn inserting_a_held_version_again_replaces_it() {
    let mut repo = holding(&[id("1.0.0")]);
    repo.insert(archetype(&id("1.0.0")));
    repo.insert(archetype(&id("1.0.1")));
    assert_eq!(resolved(&repo, &id("1")), Some(id("1.0.1")));
    assert_eq!(resolved(&repo, &id("1.0.0")), Some(id("1.0.0")));
}
