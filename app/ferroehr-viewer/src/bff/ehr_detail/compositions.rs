// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHR-detail Compositions tab: the AQL-driven composition list, its
//! URL-borne row filters, and the "Commit composition" form.

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
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr));
/// [`ViewerError::Invalid`] when a date bound is neither a date nor an instant;
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
    let cdr = crate::bff::cdr()?;
    let filter = CompositionFilter::new(&template, &from, &to, &composer);
    let query = composition_query(&ehr_id, &filter)?;
    let url = cdr.rest_v1("query/aql");
    let body = aql_request_body(&query.aql, &query.parameters, offset, fetch);
    let response = cdr.post(&url).body("application/json", body).ok().await?;
    parse_result_set(&response.body, offset)
}

/// The EHR's compositions with NO filter applied.
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
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr));
/// [`ViewerError::Invalid`] on an empty body or a FLAT commit without a
/// template id; a non-2xx CDR answer (its validation diagnostics, which the UI
/// renders verbatim, included) normalizes via
/// [`CdrResponse::ok`](crate::cdr::CdrResponse::ok).
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
    let cdr = crate::bff::cdr()?;
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
    let url = cdr.rest_v1(&format!("ehr/{}/composition", urlencoding::encode(&ehr_id)));
    let response = cdr
        .post(&url)
        .body(format.media_type(), body)
        .headers(&headers)
        .ok()
        .await?;
    Ok(uid_value_of(&response.body))
}
