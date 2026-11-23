// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The abstract `Iso8601_type` functions `is_partial` and `is_extended`,
//! dispatched over its descendants (`iso8601_type.adoc` §Functions).

use super::iso8601_type::Iso8601Type;

impl Iso8601Type {
    /// `Iso8601_type.is_partial` of the carried descendant; an unparseable
    /// duration answers its effected "Returns False".
    #[must_use]
    pub fn is_partial(&self) -> bool {
        match self {
            Self::Iso8601Date(date) => date.is_partial(),
            Self::Iso8601DateTime(date_time) => date_time.is_partial(),
            Self::Iso8601Duration(duration) => duration.is_partial().unwrap_or(false),
            Self::Iso8601Time(time) => time.is_partial(),
            Self::Iso8601Timezone(timezone) => timezone.is_partial(),
        }
    }

    /// `Iso8601_type.is_extended` of the carried descendant; an unparseable
    /// duration answers its effected "Returns True".
    #[must_use]
    pub fn is_extended(&self) -> bool {
        match self {
            Self::Iso8601Date(date) => date.is_extended(),
            Self::Iso8601DateTime(date_time) => date_time.is_extended(),
            Self::Iso8601Duration(duration) => duration.is_extended().unwrap_or(true),
            Self::Iso8601Time(time) => time.is_extended(),
            Self::Iso8601Timezone(timezone) => timezone.is_extended(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::iso8601_date::Iso8601Date;
    use super::super::iso8601_date_time::Iso8601DateTime;
    use super::super::iso8601_duration::Iso8601Duration;
    use super::super::iso8601_time::Iso8601Time;
    use super::super::iso8601_timezone::Iso8601Timezone;
    use super::*;

    #[test]
    fn the_abstract_functions_reach_every_descendant() {
        let cases: [(Iso8601Type, bool, bool); 5] = [
            (
                Iso8601Type::Iso8601Date(Iso8601Date {
                    value: "2020-06".to_owned(),
                }),
                true,
                true,
            ),
            (
                Iso8601Type::Iso8601DateTime(Iso8601DateTime {
                    value: "20200615T120000".to_owned(),
                }),
                false,
                false,
            ),
            (
                Iso8601Type::Iso8601Duration(Iso8601Duration {
                    value: "P1Y".to_owned(),
                }),
                false,
                true,
            ),
            (
                Iso8601Type::Iso8601Time(Iso8601Time {
                    value: "1200".to_owned(),
                }),
                true,
                false,
            ),
            (
                Iso8601Type::Iso8601Timezone(Iso8601Timezone {
                    value: "+0100".to_owned(),
                }),
                false,
                false,
            ),
        ];
        for (value, is_partial, is_extended) in cases {
            assert_eq!(value.is_partial(), is_partial, "is_partial of {value:?}");
            assert_eq!(value.is_extended(), is_extended, "is_extended of {value:?}");
        }
    }

    /// A value none of the descendants can decompose still answers both
    /// functions — they are `1..1` on this class.
    #[test]
    fn an_undecidable_value_still_answers_both_functions() {
        let bad = Iso8601Type::Iso8601Date(Iso8601Date {
            value: "not-a-date".to_owned(),
        });
        assert!(bad.is_partial());
        assert!(!bad.is_extended());

        let bad_duration = Iso8601Type::Iso8601Duration(Iso8601Duration {
            value: "1Y".to_owned(),
        });
        assert!(!bad_duration.is_partial());
        assert!(bad_duration.is_extended());
    }
}
