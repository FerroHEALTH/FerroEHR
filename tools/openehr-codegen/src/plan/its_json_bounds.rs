// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The optional container attributes whose published ITS-JSON schema refuses
//! an empty array (`minItems: 1`) where the BMM lets the list be empty, and
//! the canonical-JSON reader decision they drive.
//!
//! The model keeps the BMM shape (`Option<Vec<T>>`, an empty list stays
//! representable); only the canonical-JSON reader refuses a present-but-empty
//! array, so the refusal is a parse failure the ITS-JSON schema shares
//! (ITS-REST `overview/Resources.md` §JSON Format: a canonical JSON payload
//! SHOULD be valid against the ITS-JSON schemas). The serializer already omits
//! an empty list, so a served document never carries one.
//!
//! `tests/emitter_invariants.rs` checks every entry against the loaded BMM
//! and the vendored ITS-JSON schema through
//! [`crate::testsupport::decision_checks`].

/// One optional container attribute the canonical-JSON reader refuses empty.
pub(crate) struct ItsJsonMinItems {
    /// The declaring BMM class; the entry reaches every descendant.
    pub owner: &'static str,
    /// The container attribute.
    pub field: &'static str,
    /// The ITS-JSON definitions that state the bound (the owner itself, or its
    /// concrete descendants where ITS-JSON defines no abstract class).
    pub its_json_classes: &'static [&'static str],
    /// The ITS-JSON schema statement the refusal realizes.
    pub citation: &'static str,
}

/// Every attribute whose present-but-empty canonical-JSON array is refused at
/// parse.
///
/// An attribute whose BMM already carries a present-implies-non-empty
/// invariant needs no entry: it emits `Option<NonEmptyVec<T>>`
/// (`analyze::nonempty_optional_lists`).
pub(crate) const ITS_JSON_MIN_ITEMS: &[ItsJsonMinItems] = &[
    ItsJsonMinItems {
        owner: "FOLDER",
        field: "folders",
        its_json_classes: &["FOLDER"],
        citation: "docs/specs/openehr/ITS-JSON/components/RM/Release-1.1.0/Common/FOLDER.json \
                   (and Release-1.0.3, Release-1.0.4): `folders` is an array with `minItems: 1`",
    },
    ItsJsonMinItems {
        owner: "HISTORY",
        field: "events",
        its_json_classes: &["HISTORY"],
        citation: "docs/specs/openehr/ITS-JSON/components/RM/Release-1.1.0/Data_structures/\
                   HISTORY.json (and Release-1.0.3, Release-1.0.4): `events` is an array with \
                   `minItems: 1`; the RM invariant `Events_valid` \
                   (org.openehr.rm.data_structures.history.adoc) admits an empty list beside a \
                   `summary`, which the schema does not",
    },
    ItsJsonMinItems {
        owner: "ACTOR",
        field: "languages",
        its_json_classes: &["AGENT", "GROUP", "ORGANISATION", "PERSON"],
        citation: "docs/specs/openehr/ITS-JSON/components/RM/Release-1.1.0/Demographic/\
                   {AGENT,GROUP,ORGANISATION,PERSON}.json (and Release-1.0.3, Release-1.0.4): \
                   `languages` is an array with `minItems: 1`",
    },
];

/// Whether the canonical-JSON reader refuses an empty `(owner, field)` array.
#[must_use]
pub(crate) fn refuses_empty(owner: &str, field: &str) -> bool {
    ITS_JSON_MIN_ITEMS
        .iter()
        .any(|e| e.owner == owner && e.field == field)
}
