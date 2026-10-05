// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `UPDATE_ITEM_TAG` builders shared by the tag-facing suites.
//!
//! The tag write seams take the GENERATED ITS-REST DTO
//! (`schemas/common/UpdateItemTag.yaml`: required `key`, optional
//! `value`/`target_path`, `additionalProperties: false`) rather than untyped
//! JSON, so an undeclared member or a wrongly-typed one cannot reach them at
//! all. These builders keep the test bodies readable without re-introducing a
//! second, untyped tag shape.

/// One `UPDATE_ITEM_TAG` for the EHR API group's tag seams.
pub(crate) fn ehr_tag(
    key: &str,
    value: Option<&str>,
    target_path: Option<&str>,
) -> openehr_its::rest::generated::common::UpdateItemTag {
    openehr_its::rest::generated::common::UpdateItemTag {
        key: key.to_owned(),
        value: value.map(str::to_owned),
        target_path: target_path.map(str::to_owned),
    }
}

/// One `UPDATE_ITEM_TAG` for the demographic API group's tag seams.
pub(crate) fn party_tag(
    key: &str,
    value: Option<&str>,
    target_path: Option<&str>,
) -> openehr_its::rest::generated::common::UpdateItemTag {
    openehr_its::rest::generated::common::UpdateItemTag {
        key: key.to_owned(),
        value: value.map(str::to_owned),
        target_path: target_path.map(str::to_owned),
    }
}
