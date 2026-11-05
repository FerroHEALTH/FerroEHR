// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Reads the examples of the vendored SCIM documents by line range.
//!
//! The fixtures of the SCIM protocol suite are the published bytes, read from
//! `docs/specs/` when a test runs: nothing is transcribed into this tree. An
//! RFC is redistributed under the IETF Trust Legal Provisions §3.c, "in full
//! and without modification" (`docs/specs/rfc/PROVENANCE.md`), so a corrected
//! copy of an example would be a modified excerpt; the corrections live here
//! instead, each a [`Fix`] that names what the published text gets wrong.
//!
//! An RFC is laid out for print: each page ends with a footer line
//! (`Hunt, et al. ...`), a form feed and a header line (`RFC 7644 ...`), and a
//! string longer than the line is wrapped onto the next one. [`json`] drops the
//! page furniture and joins a wrapped string with one space, the reading RFC
//! 7643 §8.7 asks for ("the JSON representation has been modified for
//! readability and to fit the specification format").

use std::ops::RangeInclusive;
use std::path::PathBuf;

use serde_json::Value;

/// A vendored document an example is read from.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Doc {
    /// RFC 7643, SCIM Core Schema.
    Rfc7643,
    /// RFC 7644, SCIM Protocol.
    Rfc7644,
    /// RFC 9110, HTTP Semantics.
    Rfc9110,
    /// Microsoft's "Develop a SCIM endpoint for user provisioning to apps from
    /// Microsoft Entra ID".
    EntraEndpoint,
    /// Microsoft's "Customize Microsoft Entra attribute mappings in Application
    /// Provisioning".
    EntraMappings,
}

impl Doc {
    fn path(self) -> PathBuf {
        let relative = match self {
            Self::Rfc7643 => "docs/specs/rfc/rfc7643.txt",
            Self::Rfc7644 => "docs/specs/rfc/rfc7644.txt",
            Self::Rfc9110 => "docs/specs/rfc/rfc9110.txt",
            Self::EntraEndpoint => {
                "docs/specs/microsoft-entra/docs/identity/app-provisioning/use-scim-to-provision-users-and-groups.md"
            }
            Self::EntraMappings => {
                "docs/specs/microsoft-entra/docs/identity/app-provisioning/customize-application-attributes.md"
            }
        };
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(relative)
    }

    fn is_rfc(self) -> bool {
        matches!(self, Self::Rfc7643 | Self::Rfc7644 | Self::Rfc9110)
    }
}

/// A correction of a published example that is not valid JSON as printed.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Fix {
    /// Drops each line that holds only an elision (`...`), which the RFCs mark
    /// as "Some text was removed for readability (indicated by "...")".
    Elisions,
    /// Drops a comma that the elided text, or the example itself, leaves in
    /// front of a closing `}` or `]`.
    TrailingCommas,
    /// Replaces the one occurrence of the first text with the second; the
    /// reader refuses a fix whose text does not occur exactly once.
    Replace(&'static str, &'static str),
}

/// Returns the lines `range` (1-based, inclusive) of `doc`, as printed.
pub(crate) fn lines(doc: Doc, range: RangeInclusive<usize>) -> Vec<String> {
    let text =
        std::fs::read_to_string(doc.path()).expect("the vendored document should be readable");
    let all: Vec<&str> = text.split('\n').collect();
    all[range.start() - 1..*range.end()]
        .iter()
        .map(|line| (*line).to_owned())
        .collect()
}

/// Returns the line `number` (1-based) of `doc`, with its indentation trimmed.
pub(crate) fn line(doc: Doc, number: usize) -> String {
    lines(doc, number..=number)[0].trim().to_owned()
}

/// Returns the text of lines `range` of `doc` with the page furniture dropped
/// and every wrapped string joined.
pub(crate) fn joined(doc: Doc, range: RangeInclusive<usize>) -> String {
    let lines = lines(doc, range);
    let kept = lines
        .iter()
        .filter(|line| !(doc.is_rfc() && is_page_furniture(line)));
    join_wrapped_strings(kept)
}

/// Returns the JSON value printed at lines `range` of `doc`, after `fixes`.
pub(crate) fn json(doc: Doc, range: RangeInclusive<usize>, fixes: &[Fix]) -> Value {
    let mut text = joined(doc, range.clone());
    for fix in fixes {
        text = apply(*fix, &text);
    }
    serde_json::from_str(&text).unwrap_or_else(|error| {
        panic!("{doc:?} lines {range:?} should be JSON after {fixes:?}: {error}\n{text}")
    })
}

fn is_page_furniture(line: &str) -> bool {
    line.starts_with('\u{c}')
        || line.starts_with("Hunt, et al.")
        || line.starts_with("Fielding, et al.")
        || line.starts_with("RFC 7643 ")
        || line.starts_with("RFC 7644 ")
        || line.starts_with("RFC 9110 ")
}

fn join_wrapped_strings<'a>(lines: impl Iterator<Item = &'a String>) -> String {
    let mut out = String::new();
    let mut in_string = false;
    let mut escaped = false;
    for line in lines {
        let text = if in_string {
            out.push(' ');
            line.trim_start()
        } else {
            line.as_str()
        };
        for c in text.chars() {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' && in_string {
                escaped = true;
            } else if c == '"' {
                in_string = !in_string;
            }
        }
        if !in_string {
            out.push('\n');
        }
    }
    out
}

fn apply(fix: Fix, text: &str) -> String {
    match fix {
        Fix::Elisions => text
            .split('\n')
            .filter(|line| !line.trim_start().starts_with("..."))
            .collect::<Vec<_>>()
            .join("\n"),
        Fix::TrailingCommas => drop_trailing_commas(text),
        Fix::Replace(from, to) => {
            assert_eq!(
                text.matches(from).count(),
                1,
                "the fix text {from:?} should occur exactly once in\n{text}"
            );
            text.replacen(from, to, 1)
        }
    }
}

fn drop_trailing_commas(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut in_string = false;
    let mut escaped = false;
    for (i, c) in chars.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if *c == '\\' {
                escaped = true;
            } else if *c == '"' {
                in_string = false;
            }
        } else if *c == '"' {
            in_string = true;
        } else if *c == ',' {
            let next = chars[i + 1..].iter().find(|n| !n.is_whitespace());
            if matches!(next, Some('}' | ']')) {
                continue;
            }
        }
        out.push(*c);
    }
    out
}
