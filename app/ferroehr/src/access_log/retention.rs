// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The retention an access-log entry states when it is written (#5128,
//! #5660).
//!
//! Each entry carries its own period (`docs/specifications/access-log`,
//! semantics §Retention), so any store applies it without the recording system's
//! configuration: `retain_until` is the last day the entry is kept, `grounds`
//! names the provisions that set it, and `policy` the version of the
//! controller's access policy the period was taken from. The period is the
//! one `ehds-logging`'s retention policy computes within the bounds the
//! access policy declares ([`LogRetention`]); the grounds are Regulation (EU)
//! 2025/327 Art. 9(2) for a controller in the Union and each declared floor
//! and ceiling, coded as the controller codes them. The store replaces an
//! entry by its tombstone once `retain_until` has passed
//! ([`crate::access_log::store::AccessLogStore::tombstone_expired`]).

use ehds_logging::classify::Classification;
use ehds_logging::retention::RetentionPolicy;
use jiff::civil::Date;
use openehr_base::containers::NonEmptyVec;
use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
use openehr_rm::draft::access_control::terminology::coded_text;
use openehr_rm::v1_2::data_types::quantity::date_time::dv_date::DvDate;

use openehr_rm::draft::access_log::access_log::access_retention::AccessRetention;

use crate::system_log::retention::LogRetention;

/// The first ground of an entry.
#[expect(
    clippy::expect_used,
    reason = "a retention names Art. 9(2) in the Union and at least one floor outside it, which \
              the access policy's Floor_outside_union invariant holds at commit"
)]
fn first_ground(
    ground: Option<openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText>,
) -> openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText {
    ground.expect("a retention should name at least one ground")
}

/// The `OBJECT_VERSION_ID` of the access policy version `uid`.
#[expect(
    clippy::expect_used,
    reason = "the uid is composed by versioning::object_version_id when the policy is installed"
)]
fn policy_version(uid: &str) -> ObjectVersionId {
    ObjectVersionId::new(uid).expect("an installed policy's uid should be an OBJECT_VERSION_ID")
}

/// The retention of an access made at `recorded`, classified as `classified`,
/// from the origins keyed `origins`, under `policy` and the access policy's
/// `log`.
///
/// The period is [`RetentionPolicy::retain`]'s: the longest of the default,
/// each category's and each origin's. `retain_until` is the day before the
/// first date the entry may be removed, and is void for an entry kept without
/// end.
#[must_use]
pub fn retention<'a>(
    policy: &RetentionPolicy,
    log: &LogRetention,
    recorded: jiff::Timestamp,
    classified: &Classification,
    origins: impl IntoIterator<Item = &'a str>,
) -> AccessRetention {
    stated(policy.retain(recorded, classified, origins).ends(), log)
}

/// The `ACCESS_RETENTION` of an entry kept until the day before `ends`, the
/// first date it may be removed on, or without end when `ends` is `None`,
/// under the access policy's `log`.
#[must_use]
pub fn stated(ends: Option<Date>, log: &LogRetention) -> AccessRetention {
    let retain_until = ends.map(|ends| {
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
    let mut grounds = log
        .grounds()
        .into_iter()
        .map(|ground| coded_text(&ground.terminology, &ground.code, &ground.rubric));
    let mut all = NonEmptyVec::of(first_ground(grounds.next()));
    for ground in grounds {
        all.push(ground);
    }
    AccessRetention {
        retain_until,
        grounds: all,
        policy: log.policy.as_deref().map(policy_version),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ehds_logging::retention::{Bounds, Period};

    use super::*;
    use crate::system_log::categories::{AccessedContent, CategoryMapConfig, ContentIds};
    use crate::system_log::config::Retention;
    use crate::system_log::retention::{EHDS_ELI, EHDS_FLOOR_CODE, Provision, RetentionBound};

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

    /// The access policy version the tests' retention comes from.
    const POLICY: &str = "5e6f7a8b-9c0d-4e1f-8a2b-c3d4e5f6a7b8::cdr.example.org::2";

    /// A provision of a synthetic act.
    fn provision(code: &str) -> Provision {
        Provision {
            terminology: "http://data.europa.eu/eli/example/2026/1/oj".to_owned(),
            code: code.to_owned(),
            rubric: code.to_owned(),
        }
    }

    /// The access policy's retention of a controller in the Union with no
    /// bound of its own.
    fn union() -> LogRetention {
        LogRetention {
            in_union: true,
            period: Retention::Years(3),
            floors: Vec::new(),
            ceilings: Vec::new(),
            policy: Some(POLICY.to_owned()),
        }
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
    /// longest, to the last day of the period, and names Art. 9(2) and the
    /// access policy version as its grounds.
    #[test]
    fn an_entry_of_two_categories_keeps_the_longer_period() {
        let both = retention(
            &policy(),
            &union(),
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
        assert_eq!(codes(&both), [(EHDS_ELI, EHDS_FLOOR_CODE)]);
        assert_eq!(
            both.policy.as_ref().map(ObjectVersionId::value),
            Some(POLICY)
        );

        let summary = retention(
            &policy(),
            &union(),
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
        let other = retention(
            &policy(),
            &union(),
            accessed(),
            &classified(&["CCTA report"]),
            [],
        );
        assert_eq!(
            other.retain_until.as_ref().map(|date| date.value.as_str()),
            Some("2029-11-07")
        );
        assert_eq!(codes(&other), [(EHDS_ELI, EHDS_FLOOR_CODE)]);
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
        let kept = retention(
            &forever,
            &union(),
            accessed(),
            &classified(&["CCTA report"]),
            [],
        );
        assert_eq!(kept.retain_until, None);
    }

    /// The declared floors and ceilings follow Art. 9(2) in declaration
    /// order, outside the Union only the declared bounds are named, and an
    /// entry kept under no access policy names no policy.
    #[test]
    fn the_grounds_are_the_provisions_the_policy_declares() {
        let mut log = union();
        log.period = Retention::Years(5);
        log.floors = vec![RetentionBound {
            period: Retention::Years(5),
            ground: provision("art_5"),
        }];
        log.ceilings = vec![RetentionBound {
            period: Retention::Years(10),
            ground: provision("art_10"),
        }];
        let terminology = "http://data.europa.eu/eli/example/2026/1/oj";
        assert_eq!(
            codes(&stated(None, &log)),
            [
                (EHDS_ELI, EHDS_FLOOR_CODE),
                (terminology, "art_5"),
                (terminology, "art_10")
            ]
        );
        log.in_union = false;
        assert_eq!(
            codes(&stated(None, &log)),
            [(terminology, "art_5"), (terminology, "art_10")]
        );
        let undeclared = stated(None, &LogRetention::default());
        assert_eq!(codes(&undeclared), [(EHDS_ELI, EHDS_FLOOR_CODE)]);
        assert_eq!(undeclared.policy, None);
    }
}
