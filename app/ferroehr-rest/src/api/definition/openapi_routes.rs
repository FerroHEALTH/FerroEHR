// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Native `utoipa-axum` routing for the Definition API group (ADL 1.4 / ADL 2
//! templates + stored queries).
//!
//! Operation semantics are the ITS-REST Definition API
//! (`docs/specs/openehr/ITS-REST`); no openEHR spec governs the OAS layout. Each
//! handler calls its operation's handler through [`guarded_dispatch`](crate::api::guarded_dispatch).
//!
//! NOTE (operation ids): a few released operation ids carry `.` (e.g.
//! `definition_template_adl1.4_list`, `definition_query_store.yaml`) — invalid
//! Rust identifiers, so the handler fn names sanitise `.` to `_`, while the
//! served `operationId` (`operation_id = …`) and the op string passed to the
//! dispatcher are the verbatim released id.
//!
//! ## Prose-vs-OAS reconciliations (documented real wire, per handler)
//!
//! - **ADL 2 upload** (`definition_template_adl2_upload.yaml`): the OAS declares
//!   `201/400/409`; the REST adapter emits the `409` on a duplicate HRID and the `400`
//!   for an ADL2/AOM2 artefact that is invalid on the store path. A request
//!   DECLARING a media type other than the operation's single `text/plain` body type is refused
//!   `415` before parsing — the same cross-cutting rule that grounds the ADL 1.4
//!   guard (`specifications/docs/overview/Resources.md` §"XML Format"/§"JSON
//!   Format": a payload the service cannot process as the declared format "MUST
//!   respond with HTTP status code `415 Unsupported Media Type`"); an absent
//!   `Content-Type` declares nothing to refuse (the header is a client MAY). The
//!   `at_version` query parameter is `deprecated: true` and is dropped
//!   (spec-permitted); only `Prefer` is honoured.
//! - **ADL 2 get / version get** (`definition_template_adl2_get.yaml`,
//!   `_version_get.yaml` — the latter `deprecated: true`): the OAS lists a `400`
//!   that is unreachable here, since `template_id` and `version` are path
//!   segments. The build serves the `text/plain` ADL2 source and the
//!   `application/json` `OperationalTemplateV2` projection, and declares no
//!   `application/xml` body, so an `Accept` naming only XML is a `406`.
//! - **ADL 2 example get** (`_example_get`): the stored template is compiled to
//!   its operational template and turned into a Web Template, which the shared
//!   example generator walks into an example COMPOSITION, served across the four
//!   `Accept_LOCATABLE` forms exactly as the ADL 1.4 example endpoint.
//! - **ADL 1.4 upload** (`definition_template_adl1.4_upload.yaml`): the OAS
//!   declares `201/400/409`; an OPT that parses as XML but is structurally
//!   invalid is the `400` too.
//! - **ADL 1.4 get / example** (`_get`, `_example_get`): the OAS `400` on `_get`
//!   is unreachable (only a path parameter); the reachable statuses are the
//!   ones documented on each operation below.
//! - **Stored-query list by name** (`definition_query_list.yaml`): the qualified
//!   name is a prefix pattern, so an unmatched name yields an empty `200` list,
//!   never `404` — the released operation declares only `200`
//!   (`specifications/responses/200_QueryList.yaml`). Its "when is empty, it
//!   will be treated as 'wildcard' in the search" clause is unreachable on the
//!   released wire: `specifications/parameters/path/qualified_query_name.yaml`
//!   is `required: true` and the release defines no bare `/definition/query`
//!   operation — this build serves that empty-prefix case on an extension route
//!   of its own (flagged as such on the declaration).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 8): genuinely open operational JSON (config \
              dump, management env, validity-checker input, OpenAPI schema literals)"
)]

mod adl14;
mod adl2;
mod query;

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::state::AppState;

/// The Definition API group as a native `utoipa-axum` router (group-relative
/// paths; nested under the configured base path).
// `definition_template_adl2_version_get` carries `#[deprecated]` — the released
// operation is `deprecated: true` under SPECITS-87
// (`ITS-REST specifications/operations/definition_template_adl2_version_get.yaml`
// + `docs/overview/Amendment_record.md`), and utoipa reflects the Rust attribute
// into the served OpenAPI. It is the only handler in this group so marked; the
// `routes!` macro references it by name, so the deprecation lint is allowed here.
#[expect(
    deprecated,
    reason = "`definition_template_adl2_version_get` is `#[deprecated]` so utoipa \
              reflects `deprecated: true` into the served document; the \
              `routes!` macro has to reference it by name"
)]
pub(crate) fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(
            adl14::definition_template_adl1_4_list,
            adl14::definition_template_adl1_4_upload
        ))
        .routes(routes!(adl14::definition_template_adl1_4_get))
        .routes(routes!(adl14::definition_template_adl1_4_example_get))
        .routes(routes!(
            adl2::definition_template_adl2_list,
            adl2::definition_template_adl2_upload
        ))
        .routes(routes!(adl2::definition_template_adl2_get))
        .routes(routes!(adl2::definition_template_adl2_example_get))
        .routes(routes!(adl2::definition_template_adl2_version_get))
        .routes(routes!(query::definition_query_list_all))
        .routes(routes!(
            query::definition_query_list,
            query::definition_query_store_yaml
        ))
        .routes(routes!(
            query::definition_query_version_get,
            query::definition_query_version_store_yaml
        ))
}
