// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The licence document: what a token asserts, and nothing more.

use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The permitted-use classes a licence can grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Use {
    /// Production use in the course of a business, which BUSL-1.1 reserves for
    /// holders of a commercial licence.
    #[serde(rename = "commercial")]
    Commercial,
    /// What BUSL-1.1 grants everyone: non-production use, and production use
    /// for non-commercial purposes. The token the build embeds carries this
    /// class, so every deployment runs under an explicit licence.
    #[serde(rename = "non-commercial")]
    NonCommercial,
}

impl Use {
    /// The wire spelling, as the status document reports it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Commercial => "commercial",
            Self::NonCommercial => "non-commercial",
        }
    }
}

/// The signed payload of a licence token.
///
/// Field order is the wire order: [`Licence::to_canonical_json`] renders the
/// struct in declaration order and the signature covers exactly that text.
/// The reader is strict, so an unknown field refuses the document instead of
/// being ignored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Licence {
    /// Stable identity of this grant, also the material of its stamp key.
    #[serde(rename = "licence_id")]
    pub id: Uuid,
    /// The legal entity the grant is issued to.
    pub licensee: String,
    /// The day the token was signed.
    pub issued: Date,
    /// First day (inclusive, UTC) the grant is active.
    pub not_before: Date,
    /// Last day (inclusive, UTC) the grant is active.
    pub not_after: Date,
    /// What the grant permits.
    #[serde(rename = "use")]
    pub permitted_use: Use,
}

/// Where `today` falls relative to a licence's validity window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Window {
    /// Today precedes `not_before`.
    NotYetValid,
    /// Today is inside the window.
    Active,
    /// Today follows `not_after`.
    Expired,
}

impl Licence {
    /// The exact text that is signed: pretty JSON in field order, one trailing
    /// newline, no tabs and no trailing whitespace on any line (the cleartext
    /// signature format strips trailing whitespace, so none may carry meaning).
    ///
    /// # Errors
    /// Propagates the serialiser's error; with only scalar fields it has no
    /// realistic failure path, but a silent empty document would be worse.
    pub fn to_canonical_json(&self) -> Result<String, serde_json::Error> {
        let body = serde_json::to_string_pretty(self)?;
        Ok(format!("{body}\n"))
    }

    /// Parse a signed text back into a licence, strictly.
    ///
    /// # Errors
    /// Any deviation from the schema: an unknown field, a malformed date, a
    /// use class this module does not know.
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    /// Classify `today` against the validity window.
    #[must_use]
    pub fn window(&self, today: Date) -> Window {
        if today < self.not_before {
            Window::NotYetValid
        } else if today > self.not_after {
            Window::Expired
        } else {
            Window::Active
        }
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::*;

    fn sample() -> Licence {
        Licence {
            id: Uuid::nil(),
            licensee: "Example Hospital NV".to_owned(),
            issued: date(2026, 9, 11),
            not_before: date(2026, 9, 11),
            not_after: date(2027, 9, 10),
            permitted_use: Use::Commercial,
        }
    }

    #[test]
    fn canonical_json_round_trips_and_is_stable() {
        let text = sample().to_canonical_json().unwrap();
        assert!(text.ends_with("}\n"));
        assert!(!text.contains('\t'));
        assert!(text.lines().all(|l| l.trim_end() == l));
        assert!(text.contains("\"use\": \"commercial\""));
        assert_eq!(Licence::from_json(&text).unwrap(), sample());
    }

    #[test]
    fn both_use_classes_spell_as_the_status_reports_them() {
        for (class, spelling) in [
            (Use::Commercial, "commercial"),
            (Use::NonCommercial, "non-commercial"),
        ] {
            assert_eq!(class.as_str(), spelling);
            assert_eq!(
                serde_json::to_value(class).unwrap(),
                serde_json::Value::String(spelling.to_owned())
            );
        }
    }

    #[test]
    fn unknown_fields_are_refused() {
        let text = sample()
            .to_canonical_json()
            .unwrap()
            .replace("\"licensee\"", "\"seats\": 5,\n  \"licensee\"");
        assert!(Licence::from_json(&text).is_err());
    }

    #[test]
    fn window_is_inclusive_on_both_ends() {
        let l = sample();
        assert_eq!(l.window(date(2026, 9, 10)), Window::NotYetValid);
        assert_eq!(l.window(date(2026, 9, 11)), Window::Active);
        assert_eq!(l.window(date(2027, 9, 10)), Window::Active);
        assert_eq!(l.window(date(2027, 9, 11)), Window::Expired);
    }
}
