// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Strongly-typed object identities.
//!
//! An EHR id and a versioned-object id are both UUIDs on the wire and in the
//! store, and confusing them is a clinical fault: a swapped pair reads or writes
//! another record's data and still type-checks. These newtypes make that mixup
//! uncompilable (the API-guidelines C-NEWTYPE discipline). The wire shapes are
//! unchanged: both serialize as the bare UUID (`#[serde(transparent)]`) and both
//! bind to `PostgreSQL` `uuid` columns (`#[sqlx(transparent)]`).
//!
//! Spec identities: the EHR id is RM ehr §EHR `ehr_id`, a `HIER_OBJECT_ID`; the
//! versioned-object id is RM common master06 §Version Identification, the
//! `object_id` part of an `OBJECT_VERSION_ID` and the `VERSIONED_OBJECT.uid`.
//!
//! NOTE: the inner type is `Uuid` rather than the full `HIER_OBJECT_ID` lexical
//! space because every ITS-REST 1.1.0 door that names an EHR types the parameter
//! `format: uuid` (`specifications/parameters/path/ehr_id.yaml`,
//! `query-codegen.openapi.yaml` §`components.parameters.ehr_id_Query`), and both
//! server-minted ids are `uuidv7`.
//!
//! The narrowing is real only for a client-supplied id on `PUT /ehr/{ehr_id}`,
//! where the release contradicts itself: the operation prose says the id "MUST
//! be valid `HIER_OBJECT_ID` value. It is strongly RECOMMENDED that an UUID
//! always be used for this" (`operations/ehr_create_with_id.yaml`) while the
//! parameter schema it references admits only `format: uuid`. This file follows
//! the parameter schema, the computable artifact binding every
//! `ehr_id`-carrying operation, so a non-UUID id is refused at the door rather
//! than admitted into a store and a query surface that cannot represent it.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::licence::stamp::StampKey;

/// The identity of one EHR (RM ehr §EHR `ehr_id`).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, sqlx::Type,
)]
#[serde(transparent)]
#[sqlx(transparent)]
pub struct EhrId(pub Uuid);

/// The identity of one versioned object — COMPOSITION / `EHR_STATUS` /
/// FOLDER / demographic party (RM common master06 §Version Identification:
/// the `object_id` of an `OBJECT_VERSION_ID`).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, sqlx::Type,
)]
#[serde(transparent)]
#[sqlx(transparent)]
pub struct VoId(pub Uuid);

/// The id of a versioned object a CONTRIBUTION creates, allocated before that
/// CONTRIBUTION commits, so another body of the same set can reference it.
///
/// It is minted fresh and is neither `Clone` nor `Copy`, so it names no
/// stored object and is handed to exactly one create.
#[derive(Debug, PartialEq, Eq)]
pub struct AllocatedVoId(VoId);

impl AllocatedVoId {
    /// A fresh id, stamped with the server's licence key.
    pub(crate) fn minted(stamp: &StampKey) -> Self {
        Self(VoId::minted(stamp))
    }

    /// The id the create will give the object.
    #[must_use]
    pub fn id(&self) -> VoId {
        self.0
    }
}

impl EhrId {
    /// A fresh time-ordered id for a new EHR, stamped with the server's
    /// licence key ([`crate::licence::stamp`]).
    #[must_use]
    pub fn minted(stamp: &StampKey) -> Self {
        Self(stamp.mint())
    }
}

impl VoId {
    /// A fresh time-ordered id for a new versioned object, stamped with the
    /// server's licence key ([`crate::licence::stamp`]).
    #[must_use]
    pub fn minted(stamp: &StampKey) -> Self {
        Self(stamp.mint())
    }

    /// The versioned object `uid` names: the `object_id` before the first `::`
    /// of an `OBJECT_VERSION_ID` (`object_id '::' creating_system_id '::'
    /// version_tree_id`, BASE `object_version_id.adoc`), or the whole of a bare
    /// uid.
    ///
    /// # Errors
    /// [`uuid::Error`] when that `object_id` is not a UUID, so it names no
    /// versioned object this server holds.
    pub fn from_uid_head(uid: &str) -> Result<Self, uuid::Error> {
        uid.split("::").next().unwrap_or(uid).parse()
    }
}

impl std::fmt::Display for EhrId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::fmt::Display for VoId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for EhrId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::from_str(s)?))
    }
}

impl std::str::FromStr for VoId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::from_str(s)?))
    }
}

impl From<EhrId> for Uuid {
    fn from(id: EhrId) -> Self {
        id.0
    }
}

impl From<VoId> for Uuid {
    fn from(id: VoId) -> Self {
        id.0
    }
}
