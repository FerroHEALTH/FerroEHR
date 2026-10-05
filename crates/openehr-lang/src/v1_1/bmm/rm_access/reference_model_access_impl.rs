// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written spec functions of `REFERENCE_MODEL_ACCESS` — the entry point of
//! the `rm_access` schema-loading facade.
//!
//! Spec: `LANG/docs/bmm/master04-rm_access.adoc` §Overview — the `rm_access`
//! package "provides an interface for the application to load and access BMM
//! schemas" — and
//! `LANG/docs/UML/classes/org.openehr.lang.bmm.reference_model_access.adoc`
//! §Attributes (`schema_directories`, `all_schemas`, `valid_models`) +
//! §Functions (`initialise_with_load_list`, `initialise_all`,
//! `reload_schemas`).
//!
//! Initialisation runs the descriptor lifecycle
//! ([`crate::v1_1::bmm::rm_access::schema_descriptor`]) over every `.bmm` file found in
//! `schema_directories`: describe → select → `load` → `validate` +
//! `validate_includes` → `create_schema` for each top-level schema, whose
//! `BMM_MODEL` lands in `valid_models`.
//!
//! This is the layer that touches the filesystem, which is why the spec puts it
//! here: `schema_directories` is part of the class.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::v1_1::bmm::core::bmm_definitions::BmmDefinitionsData;
use crate::v1_1::bmm::core::bmm_model::BmmModel;
use crate::v1_1::bmm::rm_access::error::RmAccessError;
use crate::v1_1::bmm::rm_access::reference_model_access::ReferenceModelAccess;
use crate::v1_1::bmm::rm_access::schema_descriptor::SchemaDescriptor;
use crate::v1_1::bmm_persistence::p_bmm_schema::PBmmSchema;
use openehr_base::containers::present;

impl ReferenceModelAccess {
    /// A schema repository over `schema_directories`, with nothing loaded yet.
    ///
    /// `REFERENCE_MODEL_ACCESS.schema_directories` is "List of directories where
    /// all the schemas loaded here are found" (class doc §Attributes);
    /// [`ReferenceModelAccess::initialise_all`] or
    /// [`ReferenceModelAccess::initialise_with_load_list`] then populates
    /// `all_schemas` and `valid_models`.
    #[must_use]
    pub fn new(schema_directories: Vec<String>) -> Self {
        Self {
            schema_directories: present(schema_directories),
            all_schemas: None,
            valid_models: None,
        }
    }

    /// `REFERENCE_MODEL_ACCESS.all_schemas`: "All schemas found and loaded from
    /// `schema_directories`. Keyed by schema_id" (class doc §Attributes).
    #[must_use]
    pub fn all_schemas(&self) -> Option<&BTreeMap<String, SchemaDescriptor>> {
        self.all_schemas.as_ref()
    }

    /// `REFERENCE_MODEL_ACCESS.valid_models`: "Top-level (root) models in use.
    /// Keyed by logical schema_name, i.e. model_publisher '_' model_name, e.g.
    /// \"openehr_rm\"" (class doc §Attributes) — see
    /// [`SchemaDescriptor::logical_model_name`].
    #[must_use]
    pub fn valid_models(&self) -> Option<&BTreeMap<String, BmmModel>> {
        self.valid_models.as_ref()
    }

    /// `REFERENCE_MODEL_ACCESS.initialise_all`: "Initialise with all schemas
    /// found in the schema directories" (class doc §Functions).
    ///
    /// # Errors
    /// Returns any [`RmAccessError`] the scan or the descriptor lifecycle raises
    /// (see [`ReferenceModelAccess::initialise_with_load_list`]).
    pub fn initialise_all(&mut self) -> Result<(), RmAccessError> {
        let directories = self.schema_directories.clone().unwrap_or_default();
        self.initialise(&directories, None)
    }

    /// `REFERENCE_MODEL_ACCESS.initialise_with_load_list`: "Initialise with a
    /// specific schema load list, usually a sub-set of schemas that will be
    /// found in the directories `a_schema_dirs`" (class doc §Functions).
    ///
    /// `a_schema_dirs` replaces `schema_directories` (the class doc passes the
    /// directories to this function, not to a constructor). The load list names
    /// `schema_id`s; each listed schema is loaded together with everything it
    /// includes, transitively — a schema cannot be materialised without them
    /// (`P_BMM_SCHEMA.merge`'s precondition, `…p_bmm_schema.adoc` §Functions), so
    /// pulling the closure in is what makes a sub-set load list usable at all.
    /// Ids are matched case-insensitively, as inclusion resolution does.
    ///
    /// # Errors
    /// Returns [`RmAccessError::Directory`] or [`RmAccessError::File`] on I/O
    /// failure, [`RmAccessError::Schema`] when a file is not a P_BMM schema,
    /// [`RmAccessError::DuplicateSchemaId`] when two files render one id,
    /// [`RmAccessError::UnknownLoadListEntry`] when a listed or included id
    /// matches no file, and whatever the descriptor lifecycle raises
    /// ([`SchemaDescriptor::validate`],
    /// [`SchemaDescriptor::validate_includes`],
    /// [`SchemaDescriptor::create_schema`]).
    pub fn initialise_with_load_list(
        &mut self,
        a_schema_dirs: Vec<String>,
        a_schema_load_list: &[String],
    ) -> Result<(), RmAccessError> {
        self.schema_directories = present(a_schema_dirs);
        let directories = self.schema_directories.clone().unwrap_or_default();
        self.initialise(&directories, Some(a_schema_load_list))
    }

    /// `REFERENCE_MODEL_ACCESS.reload_schemas`: "Reload all schemas" (class doc
    /// §Functions) — re-scans the schema directories and re-reads every schema
    /// currently in `all_schemas` from disk, so an edited file takes effect.
    ///
    /// NOTE (adjudicated): the reload keeps the CURRENT selection, which is the
    /// key set of `all_schemas` ("All schemas found and loaded from
    /// `schema_directories`", class doc §Attributes) — the class declares no
    /// attribute holding the original load list, so there is nothing else to
    /// replay. A file that appeared since initialisation is therefore picked up
    /// by a fresh `initialise_*` call, not by a reload; a file that has
    /// DISAPPEARED fails with [`RmAccessError::UnknownLoadListEntry`]. No
    /// openEHR spec governs this — our own design/extension.
    ///
    /// # Errors
    /// As [`ReferenceModelAccess::initialise_with_load_list`].
    pub fn reload_schemas(&mut self) -> Result<(), RmAccessError> {
        let selection: Vec<String> = self
            .all_schemas
            .iter()
            .flat_map(BTreeMap::keys)
            .cloned()
            .collect();
        let directories = self.schema_directories.clone().unwrap_or_default();
        if selection.is_empty() {
            return self.initialise(&directories, None);
        }
        self.initialise(&directories, Some(&selection))
    }

    /// Runs the whole lifecycle over `directories`, loading either everything
    /// found (`selection` is `None`) or the transitive closure of `selection`.
    fn initialise(
        &mut self,
        directories: &[String],
        selection: Option<&[String]>,
    ) -> Result<(), RmAccessError> {
        let mut found = scan(directories)?;
        let selected = match selection {
            None => found.keys().cloned().collect(),
            Some(list) => closure(&found, list)?,
        };

        let mut all: BTreeMap<String, SchemaDescriptor> = BTreeMap::new();
        for id in &selected {
            let mut descriptor = found
                .remove(id)
                .ok_or_else(|| RmAccessError::UnknownLoadListEntry { id: id.clone() })?;
            descriptor.load()?;
            all.insert(id.clone(), descriptor);
        }

        let ids: Vec<String> = all.keys().cloned().collect();
        for descriptor in all.values() {
            descriptor.validate()?;
            descriptor.validate_includes(&ids)?;
        }

        // The persisted forms every inclusion resolves against, and the
        // top-level schemas — computed before the mutable materialisation pass.
        let persisted: BTreeMap<String, PBmmSchema> = all
            .values()
            .filter_map(|descriptor| {
                descriptor
                    .p_schema()
                    .map(|schema| (descriptor.schema_id().to_owned(), schema.clone()))
            })
            .collect();
        let top_level: Vec<String> = all
            .iter()
            .filter(|(_, descriptor)| descriptor.is_top_level(&all))
            .map(|(id, _)| id.clone())
            .collect();

        let mut valid_models: BTreeMap<String, BmmModel> = BTreeMap::new();
        for id in &top_level {
            let Some(descriptor) = all.get_mut(id) else {
                continue;
            };
            descriptor.create_schema(&persisted)?;
            if let Some(model) = descriptor.schema() {
                valid_models.insert(descriptor.logical_model_name(), model.clone());
            }
        }

        self.all_schemas = (!all.is_empty()).then_some(all);
        self.valid_models = (!valid_models.is_empty()).then_some(valid_models);
        Ok(())
    }
}

/// Describes every schema file in `directories`, keyed by its `schema_id`.
///
/// A file is a schema file when its name ends with
/// `BMM_DEFINITIONS.Bmm_schema_file_extension` ("Extension used for BMM files"
/// = `".bmm"`, `org.openehr.lang.bmm.bmm_definitions.adoc` §Constants).
///
/// NOTE (adjudicated): each listed directory is scanned NON-recursively —
/// `schema_directories` is "List of directories where all the schemas loaded
/// here are found" (`…reference_model_access.adoc` §Attributes), i.e. the
/// directories holding the schemas, so a nested layout is expressed by listing
/// each directory. No openEHR spec governs the scan depth — our own
/// design/extension. Entries are processed in sorted path order so a duplicate
/// id is always reported against the same pair of files.
fn scan(directories: &[String]) -> Result<BTreeMap<String, SchemaDescriptor>, RmAccessError> {
    let mut files: Vec<PathBuf> = Vec::new();
    for directory in directories {
        let entries = std::fs::read_dir(directory).map_err(|source| RmAccessError::Directory {
            directory: directory.clone(),
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| RmAccessError::Directory {
                directory: directory.clone(),
                source,
            })?;
            let path = entry.path();
            let is_schema_file = path
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|name| name.ends_with(BmmDefinitionsData::BMM_SCHEMA_FILE_EXTENSION));
            if is_schema_file && path.is_file() {
                files.push(path);
            }
        }
    }
    files.sort();

    let mut out: BTreeMap<String, SchemaDescriptor> = BTreeMap::new();
    for path in files {
        let descriptor = SchemaDescriptor::from_schema_file(&path)?;
        let id = descriptor.schema_id().to_owned();
        if let Some(first) = out.get(&id) {
            return Err(RmAccessError::DuplicateSchemaId {
                id,
                first: first.schema_path().unwrap_or("").to_owned(),
                second: path.display().to_string(),
            });
        }
        out.insert(id, descriptor);
    }
    Ok(out)
}

/// The load-list selection: every listed `schema_id` plus, transitively, every
/// schema those include. Matched case-insensitively against `found`.
fn closure(
    found: &BTreeMap<String, SchemaDescriptor>,
    load_list: &[String],
) -> Result<BTreeSet<String>, RmAccessError> {
    // schema_id lower-cased → the key `found` holds it under.
    let by_lower: BTreeMap<String, String> = found
        .keys()
        .map(|id| (id.to_lowercase(), id.clone()))
        .collect();
    let mut selected: BTreeSet<String> = BTreeSet::new();
    let mut pending: Vec<String> = load_list.to_vec();
    while let Some(wanted) = pending.pop() {
        let key = by_lower
            .get(&wanted.to_lowercase())
            .ok_or_else(|| RmAccessError::UnknownLoadListEntry { id: wanted.clone() })?;
        if !selected.insert(key.clone()) {
            continue;
        }
        let Some(descriptor) = found.get(key) else {
            continue;
        };
        pending.extend(descriptor.includes().iter().cloned());
    }
    Ok(selected)
}
