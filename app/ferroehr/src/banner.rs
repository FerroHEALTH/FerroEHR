// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The startup ASCII-art banner printed to stdout before the structured
//! startup logs, in the spirit of the reference implementation's Spring banner.
//!
//! No openEHR spec governs this — it is our own presentation. The wordmark is a
//! fixed, hand-committed static string (generated once with a `FIGlet`
//! "standard" font and vendored here), so the boot path carries **zero** runtime
//! dependency and no font-asset load for a fixed piece of art. The version is
//! substituted from `CARGO_PKG_VERSION` at build time; the load-bearing spec
//! pins are read from [`crate::telemetry::provenance`] — the derived
//! crate-version constants — never re-typed here.

use std::fmt::Write as _;

/// The `FerroEHR` ASCII wordmark (`FIGlet` "standard" font, five lines,
/// ≤ 55 columns). A raw string so the backslashes/quotes in the art are
/// literal.
const WORDMARK: &str = r"
 _____                   _____ _   _ ____
|  ___|__ _ __ _ __ ___ | ____| | | |  _ \
| |_ / _ \ '__| '__/ _ \|  _| | |_| | |_) |
|  _|  __/ |  | | | (_) | |___|  _  |  _ <
|_|  \___|_|  |_|  \___/|_____|_| |_|_| \_\";

/// The project's public repository.
const PROJECT_URL: &str = "https://github.com/FerroHEALTH/FerroEHR";

/// Render the full banner for the given product `version` and ACTIVE
/// generation set.
///
/// Kept parameterized (rather than reading `CARGO_PKG_VERSION` / a global)
/// so it is unit-testable; [`print()`] supplies the real values. The pins
/// are read from the shared [`crate::telemetry::provenance`] source, so the
/// banner can never drift from what the server actually serves.
#[must_use]
pub fn render(
    version: &str,
    profile: crate::config::profile::SpecProfile,
    deployment: &crate::config::deployment::DeploymentPosture,
    colour: bool,
) -> String {
    let mut out = format!(
        "{WORDMARK}\n\n  \
         openEHR-conformant Clinical Data Repository · v{version}\n  \
         Maintained by Ruben Talstra · {PROJECT_URL}\n\n"
    );
    let pins: &[(&str, &str)] = &[
        ("Profile", profile.as_str()),
        ("openEHR RM", crate::telemetry::provenance::rm_for(profile)),
        ("ITS-REST", crate::telemetry::provenance::ITS_REST),
        ("AQL", crate::telemetry::provenance::AQL),
        ("PostgreSQL", crate::telemetry::provenance::PG_TARGET),
    ];
    for (label, pin) in pins {
        // Left-pad the version column so the pins line up as a list.
        let _ = writeln!(out, "  {label:<12}{pin}");
    }
    let _ = writeln!(out, "  {:<12}{}", "Deployment", deployment.profile);
    // The sandbox notice: red where a terminal shows colour, and the same
    // words where it does not, because colour is the first thing a scraped
    // log loses (#3226).
    if deployment.profile == crate::config::deployment::DeploymentProfile::Sandbox {
        let (on, off) = if colour {
            ("\x1b[1;31m", "\x1b[0m")
        } else {
            ("", "")
        };
        let _ = writeln!(out);
        for line in wrap(
            "SANDBOX: this deployment has not made the production separations and must not hold \
             real patient data.",
            88,
        ) {
            let _ = writeln!(out, "{on}  {line}{off}");
        }
        for gap in &deployment.gaps {
            let _ = writeln!(out, "{on}  - {gap}{off}");
            for line in wrap(gap.describe(), 88) {
                let _ = writeln!(out, "{on}      {line}{off}");
            }
        }
    }
    out
}

/// Greedy word wrap at `width` columns, for the banner's fixed-width lines.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if !current.is_empty() && current.len() + 1 + word.len() > width {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Print the banner to stdout. Called from the binary before telemetry/log
/// initialisation so the structured formatter never mangles the art.
#[expect(
    clippy::print_stdout,
    reason = "the boot banner IS console output, and it prints before any \
              tracing subscriber exists"
)]
pub fn print(
    profile: crate::config::profile::SpecProfile,
    deployment: &crate::config::deployment::DeploymentPosture,
    colour: bool,
) {
    println!(
        "{}",
        render(env!("CARGO_PKG_VERSION"), profile, deployment, colour)
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::deployment::{DeploymentGap, DeploymentPosture, DeploymentProfile};

    fn sandbox() -> DeploymentPosture {
        DeploymentPosture {
            profile: DeploymentProfile::Sandbox,
            gaps: vec![DeploymentGap::SharedCredential],
            accepted: Vec::new(),
        }
    }

    /// The sandbox notice names the rule and every open gap, with or without
    /// colour; a production banner carries neither.
    #[test]
    fn the_sandbox_notice_survives_without_colour() {
        let plain = render(
            "1.0.0",
            crate::config::profile::SpecProfile::Development,
            &sandbox(),
            false,
        );
        // The notice is wrapped to the banner's width, so compare on words.
        let words = plain.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(words.contains("must not hold real patient data"), "{plain}");
        assert!(plain.contains("shared_credential"), "{plain}");
        assert!(!plain.contains("\x1b["), "no escape codes without colour");
        let coloured = render(
            "1.0.0",
            crate::config::profile::SpecProfile::Development,
            &sandbox(),
            true,
        );
        assert!(coloured.contains("\x1b[1;31m"), "{coloured}");
        let production = DeploymentPosture {
            profile: DeploymentProfile::Production,
            gaps: Vec::new(),
            accepted: Vec::new(),
        };
        let quiet = render(
            "1.0.0",
            crate::config::profile::SpecProfile::Development,
            &production,
            true,
        );
        assert!(
            quiet.contains("production") && !quiet.contains("SANDBOX"),
            "{quiet}"
        );
    }

    #[test]
    fn banner_contains_version_maintainer_and_url() {
        let b = render(
            "9.9.9",
            crate::config::profile::SpecProfile::Development,
            &sandbox(),
            false,
        );
        assert!(b.contains("v9.9.9"), "version must be substituted");
        assert!(b.contains("Ruben Talstra"), "maintainer credit must appear");
        assert!(
            b.contains("https://github.com/FerroHEALTH/FerroEHR"),
            "project URL must appear"
        );
        assert!(b.contains("Profile"));
        assert!(b.contains("development"));
        assert!(b.contains("openEHR RM"));
        assert!(b.contains("PostgreSQL"));
    }

    /// The banner reports the ACTIVE generation, not a fixed pin: the stable
    /// profile prints the released RM version.
    #[test]
    fn banner_follows_the_active_profile() {
        let b = render(
            "9.9.9",
            crate::config::profile::SpecProfile::Stable,
            &sandbox(),
            false,
        );
        assert!(b.contains("stable"));
        assert!(b.contains("1.1.0"), "stable profile serves RM 1.1.0");
    }

    #[test]
    fn banner_lines_are_within_100_chars() {
        for line in render(
            env!("CARGO_PKG_VERSION"),
            crate::config::profile::SpecProfile::default(),
            &sandbox(),
            false,
        )
        .lines()
        {
            assert!(
                line.chars().count() <= 100,
                "banner line exceeds 100 chars ({}): {line:?}",
                line.chars().count()
            );
        }
    }

    #[test]
    fn banner_uses_real_package_version() {
        assert!(
            render(
                env!("CARGO_PKG_VERSION"),
                crate::config::profile::SpecProfile::default(),
                &sandbox(),
                false,
            )
            .contains(env!("CARGO_PKG_VERSION"))
        );
    }
}
