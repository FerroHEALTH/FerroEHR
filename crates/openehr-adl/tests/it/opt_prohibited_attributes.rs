// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The raw OPT names, beside itself, every attribute it removed for
//! `existence matches {0}` (#5154).
//!
//! AM `OPT2/master03-opt_raw.adoc` §Flattening removes such an attribute from
//! the raw OPT, and ADL2 `master04.3-cadl_complex_types.adoc` §Existence then
//! reads the missing attribute at its reference-model existence, so the list
//! is what keeps the prohibition (no specification governs it: our own
//! design).

#![expect(
    clippy::expect_used,
    reason = "integration-test fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_adl::artefact::ArchetypeRepository;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::opt::{RawOpt, create_opt};
use openehr_adl::parse::Dialect;

/// A COMPOSITION archetype with `constraints` on its root and an OBSERVATION
/// under `content` carrying `observation`.
fn raw_opt(constraints: &str, observation: &str) -> RawOpt {
    let source = format!(
        "archetype (adl_version=2.0.6; rm_release=1.0.2; generated)\n    \
         openEHR-EHR-COMPOSITION.prohibited_probe.v1.0.0\n\n\
         language\n    original_language = <[ISO_639-1::en]>\n\n\
         description\n    lifecycle_state = <\"unmanaged\">\n\n\
         definition\n    COMPOSITION[id1] matches {{\n        {constraints}\n        \
         content cardinality matches {{0..*; unordered}} matches {{\n            \
         OBSERVATION[id2] occurrences matches {{0..*}} matches {{\n                \
         {observation}\n            }}\n        }}\n    }}\n\n\
         terminology\n    term_definitions = <\n        [\"en\"] = <\n            \
         [\"id1\"] = <text = <\"Probe\">; description = <\"A probe\">>\n            \
         [\"id2\"] = <text = <\"Observation\">; description = <\"An observation\">>\n        \
         >\n    >\n"
    );
    let archetype = parse_artefact(&source, Dialect::Adl2).expect("the probe parses");
    create_opt(&archetype, &ArchetypeRepository::default()).expect("the OPT builds")
}

fn attribute_names(opt: &RawOpt) -> Vec<&str> {
    opt.template
        .definition
        .attributes()
        .unwrap_or_default()
        .iter()
        .map(|a| a.rm_attribute_name.as_str())
        .collect()
}

#[test]
fn a_removed_attribute_is_named_by_its_path_beside_the_opt() {
    let opt = raw_opt(
        "context existence matches {0}",
        "protocol existence matches {0}",
    );
    assert_eq!(
        opt.prohibited_attributes,
        ["/context", "/content[id2]/protocol"]
    );
    assert!(
        !attribute_names(&opt).contains(&"context"),
        "the raw OPT keeps its master03 shape: {:?}",
        attribute_names(&opt)
    );
}

#[test]
fn an_optional_or_open_attribute_is_not_named() {
    for constraints in ["context existence matches {0..1}", ""] {
        let opt = raw_opt(constraints, "protocol existence matches {0..1}");
        assert!(
            opt.prohibited_attributes.is_empty(),
            "{constraints:?}: {:?}",
            opt.prohibited_attributes
        );
    }
}
