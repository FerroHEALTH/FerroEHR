// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `spec_profile` key — which openEHR specification GENERATION SET the
//! server runs.
//!
//! One coupled profile selects the whole set: per-component free choice would
//! admit incoherent combinations, an RM modelled against a BASE it never
//! included, so the enum's variants are the only representable states. The within-major
//! compatibility ground is the openEHR release strategy
//! (<https://specifications.openehr.org/governance>): minor releases are
//! additive, so every stable-generation instance is valid under the
//! development generations — the reverse is NOT guaranteed, which is what
//! the acceptance boundary enforces.
//!
//! No openEHR spec governs runtime version selection — our own
//! design/extension.

use serde::{Deserialize, Serialize};

/// The openEHR specification generation set the server runs.
///
/// Selected by the top-level `spec_profile` configuration key. Each variant
/// maps to one coherent generation triple; the accessors return the
/// generated crates' own [`openehr_rm::Generation`]-family values, so the
/// profile can never name a generation the crates do not emit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpecProfile {
    /// The latest RELEASED generations: RM 1.1.0 + BASE 1.2.0 + LANG 1.0.0.
    Stable,
    /// The development pins this workspace is built against: RM 1.2.0 +
    /// BASE 1.3.0 + LANG 1.1.0. The default — today's behaviour.
    #[default]
    Development,
}

impl SpecProfile {
    /// Returns the RM generation this profile selects.
    #[must_use]
    pub const fn rm(self) -> openehr_rm::Generation {
        match self {
            Self::Stable => openehr_rm::Generation::V1_1,
            Self::Development => openehr_rm::Generation::V1_2,
        }
    }

    /// Returns the BASE generation this profile selects.
    #[must_use]
    pub const fn base(self) -> openehr_base::Generation {
        match self {
            Self::Stable => openehr_base::Generation::V1_2,
            Self::Development => openehr_base::Generation::V1_3,
        }
    }

    /// Returns the LANG generation this profile selects.
    #[must_use]
    pub const fn lang(self) -> openehr_lang::Generation {
        match self {
            Self::Stable => openehr_lang::Generation::V1_0,
            Self::Development => openehr_lang::Generation::V1_1,
        }
    }

    /// Returns the configuration token (`"stable"` / `"development"`) — the
    /// [`std::fmt::Display`] and [`std::str::FromStr`] form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Development => "development",
        }
    }
}

impl std::fmt::Display for SpecProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when parsing a [`SpecProfile`] from an unknown token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecProfileParseError {
    unrecognized: String,
}

impl std::fmt::Display for SpecProfileParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "unknown spec_profile {:?} (valid: `stable`, `development`)",
            self.unrecognized
        )
    }
}

impl std::error::Error for SpecProfileParseError {}

impl std::str::FromStr for SpecProfile {
    type Err = SpecProfileParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "stable" => Ok(Self::Stable),
            "development" => Ok(Self::Development),
            other => Err(SpecProfileParseError {
                unrecognized: other.to_owned(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default profile is `development` — the pins this workspace is
    /// built against; adding the key cannot change existing deployments.
    #[test]
    fn default_is_development() {
        assert_eq!(SpecProfile::default(), SpecProfile::Development);
        assert_eq!(
            SpecProfile::default().rm(),
            openehr_rm::Generation::default()
        );
        assert_eq!(
            SpecProfile::default().base(),
            openehr_base::Generation::default()
        );
        assert_eq!(
            SpecProfile::default().lang(),
            openehr_lang::Generation::default()
        );
    }

    /// The coherent triples (owner hard rule 2026-08-05): stable = the
    /// released generations, development = the current pins.
    #[test]
    fn profiles_map_to_the_coherent_triples() {
        assert_eq!(SpecProfile::Stable.rm().spec_version(), "1.1.0");
        assert_eq!(SpecProfile::Stable.base().spec_version(), "1.2.0");
        assert_eq!(SpecProfile::Stable.lang().spec_version(), "1.0.0");
        assert_eq!(SpecProfile::Development.rm().spec_version(), "1.2.0");
        assert_eq!(SpecProfile::Development.base().spec_version(), "1.3.0");
        assert_eq!(SpecProfile::Development.lang().spec_version(), "1.1.0");
    }

    /// `Display`/`FromStr` round-trip the configuration token, and serde uses
    /// the same spelling.
    #[test]
    #[expect(
        clippy::panic_in_result_fn,
        reason = "Result-returning test with assertions — the Book ch11 shape \
                  (https://doc.rust-lang.org/book/ch11-01-writing-tests.html)"
    )]
    fn tokens_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        #[derive(Deserialize)]
        struct Probe {
            profile: SpecProfile,
        }
        for p in [SpecProfile::Stable, SpecProfile::Development] {
            assert_eq!(p.to_string().parse::<SpecProfile>()?, p);
            let toml = format!("profile = \"{p}\"");
            let probe: Probe = toml::from_str(&toml)?;
            assert_eq!(probe.profile, p);
        }
        assert!("prod".parse::<SpecProfile>().is_err());
        Ok(())
    }
}
