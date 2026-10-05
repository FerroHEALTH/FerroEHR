// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! HTTP dispatch for the `definition` API group (templates + stored queries).
//!
//! This module is the operation-id `match` only: it decodes nothing and renders
//! nothing itself, delegating each arm to the resource module that owns the spec
//! resource ([`template_adl14`] /
//! [`template_adl2`] /
//! [`stored_query`]). Every arm rebuilds the operation's
//! `*Params`, decodes any body, calls the trait method on [`AppState`], and
//! renders a negotiated response inside its module.
//!
//! Note: the generated `ROUTES` operation ids carry dots (e.g.
//! `definition_template_adl1.4_list`, `definition_query_store.yaml`); the match
//! keys below are those exact strings.

use axum::response::{IntoResponse, Response};

use ferroehr::service::definition::types::TemplateListFilter;
use ferroehr::service::list::Page;

use crate::api::{BoxResponse, RequestParts};
use crate::overview::error::RestError;
use crate::state::AppState;

use super::{stored_query, template_adl2, template_adl14};

pub(crate) fn dispatch(state: AppState, op: &'static str, parts: RequestParts) -> BoxResponse {
    Box::pin(async move {
        run(state, op, parts)
            .await
            .unwrap_or_else(IntoResponse::into_response)
    })
}

async fn run(
    state: AppState,
    op: &'static str,
    parts: RequestParts,
) -> Result<Response, RestError> {
    match op {
        "definition_template_adl1.4_list" => template_adl14::list(&state, &parts).await,
        "definition_template_adl1.4_upload" => template_adl14::upload(&state, &parts).await,
        "definition_template_adl1.4_get" => template_adl14::get(&state, &parts).await,
        "definition_template_adl1.4_example_get" => {
            template_adl14::example_get(&state, &parts).await
        }
        "definition_template_adl2_list" => template_adl2::list(&state, &parts).await,
        "definition_template_adl2_upload" => template_adl2::upload(&state, &parts).await,
        "definition_template_adl2_get" => template_adl2::get(&state, &parts).await,
        "definition_template_adl2_example_get" => template_adl2::example_get(&state, &parts).await,
        "definition_template_adl2_version_get" => template_adl2::version_get(&state, &parts).await,
        "definition_query_list" => stored_query::list(&state, &parts).await,
        "definition_query_list_all" => stored_query::list_all(&state, &parts).await,
        "definition_query_store.yaml" => stored_query::store(&state, &parts).await,
        "definition_query_version_get" => stored_query::version_get(&state, &parts).await,
        "definition_query_version_store.yaml" => stored_query::version_store(&state, &parts).await,
        other => Err(RestError(openehr_its::rest::runtime::ApiError::Internal(
            format!("unrouted definition operation: {other}"),
        ))),
    }
}

/// Fold the wire `template_id`/`concept`/`version`/`offset`/`fetch` query
/// parameters (decoded on both `definition_template_*_list` operations) into the
/// [`TemplateListFilter`] + SM [`Page`] the `DefinitionAdapter` list methods
/// carry. `offset` defaults to `0` (`parameters/query/offset.yaml`); a `fetch`
/// of `0`/absent is normalized to "all" by [`Page::limit`]. Negative wire values
/// (out of the spec's `integer`/`0`-based range) degrade to the defaults rather
/// than erroring.
pub(super) fn list_filter_and_page(
    template_id: Option<String>,
    concept: Option<String>,
    version: Option<String>,
    offset: Option<i64>,
    fetch: Option<i64>,
) -> (TemplateListFilter, Page) {
    let filter = TemplateListFilter {
        template_id,
        concept,
        version,
    };
    let page = Page {
        item_offset: offset.and_then(|o| u64::try_from(o).ok()),
        items_to_fetch: fetch.and_then(|f| u64::try_from(f).ok()),
    };
    (filter, page)
}
