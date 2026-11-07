// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The SCIM 2.0 service provider (RFC 7643, RFC 7644).
//!
//! [`protocol`] is the standards layer: the resource schemas, the messages,
//! the filter grammar, PATCH, entity tags and the discovery documents. It
//! holds no route, no store and no demographic mapping. No openEHR
//! specification governs SCIM; the provider follows RFC 7644 and is our own
//! design where the RFCs leave a choice.

#[expect(
    clippy::disallowed_types,
    reason = "SCIM resource content (#1694 boundary class): RFC 7643 §2 makes the schema \
              attribute-based and §3.3 lets configuration add extension attributes, so no \
              compile-time carrier holds a resource; the protocol layer works on the RFC 7644 \
              §3.8 JSON representation by definition"
)]
pub mod protocol;

pub mod endpoint;
