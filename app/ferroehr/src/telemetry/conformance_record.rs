// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The conformance profile `OPTIONS /` sends, derived from the released-profile
//! summary of a Veredictum run (`docs/conformance/ferroehr/summary.md`).
//!
//! The build script compiles this file through `#[path]` and the library
//! compiles it for its tests, so the derivation the server ships is the one
//! the tests pin. It reads the summary's `BASE` and `OPTIONS` rows, the two
//! parts `scripts/render/conformance-stats.sh` checks against the record.

/// The counts of one part of the summary table: `Passed`, `Failed`, `Warning`,
/// `Errored`, `Skipped` and `Total`, in the table's column order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Part {
    failed: u64,
    errored: u64,
    skipped: u64,
    total: u64,
}

impl Part {
    /// Whether the part ran at least one row and none of them failed or
    /// errored; a skipped row is one the run did not claim.
    fn holds(self) -> bool {
        self.failed == 0 && self.errored == 0 && self.total > self.skipped
    }
}

/// The profile a summary earns: `BASE+OPTIONS` when both parts hold, `BASE`
/// when only BASE holds, and none otherwise.
///
/// # Errors
///
/// A message naming what is missing or malformed when the summary has no
/// `## Summary` table carrying a `BASE` and an `OPTIONS` row of six counts.
pub(crate) fn profile(summary: &str) -> Result<Option<&'static str>, String> {
    let base = part(summary, "BASE")?;
    let options = part(summary, "OPTIONS")?;
    // NOTE: ITS-REST 1.1.0 `schemas/others/Options.yaml` types the member as a free
    // string and CNF names no profiles: no specification governs this: our own design.
    Ok(match (base.holds(), options.holds()) {
        (true, true) => Some("BASE+OPTIONS"),
        (true, false) => Some("BASE"),
        (false, _) => None,
    })
}

/// The counts of the row named `name` in the summary's `## Summary` table.
fn part(summary: &str, name: &str) -> Result<Part, String> {
    let row = summary
        .lines()
        .skip_while(|line| line.trim() != "## Summary")
        .skip(1)
        .take_while(|line| !line.starts_with("## "))
        .find_map(|line| {
            let mut cells = line.trim().strip_prefix('|')?.split('|').map(str::trim);
            (cells.next()? == name).then(|| cells.collect::<Vec<_>>())
        })
        .ok_or_else(|| format!("the summary table has no {name} row"))?;
    let counts: Vec<u64> = row
        .iter()
        .filter(|cell| !cell.is_empty())
        .map(|cell| {
            cell.parse::<u64>()
                .map_err(|e| format!("the {name} row holds `{cell}`, not a count: {e}"))
        })
        .collect::<Result<_, _>>()?;
    match counts.as_slice() {
        [_passed, failed, _warning, errored, skipped, total] => Ok(Part {
            failed: *failed,
            errored: *errored,
            skipped: *skipped,
            total: *total,
        }),
        _ => Err(format!(
            "the {name} row holds {} counts where the table has six",
            counts.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::profile;

    fn summary(base: &str, options: &str) -> String {
        format!(
            "# Veredictum results\n\n## Summary\n\n\
             | Part | Passed | Failed | Warning | Errored | Skipped | Total |\n\
             | --- | ---: | ---: | ---: | ---: | ---: | ---: |\n\
             | BASE | {base} |\n| OPTIONS | {options} |\n\n## BASE\n\n\
             | Band | Passed | Failed | Warning | Errored | Skipped |\n\
             | BASE | 1 | 1 | 0 | 0 | 0 |\n"
        )
    }

    #[test]
    fn both_parts_clean_earn_base_and_options() {
        let s = summary("10 | 0 | 2 | 0 | 0 | 10", "4 | 0 | 0 | 0 | 6 | 10");
        assert_eq!(profile(&s), Ok(Some("BASE+OPTIONS")));
    }

    #[test]
    fn a_failed_or_errored_option_row_leaves_base() {
        let failed = summary("10 | 0 | 0 | 0 | 0 | 10", "3 | 1 | 0 | 0 | 0 | 4");
        assert_eq!(profile(&failed), Ok(Some("BASE")));
        let errored = summary("10 | 0 | 0 | 0 | 0 | 10", "3 | 0 | 0 | 1 | 0 | 4");
        assert_eq!(profile(&errored), Ok(Some("BASE")));
        let unclaimed = summary("10 | 0 | 0 | 0 | 0 | 10", "0 | 0 | 0 | 0 | 4 | 4");
        assert_eq!(profile(&unclaimed), Ok(Some("BASE")));
    }

    #[test]
    fn a_failed_or_errored_base_row_leaves_the_member_out() {
        let failed = summary("9 | 1 | 0 | 0 | 0 | 10", "4 | 0 | 0 | 0 | 0 | 4");
        assert_eq!(profile(&failed), Ok(None));
        let errored = summary("9 | 0 | 0 | 1 | 0 | 10", "4 | 0 | 0 | 0 | 0 | 4");
        assert_eq!(profile(&errored), Ok(None));
    }

    #[test]
    fn a_summary_without_its_table_is_refused() {
        assert!(profile("# Veredictum results\n").is_err());
        assert!(profile(&summary("10 | 0 | 0 | 0 | 10", "4 | 0 | 0 | 0 | 0 | 4")).is_err());
        assert!(profile(&summary("10 | 0 | x | 0 | 0 | 10", "4 | 0 | 0 | 0 | 0 | 4")).is_err());
    }

    /// The committed record earns the profile the server sends.
    #[test]
    fn the_committed_record_earns_the_served_profile() {
        let committed = include_str!("../../../../docs/conformance/ferroehr/summary.md");
        assert_eq!(profile(committed), Ok(Some("BASE+OPTIONS")));
        assert_eq!(
            crate::telemetry::provenance::CONFORMANCE_PROFILE,
            Some("BASE+OPTIONS")
        );
    }
}
