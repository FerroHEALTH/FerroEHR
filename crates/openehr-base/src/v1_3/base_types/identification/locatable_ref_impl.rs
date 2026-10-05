// @generated-from-template templates/openehr-base/base_types/identification/locatable_ref_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written accessor function for `LOCATABLE_REF`.
//!
//! Spec: BASE 1.3.0
//! `docs/specs/openehr/BASE/docs/UML/classes/org.openehr.base.base_types.locatable_ref.adoc`.
//! `as_uri()`: the URI form of the reference — scheme derived from `namespace`
//! (e.g. `ehr:`), then `id.value`, then `/` + `path` where `path` is non-empty.

use super::locatable_ref::LocatableRef;

impl LocatableRef {
    /// A URI form of this reference (BASE `LOCATABLE_REF.as_uri`): the scheme
    /// derived from `namespace` (e.g. `ehr:`), concatenated with `id.value`,
    /// and `/` + `path` when a non-empty `path` is present.
    #[must_use]
    pub fn as_uri(&self) -> String {
        let mut uri = format!("{}:{}", self.namespace, self.id.value());
        if let Some(path) = self.path.as_deref()
            && !path.is_empty()
        {
            uri.push('/');
            uri.push_str(path.strip_prefix('/').unwrap_or(path));
        }
        uri
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_3::base_types::identification::hier_object_id::HierObjectId;
    use crate::v1_3::base_types::identification::uid_based_id::UidBasedId;

    fn lref(path: Option<&str>) -> LocatableRef {
        LocatableRef {
            namespace: "ehr".to_owned(),
            r#type: "COMPOSITION".to_owned(),
            id: UidBasedId::HierObjectId(HierObjectId {
                value: "87284370-2D4B-4e3d-A3F3-F303D2F4F34B".to_owned(),
            }),
            path: path.map(str::to_owned),
        }
    }

    #[test]
    fn without_path() {
        assert_eq!(
            lref(None).as_uri(),
            "ehr:87284370-2D4B-4e3d-A3F3-F303D2F4F34B"
        );
        assert_eq!(
            lref(Some("")).as_uri(),
            "ehr:87284370-2D4B-4e3d-A3F3-F303D2F4F34B"
        );
    }

    #[test]
    fn with_path() {
        assert_eq!(
            lref(Some("/content[at0001]")).as_uri(),
            "ehr:87284370-2D4B-4e3d-A3F3-F303D2F4F34B/content[at0001]"
        );
    }
}
