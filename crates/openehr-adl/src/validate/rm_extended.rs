// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The openEHR RM extended by the class tables of packages that are no RM
//! release, and the class-table reads it shares with [`ProductionRmModel`].

use openehr_rm::v1_2::model;

use super::rm::{
    ProductionRmModel, RmAttr, RmEnum, RmModel, base_type_name, cardinality_bounds,
    production_class, render_declared_type,
};
use crate::aom::interval::Bounds;

/// The openEHR RM 1.2.0 of [`ProductionRmModel`] extended by further classes.
///
/// The classes belong to packages that are no RM release, each given as a
/// class table in the shape of the generated RM table, such as a draft change
/// proposal's own classes.
///
/// An extension class is found before an RM class of the same name, may
/// inherit from RM classes and may be the type of an RM attribute it
/// conforms to, which is the RM adaptation an archetype over such a package
/// needs (`docs/specs/openehr/AM/docs/AOM2/master11-rm_adaptation.adoc`).
/// Enumerations are the RM's alone.
#[derive(Debug, Clone, Copy)]
pub struct ExtendedRmModel {
    name: &'static str,
    extensions: &'static [&'static [model::RmClass]],
}

impl ExtendedRmModel {
    /// The RM extended by `extensions`, reported as `name`.
    #[must_use]
    pub const fn new(name: &'static str, extensions: &'static [&'static [model::RmClass]]) -> Self {
        Self { name, extensions }
    }

    /// The extension class or RM class named `base`, case-insensitively.
    fn class(&self, base: &str) -> Option<&'static model::RmClass> {
        self.extensions
            .iter()
            .flat_map(|table| table.iter())
            .find(|class| class.name.eq_ignore_ascii_case(base))
            .or_else(|| production_class(base))
    }
}

impl RmModel for ExtendedRmModel {
    fn name(&self) -> &str {
        self.name
    }

    fn type_exists(&self, rm_type: &str) -> bool {
        self.class(base_type_name(rm_type)).is_some()
    }

    fn conforms(&self, sub: &str, sup: &str) -> Option<bool> {
        let child = self.class(base_type_name(sub))?;
        let ancestor = self.class(base_type_name(sup))?;
        Some(class_conforms(child, ancestor))
    }

    fn attribute(&self, rm_type: &str, attr: &str) -> Option<RmAttr> {
        class_attribute(self.class(base_type_name(rm_type))?, attr)
    }

    fn enumeration(&self, rm_type: &str) -> Option<RmEnum> {
        ProductionRmModel.enumeration(rm_type)
    }
}

/// Whether `child` is `ancestor` or one of its descendants, by the transitive
/// ancestors the table records.
pub(super) fn class_conforms(child: &model::RmClass, ancestor: &model::RmClass) -> bool {
    child.name == ancestor.name || child.ancestors.contains(&ancestor.name)
}

/// Attribute `attr` of `class`, stored or computed, as an [`RmAttr`].
pub(super) fn class_attribute(class: &model::RmClass, attr: &str) -> Option<RmAttr> {
    let a = class
        .attributes
        .iter()
        .chain(class.computed_attributes)
        .find(|a| a.name == attr)?;
    let is_multiple = a.container != model::Container::None;
    Some(RmAttr {
        declared_type: render_declared_type(a.declared_type, a.type_params),
        is_multiple,
        existence: Bounds::new(i32::from(a.is_mandatory), Some(1)),
        // Real BMM cardinality where recorded; the permissive `{0..*}` only for
        // a container the BMM leaves un-cardinalitied (`master04.5` VCACA).
        cardinality: is_multiple.then(|| {
            a.cardinality
                .map_or(Bounds::new(0, None), cardinality_bounds)
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    static DRAFT: [model::RmClass; 1] = [model::RmClass {
        name: "DRAFT_ROOT",
        package: "org.example.draft",
        generated_type: true,
        is_abstract: false,
        ancestors: &["Any", "LOCATABLE", "PATHABLE"],
        descendants: &["DRAFT_ROOT"],
        attributes: &[model::RmAttribute {
            name: "parts",
            declared_type: "DV_TEXT",
            formal: None,
            container: model::Container::List,
            is_mandatory: false,
            type_params: &[],
            cardinality: Some(model::Cardinality {
                lower: 0,
                upper: None,
            }),
            nonempty: true,
            ref_target: None,
        }],
        computed_attributes: &[],
        is_structure_root: false,
        generic_params: &[],
    }];
    static TABLES: [&[model::RmClass]; 1] = [&DRAFT];

    #[test]
    fn an_extension_class_is_known_and_conforms_to_its_rm_ancestors() {
        let rm = ExtendedRmModel::new("RM with a draft", &TABLES);
        assert!(rm.type_exists("DRAFT_ROOT") && rm.type_exists("draft_root"));
        assert!(!ProductionRmModel.type_exists("DRAFT_ROOT"));
        assert_eq!(rm.conforms("DRAFT_ROOT", "LOCATABLE"), Some(true));
        assert_eq!(rm.conforms("DRAFT_ROOT", "COMPOSITION"), Some(false));
        assert_eq!(rm.conforms("OBSERVATION", "LOCATABLE"), Some(true));
        let parts = rm.attribute("DRAFT_ROOT", "parts").expect("an attribute");
        assert!(parts.is_multiple);
        assert_eq!(parts.declared_type, "DV_TEXT");
        assert!(rm.attribute("DRAFT_ROOT", "missing").is_none());
        assert_eq!(
            rm.attribute("OBSERVATION", "data"),
            ProductionRmModel.attribute("OBSERVATION", "data")
        );
    }
}
