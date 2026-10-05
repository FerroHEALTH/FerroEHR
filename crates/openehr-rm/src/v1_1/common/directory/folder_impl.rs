// @generated-from-template templates/openehr-rm/common/directory/folder_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM class invariants for `FOLDER`.
//!
//! `FOLDER` declares NO invariants of its own in RM 1.2.0: its class table
//! (`RM/docs/UML/classes/org.openehr.rm.common.folder.adoc`) carries
//! Description / Inherit / Attributes rows and no Invariants section, and the
//! vendored BMM class definition likewise has no `invariants` member. So the
//! `FOLDER` inherits three from `LOCATABLE` (`…common.locatable.adoc`
//! §Invariants): `Archetype_node_id_valid`, which this impl runs; `Links_valid`,
//! structural (`links` is `Option<NonEmptyVec<Link>>`); and `Archetyped_valid`,
//! whose enforceable arm runs per node in the `openehr-its` instance pass.

use crate::v1_1::common::directory::folder::Folder;
use openehr_base::validate::{InvariantViolation, Validate};

impl Validate for Folder {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_1::validate::generated::archetype_node_id_core(
            "FOLDER",
            &self.archetype_node_id,
            out,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_1::data_types::text::dv_text::{DvText, DvTextData};

    fn folder(node_id: &str) -> Folder {
        Folder {
            name: DvText::DvText(DvTextData {
                value: "folder".to_owned(),
                hyperlink: None,
                formatting: None,
                mappings: openehr_base::containers::present_nonempty(Vec::new()),
                language: None,
                encoding: None,
            }),
            archetype_node_id: node_id.to_owned(),
            uid: None,
            links: openehr_base::containers::present_nonempty(Vec::new()),
            archetype_details: None,
            feeder_audit: None,
            items: openehr_base::containers::present(Vec::new()),
            folders: openehr_base::containers::present(Vec::new()),
            details: None,
        }
    }

    #[test]
    fn valid_folder() {
        assert!(folder("at0001").invariants().is_empty());
    }

    #[test]
    fn empty_node_id_invalid() {
        assert_eq!(
            folder("").invariants()[0].message,
            "Invariant Archetype_node_id_valid failed on type FOLDER"
        );
    }
}
