// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written spec functions of `SCHEMA_DESCRIPTOR` — the descriptor
//! lifecycle of the `rm_access` schema-loading facade.
//!
//! Spec: `LANG/docs/UML/classes/org.openehr.lang.bmm.schema_descriptor.adoc`
//! ("Descriptor for a BMM schema. Contains a meta-data table of attributes
//! obtained from a mini-ODIN parse of the schema file") §Attributes (`p_schema`,
//! `schema`, `schema_id`, `meta_data`, `includes`) and §Functions
//! (`is_top_level`, `is_bmm_compatible`, `load`, `validate`,
//! `validate_includes`, `create_schema`), within
//! `LANG/docs/bmm/master04-rm_access.adoc` §Overview: the package "provides an
//! interface for the application to load and access BMM schemas".
//!
//! The behaviour is realized ON the generated data classes (this file's
//! `*_impl.rs` sibling
//! [`crate::v1_1::bmm::rm_access::schema_descriptor::SchemaDescriptor`]), which already
//! carry every attribute the class doc declares — including `meta_data`'s
//! `schema_path`, which is where the file to (re)read is remembered.
//!
//! Two signature deviations from the class doc, both because the pinned
//! `SCHEMA_DESCRIPTOR` declares no attribute the zero-argument forms could read
//! (§Attributes lists exactly `p_schema`, `schema`, `schema_id`, `meta_data`,
//! `includes` — no `is_included` flag and no back-reference to the loader):
//! [`SchemaDescriptor::is_top_level`] takes the candidate set, and
//! [`SchemaDescriptor::create_schema`] takes the persisted forms its inclusions
//! resolve against. NOTE: no openEHR spec governs these parameters — our own
//! design/extension; the alternative (a synthetic `meta_data` key, or a shadow
//! field) would invent model state the spec does not declare.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;

use crate::v1_1::bmm::core::bmm_definitions::BmmDefinitionsData;
use crate::v1_1::bmm::core::bmm_model::BmmModel;
use crate::v1_1::bmm::rm_access::error::RmAccessError;
use crate::v1_1::bmm::rm_access::schema_descriptor::SchemaDescriptor;
use crate::v1_1::bmm_persistence::create_model::create_bmm_model;
use crate::v1_1::bmm_persistence::include_resolution::resolve_includes;
use crate::v1_1::bmm_persistence::p_bmm_schema::PBmmSchema;
use crate::v1_1::bmm_persistence::p_bmm_schema_descriptor::PBmmSchemaDescriptor;
use crate::v1_1::bmm_persistence::reader::read_schema;
use openehr_base::containers::present;

/// The P_BMM generation this reader implements, as its major version.
///
/// NOTE: `BMM_DEFINITIONS.Bmm_internal_version` — the intended comparison
/// operand — is declared with NO VALUE in the pinned generation
/// (`org.openehr.lang.bmm.bmm_definitions.adoc` §Constants), so compatibility
/// is judged on the MAJOR component only (within a major line a release is a
/// compatible superset —
/// <https://specifications.openehr.org/governance/release_strategy>); where
/// the constant does state a version it wins, and the empty-constant fallback
/// is our own design/extension.
pub const P_BMM_GENERATION: &str = "2";

/// The `meta_data` key holding the schema file's path.
///
/// The key set is the one `SCHEMA_DESCRIPTOR.meta_data` enumerates: "Table of
/// {key, value} pairs of schema meta-data (bmm_version, model_publisher,
/// schema_name, model_release, schema_revision, schema_lifecycle_state,
/// schema_description, schema_path)" (class doc §Attributes).
pub const META_SCHEMA_PATH: &str = "schema_path";

/// The `meta_data` key holding the P_BMM version the schema conforms to.
pub const META_BMM_VERSION: &str = "bmm_version";

/// The `meta_data` key holding the publishing organisation.
///
/// NOTE (adjudicated): the class doc spells the two identity keys
/// `model_publisher` and `model_release` while the schema attributes they come
/// from are `P_BMM_SCHEMA.rm_publisher` and `rm_release`
/// (`…p_bmm_schema.adoc` §Attributes, and `master04-syntax.adoc` §Header Items,
/// which writes `rm_publisher`/`rm_release` in the file). The KEYS follow the
/// class doc, the VALUES come from those attributes.
pub const META_MODEL_PUBLISHER: &str = "model_publisher";

/// The `meta_data` key holding the schema's own name.
pub const META_SCHEMA_NAME: &str = "schema_name";

/// The `meta_data` key holding the model release.
pub const META_MODEL_RELEASE: &str = "model_release";

/// The `meta_data` key holding the schema revision.
pub const META_SCHEMA_REVISION: &str = "schema_revision";

/// The `meta_data` key holding the schema's lifecycle state.
pub const META_SCHEMA_LIFECYCLE_STATE: &str = "schema_lifecycle_state";

/// The `meta_data` key holding the schema description.
pub const META_SCHEMA_DESCRIPTION: &str = "schema_description";

/// Reads one attribute of whichever `SCHEMA_DESCRIPTOR` leaf a
/// [`SchemaDescriptor`] holds — both leaves carry the same inherited attribute
/// set.
macro_rules! descriptor {
    ($value:expr, |$leaf:ident| $body:expr) => {
        match $value {
            SchemaDescriptor::PBmmSchemaDescriptor($leaf) => $body,
            SchemaDescriptor::SchemaDescriptor($leaf) => $body,
        }
    };
}

impl SchemaDescriptor {
    /// Describe the schema file at `path` from a metadata read of its header.
    ///
    /// This is the "meta-data table of attributes obtained from a mini-ODIN
    /// parse of the schema file" the class doc describes: the returned
    /// descriptor carries `schema_id`, `meta_data` and `includes`, and NOTHING
    /// of the model itself — populating `p_schema` is [`SchemaDescriptor::load`]'s
    /// job, the next step of the lifecycle.
    ///
    /// NOTE (adjudicated): the P_BMM reader is whole-document
    /// ([`crate::v1_1::bmm_persistence::reader::read_schema`]), and the pinned syntax
    /// offers no partial-parse entry point — `master04-syntax.adoc`
    /// §Serialisation Formats describes one file shape whose "File heading" is
    /// simply its first attributes — so the metadata read is realized by reading
    /// the whole document and keeping only the header items. The observable
    /// behaviour is the class doc's: a descriptor that is not yet loaded.
    ///
    /// The built leaf is `P_BMM_SCHEMA_DESCRIPTOR`, the class the spec defines for
    /// exactly this job: "Concrete descendant of `BMM_SCHEMA_DESCRIPTOR` that
    /// provides a way to read an ODIN or other similarly encoded P_BMM schema
    /// file"
    /// (`org.openehr.lang.bmm_persistence.p_bmm_schema_descriptor.adoc`
    /// §Description) — exactly what this facade does. Its own `bmm_schema`
    /// attribute re-declares the inherited `p_schema` verbatim ("Persistent form
    /// of model"), so only the inherited slot is populated and
    /// [`SchemaDescriptor::p_schema`] reads either.
    ///
    /// # Errors
    /// Returns [`RmAccessError::File`] when `path` cannot be read and
    /// [`RmAccessError::Schema`] when its content is not a P_BMM schema.
    pub fn from_schema_file(path: &Path) -> Result<Self, RmAccessError> {
        let display = path.display().to_string();
        let src = std::fs::read_to_string(path).map_err(|source| RmAccessError::File {
            path: display.clone(),
            source,
        })?;
        let schema = read_schema(&src).map_err(|source| RmAccessError::Schema {
            path: display.clone(),
            source,
        })?;
        Ok(Self::PBmmSchemaDescriptor(PBmmSchemaDescriptor {
            p_schema: None,
            schema: None,
            schema_id: schema.schema_id(),
            meta_data: meta_data_of(&schema, &display),
            includes: present(include_ids_of(&schema)),
            bmm_schema: None,
        }))
    }

    /// `SCHEMA_DESCRIPTOR.schema_id`: "Schema id, formed from meta_data
    /// model_publisher '_' schema_name '_' model_release, e.g. openehr_rm_1.0.3"
    /// (class doc §Attributes).
    #[must_use]
    pub fn schema_id(&self) -> &str {
        descriptor!(self, |leaf| leaf.schema_id.as_str())
    }

    /// `SCHEMA_DESCRIPTOR.meta_data`: the "Table of {key, value} pairs of
    /// schema meta-data" (class doc §Attributes) — see [`META_SCHEMA_PATH`] and
    /// the sibling key constants.
    #[must_use]
    pub fn meta_data(&self) -> &BTreeMap<String, String> {
        descriptor!(self, |leaf| &leaf.meta_data)
    }

    /// `SCHEMA_DESCRIPTOR.includes`: "Schema_ids of schemas included by this
    /// schema" (class doc §Attributes).
    #[must_use]
    pub fn includes(&self) -> &[String] {
        descriptor!(self, |leaf| leaf.includes.as_deref().unwrap_or_default())
    }

    /// The path of the file this descriptor was read from
    /// (the [`META_SCHEMA_PATH`] entry of `meta_data`).
    #[must_use]
    pub fn schema_path(&self) -> Option<&str> {
        self.meta_data().get(META_SCHEMA_PATH).map(String::as_str)
    }

    /// `SCHEMA_DESCRIPTOR.p_schema`: "Persistent form of model" (class doc
    /// §Attributes) — `None` until [`SchemaDescriptor::load`] has run.
    ///
    /// `P_BMM_SCHEMA_DESCRIPTOR.bmm_schema` re-declares the same attribute with
    /// the same meaning, so it is read as a fallback (see
    /// [`SchemaDescriptor::from_schema_file`]).
    #[must_use]
    pub fn p_schema(&self) -> Option<&PBmmSchema> {
        match self {
            SchemaDescriptor::PBmmSchemaDescriptor(leaf) => {
                leaf.p_schema.as_ref().or(leaf.bmm_schema.as_ref())
            }
            SchemaDescriptor::SchemaDescriptor(leaf) => leaf.p_schema.as_ref(),
        }
    }

    /// `SCHEMA_DESCRIPTOR.schema`: "Computable form of model" (class doc
    /// §Attributes) — `None` until [`SchemaDescriptor::create_schema`] has run.
    #[must_use]
    pub fn schema(&self) -> Option<&BmmModel> {
        descriptor!(self, |leaf| leaf.schema.as_ref())
    }

    /// `SCHEMA_DESCRIPTOR.is_top_level`: "True if this is a top-level schema,
    /// i.e. not included by some other schema" (class doc §Functions).
    ///
    /// `all_schemas` is the candidate set — conventionally
    /// [`crate::v1_1::bmm::rm_access::reference_model_access::ReferenceModelAccess::all_schemas`]
    /// — matched case-insensitively, because the vendored schemas write include
    /// ids in either case and
    /// [`crate::v1_1::bmm_persistence::include_resolution::resolve_includes`] matches
    /// them lower-cased. A schema that includes ITSELF does not thereby stop
    /// being top-level.
    #[must_use]
    pub fn is_top_level(&self, all_schemas: &BTreeMap<String, SchemaDescriptor>) -> bool {
        let own = self.schema_id().to_lowercase();
        !all_schemas.values().any(|other| {
            other.schema_id().to_lowercase() != own
                && other.includes().iter().any(|id| id.to_lowercase() == own)
        })
    }

    /// `SCHEMA_DESCRIPTOR.is_bmm_compatible`: "True if the BMM version found in
    /// the schema (or assumed, if none) is compatible with that in this
    /// software" (class doc §Functions).
    ///
    /// Compared on the major version only, against [`P_BMM_GENERATION`] (see its
    /// adjudication). A descriptor whose `meta_data` states no `bmm_version` is
    /// compatible — the class doc's "or assumed, if none".
    #[must_use]
    pub fn is_bmm_compatible(&self) -> bool {
        match self.meta_data().get(META_BMM_VERSION) {
            None => true,
            Some(found) => major_of(found) == expected_generation(),
        }
    }

    /// `SCHEMA_DESCRIPTOR.load`: "Load schema into in-memory form" (class doc
    /// §Functions) — reads the file named by
    /// the [`META_SCHEMA_PATH`] entry of `meta_data`
    /// into `p_schema`.
    ///
    /// Re-reads from disk on every call, which is what makes
    /// [`crate::v1_1::bmm::rm_access::reference_model_access::ReferenceModelAccess::reload_schemas`]
    /// ("Reload all schemas") pick up an edited file. Any previously computed
    /// `schema` is dropped, because it was materialised from the old text.
    ///
    /// # Errors
    /// Returns [`RmAccessError::NoSchemaPath`] when the descriptor names no
    /// file, [`RmAccessError::File`] when it cannot be read, and
    /// [`RmAccessError::Schema`] when its content is not a P_BMM schema.
    pub fn load(&mut self) -> Result<(), RmAccessError> {
        let path = self
            .schema_path()
            .ok_or_else(|| RmAccessError::NoSchemaPath {
                schema_id: self.schema_id().to_owned(),
            })?
            .to_owned();
        let src = std::fs::read_to_string(&path).map_err(|source| RmAccessError::File {
            path: path.clone(),
            source,
        })?;
        let schema = read_schema(&src).map_err(|source| RmAccessError::Schema {
            path: path.clone(),
            source,
        })?;
        let meta_data = meta_data_of(&schema, &path);
        let includes = include_ids_of(&schema);
        match self {
            SchemaDescriptor::PBmmSchemaDescriptor(leaf) => {
                leaf.meta_data = meta_data;
                leaf.includes = present(includes);
                leaf.schema = None;
                leaf.bmm_schema = None;
                leaf.p_schema = Some(schema);
            }
            SchemaDescriptor::SchemaDescriptor(leaf) => {
                leaf.meta_data = meta_data;
                leaf.includes = present(includes);
                leaf.schema = None;
                leaf.p_schema = Some(schema);
            }
        }
        Ok(())
    }

    /// `SCHEMA_DESCRIPTOR.validate`: "Validate entire schema" (class doc
    /// §Functions).
    ///
    /// NOTE: the class doc states no check list; the reader and the transform
    /// own the P_BMM well-formedness rules, so what this checks is the
    /// descriptor-level agreement only they cannot see — loaded, a BMM
    /// version this software processes
    /// ([`SchemaDescriptor::is_bmm_compatible`]), and a rendered id matching
    /// the metadata's claim. No openEHR spec governs this check list — our
    /// own design/extension.
    ///
    /// # Errors
    /// Returns [`RmAccessError::NotLoaded`],
    /// [`RmAccessError::IncompatibleBmmVersion`] or
    /// [`RmAccessError::SchemaIdMismatch`].
    pub fn validate(&self) -> Result<(), RmAccessError> {
        let Some(schema) = self.p_schema() else {
            return Err(RmAccessError::NotLoaded {
                schema_id: self.schema_id().to_owned(),
            });
        };
        if !self.is_bmm_compatible() {
            return Err(RmAccessError::IncompatibleBmmVersion {
                schema_id: self.schema_id().to_owned(),
                found: self
                    .meta_data()
                    .get(META_BMM_VERSION)
                    .cloned()
                    .unwrap_or_default(),
                expected: expected_generation().to_owned(),
            });
        }
        let loaded = schema.schema_id();
        if loaded != self.schema_id() {
            return Err(RmAccessError::SchemaIdMismatch {
                path: self.schema_path().unwrap_or("").to_owned(),
                descriptor: self.schema_id().to_owned(),
                loaded,
            });
        }
        Ok(())
    }

    /// `SCHEMA_DESCRIPTOR.validate_includes`: "Validate includes list for this
    /// schema, to see if each mentioned schema exists in read schemas" (class
    /// doc §Functions).
    ///
    /// Matched case-insensitively, as
    /// [`crate::v1_1::bmm_persistence::include_resolution::resolve_includes`] does.
    ///
    /// # Errors
    /// Returns [`RmAccessError::MissingInclude`] naming the first include that
    /// is in no entry of `all_schemas_list`.
    pub fn validate_includes(&self, all_schemas_list: &[String]) -> Result<(), RmAccessError> {
        let available: BTreeSet<String> = all_schemas_list
            .iter()
            .map(|id| id.to_lowercase())
            .collect();
        for id in self.includes() {
            if !available.contains(&id.to_lowercase()) {
                return Err(RmAccessError::MissingInclude {
                    requester: self.schema_id().to_owned(),
                    id: id.clone(),
                });
            }
        }
        Ok(())
    }

    /// `SCHEMA_DESCRIPTOR.create_schema`: "Create `schema`" (class doc
    /// §Functions) — resolves this schema's inclusions against `all_p_schemas`
    /// and materialises the `BMM_MODEL` into `schema`.
    ///
    /// This is the tail of the pipeline `master02-overview.adoc` §Conceptual
    /// Approach prescribes, and it carries `P_BMM_SCHEMA.create_bmm_model`'s
    /// precondition (`state = P_BMM_PACKAGE_STATE.State_includes_processed`,
    /// `…p_bmm_schema.adoc` §Functions) as its `all_p_schemas` argument: every
    /// schema this one includes must be present and loaded.
    ///
    /// # Errors
    /// Returns [`RmAccessError::NotLoaded`] when [`SchemaDescriptor::load`] has
    /// not run, and [`RmAccessError::Schema`] when inclusion resolution or the
    /// transform refuses the schema.
    pub fn create_schema(
        &mut self,
        all_p_schemas: &BTreeMap<String, PBmmSchema>,
    ) -> Result<(), RmAccessError> {
        let Some(persisted) = self.p_schema().cloned() else {
            return Err(RmAccessError::NotLoaded {
                schema_id: self.schema_id().to_owned(),
            });
        };
        let path = self.schema_path().unwrap_or("").to_owned();
        let resolved =
            resolve_includes(persisted, all_p_schemas).map_err(|source| RmAccessError::Schema {
                path: path.clone(),
                source,
            })?;
        let model =
            create_bmm_model(&resolved).map_err(|source| RmAccessError::Schema { path, source })?;
        descriptor!(self, |leaf| leaf.schema = Some(model));
        Ok(())
    }

    /// The logical model name a materialised schema is published under:
    /// "model_publisher '_' model_name, e.g. \"openehr_rm\""
    /// (`org.openehr.lang.bmm.reference_model_access.adoc` §Attributes, on
    /// `valid_models`).
    ///
    /// Lower-cased, like
    /// [`crate::v1_1::bmm_persistence::p_bmm_schema::PBmmSchema::schema_id`]. Both
    /// parts are mandatory schema attributes (`P_BMM_SCHEMA.rm_publisher` and
    /// `schema_name` are `1..1`, `…p_bmm_schema.adoc` §Attributes, and the reader
    /// refuses a schema missing either), so a descriptor read from a file always
    /// renders both; a hand-built descriptor whose `meta_data` omits one renders
    /// that part empty rather than failing.
    #[must_use]
    pub fn logical_model_name(&self) -> String {
        let publisher = self
            .meta_data()
            .get(META_MODEL_PUBLISHER)
            .map(String::as_str)
            .unwrap_or_default();
        let name = self
            .meta_data()
            .get(META_SCHEMA_NAME)
            .map(String::as_str)
            .unwrap_or_default();
        format!("{publisher}_{name}").to_lowercase()
    }
}

/// The BMM generation `is_bmm_compatible` compares against: the
/// `BMM_DEFINITIONS.Bmm_internal_version` constant where the pinned generation
/// states one, else [`P_BMM_GENERATION`].
fn expected_generation() -> &'static str {
    if BmmDefinitionsData::BMM_INTERNAL_VERSION.is_empty() {
        P_BMM_GENERATION
    } else {
        major_of(BmmDefinitionsData::BMM_INTERNAL_VERSION)
    }
}

/// The major component of a dotted version (`"2.4"` → `"2"`).
fn major_of(version: &str) -> &str {
    version.split('.').next().unwrap_or(version)
}

/// The `meta_data` table of `schema`, with `path` recorded under
/// [`META_SCHEMA_PATH`].
///
/// The key set is exactly the one `SCHEMA_DESCRIPTOR.meta_data` enumerates
/// (class doc §Attributes); an empty header item is omitted rather than stored
/// as an empty string, so a lookup answers "not stated".
fn meta_data_of(schema: &PBmmSchema, path: &str) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for (key, value) in [
        (META_BMM_VERSION, schema.bmm_version.as_str()),
        (META_MODEL_PUBLISHER, schema.rm_publisher.as_str()),
        (META_SCHEMA_NAME, schema.schema_name.as_str()),
        (META_MODEL_RELEASE, schema.rm_release.as_str()),
        (META_SCHEMA_REVISION, schema.schema_revision.as_str()),
        (
            META_SCHEMA_LIFECYCLE_STATE,
            schema.schema_lifecycle_state.as_str(),
        ),
        (META_SCHEMA_DESCRIPTION, schema.schema_description.as_str()),
        (META_SCHEMA_PATH, path),
    ] {
        if !value.is_empty() {
            out.insert(key.to_owned(), value.to_owned());
        }
    }
    out
}

/// The `schema_id`s of the schemas `schema` includes
/// (`BMM_INCLUDE_SPEC.id`, "Full identifier of the included schema",
/// `org.openehr.lang.bmm.bmm_include_spec.adoc` §Attributes), in the schema's
/// own key order.
fn include_ids_of(schema: &PBmmSchema) -> Vec<String> {
    schema
        .includes
        .iter()
        .flat_map(BTreeMap::values)
        .map(|spec| spec.id.clone())
        .collect()
}
