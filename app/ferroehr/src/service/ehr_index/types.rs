// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHR Index information structures (`resource_status.adoc`,
//! `resource_instance_type.adoc`, `location_desc.adoc`).

/// A subject identifier reference (`i_ehr_index.adoc` `a_subject_id:
/// OBJECT_REF`).
///
/// The realization of the `OBJECT_REF` the EHR Index keys its associations
/// by: `id` = `OBJECT_ID.value`, `namespace`, `type`.
///
/// NOTE: the SM types the subject as a full `OBJECT_REF`; we carry the
/// three fields the index actually keys on (`(id, namespace)` is the
/// association key, `type` defaults to `PERSON` — the common MPI case).
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
    /// recorded: the common MPI case the SM's own wording assumes ("patient or
    /// other subject or care identifiers", `master07 §Overview`).
    pub const DEFAULT_TYPE: &'static str = "PERSON";

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
/// "Enumeration of resource instance types" (`resource_instance_type.adoc`),
/// surfacing the N:M duplicate-management states master07 §Overview describes.
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

    /// Parse a stored token, defaulting unknown values to `Primary`.
    #[must_use]
    pub fn from_str_or_primary(s: &str) -> Self {
        match s {
            "Duplicate" => ResourceInstanceType::Duplicate,
            "Supplementary" => ResourceInstanceType::Supplementary,
            _ => ResourceInstanceType::Primary,
        }
    }
}

/// `RESOURCE_STATUS` — "Object describing the status of a reference to a
/// resource" (`resource_status.adoc`).
///
/// NOTE: `start_valid_time`/`end_valid_time` are typed `@@` (an
/// unresolved placeholder) in the SM — a recorded spec defect; implemented as
/// ISO date-time strings (stored `timestamptz`).
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

/// `LOCATION_DESC` — "A descriptor containing location information for the
/// EHR with which this descriptor is associated" (`location_desc.adoc`).
///
/// NOTE: the SM class is an **empty stub** (no attributes defined) — a
/// recorded spec defect; the designed contract `{system_id, uri?,
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
/// — the SM defines no reads; NOTE on those methods).
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
