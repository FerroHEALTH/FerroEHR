// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Errors for the Simplified Formats layer.
//!
//! Every MUST-level rejection in the Simplified Formats specification
//! (`ITS-REST/docs/simplified_formats/`) has its own variant so callers can
//! branch on the outcome without string matching; the wire layer maps these
//! to `422`/`400` per the ITS-REST operation contracts.

/// An error in Simplified-Format (FLAT / STRUCTURED / Web Template) handling.
#[derive(Debug, thiserror::Error)]
pub enum FlatError {
    /// The operational template lacked something the Web Template builder
    /// requires.
    #[error("invalid operational template: {0}")]
    InvalidTemplate(String),

    /// An OPT 1.4 XML document failed to parse.
    #[error("failed to parse OPT 1.4 XML: {0}")]
    OptParse(#[from] openehr_its::xml::runtime::XmlError),

    /// A value could not be serialized to JSON.
    #[error("failed to serialize to JSON: {0}")]
    Serialize(#[from] serde_json::Error),

    /// A FLAT key is not syntactically valid (ITS-REST simplified_formats
    /// master04 §Flat format syntax rules).
    #[error("malformed simplified path {path:?}: {reason}")]
    MalformedPath {
        /// The offending key as received.
        path: String,
        /// What rule the key breaks.
        reason: String,
    },

    /// A path does not resolve to a node of the target Web Template
    /// (master04 §Validation: "Field identifiers match WT metadata
    /// structure").
    #[error("unknown simplified path: {0}")]
    UnknownPath(String),

    /// An attribute suffix is not defined for the RM type at its path
    /// (master05 per-type tables).
    #[error("unknown attribute suffix |{suffix} for {rm_type} at {path}")]
    UnknownSuffix {
        /// The RM type of the node the path resolved to.
        rm_type: String,
        /// The offending suffix (without the pipe).
        suffix: String,
        /// The path the suffix was attached to.
        path: String,
    },

    /// `|other` combined with `|code`/`|value`/`|terminology` on one leaf —
    /// the combination servers MUST reject (master04 §Open Value-Sets and
    /// the `|other` Suffix).
    #[error("|other is mutually exclusive with |code/|value/|terminology at {0}")]
    OtherSuffixConflict(String),

    /// `|other` used where the value-set constraint is closed
    /// (`listOpen: false`) — MUST be rejected (master04 §Open Value-Sets
    /// and the `|other` Suffix).
    #[error("|other not allowed at {0}: the value set is closed")]
    OtherOnClosedValueSet(String),

    /// A mandatory context field is absent (master04 §Context: "Mandatory:
    /// language, territory").
    #[error("mandatory ctx field {0} is required (master04 §Context: language and territory)")]
    MissingContext(&'static str),

    /// A `|code` on a closed value-set leaf names a code outside the bound
    /// list and carries no explicit `|value` to supply the text
    /// (master04 §Validation: "Terminology bindings are valid").
    #[error("coded value is not a member of the bound value set: '{code}' at {path}")]
    CodeNotInValueSet {
        /// The simplified path of the coded leaf.
        path: String,
        /// The code the document supplied.
        code: String,
    },

    /// A `ctx/` key outside the vocabulary of master06-context_information.
    #[error("unknown context key ctx/{0}")]
    UnknownContext(String),

    /// A FLAT key the RM types 1..1 is absent from the submitted document:
    /// either a `|suffix` the master05 mapping table marks `Required: yes` —
    /// e.g. all three of §LINK's `|type`, `|meaning` and `|target`
    /// (`RM/docs/UML/classes/org.openehr.rm.common.link.adoc` §Attributes) —
    /// or a `ctx/` key standing in for a mandatory attribute of the RM object
    /// it builds, e.g. `ctx/participation_function:<i>` for
    /// `PARTICIPATION.function`
    /// (`RM/docs/UML/classes/org.openehr.rm.common.participation.adoc`
    /// §Attributes).
    #[error("{key} is required")]
    MissingRequiredSuffix {
        /// The full FLAT key the client omitted, e.g. `…/_link:0|meaning` or
        /// `ctx/participation_function:0`.
        key: String,
    },

    /// A value has the wrong JSON type or an unparsable encoding for its
    /// slot (e.g. a non-numeric `|magnitude`, a malformed compact
    /// participation-identifier list).
    #[error("invalid value at {path}: {reason}")]
    InvalidValue {
        /// The simplified path of the offending value.
        path: String,
        /// What is wrong with it.
        reason: String,
    },

    /// A `|raw` payload is not a usable canonical-JSON RM fragment
    /// (master04 §Raw canonical JSON: the value must carry `_type`).
    #[error("invalid |raw payload at {path}: {reason}")]
    InvalidRaw {
        /// The simplified path of the `|raw` key.
        path: String,
        /// What is wrong with the payload.
        reason: String,
    },

    /// The input or produced structure is not the shape the converter
    /// expects (e.g. the RM value at a template path is not the declared RM
    /// type, or a STRUCTURED document is not a JSON object).
    #[error("simplified-format conversion error: {0}")]
    Conversion(String),
}
