// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The two query options the Definition API's example resources take.
//!
//! Both example operations — ADL 1.4
//! (`docs/specs/openehr/ITS-REST/specifications/operations/definition_template_adl1.4_example_get.yaml`)
//! and ADL 2 (`…/definition_template_adl2_example_get.yaml`) — declare
//! `detail_level` ∈ {required, medium, complete} (default `required`) and
//! `type` ∈ {input, output} (default `input`), described in
//! `…/parameters/query/example_detail_level.yaml` and
//! `…/parameters/query/example_type.yaml`.
//!
//! Component-free: the query string a pane sends is a pure, unit-tested
//! function of the controls the operator picked, so both template families
//! ask the CDR the same way.

use crate::format::ReprFormat;
use crate::locale::Msg;
use serde::{Deserialize, Serialize};

/// The canonical JSON and XML representations, the forms every example
/// comes in.
pub const CANONICAL_EXAMPLE_FORMATS: &[(&str, ReprFormat)] = &[
    ("json", ReprFormat::CanonicalJson),
    ("xml", ReprFormat::CanonicalXml),
];

/// The four representations of an example COMPOSITION: canonical JSON and XML
/// and the two Simplified Formats.
const COMPOSITION_EXAMPLE_FORMATS: &[(&str, ReprFormat)] = &[
    ("json", ReprFormat::CanonicalJson),
    ("xml", ReprFormat::CanonicalXml),
    ("flat", ReprFormat::Flat),
    ("structured", ReprFormat::Structured),
];

/// The representations the CDR serves the ADL 1.4 example of a template rooted
/// at `root` in, each with its `format` query key; empty when it builds none.
///
/// The example operation answers a COMPOSITION or one of the five parties
/// (`docs/specs/openehr/ITS-REST/specifications/responses/200_Template_example_retrieved.yaml`).
/// The Simplified Formats RM mapping
/// (`docs/specs/openehr/ITS-REST/docs/simplified_formats/master05-rm_mapping.adoc`)
/// maps no demographic party class, so a party example comes in canonical JSON
/// and XML only, and a template rooted at an ENTRY or a SECTION has no example.
#[must_use]
pub fn example_formats(root: &str) -> &'static [(&'static str, ReprFormat)] {
    match root {
        "COMPOSITION" => COMPOSITION_EXAMPLE_FORMATS,
        "PERSON" | "ORGANISATION" | "GROUP" | "AGENT" | "ROLE" => CANONICAL_EXAMPLE_FORMATS,
        _ => &[],
    }
}

/// How much of the template a generated example fills in (`detail_level`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExampleDetail {
    /// Only the mandatory data points; expected to be committable as served.
    #[default]
    Required,
    /// A realistic set including some optional attributes and elements.
    Medium,
    /// Every possible data point; reference guidance rather than a
    /// committable document.
    Complete,
}

impl ExampleDetail {
    /// The `detail_level` value on the wire.
    #[must_use]
    pub fn as_query(self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::Medium => "medium",
            Self::Complete => "complete",
        }
    }

    /// Short human label for the selector.
    #[must_use]
    pub fn label(self) -> Msg {
        match self {
            Self::Required => Msg::Required,
            Self::Medium => Msg::Medium,
            Self::Complete => Msg::Complete,
        }
    }
}

/// Which use the generated example is shaped for (`type`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExampleType {
    /// The document as submitted to the repository.
    #[default]
    Input,
    /// The document as it comes back out of the repository.
    Output,
}

impl ExampleType {
    /// The `type` value on the wire.
    #[must_use]
    pub fn as_query(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Output => "output",
        }
    }

    /// Short human label for the selector.
    #[must_use]
    pub fn label(self) -> Msg {
        match self {
            Self::Input => Msg::Input,
            Self::Output => Msg::Output,
        }
    }
}

/// The query string an example request carries, `?` included.
///
/// Both parameters are always spelled, defaults included, so the request the
/// viewer sends states exactly what the pane's controls show. Every value is
/// a fixed token of the two enums above, so nothing here is user input to
/// encode.
#[must_use]
pub fn example_query(detail: ExampleDetail, kind: ExampleType) -> String {
    format!(
        "?detail_level={}&type={}",
        detail.as_query(),
        kind.as_query()
    )
}

#[cfg(test)]
mod tests {
    use crate::example_options::{
        CANONICAL_EXAMPLE_FORMATS, ExampleDetail, ExampleType, example_formats, example_query,
    };
    use crate::format::ReprFormat;
    use crate::locale::tr;

    #[test]
    fn a_composition_template_offers_all_four_example_formats() {
        let keys: Vec<&str> = example_formats("COMPOSITION")
            .iter()
            .map(|(key, _)| *key)
            .collect();
        assert_eq!(keys, ["json", "xml", "flat", "structured"]);
    }

    #[test]
    fn a_party_template_offers_only_the_canonical_formats() {
        for root in ["PERSON", "ORGANISATION", "GROUP", "AGENT", "ROLE"] {
            assert_eq!(example_formats(root), CANONICAL_EXAMPLE_FORMATS, "{root}");
            assert!(
                example_formats(root)
                    .iter()
                    .all(|(_, f)| !matches!(f, ReprFormat::Flat | ReprFormat::Structured)),
                "{root} offers no Simplified Format"
            );
        }
    }

    #[test]
    fn an_entry_or_section_template_offers_no_example() {
        for root in ["OBSERVATION", "SECTION", "CLUSTER"] {
            assert!(example_formats(root).is_empty(), "{root}");
        }
    }

    #[test]
    fn every_option_spells_the_token_its_spec_enum_names() {
        assert_eq!(ExampleDetail::Required.as_query(), "required");
        assert_eq!(ExampleDetail::Medium.as_query(), "medium");
        assert_eq!(ExampleDetail::Complete.as_query(), "complete");
        assert_eq!(ExampleType::Input.as_query(), "input");
        assert_eq!(ExampleType::Output.as_query(), "output");
    }

    #[test]
    fn the_defaults_are_the_ones_the_operations_declare() {
        assert_eq!(ExampleDetail::default(), ExampleDetail::Required);
        assert_eq!(ExampleType::default(), ExampleType::Input);
    }

    #[test]
    fn the_query_always_names_both_parameters() {
        assert_eq!(
            example_query(ExampleDetail::default(), ExampleType::default()),
            "?detail_level=required&type=input"
        );
        assert_eq!(
            example_query(ExampleDetail::Complete, ExampleType::Output),
            "?detail_level=complete&type=output"
        );
        assert_eq!(
            example_query(ExampleDetail::Medium, ExampleType::Input),
            "?detail_level=medium&type=input"
        );
    }

    #[test]
    fn every_option_carries_a_selector_label() {
        for (option, label) in [
            (ExampleDetail::Required, "Required"),
            (ExampleDetail::Medium, "Medium"),
            (ExampleDetail::Complete, "Complete"),
        ] {
            assert_eq!(tr(option.label()), label);
        }
        assert_eq!(tr(ExampleType::Input.label()), "Input");
        assert_eq!(tr(ExampleType::Output.label()), "Output");
    }
}
