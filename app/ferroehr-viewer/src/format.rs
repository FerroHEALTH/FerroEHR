// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Representation formats the viewer negotiates with the CDR.
//!
//! The ITS-REST canonical forms plus the Simplified Formats media types.
//! FLAT and STRUCTURED are the two the format spec itself defines
//! (`docs/specs/openehr/ITS-REST/docs/simplified_formats/master02-overview.adoc`
//! §MIME Types); the Web Template type is named by the REST spec
//! (`docs/specs/openehr/ITS-REST/specifications/docs/overview/Resources.md`
//! §Data representation / Simplified Formats). Negotiation is strict
//! `Accept`/`Content-Type` — there is no `?format=` parameter.
//!
//! Beside the media types, the one VALUE format the viewer completes by hand:
//! [`datetime_local_to_rfc3339`], which turns a browser `datetime-local`
//! control's value into the instant `version_at_time` takes — the same
//! `Resources.md` the media types above cite.

use serde::{Deserialize, Serialize};

/// One selectable representation of an openEHR resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReprFormat {
    /// Canonical JSON (`application/json`).
    CanonicalJson,
    /// Canonical XML (`application/xml`).
    CanonicalXml,
    /// Simplified FLAT (`application/openehr.wt.flat+json`).
    Flat,
    /// Simplified STRUCTURED (`application/openehr.wt.structured+json`).
    Structured,
    /// The Web Template rendering of a template (`application/openehr.wt+json`).
    WebTemplate,
}

impl ReprFormat {
    /// The exact media type sent as `Accept` / `Content-Type`.
    #[must_use]
    pub fn media_type(self) -> &'static str {
        match self {
            Self::CanonicalJson => "application/json",
            Self::CanonicalXml => "application/xml",
            Self::Flat => "application/openehr.wt.flat+json",
            Self::Structured => "application/openehr.wt.structured+json",
            Self::WebTemplate => "application/openehr.wt+json",
        }
    }

    /// Short human label for the format selector.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::CanonicalJson => "JSON",
            Self::CanonicalXml => "XML",
            Self::Flat => "FLAT",
            Self::Structured => "STRUCTURED",
            Self::WebTemplate => "WT",
        }
    }
}

/// Completes a browser `datetime-local` value into a `version_at_time` instant.
///
/// Query parameters that are datetimes "MUST always use the _extended_ ISO 8601
/// format", whose general form is `YYYY-MM-DDThh:mm:ss.sss[Z|±hh:mm]`
/// (`docs/specs/openehr/ITS-REST/specifications/docs/overview/Resources.md`
/// §Datetime format) — and a `datetime-local` control emits only the
/// `YYYY-MM-DDThh:mm` prefix of that.
///
/// Absent seconds therefore default to `:00` and the zone to `Z`; a value that
/// already names its zone — a trailing `Z`, or a numeric offset — is returned
/// unchanged. Empty input yields an empty string, which every caller rejects
/// before the round-trip.
///
/// This is the viewer's ONE normalizer for the parameter: every time-travel
/// picker goes through it, so one typed instant can never mean two things on the
/// wire, and the CDR's own `400` is the arbiter for anything else.
///
/// NOTE: the zone is stamped deliberately — a zone-less parameter means "the
/// local timezone is assumed"
/// (`docs/specs/openehr/ITS-REST/specifications/docs/overview/Resources.md`
/// §Datetime format), which on the wire is the CDR's local zone, not the
/// operator's; every picker feeding this function labels its field as UTC, so
/// the `Z` states the instant the operator actually asked for.
#[must_use]
pub fn datetime_local_to_rfc3339(local: &str) -> String {
    let trimmed = local.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if carries_zone(trimmed) {
        return trimmed.to_owned();
    }
    // `HH:MM` carries one colon, `HH:MM:SS` two — add seconds when absent.
    let with_seconds = if trimmed.matches(':').count() < 2 {
        format!("{trimmed}:00")
    } else {
        trimmed.to_owned()
    };
    format!("{with_seconds}Z")
}

/// Whether a datetime value already names its zone: a trailing `Z`, or a
/// numeric UTC offset, which can only sit after the `YYYY-MM-DD` date part —
/// so the date's own separators never read as one.
fn carries_zone(value: &str) -> bool {
    value.ends_with('Z')
        || value.ends_with('z')
        || value
            .get(10..)
            .is_some_and(|tail| tail.contains('+') || tail.contains('-'))
}

#[cfg(test)]
mod tests {
    use super::{ReprFormat, datetime_local_to_rfc3339};

    #[test]
    fn datetime_local_completes_to_an_extended_iso_8601_instant() {
        // A `datetime-local` value with no seconds gains `:00` and a `Z` zone.
        assert_eq!(
            datetime_local_to_rfc3339("2026-07-12T10:30"),
            "2026-07-12T10:30:00Z"
        );
        // With seconds, only the zone is appended.
        assert_eq!(
            datetime_local_to_rfc3339("2026-07-12T10:30:45"),
            "2026-07-12T10:30:45Z"
        );
        // Surrounding whitespace is trimmed.
        assert_eq!(
            datetime_local_to_rfc3339("  2026-07-12T08:00  "),
            "2026-07-12T08:00:00Z"
        );
        // Empty stays empty (the server fn rejects it before the round-trip).
        assert_eq!(datetime_local_to_rfc3339(""), "");
        assert_eq!(datetime_local_to_rfc3339("  "), "");
    }

    #[test]
    fn an_already_zoned_value_is_never_stamped_a_second_time() {
        for zoned in [
            "2026-07-12T10:30:00Z",
            "2026-07-18T14:30:00+02:00",
            "2026-07-18T14:30:00-05:00",
            "2026-07-18T14:30:00.500Z",
        ] {
            assert_eq!(datetime_local_to_rfc3339(zoned), zoned);
        }
    }

    #[test]
    fn a_value_of_neither_shape_is_completed_all_the_same() {
        // No pass-through leniency: an unrecognized length is completed like
        // any other zone-less value, and the CDR's `400` judges it.
        assert_eq!(
            datetime_local_to_rfc3339("2026-07-18T14"),
            "2026-07-18T14:00Z"
        );
        assert_eq!(
            datetime_local_to_rfc3339("2026-07-18T14:30:15.250"),
            "2026-07-18T14:30:15.250Z"
        );
        assert_eq!(datetime_local_to_rfc3339("nonsense"), "nonsense:00Z");
    }

    #[test]
    fn media_types_match_the_simplified_formats_spec() {
        // FLAT + STRUCTURED: simplified_formats/master02-overview.adoc §MIME
        // Types; wt+json: specifications/docs/overview/Resources.md §Data
        // representation.
        assert_eq!(
            ReprFormat::Flat.media_type(),
            "application/openehr.wt.flat+json"
        );
        assert_eq!(
            ReprFormat::Structured.media_type(),
            "application/openehr.wt.structured+json"
        );
        assert_eq!(
            ReprFormat::WebTemplate.media_type(),
            "application/openehr.wt+json"
        );
        assert_eq!(ReprFormat::CanonicalJson.media_type(), "application/json");
        assert_eq!(ReprFormat::CanonicalXml.media_type(), "application/xml");
    }
}
