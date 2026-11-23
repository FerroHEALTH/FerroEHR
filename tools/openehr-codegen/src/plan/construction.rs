// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The construction-door decision map: which generated spec classes hide
//! their fields behind a validating constructor, and which stay plain records.
//!
//! A class is [`Door::Validated`] when the released spec states a constraint
//! decidable from its own fields: a lexical form (BASE
//! `base_types/master05-identification_package.adoc` §Syntaxes) or an own-field
//! invariant (RM `org.openehr.rm.common.item_tag.adoc` §Invariants). Its
//! fields are `pub(crate)`, and every codec constructs through the
//! hand-written `*_impl.rs` constructor.

/// How a generated spec class is constructed.
pub(crate) enum Door {
    /// Fields are `pub(crate)`; the sibling `*_impl.rs` owns a validating `new`,
    /// through which the codecs construct.
    Validated {
        /// The constructor's parameters as `(field, type)` pairs in call order,
        /// keyed by field name; a spec-typed parameter is the marker `@SPEC_CLASS`,
        /// resolved per generation.
        params: &'static [(&'static str, &'static str)],
        /// Whether `new` returns `Result`; `false` when the field types already
        /// make an invalid value unrepresentable.
        fallible: bool,
        /// The released grammar or invariant the constructor runs.
        citation: &'static str,
    },
    /// Fields stay `pub`: the released spec states **no** constraint over this
    /// class's field values, so a construction door would validate nothing.
    PlainRecord {
        /// The released text that says so.
        citation: &'static str,
    },
    /// The class has a released lexical form, enforced at the
    /// `openehr_base::validate::Validate` tier rather than at construction, since
    /// a parse refusal moves the accept/reject boundary.
    TierEnforced {
        /// The released text that both defines the grammar and motivates the
        /// tier split.
        citation: &'static str,
    },
}

/// One class's construction decision.
pub(crate) struct Construction {
    /// The BMM class name.
    pub class: &'static str,
    /// The decision.
    pub door: Door,
}

/// Every adjudicated construction decision: the identification family for
/// lexical forms, `ITEM_TAG` for its invariants.
pub(crate) static CONSTRUCTION: &[Construction] = &[
    // ── the UID hierarchy (uid = iso_oid | uuid | internet_id) ──────────────
    Construction {
        class: "UUID",
        door: Door::Validated {
            params: &[("value", "uuid::Uuid")],
            fallible: false,
            citation: "docs/specs/openehr/BASE/docs/base_types/\
                       master05-identification_package.adoc \u{a7}Syntaxes: `uuid = hex-number, \
                       '-', hex-number, '-', hex-number, '-', hex-number, '-', hex-number`. The \
                       field carries the pinned `uuid` crate's RFC-4122 type (the settled \
                       strong-typing override), so parsing IS validation and the constructor is \
                       total \u{2014} but the door is still the only way in, so the value can \
                       never be replaced by an unparsed one after the fact.",
        },
    },
    Construction {
        class: "ISO_OID",
        door: Door::Validated {
            params: &[("value", "String")],
            fallible: true,
            citation: "docs/specs/openehr/BASE/docs/base_types/\
                       master05-identification_package.adoc \u{a7}Syntaxes: \
                       `iso_oid = number, { '.', number }`.",
        },
    },
    Construction {
        class: "INTERNET_ID",
        door: Door::Validated {
            params: &[("value", "String")],
            fallible: true,
            citation: "docs/specs/openehr/BASE/docs/base_types/\
                       master05-identification_package.adoc \u{a7}Syntaxes: \
                       `internet_id = subdomain`, `subdomain = label | subdomain, '.', label`.",
        },
    },
    // ── the UID_BASED_ID family (root, [ '::', extension ]) ─────────────────
    Construction {
        class: "HIER_OBJECT_ID",
        door: Door::Validated {
            params: &[("value", "String")],
            fallible: true,
            citation: "docs/specs/openehr/BASE/docs/base_types/\
                       master05-identification_package.adoc \u{a7}Syntaxes: \
                       `hier_object_id = uid_based_id`, \
                       `uid_based_id = root, [ '::', extension ]`, `root = uid`.",
        },
    },
    Construction {
        class: "OBJECT_VERSION_ID",
        door: Door::Validated {
            params: &[("value", "String")],
            fallible: true,
            citation: "docs/specs/openehr/BASE/docs/base_types/\
                       master05-identification_package.adoc \u{a7}Syntaxes: \
                       `object_version_id = object_id, '::', creating_system_id, '::', \
                       version_tree_id` with `object_id = uid` and `creating_system_id = uid`.",
        },
    },
    Construction {
        class: "VERSION_TREE_ID",
        door: Door::Validated {
            params: &[("value", "String")],
            fallible: true,
            citation: "docs/specs/openehr/BASE/docs/base_types/\
                       master05-identification_package.adoc \u{a7}Syntaxes: \
                       `version_tree_id = trunk_version, [ '.', branch_number, '.', \
                       branch_version ]`, each part a `number`; RM common \
                       master06-change_control_package.adoc \u{a7}\u{201c}The 'Virtual Version \
                       Tree'\u{201d} starts every part at 1 (`VERSION_TREE_ID.\
                       Trunk_version_valid` / `.Branch_validity`).",
        },
    },
    // ── the invariant half: a class constrained by its OWN §Invariants ──────
    Construction {
        class: "ITEM_TAG",
        door: Door::Validated {
            params: &[
                ("key", "String"),
                ("value", "Option<String>"),
                ("target", "@UID_BASED_ID"),
                ("target_path", "Option<String>"),
                ("owner_id", "@OBJECT_REF"),
            ],
            fallible: true,
            citation: "docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.item_tag.adoc \
                       \u{a7}Invariants: `Inv_key_valid: not key.is_empty and key.is_justified` \
                       and `Inv_value_valid: value /= Void implies not value.is_empty`. Both are \
                       stated over ITEM_TAG's own fields and decidable from them alone, so an \
                       instance violating either is not an instance of the class \u{2014} the \
                       same standing the \u{a7}Syntaxes grammar has for an identifier. The \
                       remaining checks an ITEM_TAG needs (does `target` exist, does \
                       `target_path` resolve within it) read state the instance does NOT carry \
                       and stay in the service layer.",
        },
    },
    // ── recorded NON-entries: identifier classes that stay plain records ────
    Construction {
        class: "TEMPLATE_ID",
        door: Door::PlainRecord {
            citation: "docs/specs/openehr/BASE/docs/base_types/\
                       master05-identification_package.adoc \u{a7}Template Identifiers: \
                       \u{201c}a legacy free-text template name remains a valid `TEMPLATE_ID`; \
                       the HRID form cannot be enforced\u{201d} \u{2014} every string is a \
                       valid value, so there is nothing a construction door could check.",
        },
    },
    Construction {
        class: "GENERIC_ID",
        door: Door::PlainRecord {
            citation: "docs/specs/openehr/BASE/docs/UML/classes/\
                       org.openehr.base.base_types.generic_id.adoc: \u{201c}Generic identifier \
                       type for identifiers whose format is otherwise unknown to \
                       openEHR\u{201d}; master05 \u{a7}\u{201c}Generic and External \
                       Identifiers\u{201d}: \u{201c}The names of schemes are not currently \
                       controlled.\u{201d} Both the value and the scheme are unconstrained by \
                       the release.",
        },
    },
    Construction {
        class: "ARCHETYPE_ID",
        door: Door::TierEnforced {
            citation: "docs/specs/openehr/BASE/docs/base_types/\
                       master05-identification_package.adoc \u{a7}Syntaxes gives the \
                       `archetype_id` production (the legacy ADL 1.4 form and, from BASE \
                       1.3.0, the namespaced ADL 2 form), but \u{a7}\u{201c}Archetype \
                       Identifiers\u{201d} carries a \
                       WARNING that \u{201c}some archetype authoring tools have historically \
                       allowed a nonconforming version part \u{2026} of the form `.v1draft` or \
                       similar\u{201d} \u{2014} i.e. the release itself states that \
                       nonconforming instances are in circulation. Refusing them at PARSE \
                       rather than at validation is an accept/reject boundary change requiring \
                       its own adjudication; the grammar is enforced today by \
                       `impl Validate for ArchetypeId`.",
        },
    },
    Construction {
        class: "TERMINOLOGY_ID",
        door: Door::TierEnforced {
            citation: "docs/specs/openehr/BASE/docs/base_types/\
                       master05-identification_package.adoc \u{a7}Syntaxes gives \
                       `terminology_id = name-str, [ '(', name-str, ')' ]` with \
                       `name-str = letter, { letter | digit | '_' | '-' | '/' | '+' }`, which \
                       the same chapter's own examples contradict \
                       (\u{a7}\u{201c}Terminology Identifiers\u{201d}: `\"ICD10AM(3rd_ed)\"` \
                       \u{2014} the version part starts with a digit, which `name-str` forbids). \
                       A parse-time refusal would reject the release's own examples, so the \
                       grammar stays at the `Validate` tier pending adjudication.",
        },
    },
];

/// The construction decision recorded for `class`, or `None` (the default:
/// a plain record with public fields).
pub(crate) fn door(class: &str) -> Option<&'static Door> {
    CONSTRUCTION
        .iter()
        .find(|c| c.class == class)
        .map(|c| &c.door)
}

/// Does `class` construct through a validating door (fields `pub(crate)`,
/// codecs routed through `new`)?
#[must_use]
pub(crate) fn is_validated(class: &str) -> bool {
    matches!(door(class), Some(Door::Validated { .. }))
}

/// The declared `(parameter types, fallible)` of a validated `class`'s
/// constructor, or `None` when the class has no validating door.
#[must_use]
pub(crate) fn validated_ctor(
    class: &str,
) -> Option<(&'static [(&'static str, &'static str)], bool)> {
    match door(class) {
        Some(Door::Validated {
            params, fallible, ..
        }) => Some((*params, *fallible)),
        _ => None,
    }
}

/// The spec citation the emitter writes above a validated class's fields, so
/// the reason the fields are not `pub` is readable where they are.
#[must_use]
pub(crate) fn validated_citation(class: &str) -> Option<&'static str> {
    match door(class) {
        Some(Door::Validated { citation, .. }) => Some(citation),
        _ => None,
    }
}

/// The adjudication emitted into a class that stays a plain record, or `None`.
#[must_use]
pub(crate) fn plain_record_note(class: &str) -> Option<String> {
    match door(class)? {
        Door::Validated { .. } => None,
        Door::PlainRecord { citation } => Some(format!(
            "the fields stay public deliberately: the release states no constraint over \
             this class's field values, so a validating construction door would check \
             nothing \u{2014} {citation}"
        )),
        Door::TierEnforced { citation } => Some(format!(
            "the fields stay public deliberately: this class HAS a released lexical form, \
             but it is enforced at the `openehr_base::validate::Validate` tier rather than \
             at construction \u{2014} moving it to the door would turn a validation verdict \
             into a parse refusal, which needs its own adjudication \u{2014} {citation}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{CONSTRUCTION, Door, door, is_validated, validated_ctor};

    /// Every entry names a distinct class (a duplicate would make `door`'s
    /// first-match silently authoritative).
    #[test]
    fn entries_are_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for c in CONSTRUCTION {
            assert!(seen.insert(c.class), "duplicate entry for {}", c.class);
        }
    }

    /// Every citation names a vendored spec path — the map is the decision
    /// record, and a decision without its released source is not one.
    #[test]
    fn every_decision_cites_the_release() {
        for c in CONSTRUCTION {
            let citation = match &c.door {
                Door::Validated { citation, .. }
                | Door::PlainRecord { citation }
                | Door::TierEnforced { citation } => *citation,
            };
            assert!(
                citation.contains("docs/specs/openehr/"),
                "{} cites no vendored spec text",
                c.class
            );
        }
    }

    /// The identification family's validated set is exactly the classes whose
    /// §Syntaxes production is total and uncontradicted.
    #[test]
    fn validated_set_is_the_scoped_family() {
        for class in [
            "UUID",
            "ISO_OID",
            "INTERNET_ID",
            "HIER_OBJECT_ID",
            "OBJECT_VERSION_ID",
            "VERSION_TREE_ID",
        ] {
            assert!(is_validated(class), "{class} must construct through a door");
            let (params, _) = validated_ctor(class).expect("a validated class has a constructor");
            assert_eq!(params.len(), 1, "{class}");
        }
        for class in [
            "TEMPLATE_ID",
            "GENERIC_ID",
            "ARCHETYPE_ID",
            "TERMINOLOGY_ID",
        ] {
            assert!(!is_validated(class), "{class} stays a plain record");
            assert!(door(class).is_some(), "{class} must be a RECORDED decision");
        }
        // A class the map says nothing about defaults to a plain record.
        assert!(door("DV_TEXT").is_none());
        assert!(!is_validated("DV_TEXT"));
    }
}
