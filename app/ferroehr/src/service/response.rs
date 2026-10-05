// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Protocol-adapter response envelope — the typed resource metadata from
//! which the ITS-REST adapter derives `ETag`/`Location`/`Last-Modified`.
//!
//! No openEHR spec governs this — our own design: the SM returns plain
//! values; ITS-REST 1.1.0 mandates the headers (`headers/ETag_*.yaml`,
//! `headers/Location_*.yaml`; `Last-Modified` per the ITS-REST overview,
//! `VERSION.commit_audit.time_committed.value`). This envelope is the seam
//! that carries what the headers need.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): ServiceResponse::body — the hybrid \
              RM/stored/dynamic carrier (commit-seam class)"
)]

use jiff::Timestamp;
use openehr_rm::prelude::ItemTag;
use serde_json::Value;

/// Typed metadata about a versioned resource a write/read produced, from
/// which the protocol adapter derives the spec-mandated `ETag`/`Location`
/// headers.
#[derive(Debug, Clone)]
pub struct ResourceMeta {
    /// The owning EHR id (the `Location` path root; the `ETag` value for an
    /// EHR). Empty for EHR-less resources (demographic parties).
    pub ehr_id: String,
    /// The resource's identifier that is both the `ETag` value (quoted) and
    /// the `Location` path tail: an `OBJECT_VERSION_ID` for a versioned
    /// resource, the `ehr_id` for an EHR, or the contribution uid for a
    /// CONTRIBUTION.
    pub uid: String,
    /// The commit time of this version (its audit `time_committed`). Emitted
    /// as the `Last-Modified` response header — SHOULD-present on
    /// `VERSION`/`VERSIONED_OBJECT` responses.
    pub last_modified: Option<Timestamp>,
    /// The distinct origins of the data this response serves (#3212): the
    /// version's `FEEDER_AUDIT` originating systems, or the server that
    /// created it. Empty when the operation serves no version body. The access
    /// log records it (EHDS Annex II 3.2(e)); no header renders it.
    pub origins: Vec<String>,
    /// The `ITEM_TAGs` (RM `common.item_tag`) currently associated with this
    /// resource, or `None` when the operation carries no tags. The ITS-REST
    /// adapter renders this into the `openehr-item-tag` /
    /// `openehr-version-item-tag` response headers
    /// (`headers/openehr-item-tag.yaml`,
    /// `headers/openehr-version-item-tag.yaml`).
    ///
    /// No openEHR spec governs this envelope field — our own design: the SM
    /// returns plain values, and this seam carries what the mandated headers
    /// need. The tags are the RM `common.item_tag.ITEM_TAG` type itself.
    pub item_tags: Option<Vec<ItemTag>>,
    /// The served/committed VERSION's own `ITEM_TAG` collection, when it is
    /// distinct from the container's (`openehr-version-item-tag` — overview
    /// §"openehr-item-tag and openehr-version-item-tag": the two headers
    /// address a `VERSIONED_OBJECT` and a specific VERSION within it).
    pub version_item_tags: Option<Vec<ItemTag>>,
}

impl ResourceMeta {
    /// Metadata for a versioned resource: the owning EHR plus the resource
    /// `uid` used as the `ETag` and `Location` tail.
    #[must_use]
    pub fn new(ehr_id: impl Into<String>, uid: impl Into<String>) -> Self {
        Self {
            ehr_id: ehr_id.into(),
            uid: uid.into(),
            last_modified: None,
            origins: Vec::new(),
            item_tags: None,
            version_item_tags: None,
        }
    }

    /// Attach the origins of the data this response serves.
    #[must_use]
    pub fn with_origins(mut self, origins: Vec<String>) -> Self {
        self.origins = origins;
        self
    }

    /// Attach the version's commit time.
    #[must_use]
    pub fn with_last_modified(mut self, at: Timestamp) -> Self {
        self.last_modified = Some(at);
        self
    }

    /// Attach the resource's `ITEM_TAG` list for the `openehr-item-tag` /
    /// `openehr-version-item-tag` response headers.
    #[must_use]
    pub fn with_item_tags(mut self, tags: Vec<ItemTag>) -> Self {
        self.item_tags = Some(tags);
        self
    }
}

/// A service response: the canonical-JSON RM payload plus optional resource
/// metadata.
///
/// A `null` `body` means "no representation" (a logically deleted read, or a
/// resource whose value is not returned); `meta` is present whenever the
/// operation produced/identified a versioned resource.
#[derive(Debug, Clone)]
pub struct ServiceResponse {
    /// The canonical-JSON RM payload, or `Value::Null` when there is none.
    pub body: Value,
    /// Resource metadata for header derivation, if any.
    pub meta: Option<ResourceMeta>,
}

/// A read body: the parsed canonical value, or the stored text verbatim.
///
/// The raw form carries the stored canonical text (already uid-stamped at
/// commit) — the JSON-accept passthrough the protocol adapter writes to the
/// wire without a parse → serialize round trip.
#[derive(Debug, Clone)]
pub enum ReadBody {
    /// The parsed canonical value.
    Value(Value),
    /// The stored body's own jsonb text rendering, served verbatim for a
    /// JSON accept; a consumer needing the typed value parses it.
    RawJson(String),
}

impl ReadBody {
    /// The typed value: the parsed representation as-is, or the raw text
    /// parsed on demand.
    ///
    /// # Errors
    /// The `serde_json` rejection of undecodable text (the stored body is our
    /// own jsonb rendering — a failure is corrupt data, never a client
    /// condition; callers map it to an internal fault).
    pub fn into_value(self) -> Result<Value, serde_json::Error> {
        match self {
            ReadBody::Value(v) => Ok(v),
            ReadBody::RawJson(text) => serde_json::from_str(&text),
        }
    }
}

/// A [`ServiceResponse`] whose body may still be the stored canonical text
/// (see [`ReadBody`]).
#[derive(Debug, Clone)]
pub struct RawServiceResponse {
    /// The read body, parsed or verbatim.
    pub body: ReadBody,
    /// Resource metadata for header derivation, if any.
    pub meta: Option<ResourceMeta>,
}

impl ServiceResponse {
    /// A response carrying both an RM body and resource metadata.
    #[must_use]
    pub fn new(body: Value, meta: ResourceMeta) -> Self {
        Self {
            body,
            meta: Some(meta),
        }
    }

    /// A plain response with a body but no resource metadata.
    #[must_use]
    pub fn plain(body: Value) -> Self {
        Self { body, meta: None }
    }

    /// A bodyless response carrying resource metadata — the delete outcome
    /// that still returns the (now deleted) `version_uid` in
    /// `ETag`/`Location` (`204_COMPOSITION_deleted.yaml`).
    #[must_use]
    pub fn deleted(meta: ResourceMeta) -> Self {
        Self {
            body: Value::Null,
            meta: Some(meta),
        }
    }

    /// Whether the body is absent (a logically deleted read → `204`, or a
    /// bodyless delete outcome).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.body.is_null()
    }
}
