// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! **ITS-REST** — the openEHR REST API contract (ITS-REST 1.1.0).
//!
//! The transport DTOs, the per-operation param structs and response headers
//! structs, a server half and a client half per API group, and a route table
//! are **generated** by `openehr-codegen`'s `emit-rest` target into
//! [`generated`], spec-first from the vendored OpenAPI
//! (`vendor/rest-oas/*-codegen.openapi.yaml`) — both halves from one read, so
//! they cannot drift. RM payload types resolve to `openehr-rm`/`openehr-base`
//! rather than being re-emitted. The hand-written parts are [`runtime`]
//! (`ApiError`, and the `Refusal` the server traits return), `routes` (the
//! operation matcher over every route table), `server` (the whole-API router,
//! its `404`/`405` fallbacks, and what the generated routers share) and
//! `client` (the engine seam, credentials, retries, deadlines and errors the
//! generated clients build on). A server implements the generated traits and
//! mounts the generated routers; a consumer that calls a CDR takes the
//! generated clients; an intermediary matches a request with `routes` and
//! forwards it through `client`. Regenerate with
//! `cargo run -p openehr-codegen -- emit-rest`.

// The contract (DTOs, params, routes, `ApiError`, the operation matcher) rides
// `rest`; the server traits, the routers and their runtime ride `rest-server`;
// the generated clients and the client runtime ride `rest-client` — see the
// crate docs.
#[cfg(feature = "rest-client")]
pub mod client;
#[cfg(feature = "rest")]
pub mod decode;
#[cfg(feature = "rest")]
pub mod generated;
#[cfg(feature = "rest")]
pub mod routes;
#[cfg(feature = "rest")]
pub mod runtime;
#[cfg(feature = "rest-server")]
pub mod server;
