// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Specialisation-code utilities over the AOM2 `ADL_CODE_DEFINITIONS` class.
//!
//! The code space itself — the leaders, `codes_conformant`, `is_valid_code`,
//! `is_redefined_code` — is the spec class, realized on
//! `openehr_am::v2_4::aom2::definitions::adl_code_definitions::AdlCodeDefinitionsData`
//! and read through from here. This module carries only the ADL engine's own
//! reading of a code: the parsed segment form, specialisation lineage, and the
//! root-code tests the validity catalogue needs.
//!
//! Spec oracle:
//! - `docs/specs/openehr/AM/docs/UML/classes/org.openehr.am.aom2.adl_code_definitions.adoc`
//!   — the leader constants, `Code_regex_pattern`, and
//!   `Root_code_regex_pattern = "^(id1|at0000)(\.1)*$"`.
//! - `docs/specs/openehr/AM/docs/AOM2/master07-terminology_package.adoc`
//!   §Specialisation Depth — depth = dot-count for at/id codes; ac-codes
//!   "exist in a flat code space instead" (no depth semantics).
//!
//! NOTE: `specialisation_depth_from_code` and `specialisation_parent_from_code`
//! are referenced by the spec but carry no formal function body in the vendored
//! AM text — they are derived here from the master07 §Specialisation Depth
//! prose (dot-count).

use std::sync::LazyLock;

use openehr_am::v2_4::aom2::definitions::adl_code_definitions::AdlCodeDefinitionsData;
use regex::Regex;

/// The kind of ADL local code, distinguished by its alphabetic leader
/// (`org.openehr.am.aom2.adl_code_definitions` §Constants).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CodePrefix {
    /// `id`-code — an id-coded object-node identifier (`Id_code_leader`).
    Id,
    /// `at`-code — an at-coded node identifier or a value/term code
    /// (`At_code_leader` == `Value_code_leader` == `"at"`).
    At,
    /// `ac`-code — a value-set (constraint) code (`Value_set_code_leader`).
    /// ac-codes live in a flat code space (master07 §Specialisation Depth).
    Ac,
}

impl CodePrefix {
    /// The literal leader string (`"id"`, `"at"`, `"ac"`).
    #[must_use]
    pub fn leader(self) -> &'static str {
        match self {
            Self::Id => AdlCodeDefinitionsData::ID_CODE_LEADER,
            Self::At => AdlCodeDefinitionsData::AT_CODE_LEADER,
            Self::Ac => AdlCodeDefinitionsData::VALUE_SET_CODE_LEADER,
        }
    }
}

/// A parsed ADL local code.
///
/// Carries its [`CodePrefix`] and the ordered `.`-separated numeric segments
/// (verbatim strings, so zero-padding survives round-trips — `at0004.0.1` →
/// prefix `At`, segments `["0004", "0", "1"]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCode {
    /// The alphabetic leader.
    pub prefix: CodePrefix,
    /// The `.`-separated numeric segments, in source order (never empty for a
    /// valid code).
    pub segments: Vec<String>,
}

impl ParsedCode {
    /// The specialisation depth of this code = number of specialisation
    /// separators, i.e. `segments.len() - 1`
    /// (master07 §Specialisation Depth: `at0004` depth 0, `at0004.1` depth 1,
    /// `at0004.0.1` depth 2).
    ///
    /// ac-codes are a flat code space and have no depth semantics; this returns
    /// their raw dot-count all the same (ac-codes carry no dots in practice).
    #[must_use]
    pub fn specialisation_depth(&self) -> usize {
        self.segments.len().saturating_sub(1)
    }
}

/// `Root_code_regex_pattern` verbatim: `^(id1|at0000)(\.1)*$`
/// (`org.openehr.am.aom2.adl_code_definitions` §Constants).
static ROOT_CODE_RE: LazyLock<Regex> = LazyLock::new(|| {
    #[expect(
        clippy::unwrap_used,
        reason = "the pattern is a compile-time string constant proven to compile by this module's own tests, so an Err is unreachable"
    )]
    Regex::new(r"^(id1|at0000)(\.1)*$").unwrap()
});

/// The [`CodePrefix`] of a raw code string, or `None` if it carries no
/// recognised leader.
///
/// A dispatch over the class's own leader predicates (`is_id_code` /
/// `is_value_set_code` / `is_at_code`, `adl_code_definitions` §Functions).
#[must_use]
pub fn code_prefix(code: &str) -> Option<CodePrefix> {
    if AdlCodeDefinitionsData::is_id_code(code) {
        Some(CodePrefix::Id)
    } else if AdlCodeDefinitionsData::is_value_set_code(code) {
        Some(CodePrefix::Ac)
    } else if AdlCodeDefinitionsData::is_at_code(code) {
        Some(CodePrefix::At)
    } else {
        None
    }
}

/// Parse a raw local code into its [`ParsedCode`] form, or `None` if it is not
/// a valid ADL code.
#[must_use]
pub fn parse_code(code: &str) -> Option<ParsedCode> {
    if !AdlCodeDefinitionsData::is_valid_code(code) {
        return None;
    }
    let prefix = code_prefix(code)?;
    let numeric = code.get(prefix.leader().len()..)?;
    let segments = numeric.split('.').map(str::to_owned).collect();
    Some(ParsedCode { prefix, segments })
}

/// The specialisation depth of a code (= dot-count of its numeric part), or
/// `None` for a non-code string (master07 §Specialisation Depth).
///
/// ac-codes are flat (no depth); callers that care must gate on the code's
/// [`CodePrefix`].
#[must_use]
pub fn specialisation_depth(code: &str) -> Option<usize> {
    parse_code(code).map(|c| c.specialisation_depth())
}

/// The immediate specialisation parent of a code — the code with its trailing
/// `.N` segment removed (master07 §Specialisation Depth: the parent of
/// `at0025.1.1` is `at0025.1`).
///
/// Returns `None` for a level-0 code (no separators) or a non-code string.
///
/// NOTE: no formal `specialisation_parent_from_code` is vendored; this is
/// derived from the master07 dot-structure prose. The intervening `.0` filler
/// (`at0004.0.1`) is preserved literally — one specialisation level is removed
/// per call.
#[must_use]
pub fn specialisation_parent_from_code(code: &str) -> Option<String> {
    let parsed = parse_code(code)?;
    if parsed.segments.len() <= 1 {
        return None;
    }
    let keep = parsed.segments.len() - 1;
    let mut out = String::from(parsed.prefix.leader());
    out.push_str(&parsed.segments.get(..keep)?.join("."));
    Some(out)
}

/// True if `code` is a well-formed archetype **root** code — `at0000{.1}*` or
/// `id1{.1}*` (`Root_code_regex_pattern`, `adl_code_definitions` §Constants).
#[must_use]
pub fn is_root_code(code: &str) -> bool {
    ROOT_CODE_RE.is_match(code)
}

/// True if `code` is a valid archetype root code at the given specialisation
/// depth.
///
/// A root code qualifies when its number of `.1` segments equals `depth`
/// (AOM2 master03 VARCN: root `node_id` of the form `at0000{.1}*` / `id1{.1}*`
/// where the number of `.1` components equals the specialisation depth).
#[must_use]
pub fn is_root_code_at_depth(code: &str, depth: usize) -> bool {
    is_root_code(code) && specialisation_depth(code) == Some(depth)
}

/// True if `code` is a *new* node code introduced at its own specialisation
/// level.
///
/// A new code is not a redefinition of a parent code — i.e. every segment above
/// the last is a `0` filler (`at0.0.1`, `at0004.0.1`'s new-node cousin
/// `at0.0.1`) (master07 §Specialisation Depth; master09.05 `at0.{0.}*N` new-node
/// form). A level-0 code is never "new at level" in this sense.
///
/// NOTE: no formal `is_added_code` is vendored (only `is_redefined_code`); this
/// is its complement for a specialised code, derived from the master07/09.05
/// dot-structure prose.
#[must_use]
pub fn is_new_at_level(code: &str) -> bool {
    let Some(parsed) = parse_code(code) else {
        return false;
    };
    if parsed.segments.len() <= 1 {
        return false;
    }
    // First numeric segment is a pure-zero leader (`at0…` / `id0…`) — a code
    // that specialises no parent term.
    parsed
        .segments
        .first()
        .is_some_and(|seg| seg.bytes().all(|b| b == b'0'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_and_validity() {
        assert_eq!(code_prefix("id1"), Some(CodePrefix::Id));
        assert_eq!(code_prefix("at0004"), Some(CodePrefix::At));
        assert_eq!(code_prefix("ac1"), Some(CodePrefix::Ac));
        assert_eq!(code_prefix("XYZ"), None);
        assert!(AdlCodeDefinitionsData::is_valid_code("at0000"));
        assert!(AdlCodeDefinitionsData::is_valid_code("at0004.0.1"));
        assert!(AdlCodeDefinitionsData::is_valid_code("id1.1.1"));
        assert!(AdlCodeDefinitionsData::is_valid_code("ac1"));
        assert!(!AdlCodeDefinitionsData::is_valid_code("at"));
        assert!(!AdlCodeDefinitionsData::is_valid_code("at.1"));
        assert!(!AdlCodeDefinitionsData::is_valid_code("foo"));
    }

    #[test]
    fn depth_is_dot_count() {
        assert_eq!(specialisation_depth("at0000"), Some(0));
        assert_eq!(specialisation_depth("at0004.1"), Some(1));
        assert_eq!(specialisation_depth("at0004.0.1"), Some(2));
        assert_eq!(specialisation_depth("id1.1.1"), Some(3 - 1));
        assert_eq!(specialisation_depth("ac1"), Some(0));
        assert_eq!(specialisation_depth("nope"), None);
    }

    #[test]
    fn root_codes() {
        assert!(is_root_code("at0000"));
        assert!(is_root_code("id1"));
        assert!(is_root_code("at0000.1.1"));
        assert!(is_root_code("id1.1"));
        assert!(!is_root_code("at0001"));
        assert!(!is_root_code("id2"));
        assert!(!is_root_code("at0000.2"));
        assert!(is_root_code_at_depth("id1.1", 1));
        assert!(!is_root_code_at_depth("id1.1", 0));
        assert!(is_root_code_at_depth("at0000", 0));
    }

    #[test]
    fn conformance_and_parent() {
        // exact Eiffel algorithm: separator-boundary guard.
        assert!(AdlCodeDefinitionsData::codes_conformant("at0004", "at0004"));
        assert!(AdlCodeDefinitionsData::codes_conformant(
            "at0004.1", "at0004"
        ));
        assert!(AdlCodeDefinitionsData::codes_conformant(
            "at0004.1.2",
            "at0004.1"
        ));
        // no separator boundary.
        assert!(!AdlCodeDefinitionsData::codes_conformant(
            "at00040", "at0004"
        ));
        assert!(!AdlCodeDefinitionsData::codes_conformant(
            "at0005", "at0004"
        ));
        assert_eq!(
            specialisation_parent_from_code("at0025.1.1").as_deref(),
            Some("at0025.1")
        );
        assert_eq!(
            specialisation_parent_from_code("at0004.0.1").as_deref(),
            Some("at0004.0")
        );
        assert_eq!(specialisation_parent_from_code("id1"), None);
    }

    #[test]
    fn redefined_vs_new() {
        assert!(!AdlCodeDefinitionsData::is_redefined_code("at0.0.1"));
        assert!(AdlCodeDefinitionsData::is_redefined_code("at1.0.1"));
        assert!(AdlCodeDefinitionsData::is_redefined_code("at0004.1"));
        assert!(!AdlCodeDefinitionsData::is_redefined_code("at0004"));
        assert!(is_new_at_level("at0.0.1"));
        assert!(!is_new_at_level("at0004.1"));
        assert!(!is_new_at_level("id1"));
    }

    // ── property tests (depth / lineage math) ─────────────────────────────

    /// A deterministic pseudo-random valid code: `prefix` + `seg_count`
    /// `.`-separated numeric segments (the first at-segment zero-padded to 4
    /// digits, matching the openEHR convention).
    fn gen_code(prefix: &str, seed: u64, seg_count: usize) -> String {
        use std::fmt::Write as _;
        let mut s = String::from(prefix);
        let mut x = seed;
        for i in 0..seg_count {
            x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            if i > 0 {
                s.push('.');
            }
            if i == 0 && prefix == "at" {
                let _ = write!(s, "{:04}", x % 9999);
            } else {
                let _ = write!(s, "{}", x % 12);
            }
        }
        s
    }

    #[test]
    fn prop_depth_equals_dot_count() {
        for seed in 0..500u64 {
            for segs in 1..6usize {
                let code = gen_code(if seed % 2 == 0 { "id" } else { "at" }, seed, segs);
                if let Some(d) = specialisation_depth(&code) {
                    assert_eq!(
                        d,
                        code.matches('.').count(),
                        "depth must equal dot count for {code}"
                    );
                }
            }
        }
    }

    #[test]
    fn prop_conformant_is_reflexive_and_prefix_monotone() {
        for seed in 0..500u64 {
            let base = format!("id{}", (seed % 30) + 1);
            assert!(
                AdlCodeDefinitionsData::codes_conformant(&base, &base),
                "reflexive: {base}"
            );
            let child = format!("{base}.1");
            // a `.1`-extended child always conforms to its parent.
            assert!(
                AdlCodeDefinitionsData::codes_conformant(&child, &base),
                "child {child} conforms to {base}"
            );
            // and the parent never conforms to the strictly-deeper child.
            assert!(!AdlCodeDefinitionsData::codes_conformant(&base, &child));
        }
    }

    #[test]
    fn prop_parent_reduces_depth_by_one() {
        for seed in 0..500u64 {
            for segs in 2..6usize {
                let code = gen_code("id", seed, segs);
                if let (Some(d), Some(parent)) = (
                    specialisation_depth(&code),
                    specialisation_parent_from_code(&code),
                ) {
                    assert_eq!(specialisation_depth(&parent), Some(d - 1));
                    assert!(AdlCodeDefinitionsData::codes_conformant(&code, &parent));
                }
            }
        }
    }
}
