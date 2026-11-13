// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The demographic references a party write carries and the version
//! containers they must name.
//!
//! RM demographic `master02-demographic_package.adoc` §Party Relationships has
//! a relationship's `source` and `target` denote "the Version container of a
//! Party" by `HIER_OBJECT_ID`, "rather than `OBJECT_VERSION_IDs`, which would
//! denote particular versions"; `role.adoc` has `performer` be the "Reference
//! to Version container of Actor playing the role", and `actor.adoc` has
//! `roles` be the "Identifiers of the Version container for each Role played
//! by this Party". No invariant states either attribute's rule, so each
//! refusal names the attribute it reads.
//!
//! [`shape`] judges one reference by itself, at the write door. [`check`]
//! judges that each named container exists and is of the right kind, in the
//! commit's transaction after the whole set is written, since one set may
//! create a ROLE and the PERSON that names it.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once"
)]

use std::collections::BTreeMap;

use openehr_base::prelude::{ObjectId, PartyRef};
use openehr_base::validate::Validate;
use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::error::{ServiceError, Violation};
use crate::versioning::Kind;
use crate::versioning::change::Change;

/// The label a reference to one version rather than its container carries,
/// on every attribute and both relationship representations.
const VERSION_CONTAINER: &str = "RM demographic master02";

/// What a demographic reference must name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Referent {
    /// Any party: a relationship's `source` or `target`.
    Party,
    /// An Actor: `ROLE.performer`.
    Actor,
    /// A Role: an entry of `ACTOR.roles`.
    Role,
}

impl Referent {
    /// The `PARTY_REF.type` values that may name the referent: its own class,
    /// its subtypes and its supertypes, `ANY` among them (BASE `OBJECT_REF.type`,
    /// "Name of the class (concrete or abstract) of object to which this
    /// identifier type refers").
    fn types(self) -> &'static [&'static str] {
        match self {
            Referent::Party => &[
                "ANY",
                "PARTY",
                "ACTOR",
                "PERSON",
                "ORGANISATION",
                "GROUP",
                "AGENT",
                "ROLE",
            ],
            Referent::Actor => &[
                "ANY",
                "PARTY",
                "ACTOR",
                "PERSON",
                "ORGANISATION",
                "GROUP",
                "AGENT",
            ],
            Referent::Role => &["ANY", "PARTY", "ROLE"],
        }
    }

    /// The stored kinds a named container may have.
    fn kinds(self) -> &'static [Kind] {
        match self {
            Referent::Party => &[
                Kind::Agent,
                Kind::Group,
                Kind::Organisation,
                Kind::Person,
                Kind::Role,
            ],
            Referent::Actor => &[Kind::Agent, Kind::Group, Kind::Organisation, Kind::Person],
            Referent::Role => &[Kind::Role],
        }
    }

    /// The attribute documentation each refusal cites.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Referent::Party => "RM demographic party_relationship.adoc",
            Referent::Actor => "RM demographic role.adoc performer",
            Referent::Role => "RM demographic actor.adoc roles",
        }
    }

    /// What the container must be, for a refusal's detail.
    fn noun(self) -> &'static str {
        match self {
            Referent::Party => "a party",
            Referent::Actor => "an Actor (AGENT, GROUP, ORGANISATION or PERSON)",
            Referent::Role => "a ROLE",
        }
    }
}

/// Judges one decoded reference at `path` by itself: a version container
/// (`HIER_OBJECT_ID`, never `OBJECT_VERSION_ID`), the BASE `PARTY_REF` and
/// `OBJECT_REF` invariants, and a `type` that can name `referent`.
///
/// # Errors
/// [`ServiceError::Unprocessable`] naming the rule and `path`.
pub(crate) fn shape(
    reference: &PartyRef,
    path: &str,
    referent: Referent,
) -> Result<(), ServiceError> {
    if matches!(reference.id, ObjectId::ObjectVersionId(_)) {
        return Err(ServiceError::content_invalid(
            Violation::new(
                "must identify the party's version container (HIER_OBJECT_ID), \
                 not one version (OBJECT_VERSION_ID)",
            )
            .with_path(format!("{path}.id"))
            .with_invariant(VERSION_CONTAINER),
        ));
    }
    // PARTY_REF.Type_validity + OBJECT_REF.namespace (BASE `party_ref.adoc` /
    // `object_ref.adoc`), the class's own realization in `openehr-base`.
    let violations = reference.invariants();
    if !violations.is_empty() {
        return Err(ServiceError::content_invalid(
            Violation::new("fails its BASE class invariants")
                .with_path(path)
                .with_causes(violations.into_iter().take(1).collect()),
        ));
    }
    if !referent.types().contains(&reference.r#type.as_str()) {
        return Err(ServiceError::content_invalid(
            Violation::new(format!(
                "names a {}, but must name {}",
                reference.r#type,
                referent.noun()
            ))
            .with_path(format!("{path}.type"))
            .with_invariant(referent.label()),
        ));
    }
    Ok(())
}

/// Decodes the reference `value` at `path` and judges it with [`shape`].
///
/// Decoding enforces the mandatory `OBJECT_REF` attributes (`id`,
/// `namespace`, `type` are 1..1, BASE `object_ref.adoc` §Attributes).
///
/// # Errors
/// [`ServiceError::Unprocessable`] for a reference that does not decode, or
/// one [`shape`] refuses.
pub(crate) fn shape_of(value: &Value, path: &str, referent: Referent) -> Result<(), ServiceError> {
    let typed = openehr_its::json::from_canonical_value::<PartyRef>(value).map_err(|e| {
        ServiceError::content_invalid(
            Violation::new("is not a well-formed PARTY_REF")
                .with_path(path)
                .with_decode_failure(&e),
        )
    })?;
    shape(&typed, path, referent)
}

/// One reference a committed body carries: the container it names (`None`
/// when its id is no container id this store can hold), what it must be, and
/// its path in the body.
#[derive(Debug, Clone)]
pub(crate) struct Reference {
    container: Option<Uuid>,
    referent: Referent,
    path: String,
}

/// The references to judge in the body of `change`, each at its path: a
/// ROLE's `performer`, an Actor's `roles` and a standalone relationship's
/// `source` and `target`.
///
/// An inline relationship's `target` is not among them: no openEHR rule binds
/// it to a container this server holds, and the reverse index tolerates one it
/// does not (our own design).
pub(crate) fn of_change(change: &Change) -> Vec<Reference> {
    let (kind, body) = match change {
        Change::Create {
            kind, canonical, ..
        }
        | Change::Modify {
            kind, canonical, ..
        } => (*kind, canonical),
        Change::Delete { .. } => return Vec::new(),
    };
    let mut out = Vec::new();
    let mut add = |value: Option<&Value>, path: String, referent: Referent| {
        if let Some(value) = value.filter(|v| !v.is_null()) {
            let container = value
                .pointer("/id/value")
                .and_then(Value::as_str)
                .and_then(crate::party_binding::derive::container);
            out.push(Reference {
                container,
                referent,
                path,
            });
        }
    };
    match kind {
        Kind::PartyRelationship => {
            for end in ["source", "target"] {
                add(
                    body.get(end),
                    format!("PARTY_RELATIONSHIP.{end}"),
                    Referent::Party,
                );
            }
        }
        Kind::Role => add(
            body.get("performer"),
            "ROLE.performer".to_owned(),
            Referent::Actor,
        ),
        Kind::Agent | Kind::Group | Kind::Organisation | Kind::Person => {
            for (i, role) in body
                .get("roles")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
            {
                add(
                    Some(role),
                    format!("{}.roles[{i}]", kind.as_str()),
                    Referent::Role,
                );
            }
        }
        _ => {}
    }
    out
}

/// Refuses the first of `references` whose container this server does not
/// hold, or holds as another kind than it must be. `member` is the index of
/// the CONTRIBUTION version each reference came from, when it came from one.
///
/// One indexed read of the containers' heads (`vo_head` by `vo_id`), on the
/// commit's transaction, so a container the set itself creates is seen.
///
/// NOTE: a logically deleted container still exists (RM common master06 §Logical
/// Deletion), so a reference to one stands: our own design.
///
/// # Errors
/// [`ServiceError::Unprocessable`] naming the attribute and its path; a
/// storage failure.
pub(crate) async fn check(
    tx: &mut PgConnection,
    references: &[(Option<usize>, Reference)],
) -> Result<(), ServiceError> {
    if references.is_empty() {
        return Ok(());
    }
    let mut ids: Vec<Uuid> = references.iter().filter_map(|(_, r)| r.container).collect();
    ids.sort_unstable();
    ids.dedup();
    let kinds: BTreeMap<Uuid, Kind> = crate::storage::party_heads::kinds(&mut *tx, &ids)
        .await?
        .into_iter()
        .filter_map(|(id, kind)| Kind::from_type(&kind).map(|k| (id, k)))
        .collect();
    for (member, reference) in references {
        let held = reference
            .container
            .and_then(|id| kinds.get(&id))
            .is_some_and(|kind| reference.referent.kinds().contains(kind));
        if held {
            continue;
        }
        let refusal = ServiceError::content_invalid(
            Violation::new(format!(
                "must name the version container of {} this server holds",
                reference.referent.noun()
            ))
            .with_path(reference.path.clone())
            .with_invariant(reference.referent.label()),
        );
        return Err(match member {
            Some(index) => refusal.for_version_member(*index),
            None => refusal,
        });
    }
    Ok(())
}
