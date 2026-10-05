// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The composed P_BMM load: ODIN schema text (plus the text of every schema it
//! includes) → an in-memory `BMM_MODEL`.
//!
//! This is the whole of what `master02-overview.adoc` §Conceptual Approach asks a
//! schema reader to do — "A schema reading component has to resolve the schema
//! inclusions and ultimately `BMM_*` object instantiations to obtain the
//! in-memory form of the model" — as one call over three stages:
//! [`crate::v1_1::bmm_persistence::reader::read_schema`] →
//! [`crate::v1_1::bmm_persistence::include_resolution::resolve_includes`] →
//! [`crate::v1_1::bmm_persistence::create_model::create_bmm_model`].
//!
//! It takes SOURCE TEXT, never paths: locating `.bmm` files on disk is a
//! repository concern, not a schema-reading one, and keeping it out of this
//! module keeps the whole pipeline testable from string literals.

use std::collections::BTreeMap;

use crate::v1_1::bmm::core::bmm_model::BmmModel;
use crate::v1_1::bmm_persistence::create_model::create_bmm_model;
use crate::v1_1::bmm_persistence::error::PBmmReadError;
use crate::v1_1::bmm_persistence::include_resolution::resolve_includes;
use crate::v1_1::bmm_persistence::p_bmm_schema::PBmmSchema;
use crate::v1_1::bmm_persistence::reader::read_schema;

/// Load the `BMM_MODEL` of `root_src`, resolving its inclusions against
/// `includes`.
///
/// `includes` maps a caller-side label (conventionally the schema id) to that
/// schema's ODIN text. Resolution keys on each parsed schema's OWN
/// [`PBmmSchema::schema_id`], so a mislabelled entry still resolves correctly
/// and a duplicate id is refused rather than silently shadowed. Supplying more
/// schemas than `root_src` needs is harmless; the unused ones are read (and so
/// validated) but not merged.
///
/// # Errors
/// Returns any [`PBmmReadError`] the three stages raise: an ODIN or schema-shape
/// failure while reading `root_src` or any entry of `includes`, an unresolvable
/// or cyclic inclusion, or an unresolvable symbolic reference while
/// materialising the model.
pub fn load_model(
    root_src: &str,
    includes: &BTreeMap<String, String>,
) -> Result<BmmModel, PBmmReadError> {
    let root = read_schema(root_src)?;
    let mut loaded: BTreeMap<String, PBmmSchema> = BTreeMap::new();
    for (label, src) in includes {
        loaded.insert(label.clone(), read_schema(src)?);
    }
    let resolved = resolve_includes(root, &loaded)?;
    create_bmm_model(&resolved)
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::panic_in_result_fn,
        reason = "the Book ch11 test shape: `?` propagates the read/resolve/model plumbing while the assertions ARE the test — an assertion panic is how these tests fail"
    )]
    use std::collections::BTreeMap;

    use crate::v1_1::bmm::core::bmm_class::BmmClass;
    use crate::v1_1::bmm::core::bmm_enumeration::BmmEnumeration;
    use crate::v1_1::bmm::core::bmm_model::BmmModel;
    use crate::v1_1::bmm::core::bmm_property::BmmProperty;
    use crate::v1_1::bmm_persistence::error::PBmmReadError;
    use crate::v1_1::bmm_persistence::loader::load_model;

    /// A two-file schema set: `primitive_types` defining `Any`/`Ordered`/`String`
    /// plus the generic `Interval`, and `rm` including it and defining
    /// `DV_ORDERED` / `DV_QUANTITY` / `ITEM_TREE`.
    const PRIMITIVE_TYPES: &str = r#"
        bmm_version = <"2.4">
        rm_publisher = <"openehr">
        schema_name = <"primitive_types">
        rm_release = <"1.0.2">
        packages = <
            ["org.openehr.rm.support.assumed_types"] = <
                name = <"org.openehr.rm.support.assumed_types">
                classes = <"Any", "Ordered", "String", "Real", "List", "Interval">
            >
        >
        primitive_types = <
            ["Any"] = <
                name = <"Any">
                is_abstract = <True>
            >
            ["Ordered"] = <
                name = <"Ordered">
                is_abstract = <True>
                ancestors = <"Any", ...>
            >
            ["String"] = <
                name = <"String">
                ancestors = <"Any", ...>
            >
            ["Real"] = <
                name = <"Real">
                ancestors = <"Ordered", ...>
            >
            ["List"] = <
                name = <"List">
                ancestors = <"Any", ...>
                generic_parameter_defs = <
                    ["T"] = <
                        name = <"T">
                    >
                >
            >
            ["Interval"] = <
                name = <"Interval">
                ancestors = <"Any", ...>
                generic_parameter_defs = <
                    ["T"] = <
                        name = <"T">
                        conforms_to_type = <"Ordered">
                    >
                >
                properties = <
                    ["lower"] = (P_BMM_SINGLE_PROPERTY_OPEN) <
                        name = <"lower">
                        type = <"T">
                    >
                >
            >
        >
    "#;

    const RM: &str = r#"
        bmm_version = <"2.4">
        rm_publisher = <"openehr">
        schema_name = <"rm">
        rm_release = <"1.0.2">
        includes = <
            ["1"] = <
                id = <"openehr_primitive_types_1.0.2">
            >
        >
        packages = <
            ["org.openehr.rm.data_types"] = <
                name = <"org.openehr.rm.data_types">
                classes = <"DV_ORDERED", "DV_QUANTITY", "MAGNITUDE_STATUS">
                packages = <
                    ["item_structure"] = <
                        name = <"item_structure">
                        classes = <"ITEM_TREE">
                    >
                >
            >
        >
        class_definitions = <
            ["DV_ORDERED"] = <
                name = <"DV_ORDERED">
                ancestors = <"Any">
                is_abstract = <True>
                properties = <
                    ["magnitude_status"] = (P_BMM_SINGLE_PROPERTY) <
                        name = <"magnitude_status">
                        type = <"MAGNITUDE_STATUS">
                    >
                >
            >
            ["DV_QUANTITY"] = <
                name = <"DV_QUANTITY">
                ancestors = <"DV_ORDERED">
                properties = <
                    ["magnitude"] = (P_BMM_SINGLE_PROPERTY) <
                        name = <"magnitude">
                        type = <"Real">
                        is_mandatory = <True>
                    >
                    ["normal_range"] = (P_BMM_GENERIC_PROPERTY) <
                        name = <"normal_range">
                        type_def = <
                            root_type = <"Interval">
                            generic_parameters = <"DV_QUANTITY">
                        >
                    >
                >
            >
            ["MAGNITUDE_STATUS"] = (P_BMM_ENUMERATION_STRING) <
                name = <"MAGNITUDE_STATUS">
                ancestors = <"String", ...>
                item_names = <"le", "ge", "eq">
                item_values = <"<=", ">=", "=">
            >
            ["ITEM_TREE"] = <
                name = <"ITEM_TREE">
                ancestors = <"Any">
                properties = <
                    ["items"] = (P_BMM_CONTAINER_PROPERTY) <
                        name = <"items">
                        type_def = <
                            container_type = <"List">
                            type = <"DV_QUANTITY">
                        >
                        cardinality = <|>=1|>
                    >
                >
            >
        >
    "#;

    /// The `RM` schema loaded over its include.
    fn model() -> Result<BmmModel, PBmmReadError> {
        let includes: BTreeMap<String, String> = [(
            "openehr_primitive_types_1.0.2".to_owned(),
            PRIMITIVE_TYPES.to_owned(),
        )]
        .into_iter()
        .collect();
        load_model(RM, &includes)
    }

    #[test]
    fn the_loaded_model_carries_the_root_header_and_every_included_class()
    -> Result<(), PBmmReadError> {
        let model = model()?;
        assert_eq!(model.schema_id(), "openehr_rm_1.0.2");
        let mut names: Vec<&str> = model
            .class_definitions
            .iter()
            .flatten()
            .map(|(name, _)| name.as_str())
            .collect();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "Any",
                "DV_ORDERED",
                "DV_QUANTITY",
                "ITEM_TREE",
                "Interval",
                "List",
                "MAGNITUDE_STATUS",
                "Ordered",
                "Real",
                "String",
            ]
        );
        // The primitive_types list is carried into is_primitive_type.
        let mut primitives = model.primitive_types();
        primitives.sort_unstable();
        assert_eq!(
            primitives,
            ["Any", "Interval", "List", "Ordered", "Real", "String"]
        );
        assert_eq!(model.enumeration_types(), ["MAGNITUDE_STATUS"]);
        Ok(())
    }

    #[test]
    fn a_generic_class_reports_its_type_signature() -> Result<(), PBmmReadError> {
        let model = model()?;
        let interval = model
            .class_definition("Interval")
            .expect("Interval is defined by the included schema");
        assert_eq!(interval.type_name(), "Interval<T>");
        assert_eq!(interval.type_signature(), "Interval<T:Ordered>");
        assert!(matches!(interval, BmmClass::BmmGenericClass(_)));
        Ok(())
    }

    #[test]
    fn immediate_descendants_invert_the_ancestor_graph() -> Result<(), PBmmReadError> {
        let model = model()?;
        let ordered = model
            .class_definition("DV_ORDERED")
            .expect("DV_ORDERED is defined");
        assert_eq!(ordered.immediate_descendants(), ["DV_QUANTITY"]);
        let any = model.class_definition("Any").expect("Any is defined");
        let mut descendants = any.immediate_descendants().to_vec();
        descendants.sort_unstable();
        assert_eq!(
            descendants,
            [
                "DV_ORDERED".to_owned(),
                "ITEM_TREE".to_owned(),
                "Interval".to_owned(),
                "List".to_owned(),
                "Ordered".to_owned(),
                "String".to_owned(),
            ]
        );
        Ok(())
    }

    #[test]
    fn property_definition_at_path_navigates_the_loaded_model() -> Result<(), PBmmReadError> {
        let model = model()?;
        // `ITEM_TREE.items` is List<DV_QUANTITY>; the container step navigates
        // into the contained type, whose `magnitude` is inherited-free and whose
        // `magnitude_status` comes from DV_ORDERED.
        let magnitude = model
            .property_definition_at_path("ITEM_TREE", "items/magnitude")
            .expect("the nested property resolves through the contained type");
        assert_eq!(magnitude.type_name(), "Real");
        let status = model
            .property_definition_at_path("ITEM_TREE", "items/magnitude_status")
            .expect("the inherited property resolves too");
        assert_eq!(status.type_name(), "MAGNITUDE_STATUS");
        assert!(
            model
                .property_definition_at_path("ITEM_TREE", "items/absent")
                .is_none()
        );
        Ok(())
    }

    #[test]
    fn container_cardinality_and_type_map_onto_the_bmm_shapes() -> Result<(), PBmmReadError> {
        let model = model()?;
        let items = model
            .property_definition("ITEM_TREE", "items")
            .expect("ITEM_TREE declares items");
        assert_eq!(items.type_name(), "List<DV_QUANTITY>");
        assert_eq!(items.conformance_type_name(), "DV_QUANTITY");
        // `|>=1|` maps onto a lower-bounded, upper-unbounded
        // Multiplicity_interval (BMM_CONTAINER_PROPERTY.cardinality).
        let BmmProperty::BmmContainerProperty(container) = items else {
            panic!("a (P_BMM_CONTAINER_PROPERTY) maps onto BMM_CONTAINER_PROPERTY");
        };
        let cardinality = container
            .cardinality
            .as_ref()
            .expect("the property states a cardinality");
        assert_eq!(cardinality.lower, Some(1));
        assert!(cardinality.lower_included);
        assert_eq!(cardinality.upper, None);
        assert!(cardinality.upper_unbounded);
        Ok(())
    }

    #[test]
    fn a_generic_property_binds_its_actual_parameter() -> Result<(), PBmmReadError> {
        let model = model()?;
        let range = model
            .property_definition("DV_QUANTITY", "normal_range")
            .expect("DV_QUANTITY declares normal_range");
        assert_eq!(range.type_name(), "Interval<DV_QUANTITY>");
        Ok(())
    }

    #[test]
    fn an_enumeration_maps_onto_the_string_enumeration_form() -> Result<(), PBmmReadError> {
        let model = model()?;
        let enumeration = model
            .enumeration_definition("MAGNITUDE_STATUS")
            .expect("MAGNITUDE_STATUS is an enumeration");
        let BmmEnumeration::BmmEnumerationString(string) = enumeration else {
            panic!("a (P_BMM_ENUMERATION_STRING) maps onto BMM_ENUMERATION_STRING");
        };
        assert_eq!(
            string.item_names,
            Some(["le", "ge", "eq"].to_vec()).map(|v| v.into_iter().map(str::to_owned).collect())
        );
        assert_eq!(string.underlying_type_name, "STRING");
        assert_eq!(string.item_values.as_ref().map_or(0, Vec::len), 3);
        Ok(())
    }

    #[test]
    fn a_class_carries_the_fully_qualified_path_of_its_package() -> Result<(), PBmmReadError> {
        let model = model()?;
        let item_tree = model
            .class_definition("ITEM_TREE")
            .expect("ITEM_TREE is defined");
        assert_eq!(
            item_tree.package_path(),
            "org.openehr.rm.data_types.item_structure"
        );
        assert_eq!(
            item_tree.class_path(),
            "org.openehr.rm.data_types.item_structure.ITEM_TREE"
        );
        // The package TREE keeps each node's own name as written.
        let nested = model
            .package_at_path("org.openehr.rm.data_types.item_structure")
            .expect("the nested package resolves from the model root");
        assert_eq!(nested.name, "item_structure");
        assert_eq!(nested.classes.as_ref().map_or(0, Vec::len), 1);
        Ok(())
    }

    #[test]
    fn an_open_property_resolves_its_formal_parameter() -> Result<(), PBmmReadError> {
        let model = model()?;
        let lower = model
            .property_definition("Interval", "lower")
            .expect("Interval declares lower");
        assert_eq!(lower.type_name(), "T");
        // BMM_OPEN_TYPE.conformance_type_name is the constrainer.
        assert_eq!(lower.conformance_type_name(), "Ordered");
        Ok(())
    }

    #[test]
    fn a_missing_include_is_reported_from_the_composed_entry_point() {
        let error = load_model(RM, &BTreeMap::new()).expect_err("the include is not supplied");
        assert_eq!(
            error,
            PBmmReadError::MissingInclude {
                requester: "openehr_rm_1.0.2".to_owned(),
                id: "openehr_primitive_types_1.0.2".to_owned(),
            }
        );
    }
}
