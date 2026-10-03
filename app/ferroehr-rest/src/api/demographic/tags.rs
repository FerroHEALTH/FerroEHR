// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `ITEM_TAG` sub-resources — `operations/person_tags_get.yaml`,
//! `person_tags_update.yaml`, `person_tags_delete.yaml` (and the field-identical
//! `agent_*`/`group_*`/`organisation_*`/`role_*`), plus the kind-agnostic
//! collection filter `demographic_tags_get.yaml`. Tags use the **canonical**
//! content negotiation (`Accept_canonical`/`ContentType_canonical`).
//!
//! **Each kind decodes through its OWN generated params type.** The five
//! families' parameter sets are field-identical in Release-1.1.0, so one type
//! would decode all five today — but that is a property of the current release,
//! not a contract. Routing every kind through `Agent*Params` would mean a
//! future release that adds a parameter to (say) `person_tags_get` alone would
//! be silently mis-decoded on four other families, with no compile error. The
//! per-kind match below makes any such divergence a build failure instead.

use axum::response::Response;

use openehr_its::rest::generated::demographic::{
    AgentTagsDeleteParams, AgentTagsGetParams, AgentTagsUpdateParams, DemographicTagsGetParams,
    GroupTagsDeleteParams, GroupTagsGetParams, GroupTagsUpdateParams, OrganisationTagsDeleteParams,
    OrganisationTagsGetParams, OrganisationTagsUpdateParams, PersonTagsDeleteParams,
    PersonTagsGetParams, PersonTagsUpdateParams, RoleTagsDeleteParams, RoleTagsGetParams,
    RoleTagsUpdateParams,
};
use openehr_its::rest::runtime::ApiError;

use crate::api::RequestParts;
use crate::api::item_tags;
use crate::negotiate;
use crate::overview::error::RestError;
use crate::state::AppState;
use ferroehr::service::demographic::types::PartyKind;
use http::StatusCode;

/// The per-kind `ITEM_TAG` operations (`tags_get`/`tags_update`/`tags_delete`).
pub(super) async fn run(
    state: AppState,
    kind: PartyKind,
    action: &str,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    let seg = kind.segment();
    // Each kind's own generated params struct, decoded for its `uid_based_id`.
    macro_rules! uid_based_id {
        ($params:ident) => {
            parts
                .decode($params::from_request, $params::PARAMS)?
                .uid_based_id
        };
    }

    match action {
        "tags_get" => {
            // Each kind's own generated params type (see the module doc).
            let uid_based_id = match kind {
                PartyKind::Agent => uid_based_id!(AgentTagsGetParams),
                PartyKind::Group => uid_based_id!(GroupTagsGetParams),
                PartyKind::Organisation => uid_based_id!(OrganisationTagsGetParams),
                PartyKind::Person => uid_based_id!(PersonTagsGetParams),
                PartyKind::Role => uid_based_id!(RoleTagsGetParams),
            };
            let tags = state.backend().party_tags_get(kind, uid_based_id).await?;
            Ok(negotiate::respond(
                h,
                StatusCode::OK,
                &openehr_its::json::to_canonical_value(&tags),
            ))
        }
        "tags_update" => {
            let uid_based_id = match kind {
                PartyKind::Agent => uid_based_id!(AgentTagsUpdateParams),
                PartyKind::Group => uid_based_id!(GroupTagsUpdateParams),
                PartyKind::Organisation => uid_based_id!(OrganisationTagsUpdateParams),
                PartyKind::Person => uid_based_id!(PersonTagsUpdateParams),
                PartyKind::Role => uid_based_id!(RoleTagsUpdateParams),
            };
            let body = item_tags::write_body(h, &parts.body)?;
            let tags = state
                .backend()
                .party_tags_update(kind, uid_based_id, body)
                .await?;
            // person_tags_update.yaml — 200 (200_PERSON_ItemTagList_updated)
            // with the tag list on `Prefer: return=representation`; 204
            // (204_updated) when `Prefer` is missing or `return=minimal`,
            // with `Preference-Applied` declaring which.
            Ok(negotiate::write_collection(
                h,
                StatusCode::NO_CONTENT,
                StatusCode::OK,
                &openehr_its::json::to_canonical_value(&tags),
            ))
        }
        "tags_delete" => {
            let (uid_based_id, key) = match kind {
                PartyKind::Agent => {
                    let p = parts.decode(
                        AgentTagsDeleteParams::from_request,
                        AgentTagsDeleteParams::PARAMS,
                    )?;
                    (p.uid_based_id, p.key)
                }
                PartyKind::Group => {
                    let p = parts.decode(
                        GroupTagsDeleteParams::from_request,
                        GroupTagsDeleteParams::PARAMS,
                    )?;
                    (p.uid_based_id, p.key)
                }
                PartyKind::Organisation => {
                    let p = parts.decode(
                        OrganisationTagsDeleteParams::from_request,
                        OrganisationTagsDeleteParams::PARAMS,
                    )?;
                    (p.uid_based_id, p.key)
                }
                PartyKind::Person => {
                    let p = parts.decode(
                        PersonTagsDeleteParams::from_request,
                        PersonTagsDeleteParams::PARAMS,
                    )?;
                    (p.uid_based_id, p.key)
                }
                PartyKind::Role => {
                    let p = parts.decode(
                        RoleTagsDeleteParams::from_request,
                        RoleTagsDeleteParams::PARAMS,
                    )?;
                    (p.uid_based_id, p.key)
                }
            };
            state
                .backend()
                .party_tags_delete(kind, uid_based_id, key)
                .await?;
            Ok(negotiate::empty(StatusCode::NO_CONTENT))
        }
        other => Err(RestError(ApiError::Internal(format!(
            "unrouted demographic tag operation: {seg}_{other}"
        )))),
    }
}

/// `GET /demographic/tags` — the kind-agnostic `ITEM_TAG` collection filter
/// (`demographic_tags_get.yaml`).
pub(super) async fn run_collection(
    state: AppState,
    parts: RequestParts,
) -> Result<Response, RestError> {
    let h = &parts.headers;
    let p = parts.decode(
        DemographicTagsGetParams::from_request,
        DemographicTagsGetParams::PARAMS,
    )?;
    let tags = state
        .backend()
        .demographic_tags_get(p.tag_key, p.tag_value, p.tag_target_path)
        .await?;
    Ok(negotiate::respond(
        h,
        StatusCode::OK,
        &openehr_its::json::to_canonical_value(&tags),
    ))
}
