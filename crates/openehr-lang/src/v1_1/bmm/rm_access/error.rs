// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The typed failures of the `rm_access` schema-loading facade.
//!
//! The `rm_access` package "provides an interface for the application to load
//! and access BMM schemas" (`LANG/docs/bmm/master04-rm_access.adoc` §Overview),
//! so it is the one layer of this crate that touches the filesystem: its failure
//! set is the P_BMM pipeline's failures
//! ([`crate::v1_1::bmm_persistence::error::PBmmReadError`], wrapped) plus the
//! repository-level ones the pipeline cannot see (an unreadable directory or
//! file, a duplicate schema id, a load-list entry naming no schema, a lifecycle
//! step run out of order).
//!
//! Every variant is a discriminant a caller can branch on; the display text is
//! never a decision input.

/// A BMM schema repository could not be scanned, a schema file could not be
/// read, or a schema descriptor's lifecycle step failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RmAccessError {
    /// A schema directory could not be scanned.
    ///
    /// NOTE: no openEHR spec governs the filesystem errors —
    /// `REFERENCE_MODEL_ACCESS.schema_directories` is only "List of directories
    /// where all the schemas loaded here are found"
    /// (`org.openehr.lang.bmm.reference_model_access.adoc` §Attributes) — so the
    /// I/O failure set is our own design.
    #[error("schema directory `{directory}` could not be read: {source}")]
    Directory {
        /// The directory as listed in `schema_directories`.
        directory: String,
        /// The underlying I/O failure.
        #[source]
        source: std::io::Error,
    },

    /// A `.bmm` schema file could not be read.
    #[error("schema file `{path}` could not be read: {source}")]
    File {
        /// The file's path.
        path: String,
        /// The underlying I/O failure.
        #[source]
        source: std::io::Error,
    },

    /// The P_BMM pipeline refused a schema file's content.
    #[error("schema file `{path}`: {source}")]
    Schema {
        /// The file's path.
        path: String,
        /// The refusal from
        /// [`crate::v1_1::bmm_persistence::reader::read_schema`],
        /// [`crate::v1_1::bmm_persistence::include_resolution::resolve_includes`] or
        /// [`crate::v1_1::bmm_persistence::create_model::create_bmm_model`].
        #[source]
        source: crate::v1_1::bmm_persistence::error::PBmmReadError,
    },

    /// Two files in the schema directories render the same
    /// `SCHEMA_DESCRIPTOR.schema_id`, so an include naming it would be
    /// ambiguous (`org.openehr.lang.bmm.schema_descriptor.adoc` §Attributes:
    /// the id is "formed from meta_data model_publisher '_' schema_name '_'
    /// model_release").
    #[error("schema id `{id}` is rendered by both `{first}` and `{second}`")]
    DuplicateSchemaId {
        /// The shared id.
        id: String,
        /// The first file rendering it, in scan order.
        first: String,
        /// The second file rendering it.
        second: String,
    },

    /// A `initialise_with_load_list` entry names a schema no file in the schema
    /// directories declares.
    #[error("the load list names schema `{id}`, which no file in the schema directories declares")]
    UnknownLoadListEntry {
        /// The unmatched `schema_id`.
        id: String,
    },

    /// A schema includes another that is not in the candidate set —
    /// `SCHEMA_DESCRIPTOR.validate_includes` exists to "see if each mentioned
    /// schema exists in read schemas" (class doc §Functions).
    #[error("schema `{requester}` includes `{id}`, which is not among the read schemas")]
    MissingInclude {
        /// `schema_id` of the including schema.
        requester: String,
        /// The missing schema's id.
        id: String,
    },

    /// A lifecycle step needs the schema in memory, but
    /// `SCHEMA_DESCRIPTOR.load` has not run (or did not complete).
    #[error("schema `{schema_id}` is not loaded")]
    NotLoaded {
        /// The descriptor's `schema_id`.
        schema_id: String,
    },

    /// The loaded schema's own `schema_id` disagrees with the descriptor's — the
    /// file changed between the metadata read and the load.
    #[error("schema file `{path}` describes `{descriptor}` but loaded as `{loaded}`")]
    SchemaIdMismatch {
        /// The file's path.
        path: String,
        /// The id the descriptor's `meta_data` states.
        descriptor: String,
        /// The id the loaded `P_BMM_SCHEMA` renders.
        loaded: String,
    },

    /// The schema's declared BMM version is not one this software processes
    /// (`SCHEMA_DESCRIPTOR.is_bmm_compatible`, class doc §Functions).
    #[error(
        "schema `{schema_id}` declares bmm_version `{found}`, which is not compatible with the P_BMM generation this software reads (`{expected}`)"
    )]
    IncompatibleBmmVersion {
        /// The descriptor's `schema_id`.
        schema_id: String,
        /// The `bmm_version` the schema declares.
        found: String,
        /// The generation this software reads.
        expected: String,
    },

    /// A descriptor carries no `schema_path`, so there is no file to (re)read.
    #[error("schema `{schema_id}` has no `schema_path` meta-data entry")]
    NoSchemaPath {
        /// The descriptor's `schema_id`.
        schema_id: String,
    },
}
