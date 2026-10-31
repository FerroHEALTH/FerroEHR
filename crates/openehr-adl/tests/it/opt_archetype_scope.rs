// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Archetype scope in an operational template: the node ids and codes below a
//! `C_ARCHETYPE_ROOT` belong to the constituent archetype it includes.
//!
//! Oracle: `AOM2/master04.5-constraint_model-class_definitions.adoc` §Validity
//! Rules `C_OBJECT` (VCOSU: "every object node must be unique within the
//! archetype") and `OPT2/master03-opt_raw.adoc` (the flat terminology of each
//! constituent archetype is gathered under `component_terminologies`). The
//! fixture is a synthetic operational template: a COMPOSITION whose `content`
//! includes an OBSERVATION archetype that reuses the template's `id2` and `id3`.

#![allow(
    clippy::expect_used,
    reason = "integration-test fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_adl::artefact::ArchetypeRepository;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::parse::Dialect;
use openehr_adl::validate::bindings::NoTerminologyResolver;
use openehr_adl::validate::catalogue::{Severity, ValidationCode};
use openehr_adl::validate::rm::ProductionRmModel;
use openehr_adl::validate::{ValidationIssue, validate_source, validate_source_integrity};

/// A minimal operational template whose included OBSERVATION reuses the node
/// ids `id2` and `id3` of the COMPOSITION template around it.
const OPT: &str = "operational_template (adl_version=2.0.6; rm_release=1.0.2; generated)
    openEHR-EHR-COMPOSITION.opt_archetype_scope.v1.0.0

language
    original_language = <[ISO_639-1::en]>

description
    original_author = <
        [\"name\"] = <\"Veredictum contributors\">
    >
    lifecycle_state = <\"unmanaged\">
    details = <
        [\"en\"] = <
            language = <[ISO_639-1::en]>
            purpose = <\"A minimal operational template for conformance testing\">
        >
    >

definition
    COMPOSITION[id1] matches {
        category matches {
            DV_CODED_TEXT[id2] matches {
                defining_code matches {[at1]}
            }
        }
        content cardinality matches {0..*; unordered} matches {
            OBSERVATION[id3, openEHR-EHR-OBSERVATION.cnf_minimal.v1.0.0] occurrences matches {0..*} matches {
                data matches {
                    HISTORY[id2] matches {
                        events cardinality matches {1..*; unordered} matches {
                            EVENT[id3] occurrences matches {0..*} matches {
                                data matches {
                                    ITEM_TREE[id4] matches {
                                        items cardinality matches {0..*; unordered} matches {
                                            ELEMENT[id5] occurrences matches {0..*} matches {
                                                value matches {
                                                    DV_TEXT[id6]
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

terminology
    term_definitions = <
        [\"en\"] = <
            [\"id1\"] = <
                text = <\"Minimal\">
                description = <\"A minimal event composition\">
            >
            [\"id3\"] = <
                text = <\"Minimal observation\">
                description = <\"The observation the composition holds\">
            >
            [\"at1\"] = <
                text = <\"event\">
                description = <\"The openEHR composition category event\">
            >
        >
    >
    term_bindings = <
        [\"openehr\"] = <
            [\"at1\"] = <http://openehr.org/id/433>
        >
    >

component_terminologies
    [\"openEHR-EHR-OBSERVATION.cnf_minimal.v1.0.0\"] = <
        term_definitions = <
            [\"en\"] = <
                [\"id1\"] = <
                    text = <\"Minimal observation\">
                    description = <\"An observation with one text element\">
                >
                [\"id3\"] = <
                    text = <\"Any event\">
                    description = <\"Any event\">
                >
                [\"id5\"] = <
                    text = <\"Value\">
                    description = <\"A text value\">
                >
            >
        >
    >
";

/// The errors the full ADL 2 catalogue raises for `src`, as `(code, path)`.
fn errors(src: &str) -> Vec<(ValidationCode, Option<String>)> {
    let issues: Vec<ValidationIssue> =
        validate_source(src, None, &ProductionRmModel, &NoTerminologyResolver)
            .expect("the fixture must parse");
    issues
        .into_iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| (i.code, i.path))
        .collect()
}

/// `src` with `from` replaced by `to`; `from` must occur exactly once.
fn edit(src: &str, from: &str, to: &str) -> String {
    assert_eq!(src.matches(from).count(), 1, "{from:?} must occur once");
    src.replacen(from, to, 1)
}

/// The template's own terminology anchor, before which a term is added.
const TEMPLATE_TERM: &str = "            [\"at1\"] = <\n                text = <\"event\">";

/// The constituent's terminology anchor, before which a term is added.
const CONSTITUENT_TERM: &str = "                [\"id5\"] = <\n";

/// A term definition for `code`, indented for the template's terminology.
fn template_term(code: &str) -> String {
    format!(
        "            [\"{code}\"] = <\n                text = <\"x\">\n                description = <\"x\">\n            >\n{TEMPLATE_TERM}"
    )
}

#[test]
fn an_included_archetype_may_reuse_the_templates_node_ids() {
    assert_eq!(errors(OPT), Vec::new());
    let integrity =
        validate_source_integrity(OPT, Dialect::Adl2, Some(&ArchetypeRepository::new()))
            .expect("the fixture must parse");
    assert!(
        integrity.iter().all(|i| i.severity != Severity::Error),
        "{integrity:?}"
    );
}

#[test]
fn a_node_id_repeated_inside_the_included_archetype_is_refused() {
    let src = edit(OPT, "ITEM_TREE[id4]", "ITEM_TREE[id2]");
    let raised = errors(&src);
    let at = "/content[id3]/data[id2]/events[id3]/data[id2]";
    assert!(
        raised.contains(&(ValidationCode::Vcosu, Some(at.to_owned()))),
        "{raised:?}"
    );
}

#[test]
fn a_node_id_repeated_inside_the_template_is_refused() {
    let context =
        "context matches {\n            EVENT_CONTEXT[id2]\n        }\n        content cardinality";
    let src = edit(OPT, "content cardinality", context);
    let raised = errors(&src);
    assert!(
        raised.contains(&(ValidationCode::Vcosu, Some("/context[id2]".to_owned()))),
        "{raised:?}"
    );
}

#[test]
fn a_value_code_below_the_root_is_judged_against_the_constituent_terminology() {
    let coded = "DV_CODED_TEXT[id6] matches {\n    defining_code matches {[at10]}\n}";
    let used = edit(OPT, "DV_TEXT[id6]", coded);
    // Defined in the template's own terminology only: undefined for the constituent.
    let in_template = edit(&used, TEMPLATE_TERM, &template_term("at10"));
    let raised = errors(&in_template);
    assert!(
        raised.contains(&(ValidationCode::Vatdf, Some("/content[id3]".to_owned()))),
        "{raised:?}"
    );
    // Defined in the constituent's terminology: accepted.
    let term = "                [\"at10\"] = <\n                    text = <\"x\">\n                    description = <\"x\">\n                >\n";
    let in_constituent = edit(
        &used,
        CONSTITUENT_TERM,
        &format!("{term}{CONSTITUENT_TERM}"),
    );
    assert_eq!(errors(&in_constituent), Vec::new());
}

#[test]
fn an_interior_node_id_below_the_root_is_judged_against_the_constituent_terminology() {
    // `id5`, an ELEMENT of the multiply-valued `items`, defined in the
    // template's terminology and not in the constituent's.
    let renamed = edit(OPT, CONSTITUENT_TERM, "                [\"id9\"] = <\n");
    let src = edit(&renamed, TEMPLATE_TERM, &template_term("id5"));
    let raised = errors(&src);
    assert!(
        raised
            .iter()
            .any(|(code, path)| *code == ValidationCode::Vatid
                && path.as_deref().is_some_and(|p| p.ends_with("/items[id5]"))),
        "{raised:?}"
    );
}

/// A parent OBSERVATION for the flat-form case, with `id2` on its `data`.
const PARENT: &str = "\
archetype (adl_version=2.0.6; rm_release=1.0.2)
    openEHR-EHR-OBSERVATION.scope_parent.v1.0.0

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <\"unmanaged\">
    original_author = <
        [\"name\"] = <\"x\">
    >

definition
    OBSERVATION[id1] matches {
        data matches {
            HISTORY[id2]
        }
    }

terminology
    term_definitions = <
        [\"en\"] = <
            [\"id1\"] = <
                text = <\"x\">
                description = <\"x\">
            >
        >
    >
";

/// A specialisation of [`PARENT`] whose `protocol` includes an ITEM_TREE
/// archetype holding an ELEMENT with the node id `item`.
fn child(item: &str) -> String {
    format!(
        "\
archetype (adl_version=2.0.6; rm_release=1.0.2)
    openEHR-EHR-OBSERVATION.scope_parent-child.v1.0.0

specialize
    openEHR-EHR-OBSERVATION.scope_parent.v1

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <\"unmanaged\">
    original_author = <
        [\"name\"] = <\"x\">
    >

definition
    OBSERVATION[id1.1] matches {{
        protocol matches {{
            ITEM_TREE[id0.1, openEHR-EHR-ITEM_TREE.scope_inner.v1.0.0] matches {{
                items matches {{
                    ELEMENT[{item}]
                }}
            }}
        }}
    }}

terminology
    term_definitions = <
        [\"en\"] = <
            [\"id1.1\"] = <
                text = <\"x\">
                description = <\"x\">
            >
            [\"id0.1\"] = <
                text = <\"x\">
                description = <\"x\">
            >
        >
    >
"
    )
}

/// The flat-form VCOSU walk of a specialised archetype restarts at an
/// archetype root too: the included ITEM_TREE's `id2` is not the parent's.
#[test]
fn the_flat_form_walk_restarts_at_an_archetype_root() {
    let mut repo = ArchetypeRepository::new();
    repo.insert(parse_artefact(PARENT, Dialect::Adl2).expect("the parent must parse"));
    let vcosu = |src: &str| {
        validate_source(src, Some(&repo), &ProductionRmModel, &NoTerminologyResolver)
            .expect("the fixture must parse")
            .into_iter()
            .filter(|i| i.code == ValidationCode::Vcosu)
            .count()
    };
    assert_eq!(vcosu(&child("id2")), 0);
}
