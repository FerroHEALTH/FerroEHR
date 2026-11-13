// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The bounds a jurisdiction sets on how long an access-log record is kept,
//! which `[audit.store] retention` is checked against at boot.

use crate::system_log::config::Retention;
use crate::system_log::message::AuditContext;
use crate::system_log::store::{AuditRecord, RecordEnd};

/// A bound a jurisdiction sets on how long an access-log record is kept,
/// written in calendar years as the law writes it, with the provision it rests
/// on.
///
/// A horizon in years (`retention = "P3Y"`) is compared against [`Self::years`]
/// exactly. A horizon in days is compared conservatively against [`Self::days`]:
/// for a floor, the most that many calendar years can span; for a ceiling, the
/// fewest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionBound {
    /// The bound in calendar years.
    pub years: u32,
    /// The bound in days a day-counted horizon is held to.
    pub days: u32,
    /// The provision, as the boot refusal names it.
    pub source: &'static str,
}

impl RetentionBound {
    /// Whether `retention` keeps records at least this long.
    #[must_use]
    pub fn admits_as_floor(self, retention: Retention) -> bool {
        match retention {
            Retention::Forever => true,
            Retention::Days(days) => days >= self.days,
            Retention::Years(years) => years >= self.years,
        }
    }

    /// Whether `retention` keeps records no longer than this.
    #[must_use]
    pub fn admits_as_ceiling(self, retention: Retention) -> bool {
        match retention {
            Retention::Forever => false,
            Retention::Days(days) => days <= self.days,
            Retention::Years(years) => years <= self.years,
        }
    }
}

/// The Member States of the European Union, as ISO 3166-1 alpha-2 codes.
///
/// EHDS is a Union regulation, "binding in its entirety and directly applicable
/// in all Member States" (Regulation (EU) 2025/327 Art. 105,
/// `docs/law/eu/ehds/text.html`). The EEA states outside the Union (Iceland,
/// Liechtenstein, Norway) are deliberately absent: the act is marked "Text with
/// EEA relevance", and it reaches them only through an EEA Joint Committee
/// decision, which the vendored corpus does not carry.
pub const EU_MEMBER_STATES: [&str; 27] = [
    "AT", "BE", "BG", "CY", "CZ", "DE", "DK", "EE", "ES", "FI", "FR", "GR", "HR", "HU", "IE", "IT",
    "LT", "LU", "LV", "MT", "NL", "PL", "PT", "RO", "SE", "SI", "SK",
];

/// The access-log floor EHDS sets in every Member State.
///
/// Art. 9(2): the access information "shall be available for at least three
/// years from each date of access to the data" (`docs/law/eu/ehds/text.html`).
/// Art. 105 applies Art. 9 from 26 March 2029 to the priority categories of
/// Art. 14(1)(a) to (c) and from 26 March 2031 to (d) to (f); the floor is
/// enforced from now on, which only keeps a log longer. Three calendar years
/// can span 1096 days, so that is the floor a day-counted horizon is held to.
const EHDS_FLOOR: RetentionBound = RetentionBound {
    years: 3,
    days: 1096,
    source: "three years from each date of access, EHDS Art. 9(2), Regulation (EU) 2025/327, \
             applying under Art. 105 from 26 March 2029 to Art. 14(1)(a)-(c) data and from \
             26 March 2031 to (d)-(f)",
};

/// The minimum an access-log record must be kept in `jurisdiction`, where one
/// is registered (#3242, #3625).
///
/// Keyed by ISO 3166-1 alpha-2, the same key the identifier rules carry, so a
/// deployment's jurisdictions are the ones its `[privacy.identifier_scan]`
/// rules name. A jurisdiction with no registered floor returns `None`: an
/// unknown requirement is never guessed at. Every EU Member State
/// ([`EU_MEMBER_STATES`]) carries the EHDS floor, and a national floor wins
/// where it is longer.
///
/// `NL`: five years from the moment the entry is written, Besluit vaststelling
/// bewaartermijn logging (<https://wetten.overheid.nl/BWBR0042391>, Stcrt.
/// 2019, 38007) under Art. 5 of the Besluit elektronische gegevensverwerking
/// door zorgaanbieders (<https://wetten.overheid.nl/BWBR0040238>), which binds
/// the retention to NEN 7513. Five calendar years never exceed 1830 days, so
/// that is the floor a day-counted horizon is held to.
///
/// `CH`: at least one year, and kept apart from the processing system, for the
/// logs of a large-scale automated processing of sensitive personal data —
/// Datenschutzverordnung (DSV, SR 235.11) Art. 4 Abs. 5 as amended on
/// 1 December 2025 (`docs/law/ch/dpo/text-de.html`,
/// <https://www.fedlex.admin.ch/eli/cc/2022/568/de>). One calendar year never
/// exceeds 366 days. Switzerland is outside the Union, so the EHDS floor does
/// not reach it.
#[must_use]
pub fn retention_floor(jurisdiction: &str) -> Option<RetentionBound> {
    let national = match jurisdiction {
        "NL" => Some(RetentionBound {
            years: 5,
            days: 1830,
            source: "five years, Besluit vaststelling bewaartermijn logging, \
                     https://wetten.overheid.nl/BWBR0042391",
        }),
        "CH" => Some(RetentionBound {
            years: 1,
            days: 366,
            source: "one year, DSV Art. 4 Abs. 5, \
                     https://www.fedlex.admin.ch/eli/cc/2022/568/de",
        }),
        _ => None,
    };
    let union = EU_MEMBER_STATES
        .contains(&jurisdiction)
        .then_some(EHDS_FLOOR);
    match (national, union) {
        (Some(national), Some(union)) if union.years > national.years => Some(union),
        (Some(national), _) => Some(national),
        (None, union) => union,
    }
}

/// The maximum an access-log record may be kept in `jurisdiction`, where one
/// is registered (#3346).
///
/// The mirror of [`retention_floor`], keyed the same way, and empty for
/// every jurisdiction but one: a rule that caps a log is rare, and an unknown
/// cap is never guessed at.
///
/// `DE`: the access logs of a telematics-infrastructure application under
/// §§ 327 and 334 Abs. 1 SGB V are kept for the three-year limitation period of
/// § 195 BGB and then deleted "unverzüglich" — SGB V § 309 Abs. 1 and Abs. 3
/// (`docs/law/de/sgb-v/BJNR024820988.xml`,
/// <https://www.gesetze-im-internet.de/sgb_5/__309.html>). Three calendar years
/// are never fewer than 1095 days, so that is the ceiling a day-counted horizon
/// is held to. The provision binds "die Verantwortlichen nach § 307", so it
/// reaches a deployment only once that deployment declares itself one
/// (`[audit.store] sgb_v_309_controller`), which is why `declared` gates the row
/// rather than the jurisdiction alone.
#[must_use]
pub fn retention_ceiling(jurisdiction: &str, sgb_v_309_controller: bool) -> Option<RetentionBound> {
    match jurisdiction {
        "DE" if sgb_v_309_controller => Some(RetentionBound {
            years: 3,
            days: 1095,
            source: "the three-year limitation period of SGB V § 309 Abs. 1, after which Abs. 3 \
                     requires deletion unverzüglich, \
                     https://www.gesetze-im-internet.de/sgb_5/__309.html",
        }),
        _ => None,
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
    use super::*;

    /// Every EU Member State carries the EHDS floor of three years, a longer
    /// national floor wins, and a state outside the Union keeps its own.
    #[test]
    fn the_ehds_floor_reaches_every_member_state_and_a_longer_national_floor_wins() {
        for state in ["DE", "FI", "SE", "FR", "AT"] {
            let floor = retention_floor(state).expect(state);
            assert_eq!((floor.years, floor.days), (3, 1096), "{state}");
            assert!(floor.source.contains("EHDS Art. 9(2)"), "{state}");
        }
        let nl = retention_floor("NL").expect("NL");
        assert_eq!(
            (nl.years, nl.days),
            (5, 1830),
            "the five Dutch years outlast the three EHDS years"
        );
        let ch = retention_floor("CH").expect("CH");
        assert_eq!(
            (ch.years, ch.days),
            (1, 366),
            "Switzerland is not a Member State"
        );
        for outside in ["NO", "GB", "IS", "LI"] {
            assert_eq!(retention_floor(outside), None, "{outside}");
        }
        assert_eq!(EU_MEMBER_STATES.len(), 27);
    }

    /// A horizon in years is compared exactly against a bound in years; a
    /// horizon in days conservatively, so three years pass a three-year floor
    /// and a three-year ceiling while no day count passes both.
    #[test]
    fn a_horizon_in_years_meets_bounds_in_years_exactly() {
        let floor = retention_floor("DE").expect("the EHDS floor");
        let ceiling = retention_ceiling("DE", true).expect("the SGB V ceiling");
        let three = Retention::Years(3);
        assert!(floor.admits_as_floor(three) && ceiling.admits_as_ceiling(three));
        assert!(!floor.admits_as_floor(Retention::Years(2)));
        assert!(!ceiling.admits_as_ceiling(Retention::Years(4)));
        for days in 1090..1100 {
            let horizon = Retention::Days(days);
            assert!(
                !(floor.admits_as_floor(horizon) && ceiling.admits_as_ceiling(horizon)),
                "{days} days cannot satisfy a three-year floor and ceiling at once"
            );
        }
        assert!(floor.admits_as_floor(Retention::Forever));
        assert!(!ceiling.admits_as_ceiling(Retention::Forever));
        assert_eq!(retention_ceiling("DE", false), None);
    }
}
