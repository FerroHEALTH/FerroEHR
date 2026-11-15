// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access log retention an `ACCESS_POLICY` states
//! (`ACCESS_POLICY.log_retention`), as the audit trail keeps records under it.
//!
//! The draft writes each period as a `DV_DURATION`; FerroEHR keeps a record
//! for whole calendar years or whole days, so a period in any other form is
//! refused at commit (our own design: the bounds a law sets are written in
//! years or days, and `ehds-logging` counts no other unit).

use std::str::FromStr;

use openehr_rm::draft::access_control::access_control::access_log_retention::AccessLogRetention;
use openehr_rm::draft::access_control::access_control::access_retention_bound::AccessRetentionBound;
use openehr_rm::v1_2::data_types::quantity::date_time::dv_duration::DvDuration;

use crate::config::retention::RetentionPeriod;
use crate::system_log::config::Retention;
use crate::system_log::retention::{LogRetention, Provision, RetentionBound};

/// A period of the access log retention that is no whole number of years or
/// days.
#[derive(Debug, thiserror::Error)]
#[error(
    "log_retention names the period {value:?}: write a retention period as whole years or \
     whole days, such as P5Y or P1830D"
)]
pub struct PeriodForm {
    /// The period as written.
    pub value: String,
    /// Why it is no retention period.
    #[source]
    pub source: crate::config::retention::RetentionPeriodError,
}

/// The access log retention `declared` states, from the policy version `uid`;
/// with nothing declared, every record is kept without end under Art. 9(2).
///
/// # Errors
/// [`PeriodForm`] for a period that is no whole number of years or days.
pub fn log_retention(
    declared: Option<&AccessLogRetention>,
    uid: Option<&str>,
) -> Result<LogRetention, PeriodForm> {
    let Some(declared) = declared else {
        return Ok(LogRetention::default());
    };
    let bounds = |bounds: Option<&[AccessRetentionBound]>| {
        bounds
            .unwrap_or_default()
            .iter()
            .map(bound)
            .collect::<Result<Vec<_>, _>>()
    };
    Ok(LogRetention {
        in_union: declared.in_union,
        period: declared
            .period
            .as_ref()
            .map_or(Ok(Retention::Forever), period)?,
        floors: bounds(declared.floors.as_deref())?,
        ceilings: bounds(declared.ceilings.as_deref())?,
        policy: uid.map(str::to_owned),
    })
}

/// One floor or ceiling.
fn bound(bound: &AccessRetentionBound) -> Result<RetentionBound, PeriodForm> {
    let code = &bound.ground.defining_code;
    Ok(RetentionBound {
        period: period(&bound.period)?,
        ground: Provision {
            terminology: code.terminology_id.value.clone(),
            code: code.code_string.clone(),
            rubric: bound.ground.value.clone(),
        },
    })
}

/// `duration` as a period of whole years or days.
fn period(duration: &DvDuration) -> Result<Retention, PeriodForm> {
    match RetentionPeriod::from_str(&duration.value) {
        Ok(RetentionPeriod::Years(years)) => Ok(Retention::Years(years)),
        Ok(RetentionPeriod::Days(days)) => Ok(Retention::Days(days)),
        Err(source) => Err(PeriodForm {
            value: duration.value.clone(),
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use openehr_rm::draft::access_control::terminology::coded_text;

    use super::*;

    fn duration(value: &str) -> DvDuration {
        DvDuration {
            normal_status: None,
            normal_range: None,
            other_reference_ranges: None,
            magnitude_status: None,
            accuracy: None,
            accuracy_is_percent: None,
            value: value.to_owned(),
        }
    }

    fn declared(period: Option<&str>, floor: &str) -> AccessLogRetention {
        AccessLogRetention {
            name: openehr_rm::v1_2::data_types::text::dv_text::DvText::DvText(
                openehr_rm::v1_2::data_types::text::dv_text::DvTextData {
                    value: "Access log retention".to_owned(),
                    hyperlink: None,
                    formatting: None,
                    mappings: None,
                    language: None,
                    encoding: None,
                },
            ),
            archetype_node_id: "id18".to_owned(),
            uid: None,
            links: None,
            archetype_details: None,
            feeder_audit: None,
            in_union: true,
            period: period.map(duration),
            floors: Some(openehr_base::containers::NonEmptyVec::of(
                AccessRetentionBound {
                    period: duration(floor),
                    ground: coded_text(
                        "http://data.europa.eu/eli/example/2026/1/oj",
                        "art_5",
                        "Art. 5",
                    ),
                },
            )),
            ceilings: None,
        }
    }

    /// A declared retention reads with its periods, its provisions and the
    /// policy version; none declared keeps every record without end.
    #[test]
    fn a_declared_retention_reads_with_its_policy_version() {
        let read = log_retention(Some(&declared(Some("P5Y"), "P1826D")), Some("uid::cdr::2"))
            .expect("whole years and days read");
        assert_eq!(read.period, Retention::Years(5));
        assert_eq!(
            read.floors.first().map(|b| b.period),
            Some(Retention::Days(1826))
        );
        assert_eq!(
            read.floors
                .first()
                .map(|b| (b.ground.code.as_str(), b.ground.rubric.as_str())),
            Some(("art_5", "Art. 5"))
        );
        assert_eq!(read.policy.as_deref(), Some("uid::cdr::2"));
        let forever = log_retention(Some(&declared(None, "P5Y")), None).expect("reads");
        assert_eq!(forever.period, Retention::Forever);
        assert_eq!(
            log_retention(None, None).expect("none"),
            LogRetention::default()
        );
    }

    /// A period in months, weeks or a mix of units is refused by its value.
    #[test]
    fn a_period_of_other_units_is_refused() {
        for value in ["P18M", "P1Y6M", "P52W", "PT1H"] {
            let refused = log_retention(Some(&declared(Some(value), "P1Y")), None)
                .expect_err("only whole years or days");
            assert_eq!(refused.value, value);
        }
    }
}
