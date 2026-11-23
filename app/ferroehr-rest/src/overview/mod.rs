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
//! - `service_error` — the single service-error → HTTP mapping table
//!   (`ServiceError` meets the wire here and only here).
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
pub mod problem;
pub(crate) mod service_error;
pub mod status;
pub mod version_id;
