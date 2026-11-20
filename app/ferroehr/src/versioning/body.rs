// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The content a commit carries: a typed COMPOSITION, or the canonical value
//! of a kind whose commit has not moved onto typed RM yet (#6003).
//!
//! The typed COMPOSITION is what the commit validates, offloads, stamps,
//! renders once as `version.body` and its node rows, and signs, with no
//! canonical value built; only a `553|incomplete|` commit validates over one.
//! No openEHR spec governs the commit's internal representation: our own
//! design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the canonical value of the kinds whose commit \
              is not typed yet, and the view the JSON-reading passes take (#6003)"
)]

use std::borrow::Cow;

use openehr_base::prelude::{ObjectVersionId, UidBasedId};
use openehr_its::json::JsonParseError;
use openehr_rm::prelude::{
    Agent, Composition, EhrAccess, EhrStatus, Folder, Group, Organisation, PartyProxy, Person, Role,
};
use openehr_rm::v1_2::common::archetyped::archetyped::Archetyped;
use openehr_rm::v1_2::demographic::party::Party;
use openehr_rm::v1_2::demographic::party_relationship::PartyRelationship;
use serde::{Serialize, Serializer};
use serde_json::Value;

use super::Kind;
use crate::error::ServiceError;
use crate::storage::render::index::Rendered;

/// The content of a create or a modify. It has no `Debug`: it is clinical
/// content (`reliability.md`).
#[derive(Clone)]
pub(crate) enum Body {
    /// A COMPOSITION.
    Composition(Box<Composition>),
    /// An `EHR_STATUS`.
    EhrStatus(Box<EhrStatus>),
    /// An `EHR_ACCESS`.
    EhrAccess(Box<EhrAccess>),
    /// A FOLDER tree.
    Folder(Box<Folder>),
    /// A PARTY of any of its five concrete kinds.
    Party(Box<Party>),
    /// A `PARTY_RELATIONSHIP`.
    PartyRelationship(Box<PartyRelationship>),
    /// The canonical value of a body with no typed form.
    Canonical(Value),
}

/// Runs `$typed` over the typed RM value of a body, bound to `$rm`, or
/// `$canonical` over its canonical value, bound to `$view`.
macro_rules! each_typed {
    ($body:expr, $rm:ident => $typed:expr, $view:ident => $canonical:expr) => {
        match $body {
            Body::Composition($rm) => $typed,
            Body::EhrStatus($rm) => $typed,
            Body::EhrAccess($rm) => $typed,
            Body::Folder($rm) => $typed,
            Body::Party($rm) => $typed,
            Body::PartyRelationship($rm) => $typed,
            Body::Canonical($view) => $canonical,
        }
    };
}

/// Runs `$e` over the concrete RM value of a PARTY, bound to `$p`.
macro_rules! each_party {
    ($party:expr, $p:ident => $e:expr) => {
        match $party {
            Party::Agent($p) => $e,
            Party::Group($p) => $e,
            Party::Organisation($p) => $e,
            Party::Person($p) => $e,
            Party::Role($p) => $e,
        }
    };
}

impl From<Composition> for Body {
    fn from(rm: Composition) -> Self {
        Body::Composition(Box::new(rm))
    }
}

impl From<EhrStatus> for Body {
    fn from(rm: EhrStatus) -> Self {
        Body::EhrStatus(Box::new(rm))
    }
}

impl From<EhrAccess> for Body {
    fn from(rm: EhrAccess) -> Self {
        Body::EhrAccess(Box::new(rm))
    }
}

impl From<Folder> for Body {
    fn from(rm: Folder) -> Self {
        Body::Folder(Box::new(rm))
    }
}

impl From<Party> for Body {
    fn from(rm: Party) -> Self {
        Body::Party(Box::new(rm))
    }
}

impl From<PartyRelationship> for Body {
    fn from(rm: PartyRelationship) -> Self {
        Body::PartyRelationship(Box::new(rm))
    }
}

/// The body serializes as its canonical JSON, written straight from the typed
/// value.
impl Serialize for Body {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        each_typed!(self, rm => rm.serialize(serializer), view => view.serialize(serializer))
    }
}

impl Body {
    /// A typed COMPOSITION body.
    pub(crate) fn composition(rm: Composition) -> Self {
        Body::Composition(Box::new(rm))
    }

    /// The body as a canonical value, for a response that serves it as one.
    pub(crate) fn into_canonical(self) -> Value {
        match self {
            Body::Canonical(view) => view,
            typed => openehr_its::json::to_canonical_value(&typed),
        }
    }

    /// The canonical value of a body of a kind that has no typed form.
    pub(crate) fn canonical(&self) -> Option<&Value> {
        match self {
            Body::Canonical(view) => Some(view),
            _ => None,
        }
    }

    /// The composer a COMPOSITION body states: the typed one, or a canonical
    /// one read strictly; `None` for a body of another kind, or a canonical
    /// one stating no composer.
    pub(crate) fn composer(&self) -> Option<Result<Cow<'_, PartyProxy>, JsonParseError>> {
        match self {
            Body::Composition(rm) => Some(Ok(Cow::Borrowed(&rm.composer))),
            Body::Canonical(view) => view
                .get("composer")
                .map(|composer| openehr_its::json::from_canonical_value(composer).map(Cow::Owned)),
            _ => None,
        }
    }

    /// Puts `composer` in the place of a COMPOSITION body's composer; a body
    /// of another kind is left as it is.
    pub(crate) fn set_composer(&mut self, composer: PartyProxy) {
        match self {
            Body::Composition(rm) => *rm.composer = composer,
            Body::Canonical(view) => {
                if let Some(map) = view.as_object_mut() {
                    map.insert(
                        "composer".to_owned(),
                        openehr_its::json::to_canonical_value(&composer),
                    );
                }
            }
            _ => {}
        }
    }

    /// The root's `archetype_node_id`.
    pub(crate) fn archetype_node_id(&self) -> Option<&str> {
        match self {
            Body::Party(party) => {
                Some(each_party!(party.as_ref(), p => p.archetype_node_id.as_str()))
            }
            Body::Canonical(view) => view.get("archetype_node_id").and_then(Value::as_str),
            Body::Composition(rm) => Some(rm.archetype_node_id.as_str()),
            Body::EhrStatus(rm) => Some(rm.archetype_node_id.as_str()),
            Body::EhrAccess(rm) => Some(rm.archetype_node_id.as_str()),
            Body::Folder(rm) => Some(rm.archetype_node_id.as_str()),
            Body::PartyRelationship(rm) => Some(rm.archetype_node_id.as_str()),
        }
    }

    /// The root's `archetype_details`, typed; `None` for a canonical body.
    fn archetype_details(&self) -> Option<&Archetyped> {
        match self {
            Body::Party(party) => each_party!(party.as_ref(), p => p.archetype_details.as_deref()),
            Body::Canonical(_) => None,
            Body::Composition(rm) => rm.archetype_details.as_deref(),
            Body::EhrStatus(rm) => rm.archetype_details.as_deref(),
            Body::EhrAccess(rm) => rm.archetype_details.as_deref(),
            Body::Folder(rm) => rm.archetype_details.as_deref(),
            Body::PartyRelationship(rm) => rm.archetype_details.as_deref(),
        }
    }

    /// The template id the root's `archetype_details` names.
    pub(crate) fn template_id(&self) -> Option<&str> {
        match self {
            Body::Canonical(view) => view
                .pointer("/archetype_details/template_id/value")
                .and_then(Value::as_str),
            typed => typed
                .archetype_details()
                .and_then(|details| details.template_id.as_ref())
                .map(|id| id.value.as_str()),
        }
    }

    /// The code string of a COMPOSITION's `category`.
    pub(crate) fn category_code(&self) -> Option<&str> {
        match self {
            Body::Composition(rm) => Some(rm.category.defining_code.code_string.as_str()),
            Body::Canonical(view) => view
                .pointer("/category/defining_code/code_string")
                .and_then(Value::as_str),
            _ => None,
        }
    }

    /// What the write rules of a caller read of a COMPOSITION body
    /// ([`super::content_facts`]); none of another kind.
    pub(crate) fn content_facts(&self) -> super::content_facts::ContentFacts<'_> {
        match self {
            Body::Composition(rm) => super::content_facts::ContentFacts::of_typed(rm),
            Body::Canonical(view) => super::content_facts::ContentFacts::of_canonical(view),
            _ => super::content_facts::ContentFacts::default(),
        }
    }

    /// Stamps the version's own `OBJECT_VERSION_ID` into the root `uid`.
    pub(crate) fn stamp_uid(&mut self, uid: &ObjectVersionId) {
        let stamped = Some(UidBasedId::ObjectVersionId(uid.clone()));
        match self {
            Body::Party(party) => each_party!(party.as_mut(), p => p.uid = stamped),
            Body::Canonical(view) => super::change::stamp_uid_value(view, uid),
            Body::Composition(rm) => rm.uid = stamped,
            Body::EhrStatus(rm) => rm.uid = stamped,
            Body::EhrAccess(rm) => rm.uid = stamped,
            Body::Folder(rm) => rm.uid = stamped,
            Body::PartyRelationship(rm) => rm.uid = stamped,
        }
    }

    /// Whether the body is expressible in the released generation set
    /// ([`openehr_rm::v1_2::released::expressible_in_released`]); `None` for a
    /// canonical body, which the caller judges by reading it.
    pub(crate) fn expressible_in_released(&self) -> Option<bool> {
        use openehr_rm::v1_2::released::expressible_in_released as judge;
        each_typed!(self, rm => Some(judge(rm.as_ref())), _view => None)
    }

    /// The typed RM view of the body, for the passes that walk any RM value;
    /// `None` for a canonical body.
    pub(crate) fn as_rm(&self) -> Option<&dyn openehr_rm::v1_2::access::RmObject> {
        use openehr_rm::v1_2::access::AsRm as _;
        each_typed!(self, rm => Some(rm.as_rm()), _view => None)
    }

    /// The typed RM view of the body, mutably; `None` for a canonical body.
    pub(crate) fn as_rm_mut(&mut self) -> Option<&mut dyn openehr_rm::v1_2::access::RmObject> {
        use openehr_rm::v1_2::access::AsRm as _;
        each_typed!(self, rm => Some(rm.as_rm_mut()), _view => None)
    }

    /// The one rendering of the body to compact canonical JSON, indexed
    /// ([`crate::storage::render::index`]).
    ///
    /// # Errors
    /// [`ServiceError`] when the body does not serialize (a non-finite
    /// number, unreachable for a codec-produced value, refused loudly anyway).
    pub(crate) fn render(&self) -> Result<Rendered, ServiceError> {
        match self {
            Body::Canonical(view) => Self::render_canonical(view),
            typed => {
                crate::telemetry::conversion::record("rm_to_text");
                crate::storage::render::index::render(typed)
                    .map_err(|e| ServiceError::internal("serialize the canonical body", e))
            }
        }
    }

    /// [`Self::render`] of a canonical value.
    ///
    /// # Errors
    /// As [`Self::render`].
    pub(crate) fn render_canonical(canonical: &Value) -> Result<Rendered, ServiceError> {
        crate::telemetry::conversion::record("value_to_text");
        crate::storage::render::index::render(canonical)
            .map_err(|e| ServiceError::internal("serialize the canonical body", e))
    }
}

/// The strict canonical-JSON door over a raw commit member's `data`: its
/// typed RM, which the member is validated over.
///
/// The generated envelope leaves a CONTRIBUTION member's `data` raw (the
/// `NewContribution<Value>` instantiation keeps the bytes the signature
/// covers), so the payload runs the same typed door every direct commit route
/// passes through: content that cannot be converted to
/// its RM resource is the 400 row (ITS-REST overview `Requests_and_responses.md`
/// §HTTP status codes). A `553|incomplete|` commit skips it, the generated
/// types making mandatory attributes structural where RM common master06
/// §Incomplete Content lifts those bounds.
///
/// # Errors
/// [`ServiceError::BadRequest`] when the strict reader refuses the payload.
pub(crate) fn decode_member(
    kind: Kind,
    data: &Value,
    incomplete: bool,
) -> Result<Option<Body>, ServiceError> {
    if incomplete {
        return Ok(None);
    }
    read_member(kind, data)
        .map(Some)
        .map_err(|e| ServiceError::canonical_decode(data, e))
}

/// [`decode_member`] over a member's canonical-JSON text, with no JSON value
/// between: its typed RM, or `None` for a text the strict reader refuses,
/// which its caller then reads through [`decode_member`] for the refusal.
pub(crate) fn decode_member_text(kind: Kind, text: &str) -> Option<Body> {
    read_member(kind, text).ok()
}

/// A canonical-JSON document the strict reader takes: its text or its value.
trait Canonical {
    /// The document read as `T`.
    fn read<T: serde::de::DeserializeOwned>(&self) -> Result<T, JsonParseError>;
}

impl Canonical for Value {
    fn read<T: serde::de::DeserializeOwned>(&self) -> Result<T, JsonParseError> {
        openehr_its::json::from_canonical_value(self)
    }
}

impl Canonical for str {
    fn read<T: serde::de::DeserializeOwned>(&self) -> Result<T, JsonParseError> {
        openehr_its::json::from_canonical_json(self)
    }
}

/// `data` read strictly as the RM class of `kind`.
fn read_member<S: Canonical + ?Sized>(kind: Kind, data: &S) -> Result<Body, JsonParseError> {
    fn party<T, S>(data: &S) -> Result<Body, JsonParseError>
    where
        T: serde::de::DeserializeOwned,
        Party: From<T>,
        S: Canonical + ?Sized,
    {
        data.read::<T>().map(|rm| Body::from(Party::from(rm)))
    }
    // NOTE: a party is read as the concrete class its container holds, so a body
    // of another class is refused at this door as it is on the direct routes.
    match kind {
        Kind::Composition => data.read::<Composition>().map(Body::from),
        Kind::EhrStatus => data.read::<EhrStatus>().map(Body::from),
        Kind::EhrAccess => data.read::<EhrAccess>().map(Body::from),
        Kind::Folder => data.read::<Folder>().map(Body::from),
        Kind::PartyRelationship => data.read::<PartyRelationship>().map(Body::from),
        Kind::Agent => party::<Agent, S>(data),
        Kind::Group => party::<Group, S>(data),
        Kind::Organisation => party::<Organisation, S>(data),
        Kind::Person => party::<Person, S>(data),
        Kind::Role => party::<Role, S>(data),
    }
}

/// The body a server-built write of `kind` commits from its canonical value
/// `data`: its typed RM, read through [`decode_member`], or the value itself
/// for a `553|incomplete|` one.
///
/// # Errors
/// [`ServiceError::BadRequest`] when the strict reader refuses `data`.
pub(crate) fn decoded(kind: Kind, data: Value, incomplete: bool) -> Result<Body, ServiceError> {
    Ok(decode_member(kind, &data, incomplete)?.unwrap_or(Body::Canonical(data)))
}

#[cfg(test)]
#[path = "body_tests.rs"]
mod tests;
