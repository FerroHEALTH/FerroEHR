// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADMIN group's information classes (SM `I_ADMIN_SERVICE` +
//! `I_ADMIN_DUMP_LOAD` parameter/report types).

/// A statistics time filter: an optional `(lower, upper)` pair of ISO 8601
/// date-time bounds, each independently optional (open bounds allowed).
///
/// Realizes the SM `time_interval: Interval<Iso8601_date_time> [0..1]`
/// parameter of the four `i_admin_service.adoc` statistics calls.
///
/// NOTE: the SM `Interval` is treated as **closed** `[lower, upper]` — the
/// default openEHR `Interval` bound inclusivity — matched against each
/// CONTRIBUTION/version audit `time_committed`. An invalid ISO bound is a `400`
/// (rejected at the service boundary before the query runs), and so is a
/// bounded pair with `lower > upper`: BASE
/// `org.openehr.base.foundation_types.interval.adoc` §Invariants
/// (`Limits_consistent`) makes such a pair no `Interval` at all.
pub type StatTimeRange = Option<(Option<String>, Option<String>)>;

/// `EXPORT_FORMAT` enumeration
/// (`docs/specs/openehr/SM/docs/UML/classes/export_format.adoc`): the logical
/// serialization flavour a dump is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// `openehr_canonical_xml`.
    OpenehrCanonicalXml,
    /// `openehr_canonical_json`.
    OpenehrCanonicalJson,
}

impl ExportFormat {
    /// The SM enumeration literal, exactly as the spec spells it.
    #[must_use]
    pub fn sm_name(self) -> &'static str {
        match self {
            Self::OpenehrCanonicalXml => "openehr_canonical_xml",
            Self::OpenehrCanonicalJson => "openehr_canonical_json",
        }
    }

    /// Both vendored members, in `export_format.adoc` order.
    pub const ALL: &[ExportFormat] = &[Self::OpenehrCanonicalXml, Self::OpenehrCanonicalJson];
}

impl std::str::FromStr for ExportFormat {
    type Err = ();

    /// Parse an SM enumeration literal ASCII-case-insensitively (the same
    /// case rule [`crate::service::platform_service::PlatformService`] states:
    /// BASE `master05` §"Composite Identifiers and Case" — case alone must not
    /// make two spellings name different things). No openEHR spec governs how
    /// a member is spelled on a REST wire; `export_ehrs` has no released
    /// endpoint at all, so the transport is our own design/extension.
    ///
    /// # Errors
    /// `Err(())` when the text is neither vendored member; the caller decides
    /// the wire status.
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|member| member.sm_name().eq_ignore_ascii_case(raw))
            .ok_or(())
    }
}

/// `COMPRESSION_FORMAT` enumeration
/// (`docs/specs/openehr/SM/docs/UML/classes/compression_format.adoc`): the
/// compression to apply while dumping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionFormat {
    /// `zip`.
    Zip,
    /// `7z` (not a valid Rust identifier — the SM literal is `7z`).
    SevenZip,
}

impl CompressionFormat {
    /// The SM enumeration literal, exactly as the spec spells it.
    #[must_use]
    pub fn sm_name(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::SevenZip => "7z",
        }
    }

    /// Both vendored members, in `compression_format.adoc` order.
    pub const ALL: &[CompressionFormat] = &[Self::Zip, Self::SevenZip];
}

impl std::str::FromStr for CompressionFormat {
    type Err = ();

    /// Parse an SM enumeration literal ASCII-case-insensitively (see
    /// `ExportFormat::from_str` for the case rule and the spec-silence flag).
    ///
    /// # Errors
    /// `Err(())` when the text is neither vendored member.
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|member| member.sm_name().eq_ignore_ascii_case(raw))
            .ok_or(())
    }
}

/// `EXPORT_SPEC` class
/// (`docs/specs/openehr/SM/docs/UML/classes/export_spec.adoc`): "the details for
/// an export operation".
///
/// NOTE: `export_ehrs` takes an `ExportSpec` rather than the loose format enums
/// its `I_ADMIN_DUMP_LOAD` signature passes, because `EXPORT_SPEC` is the SM's
/// own richer bundle for exactly this operation (it also carries the mandatory
/// `segment_split_size [1..1]` the signature omits). `ENCODING_FORMAT` is an
/// **empty enumeration** (no values in `encoding_format.adoc`), so the SM
/// `encoding` attribute has no representable value and is dropped here.
#[derive(Debug, Clone)]
pub struct ExportSpec {
    /// Logical format to use, i.e. flavour of XML, JSON etc.
    pub logical_format: Option<ExportFormat>,
    /// Compression format to use during dump.
    pub compression_format: Option<CompressionFormat>,
    /// Size in kb of segment size on file system to split the export into
    /// (`segment_split_size`, 1..1).
    pub segment_split_size: i32,
}

impl ExportSpec {
    /// An uncompressed canonical-JSON export split into `segment_split_size_kb`
    /// segments — the format the greenfield storage exports natively (a
    /// deliberate design decision: `node.data` is verbatim canonical openEHR
    /// JSON).
    #[must_use]
    pub fn canonical_json(segment_split_size_kb: i32) -> Self {
        Self {
            logical_format: Some(ExportFormat::OpenehrCanonicalJson),
            compression_format: None,
            segment_split_size: segment_split_size_kb,
        }
    }

    /// An uncompressed `openehr_canonical_xml` export split into
    /// `segment_split_size_kb` segments: the archive skeleton stays JSON and
    /// each version's payload is externalized as an `ORIGINAL_VERSION`
    /// document under the published ITS-XML `<version>` root (the derivation
    /// is in `crate::service::admin::dump_load`'s module docs).
    #[must_use]
    pub fn canonical_xml(segment_split_size_kb: i32) -> Self {
        Self {
            logical_format: Some(ExportFormat::OpenehrCanonicalXml),
            compression_format: None,
            segment_split_size: segment_split_size_kb,
        }
    }
}

/// `DUMP_LOAD_FAIL_REPORT` class
/// (`docs/specs/openehr/SM/docs/UML/classes/dump_load_fail_report.adoc`):
/// "Dump or Load fail report for a single entity, e.g. `EHR`, `PARTY` etc."
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DumpLoadFailReport {
    /// Type name of entity (`entity_type`, 1..1).
    pub entity_type: String,
    /// Identifier of entity (`entity_id`, 1..1).
    pub entity_id: String,
    /// Status of entity in the dump/load operation (`dump_status`, 1..1):
    /// `true` = successfully dumped/loaded; `false` = failed for this entity.
    pub dump_status: bool,
    /// Detailed error information, if available (`error`, 0..1).
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_enum_literals_round_trip_through_from_str() {
        use std::str::FromStr;
        for member in ExportFormat::ALL {
            assert_eq!(ExportFormat::from_str(member.sm_name()), Ok(*member));
        }
        for member in CompressionFormat::ALL {
            assert_eq!(CompressionFormat::from_str(member.sm_name()), Ok(*member));
        }
        // The case rule (BASE master05 §"Composite Identifiers and Case").
        assert_eq!(
            CompressionFormat::from_str("ZIP"),
            Ok(CompressionFormat::Zip)
        );
        // A non-member is refused, never coerced to a default.
        assert_eq!(ExportFormat::from_str("canonical_json"), Err(()));
        assert_eq!(CompressionFormat::from_str("gzip"), Err(()));
    }

    #[test]
    fn format_enum_sm_names_match_the_spec_literals() {
        assert_eq!(
            ExportFormat::OpenehrCanonicalXml.sm_name(),
            "openehr_canonical_xml"
        );
        assert_eq!(
            ExportFormat::OpenehrCanonicalJson.sm_name(),
            "openehr_canonical_json"
        );
        assert_eq!(CompressionFormat::Zip.sm_name(), "zip");
        assert_eq!(CompressionFormat::SevenZip.sm_name(), "7z");
    }

    #[test]
    fn canonical_json_spec_defaults_to_uncompressed_json() {
        let spec = ExportSpec::canonical_json(1024);
        assert_eq!(
            spec.logical_format,
            Some(ExportFormat::OpenehrCanonicalJson)
        );
        assert_eq!(spec.compression_format, None);
        assert_eq!(spec.segment_split_size, 1024);
    }
}
