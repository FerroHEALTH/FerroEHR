// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHR-detail Compositions tab: the AQL-driven composition list, its
//! URL-borne row filters, and the "Commit composition" form.
//!
//! The filters (template, a context-start-time window, composer) live in the
//! query string — `?template=`/`?from=`/`?to=`/`?composer=`, submitted by a GET
//! `<Form>` — so a filtered view is shareable, refresh-safe and reproducible
//! before the WASM bundle loads. They are read in SETUP and feed the list
//! resource's source, so a filter change refetches exactly once and a
//! submission drops `?offset=`, putting the reader on the first page of the new
//! result set. The statement they produce is assembled by the pure,
//! component-free [`composition_filter`](super::composition_filter): operator
//! input is bound as AQL `query_parameters`, never concatenated into the query.

use crate::uid::uid_value_of;

use crate::bff::ehr_detail::composition_filter::CompositionFilter;
use crate::bff::ehr_detail::composition_filter::composition_query;
use crate::bff::ehrs::ResultPage;
use crate::bff::ehrs::{aql_request_body, parse_result_set};
use crate::error::ViewerError;
use crate::format::ReprFormat;
use crate::locale::Msg;
use crate::locale::records::RecordsMsg;
use crate::locale::tr_served;

/// List an EHR's compositions newest-first, one page at `offset`, narrowed by
/// whichever filters are filled.
///
/// Every argument is operator input and every one of them travels as an AQL
/// `query_parameters` binding — see
/// [`composition_query`], which
/// builds the statement and the bindings together.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] when a date bound is neither a date nor an
/// instant; CDR transport errors pass through; a non-2xx CDR answer normalizes
/// via [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success);
/// [`ViewerError::Internal`] when the result set is not valid JSON.
pub async fn list_compositions(
    // The EHR whose compositions to list.
    ehr_id: String,
    // Substring of the template id to keep; empty means every template.
    template: String,
    // Inclusive lower bound on the context start time; empty means unbounded.
    from: String,
    // Inclusive upper bound on the context start time; empty means unbounded.
    to: String,
    // Substring of the composer's name to keep; empty means every composer.
    composer: String,
    // First row of the page to return.
    offset: u32,
    // How many rows the page holds.
    fetch: u32,
) -> Result<ResultPage, ViewerError> {
    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
    let filter = CompositionFilter::new(&template, &from, &to, &composer);
    let query = composition_query(&ehr_id, &filter)?;
    let url = state.cdr.rest_v1("query/aql");
    let body = aql_request_body(&query.aql, &query.parameters, offset, fetch);
    let response = state
        .cdr
        .post(
            &session.credential,
            &url,
            "application/json",
            "application/json",
            &[],
            body,
        )
        .await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    parse_result_set(&response.body, offset)
}

/// The EHR's compositions with NO filter applied.
///
/// The viewer's composition PICKERS (the Commit tab's amend selector, the
/// directory editor's item selector) offer every composition in the EHR, not
/// the view the tab happens to be filtered to — this names that intent at the
/// call site instead of four empty strings.
///
/// `ehr_id` names the EHR and `offset` the page's first row.
///
/// # Errors
/// Whatever [`list_compositions`] returns.
pub async fn all_compositions(ehr_id: String, offset: u32) -> Result<ResultPage, ViewerError> {
    list_compositions(
        ehr_id,
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        offset,
        crate::bff::PAGE_SIZE,
    )
    .await
}

/// Commit a new COMPOSITION to the EHR (`POST /ehr/{ehr_id}/composition`).
///
/// The `format` picks the `Content-Type` (canonical JSON `application/json`,
/// canonical XML `application/xml`, FLAT `application/openehr.wt.flat+json`); a
/// FLAT commit additionally requires the `openehr-template-id` header.
///
/// `Accept: application/json` + `Prefer: return=representation` yields a
/// canonical composition body whose `uid.value` is returned as the new
/// version uid.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] on an empty body or a FLAT commit without a
/// template id; CDR transport errors pass through; a non-2xx CDR answer (its
/// validation diagnostics, which the UI renders verbatim, included)
/// normalizes via
/// [`CdrClient::expect_success`](crate::cdr::CdrClient::expect_success).
pub async fn commit_composition(
    // The EHR to commit into.
    ehr_id: String,
    // Which representation the body is written in.
    format: ReprFormat,
    // The template the composition is built from (required by the simplified formats).
    template_id: String,
    // The composition document to commit, as text.
    body: String,
) -> Result<String, ViewerError> {
    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
    if body.trim().is_empty() {
        return Err(ViewerError::Invalid(tr_served(Msg::CompositionBodyEmpty)));
    }
    let template_id = template_id.trim();
    let mut headers: Vec<(&str, &str)> = vec![("Prefer", "return=representation")];
    if matches!(format, ReprFormat::Flat) {
        if template_id.is_empty() {
            return Err(ViewerError::Invalid(tr_served(
                RecordsMsg::TemplateIdRequiredCommitFlatComposition,
            )));
        }
        headers.push(("openehr-template-id", template_id));
    }
    let url = state
        .cdr
        .rest_v1(&format!("ehr/{}/composition", urlencoding::encode(&ehr_id)));
    let response = state
        .cdr
        .post(
            &session.credential,
            &url,
            format.media_type(),
            "application/json",
            &headers,
            body,
        )
        .await?;
    let response = crate::cdr::CdrClient::expect_success(response)?;
    Ok(uid_value_of(&response.body))
}
