// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The attributes of a generic class whose declared type drops the argument
//! the class's own parameter supplies, and the load-time step that restores it.
//!
//! The decision map in [`crate::testsupport`] checks every entry names an
//! attribute of the loaded schemas.

use crate::load::bmm::{BmmPropKind, BmmSchema, BmmType};

/// One elided generic argument: `class.property` is declared as the bare
/// generic class `root` and is read as `root<arguments…>`, each argument a
/// generic parameter of `class` that the RM text gives no other attribute to
/// act through.
pub(crate) struct ElidedArgument {
    /// The class declaring the attribute.
    pub class: &'static str,
    /// The attribute.
    pub property: &'static str,
    /// The bare generic class the vendored schema types the attribute with.
    pub root: &'static str,
    /// The restored arguments, in the root's parameter order.
    pub arguments: &'static [&'static str],
    /// Spec citation.
    pub citation: &'static str,
    /// One-line reason.
    pub reason: &'static str,
}

/// Attributes whose declared type drops the argument its class's own generic
/// parameter supplies. The loader restores each one in every schema declaring
/// the attribute and refuses a schema whose declaration is no longer the bare
/// `root` (an upstream fix, or a retyped attribute), so a stale entry fails
/// emission instead of rewriting a type it no longer describes.
pub(crate) const ELIDED_ARGUMENTS: &[ElidedArgument] = &[
    ElidedArgument {
        class: "REFERENCE_RANGE",
        property: "range",
        root: "DV_INTERVAL",
        arguments: &["T"],
        citation: "RM data_types UML/classes/org.openehr.rm.data_types.reference_range.adoc heads \
                   the class `REFERENCE_RANGE<T>` and types `range: DV_INTERVAL`, leaving `T` \
                   unused; `DV_QUANTITY`, `DV_COUNT` and `DV_PROPORTION` redefine \
                   `other_reference_ranges: List<REFERENCE_RANGE<X>>` beside `normal_range: \
                   DV_INTERVAL<X>`, the invariant `Range_is_simple` reads `range.lower.is_simple`, \
                   and data_types/master06-quantity_package.adoc §Ranges: \"Ranges are modelled \
                   by the generic type `DV_INTERVAL<T:DV_ORDERED>`\"",
        reason: "The range is the one attribute through which the declared T can act; reported \
                 upstream as an RM defect (tracker issue #5196).",
    },
    ElidedArgument {
        class: "IMPORTED_VERSION",
        property: "item",
        root: "ORIGINAL_VERSION",
        arguments: &["T"],
        citation: "RM common UML/classes/org.openehr.rm.common.imported_version.adoc heads the \
                   class `IMPORTED_VERSION<T>` and types `item: ORIGINAL_VERSION`, while its \
                   effected `data (): T` is the \"Original content of this Version\", which only \
                   `item` carries",
        reason: "The imported item is the one attribute through which the declared T can act; \
                 reported upstream with REFERENCE_RANGE (tracker issue #5196).",
    },
];

/// Restore every [`ELIDED_ARGUMENTS`] entry in `schema`.
///
/// # Errors
/// Returns a message naming the entry when `schema` declares the attribute
/// with any type other than the bare `root` the entry restores.
pub(crate) fn restore_elided_arguments(schema: &mut BmmSchema) -> Result<(), String> {
    for entry in ELIDED_ARGUMENTS {
        let Some(prop) = schema.classes.get_mut(entry.class).and_then(|class| {
            class
                .properties
                .iter_mut()
                .find(|p| p.name == entry.property)
        }) else {
            continue;
        };
        match &prop.kind {
            BmmPropKind::Single(BmmType::Simple(root)) if root == entry.root => {}
            other => {
                return Err(format!(
                    "{} {}: {}.{} is no longer the bare `{}` the elided-argument entry restores \
                     (found {other:?})",
                    schema.schema_name, schema.rm_release, entry.class, entry.property, entry.root
                ));
            }
        }
        prop.kind = BmmPropKind::Single(BmmType::Generic {
            root: entry.root.to_string(),
            params: entry
                .arguments
                .iter()
                .map(|a| BmmType::Simple((*a).to_string()))
                .collect(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::restore_elided_arguments;
    use crate::load::bmm::{BmmPropKind, BmmSchema, BmmType};

    /// A schema declaring `REFERENCE_RANGE.range` with the given property JSON.
    fn schema(range: &str) -> BmmSchema {
        let src = format!(
            r#"{{
            "schema_name": "rm", "rm_release": "1.2.0", "bmm_version": "2.4",
            "class_definitions": {{
                "REFERENCE_RANGE": {{
                    "name": "REFERENCE_RANGE",
                    "generic_parameter_defs": {{ "T": {{ "name": "T", "conforms_to_type": "DV_ORDERED" }} }},
                    "properties": {{ "range": {range} }}
                }}
            }}
        }}"#
        );
        BmmSchema::parse_json(&src).unwrap()
    }

    #[test]
    fn the_bare_range_is_read_as_an_interval_of_t() {
        let mut s = schema(
            r#"{ "_type": "P_BMM_SINGLE_PROPERTY", "name": "range", "is_mandatory": true, "type": "DV_INTERVAL" }"#,
        );
        restore_elided_arguments(&mut s).unwrap();
        let range = &s.classes["REFERENCE_RANGE"].properties[0];
        assert!(matches!(
            &range.kind,
            BmmPropKind::Single(BmmType::Generic { root, params })
                if root == "DV_INTERVAL" && params == &[BmmType::Simple("T".to_owned())]
        ));
        assert!(range.is_mandatory);
    }

    #[test]
    fn a_range_upstream_already_types_refuses_the_entry() {
        let mut s = schema(
            r#"{ "_type": "P_BMM_GENERIC_PROPERTY", "name": "range", "is_mandatory": true, "type_def": { "root_type": "DV_INTERVAL", "generic_parameters": ["T"] } }"#,
        );
        let err = restore_elided_arguments(&mut s).unwrap_err();
        assert!(err.contains("REFERENCE_RANGE.range"), "{err}");
    }

    #[test]
    fn a_schema_without_the_class_is_left_alone() {
        let mut s = BmmSchema::parse_json(
            r#"{ "schema_name": "base", "rm_release": "1.3.0", "bmm_version": "2.4" }"#,
        )
        .unwrap();
        restore_elided_arguments(&mut s).unwrap();
        assert!(s.classes.is_empty());
    }
}
