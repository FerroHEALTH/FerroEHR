// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHR Index information structures: resource status, resource instance
//! type and location descriptor.

/// A subject identifier reference, an `OBJECT_REF`.
///
/// The realization of the `OBJECT_REF` the EHR Index keys its associations
/// by: `id` = `OBJECT_ID.value`, `namespace`, `type`.
///
/// The three fields the index keys on: `(id, namespace)` is the association
/// key, `type` defaults to `PERSON` (the common MPI case).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectRef {
    /// The subject identifier value (`OBJECT_REF.id.value`).
    pub id: String,
    /// The identifier namespace (`OBJECT_REF.namespace`).
    pub namespace: String,
    /// The referenced object type (`OBJECT_REF.type`); `PERSON` by default.
    pub r#type: String,
}

impl SubjectRef {
    /// The `OBJECT_REF.type` a subject reference carries when none was
    /// recorded: the common MPI case, a patient (our own default).
    pub const DEFAULT_TYPE: &str = "PERSON";

    /// A subject reference of the default `PERSON` type.
    #[must_use]
    pub fn person(id: impl Into<String>, namespace: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            namespace: namespace.into(),
            r#type: Self::DEFAULT_TYPE.to_owned(),
        }
    }
}

/// `RESOURCE_INSTANCE_TYPE` — the kind of a subject↔EHR association.
///
/// The N:M duplicate-management states of a subject↔EHR association.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ResourceInstanceType {
    /// "Primary instance of the resource."
    #[default]
    Primary,
    /// "A duplicate instance of the resource" — the N:M error state.
    Duplicate,
    /// `Supplementary` (meaning blank in the source).
    Supplementary,
}

impl ResourceInstanceType {
    /// The stored token (`Primary`/`Duplicate`/`Supplementary`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ResourceInstanceType::Primary => "Primary",
            ResourceInstanceType::Duplicate => "Duplicate",
            ResourceInstanceType::Supplementary => "Supplementary",
        }
    }
}

/// `RESOURCE_STATUS` — the status of a reference to a resource.
///
/// `start_valid_time`/`end_valid_time` are ISO date-time strings (stored
/// `timestamptz`).
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct ResourceStatus {
    /// `instance_type [1]` — "Type of resource instance."
    pub instance_type: ResourceInstanceType,
    /// `start_valid_time [0..1]` — "First time point at which resource can be
    /// assumed to be available."
    pub start_valid_time: Option<String>,
    /// `end_valid_time [0..1]` — "Last time point at which resource can be
    /// assumed to be available."
    pub end_valid_time: Option<String>,
    /// `notes [0..1]` — "Human-readable notes on the resource."
    pub notes: Option<String>,
}

/// `LOCATION_DESC` — location information for the EHR it is associated with.
///
/// NOTE: no specification governs this: our own design; `{system_id, uri?,
/// description?}` makes the optional location descriptor carry usable data.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LocationDesc {
    /// Identifier of the system hosting the EHR.
    pub system_id: String,
    /// A resolvable location for the EHR, if any.
    pub uri: Option<String>,
    /// Human-readable description of the location.
    pub description: Option<String>,
}

/// One EHR Index record: a subject↔EHR association with its status and
/// optional location descriptor.
///
/// Returned by the design-filled read calls (`ehr_subjects` / `subject_ehrs`
/// — our own design).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EhrIndexEntry {
    /// The associated EHR id.
    pub ehr_id: String,
    /// The subject identifier.
    pub subject: SubjectRef,
    /// The association status (instance type + validity + notes).
    pub status: ResourceStatus,
    /// The optional location descriptor.
    pub location: Option<LocationDesc>,
}
