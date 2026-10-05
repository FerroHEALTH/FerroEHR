// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Strict-validation helpers: the reserved-namespace env sweep, the
//! did-you-mean suggester, and its Damerau-Levenshtein metric. No openEHR spec
//! governs configuration — our own design.

use std::collections::HashMap;

use super::alias::{ALLOWLIST, SECTIONS};
use super::loader::ConfigError;

/// Sweep the reserved `FERROEHR_` namespace: every such variable must be an
/// allowlisted non-config name or a known-section uniform key — anything else
/// is a boot error (with a did-you-mean), which is what makes a
/// set-but-never-read variable impossible. Deeper key typos inside a known
/// section are caught at deserialize by `deny_unknown_fields`.
#[must_use]
pub(super) fn strict_env<S: std::hash::BuildHasher>(
    env: &HashMap<String, String, S>,
) -> Vec<ConfigError> {
    let mut errors = Vec::new();
    // Sorted: these errors are reported to the operator as a list, and
    // `HashMap` iteration order is unspecified — an unsorted sweep would
    // reorder the boot report run to run.
    let mut keys: Vec<&String> = env.keys().collect();
    keys.sort();
    for key in keys {
        if !key.starts_with("FERROEHR_") || ALLOWLIST.contains(&key.as_str()) {
            continue;
        }
        // Canonical uniform form: `FERROEHR__<SECTION>__<TAIL>` — every segment
        // boundary, including after the prefix, is `__`. The leading section
        // must be one of the known sections. `FERROEHR__SERVER__BIND` → `server`.
        if let Some(tail) = key.strip_prefix("FERROEHR__") {
            let section = tail
                .split("__")
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            if section.is_empty() || !SECTIONS.contains(&section.as_str()) {
                errors.push(ConfigError::unknown_env(
                    key,
                    did_you_mean(&section, SECTIONS),
                ));
            }
            continue;
        }
        // `FERROEHR_` but not `FERROEHR__`, and not allowlisted: a near-miss for
        // the uniform grammar, repaired mechanically — insert the missing prefix
        // separator, else double the leading word's boundary too, else fall back
        // to a section did-you-mean.
        let tail = key.strip_prefix("FERROEHR_").unwrap_or_default();
        let first = tail.split("__").next().unwrap_or_default();
        let section = first.to_ascii_lowercase();
        if SECTIONS.contains(&section.as_str()) {
            errors.push(ConfigError::near_miss_env(
                key,
                &format!("FERROEHR__{tail}"),
            ));
        } else if let Some(flat) = SECTIONS.iter().find_map(|s| {
            tail.strip_prefix(&format!("{}_", s.to_ascii_uppercase()))
                .map(|rest| format!("FERROEHR__{}__{rest}", s.to_ascii_uppercase()))
        }) {
            errors.push(ConfigError::near_miss_env(key, &flat));
        } else {
            errors.push(ConfigError::unknown_env(
                key,
                did_you_mean(&section, SECTIONS),
            ));
        }
    }
    // Deterministic order for reproducible error blocks + tests.
    errors.sort_by_key(ToString::to_string);
    errors
}

/// The closest candidate within edit distance 2 (ties broken lexicographically),
/// or `None` when nothing is close enough.
#[must_use]
pub(super) fn did_you_mean(unknown: &str, candidates: &[&str]) -> Option<String> {
    candidates
        .iter()
        .map(|c| (*c, damerau_levenshtein(unknown, c)))
        .filter(|(_, d)| *d <= 2)
        .min_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(b.0)))
        .map(|(c, _)| c.to_owned())
}

/// Optimal-string-alignment (restricted Damerau-Levenshtein) distance: single
/// insert/delete/substitute + adjacent transposition. ~byte-level; the config
/// keys are ASCII.
#[must_use]
#[expect(
    clippy::indexing_slicing,
    reason = "every index is bounded by the loop headers against vectors sized \
              from the same bounds: `i` runs 1..=n over `a` (len n) and `prev`/\
              `cur`/`prev2` (len m+1), `j` runs 1..=m over `b` (len m), and the \
              i-2/j-2 reads are guarded by the `i > 1 && j > 1` test"
)]
pub(super) fn damerau_levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<u8> = a.bytes().collect();
    let b: Vec<u8> = b.bytes().collect();
    let (n, m) = (a.len(), b.len());
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }
    let mut prev2 = vec![0usize; m + 1];
    let mut prev = (0..=m).collect::<Vec<_>>();
    let mut cur = vec![0usize; m + 1];
    for i in 1..=n {
        cur[0] = i;
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut val = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                val = val.min(prev2[j - 2] + 1);
            }
            cur[j] = val;
        }
        std::mem::swap(&mut prev2, &mut prev);
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[m]
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    #[test]
    fn distance_basics() {
        assert_eq!(damerau_levenshtein("signing", "signing"), 0);
        assert_eq!(damerau_levenshtein("signin", "signing"), 1);
        assert_eq!(damerau_levenshtein("signign", "signing"), 1); // transposition
        // "telemetry" (9) → "db" (2): 2 substitutions + 7 deletions = 9.
        assert_eq!(damerau_levenshtein("telemetry", "db"), 9);
    }

    #[test]
    fn suggestion_finds_close_section() {
        assert_eq!(did_you_mean("signin", SECTIONS).as_deref(), Some("signing"));
        assert_eq!(did_you_mean("aut", SECTIONS).as_deref(), Some("auth"));
        assert_eq!(did_you_mean("completely_bogus", SECTIONS), None);
    }

    #[test]
    fn sweep_flags_unknown_and_near_miss_and_passes_allowlisted() {
        let mut env = HashMap::new();
        env.insert("FERROEHR__SERVER__BIND".to_owned(), "0.0.0.0:9".to_owned());
        env.insert("FERROEHR__SIGNIN__ENABLED".to_owned(), "true".to_owned());
        // Single-separator near-misses are never aliased — both fail:
        env.insert("FERROEHR_SIGNING_CONFIG".to_owned(), "/x.toml".to_owned());
        env.insert("FERROEHR_DB_MAX_CONNECTIONS".to_owned(), "5".to_owned());
        env.insert("FERROEHR_CONFIG".to_owned(), "/e.toml".to_owned()); // allowlisted
        let errs = strict_env(&env);
        let joined = errs
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("FERROEHR__SIGNIN__ENABLED"), "{joined}");
        assert!(joined.contains("signing"), "did-you-mean: {joined}");
        assert!(joined.contains("FERROEHR_SIGNING_CONFIG"), "{joined}");
        assert!(joined.contains("FERROEHR_DB_MAX_CONNECTIONS"), "{joined}");
        assert_eq!(errs.len(), 3, "the typo + both near-misses: {joined}");
    }

    #[test]
    fn near_miss_prefix_is_flagged_with_uniform_spelling() {
        let mut env = HashMap::new();
        // Right section, old single-`_` prefix → near-miss.
        env.insert("FERROEHR_DB__URL".to_owned(), "postgres://x".to_owned());
        let errs = strict_env(&env);
        let joined = errs
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(errs.len(), 1, "{joined}");
        assert!(joined.contains("FERROEHR__DB__URL"), "suggestion: {joined}");
    }

    // ── The shipped Compose artifacts must pass this very sweep ───────────────
    //
    // A Compose file that spells a reserved-namespace variable any other way
    // produces a container that CANNOT BOOT, and the failure surfaces only at
    // `docker compose up`. This guard walks the committed YAML and runs the real
    // sweep over every variable the CDR service sets, so the drift fails in the
    // test suite instead. No openEHR spec governs configuration or Compose —
    // our own design.

    /// The compose service that runs the CDR binary this crate configures.
    /// Every other service (the database, the viewer with its own
    /// `FERROEHR_VIEWER__…` namespace, an upstream SUT) is unrelated.
    const CDR_SERVICE: &str = "ferroehr";

    /// The repository root — this crate lives at `app/ferroehr`.
    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn is_yaml(path: &Path) -> bool {
        matches!(
            Path::extension(path).and_then(std::ffi::OsStr::to_str),
            Some("yml" | "yaml")
        )
    }

    /// Whether the file is a Compose file at all (as opposed to a Prometheus /
    /// Grafana asset living beside one).
    fn declares_services(path: &Path) -> bool {
        std::fs::read_to_string(path)
            .is_ok_and(|yaml| yaml.lines().any(|line| line.trim_end() == "services:"))
    }

    /// Every committed Compose artifact: the root `docker-compose*.yml` files
    /// plus every `services:`-bearing YAML under `docker/`.
    fn compose_files() -> Vec<PathBuf> {
        let root = repo_root();
        let mut files = Vec::new();
        for entry in std::fs::read_dir(&root).expect("read repo root").flatten() {
            let path = entry.path();
            let is_root_compose = path
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|name| name.starts_with("docker-compose"));
            if is_root_compose && is_yaml(&path) {
                files.push(path);
            }
        }
        let mut dirs = vec![root.join("docker")];
        while let Some(dir) = dirs.pop() {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    dirs.push(path);
                } else if is_yaml(&path) && declares_services(&path) {
                    files.push(path);
                }
            }
        }
        files.sort();
        files
    }

    /// `(service, 1-based line, variable name)` for every `environment:` entry
    /// in a Compose file. Parsed by indentation, which is all this guard needs:
    /// these files are uniformly two-space indented, so a service name sits at
    /// indent 2, its `environment:` key at 4, and the entries below that — in
    /// either the mapping (`NAME: value`) or the list (`- NAME=value`) form
    /// (docs.docker.com/reference/compose-file/services/#environment).
    fn env_entries(yaml: &str) -> Vec<(String, usize, String)> {
        let mut out = Vec::new();
        let mut in_services = false;
        let mut service = String::new();
        let mut in_env = false;
        for (index, raw) in yaml.lines().enumerate() {
            let line = raw.trim_end();
            let body = line.trim_start();
            if body.is_empty() || body.starts_with('#') {
                continue;
            }
            match line.len() - body.len() {
                0 => {
                    in_services = body == "services:";
                    service.clear();
                    in_env = false;
                }
                2 => {
                    service = body.trim_end_matches(':').to_owned();
                    in_env = false;
                }
                4 => in_env = body == "environment:",
                _ => {
                    if in_services
                        && in_env
                        && !service.is_empty()
                        && let Some(name) = entry_name(body)
                    {
                        out.push((service.clone(), index + 1, name));
                    }
                }
            }
        }
        out
    }

    /// The variable name of one `environment:` entry, in either form.
    fn entry_name(entry: &str) -> Option<String> {
        let name = match entry.strip_prefix("- ") {
            // List form: `- NAME=value`, or a bare `- NAME` host pass-through.
            Some(item) => item.trim().split_once('=').map_or(item.trim(), |(n, _)| n),
            // Mapping form: `NAME: value`.
            None => entry.split_once(':')?.0,
        };
        let name = name.trim();
        if name.is_empty() {
            None
        } else {
            Some(name.to_owned())
        }
    }

    #[test]
    fn shipped_compose_env_passes_the_strict_sweep() {
        let files = compose_files();
        assert!(
            files.len() >= 2,
            "no Compose artifacts discovered — the guard is blind: {files:?}"
        );

        let mut cdr_entries = Vec::new();
        for path in &files {
            let yaml = std::fs::read_to_string(path).expect("read compose file");
            for (service, line, name) in env_entries(&yaml) {
                if service == CDR_SERVICE {
                    cdr_entries.push((path.clone(), line, name));
                }
            }
        }

        assert!(
            cdr_entries
                .iter()
                .any(|(_, _, name)| name == "FERROEHR__DB__URL"),
            "the CDR service's DSN variable was not extracted — the guard is \
             blind (Compose layout or service name drift): {cdr_entries:?}"
        );

        let mut failures = Vec::new();
        for (path, line, name) in &cdr_entries {
            let mut one = HashMap::new();
            one.insert(name.clone(), String::new());
            for error in strict_env(&one) {
                failures.push(format!("{}:{line}: {error}", path.display()));
            }
        }
        assert!(
            failures.is_empty(),
            "Compose sets variables the boot-time sweep rejects, so the \
             composed server cannot start:\n{}",
            failures.join("\n")
        );
    }
}
