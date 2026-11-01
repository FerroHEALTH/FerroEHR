// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHR index's subject reference.

/// A subject identifier reference, an `OBJECT_REF`.
///
/// `id` = `OBJECT_ID.value`, `namespace`, `type`: `(id, namespace)` names the
/// subject, `type` defaults to `PERSON` (the common MPI case).
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
    /// A subject reference of the default `PERSON` type: the common MPI
    /// case, a patient (our own default).
    #[must_use]
    pub fn person(id: impl Into<String>, namespace: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            namespace: namespace.into(),
            r#type: "PERSON".to_owned(),
        }
    }
}
