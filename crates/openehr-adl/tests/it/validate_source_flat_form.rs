// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `validate_source` reaches the flat-form checks VUNP and VACMCO
//! (`AOM2/master08-validation.adoc` §Phase 3) on top-level archetypes, whose
//! flat form is their differential (`ADL2/master09.02`).

#![allow(
    clippy::expect_used,
    reason = "integration-test fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_adl::validate::bindings::NoTerminologyResolver;
use openehr_adl::validate::catalogue::ValidationCode;
use openehr_adl::validate::rm::ProductionRmModel;
use openehr_adl::validate::validate_source;

/// The codes `validate_source` raises for `src`, with no repository.
fn codes(src: &str) -> Vec<ValidationCode> {
    validate_source(src, None, &ProductionRmModel, &NoTerminologyResolver)
        .expect("the fixture must parse")
        .into_iter()
        .map(|i| i.code)
        .collect()
}

/// An unresolvable `use_node` raises VUNP through `validate_source`, over real RM
/// classes so phase 2 lets the pipeline reach phase 3.
#[test]
fn an_unresolvable_use_node_path_raises_vunp() {
    let src = "\
archetype (adl_version=2.0.5; rm_release=1.0.2)
\topenEHR-EHR-CLUSTER.validate_source_vunp.v1.0.0

language
\toriginal_language = <[ISO_639-1::en]>

description
\tlifecycle_state = <\"draft\">

definition
\tCLUSTER[id1] matches {
\t\titems matches {
\t\t\tELEMENT[id2]
\t\t\tuse_node ELEMENT[id5] /items[id99]
\t\t}
\t}

terminology
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\t[\"id1\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id2\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id5\"] = <text=<\"\"> description=<\"\">>
\t\t>
\t>
";
    let raised = codes(src);
    assert!(
        raised.contains(&ValidationCode::Vunp),
        "expected VUNP from validate_source, got {raised:?}"
    );
}

/// A container whose mandatory children cannot fit its cardinality raises
/// VACMCO through `validate_source` (`master04.5` §Validity Rules:
/// `C_ATTRIBUTE`; scheduled by `master08` §Phase 3).
#[test]
fn a_cardinality_that_cannot_hold_its_mandatory_children_raises_vacmco() {
    // items cardinality {1..2} with three children of occurrences {1}: the
    // minimum of three cannot fit in two.
    let src = "\
archetype (adl_version=2.0.5; rm_release=1.0.2)
\topenEHR-EHR-CLUSTER.validate_source_vacmco.v1.0.0

language
\toriginal_language = <[ISO_639-1::en]>

description
\tlifecycle_state = <\"draft\">

definition
\tCLUSTER[id1] matches {
\t\titems cardinality matches {1..2} matches {
\t\t\tELEMENT[id2] occurrences matches {1}
\t\t\tELEMENT[id3] occurrences matches {1}
\t\t\tELEMENT[id4] occurrences matches {1}
\t\t}
\t}

terminology
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\t[\"id1\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id2\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id3\"] = <text=<\"\"> description=<\"\">>
\t\t\t[\"id4\"] = <text=<\"\"> description=<\"\">>
\t\t>
\t>
";
    let raised = codes(src);
    assert!(
        raised.contains(&ValidationCode::Vacmco),
        "expected VACMCO from validate_source, got {raised:?}"
    );
}
