// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The `P_BMM_SCHEMA` → `BMM_MODEL` transform.
//!
//! The spec calls it "the in-memory model-to-model transform step required to
//! produce a materialised BMM model"
//! (`LANG/docs/bmm_persistence/master02-overview.adoc` §Conceptual Approach).
//!
//! `P_BMM_SCHEMA.create_bmm_model` (precondition
//! `state = P_BMM_PACKAGE_STATE.State_includes_processed`,
//! `LANG/docs/UML/classes/org.openehr.lang.bmm_persistence.p_bmm_schema.adoc`
//! §Functions) — so the schema passed here must already be inclusion-resolved
//! ([`crate::v1_1::bmm_persistence::include_resolution::resolve_includes`]).
//!
//! The transform is where "symbolic referencing via class names, syntactical
//! type names" becomes "the fully computable in-memory object structure with all
//! name references resolved to object references"
//! (`master02-overview.adoc` §Conceptual Approach). Two consequences are pinned
//! here as adjudications, because the openEHR BMM object graph is cyclic while
//! Rust values are not:
//!
//! * **Embedding depth.** `master03-model.adoc` §Overview calls the `bmm_xxx`
//!   attributes "in-memory only references to reconstructed instances", and
//!   `BMM_CLASS.ancestors` is a map of `BMM_CLASS` while
//!   `BMM_SIMPLE_TYPE.base_class` IS a `BMM_CLASS`, so full embedding would not
//!   terminate. A class's ancestors are therefore embedded as resolved copies
//!   carrying their own properties, whose own ancestors are name-bearing stubs,
//!   and every type's base class is such a stub. The complete definition of a
//!   class is always `BMM_MODEL.class_definitions` — "All classes in this
//!   schema" (`org.openehr.lang.bmm.bmm_model.adoc` §Attributes) — which the
//!   model-level lookups already prefer over embedded copies.
//! * **`value_constraint` has no destination in the v2.x generation this
//!   transform targets.** `P_BMM_BASE_TYPE.value_constraint`
//!   (`master04-syntax.adoc` §Value-set Constraints) carries a reference such as
//!   `openEHR::languages`, but the v2 `BMM_SIMPLE_TYPE` declares only
//!   `base_class` (`org.openehr.lang.bmm.bmm_simple_type.adoc` §Attributes) and
//!   no v2 `BMM_*` class references `BMM_VALUE_SET_SPEC` at all. The constraint
//!   is therefore preserved in the P_BMM graph and not carried further — a
//!   boundary of that generation's model, not of the openEHR specs. The v3
//!   generation declares the destination
//!   (`org.openehr.lang.bmm3.bmm_model_type.adoc` §Attributes), so
//!   [`crate::v1_1::bmm_persistence::create_bmm3_model::create_bmm3_model`] is
//!   the transform that keeps it.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use openehr_base::v1_3::prelude::Interval;
use openehr_base::v1_3::prelude::MultiplicityInterval;
use openehr_base::v1_3::prelude::PointInterval;
use openehr_base::v1_3::prelude::ProperInterval;

use crate::v1_1::bmm::core::bmm_class::BmmClass;
use crate::v1_1::bmm::core::bmm_class::BmmClassData;
use crate::v1_1::bmm::core::bmm_container_property::BmmContainerProperty;
use crate::v1_1::bmm::core::bmm_container_type::BmmContainerType;
use crate::v1_1::bmm::core::bmm_container_type::BmmContainerTypeData;
use crate::v1_1::bmm::core::bmm_enumeration::BmmEnumeration;
use crate::v1_1::bmm::core::bmm_enumeration::BmmEnumerationData;
use crate::v1_1::bmm::core::bmm_enumeration_integer::BmmEnumerationInteger;
use crate::v1_1::bmm::core::bmm_enumeration_string::BmmEnumerationString;
use crate::v1_1::bmm::core::bmm_generic_class::BmmGenericClass;
use crate::v1_1::bmm::core::bmm_generic_parameter::BmmGenericParameter;
use crate::v1_1::bmm::core::bmm_generic_type::BmmGenericType;
use crate::v1_1::bmm::core::bmm_indexed_container_type::BmmIndexedContainerType;
use crate::v1_1::bmm::core::bmm_model::BmmModel;
use crate::v1_1::bmm::core::bmm_open_type::BmmOpenType;
use crate::v1_1::bmm::core::bmm_package::BmmPackage;
use crate::v1_1::bmm::core::bmm_property::BmmProperty;
use crate::v1_1::bmm::core::bmm_property::BmmPropertyData;
use crate::v1_1::bmm::core::bmm_simple_type::BmmSimpleType;
use crate::v1_1::bmm::core::bmm_type::BmmType;
use crate::v1_1::bmm_persistence::error::PBmmReadError;
use crate::v1_1::bmm_persistence::p_bmm_base_type::PBmmBaseType;
use crate::v1_1::bmm_persistence::p_bmm_class::PBmmClass;
use crate::v1_1::bmm_persistence::p_bmm_container_property::PBmmContainerProperty;
use crate::v1_1::bmm_persistence::p_bmm_container_type::PBmmContainerType;
use crate::v1_1::bmm_persistence::p_bmm_enumeration::PBmmEnumeration;
use crate::v1_1::bmm_persistence::p_bmm_generic_type::PBmmGenericType;
use crate::v1_1::bmm_persistence::p_bmm_package::PBmmPackage;
use crate::v1_1::bmm_persistence::p_bmm_property::PBmmProperty;
use crate::v1_1::bmm_persistence::p_bmm_schema::PBmmSchema;
use crate::v1_1::bmm_persistence::p_bmm_type::PBmmType;
use openehr_base::containers::present;

/// Materialise the in-memory `BMM_MODEL` of an inclusion-resolved
/// `P_BMM_SCHEMA`.
///
/// # Errors
/// Returns [`PBmmReadError::UnknownAncestor`], [`PBmmReadError::UnknownType`],
/// [`PBmmReadError::ClassNotInAnyPackage`], [`PBmmReadError::ClassNotDefined`],
/// [`PBmmReadError::ContainerTargetTypeMissing`],
/// [`PBmmReadError::TypeDefinitionMissing`],
/// [`PBmmReadError::UndeclaredGenericParameter`] or
/// [`PBmmReadError::NotAGenericClass`] when a symbolic reference in the schema
/// cannot be resolved to the object reference the `BMM_*` shapes require, and
/// [`PBmmReadError::EnumerationAncestorCount`] /
/// [`PBmmReadError::EnumerationItemListsNotOneToOne`] when an enumeration class
/// violates a `BMM_ENUMERATION` validity rule — "may have only one ancestor"
/// (`LANG/docs/bmm3/master07-core-classes.adoc` §Range-Constrained Classes) and
/// `item_values` "Must be 1:1 with `item_names` list"
/// (`org.openehr.lang.bmm.bmm_enumeration.adoc` §Attributes).
pub fn create_bmm_model(schema: &PBmmSchema) -> Result<BmmModel, PBmmReadError> {
    let builder = Builder::new(schema)?;
    // Keyed by each class's OWN name, the form `BMM_CLASS.name` states and the
    // form `BMM_MODEL.class_definition` looks up with; the upper-cased keys of
    // `Builder::classes` are an internal matching index only.
    let mut class_definitions: BTreeMap<String, BmmClass> = BTreeMap::new();
    for entry in builder.classes.values() {
        class_definitions.insert(
            entry.class.name().to_owned(),
            builder.build_class(entry, EmbedDepth::Full)?,
        );
    }
    let packages = builder.build_packages(&schema.packages, "")?;
    Ok(BmmModel {
        rm_publisher: schema.rm_publisher.clone(),
        rm_release: schema.rm_release.clone(),
        schema_name: schema.schema_name.clone(),
        schema_revision: schema.schema_revision.clone(),
        schema_lifecycle_state: schema.schema_lifecycle_state.clone(),
        schema_author: schema.schema_author.clone(),
        schema_description: schema.schema_description.clone(),
        schema_contributors: schema.schema_contributors.clone(),
        archetype_parent_class: schema.archetype_parent_class.clone(),
        archetype_data_value_parent_class: schema.archetype_data_value_parent_class.clone(),
        archetype_rm_closure_packages: schema.archetype_rm_closure_packages.clone(),
        archetype_visualise_descendants_of: schema.archetype_visualise_descendants_of.clone(),
        // P_BMM_SCHEMA carries no `documentation` attribute (class doc
        // §Attributes), so the model's inherited BMM_MODEL_ELEMENT.documentation
        // has no persisted source.
        documentation: None,
        packages: if packages.is_empty() {
            None
        } else {
            Some(packages)
        },
        class_definitions: if class_definitions.is_empty() {
            None
        } else {
            Some(class_definitions)
        },
    })
}

/// How much of a referenced class is embedded in the value being built (see the
/// module docs' embedding-depth adjudication).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmbedDepth {
    /// The class's own definition: properties built, ancestors embedded at
    /// [`EmbedDepth::Ancestor`].
    Full,
    /// An embedded ancestor copy: properties built, ancestors embedded at
    /// [`EmbedDepth::Stub`].
    Ancestor,
    /// A name-bearing stub: no properties, no ancestors, formal generic
    /// parameters resolved with their constrainers at [`EmbedDepth::Bare`].
    Stub,
    /// The innermost stub: name, package and flags only — no properties, no
    /// ancestors and no formal generic parameters.
    ///
    /// This depth exists because `BMM_GENERIC_PARAMETER.conforms_to_type` is
    /// itself a `BMM_CLASS` (`org.openehr.lang.bmm.bmm_generic_parameter.adoc`
    /// §Attributes) that may in turn be generic, so resolving constrainers
    /// without a floor would not terminate on a schema whose parameter
    /// constraints are mutually recursive.
    Bare,
}

/// One class definition of the schema, with the list it was declared in.
pub(super) struct ClassEntry<'a> {
    /// The persisted definition.
    pub(super) class: &'a PBmmClass,
    /// Whether it was declared in `primitive_types` — `BMM_CLASS.is_primitive_type`,
    /// "True if this class is designated a primitive type within the overall type
    /// system of the schema" (`org.openehr.lang.bmm.bmm_class.adoc` §Attributes).
    pub(super) is_primitive_type: bool,
}

/// The resolution indexes the transform needs, all derived from the schema
/// before any `BMM_*` value is built.
///
/// Every class index is keyed by the UPPER-CASED class name, and every
/// name-based lookup upper-cases its argument.
///
/// NOTE: class names in a BMM schema are case-flexible
/// (`master04-syntax.adoc` §Non-primitive Classes: "any capitalisation can be
/// used"), and the model itself keys upper-case "for guaranteed matching"
/// (`org.openehr.lang.bmm.bmm_package_container.adoc` §Attributes) — so
/// resolution is case-insensitive while every name WRITTEN into the produced
/// `BMM_*` values is the class's own `name`, never the upper-cased key.
pub(super) struct Builder<'a> {
    /// The schema being materialised
    /// ([`crate::v1_1::bmm_persistence::p_bmm_schema::PBmmSchema::schema_id`]) — the
    /// `BMM_CLASS.source_schema_id` of a definition that carries none of its
    /// own (see [`Builder::build_class`]).
    pub(super) schema_id: String,
    /// Class definitions by upper-cased name (`primitive_types` first, then
    /// `class_definitions`).
    pub(super) classes: BTreeMap<String, ClassEntry<'a>>,
    /// Upper-cased class name → the fully qualified path of the package that
    /// lists it.
    pub(super) owning_package: BTreeMap<String, String>,
    /// Findings collected while materialising: the v3 transform records here
    /// every persisted assertion string it could not turn into a
    /// `BMM_ASSERTION`. Interior mutability, because a finding is discovered
    /// deep inside an otherwise read-only class walk. The v2.x transform
    /// records none.
    pub(super) findings: std::cell::RefCell<Vec<super::validate::PBmmValidityFinding>>,
    /// Upper-cased class name → immediate inheritance descendants (their own
    /// names), sorted.
    descendants: BTreeMap<String, Vec<String>>,
}

impl<'a> Builder<'a> {
    /// Indexes `schema`'s classes, package membership and inverted inheritance
    /// graph.
    pub(super) fn new(schema: &'a PBmmSchema) -> Result<Self, PBmmReadError> {
        let mut classes: BTreeMap<String, ClassEntry<'a>> = BTreeMap::new();
        for (class, is_primitive_type) in schema
            .primitive_types
            .iter()
            .flatten()
            .map(|class| (class, true))
            .chain(
                schema
                    .class_definitions
                    .iter()
                    .flatten()
                    .map(|class| (class, false)),
            )
        {
            classes
                .entry(class.name().to_uppercase())
                .or_insert(ClassEntry {
                    class,
                    is_primitive_type,
                });
        }

        let mut owning_package: BTreeMap<String, String> = BTreeMap::new();
        index_packages(&schema.packages, "", &classes, &mut owning_package)?;

        let mut descendants: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for entry in classes.values() {
            for parent in ancestor_names(entry.class) {
                let key = parent.to_uppercase();
                if classes.contains_key(&key) {
                    descendants
                        .entry(key)
                        .or_default()
                        .push(entry.class.name().to_owned());
                }
            }
        }
        for names in descendants.values_mut() {
            names.sort_unstable();
            names.dedup();
        }

        Ok(Self {
            schema_id: schema.schema_id(),
            classes,
            owning_package,
            descendants,
            findings: std::cell::RefCell::new(Vec::new()),
        })
    }

    /// The class definition named `name`, matched case-insensitively.
    pub(super) fn entry(
        &self,
        context: &str,
        name: &str,
    ) -> Result<&ClassEntry<'a>, PBmmReadError> {
        self.classes
            .get(&name.to_uppercase())
            .ok_or_else(|| PBmmReadError::UnknownType {
                context: context.to_owned(),
                type_name: name.to_owned(),
            })
    }

    /// The owning-package stub of the class named `name`.
    ///
    /// `BMM_CLASS.package` is `1..1` and `BMM_CLASS.package_path` is the "Fully
    /// qualified package name, of form: 'package.package'" (class doc
    /// §Attributes + §Functions), so the stub carries the class's package path
    /// from the schema root rather than the tree node's own unqualified segment.
    /// A class listed by more than one package keeps the first package the
    /// document declares.
    fn package_of(&self, name: &str) -> Result<BmmPackage, PBmmReadError> {
        let path = self
            .owning_package
            .get(&name.to_uppercase())
            .ok_or_else(|| PBmmReadError::ClassNotInAnyPackage {
                class: name.to_owned(),
            })?;
        Ok(BmmPackage {
            documentation: None,
            packages: None,
            name: path.clone(),
            classes: present(Vec::new()),
        })
    }

    /// Builds one `BMM_CLASS` at the given embedding depth.
    ///
    /// NOTE: a persisted `P_BMM_INTERFACE` materialises as an ABSTRACT class
    /// with no properties — the v2 `BMM_*` model has no interface class
    /// (`master02-overview.adoc` §Conceptual Approach;
    /// `org.openehr.lang.bmm.bmm_class.adoc`), so the abstract-class
    /// projection is the only faithful destination that keeps interface-typed
    /// references resolvable. Its FUNCTIONS stay in the P_BMM graph — the
    /// v2 `BMM_CLASS` declares no function map; the v3 target of
    /// `create_bmm3_model` lands them.
    ///
    /// `BMM_CLASS.source_schema_id` is `1..1` ("Reference to original source
    /// schema defining this class", class doc §Attributes) while an interface
    /// declares no such attribute to stamp, so it takes the id of the schema
    /// being materialised.
    fn build_class(
        &self,
        entry: &ClassEntry<'a>,
        depth: EmbedDepth,
    ) -> Result<BmmClass, PBmmReadError> {
        let persisted = entry.class;
        let name = persisted.name();
        let core = ClassCore {
            documentation: persisted.documentation().map(str::to_owned),
            name: name.to_owned(),
            ancestors: self.build_ancestors(persisted, depth)?,
            package: self.package_of(name)?,
            properties: match depth {
                EmbedDepth::Stub | EmbedDepth::Bare => None,
                EmbedDepth::Full | EmbedDepth::Ancestor => self.build_properties(persisted)?,
            },
            source_schema_id: persisted
                .source_schema_id()
                .unwrap_or(&self.schema_id)
                .to_owned(),
            immediate_descendants: self
                .descendants
                .get(&name.to_uppercase())
                .cloned()
                .unwrap_or_default(),
            is_abstract: persisted.is_abstract(),
            is_primitive_type: entry.is_primitive_type,
            is_override: persisted.is_override(),
        };
        if let PBmmClass::PBmmEnumeration(enumeration) = persisted {
            check_enumeration_validity(name, enumeration)?;
            return Ok(BmmClass::BmmEnumeration(build_enumeration(
                core,
                enumeration,
            )));
        }
        if persisted.is_generic() {
            return Ok(BmmClass::BmmGenericClass(BmmGenericClass {
                documentation: core.documentation,
                name: core.name,
                ancestors: core.ancestors,
                package: core.package,
                properties: core.properties,
                source_schema_id: core.source_schema_id,
                immediate_descendants: present(core.immediate_descendants),
                is_abstract: core.is_abstract,
                is_primitive_type: core.is_primitive_type,
                is_override: core.is_override,
                generic_parameters: self.build_generic_parameters(persisted, depth)?,
            }));
        }
        Ok(BmmClass::BmmClass(BmmClassData {
            documentation: core.documentation,
            name: core.name,
            ancestors: core.ancestors,
            package: core.package,
            properties: core.properties,
            source_schema_id: core.source_schema_id,
            immediate_descendants: present(core.immediate_descendants),
            is_abstract: core.is_abstract,
            is_primitive_type: core.is_primitive_type,
            is_override: core.is_override,
        }))
    }

    /// Builds a class's `BMM_CLASS.ancestors` map at the depth one level below
    /// `depth`; `None` at [`EmbedDepth::Stub`].
    fn build_ancestors(
        &self,
        persisted: &PBmmClass,
        depth: EmbedDepth,
    ) -> Result<Option<BTreeMap<String, BmmClass>>, PBmmReadError> {
        let next = match depth {
            EmbedDepth::Stub | EmbedDepth::Bare => return Ok(None),
            EmbedDepth::Full => EmbedDepth::Ancestor,
            EmbedDepth::Ancestor => EmbedDepth::Stub,
        };
        let mut out: BTreeMap<String, BmmClass> = BTreeMap::new();
        for parent in ancestor_names(persisted) {
            let entry = self.classes.get(&parent.to_uppercase()).ok_or_else(|| {
                PBmmReadError::UnknownAncestor {
                    class: persisted.name().to_owned(),
                    ancestor: parent.clone(),
                }
            })?;
            // Keyed by the ancestor's OWN name, so `BMM_CLASS.all_ancestors`
            // yields names `BMM_MODEL.class_definitions` can be looked up with.
            out.insert(
                entry.class.name().to_owned(),
                self.build_class(entry, next)?,
            );
        }
        Ok(if out.is_empty() { None } else { Some(out) })
    }

    /// Builds a generic class's formal parameter declarations.
    ///
    /// `BMM_GENERIC_CLASS.generic_parameters` is keyed "by name of generic
    /// parameter" (`org.openehr.lang.bmm.bmm_generic_class.adoc` §Attributes)
    /// and `BMM_GENERIC_PARAMETER.conforms_to_type` is an "Optional conformance
    /// constraint that must be another valid class name"
    /// (`…bmm.bmm_generic_parameter.adoc` §Attributes), so the constrainer is
    /// resolved to a class stub.
    fn build_generic_parameters(
        &self,
        persisted: &PBmmClass,
        depth: EmbedDepth,
    ) -> Result<BTreeMap<String, BmmGenericParameter>, PBmmReadError> {
        if depth == EmbedDepth::Bare {
            return Ok(BTreeMap::new());
        }
        let mut out = BTreeMap::new();
        for (key, parameter) in persisted.generic_parameter_defs().into_iter().flatten() {
            out.insert(
                key.clone(),
                self.build_generic_parameter(persisted.name(), parameter)?,
            );
        }
        Ok(out)
    }

    /// Builds one `BMM_GENERIC_PARAMETER`.
    fn build_generic_parameter(
        &self,
        owner: &str,
        parameter: &crate::v1_1::bmm_persistence::p_bmm_generic_parameter::PBmmGenericParameter,
    ) -> Result<BmmGenericParameter, PBmmReadError> {
        let conforms_to_type = match parameter.conforms_to_type.as_deref() {
            None => None,
            Some(constrainer) => {
                let context = format!("class `{owner}` generic parameter `{}`", parameter.name);
                Some(self.build_class(self.entry(&context, constrainer)?, EmbedDepth::Bare)?)
            }
        };
        Ok(BmmGenericParameter {
            documentation: parameter.documentation.clone(),
            name: parameter.name.clone(),
            conforms_to_type,
            inheritance_precursor: None,
        })
    }

    /// Builds a class's `BMM_CLASS.properties` map.
    fn build_properties(
        &self,
        persisted: &PBmmClass,
    ) -> Result<Option<BTreeMap<String, BmmProperty<BmmType>>>, PBmmReadError> {
        let Some(properties) = persisted.properties() else {
            return Ok(None);
        };
        let mut out = BTreeMap::new();
        for (key, property) in properties {
            out.insert(key.clone(), self.build_property(persisted, property)?);
        }
        Ok(if out.is_empty() { None } else { Some(out) })
    }

    /// Builds one `BMM_PROPERTY`.
    ///
    /// A single, open or generic property becomes the least-rich
    /// `BMM_PROPERTY` form over a `BMM_TYPE`: `BMM_UNITARY_PROPERTY` redefines
    /// its type to `BMM_UNITARY_TYPE`
    /// (`org.openehr.lang.bmm3.bmm_unitary_property.adoc` §Attributes), whose
    /// members are the parameter/signature/status/tuple meta-types — not
    /// `BMM_SIMPLE_TYPE` — so a class-typed property is not a unitary property.
    #[expect(
        clippy::too_many_lines,
        reason = "one arm per P_BMM_PROPERTY subtype; splitting the dispatch would hide the five-way persisted-property → BMM_PROPERTY mapping"
    )]
    fn build_property(
        &self,
        owner: &PBmmClass,
        property: &PBmmProperty,
    ) -> Result<BmmProperty<BmmType>, PBmmReadError> {
        let class_name = owner.name();
        match property {
            PBmmProperty::PBmmSingleProperty(single) => {
                let context = property_context(class_name, &single.name);
                let r#type = match (
                    single.type_def.as_ref(),
                    single.type_ref.as_ref(),
                    single.r#type.as_deref(),
                ) {
                    (Some(type_def), _, _) => self.build_type(&context, type_def, owner)?,
                    (None, Some(type_ref), _) => {
                        self.build_named_type(&context, &type_ref.r#type, owner)?
                    }
                    (None, None, Some(name)) => self.build_named_type(&context, name, owner)?,
                    (None, None, None) => {
                        return Err(PBmmReadError::TypeDefinitionMissing { context });
                    }
                };
                Ok(BmmProperty::BmmProperty(BmmPropertyData {
                    documentation: single.documentation.clone(),
                    name: single.name.clone(),
                    is_mandatory: single.is_mandatory,
                    is_computed: single.is_computed,
                    r#type,
                    is_im_runtime: single.is_im_runtime,
                    is_im_infrastructure: single.is_im_infrastructure,
                }))
            }
            PBmmProperty::PBmmSinglePropertyOpen(open) => {
                let context = property_context(class_name, &open.name);
                let parameter = match (open.type_ref.as_ref(), open.r#type.as_deref()) {
                    (Some(type_ref), _) => type_ref.r#type.as_str(),
                    (None, Some(name)) => name,
                    (None, None) => {
                        return Err(PBmmReadError::TypeDefinitionMissing { context });
                    }
                };
                let r#type =
                    BmmType::BmmOpenType(self.build_open_type(owner, &open.name, parameter)?);
                Ok(BmmProperty::BmmProperty(BmmPropertyData {
                    documentation: open.documentation.clone(),
                    name: open.name.clone(),
                    is_mandatory: open.is_mandatory,
                    is_computed: open.is_computed,
                    r#type,
                    is_im_runtime: open.is_im_runtime,
                    is_im_infrastructure: open.is_im_infrastructure,
                }))
            }
            PBmmProperty::PBmmGenericProperty(generic) => {
                let context = property_context(class_name, &generic.name);
                let Some(type_def) = generic.type_def.as_ref() else {
                    return Err(PBmmReadError::TypeDefinitionMissing { context });
                };
                let r#type =
                    BmmType::BmmGenericType(self.build_generic_type(&context, type_def, owner)?);
                Ok(BmmProperty::BmmProperty(BmmPropertyData {
                    documentation: generic.documentation.clone(),
                    name: generic.name.clone(),
                    is_mandatory: generic.is_mandatory,
                    is_computed: generic.is_computed,
                    r#type,
                    is_im_runtime: generic.is_im_runtime,
                    is_im_infrastructure: generic.is_im_infrastructure,
                }))
            }
            PBmmProperty::PBmmContainerProperty(PBmmContainerProperty::PBmmContainerProperty(
                container,
            )) => {
                let context = property_context(class_name, &container.name);
                let Some(type_def) = container.type_def.as_ref() else {
                    return Err(PBmmReadError::TypeDefinitionMissing { context });
                };
                let r#type = self.build_container_type(&context, type_def, owner)?;
                Ok(BmmProperty::BmmContainerProperty(BmmContainerProperty {
                    documentation: container.documentation.clone(),
                    name: container.name.clone(),
                    is_mandatory: container.is_mandatory,
                    is_computed: container.is_computed,
                    r#type,
                    is_im_runtime: container.is_im_runtime,
                    is_im_infrastructure: container.is_im_infrastructure,
                    cardinality: container.cardinality.as_ref().map(multiplicity_of),
                }))
            }
            // NOTE: the v2 feature model has no indexed container PROPERTY, so the
            // index lives on the TYPE — `BMM_INDEXED_CONTAINER_TYPE` IS a
            // `BMM_CONTAINER_TYPE` (`…bmm.bmm_indexed_container_type.adoc` §Inherit).
            PBmmProperty::PBmmContainerProperty(
                PBmmContainerProperty::PBmmIndexedContainerProperty(indexed),
            ) => {
                let context = property_context(class_name, &indexed.name);
                let Some(type_def) = indexed.type_def.as_ref() else {
                    return Err(PBmmReadError::TypeDefinitionMissing { context });
                };
                let r#type = BmmContainerType::BmmIndexedContainerType(Box::new(
                    self.build_indexed_container_type(&context, type_def, owner)?,
                ));
                Ok(BmmProperty::BmmContainerProperty(BmmContainerProperty {
                    documentation: indexed.documentation.clone(),
                    name: indexed.name.clone(),
                    is_mandatory: indexed.is_mandatory,
                    is_computed: indexed.is_computed,
                    r#type,
                    is_im_runtime: indexed.is_im_runtime,
                    is_im_infrastructure: indexed.is_im_infrastructure,
                    cardinality: indexed.cardinality.as_ref().map(multiplicity_of),
                }))
            }
        }
    }

    /// Builds a `BMM_TYPE` from a bare type NAME.
    ///
    /// A name that is a formal generic parameter of `owner` (or of an ancestor)
    /// becomes a `BMM_OPEN_TYPE` — `master04-syntax.adoc` §Inheritance writes
    /// exactly that in a generic ancestor's parameter list
    /// (`generic_parameters = <"T", "SUPPLIER_B">`, "the ancestors are generic
    /// types, which may be open, partially closed or fully closed"); any other
    /// name is a class reference and becomes a `BMM_SIMPLE_TYPE`.
    fn build_named_type(
        &self,
        context: &str,
        name: &str,
        owner: &PBmmClass,
    ) -> Result<BmmType, PBmmReadError> {
        if let Some(parameter) = self.find_generic_parameter(owner, name) {
            return Ok(BmmType::BmmOpenType(BmmOpenType {
                documentation: None,
                generic_constraint: self.build_generic_parameter(owner.name(), parameter)?,
            }));
        }
        Ok(BmmType::BmmSimpleType(BmmSimpleType {
            documentation: None,
            base_class: self.build_class(self.entry(context, name)?, EmbedDepth::Stub)?,
        }))
    }

    /// Builds a `BMM_TYPE` from a persisted `P_BMM_TYPE`.
    fn build_type(
        &self,
        context: &str,
        r#type: &PBmmType,
        owner: &PBmmClass,
    ) -> Result<BmmType, PBmmReadError> {
        match r#type {
            PBmmType::PBmmSimpleType(simple) => {
                self.build_named_type(context, &simple.r#type, owner)
            }
            PBmmType::PBmmOpenType(open) => Ok(BmmType::BmmOpenType(self.build_open_type(
                owner,
                context,
                &open.r#type,
            )?)),
            PBmmType::PBmmGenericType(generic) => Ok(BmmType::BmmGenericType(
                self.build_generic_type(context, generic, owner)?,
            )),
            PBmmType::PBmmContainerType(container) => Ok(BmmType::BmmContainerType(Box::new(
                self.build_container_type(context, container, owner)?,
            ))),
        }
    }

    /// Builds a `BMM_TYPE` from a persisted `P_BMM_BASE_TYPE`.
    fn build_base_type(
        &self,
        context: &str,
        r#type: &PBmmBaseType,
        owner: &PBmmClass,
    ) -> Result<BmmType, PBmmReadError> {
        match r#type {
            PBmmBaseType::PBmmSimpleType(simple) => {
                self.build_named_type(context, &simple.r#type, owner)
            }
            PBmmBaseType::PBmmOpenType(open) => Ok(BmmType::BmmOpenType(self.build_open_type(
                owner,
                context,
                &open.r#type,
            )?)),
            PBmmBaseType::PBmmGenericType(generic) => Ok(BmmType::BmmGenericType(
                self.build_generic_type(context, generic, owner)?,
            )),
        }
    }

    /// Builds a `BMM_OPEN_TYPE` for the formal parameter named `parameter`.
    ///
    /// `BMM_OPEN_TYPE.generic_constraint` is "The generic constraint, which will
    /// be 'Any' if nothing set in original model"
    /// (`org.openehr.lang.bmm.bmm_open_type.adoc` §Attributes) and "The
    /// parameter must be in the type declaration of the owning `BMM_CLASS`"
    /// (same class doc, §Description) — hence the refusal when it is not.
    fn build_open_type(
        &self,
        owner: &PBmmClass,
        property: &str,
        parameter: &str,
    ) -> Result<BmmOpenType, PBmmReadError> {
        let declared = self
            .find_generic_parameter(owner, parameter)
            .ok_or_else(|| PBmmReadError::UndeclaredGenericParameter {
                class: owner.name().to_owned(),
                property: property.to_owned(),
                parameter: parameter.to_owned(),
            })?;
        Ok(BmmOpenType {
            documentation: None,
            generic_constraint: self.build_generic_parameter(owner.name(), declared)?,
        })
    }

    /// The formal generic parameter named `parameter`, declared by `owner` or by
    /// any of its ancestors.
    ///
    /// `BMM_GENERIC_CLASS.generic_parameters` are "defined either directly on
    /// this class or by the addition of an ancestor class which is generic"
    /// (`org.openehr.lang.bmm.bmm_generic_class.adoc` §Attributes), so the
    /// lookup walks the ancestor graph; it is cycle-safe.
    pub(super) fn find_generic_parameter(
        &self,
        owner: &PBmmClass,
        parameter: &str,
    ) -> Option<&'a crate::v1_1::bmm_persistence::p_bmm_generic_parameter::PBmmGenericParameter>
    {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut queue: Vec<String> = vec![owner.name().to_uppercase()];
        while let Some(key) = queue.pop() {
            if !seen.insert(key.clone()) {
                continue;
            }
            let Some(entry) = self.classes.get(&key) else {
                continue;
            };
            if let Some(found) = entry
                .class
                .generic_parameter_defs()
                .and_then(|defs| defs.get(parameter))
            {
                return Some(found);
            }
            for parent in ancestor_names(entry.class) {
                queue.push(parent.to_uppercase());
            }
        }
        None
    }

    /// Builds a `BMM_GENERIC_TYPE`.
    ///
    /// `BMM_GENERIC_TYPE.base_class` is a `BMM_GENERIC_CLASS`
    /// (`org.openehr.lang.bmm.bmm_generic_type.adoc` §Attributes), so a
    /// `root_type` naming a non-generic class is refused. The actual parameters
    /// are the string list followed by the structural list, in that order —
    /// "use `_generic_parameters_` for a list of string types; use
    /// `_generic_parameter_defs_` for a list of complex type references"
    /// (`master04-syntax.adoc` §Generic Classes).
    fn build_generic_type(
        &self,
        context: &str,
        generic: &PBmmGenericType,
        owner: &PBmmClass,
    ) -> Result<BmmGenericType, PBmmReadError> {
        let entry = self.entry(context, &generic.root_type)?;
        let BmmClass::BmmGenericClass(base_class) = self.build_class(entry, EmbedDepth::Stub)?
        else {
            return Err(PBmmReadError::NotAGenericClass {
                context: context.to_owned(),
                type_name: generic.root_type.clone(),
            });
        };
        let mut generic_parameters: Vec<BmmType> = Vec::new();
        for name in generic.generic_parameters.iter().flatten() {
            generic_parameters.push(self.build_named_type(context, name, owner)?);
        }
        for parameter in &generic.generic_parameter_defs {
            generic_parameters.push(self.build_type(context, parameter, owner)?);
        }
        Ok(BmmGenericType {
            documentation: None,
            generic_parameters,
            base_class,
        })
    }

    /// Builds a `BMM_CONTAINER_TYPE`.
    fn build_container_type(
        &self,
        context: &str,
        container: &PBmmContainerType,
        owner: &PBmmClass,
    ) -> Result<BmmContainerType, PBmmReadError> {
        match container {
            PBmmContainerType::PBmmIndexedContainerType(indexed) => {
                Ok(BmmContainerType::BmmIndexedContainerType(Box::new(
                    self.build_indexed_container_type(context, indexed, owner)?,
                )))
            }
            PBmmContainerType::PBmmContainerType(data) => {
                Ok(BmmContainerType::BmmContainerType(BmmContainerTypeData {
                    documentation: None,
                    container_type: self.build_class(
                        self.entry(context, &data.container_type)?,
                        EmbedDepth::Stub,
                    )?,
                    base_type: Box::new(self.build_container_target(
                        context,
                        data.r#type.as_deref(),
                        data.type_def.as_ref(),
                        owner,
                    )?),
                }))
            }
        }
    }

    /// Builds a `BMM_INDEXED_CONTAINER_TYPE`.
    ///
    /// `BMM_INDEXED_CONTAINER_TYPE.index_type` is a `BMM_SIMPLE_TYPE` — "The key
    /// (index) type of the container, e.g. `String` in
    /// `Hash<String,EVENT_ACTION>`"
    /// (`org.openehr.lang.bmm.bmm_indexed_container_type.adoc` §Attributes).
    fn build_indexed_container_type(
        &self,
        context: &str,
        indexed: &crate::v1_1::bmm_persistence::p_bmm_indexed_container_type::PBmmIndexedContainerType,
        owner: &PBmmClass,
    ) -> Result<BmmIndexedContainerType, PBmmReadError> {
        Ok(BmmIndexedContainerType {
            documentation: None,
            container_type: self.build_class(
                self.entry(context, &indexed.container_type)?,
                EmbedDepth::Stub,
            )?,
            base_type: self.build_container_target(
                context,
                indexed.r#type.as_deref(),
                indexed.type_def.as_ref(),
                owner,
            )?,
            index_type: BmmSimpleType {
                documentation: None,
                base_class: self
                    .build_class(self.entry(context, &indexed.index_type)?, EmbedDepth::Stub)?,
            },
        })
    }

    /// Builds a container's target type from its `type` name or nested
    /// `type_def`.
    fn build_container_target(
        &self,
        context: &str,
        name: Option<&str>,
        type_def: Option<&PBmmBaseType>,
        owner: &PBmmClass,
    ) -> Result<BmmType, PBmmReadError> {
        match (type_def, name) {
            (Some(nested), _) => self.build_base_type(context, nested, owner),
            (None, Some(name)) => self.build_named_type(context, name, owner),
            (None, None) => Err(PBmmReadError::ContainerTargetTypeMissing {
                context: context.to_owned(),
            }),
        }
    }

    /// Builds the `BMM_PACKAGE` tree, keyed in upper case.
    ///
    /// `BMM_PACKAGE_CONTAINER.packages`: "Child packages; keys all in upper case
    /// for guaranteed matching"
    /// (`org.openehr.lang.bmm.bmm_package_container.adoc` §Attributes).
    fn build_packages(
        &self,
        packages: &BTreeMap<String, PBmmPackage>,
        prefix: &str,
    ) -> Result<BTreeMap<String, BmmPackage>, PBmmReadError> {
        let mut out = BTreeMap::new();
        for package in packages.values() {
            let path = qualify(prefix, &package.name);
            let mut classes = Vec::new();
            for class in package.classes.iter().flatten() {
                let entry = self.classes.get(&class.to_uppercase()).ok_or_else(|| {
                    PBmmReadError::ClassNotDefined {
                        package: package.name.clone(),
                        class: class.clone(),
                    }
                })?;
                classes.push(self.build_class(entry, EmbedDepth::Full)?);
            }
            let children = self.build_packages(&package.packages, &path)?;
            out.insert(
                package.name.to_uppercase(),
                BmmPackage {
                    documentation: package.documentation.clone(),
                    packages: if children.is_empty() {
                        None
                    } else {
                        Some(children)
                    },
                    name: package.name.clone(),
                    classes: present(classes),
                },
            );
        }
        Ok(out)
    }
}

/// The `BMM_CLASS` attributes every concrete class form carries.
struct ClassCore {
    /// `BMM_MODEL_ELEMENT.documentation`.
    documentation: Option<String>,
    /// `BMM_CLASS.name`.
    name: String,
    /// `BMM_CLASS.ancestors`.
    ancestors: Option<BTreeMap<String, BmmClass>>,
    /// `BMM_CLASS.package`.
    package: BmmPackage,
    /// `BMM_CLASS.properties`.
    properties: Option<BTreeMap<String, BmmProperty<BmmType>>>,
    /// `BMM_CLASS.source_schema_id`.
    source_schema_id: String,
    /// `BMM_CLASS.immediate_descendants`.
    immediate_descendants: Vec<String>,
    /// `BMM_CLASS.is_abstract`.
    is_abstract: bool,
    /// `BMM_CLASS.is_primitive_type`.
    is_primitive_type: bool,
    /// `BMM_CLASS.is_override`.
    is_override: bool,
}

/// Checks the two `BMM_ENUMERATION` validity rules a persisted enumeration must
/// satisfy before it can be materialised.
///
/// * "may have only one ancestor" (`LANG/docs/bmm3/master07-core-classes.adoc`
///   §Range-Constrained Classes; `org.openehr.lang.bmm3.bmm_enumeration.adoc`
///   §Description) — the ancestor provides the base type the range constraint
///   applies to, which is what `BMM_ENUMERATION.underlying_type_name` names
///   (`org.openehr.lang.bmm.bmm_enumeration.adoc` §Attributes), so two ancestors
///   leave it ambiguous. An enumeration with NO ancestor stays legal: the v2
///   class doc's "It is designed so that the default type is Integer"
///   (§Description) supplies the base type
///   ([`crate::v1_1::bmm_persistence::p_bmm_enumeration_impl::DEFAULT_UNDERLYING_TYPE_NAME`]).
/// * `item_values` "Must be 1:1 with `item_names` list" (same §Attributes) —
///   checked only when values are stated, since "If no values are supplied, the
///   integer values 0, 1, 2, ... are assumed".
///
/// # Errors
/// [`PBmmReadError::EnumerationAncestorCount`] or
/// [`PBmmReadError::EnumerationItemListsNotOneToOne`].
pub(super) fn check_enumeration_validity(
    name: &str,
    persisted: &PBmmEnumeration,
) -> Result<(), PBmmReadError> {
    let ancestors = persisted.ancestors();
    if ancestors.len() > 1 {
        return Err(PBmmReadError::EnumerationAncestorCount {
            class: name.to_owned(),
            ancestors: ancestors.to_vec(),
        });
    }
    let names = persisted.item_names().len();
    let values = persisted.item_values().len();
    if values > 0 && values != names {
        return Err(PBmmReadError::EnumerationItemListsNotOneToOne {
            class: name.to_owned(),
            names,
            values,
        });
    }
    Ok(())
}

/// Builds the `BMM_ENUMERATION` form matching the persisted enumeration kind.
fn build_enumeration(core: ClassCore, persisted: &PBmmEnumeration) -> BmmEnumeration {
    let item_names = persisted.item_names().to_vec();
    let item_values = persisted.item_values().to_vec();
    let underlying_type_name = persisted.underlying_type_name().to_owned();
    match persisted {
        PBmmEnumeration::PBmmEnumerationInteger(_) => {
            BmmEnumeration::BmmEnumerationInteger(BmmEnumerationInteger {
                documentation: core.documentation,
                name: core.name,
                ancestors: core.ancestors,
                package: core.package,
                properties: core.properties,
                source_schema_id: core.source_schema_id,
                immediate_descendants: present(core.immediate_descendants),
                is_abstract: core.is_abstract,
                is_primitive_type: core.is_primitive_type,
                is_override: core.is_override,
                item_names: present(item_names),
                item_values: present(item_values),
                underlying_type_name,
            })
        }
        PBmmEnumeration::PBmmEnumerationString(_) => {
            BmmEnumeration::BmmEnumerationString(BmmEnumerationString {
                documentation: core.documentation,
                name: core.name,
                ancestors: core.ancestors,
                package: core.package,
                properties: core.properties,
                source_schema_id: core.source_schema_id,
                immediate_descendants: present(core.immediate_descendants),
                is_abstract: core.is_abstract,
                is_primitive_type: core.is_primitive_type,
                is_override: core.is_override,
                item_names: present(item_names),
                item_values: present(item_values),
                underlying_type_name,
            })
        }
        PBmmEnumeration::PBmmEnumeration(_) => BmmEnumeration::BmmEnumeration(BmmEnumerationData {
            documentation: core.documentation,
            name: core.name,
            ancestors: core.ancestors,
            package: core.package,
            properties: core.properties,
            source_schema_id: core.source_schema_id,
            immediate_descendants: present(core.immediate_descendants),
            is_abstract: core.is_abstract,
            is_primitive_type: core.is_primitive_type,
            is_override: core.is_override,
            item_names: present(item_names),
            item_values: present(item_values),
            underlying_type_name,
        }),
    }
}

/// Indexes which package lists each class, recursively; refuses a listed class
/// the schema does not define.
///
/// `master04-syntax.adoc` §Package Definition, second NOTE: "only classes
/// defined in the same schema can be referenced in the package section in that
/// schema."
fn index_packages(
    packages: &BTreeMap<String, PBmmPackage>,
    prefix: &str,
    classes: &BTreeMap<String, ClassEntry<'_>>,
    out: &mut BTreeMap<String, String>,
) -> Result<(), PBmmReadError> {
    for package in packages.values() {
        let path = qualify(prefix, &package.name);
        for class in package.classes.iter().flatten() {
            let key = class.to_uppercase();
            if !classes.contains_key(&key) {
                return Err(PBmmReadError::ClassNotDefined {
                    package: package.name.clone(),
                    class: class.clone(),
                });
            }
            out.entry(key).or_insert_with(|| path.clone());
        }
        index_packages(&package.packages, &path, classes, out)?;
    }
    Ok(())
}

/// Joins a package name onto its parent path with the
/// `BMM_DEFINITIONS.Package_name_delimiter` (`"."`,
/// `org.openehr.lang.bmm.bmm_definitions.adoc` §Constants).
pub(super) fn qualify(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_owned()
    } else {
        format!(
            "{prefix}{}{name}",
            crate::v1_1::bmm::core::bmm_definitions::BmmDefinitionsData::PACKAGE_NAME_DELIMITER
        )
    }
}

/// Every inheritance parent a persisted class names, from `ancestors` and from
/// the `root_type` of each `ancestor_defs` entry.
///
/// NOTE: the v2 `BMM_CLASS.ancestors` is a map of CLASSES
/// (`org.openehr.lang.bmm.bmm_class.adoc` §Attributes), so a generic
/// ancestor's parameter binding has nowhere to land here — only the root
/// class is carried, the binding staying readable in `P_BMM_CLASS.
/// ancestor_defs`; the v3 generation types `ancestors` as TYPES
/// (`…bmm3.bmm_class.adoc`) and `create_bmm3_model` carries the binding into
/// it — a boundary of this transform's target generation, not the pipeline.
pub(super) fn ancestor_names(class: &PBmmClass) -> Vec<String> {
    let mut out: Vec<String> = class.ancestors().to_vec();
    out.extend(
        class
            .ancestor_defs()
            .iter()
            .map(|def| def.root_type.clone()),
    );
    out
}

/// The error context naming one class property.
pub(super) fn property_context(class: &str, property: &str) -> String {
    format!("class `{class}` property `{property}`")
}

/// The `Multiplicity_interval` form of a persisted `Interval<Integer>`
/// cardinality.
///
/// `BMM_CONTAINER_PROPERTY.cardinality` is a `Multiplicity_interval`
/// (`org.openehr.lang.bmm.bmm_container_property.adoc` §Attributes) — "An
/// Interval of Integer, used to represent multiplicity, cardinality and
/// optionality in models" (`BASE` `Multiplicity_interval`).
pub(super) fn multiplicity_of(interval: &Interval<i32>) -> MultiplicityInterval {
    match interval {
        Interval::PointInterval(PointInterval {
            lower,
            upper,
            lower_unbounded,
            upper_unbounded,
            lower_included,
            upper_included,
        }) => MultiplicityInterval {
            lower: *lower,
            upper: *upper,
            lower_unbounded: *lower_unbounded,
            upper_unbounded: *upper_unbounded,
            lower_included: *lower_included,
            upper_included: *upper_included,
        },
        Interval::ProperInterval(ProperInterval::MultiplicityInterval(multiplicity)) => {
            multiplicity.clone()
        }
        Interval::ProperInterval(ProperInterval::ProperInterval(proper)) => MultiplicityInterval {
            lower: proper.lower,
            upper: proper.upper,
            lower_unbounded: proper.lower_unbounded,
            upper_unbounded: proper.upper_unbounded,
            lower_included: proper.lower_included,
            upper_included: proper.upper_included,
        },
    }
}
