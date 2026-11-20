// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The answers several query operations declare word for word.

/// A projected version body the active `spec_profile` cannot express.
#[derive(utoipa::IntoResponses)]
#[response(
    status = 409,
    description = "OUR OWN EXTENSION — no openEHR spec \
                  governs runtime \
                  specification-generation selection. A \
                  whole-object projection (`SELECT c FROM \
                  EHR e CONTAINS COMPOSITION c`) serves a \
                  stored version BODY, so it takes the \
                  same generation gate the version reads \
                  take: under `spec_profile = \"stable\"` \
                  a projected version the released \
                  generations cannot express refuses the \
                  query — never elided from the rows, \
                  never down-converted — naming the \
                  version and the remedy. Leaf/scalar \
                  projections over the same rows serve \
                  data values rather than version bodies \
                  and are NOT gated. Unreachable under the \
                  default `development` profile."
)]
pub(super) struct SpecProfileConflict;

/// A query stopped by the configured execution budget.
#[derive(utoipa::IntoResponses)]
#[response(
    status = 408,
    description = "The released trigger, verbatim: `408 \
                  Request Timeout` \"is returned when there \
                  is a query execution timeout (i.e. \
                  maximum query execution time reached, \
                  therefore the server aborted the \
                  execution of the query)\" (ITS-REST \
                  `specifications/responses/408_Query.yaml`) \
                  — here the configured per-query execution \
                  budget (`FERROEHR__RECORD__QUERY__TIMEOUT`)."
)]
pub(super) struct QueryTimeout;

/// An `Accept` a `RESULT_SET` cannot satisfy.
#[derive(utoipa::IntoResponses)]
#[response(
    status = 406,
    description = "The `Accept` header cannot be satisfied: \
                  a RESULT_SET is served as canonical \
                  `application/json` only (the canonical \
                  XML ITS defines no RESULT_SET shape), so \
                  an exclusively-XML `Accept` is refused \
                  (`Resources.md` §\"JSON Format\": \"If \
                  the service cannot fulfill this aspect of \
                  the request, it MUST respond with HTTP \
                  status code `406 Not Acceptable`\")."
)]
pub(super) struct ResultSetNotAcceptable;
