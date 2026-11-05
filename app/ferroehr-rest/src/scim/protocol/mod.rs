// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The SCIM 2.0 standards layer, vendor-neutral, over the vendored RFCs.
//!
//! | Module | Governs | Source |
//! |---|---|---|
//! | [`urn`] | the schema and message URNs | RFC 7643 §10.4, RFC 7644 §8.2 |
//! | [`schema`] | attribute characteristics and the User, Group and Enterprise User schemas | RFC 7643 §2, §4, §7 |
//! | [`resource`] | a resource body against its schema: create, replace, representation | RFC 7643 §2 and §3, RFC 7644 §3.3, §3.5.1 |
//! | [`path`] | attribute paths and PATCH paths | RFC 7644 §3.10, §3.5.2 Figure 7 |
//! | [`filter`] | the filter grammar, its evaluation and a provider's supported subset | RFC 7644 §3.4.2.2 |
//! | [`patch`] | the `PatchOp` message and its operations | RFC 7644 §3.5.2 |
//! | [`list`] | `ListResponse`, `SearchRequest`, pagination and attribute projection | RFC 7644 §3.4.2, §3.4.3, §3.9 |
//! | [`error`] | the error response and its `scimType` | RFC 7644 §3.12 |
//! | [`etag`] | entity tags and the `If-Match` and `If-None-Match` preconditions | RFC 7644 §3.14, RFC 9110 §8.8.3, §13.1 |
//! | [`discovery`] | `ServiceProviderConfig`, `ResourceTypes` and `Schemas`, built from configuration | RFC 7644 §4, RFC 7643 §5 to §7 |
//! | [`deviation`] | the client deviations the provider tolerates, each with its vendored source | Microsoft Entra ID provisioning documents |
//!
//! The layer follows RFC 7644. Where a client is documented to send something
//! the RFCs do not allow, the tolerance is a [`deviation::Deviation`] variant
//! that names the client's vendored document, and every parser that applies
//! one reports it, so no code path names a client.

pub mod deviation;
pub mod discovery;
pub mod error;
pub mod etag;
pub mod filter;
pub mod list;
mod message;
pub mod patch;
pub mod path;
pub mod resource;
pub mod schema;
pub mod urn;

/// The media type of every SCIM request and response body (RFC 7644 §8.1).
pub const MEDIA_TYPE: &str = "application/scim+json";
