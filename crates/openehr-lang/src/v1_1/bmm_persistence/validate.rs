// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The model validity checker over a resolved `P_BMM_SCHEMA` and the v2.x
//! `BMM_MODEL` materialised from it.
//!
//! `LANG/docs/bmm3/master05-core-model.adoc` §Packages states the two rules
//! this pass enforces: "A model validity checker ensures that every class is
//! contained within exactly one package", and — because package paths "are not
//! used as namespaces as in UML" — "all classes in a BMM model should be
//! uniquely named".
//!
//! The pass COLLECTS: it returns every finding rather than stopping at the
//! first, because a validity report is only useful whole. It is distinct from
//! the typed refusals of
//! [`crate::v1_1::bmm_persistence::error::PBmmReadError`], which stay fail-fast
//! because each of them names a condition under which no `BMM_*` object can be
//! constructed at all.

use std::collections::BTreeMap;
use std::fmt;

use crate::v1_1::bmm::core::bmm_class::BmmClass;
use crate::v1_1::bmm::core::bmm_model::BmmModel;
use crate::v1_1::bmm::core::bmm_property::BmmProperty;
use crate::v1_1::bmm::core::bmm_type::BmmType;
use crate::v1_1::bmm_persistence::create_model::qualify;
use crate::v1_1::bmm_persistence::p_bmm_class::PBmmClass;
use crate::v1_1::bmm_persistence::p_bmm_package::PBmmPackage;
use crate::v1_1::bmm_persistence::p_bmm_schema::PBmmSchema;

/// One violation of BMM model validity found by [`validate_schema`].
///
/// Every variant is a discriminant a caller can branch on; the display text is
/// never a decision input.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PBmmValidityFinding {
    /// A class is not contained within exactly one package — "A model validity
    /// checker ensures that every class is contained within exactly one
    /// package" (`LANG/docs/bmm3/master05-core-model.adoc` §Packages).
    ///
    /// In practice this reports the DUPLICATE direction: a class listed in no
    /// package cannot be materialised at all, so it is already the fail-fast
    /// [`crate::v1_1::bmm_persistence::error::PBmmReadError::ClassNotInAnyPackage`].
    ClassNotInExactlyOnePackage {
        /// The class's name as the schema defines it.
        class: String,
        /// The package path of each listing, in schema order.
        packages: Vec<String>,
    },

    /// Two or more class definitions share one name — "all classes in a BMM
    /// model should be uniquely named"
    /// (`LANG/docs/bmm3/master05-core-model.adoc` §Packages).
    ///
    /// Names are compared case-insensitively, since "it is assumed that
    /// case-insensitive matching is used … the class name `"Hashable"` refers
    /// to the same class as `"HASHABLE"`" (same file, §Naming Convention).
    ClassNameNotUnique {
        /// The colliding definitions' names as written, in schema order.
        definitions: Vec<String>,
    },

    /// A persisted assertion string could not be materialised as a
    /// `BMM_ASSERTION`, so the class or routine carries it nowhere.
    ///
    /// v3 requires class invariants and routine pre-/post-conditions as
    /// `BMM_ASSERTION` (`LANG/docs/bmm3/master10-expressions.adoc` §Usage in
    /// BMM Models) whose `expression` is a `1..1` `EL_BOOLEAN_EXPRESSION`
    /// (`…bmm3.bmm_assertion.adoc` §Attributes), while P_BMM persists an
    /// opaque expression string (`…bmm_persistence.p_bmm_class.adoc`
    /// §Attributes). A string that is not EL, or whose names do not resolve,
    /// is reported here rather than refusing the schema.
    AssertionNotMaterialised {
        /// The class the assertion belongs to.
        class: String,
        /// The owning routine, for a pre-/post-condition.
        routine: Option<String>,
        /// Which assertion position the string was persisted in.
        kind: AssertionKind,
        /// The assertion's tag, as persisted.
        tag: String,
        /// The persisted expression string, verbatim.
        expression: String,
        /// The parse or resolution failure, as reported by
        /// [`crate::v1_1::el::ElError`].
        reason: String,
    },

    /// A persisted constant states no value, so the class carries it nowhere.
    ///
    /// `P_BMM_CONSTANT.value` is `0..1` — "The literal value of this constant,
    /// in its persisted (serialised) form"
    /// (`org.openehr.lang.bmm_persistence.p_bmm_constant.adoc` §Attributes) —
    /// while the v3 destination is a `1..1` `BMM_CONSTANT.generator`
    /// (`org.openehr.lang.bmm3.bmm_constant.adoc` §Attributes) over a
    /// `BMM_LITERAL_VALUE` whose `value_literal` is a `1..1` "serial
    /// representation of the value"
    /// (`org.openehr.lang.bmm3.bmm_literal_value.adoc` §Attributes). Stating no
    /// value is legal P_BMM — openEHR's own published LANG schemas do it for
    /// `BMM_DEFINITIONS.Bmm_internal_version` — so it is reported here rather
    /// than refusing the schema, and no empty serial form is invented.
    ConstantNotMaterialised {
        /// The class the constant belongs to.
        class: String,
        /// The constant's name, as persisted.
        constant: String,
    },

    /// A class redefines an inherited property with a type that does not
    /// conform to the overridden property's type.
    ///
    /// NOTE: no openEHR spec governs this — our own design/extension; the
    /// released text leaves redefinition conformance open
    /// (`LANG/docs/bmm3/master13-model_semantics.adoc` §Inheritance and
    /// Invariants, Pre-conditions and Post-conditions is `TBD`).
    OverriddenPropertyNonConformance {
        /// The redefining class's name.
        class: String,
        /// The redefined property's name.
        property: String,
        /// The nearest ancestor declaring the overridden property.
        ancestor: String,
        /// The redefined property's type name.
        redefined_type: String,
        /// The overridden property's type name.
        overridden_type: String,
    },
}

/// Which assertion position a persisted expression string was declared in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssertionKind {
    /// `P_BMM_CLASS.invariants` — a class invariant.
    Invariant,
    /// `P_BMM_FUNCTION.pre_conditions` — a routine pre-condition.
    PreCondition,
    /// `P_BMM_FUNCTION.post_conditions` — a routine post-condition.
    PostCondition,
}

impl fmt::Display for AssertionKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Invariant => "invariant",
            Self::PreCondition => "pre-condition",
            Self::PostCondition => "post-condition",
        };
        formatter.write_str(text)
    }
}

impl fmt::Display for PBmmValidityFinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ClassNotInExactlyOnePackage { class, packages } => write!(
                formatter,
                "class `{class}` is contained within {} package listing(s) ({}); \
                 a class must be contained within exactly one package",
                packages.len(),
                packages.join(", ")
            ),
            Self::ClassNameNotUnique { definitions } => write!(
                formatter,
                "{} class definitions share one name ({}); \
                 all classes in a BMM model must be uniquely named",
                definitions.len(),
                definitions.join(", ")
            ),
            Self::AssertionNotMaterialised {
                class,
                routine,
                kind,
                tag,
                expression,
                reason,
            } => {
                let owner = routine.as_ref().map_or_else(
                    || format!("class `{class}`"),
                    |routine| format!("class `{class}` routine `{routine}`"),
                );
                write!(
                    formatter,
                    "{owner} {kind} `{tag}` ({expression:?}) is not materialisable                      as a BMM_ASSERTION: {reason}"
                )
            }
            Self::ConstantNotMaterialised { class, constant } => write!(
                formatter,
                "class `{class}` constant `{constant}` states no value, so it is not \
                 materialisable as a BMM_CONSTANT"
            ),
            Self::OverriddenPropertyNonConformance {
                class,
                property,
                ancestor,
                redefined_type,
                overridden_type,
            } => write!(
                formatter,
                "class `{class}` redefines property `{property}` as `{redefined_type}`, \
                 which does not conform to `{overridden_type}` as declared by `{ancestor}`"
            ),
        }
    }
}

/// Checks `schema` and the `model` materialised from it for BMM model
/// validity, returning every finding.
///
/// Both forms are needed and neither is redundant: a duplicate package listing
/// and a case-variant name collision are visible only in the PERSISTED schema
/// (`BMM_CLASS.package` is `1..1` and `BMM_MODEL.class_definitions` is a
/// name-keyed map, so both collapse on materialisation), while property
/// conformance needs the model's resolved type graph
/// ([`BmmModel::type_conforms_to`]).
///
/// `model` must be the model
/// [`crate::v1_1::bmm_persistence::create_model::create_bmm_model`] produced from
/// `schema`; the findings are otherwise about two unrelated artefacts.
///
/// # Examples
/// ```
/// use openehr_lang::v1_1::bmm_persistence::create_model::create_bmm_model;
/// use openehr_lang::v1_1::bmm_persistence::reader::read_schema;
/// use openehr_lang::v1_1::bmm_persistence::validate::validate_schema;
///
/// let src = r#"
///     bmm_version = <"2.4">
///     rm_publisher = <"openehr">
///     schema_name = <"tiny">
///     rm_release = <"1.0.2">
///     packages = <
///         ["org.openehr.tiny"] = <
///             name = <"org.openehr.tiny">
///             classes = <"Any">
///         >
///     >
///     class_definitions = <
///         ["Any"] = < name = <"Any"> >
///     >
/// "#;
/// let schema = read_schema(src)?;
/// let model = create_bmm_model(&schema)?;
/// assert!(validate_schema(&schema, &model).is_empty());
/// # Ok::<(), openehr_lang::v1_1::bmm_persistence::error::PBmmReadError>(())
/// ```
#[must_use]
pub fn validate_schema(schema: &PBmmSchema, model: &BmmModel) -> Vec<PBmmValidityFinding> {
    let mut findings = Vec::new();
    check_package_containment(schema, &mut findings);
    check_class_name_uniqueness(schema, &mut findings);
    check_overridden_property_conformance(model, &mut findings);
    findings
}

/// Every class the schema defines, primitive types first, in schema order.
///
/// One name means one class regardless of which list holds it —
/// `LANG/docs/bmm_persistence/master04-syntax.adoc` §Classes for Primitive
/// Types: primitive types "are just normal class definitions within a
/// `primitive_types` block … otherwise are processed in the same way as types
/// defined in the main `class_definitions` group".
fn schema_classes(schema: &PBmmSchema) -> impl Iterator<Item = &PBmmClass> {
    schema
        .primitive_types
        .iter()
        .flatten()
        .chain(schema.class_definitions.iter().flatten())
}

/// Reports every class not contained within exactly one package.
fn check_package_containment(schema: &PBmmSchema, findings: &mut Vec<PBmmValidityFinding>) {
    let mut listings: BTreeMap<String, Vec<String>> = BTreeMap::new();
    collect_class_listings(&schema.packages, "", &mut listings);
    for class in schema_classes(schema) {
        let packages = listings
            .get(&class.name().to_uppercase())
            .cloned()
            .unwrap_or_default();
        if packages.len() != 1 {
            findings.push(PBmmValidityFinding::ClassNotInExactlyOnePackage {
                class: class.name().to_owned(),
                packages,
            });
        }
    }
}

/// Records the package path of every class listing in the package tree.
///
/// Keys fold to upper case, matching the case-insensitive class-name matching
/// of `LANG/docs/bmm3/master05-core-model.adoc` §Naming Convention.
fn collect_class_listings(
    packages: &BTreeMap<String, PBmmPackage>,
    prefix: &str,
    out: &mut BTreeMap<String, Vec<String>>,
) {
    for package in packages.values() {
        let path = qualify(prefix, &package.name);
        for class in package.classes.iter().flatten() {
            out.entry(class.to_uppercase())
                .or_default()
                .push(path.clone());
        }
        collect_class_listings(&package.packages, &path, out);
    }
}

/// Reports every set of class definitions sharing one case-insensitive name.
fn check_class_name_uniqueness(schema: &PBmmSchema, findings: &mut Vec<PBmmValidityFinding>) {
    let mut spellings: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for class in schema_classes(schema) {
        spellings
            .entry(class.name().to_uppercase())
            .or_default()
            .push(class.name().to_owned());
    }
    for definitions in spellings.into_values() {
        if definitions.len() > 1 {
            findings.push(PBmmValidityFinding::ClassNameNotUnique { definitions });
        }
    }
}

/// Reports every redefined property whose type does not conform to the
/// overridden one.
///
/// A class's own `properties` are the "differential" set — the features it
/// "introduces with respect to its inheritance parent(s)"
/// (`LANG/docs/bmm3/master08-core-features.adoc` §Differential and Flat Form)
/// — so a name that also resolves on an ancestor is a redefinition.
fn check_overridden_property_conformance(
    model: &BmmModel,
    findings: &mut Vec<PBmmValidityFinding>,
) {
    for class in model.class_definitions.iter().flatten().map(|(_, c)| c) {
        for property in class.properties().into_iter().flatten().map(|(_, p)| p) {
            let Some((ancestor, overridden)) = nearest_overridden(model, class, property.name())
            else {
                continue;
            };
            let redefined_type = property.type_name();
            let overridden_type = overridden.type_name();
            if !model.type_conforms_to(&redefined_type, &overridden_type) {
                findings.push(PBmmValidityFinding::OverriddenPropertyNonConformance {
                    class: class.name().to_owned(),
                    property: property.name().to_owned(),
                    ancestor,
                    redefined_type,
                    overridden_type,
                });
            }
        }
    }
}

/// The nearest ancestor of `class` that declares `property` itself, with that
/// declaration.
///
/// `BMM_MODEL.all_ancestor_classes` walks breadth-first from the class, so the
/// first hit is the nearest declaring ancestor.
fn nearest_overridden<'a>(
    model: &'a BmmModel,
    class: &BmmClass,
    property: &str,
) -> Option<(String, &'a BmmProperty<BmmType>)> {
    model
        .all_ancestor_classes(class.name())
        .into_iter()
        .find_map(|ancestor| {
            let declared = model
                .class_definition(&ancestor)?
                .properties()?
                .values()
                .find(|candidate| candidate.name() == property)?;
            Some((ancestor, declared))
        })
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::panic_in_result_fn,
        reason = "the Book ch11 test shape: `?` propagates the read/model plumbing while the assertions ARE the test — an assertion panic is how these tests fail"
    )]
    use crate::v1_1::bmm_persistence::create_model::create_bmm_model;
    use crate::v1_1::bmm_persistence::error::PBmmReadError;
    use crate::v1_1::bmm_persistence::reader::read_schema;
    use crate::v1_1::bmm_persistence::validate::PBmmValidityFinding;
    use crate::v1_1::bmm_persistence::validate::validate_schema;

    /// Reads `src` and validates the model materialised from it.
    fn findings(src: &str) -> Result<Vec<PBmmValidityFinding>, PBmmReadError> {
        let schema = read_schema(src)?;
        let model = create_bmm_model(&schema)?;
        Ok(validate_schema(&schema, &model))
    }

    /// A schema whose one package lists `ParentType1` twice — the shape of the
    /// vendored fixture
    /// `tests/vendor/bmm/…/persistence/validation/duplicate_class.bmm`.
    const DUPLICATE_LISTING: &str = r#"
        bmm_version = <"2.4">
        rm_publisher = <"openehr">
        schema_name = <"duplicate_listing">
        rm_release = <"1.0.2">
        packages = <
            ["ParentPackage"] = <
                name = <"ParentPackage">
                classes = <"ParentType1", "ParentType1", "Any">
            >
        >
        class_definitions = <
            ["ParentType1"] = < name = <"ParentType1"> >
            ["Any"] = < name = <"Any"> >
        >
    "#;

    /// Two class definitions differing only in case, each listed in its own
    /// package — the minimal §Packages line-31 violation.
    const CASE_VARIANT_NAMES: &str = r#"
        bmm_version = <"2.4">
        rm_publisher = <"openehr">
        schema_name = <"case_variant_names">
        rm_release = <"1.0.2">
        packages = <
            ["org.openehr.first"] = <
                name = <"org.openehr.first">
                classes = <"Hashable", "Any">
            >
            ["org.openehr.second"] = <
                name = <"org.openehr.second">
                classes = <"HASHABLE">
            >
        >
        class_definitions = <
            ["Hashable"] = < name = <"Hashable"> >
            ["HASHABLE"] = < name = <"HASHABLE"> >
            ["Any"] = < name = <"Any"> >
        >
    "#;

    /// A child redefining an inherited `String` property as a non-conformant
    /// type — the shape of the vendored fixture
    /// `tests/vendor/bmm/…/persistence/validation/overridden_property_non_conformance.bmm`.
    const NON_CONFORMANT_OVERRIDE: &str = r#"
        bmm_version = <"2.4">
        rm_publisher = <"openehr">
        schema_name = <"non_conformant_override">
        rm_release = <"1.0.2">
        packages = <
            ["ParentPackage"] = <
                name = <"ParentPackage">
                classes = <"ParentType1", "ChildType1", "String", "Any">
            >
        >
        class_definitions = <
            ["ParentType1"] = <
                name = <"ParentType1">
                properties = <
                    ["property_1"] = (P_BMM_SINGLE_PROPERTY) <
                        name = <"property_1">
                        type = <"String">
                    >
                >
            >
            ["ChildType1"] = <
                name = <"ChildType1">
                ancestors = <"ParentType1", ...>
                properties = <
                    ["property_1"] = (P_BMM_SINGLE_PROPERTY) <
                        name = <"property_1">
                        type = <"ParentType1">
                    >
                >
            >
            ["String"] = < name = <"String"> >
            ["Any"] = < name = <"Any"> >
        >
    "#;

    /// A child narrowing an inherited property to a descendant type — the
    /// conformant twin of [`NON_CONFORMANT_OVERRIDE`].
    const CONFORMANT_OVERRIDE: &str = r#"
        bmm_version = <"2.4">
        rm_publisher = <"openehr">
        schema_name = <"conformant_override">
        rm_release = <"1.0.2">
        packages = <
            ["ParentPackage"] = <
                name = <"ParentPackage">
                classes = <"ParentType1", "ChildType1", "String", "Coded_string", "Any">
            >
        >
        class_definitions = <
            ["ParentType1"] = <
                name = <"ParentType1">
                properties = <
                    ["property_1"] = (P_BMM_SINGLE_PROPERTY) <
                        name = <"property_1">
                        type = <"String">
                    >
                >
            >
            ["ChildType1"] = <
                name = <"ChildType1">
                ancestors = <"ParentType1", ...>
                properties = <
                    ["property_1"] = (P_BMM_SINGLE_PROPERTY) <
                        name = <"property_1">
                        type = <"Coded_string">
                    >
                >
            >
            ["String"] = < name = <"String"> >
            ["Coded_string"] = <
                name = <"Coded_string">
                ancestors = <"String", ...>
            >
            ["Any"] = < name = <"Any"> >
        >
    "#;

    #[test]
    fn a_class_listed_twice_in_one_package_is_not_in_exactly_one_package()
    -> Result<(), PBmmReadError> {
        assert_eq!(
            findings(DUPLICATE_LISTING)?,
            [PBmmValidityFinding::ClassNotInExactlyOnePackage {
                class: "ParentType1".to_owned(),
                packages: ["ParentPackage".to_owned(), "ParentPackage".to_owned()].to_vec(),
            }]
        );
        Ok(())
    }

    #[test]
    fn two_definitions_differing_only_in_case_are_one_name() -> Result<(), PBmmReadError> {
        // §Naming Convention: "the class name "Hashable" refers to the same
        // class as "HASHABLE"", so §Packages line 31's uniqueness rule bites.
        let observed = findings(CASE_VARIANT_NAMES)?;
        assert!(
            observed.contains(&PBmmValidityFinding::ClassNameNotUnique {
                definitions: ["Hashable".to_owned(), "HASHABLE".to_owned()].to_vec(),
            }),
            "expected a name-collision finding, got {observed:?}"
        );
        Ok(())
    }

    #[test]
    fn a_non_conformant_redefinition_is_reported() -> Result<(), PBmmReadError> {
        assert_eq!(
            findings(NON_CONFORMANT_OVERRIDE)?,
            [PBmmValidityFinding::OverriddenPropertyNonConformance {
                class: "ChildType1".to_owned(),
                property: "property_1".to_owned(),
                ancestor: "ParentType1".to_owned(),
                redefined_type: "ParentType1".to_owned(),
                overridden_type: "String".to_owned(),
            }]
        );
        Ok(())
    }

    #[test]
    fn a_conformant_narrowing_redefinition_is_clean() -> Result<(), PBmmReadError> {
        assert_eq!(findings(CONFORMANT_OVERRIDE)?, []);
        Ok(())
    }

    #[test]
    fn every_finding_names_its_subject() -> Result<(), PBmmReadError> {
        let rendered = findings(NON_CONFORMANT_OVERRIDE)?
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ");
        assert!(rendered.contains("ChildType1"), "{rendered}");
        assert!(rendered.contains("property_1"), "{rendered}");
        Ok(())
    }
}
