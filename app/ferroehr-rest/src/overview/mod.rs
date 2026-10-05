// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ITS-REST **Overview** specification — the cross-cutting protocol every
//! openEHR REST API inherits.
//!
//! Oracle: `docs/specs/openehr/ITS-REST/specifications/docs/overview/`
//! (`Intro.md`, `Glossary_and_conventions.md`, `Resources.md`,
//! `Requests_and_responses.md`) at **Release-1.1.0** (`24058992d`,
//! the same commit as the vendored OAS — one identity for prose + machine
//! contract), plus the shared OAS pieces
//! (`…/specifications/{headers,parameters,responses}/`).
//!
//! ## The Release-1.1.0 protocol deltas vs Release-1.0.3
//!
//! (`Requests_and_responses.md` §Deprecated headers) — implemented new-form-
//! first with the deprecated forms still accepted/emitted where the spec says
//! MAY. Status below is *descriptive* of what this layer does today:
//!
//! - **`ETag` weakness indicator — DONE.** Every resource-identifier `ETag`
//!   is emitted as the weak `W/"{uid}"` form (`negotiate::resource_etag`,
//!   used by `negotiate::set_versioning_headers` and the template path).
//!   Inbound `If-Match` accepts both the weak and the deprecated bare quoted
//!   forms (`version_id::strip_etag`).
//! - **Committal headers — DONE.** The development-edition value forms
//!   `openehr-version: lifecycle_state.code_string="…"` and
//!   `openehr-audit-details: change_type.code_string="…"` / `description.value`
//!   / `committer.*` / `system_id` are parsed and merged
//!   (`committal::merge_committal_headers`), repeated headers included; the
//!   deprecated dotted-*name* forms still work, and the new form wins on
//!   conflict. A client-supplied `system_id` is carried into
//!   `UpdateAudit.system_id`; when absent "the server MUST set it to its own
//!   configured system identifier" — asserted at the versioning seam.
//! - **Location — DONE.** `Location` is set only on create/update writes
//!   (`negotiate::set_resource_headers`); reads, deletes, and the `409`/`412`
//!   error path emit versioning headers without `Location`
//!   (`negotiate::set_versioning_headers`).
//! - **`return=identifier` — DONE.** `negotiate::write_rm` /
//!   `negotiate::write_json` honour `return=identifier` with a
//!   `{ "uid": … }` body at a `200`/`201` status (never `204`) — exactly the
//!   overview §"Prefer only identifier" shape ("a single JSON object with a
//!   single `uid` attribute"). That generic `{uid}` body is the realization for
//!   every `uid`-versioned resource (EHR, COMPOSITION, `EHR_STATUS`, FOLDER,
//!   CONTRIBUTION): the `ehr`-group OAS defines no distinct per-resource
//!   identifier schema. The one divergence is the `definition` group — templates
//!   are not `uid`-versioned, so their identifier body is
//!   `{ "template_id": … }` (`schemas/others/TemplateIdentifier.yaml`), rendered
//!   in that group's handlers.
//! - **`Preference-Applied` — DONE.** Every write path declares the preference
//!   it actually applied through the one seam
//!   (`negotiate::set_preference_applied`, a MAY): the canonical/JSON writes
//!   via `negotiate::write_negotiated`, plus the demographic writes, the
//!   template uploads, the `ITEM_TAG` collection writes, and the
//!   Simplified-Formats commit. A request with no `Prefer` gets the applied
//!   default, `return=minimal`.
//! - **Item-tag headers — DONE (EHR + demographic groups).** The parse/emit
//!   helpers (`params::parse_item_tag_header` / `params::emit_item_tag_header`)
//!   are consumed through the one shared write-wrapper
//!   (`crate::api::item_tags`): `pending` reads the request wrapper headers,
//!   `persist` folds them onto the `ITEM_TAG` service on change-controlled
//!   writes (empty value ⇒ delete all), and `echo` emits each stored
//!   collection under ITS OWN header — `openehr-item-tag` confirms the
//!   `VERSIONED_OBJECT`'s tags, `openehr-version-item-tag` the VERSION's,
//!   never merged. The demographic group writes and echoes the same DISTINCT
//!   collections (pinned by `tests/it/demographic_tags_http.rs`
//!   `wrapper_headers_write_distinct_collections_on_update`).
//! - **Method status — DONE.** `error::method_not_allowed_handler`
//!   (`405`) is mounted as the API router's `method_not_allowed_fallback`
//!   (`crate::router::router`), so a known path called with a disallowed method renders
//!   the openEHR `{ error, message }` body, and axum decorates it with the
//!   matched route's `Allow` (RFC 9110 §15.5.6). A `405` raised from a
//!   **matched** handler — the config-gated admin group — carries its own
//!   `Allow` via `error::method_not_allowed_response`. The paired `501` for a
//!   recognised-but-unimplemented operation rides
//!   [`ApiError::NotImplemented`](openehr_its::rest::runtime::ApiError) at
//!   dispatch level; there is no `501` handler, and the overview's
//!   SHOULD-`501` for an *unrecognized method* is answered `405` instead — a
//!   settled deviation (rationale in [`crate::router::router`]).
//! - **Version identity / `openehr-uri`** are out of this change's
//!   scope; `/status` reports the tested
//!   development-edition contract identity (shared provenance,
//!   `crate::extensions::provenance::ITS_REST`) and `openehr-uri` is not emitted.
//!
//! All the cross-folder wiring the redesign deferred has since landed: the
//! item-tag dispatch (the shared `crate::api::item_tags` write-wrapper),
//! the `405` router-fallback mount + the `501` `ApiError` seam, the per-API
//! identifier bodies, and the `UpdateAudit.system_id` field (set by
//! `api::ehr::mk_update_version` and merged from the
//! committal request headers).
//!
//! ## Module map (file ↦ governing sections)
//!
//! - [`negotiate`] — data representation + content negotiation
//!   (`Resources.md` §Data representation: canonical XML/JSON MUSTs, the
//!   `415`/`406` discipline, the Simplified-Format media types
//!   `application/openehr.wt.{flat,structured}+json`;
//!   `Requests_and_responses.md` §Representation details negotiation:
//!   `Prefer: return=minimal|return=representation` — minimal the current
//!   default — and `Prefer: resolve_refs`).
//! - [`committal`] — the committal metadata request headers
//!   (`Requests_and_responses.md` §openehr-version and openehr-audit-details:
//!   services MUST accept them; "whatever is provided it MUST be merged with
//!   the default VERSION and `VERSION.audit_details` attributes on commit";
//!   direct `PUT`/`POST`/`DELETE` on change-controlled resources "MUST
//!   internally be executed using the 'native' way" — a CONTRIBUTION; when
//!   `system_id` is absent "the server MUST set it to its own configured
//!   system identifier").
//! - [`error`] — the HTTP status-code table (`Requests_and_responses.md`
//!   §HTTP status codes: 200/201/204/400/401/403/404/405/406/408/409/412/
//!   415/422/500/501; §HTTP Methods: an unrecognized method SHOULD be `501` but
//!   is answered `405` for the reason [`crate::router::router`] gives, and a
//!   known-but-not-allowed one SHOULD be `405`) + the optional error body
//!   ("if `Prefer: return=representation`")
//!   + the single SM → HTTP mapping table (`CALL_STATUS_TYPE` meets the
//!     wire here and only here).
//! - [`version_id`] — resource identification (`Resources.md` §Resource
//!   identification: `versioned_object_uid` `HIER_OBJECT_ID` vs `version_uid`
//!   `OBJECT_VERSION_ID` `object_id::creating_system_id::version_tree_id`,
//!   `uid_based_id` dual addressing) and the `If-Match` discipline
//!   (§If-Match: on a false condition "MUST NOT perform the requested
//!   method … MUST respond with `412 Precondition Failed`, and SHOULD
//!   return also latest `version_uid` in the `ETag` response headers";
//!   expected-but-missing → `400`).
//! - [`params`] — common parameters (`Glossary_and_conventions.md`:
//!   `version_at_time` extended ISO 8601; `Resources.md` §Datetime format:
//!   temporal query/path values "MUST always use the extended ISO 8601
//!   format").
//! - [`status`] — response headers (`Requests_and_responses.md` §Location:
//!   201-only; §`ETag` and Last-Modified: weak `W/` quoted resource
//!   identifier + `Last-Modified` from
//!   `VERSION.commit_audit.time_committed.value`, both SHOULD be present on
//!   `VERSION/VERSIONED_OBJECT` responses; §openehr-uri MAY).
//!
//! Auth is out of band (`Requests_and_responses.md` §Authentication and
//! authorization: no scheme mandated; `401`/`403`/`407` +
//! `WWW-Authenticate` discipline when a framework is present) — the authn
//! stack lives with the extension surface (`crate::access`), not here.

pub mod committal;
pub mod error;
pub mod negotiate;
pub mod params;
pub mod status;
pub mod version_id;
