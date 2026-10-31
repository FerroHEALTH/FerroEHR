// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `Hash<K, V>` attributes whose entry order the vendored text fixes, and
//! the field-type decision they drive.
//!
//! `tests/emitter_invariants.rs` checks every entry against the loaded schemas
//! through [`crate::testsupport::decision_checks`].

use crate::analyze::Model;
use crate::load::bmm::BmmClass;

/// One `Hash<K, V>` attribute whose entry order a spec sentence fixes.
pub(crate) struct OrderedHash {
    /// The declaring BMM class; the entry reaches every descendant.
    pub owner: &'static str,
    /// The `Hash`-typed attribute.
    pub field: &'static str,
    /// The spec text that fixes the order.
    pub citation: &'static str,
    /// One-line reason.
    pub reason: &'static str,
}

/// The `Hash<K, V>` attributes emitted as an insertion-ordered
/// `indexmap::IndexMap<K, V>` instead of the key-sorted `BTreeMap<K, V>`.
///
/// `Hash` is "a table of values of any type V, keyed by values of any `Ordered`
/// descendant K" (BASE `foundation_types/master04-structure_types.adoc`), so key
/// order is the default. An entry exists only where the vendored text requires
/// the entries in declaration order; the readers keep document order.
pub(crate) const ORDERED_HASHES: &[OrderedHash] = &[
    OrderedHash {
        owner: "P_BMM_GENERIC_TYPE",
        field: "generic_parameter_defs",
        citation: "docs/specs/openehr/LANG/docs/UML/classes/org.openehr.lang.bmm_persistence.p_bmm_generic_type.adoc \
                   \u{a7}Attributes (SPECLANG-16): \"Generic parameters of the `_root_type_` in this \
                   type specifier, if non-simple types, keyed by formal generic parameter name, \
                   e.g. `T`. The order must match the order of the owning class's formal generic \
                   parameter declarations.\"",
        reason: "The actual parameters are keyed by formal name and ordered as the root class declares them.",
    },
    OrderedHash {
        owner: "P_BMM_CLASS",
        field: "generic_parameter_defs",
        citation: "docs/specs/openehr/LANG/docs/UML/classes/org.openehr.lang.bmm_persistence.p_bmm_generic_type.adoc \
                   \u{a7}Attributes: both `generic_parameter_defs` and `generic_parameters` \"must \
                   match the order of the owning class's formal generic parameter declarations\", \
                   which are this attribute's entries.",
        reason: "The formal declarations fix the order every actual parameter list follows.",
    },
    OrderedHash {
        owner: "BMM_GENERIC_CLASS",
        field: "generic_parameters",
        citation: "docs/specs/openehr/LANG/docs/UML/classes/org.openehr.lang.bmm.bmm_generic_type.adoc \
                   and org.openehr.lang.bmm3.bmm_generic_type.adoc \u{a7}Attributes: \
                   `BMM_GENERIC_TYPE.generic_parameters` \"The order must match the order of the \
                   owning class's formal generic parameter declarations\", which are this \
                   attribute's entries.",
        reason: "An actual parameter list binds to these formal declarations by position.",
    },
];

/// `rendered`, the Rust type of `field` on `class`, with its key-sorted
/// `BTreeMap` retyped as an insertion-ordered `IndexMap` when the attribute is
/// an [`ORDERED_HASHES`] entry declared on `class` or inherited from an
/// ancestor.
pub(crate) fn retype(model: &Model, class: &BmmClass, field: &str, rendered: String) -> String {
    let ordered = ORDERED_HASHES.iter().any(|o| {
        o.field == field && (o.owner == class.name || model.inherits(&class.name, o.owner))
    });
    match rendered.strip_prefix("std::collections::BTreeMap<") {
        Some(args) if ordered => format!("indexmap::IndexMap<{args}"),
        _ => rendered,
    }
}
