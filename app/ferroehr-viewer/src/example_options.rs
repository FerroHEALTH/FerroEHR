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

use serde::{Deserialize, Serialize};

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
    pub fn label(self) -> &'static str {
        match self {
            Self::Required => "Required",
            Self::Medium => "Medium",
            Self::Complete => "Complete",
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
    pub fn label(self) -> &'static str {
        match self {
            Self::Input => "Input",
            Self::Output => "Output",
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
    use crate::example_options::{ExampleDetail, ExampleType, example_query};

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
            assert_eq!(option.label(), label);
        }
        assert_eq!(ExampleType::Input.label(), "Input");
        assert_eq!(ExampleType::Output.label(), "Output");
    }
}
