// @generated-from-template templates/openehr-base/base_types/identification/lexical.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Lexical-form parsing shared by the BASE identification types.
//!
//! BASE `master05-identification_package.adoc`.

use crate::v1_2::base_types::identification::internet_id::InternetId;
use crate::v1_2::base_types::identification::iso_oid::IsoOid;
use crate::v1_2::base_types::identification::uid::Uid;
use crate::v1_2::base_types::identification::uuid::Uuid;

/// The identifier component a syntax failure is attributed to, named after the
/// master05 §Syntaxes productions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdComponent {
    /// The whole `value` string of the identifier.
    Value,
    /// `root` of a `uid_based_id` (`root, [ '::', extension ]`).
    Root,
    /// `object_id`, the first part of an `object_version_id`.
    ObjectId,
    /// `creating_system_id`, the second part of an `object_version_id`.
    CreatingSystemId,
}

impl std::fmt::Display for IdComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Value => "value",
            Self::Root => "root",
            Self::ObjectId => "object_id",
            Self::CreatingSystemId => "creating_system_id",
        })
    }
}

/// The grammar production a component was required to match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdProduction {
    /// `uid = iso_oid | uuid | internet_id`.
    Uid,
    /// `iso_oid = number, { '.', number }`.
    IsoOid,
    /// `uuid = hex-number, '-', … (five groups)`.
    Uuid,
    /// `internet_id = subdomain`.
    InternetId,
}

impl std::fmt::Display for IdProduction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Uid => "uid",
            Self::IsoOid => "iso_oid",
            Self::Uuid => "uuid",
            Self::InternetId => "internet_id",
        })
    }
}

/// Error raised when an identifier string does not conform to its openEHR
/// lexical form (BASE 1.3.0).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdError {
    /// A component did not match the grammar production its position requires.
    #[error("{component} {found:?} does not match the {expected} production")]
    Malformed {
        /// The identifier component at fault.
        component: IdComponent,
        /// The production that component was required to match.
        expected: IdProduction,
        /// The offending substring, verbatim (case-preserving).
        found: String,
    },
    /// The identifier string was empty (violates `UID.Value_valid` and the
    /// non-empty requirement of every identifier lexical form).
    #[error("empty identifier value")]
    Empty,
    /// A `::`/`.`-delimited component that must be present was empty.
    #[error("empty {0} component in identifier")]
    EmptyComponent(&'static str),
    /// The value had the wrong number of `::`-delimited parts for its type
    /// (e.g. an `OBJECT_VERSION_ID` without exactly three parts).
    #[error("expected {expected} '::'-delimited parts, found {found}")]
    PartCount {
        /// The number of parts the lexical form requires.
        expected: usize,
        /// The number of parts actually present.
        found: usize,
    },
    /// A `VERSION_TREE_ID` was neither a bare trunk (`N`) nor a full branch
    /// (`N.N.N`) with each segment a positive integer.
    #[error("malformed VERSION_TREE_ID: {0:?}")]
    VersionTree(String),
    /// An `ARCHETYPE_ID` did not match
    /// `rm_originator-rm_name-rm_entity.concept{-spec}*.vN`.
    #[error("malformed ARCHETYPE_ID: {0:?}")]
    Archetype(String),
}

/// Composite-identifier equality: "two identifiers identical apart from case
/// are considered to be identical" (master05 §"Composite Identifiers and
/// Case").
///
/// The fold is ASCII: §"Composite Identifiers and Language" restricts the
/// readable sections to basic latin, which keeps the Turkish `I/i` case out.
#[must_use]
pub fn composite_ids_equal(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// The case-folded key of a composite identifier, equal for exactly the pairs
/// [`composite_ids_equal`] matches; for lookup only, never stored.
#[must_use]
pub fn composite_id_key(id: &str) -> String {
    id.to_ascii_lowercase()
}

/// `true` for a non-empty string of ASCII digits.
#[must_use]
pub(crate) fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// A `number` (`digit, { digit }`) whose value is at least 1, leading zeros
/// allowed (`version_tree_id.adoc` §Invariants bounds the value).
#[must_use]
pub(crate) fn is_positive_int(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && s.bytes().any(|b| b != b'0')
}

/// `true` for the `iso_oid` production, `number, { '.', number }`; a single
/// group (`12345`) is an OID.
#[must_use]
pub fn is_iso_oid(s: &str) -> bool {
    s.split('.').all(all_digits)
}

/// `true` for the `uuid` production in its canonical `8-4-4-4-12` form.
///
/// The widths follow master05 §"Primitive Identifiers", which names the UUID;
/// the length check refuses the simple, braced and URN spellings
/// [`uuid::Uuid::try_parse`] also accepts.
#[must_use]
pub fn is_uuid(s: &str) -> bool {
    s.len() == uuid::fmt::Hyphenated::LENGTH && uuid::Uuid::try_parse(s).is_ok()
}

/// `true` for one `label` of the `internet_id` production:
/// `alphanum | alphanum-ext-str, alphanum`, with
/// `alphanum-ext-str = letter, { letter | digit | '_' | '-' }`.
fn is_internet_label(label: &str) -> bool {
    match label.as_bytes() {
        [] => false,
        [only] => only.is_ascii_alphanumeric(),
        [first, middle @ .., last] => {
            first.is_ascii_alphabetic()
                && last.is_ascii_alphanumeric()
                && middle
                    .iter()
                    .all(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-')
        }
    }
}

/// `true` for the `internet_id` production: `.`-separated labels.
#[must_use]
pub fn is_internet_id(s: &str) -> bool {
    s.split('.').all(is_internet_label)
}

/// `true` for the `uid` production, `uid = iso_oid | uuid | internet_id`.
#[must_use]
pub fn is_uid(s: &str) -> bool {
    is_iso_oid(s) || is_uuid(s) || is_internet_id(s)
}

/// Builds a concrete [`Uid`] from a root string, choosing the subtype by
/// lexical form, since a wire UID carries no `_type`.
///
/// The arms follow the §Syntaxes order `iso_oid | uuid | internet_id`, which
/// decides the two overlaps: `1.2` is an OID, `abcdf3f0-…` a UUID.
#[must_use]
pub(crate) fn make_uid(value: &str) -> Uid {
    // Through `is_uuid`, so the braced, URN and simple spellings are not
    // rewritten.
    if is_uuid(value)
        && let Ok(u) = value.parse::<uuid::Uuid>()
    {
        return Uid::Uuid(Uuid { value: u });
    }
    // NOTE: master05 §Syntaxes order `uid = iso_oid | uuid | internet_id`.
    if is_iso_oid(value) {
        return Uid::IsoOid(IsoOid {
            value: value.to_owned(),
        });
    }
    Uid::InternetId(InternetId {
        value: value.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_uid_picks_subtype() {
        assert!(matches!(
            make_uid("2fdbf3f0-1c0a-4a0e-9f2a-3b7f6b1e9c11"),
            Uid::Uuid(_)
        ));
        assert!(matches!(make_uid("1.2.840.113554"), Uid::IsoOid(_)));
        assert!(matches!(make_uid("openehr.org"), Uid::InternetId(_)));
        // `12345` is a legal OID and not a legal `internet_id` label.
        assert!(matches!(make_uid("12345"), Uid::IsoOid(_)));
    }

    /// The pathological shapes where the §Syntaxes productions overlap or
    /// nearly do — each classified by the grammar, not by a heuristic.
    #[test]
    fn make_uid_pathological_shapes() {
        // Single-group OIDs of every length.
        for s in ["0", "5", "12345", "99999999999999999999"] {
            assert!(matches!(make_uid(s), Uid::IsoOid(_)), "{s}");
            assert!(is_iso_oid(s), "{s}");
        }
        // `iso_oid` ∩ `internet_id`: every group one digit — both productions
        // accept, the alternation order gives it to `iso_oid`.
        for s in ["1.2", "0.0.0", "5.5.5.5"] {
            assert!(is_iso_oid(s), "{s}");
            assert!(is_internet_id(s), "{s}");
            assert!(matches!(make_uid(s), Uid::IsoOid(_)), "{s}");
        }
        // `uuid` ∩ `internet_id`: a canonical UUID starting with a letter is
        // also a legal single label — the alternation order gives it to `uuid`.
        let hexish = "abcdf3f0-1c0a-4a0e-9f2a-3b7f6b1e9c11";
        assert!(is_uuid(hexish));
        assert!(is_internet_id(hexish));
        assert!(matches!(make_uid(hexish), Uid::Uuid(_)));
        // A one-character digit label IS legal (`alphanum`), so a mixed dotted
        // form whose every group is a legal label is an internet id.
        for s in ["1.2.a3", "a.1.b2"] {
            assert!(is_internet_id(s), "{s}");
            assert!(matches!(make_uid(s), Uid::InternetId(_)), "{s}");
        }
        // Digit-leading multi-character labels are in neither production.
        for s in ["1a", "12a.34", "1.23b", ""] {
            assert!(!is_uid(s), "{s}");
        }
        // A dotted form with an empty group is no production's shape.
        for s in ["1..2", ".1", "1.", "a..b"] {
            assert!(!is_uid(s), "{s}");
        }
    }

    /// Identifiers differing only in case are equal, pairwise and by key.
    #[test]
    fn composite_id_case_rule() {
        for (a, b) in [
            ("openEHR.org", "OPENEHR.ORG"),
            ("uk.nhs.ehr1", "UK.NHS.EHR1"),
            ("FerroEHR.local", "ferroehr.local"),
            ("sys", "SYS"),
            (
                "87284370-2D4B-4E3D-A3F3-F303D2F4F34B",
                "87284370-2d4b-4e3d-a3f3-f303d2f4f34b",
            ),
        ] {
            assert!(composite_ids_equal(a, b), "{a} vs {b}");
            assert_eq!(composite_id_key(a), composite_id_key(b));
        }
        assert!(!composite_ids_equal("system.a", "system.b"));
        assert_ne!(composite_id_key("system.a"), composite_id_key("system.b"));
        // Case-preserving: neither function rewrites its input.
        let original = "openEHR.org";
        assert_eq!(original, "openEHR.org");
        assert_eq!(composite_id_key(original), "openehr.org");
    }

    /// The three `uid` productions of BASE `master05` §Syntaxes, each accepted
    /// exactly where the grammar accepts it.
    #[test]
    fn uid_productions() {
        // iso_oid = number, { '.', number } — one or more digit groups.
        assert!(is_iso_oid("12345"));
        assert!(is_iso_oid("1.2.840.113554"));
        assert!(is_iso_oid("0.0"));
        assert!(!is_iso_oid(""));
        assert!(!is_iso_oid("1."));
        assert!(!is_iso_oid(".1"));
        assert!(!is_iso_oid("1.2a"));

        // uuid = the canonical 8-4-4-4-12 hex form, and nothing else.
        assert!(is_uuid("87284370-2D4B-4e3d-A3F3-F303D2F4F34B"));
        assert!(is_uuid("2fdbf3f0-1c0a-4a0e-9f2a-3b7f6b1e9c11"));
        assert!(!is_uuid("1-2-3-4-5"));
        assert!(!is_uuid("87284370-2D4B-4e3d-A3F3-F303D2F4F34"));
        assert!(!is_uuid("87284370-2D4B-4e3d-A3F3-F303D2F4F34BB"));
        assert!(!is_uuid("872843702D4B4e3dA3F3F303D2F4F34B"));
        assert!(!is_uuid("87284370-2D4B-4e3d-A3F3-F303D2F4F34G"));
        assert!(!is_uuid("{87284370-2D4B-4e3d-A3F3-F303D2F4F34B}"));
        assert!(!is_uuid("1-2-3-4"));
        assert!(!is_uuid("1-2-3-4-5-6"));

        // internet_id = subdomain of labels.
        assert!(is_internet_id("openehr.org"));
        assert!(is_internet_id("uk.nhs.ehr1"));
        assert!(is_internet_id("a"));
        assert!(is_internet_id("7"));
        assert!(is_internet_id("my_system-1.example"));
        assert!(!is_internet_id(""));
        assert!(!is_internet_id("1234-5678"));
        assert!(!is_internet_id("-leading"));
        assert!(!is_internet_id("trailing-"));
        assert!(!is_internet_id("has space"));
        assert!(!is_internet_id("a..b"));

        // uid is the union of the three.
        assert!(is_uid("12345"));
        assert!(is_uid("87284370-2D4B-4e3d-A3F3-F303D2F4F34B"));
        assert!(is_uid("openEHR.org"));
        assert!(!is_uid(""));
        assert!(!is_uid("1-2-3-4-5"));
        assert!(!is_uid("1234-5678"));
        assert!(!is_uid("a::b"));
        assert!(!is_uid("système"));
    }

    #[test]
    fn positive_int_rules() {
        assert!(is_positive_int("1"));
        assert!(is_positive_int("42"));
        // A leading zero is a legal `number` whose value is still >= 1.
        assert!(is_positive_int("01"));
        assert!(is_positive_int("0000000009"));
        // Value < 1, whatever the spelling.
        assert!(!is_positive_int("0"));
        assert!(!is_positive_int("000"));
        // Not a `number` at all.
        assert!(!is_positive_int(""));
        assert!(!is_positive_int("1a"));
        assert!(!is_positive_int("-1"));
    }
}
