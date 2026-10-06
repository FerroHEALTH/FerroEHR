// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Capture build-time provenance for `/management/info` and the
//! `ferroehr_build_info` gauge: the git commit, the build timestamp, and the
//! `rustc` version. All are best-effort — a checkout without git, or a build
//! from a tarball, degrades to `unknown` rather than failing the build.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

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

    println!("cargo:rerun-if-env-changed=REVISION");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs");
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
