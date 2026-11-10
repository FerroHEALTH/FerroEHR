// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A retention period: how long a record is kept, as a calendar period.
//!
//! The floors and ceilings a retention answers to are written in years (EHDS
//! Art. 9(2), SGB V § 309), and a day count only approximates a year: three
//! calendar years span 1095 or 1096 days. So a period is an ISO 8601-1:2019
//! §5.5.2 duration restricted to one designator, years (`"P5Y"`) or days
//! (`"P1830D"`), and years are compared and reaped with calendar arithmetic.
//! A period of zero is refused: an unset key keeps forever. No openEHR spec
//! governs configuration: our own design.

use std::fmt;
use std::str::FromStr;

use serde::de::Visitor;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A calendar retention period, `PnY` or `PnD`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetentionPeriod {
    /// Records older than this many days are reaped.
    Days(u32),
    /// Records older than the same calendar date this many years ago are
    /// reaped.
    Years(u32),
}

/// Why a string is no retention period.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "`{0}` is not a retention period: write an ISO 8601 duration of whole years or \
     whole days, such as \"P5Y\" or \"P1830D\", or leave the key unset to keep forever"
)]
pub struct RetentionPeriodError(String);

impl FromStr for RetentionPeriod {
    type Err = RetentionPeriodError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let refused = || RetentionPeriodError(text.to_owned());
        let body = text.strip_prefix('P').ok_or_else(refused)?;
        let (count, unit) = body
            .char_indices()
            .last()
            .map(|(at, unit)| (body.get(..at).unwrap_or_default(), unit))
            .ok_or_else(refused)?;
        if count.is_empty() || !count.bytes().all(|b| b.is_ascii_digit()) {
            return Err(refused());
        }
        let Ok(count) = count.parse::<u32>() else {
            return Err(refused());
        };
        if count == 0 {
            return Err(refused());
        }
        match unit {
            'Y' => Ok(Self::Years(count)),
            'D' => Ok(Self::Days(count)),
            _ => Err(refused()),
        }
    }
}

impl fmt::Display for RetentionPeriod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Days(days) => write!(f, "P{days}D"),
            Self::Years(years) => write!(f, "P{years}Y"),
        }
    }
}

impl Serialize for RetentionPeriod {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for RetentionPeriod {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(PeriodVisitor)
    }
}

/// Reads a retention period from its string.
struct PeriodVisitor;

impl Visitor<'_> for PeriodVisitor {
    type Value = RetentionPeriod;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an ISO 8601 period of years or days, such as \"P5Y\" or \"P1830D\"")
    }

    fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<Self::Value, E> {
        text.parse().map_err(E::custom)
    }

    fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Self::Value, E> {
        Err(E::custom(RetentionPeriodError(value.to_string())))
    }

    fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
        Err(E::custom(RetentionPeriodError(value.to_string())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn years_and_days_read_and_render() {
        assert_eq!("P5Y".parse(), Ok(RetentionPeriod::Years(5)));
        assert_eq!("P1830D".parse(), Ok(RetentionPeriod::Days(1830)));
        assert_eq!(RetentionPeriod::Years(3).to_string(), "P3Y");
        assert_eq!(RetentionPeriod::Days(30).to_string(), "P30D");
    }

    #[test]
    fn anything_else_is_refused() {
        for bad in [
            "",
            "P",
            "5Y",
            "P0Y",
            "P0D",
            "P5M",
            "P1Y2D",
            "PT5H",
            "P-1D",
            "P 5Y",
            "p5y",
            "P5.5Y",
            "P99999999999D",
        ] {
            assert!(bad.parse::<RetentionPeriod>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_bare_number_is_refused_with_the_form_named() {
        #[derive(Debug, Deserialize)]
        struct Holder {
            #[expect(dead_code, reason = "read only to be refused")]
            keep: RetentionPeriod,
        }
        let err = toml::from_str::<Holder>("keep = 30").expect_err("a number");
        assert!(err.to_string().contains("P1830D"), "{err}");
    }
}
