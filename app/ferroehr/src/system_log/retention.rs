// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! How long the access log keeps a record, as the controller's access policy
//! states it, and the first date each record of a batch may be deleted on.
//!
//! The controller declares the period and the bounds its law sets
//! (`ACCESS_POLICY.log_retention`, [`crate::access_policy`]); FerroEHR carries
//! no table of national law. The one provision it names itself is Regulation
//! (EU) 2025/327 Art. 9(2), which binds a controller that declares itself in
//! the Union. A provision is coded as the controller codes it, and Art. 9(2)
//! as its European Legislation Identifier (Council conclusions 2012/C 325/02)
//! with the subdivision as the code: our own design.

use ehds_logging::retention::{Bounds, Period, RetentionPolicy};
use serde::{Deserialize, Serialize};

use crate::system_log::config::Retention;
use crate::system_log::message::AuditContext;
use crate::system_log::store::{AuditRecord, RecordEnd};

/// The European Legislation Identifier of Regulation (EU) 2025/327.
pub const EHDS_ELI: &str = "http://data.europa.eu/eli/reg/2025/327/oj";

/// The subdivision of [`EHDS_ELI`] that keeps the access information "for at
/// least three years from each date of access to the data": Art. 9(2).
pub const EHDS_FLOOR_CODE: &str = "art_9.par_2";

/// The rubric of [`EHDS_FLOOR_CODE`].
const EHDS_FLOOR_RUBRIC: &str =
    "Regulation (EU) 2025/327 Art. 9(2): at least three years from each date of access";

/// A legal provision, coded in the terminology the jurisdiction names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provision {
    /// The terminology id, such as the act's European Legislation Identifier.
    pub terminology: String,
    /// The code, such as the subdivision of the act.
    pub code: String,
    /// The rubric.
    pub rubric: String,
}

impl Provision {
    /// The provision as `ehds-logging` cites it on the EHDS record:
    /// `<terminology>#<code>`.
    fn citation(&self) -> String {
        format!("{}#{}", self.terminology, self.code)
    }
}

/// A floor or ceiling on how long a record is kept, with its provision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetentionBound {
    /// The bound from the date of access.
    pub period: Retention,
    /// The provision that sets it.
    pub ground: Provision,
}

/// How long the access log keeps a record, from the access policy in force.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogRetention {
    /// Whether the controller declares Regulation (EU) 2025/327 binds it.
    pub in_union: bool,
    /// How long each record is kept from the date of access.
    pub period: Retention,
    /// The floors the controller's law sets.
    pub floors: Vec<RetentionBound>,
    /// The ceilings the controller's law sets.
    pub ceilings: Vec<RetentionBound>,
    /// The `OBJECT_VERSION_ID` of the access policy the period comes from;
    /// `None` with no `log_retention` in force.
    pub policy: Option<String>,
}

impl Default for LogRetention {
    /// No period declared: every record kept without end, which meets every
    /// floor, under Art. 9(2).
    fn default() -> Self {
        Self {
            in_union: true,
            period: Retention::Forever,
            floors: Vec::new(),
            ceilings: Vec::new(),
            policy: None,
        }
    }
}

/// Why a declared retention is no policy `ehds-logging` keeps records under.
#[derive(Debug, thiserror::Error)]
pub enum RetentionDefect {
    /// A controller outside the Union declares no floor, so no record could
    /// name the ground of its period.
    #[error(
        "a controller outside the Union declares the floor its own law sets in \
         log_retention.floors"
    )]
    NoFloorOutsideUnion,
    /// `ehds-logging` refuses the period within the declared bounds.
    #[error("the log retention period breaks a bound")]
    Bounds(#[source] ehds_logging::retention::RetentionError),
}

impl LogRetention {
    /// Whether the controller declared the period, rather than leaving every
    /// record kept without end by default (GDPR Art. 5(1)(e) makes it theirs).
    #[must_use]
    pub fn is_declared(&self) -> bool {
        self.policy.is_some()
    }

    /// The provisions the period rests on, in the order a record names them:
    /// Art. 9(2) in the Union, then each floor and each ceiling.
    #[must_use]
    pub fn grounds(&self) -> Vec<Provision> {
        let union = self.in_union.then(|| Provision {
            terminology: EHDS_ELI.to_owned(),
            code: EHDS_FLOOR_CODE.to_owned(),
            rubric: EHDS_FLOOR_RUBRIC.to_owned(),
        });
        union
            .into_iter()
            .chain(
                self.floors
                    .iter()
                    .chain(&self.ceilings)
                    .map(|bound| bound.ground.clone()),
            )
            .collect()
    }

    /// The bounds `ehds-logging` declares the policy within: in the Union
    /// Art. 9(2), or the longest declared floor where it is longer; outside
    /// the Union the longest declared floor; and the shortest ceiling.
    ///
    /// # Errors
    /// [`RetentionDefect`] for a controller outside the Union with no floor,
    /// and for a bound `ehds-logging` refuses.
    pub fn bounds(&self) -> Result<Bounds, RetentionDefect> {
        let longest = self
            .floors
            .iter()
            .max_by_key(|bound| most_days(bound.period));
        let bounds = match (self.in_union, longest) {
            (true, Some(floor)) if floor_reaches_union(floor.period) => {
                Bounds::national(period(floor.period), floor.ground.citation())
                    .map_err(RetentionDefect::Bounds)?
            }
            (true, _) => Bounds::union(),
            (false, Some(floor)) => {
                Bounds::outside_union(period(floor.period), floor.ground.citation())
                    .map_err(RetentionDefect::Bounds)?
            }
            (false, None) => return Err(RetentionDefect::NoFloorOutsideUnion),
        };
        match self
            .ceilings
            .iter()
            .min_by_key(|bound| fewest_days(bound.period))
        {
            Some(ceiling) => bounds
                .with_ceiling(period(ceiling.period), ceiling.ground.citation())
                .map_err(RetentionDefect::Bounds),
            None => Ok(bounds),
        }
    }

    /// The `ehds-logging` policy that keeps every record [`Self::period`]
    /// within [`Self::bounds`]; the access policy declares no period per
    /// category or origin, so no category map takes part.
    ///
    /// # Errors
    /// [`RetentionDefect`] when the bounds or the period are refused.
    pub fn policy(&self) -> Result<RetentionPolicy, RetentionDefect> {
        let none = std::collections::BTreeMap::new();
        RetentionPolicy::declare_within(
            &self.bounds()?,
            period(self.period),
            &none,
            &none,
            &ehds_logging::map::CategoryMap::default(),
        )
        .map_err(RetentionDefect::Bounds)
    }
}

/// `retention` as `ehds-logging` writes a period.
fn period(retention: Retention) -> Period {
    match retention {
        Retention::Forever => Period::Indefinite,
        Retention::Days(days) => Period::Days(days),
        Retention::Years(years) => Period::Years(u16::try_from(years).unwrap_or(u16::MAX)),
    }
}

/// The most days `retention` can span: a calendar year spans 366 at most.
fn most_days(retention: Retention) -> u64 {
    match retention {
        Retention::Forever => u64::MAX,
        Retention::Days(days) => u64::from(days),
        Retention::Years(years) => u64::from(years).saturating_mul(366),
    }
}

/// The fewest days `retention` can span: a calendar year spans 365 at least.
fn fewest_days(retention: Retention) -> u64 {
    match retention {
        Retention::Forever => u64::MAX,
        Retention::Days(days) => u64::from(days),
        Retention::Years(years) => u64::from(years).saturating_mul(365),
    }
}

/// Whether a national floor of `retention` keeps records at least as long as
/// Art. 9(2), so it replaces the Union floor as the ground of the floor.
fn floor_reaches_union(retention: Retention) -> bool {
    match retention {
        Retention::Forever => true,
        Retention::Years(years) => years >= u32::from(ehds_logging::retention::FLOOR_YEARS),
        // NOTE: three calendar years span 1096 days at most (Art. 9(2) "from each date of access").
        Retention::Days(days) => days >= 1096,
    }
}

/// The first date each record may be deleted on ([`crate::system_log::fhir::record_end`]).
#[cfg(feature = "fhir")]
pub(crate) fn record_ends(records: &[AuditRecord], ctx: &AuditContext) -> Vec<RecordEnd> {
    records
        .iter()
        .map(|record| crate::system_log::fhir::record_end(&record.event, ctx))
        .collect()
}

/// A slim build has no store sink ([`crate::system_log::sender::start`] refuses one), so no record has an end.
#[cfg(not(feature = "fhir"))]
pub(crate) fn record_ends(_records: &[AuditRecord], _ctx: &AuditContext) -> Vec<RecordEnd> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use ehds_logging::retention::FloorGround;

    use super::*;

    fn provision(code: &str) -> Provision {
        Provision {
            terminology: "http://data.europa.eu/eli/example/2026/1/oj".to_owned(),
            code: code.to_owned(),
            rubric: code.to_owned(),
        }
    }

    fn bound(period: Retention, code: &str) -> RetentionBound {
        RetentionBound {
            period,
            ground: provision(code),
        }
    }

    fn retention(in_union: bool, period: Retention) -> LogRetention {
        LogRetention {
            in_union,
            period,
            ..LogRetention::default()
        }
    }

    /// In the Union the floor is Art. 9(2) unless a declared floor keeps
    /// records longer, which then grounds the floor; outside the Union the
    /// longest declared floor does, under three years too.
    #[test]
    fn the_floor_is_art_9_2_or_the_longest_declared_floor() {
        let union = retention(true, Retention::Years(3));
        assert_eq!(
            union.bounds().expect("bounds").floor_ground(),
            &FloorGround::Union
        );
        let mut national = retention(true, Retention::Years(5));
        national.floors = vec![
            bound(Retention::Years(1), "short"),
            bound(Retention::Years(5), "long"),
        ];
        assert_eq!(
            national.bounds().expect("bounds").floor_ground(),
            &FloorGround::National("http://data.europa.eu/eli/example/2026/1/oj#long".to_owned())
        );
        let mut outside = retention(false, Retention::Years(1));
        outside.floors = vec![bound(Retention::Years(1), "art_4_5")];
        assert!(matches!(
            outside.bounds().expect("bounds").floor_ground(),
            FloorGround::OutsideUnion(_)
        ));
        outside.policy().expect("one year outside the Union holds");
        outside.floors.clear();
        assert!(matches!(
            outside.bounds(),
            Err(RetentionDefect::NoFloorOutsideUnion)
        ));
    }

    /// A period under Art. 9(2) in the Union, over a ceiling, or kept without
    /// end under a ceiling is refused, counting calendar years and days as
    /// `ehds-logging` does.
    #[test]
    fn a_period_that_breaks_a_bound_is_refused() {
        assert!(retention(true, Retention::Days(1095)).policy().is_err());
        retention(true, Retention::Days(1096))
            .policy()
            .expect("1096 days reach three years from every date of access");
        let mut capped = retention(true, Retention::Years(3));
        capped.ceilings = vec![bound(Retention::Years(3), "ceiling")];
        capped
            .policy()
            .expect("three years within a three-year ceiling");
        capped.period = Retention::Days(1096);
        assert!(
            capped.policy().is_err(),
            "1096 days can outlast three years"
        );
        capped.period = Retention::Forever;
        assert!(
            capped.policy().is_err(),
            "without end outlasts every ceiling"
        );
    }

    /// The grounds name Art. 9(2) in the Union, then each declared floor and
    /// ceiling; only a policy that states the retention is declared.
    #[test]
    fn the_grounds_follow_the_declaration() {
        let mut log = retention(true, Retention::Years(5));
        log.floors = vec![bound(Retention::Years(5), "floor")];
        log.ceilings = vec![bound(Retention::Years(10), "ceiling")];
        let codes: Vec<String> = log.grounds().into_iter().map(|g| g.code).collect();
        assert_eq!(codes, [EHDS_FLOOR_CODE, "floor", "ceiling"]);
        assert_eq!(
            log.grounds().first().map(|g| g.terminology.as_str()),
            Some(EHDS_ELI)
        );
        assert!(!LogRetention::default().is_declared());
        log.policy = Some("uid::cdr::1".to_owned());
        assert!(log.is_declared());
    }
}
