// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The SPDX header every emitted file carries below its `@generated` banner
//! (REUSE Specification 3.3, <https://reuse.software/spec-3.3/>), so licensing
//! "is preserved when the file is copied and reused by third parties".

// This module writes SPDX tags, so REUSE must not read them as its own.
// REUSE-IgnoreStart

/// This project's copyright holder, as `REUSE.toml` spells it.
pub(crate) const PROJECT_COPYRIGHT: &str = "Cadasto B.V.";

/// The copyright holder of the openEHR material a spec crate carries.
pub(crate) const OPENEHR_COPYRIGHT: &str = "openEHR Foundation";

/// The generated crates whose files are Apache-2.0 and name the openEHR
/// Foundation as a second holder (owner decisions 2026-09-03 and 2026-09-24).
pub(crate) const DUAL_LICENSED_CRATES: &[&str] = &[
    "openehr-am",
    "openehr-base",
    "openehr-its",
    "openehr-lang",
    "openehr-rm",
    "openehr-term",
];

/// The tag prefix a copyright line carries.
const COPYRIGHT_TAG: &str = "// SPDX-FileCopyrightText: ";

/// The tag prefix the licence expression carries.
const LICENSE_TAG: &str = "// SPDX-License-Identifier: ";

/// How far into a file a licence identifier still counts as its header — a
/// generated banner plus a copyright line or two, never a mention in code.
const HEAD_SCAN_LINES: usize = 6;

/// Returns the SPDX comment header for a file belonging to `crate_name`.
///
/// The returned text is a whole number of lines, each newline-terminated.
pub(crate) fn header(crate_name: &str) -> String {
    if DUAL_LICENSED_CRATES.contains(&crate_name) {
        format!(
            "{COPYRIGHT_TAG}{PROJECT_COPYRIGHT}\n\
             {COPYRIGHT_TAG}{OPENEHR_COPYRIGHT}\n\
             {LICENSE_TAG}Apache-2.0\n"
        )
    } else {
        format!("{COPYRIGHT_TAG}{PROJECT_COPYRIGHT}\n{LICENSE_TAG}BUSL-1.1\n")
    }
}

/// Returns `body` with `crate_name`'s SPDX header below its banner, or at the
/// top without one; a body already carrying a licence is unchanged.
pub(crate) fn stamp(crate_name: &str, body: &str) -> String {
    if body
        .lines()
        .take(HEAD_SCAN_LINES)
        .any(|l| l.starts_with(LICENSE_TAG))
    {
        return body.to_owned();
    }
    let head = header(crate_name);
    match body.split_once('\n') {
        Some((first, rest)) if first.contains("@generated") => format!("{first}\n{head}{rest}"),
        _ => format!("{head}{body}"),
    }
}

/// Returns `text` without a leading SPDX header block.
pub(crate) fn strip_leading_header(text: &str) -> &str {
    let mut rest = text;
    let mut stripped = false;
    while rest.starts_with(COPYRIGHT_TAG) || rest.starts_with(LICENSE_TAG) {
        rest = rest.split_once('\n').map_or("", |(_, tail)| tail);
        stripped = true;
    }
    if stripped {
        rest.strip_prefix('\n').unwrap_or(rest)
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::{DUAL_LICENSED_CRATES, header, stamp, strip_leading_header};

    #[test]
    fn dual_crates_state_both_positions() {
        let h = header("openehr-rm");
        assert!(h.contains("SPDX-FileCopyrightText: Cadasto B.V."));
        assert!(h.contains("SPDX-FileCopyrightText: openEHR Foundation"));
        assert!(h.ends_with("SPDX-License-Identifier: Apache-2.0\n"));
    }

    #[test]
    fn other_crates_are_plain_busl() {
        assert_eq!(
            header("openehr-query"),
            "// SPDX-FileCopyrightText: Cadasto B.V.\n\
             // SPDX-License-Identifier: BUSL-1.1\n"
        );
    }

    #[test]
    fn the_dual_list_is_sorted_and_unique() {
        let mut sorted = DUAL_LICENSED_CRATES.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted, DUAL_LICENSED_CRATES);
    }

    #[test]
    fn the_header_lands_below_the_banner() {
        let out = stamp(
            "openehr-rm",
            "// @generated x — DO NOT EDIT.\n\npub mod a;\n",
        );
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "// @generated x — DO NOT EDIT.");
        assert_eq!(lines[1], "// SPDX-FileCopyrightText: Cadasto B.V.");
        assert_eq!(lines[3], "// SPDX-License-Identifier: Apache-2.0");
        assert_eq!(lines[4], "");
        assert_eq!(lines[5], "pub mod a;");
    }

    #[test]
    fn a_body_without_a_banner_takes_the_header_first() {
        let out = stamp("openehr-query", "pub mod a;\n");
        assert_eq!(
            out,
            "// SPDX-FileCopyrightText: Cadasto B.V.\n\
             // SPDX-License-Identifier: BUSL-1.1\n\
             pub mod a;\n"
        );
    }

    #[test]
    fn stamping_twice_changes_nothing() {
        let once = stamp("openehr-base", "// @generated x\n\npub mod a;\n");
        assert_eq!(stamp("openehr-base", &once), once);
    }

    #[test]
    fn a_leading_header_is_stripped_with_its_blank_line() {
        let text = "// SPDX-FileCopyrightText: Cadasto B.V.\n\
                    // SPDX-License-Identifier: Apache-2.0\n\
                    \n\
                    //! Docs.\n";
        assert_eq!(strip_leading_header(text), "//! Docs.\n");
    }

    #[test]
    fn stripping_leaves_a_headerless_body_alone() {
        assert_eq!(strip_leading_header("//! Docs.\n"), "//! Docs.\n");
    }
}

// REUSE-IgnoreEnd
