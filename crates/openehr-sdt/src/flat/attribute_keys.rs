// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The FLAT key an RM attribute of a structural class is written under when
//! the Web Template carries no node for it.
//!
//! The rows come from the per-class tables of ITS-REST
//! `simplified_formats/master05-rm_mapping.adoc` (Flat Path against RM Path),
//! plus the two keys the codecs write that no table lists, each marked with
//! its [`KeySource`]. A consumer that writes or reads one of these attributes
//! asks [`attribute_key`] instead of keeping its own copy of the table.

use KeyForm::{IndexedStep, Step, Suffix};
use KeySource::{Example, OwnDesign, Table};

/// Where a row of [`ATTRIBUTE_KEYS`] comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    /// A row of the class's table in `master05-rm_mapping.adoc`.
    Table,
    /// A key the `master05-rm_mapping.adoc` examples spell and no table lists.
    Example,
    /// A key no Simplified Formats text spells: our own design.
    OwnDesign,
}

/// How the key joins the node it is written under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyForm {
    /// A path step, `<node>/<key>`.
    Step,
    /// A repeating path step, `<node>/<key>:<i>`.
    IndexedStep,
    /// An attribute suffix, `<node>|<key>`.
    Suffix,
}

/// One RM attribute and the FLAT key it is written under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttributeKey {
    /// The RM class that declares the attribute (a subtype inherits the row).
    pub rm_class: &'static str,
    /// The attribute's RM path from an instance of the class, `/`-separated.
    pub rm_path: &'static str,
    /// The key, without the `/`, `|` or `:i` its form adds.
    pub key: &'static str,
    /// How the key joins its node.
    pub form: KeyForm,
    /// Where the row comes from.
    pub source: KeySource,
}

/// One row, in the order the table reads.
const fn row(
    rm_class: &'static str,
    rm_path: &'static str,
    key: &'static str,
    form: KeyForm,
    source: KeySource,
) -> AttributeKey {
    AttributeKey {
        rm_class,
        rm_path,
        key,
        form,
        source,
    }
}

/// Every RM attribute of a structural class the codecs write under a key of
/// its own.
///
/// `master05-rm_mapping.adoc` writes `history.origin` for `OBSERVATION.data`'s
/// `origin` and `uid.value` for `LOCATABLE.uid`'s value; the rows spell the RM
/// attribute names. `ENTRY.provider` as `_provider` is the codecs' own key.
pub const ATTRIBUTE_KEYS: &[AttributeKey] = &[
    row("LOCATABLE", "links", "_link", IndexedStep, Table),
    row("LOCATABLE", "feeder_audit", "_feeder_audit", Step, Table),
    row("LOCATABLE", "uid/value", "_uid", Step, Table),
    row("COMPOSITION", "language", "language", Step, Table),
    row("COMPOSITION", "territory", "territory", Step, Table),
    row("COMPOSITION", "category", "category", Step, Table),
    row("COMPOSITION", "composer", "composer", Step, Table),
    row("COMPOSITION", "context", "context", Step, Table),
    row("ENTRY", "language", "language", Step, Table),
    row("ENTRY", "territory", "territory", Step, Table),
    row("ENTRY", "subject", "subject", Step, Table),
    row("ENTRY", "workflow_id", "_work_flow_id", Step, Table),
    row("ENTRY", "provider", "_provider", Step, OwnDesign),
    row(
        "ENTRY",
        "other_participations",
        "_other_participation",
        IndexedStep,
        Example,
    ),
    row("CARE_ENTRY", "guideline_id", "_guideline_id", Step, Table),
    row("INSTRUCTION", "narrative", "narrative", Step, Table),
    row("INSTRUCTION", "expiry_time", "_expiry_time", Step, Table),
    row(
        "INSTRUCTION",
        "wf_definition",
        "_wf_definition",
        Step,
        Table,
    ),
    row("ACTION", "time", "time", Step, Table),
    row("ACTION", "ism_transition", "ism_transition", Step, Table),
    row(
        "ACTION",
        "instruction_details",
        "_instruction_details",
        Step,
        Table,
    ),
    row("OBSERVATION", "data/origin", "history_origin", Step, Table),
    row("ELEMENT", "null_flavour", "_null_flavour", Step, Table),
    row("ELEMENT", "null_reason", "_null_reason", Step, Table),
    row("ACTIVITY", "timing", "timing", Step, Table),
    row(
        "ACTIVITY",
        "action_archetype_id",
        "action_archetype_id",
        Step,
        Table,
    ),
    row(
        "ISM_TRANSITION",
        "current_state",
        "current_state",
        Step,
        Table,
    ),
    row("ISM_TRANSITION", "transition", "transition", Step, Table),
    row(
        "ISM_TRANSITION",
        "careflow_step",
        "careflow_step",
        Step,
        Table,
    ),
    row("ISM_TRANSITION", "reason", "_reason", IndexedStep, Table),
    row(
        "INSTRUCTION_DETAILS",
        "instruction_id/path",
        "path",
        Suffix,
        Table,
    ),
    row(
        "INSTRUCTION_DETAILS",
        "instruction_id/id",
        "composition_uid",
        Suffix,
        Table,
    ),
    row(
        "INSTRUCTION_DETAILS",
        "activity_id",
        "activity_id",
        Suffix,
        Table,
    ),
    row("EVENT_CONTEXT", "start_time", "start_time", Step, Table),
    row("EVENT_CONTEXT", "setting", "setting", Step, Table),
    row("EVENT_CONTEXT", "end_time", "_end_time", Step, Table),
    row("EVENT_CONTEXT", "location", "_location", Step, Table),
    row(
        "EVENT_CONTEXT",
        "health_care_facility",
        "_health_care_facility",
        Step,
        Table,
    ),
    row(
        "EVENT_CONTEXT",
        "participations",
        "_participation",
        IndexedStep,
        Table,
    ),
    row("EVENT", "time", "time", Step, Table),
    row("INTERVAL_EVENT", "width", "width", Step, Table),
    row(
        "INTERVAL_EVENT",
        "math_function",
        "math_function",
        Step,
        Table,
    ),
    row(
        "INTERVAL_EVENT",
        "sample_count",
        "sample_count",
        Suffix,
        Table,
    ),
];

/// Returns the FLAT key the attribute at `rm_path` of an `rm_class` instance
/// is written under, `None` when no row names it.
///
/// A row of an ancestor class applies to its subtypes (RM class conformance,
/// through the generated model), so `OBSERVATION` reads the `ENTRY` rows.
#[must_use]
pub fn attribute_key(rm_class: &str, rm_path: &str) -> Option<&'static AttributeKey> {
    ATTRIBUTE_KEYS
        .iter()
        .find(|row| row.rm_path == rm_path && openehr_rm::v1_2::model::is_a(rm_class, row.rm_class))
}

/// Returns the attribute an `rm_class` instance writes under `key` in `form`,
/// the inverse of [`attribute_key`].
#[must_use]
pub fn attribute_of_key(rm_class: &str, key: &str, form: KeyForm) -> Option<&'static AttributeKey> {
    ATTRIBUTE_KEYS.iter().find(|row| {
        row.key == key && row.form == form && openehr_rm::v1_2::model::is_a(rm_class, row.rm_class)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_subtype_reads_its_ancestors_rows() {
        let origin = attribute_key("OBSERVATION", "data/origin").expect("history_origin");
        assert_eq!((origin.key, origin.form), ("history_origin", Step));
        assert_eq!(
            attribute_key("OBSERVATION", "provider").map(|r| r.key),
            Some("_provider")
        );
        assert_eq!(
            attribute_key("EVALUATION", "guideline_id").map(|r| r.key),
            Some("_guideline_id")
        );
        assert_eq!(attribute_key("ADMIN_ENTRY", "guideline_id"), None);
        assert_eq!(
            attribute_key("POINT_EVENT", "time").map(|r| r.key),
            Some("time")
        );
        assert_eq!(attribute_key("POINT_EVENT", "width"), None);
        assert_eq!(
            attribute_key("CLUSTER", "links").map(|r| r.form),
            Some(IndexedStep)
        );
        assert_eq!(
            attribute_of_key("ACTION", "_instruction_details", Step).map(|r| r.rm_path),
            Some("instruction_details")
        );
    }

    #[test]
    fn every_row_names_a_declared_class() {
        for row in ATTRIBUTE_KEYS {
            assert!(
                openehr_rm::v1_2::model::is_a(row.rm_class, "PATHABLE"),
                "{} is no RM class",
                row.rm_class
            );
        }
    }
}
