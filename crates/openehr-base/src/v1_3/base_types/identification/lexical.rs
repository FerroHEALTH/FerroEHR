// @generated-from-template templates/openehr-base/base_types/identification/lexical.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written lexical-form parsing shared by the BASE identification types
//! (hand-written spec behaviour; auto-declared beside the `// @generated` files).
//!
//! openEHR BASE 1.3.0 defines each identifier class by a *lexical form* (a
//! grammar over the `value` string) plus accessor functions that decompose it
//! (`UID_BASED_ID.root`/`extension`, `OBJECT_VERSION_ID.object_id`/…,
//! `VERSION_TREE_ID.trunk_version`/…, `ARCHETYPE_ID.rm_originator`/…,
//! `TERMINOLOGY_ID.name`/…). The generator emits only the `{ value: String }`
//! struct; the accessors and a fallible strict parser live here and in the
//! sibling `*_impl.rs` files. This module holds the pieces multiple types share
//! (the error type + the `UID` subtype builder + digit predicates + the
//! composite-identifier case rule).
//!
//! Spec: `docs/specs/openehr/BASE/docs/UML/classes/org.openehr.base.base_types.*`
//! and `docs/specs/openehr/BASE/docs/base_types/master05-identification_package.adoc`.

use crate::v1_3::base_types::identification::internet_id::InternetId;
use crate::v1_3::base_types::identification::iso_oid::IsoOid;
use crate::v1_3::base_types::identification::uid::Uid;
use crate::v1_3::base_types::identification::uuid::Uuid;

/// The identifier component a syntax failure is attributed to — the *field*
/// half of [`IdError::Malformed`].
///
/// Named after the productions of BASE `base_types`
/// `master05-identification_package.adoc` §Syntaxes, so an error says which
/// part of the identifier is wrong rather than only that something is.
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

/// The grammar production a component was required to match — the *expected*
/// half of [`IdError::Malformed`] (BASE `base_types`
/// `master05-identification_package.adoc` §Syntaxes).
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
/// lexical form (BASE 1.3.0). Returned by the `FromStr`/`TryFrom<&str>`
/// implementations on the identification value types.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdError {
    /// A component of the identifier did not match the grammar production its
    /// position requires (BASE `base_types`
    /// `master05-identification_package.adoc` §Syntaxes).
    ///
    /// Carried as data — which component, what was expected, what was found —
    /// so a caller can branch on the failure instead of matching on prose.
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

/// Composite-identifier equality under the openEHR case rule.
///
/// `true` iff `a` and `b` are the same identifier — BASE `base_types`
/// `master05-identification_package.adoc` §"Composite Identifiers and Case":
/// "two identifiers identical apart from case are considered to be identical,
/// and therefore to identify the same thing".
///
/// This is the ONE comparison every composite identifier goes through —
/// `UID_BASED_ID` values ([`super::uid_based_id::UidBasedId::is_equal`]),
/// archetype and template ids, `INTERNET_ID` system ids. The sibling rule of
/// the same section, case-**preserving** ("not change case due to persistence,
/// copying, transfer or other computation processes"), belongs to whoever
/// stores the value: nothing here rewrites a stored string, only the
/// *comparison* folds case.
///
/// The fold is ASCII, which is exactly the section's intent: §"Composite
/// Identifiers and Language" restricts the human-readable identifier sections
/// to the basic latin character set, and §"Composite Identifiers and Case"
/// explicitly carves out languages where case does not exist (the Turkish
/// `I/i` caveat) — a Unicode-locale fold would *re-introduce* that hazard, so
/// [`str::eq_ignore_ascii_case`] is the correct, locale-safe choice.
#[must_use]
pub fn composite_ids_equal(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// The comparison/keying form of a composite identifier.
///
/// The value with ASCII case folded away, so that two identifiers are the same
/// identifier exactly when their keys are equal (BASE `base_types`
/// `master05-identification_package.adoc` §"Composite Identifiers and Case" —
/// the same rule [`composite_ids_equal`] decides pairwise).
///
/// For a caller that needs a *key* rather than a comparison — a hash-map entry,
/// a cache key, a SQL `lower()` predicate — this is the single derivation, so a
/// keyed lookup can never disagree with a pairwise comparison. It is
/// case-**preserving** in the spec's sense: the derived key is for lookup only,
/// never a replacement for the stored value.
#[must_use]
pub fn composite_id_key(id: &str) -> String {
    id.to_ascii_lowercase()
}

/// `true` for a non-empty string of ASCII digits.
#[must_use]
pub(crate) fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// A `number` (master05 §Syntaxes: `digit, { digit }`) whose VALUE is at least
/// 1, as the `VERSION_TREE_ID` trunk/branch segments require.
///
/// The bound is on the value, not the spelling: `version_tree_id.adoc`
/// §Invariants gives `Trunk_version_valid: … trunk_version.as_integer >= 1`,
/// and the production admits a leading zero. Refusing `"01"` lexically was a
/// prohibition neither states, and since `VersionTreeId::new` is the only
/// construction door it made a foreign system's zero-padded version id
/// impossible to accept or round-trip.
///
/// "At least 1" is read as "not every digit is zero" rather than by parsing,
/// so an id longer than any integer type still answers correctly.
#[must_use]
pub(crate) fn is_positive_int(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && s.bytes().any(|b| b != b'0')
}

/// `true` for the `iso_oid` production.
///
/// BASE `base_types` `master05-identification_package.adoc` §Syntaxes:
/// `iso_oid = number, { '.', number }` — one or more `.`-separated non-empty
/// runs of ASCII digits.
///
/// Note the "one or more": the grammar admits a single group (`12345`), and the
/// subtype *dispatch* (`make_uid`) uses exactly this predicate, so a one-group
/// OID classifies `ISO_OID` rather than falling through to `INTERNET_ID` — whose
/// own production it would violate (`12345` is not a legal `label`: a
/// multi-character label is `alphanum-ext-str, alphanum` and
/// `alphanum-ext-str` must start with a *letter*).
#[must_use]
pub fn is_iso_oid(s: &str) -> bool {
    s.split('.').all(all_digits)
}

/// `true` for the `uuid` production.
///
/// BASE `base_types` `master05-identification_package.adoc` §Syntaxes:
/// `uuid = hex-number, '-', hex-number, '-', hex-number, '-', hex-number, '-',
/// hex-number` — five `-`-separated runs of hex digits, in the `8-4-4-4-12`
/// widths of the UUID the production names.
///
/// The EBNF writes the five groups as bare `hex-number`s and does not spell the
/// widths out, but the identified thing is a UUID: master05 §"Primitive
/// Identifiers" defines the subtype as the commonly accepted UUID ("also
/// commonly known as GUIDs"), and every UUID the spec shows is the canonical
/// `8-4-4-4-12` form (§"Identifying Versions within openEHR Versioned
/// Containers": `87284370-2D4B-4e3d-A3F3-F303D2F4F34B`). The generated
/// [`Uuid`] carries an RFC 4122 `uuid::Uuid` (the settled strong-typing
/// override), whose `Display` is exactly this form — so anything else could not
/// be stored as a `UUID` in the first place, and a five-group hex string with
/// other widths (`1-2-3-4-5`) is not a UUID and is not accepted as one here.
///
/// The UUID grammar itself is not re-implemented: it is delegated to the
/// pinned [`uuid`] crate — the same parser the strong-typed `UUID.value` field
/// is built from, so the predicate and the stored type can never disagree. The
/// one thing added on top is the length check, because
/// [`uuid::Uuid::try_parse`] also accepts the simple (`8f2c…`), braced
/// (`{8f2c…}`) and URN (`urn:uuid:8f2c…`) spellings
/// (<https://docs.rs/uuid/1/uuid/struct.Uuid.html#method.try_parse>), none of
/// which is a `uuid` in the §Syntaxes sense.
#[must_use]
pub fn is_uuid(s: &str) -> bool {
    s.len() == uuid::fmt::Hyphenated::LENGTH && uuid::Uuid::try_parse(s).is_ok()
}

/// `true` for one `label` of the `internet_id` production.
///
/// BASE `base_types` `master05-identification_package.adoc` §Syntaxes:
/// `label = alphanum | alphanum-ext-str, alphanum` with
/// `alphanum-ext-str = letter, { letter | digit | '_' | '-' }` — i.e. a single
/// letter-or-digit, or a run that starts with a letter, ends with a
/// letter-or-digit, and uses only letters, digits, `_` and `-` in between.
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

/// `true` for the `internet_id` production.
///
/// BASE `base_types` `master05-identification_package.adoc` §Syntaxes:
/// `internet_id = subdomain`, `subdomain = label | subdomain, '.', label` —
/// one or more `.`-separated labels (`is_internet_label`).
#[must_use]
pub fn is_internet_id(s: &str) -> bool {
    s.split('.').all(is_internet_label)
}

/// `true` for the `uid` production of BASE `base_types`
/// `master05-identification_package.adoc` §Syntaxes:
/// `uid = iso_oid | uuid | internet_id`.
///
/// This is the predicate every validating identifier constructor runs on a
/// `root` / `object_id` / `creating_system_id` position, so an identifier whose
/// UID part is not a legal UID cannot be constructed. It accepts exactly what
/// the three productions accept — no more (the strictness rule) and no less
/// (an invented prohibition is the same defect).
#[must_use]
pub fn is_uid(s: &str) -> bool {
    is_iso_oid(s) || is_uuid(s) || is_internet_id(s)
}

/// Builds a concrete [`Uid`] from a root/identifier string.
///
/// The subtype is chosen by lexical form (BASE 1.3.0 `UID` hierarchy): a valid
/// RFC-4122 UUID becomes [`Uuid`]; an OID (dot-separated groups of digits, at
/// least two groups) becomes [`IsoOid`]; anything else becomes [`InternetId`].
/// Inference is forced by the wire form: a UID is carried as a bare string
/// with no `_type`, while `UID` is abstract with three concrete descendants
/// (`…org.openehr.base.base_types.uid.adoc` §Inherit), so the lexical form is
/// the only available discriminator.
///
/// Dispatch follows the §Syntaxes alternation order itself —
/// `uid = iso_oid | uuid | internet_id` — which is load-bearing only where two
/// productions overlap, and both overlaps resolve the way the grammar lists
/// them. An all-digit dotted string whose every group is ONE digit (`5`, `1.2`)
/// satisfies `iso_oid` and `internet_id` alike, because a lone digit is a legal
/// `label` via the `alphanum` alternative, and `iso_oid` wins; a canonical UUID
/// whose first character is a letter (`abcdf3f0-…`) is also a legal single
/// `label`, and `uuid` wins. `iso_oid` ∩ `uuid` is empty — a `uuid` requires
/// `-`, an `iso_oid` admits only digits and `.` — so the UUID arm may come
/// first.
#[must_use]
pub(crate) fn make_uid(value: &str) -> Uid {
    // The `uuid` arm goes through [`is_uuid`] rather than a bare
    // `parse::<uuid::Uuid>()`, so classification and validity answer from ONE
    // grammar: the bare parse also accepts the braced/URN/simple spellings,
    // which are not `uuid` lexical forms and whose acceptance here would
    // silently rewrite the identifier's text on the way through `Uid::value`.
    if is_uuid(value)
        && let Ok(u) = value.parse::<uuid::Uuid>()
    {
        return Uid::Uuid(Uuid { value: u });
    }
    // NOTE: the arms follow the §Syntaxes alternation order
    // (`uid = iso_oid | uuid | internet_id`), which decides the two
    // overlapping cases this function's doc comment enumerates.
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
        // `iso_oid = number, { '.', number }` — one group is a legal OID, and
        // `12345` is NOT a legal `internet_id` label (a multi-character label
        // must begin with a letter), so it can only be an ISO OID.
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
        // Multi-character digit-leading labels are legal in NEITHER the
        // `internet_id` nor (with a non-digit present) the `iso_oid`
        // production, so they are not UIDs at all.
        for s in ["1a", "12a.34", "1.23b", ""] {
            assert!(!is_uid(s), "{s}");
        }
        // A dotted form with an empty group is no production's shape.
        for s in ["1..2", ".1", "1.", "a..b"] {
            assert!(!is_uid(s), "{s}");
        }
    }

    /// BASE `master05` §"Composite Identifiers and Case": two identifiers
    /// identical apart from case identify the same thing, and the pairwise
    /// comparison agrees with the derived key.
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
        // The bound is on the VALUE, not the spelling: `number` is
        // `digit, { digit }`, so a leading zero is a legal `number` whose value
        // is still >= 1.
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
