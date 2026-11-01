// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Capture build-time provenance for `/management/info` and the
//! `ferroehr_build_info` gauge: the git commit, the version the server
//! reports, the build timestamp, the `rustc` version, and the release date the support period runs from. All are best-effort — a checkout without git, or a build
//! from a tarball, degrades to `unknown` rather than failing the build.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "src/telemetry/conformance_record.rs"]
mod conformance_record;

#[expect(
    clippy::disallowed_methods,
    reason = "a build script's ONLY input channel is the process environment \
              Cargo populates for it (Cargo book, build-scripts §Inputs), so the \
              config tree cannot apply here; `SystemTime` is likewise the right \
              clock for a build-time epoch — jiff is a runtime dependency, not a \
              build-dependency"
)]
fn main() {
    // Git short SHA: the OCI-standard REVISION value (the same one that fills
    // org.opencontainers.image.revision) wins; otherwise ask git. Degrades to
    // `unknown` rather than failing the build.
    let revision = std::env::var("REVISION")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            Command::new("git")
                .args(["rev-parse", "--short=12", "HEAD"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        })
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=REVISION={revision}");

    // The version the server reports: the package version for a release build,
    // which the release lanes mark by naming the tag they build in
    // FERROEHR_RELEASE_TAG, and the package version with the commit as SemVer
    // build metadata for every other build, so a report from a build of `main`
    // never reads as the release it follows (SemVer 2.0.0 §10).
    let package = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let release = std::env::var("FERROEHR_RELEASE_TAG")
        .is_ok_and(|tag| tag.strip_prefix('v') == Some(package.as_str()));
    let server_version = if release {
        package
    } else {
        format!("{package}+dev.{revision}")
    };
    println!("cargo:rustc-env=FERROEHR_SERVER_VERSION={server_version}");

    // Build timestamp (epoch seconds), honouring SOURCE_DATE_EPOCH for
    // reproducible builds; rendered to an ISO-8601 string at runtime (jiff).
    let epoch = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .and_then(|d| i64::try_from(d.as_secs()).ok())
                .unwrap_or(0)
        });
    println!("cargo:rustc-env=FERROEHR_BUILD_EPOCH={epoch}");

    // rustc version string.
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    let rustc_version = Command::new(&rustc)
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map_or_else(
            || "unknown".to_owned(),
            |o| String::from_utf8_lossy(&o.stdout).trim().to_owned(),
        );
    println!("cargo:rustc-env=FERROEHR_RUSTC={rustc_version}");

    // The `openehr-*` crates this crate links, with the versions the workspace
    // lock file resolved; `ferroehr report` names them. A build without the
    // lock file records none rather than failing.
    let openehr_crates = std::fs::read_to_string("../../Cargo.lock")
        .map_or_else(|_| String::new(), |lock| linked_openehr_crates(&lock));
    println!("cargo:rustc-env=FERROEHR_OPENEHR_CRATES={openehr_crates}");
    println!("cargo:rerun-if-changed=../../Cargo.lock");

    // Where those versions come from. A release lane stamps the lockstep
    // version it publishes to crates.io into the checkout and names it in
    // FERROEHR_OPENEHR_CRATE_VERSION; every other build carries the source
    // tree's manifests, a floor below the published line. A named version the
    // lock does not hold stops the build, so a report never claims a stamp
    // that did not happen.
    let stamped = std::env::var("FERROEHR_OPENEHR_CRATE_VERSION").unwrap_or_default();
    let origin = match crate_origin(&openehr_crates, &stamped) {
        Ok(origin) => origin,
        Err(message) => {
            println!("cargo::error={message}");
            "source_tree"
        }
    };
    println!("cargo:rustc-env=FERROEHR_OPENEHR_CRATES_ORIGIN={origin}");
    println!("cargo:rerun-if-env-changed=FERROEHR_OPENEHR_CRATE_VERSION");

    // The release date of this package version: the date of its
    // `## [X.Y.Z] - YYYY-MM-DD` heading in the changelog, from which the support
    // period runs (Regulation (EU) 2024/2847 Art. 13(8)). Empty when the
    // version has no dated heading, which marks a build that is no release.
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let release_date = std::fs::read_to_string("../../CHANGELOG.md")
        .ok()
        .and_then(|changelog| release_date(&changelog, &version))
        .unwrap_or_default();
    println!("cargo:rustc-env=FERROEHR_RELEASE_DATE={release_date}");
    println!("cargo:rerun-if-changed=../../CHANGELOG.md");

    // The conformance profile `OPTIONS /` sends, derived from the committed
    // released-profile summary; empty leaves the member out. A build without
    // the summary claims none, and a summary that does not parse stops it.
    let summary = "../../docs/conformance/ferroehr/summary.md";
    let profile = match std::fs::read_to_string(summary) {
        Ok(text) => match conformance_record::profile(&text) {
            Ok(profile) => profile.unwrap_or_default(),
            Err(message) => {
                println!("cargo::error={summary}: {message}");
                ""
            }
        },
        Err(_) => "",
    };
    println!("cargo:rustc-env=FERROEHR_CONFORMANCE_PROFILE={profile}");
    println!("cargo:rerun-if-changed={summary}");

    println!("cargo:rerun-if-env-changed=REVISION");
    println!("cargo:rerun-if-env-changed=FERROEHR_RELEASE_TAG");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs");
    // `sqlx::migrate!` cannot see a newly added file on stable, and the
    // directives above switch off Cargo's rerun-on-any-change, so the
    // embedded migration sets follow the directory here
    // (https://docs.rs/sqlx/latest/sqlx/macro.migrate.html).
    println!("cargo:rerun-if-changed=migrations");
}

/// The date of the `## [<version>] - YYYY-MM-DD` heading in a Keep a Changelog
/// file, when the heading exists and its date has that shape.
fn release_date(changelog: &str, version: &str) -> Option<String> {
    let heading = format!("## [{version}] - ");
    let date: String = changelog
        .lines()
        .find_map(|line| line.strip_prefix(&heading))?
        .chars()
        .take(10)
        .collect();
    let mut parts = date.split('-');
    let shaped = matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(year), Some(month), Some(day), None)
            if year.len() == 4
                && month.len() == 2
                && day.len() == 2
                && date.chars().all(|c| c.is_ascii_digit() || c == '-')
                && month.parse::<u8>().is_ok_and(|m| (1..=12).contains(&m))
                && day.parse::<u8>().is_ok_and(|d| (1..=31).contains(&d))
    );
    shaped.then_some(date)
}

/// The origin of the recorded `openehr-*` versions: `published` when the
/// release lane named the version it stamped and every linked crate is at it,
/// `source_tree` when it named none.
///
/// # Errors
///
/// The message for a named version that the lock does not record for every
/// linked crate, or a lock that records no linked crate at all.
fn crate_origin(linked: &str, stamped: &str) -> Result<&'static str, String> {
    if stamped.is_empty() {
        return Ok("source_tree");
    }
    if linked.is_empty() {
        return Err(format!(
            "FERROEHR_OPENEHR_CRATE_VERSION names {stamped}, but the build read no openehr-* crate from Cargo.lock"
        ));
    }
    let off: Vec<&str> = linked
        .split(',')
        .filter(|entry| {
            entry
                .split_once('=')
                .is_none_or(|(_, version)| version != stamped)
        })
        .collect();
    if off.is_empty() {
        Ok("published")
    } else {
        Err(format!(
            "FERROEHR_OPENEHR_CRATE_VERSION names {stamped}, but Cargo.lock records {}; run scripts/release/publish-crates.sh stamp {stamped} first",
            off.join(", ")
        ))
    }
}

/// The `openehr-*` dependencies of the `ferroehr` package in a `Cargo.lock`,
/// as `name=version` pairs joined by commas, sorted.
///
/// The lock format lists each package as a `[[package]]` table with `name`,
/// `version` and a `dependencies` array whose entries are `"name"` or
/// `"name version"` (<https://doc.rust-lang.org/cargo/guide/cargo-toml-vs-cargo-lock.html>).
fn linked_openehr_crates(lock: &str) -> String {
    let packages: Vec<&str> = lock.split("[[package]]").collect();
    let field = |block: &str, key: &str| {
        block
            .lines()
            .find_map(|line| line.strip_prefix(key))
            .map(|value| value.trim().trim_matches('"').to_owned())
    };
    let Some(ferroehr) = packages
        .iter()
        .find(|block| field(block, "name = ").as_deref() == Some("ferroehr"))
    else {
        return String::new();
    };
    let mut linked: Vec<String> = ferroehr
        .lines()
        .skip_while(|line| !line.starts_with("dependencies = ["))
        .skip(1)
        .take_while(|line| !line.starts_with(']'))
        .filter_map(|line| {
            let entry = line.trim().trim_end_matches(',').trim_matches('"');
            let name = entry.split(' ').next()?;
            name.starts_with("openehr-").then(|| name.to_owned())
        })
        .filter_map(|name| {
            let version = packages
                .iter()
                .find(|block| field(block, "name = ").as_deref() == Some(name.as_str()))
                .and_then(|block| field(block, "version = "))?;
            Some(format!("{name}={version}"))
        })
        .collect();
    linked.sort();
    linked.join(",")
}
