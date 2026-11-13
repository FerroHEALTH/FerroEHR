// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The retention an access-log entry states when it is written (#5128).
//!
//! Each entry carries its own period (`docs/plans/access-log.md`
//! §Retention), so any store applies it without the recording system's
//! configuration: `retain_until` is the last day the entry is kept, and
//! `grounds` names the provisions that set it. The period is the one
//! `ehds-logging`'s retention policy computes from the categories and origins
//! of the data (Regulation (EU) 2025/327 Annex II 3.4), over the floor of Art
//! 9(2) or the floor the deployment's law sets; an entry whose categories call
//! for different periods keeps the longest. The store replaces an entry by its
//! tombstone once `retain_until` has passed
//! ([`crate::access_log::store::AccessLogStore::tombstone_expired`]).
//!
//! The draft defines no terminology for the grounds, so they are coded in
//! [`GROUND_TERMINOLOGY`] with the codes `ehds-logging` writes on the EHDS
//! access record: our own design.

use ehds_logging::classify::Classification;
use ehds_logging::retention::{Retention, RetentionPolicy};
use openehr_base::containers::NonEmptyVec;
use openehr_base::v1_3::base_types::identification::terminology_id::TerminologyId;
use openehr_rm::v1_2::data_types::quantity::date_time::dv_date::DvDate;
use openehr_rm::v1_2::data_types::text::code_phrase::CodePhrase;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;

use openehr_rm::draft::access_log::access_log::access_retention::AccessRetention;

/// The terminology a retention ground is coded in.
///
/// Its codes are the ones `ehds-logging` writes as `ehds-retention-floor` and
/// `ehds-retention-ground` on the EHDS access record (`eu-2025-327-art-9-2`,
/// `national:<citation>`, `category:<system>|<code>`, `origin:<key>` and the
/// rest).
pub const GROUND_TERMINOLOGY: &str = "ehds-logging";

/// The retention of an access made at `recorded`, classified as `classified`,
/// from the origins keyed `origins`, under `policy`.
///
/// The period is [`RetentionPolicy::retain`]'s: the longest of the default,
/// each category's and each origin's. `retain_until` is the day before the
/// first date the entry may be removed, and is void for an entry kept without
/// end; the grounds are the floor's legal ground and what called for the
/// period, in that order.
#[must_use]
pub fn retention<'a>(
    policy: &RetentionPolicy,
    recorded: jiff::Timestamp,
    classified: &Classification,
    origins: impl IntoIterator<Item = &'a str>,
) -> AccessRetention {
    stated(&policy.retain(recorded, classified, origins))
}

/// The `ACCESS_RETENTION` `retention` states.
fn stated(retention: &Retention) -> AccessRetention {
    let retain_until = retention.ends().map(|ends| {
        // NOTE: `ends` is a day after the date of access, so it always has a day before it;
        // were it the first day of the calendar, keeping it one day longer is the safe side.
        let last = ends.yesterday().unwrap_or(ends);
        DvDate {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: None,
            magnitude_status: None,
            accuracy: None,
            value: last.to_string(),
        }
    });
    let mut grounds = NonEmptyVec::of(coded(retention.floor().code()));
    grounds.push(coded(retention.ground().code()));
    AccessRetention {
        retain_until,
        grounds,
    }
}

/// The ground `code` of [`GROUND_TERMINOLOGY`], with the code as its rubric.
fn coded(code: String) -> DvCodedText {
    DvCodedText {
        value: code.clone(),
        hyperlink: None,
        formatting: None,
        mappings: None,
        language: None,
        encoding: None,
        defining_code: CodePhrase {
            terminology_id: TerminologyId {
                value: GROUND_TERMINOLOGY.to_owned(),
            },
            code_string: code,
            preferred_term: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ehds_logging::retention::{Bounds, Period};

    use super::*;
    use crate::system_log::categories::{AccessedContent, CategoryMapConfig, ContentIds};

    /// An access on 7 November 2026 at noon, UTC.
    fn accessed() -> jiff::Timestamp {
        "2026-11-07T12:00:00Z".parse().expect("an instant")
    }

    /// The shipped map's classification of a composition of each template.
    fn classified(templates: &[&str]) -> Classification {
        let classifier = CategoryMapConfig::default()
            .compile()
            .expect("the shipped map compiles");
        let objects = templates
            .iter()
            .map(|template| ContentIds::of_kind("COMPOSITION", Some((*template).to_owned()), None))
            .collect();
        classifier
            .classify(&AccessedContent::Objects(objects), false)
            .classification
    }

    /// A Union policy of three years, five for patient summaries and ten for
    /// laboratory reports.
    fn policy() -> RetentionPolicy {
        let classifier = CategoryMapConfig::default()
            .compile()
            .expect("the shipped map compiles");
        let categories = BTreeMap::from([
            ("Patient-Summaries".to_owned(), Period::Years(5)),
            ("Laboratory-Reports".to_owned(), Period::Years(10)),
        ]);
        RetentionPolicy::declare_within(
            &Bounds::union(),
            Period::Years(3),
            &categories,
            &BTreeMap::new(),
            classifier.map(),
        )
        .expect("the policy is within the Union bounds")
    }

    fn codes(retention: &AccessRetention) -> Vec<(&str, &str)> {
        retention
            .grounds
            .iter()
            .map(|ground| {
                (
                    ground.defining_code.terminology_id.value.as_str(),
                    ground.defining_code.code_string.as_str(),
                )
            })
            .collect()
    }

    /// An entry whose categories call for different periods keeps the
    /// longest, names that category as its ground beside the Art 9(2) floor,
    /// and is kept to the last day of the period.
    #[test]
    fn an_entry_of_two_categories_keeps_the_longer_period() {
        let both = retention(
            &policy(),
            accessed(),
            &classified(&[
                "International Patient Summary",
                "Generic lab test result example simple",
            ]),
            [],
        );
        assert_eq!(
            both.retain_until.as_ref().map(|date| date.value.as_str()),
            Some("2036-11-07"),
            "ten years from the date of access, the day before the first date it may go"
        );
        let grounds = codes(&both);
        assert_eq!(
            grounds.first(),
            Some(&(GROUND_TERMINOLOGY, "eu-2025-327-art-9-2"))
        );
        let ground = grounds.get(1).map(|(_, code)| *code).unwrap_or_default();
        assert!(
            ground.starts_with("category:") && ground.ends_with("|Laboratory-Reports"),
            "{ground}"
        );

        let summary = retention(
            &policy(),
            accessed(),
            &classified(&["International Patient Summary"]),
            [],
        );
        assert_eq!(
            summary.retain_until.map(|date| date.value),
            Some("2031-11-07".to_owned())
        );
    }

    /// An access of no category the policy names keeps the default period.
    #[test]
    fn an_entry_of_no_named_category_keeps_the_default() {
        let other = retention(&policy(), accessed(), &classified(&["CCTA report"]), []);
        assert_eq!(
            other.retain_until.as_ref().map(|date| date.value.as_str()),
            Some("2029-11-07")
        );
        assert_eq!(
            codes(&other),
            [
                (GROUND_TERMINOLOGY, "eu-2025-327-art-9-2"),
                (GROUND_TERMINOLOGY, "default")
            ]
        );
    }

    /// A policy that keeps without end states no `retain_until`.
    #[test]
    fn an_indefinite_period_states_no_last_day() {
        let classifier = CategoryMapConfig::default()
            .compile()
            .expect("the shipped map compiles");
        let forever = RetentionPolicy::declare_within(
            &Bounds::union(),
            Period::Indefinite,
            &BTreeMap::new(),
            &BTreeMap::new(),
            classifier.map(),
        )
        .expect("an indefinite period is within the bounds");
        let kept = retention(&forever, accessed(), &classified(&["CCTA report"]), []);
        assert_eq!(kept.retain_until, None);
    }
}
