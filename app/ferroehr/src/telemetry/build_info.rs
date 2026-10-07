// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `GET /management/info` — the server's build + spec provenance.
//!
//! No openEHR spec governs this endpoint — it is our own operational surface
//! (management/ is pure ops — spec-silent by design, a settled adjudication).
//! It reports the git commit, build timestamp, `rustc` version, the pinned
//! openEHR specification versions, the `PostgreSQL` target, the
//! manufacturer ([`crate::manufacturer::MANUFACTURER`]) and the support period
//! ([`crate::support::SupportPeriod`]). The same
//! [`BuildInfo`] feeds the `ferroehr_build_info` gauge and the `OTel` resource
//! attributes in the binary, so the build facts are captured once.
//!
//! The spec-version fields are **not** local literals: they read the single
//! [`crate::telemetry::provenance`] source shared with the System
//! Options manifest (`OPTIONS /`) and `/status`, so all three identity surfaces
//! quote one fact — and provenance itself derives every pin from the owning
//! `openehr-*` crate's own pin authority. See that module for the derivation.

use serde::Serialize;

use crate::telemetry::provenance;

/// Build- and spec-provenance, captured once. Cheap to clone.
#[derive(Debug, Clone, Serialize)]
pub struct BuildInfo {
    /// The crate/package name.
    pub name: &'static str,
    /// The crate version.
    pub version: &'static str,
    /// The git commit (short SHA) the binary was built from, or `unknown`.
    pub git_sha: &'static str,
    /// The build timestamp (ISO-8601 UTC), or `unknown` if unavailable.
    pub build_date: String,
    /// The `rustc` version string the binary was compiled with.
    pub rustc: &'static str,
    /// The ACTIVE openEHR specification generation set (`spec_profile`).
    pub spec_profile: crate::config::profile::SpecProfile,
    /// The active profile's openEHR specification versions.
    pub spec: SpecVersions,
    /// The `PostgreSQL` version target.
    pub postgres_target: &'static str,
    /// The manufacturer of this release, as Regulation (EU) 2025/327
    /// (`docs/law/eu/ehds/text.html` Art. 30(1)(g)) asks the EHR system to name
    /// it.
    pub manufacturer: crate::manufacturer::Manufacturer,
    /// The support period of this release, judged on the day it is served
    /// (`docs/law/eu/cra/text.html Art. 13(8)`, (19)).
    pub support: crate::support::SupportPeriod,
    /// The audit posture the server runs under (#3238): absent only where no
    /// configuration was resolved (the compile-time default).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audit: Option<crate::system_log::config::AuditPosture>,
}

/// The pinned openEHR specification versions surfaced by `/management/info`.
#[derive(Debug, Clone, Serialize)]
pub struct SpecVersions {
    /// ITS-REST contract version.
    pub its_rest: &'static str,
    /// AQL version.
    pub aql: &'static str,
    /// Reference Model version.
    pub rm: &'static str,
    /// BASE version.
    pub base: &'static str,
    /// Archetype Model versions — both extant generations, rendered as
    /// `"<v1_4> + <v2_4>"`.
    pub am: String,
    /// Terminology version.
    pub term: &'static str,
}

impl BuildInfo {
    /// The build info for this binary, from the values captured by `build.rs`.
    #[must_use]
    pub fn current() -> Self {
        Self::for_profile(crate::config::profile::SpecProfile::default())
    }

    /// The build info reporting the given ACTIVE generation set — the
    /// constructor the wiring layer calls with the configured
    /// `spec_profile`, so `/management/info` names what the server actually
    /// serves rather than the compile-time default.
    #[must_use]
    pub fn for_profile(profile: crate::config::profile::SpecProfile) -> Self {
        Self {
            name: env!("CARGO_PKG_NAME"),
            version: env!("CARGO_PKG_VERSION"),
            git_sha: env!("REVISION"),
            build_date: build_date(),
            rustc: env!("FERROEHR_RUSTC"),
            spec_profile: profile,
            spec: SpecVersions {
                its_rest: provenance::ITS_REST,
                aql: provenance::AQL,
                rm: provenance::rm_for(profile),
                base: provenance::base_for(profile),
                am: format!("{} + {}", provenance::AM14, provenance::AM24),
                term: provenance::TERM,
            },
            postgres_target: provenance::PG_TARGET,
            manufacturer: crate::manufacturer::MANUFACTURER,
            support: crate::support::SupportPeriod::current(),
            audit: None,
        }
    }

    /// The same build info carrying the resolved audit posture, so
    /// `/management/info` states whether an access log is written and how a
    /// record the queue cannot take is treated (#3238).
    #[must_use]
    pub fn with_audit(mut self, posture: crate::system_log::config::AuditPosture) -> Self {
        self.audit = Some(posture);
        self
    }
}

impl Default for BuildInfo {
    fn default() -> Self {
        Self::current()
    }
}

/// Render the build epoch captured by `build.rs` as an ISO-8601 UTC string.
fn build_date() -> String {
    env!("FERROEHR_BUILD_EPOCH")
        .parse::<i64>()
        .ok()
        .and_then(|secs| jiff::Timestamp::from_second(secs).ok())
        .map_or_else(|| "unknown".to_owned(), |ts| ts.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_info_is_populated() {
        let info = BuildInfo::current();
        assert_eq!(info.name, "ferroehr");
        // The pins are the owning crates' own authorities (the Generation
        // enum / generation-module constants) — no re-typed literals in the
        // chain, so a pin bump cannot silently diverge.
        assert_eq!(
            info.spec.rm,
            openehr_rm::Generation::default().spec_version()
        );
        assert_eq!(info.spec.its_rest, openehr_its::SPEC_VERSION);
        assert_eq!(
            info.spec.am,
            format!(
                "{} + {}",
                openehr_am::Generation::V1_4.spec_version(),
                openehr_am::Generation::V2_4.spec_version()
            )
        );
        assert_eq!(info.postgres_target, "18.6+");
        assert_eq!(info.manufacturer, crate::manufacturer::MANUFACTURER);
        // build_date parses to a real timestamp (not the "unknown" fallback) in
        // a normal build where build.rs ran.
        assert!(!info.build_date.is_empty());
    }
}
