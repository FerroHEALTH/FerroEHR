// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ITS-REST `version` path parameter: its grammar, judged before any
//! lookup. An ADL2 read then resolves the version by the engine's reference
//! rule (`adl2_resolution`).
//!
//! `parameters/path/version.yaml` defines it as "A SEMVER version number. This
//! can be an exact version (e.g. `1.7.1`), or a pattern as partial prefix, in a
//! form of `{major}` or `{major}.{minor}`", and the release names SEMVER as
//! Semantic Versioning 2.0.0 (`docs/overview/Glossary_and_conventions.md`). A
//! value of neither form is a syntactically invalid parameter, the `400` of
//! `responses/400.yaml`; a well-formed value no artefact carries stays the
//! operation's `404`.

use semver::Version;

use crate::service::error::ServiceError;

/// Whether `version` is an exact SEMVER version or a `{major}` /
/// `{major}.{minor}` prefix of one.
///
/// The exact form is the full Semantic Versioning 2.0.0 grammar, pre-release
/// and build metadata included (§9, §10), since `version.yaml` admits any
/// "SEMVER version number". A prefix is the leading numeric identifiers of a
/// normal version, so it carries no pre-release or build part and, like every
/// numeric identifier, no leading zero (§2: "MUST NOT contain leading
/// zeroes").
fn is_version_selector(version: &str) -> bool {
    if Version::parse(version).is_ok() {
        return true;
    }
    if version.contains(['-', '+']) {
        return false;
    }
    // A prefix completed with zero components is a normal version exactly
    // when every component the prefix supplies is a SEMVER numeric identifier.
    let completed = match version.matches('.').count() {
        0 => format!("{version}.0.0"),
        1 => format!("{version}.0"),
        _ => return false,
    };
    Version::parse(&completed).is_ok()
}

/// Refuse a `version` path parameter that is no SEMVER version and no
/// `{major}` or `{major}.{minor}` prefix.
///
/// # Errors
///
/// A malformed `version` → `BadRequest` (`precondition_violation`, `400`,
/// `responses/400.yaml`: "syntactically invalid … parameter").
pub(super) fn check_version_selector(version: &str) -> Result<(), ServiceError> {
    if is_version_selector(version) {
        return Ok(());
    }
    Err(ServiceError::precondition(format!(
        "`{version}` is not a SEMVER version: the `version` parameter is an exact \
         version (`1.7.1`) or a `{{major}}` or `{{major}}.{{minor}}` prefix \
         (parameters/path/version.yaml)"
    )))
}

/// Whether a `version` path parameter is a SEMVER *prefix* (`{major}` or
/// `{major}.{minor}`) rather than an exact `{major}.{minor}.{patch}` triple
/// (`parameters/path/version.yaml`).
pub(super) fn is_partial_semver(version: &str) -> bool {
    let segments: Vec<&str> = version.split('.').collect();
    segments.len() < 3
        && segments
            .iter()
            .all(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_versions_are_selectors() {
        assert!(is_version_selector("1.7.1"));
        assert!(is_version_selector("0.0.0"));
        assert!(is_version_selector("10.20.30"));
        // Semantic Versioning 2.0.0 §9 and §10.
        assert!(is_version_selector("1.0.0-rc.1"));
        assert!(is_version_selector("1.0.0+build.007"));
        assert!(is_version_selector("1.0.0-alpha+001"));
    }

    #[test]
    fn major_and_minor_prefixes_are_selectors() {
        assert!(is_version_selector("1"));
        assert!(is_version_selector("0"));
        assert!(is_version_selector("1.0"));
        assert!(is_version_selector("12.34"));
    }

    #[test]
    fn malformed_values_are_refused() {
        for bad in [
            "", "v1.x", "v1", "1.x", "*", "1.", ".1", "1..0", "1.0.", "1.0.0.0", "a", "1.a",
            // §2: no leading zeroes in a numeric identifier.
            "01", "1.01", "01.0.0", "1.0.01",
            // §9: a numeric pre-release identifier has no leading zero.
            "1.0.0-01", // A prefix carries no pre-release or build part.
            "1-rc", "1.0+b", " 1", "1 ",
        ] {
            assert!(!is_version_selector(bad), "{bad:?} should be refused");
            assert!(check_version_selector(bad).is_err());
        }
    }

    #[test]
    fn partial_semver_detection() {
        // `{major}` / `{major}.{minor}` prefixes (version.yaml).
        assert!(is_partial_semver("1"));
        assert!(is_partial_semver("1.0"));
        assert!(is_partial_semver("12.34"));
        // Exact triples and malformed tokens are not prefixes.
        assert!(!is_partial_semver("1.0.0"));
        assert!(!is_partial_semver("1..0"));
        assert!(!is_partial_semver("1.a"));
        assert!(!is_partial_semver(""));
    }
}
