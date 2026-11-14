// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The roles of a SCIM User: each value of its `roles` (RFC 7643 §4.1.2) is
//! one `ROLE` its `PERSON` performs, named by the role.
//!
//! A `ROLE`'s type is its name (RM demographic
//! `org.openehr.rm.demographic.party.adoc`, `PARTY.type()` with the invariant
//! `type = name`), so a role value is the `DV_CODED_TEXT` `name` of a `ROLE`
//! of the professional role template, coded in the terminology the client's
//! `scim_provision` grant names, whose `performer` is the User's `PERSON`
//! and whose `time_validity` starts at the add. A removal ends it: the next
//! version holds the date of the removal as its exclusive upper bound, so
//! the role holds through the day before and the history stays, and the
//! `PERSON`'s `roles` no longer lists it, as it plays the role no more (RM
//! demographic `org.openehr.rm.demographic.actor.adoc`, `ACTOR.roles`). Both
//! versions the User's `PERSON` gains move its entity tag. The access
//! evaluator reads a role from the caller facts each `ROLE` commit writes
//! (`party_binding::grants::caller`), so a change decides the next request.
//! RFC 7643 sets no vocabulary for `roles` and RFC 7644 §2 leaves the
//! authorization model out: the mapping is our own design (#5625).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once; stored-content serving"
)]

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::error::ServiceError;
use crate::ids::VoId;
use crate::service::FerroEhrService;
use crate::service::demographic::mapped_write::party_ref;
use crate::service::demographic::types::PartyKind;
use crate::service::demographic::user_write::{ScimCommit, UserWriteError};
use crate::versioning::Kind;
use crate::versioning::audit::change_type;
use crate::versioning::change::{Change, WriteEnvelope};
use crate::versioning::contribution::typed_decode_gate;
use openehr_rm::draft::access_control::terminology::TERMINOLOGY_ID_ACCESS_CONTROL;

/// A role a User holds: the code of its `ROLE`'s name in its terminology.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct UserRole {
    /// The terminology the name is coded in.
    pub terminology: String,
    /// The code.
    pub code: String,
}

/// What a SCIM client writes of a User's roles: the terminology its grant
/// codes roles in, and the codes the request asks the User to hold.
#[derive(Debug, Clone)]
pub struct RoleWrite<'a> {
    /// The client's `binding_issuer`.
    pub binding_issuer: &'a str,
    /// The template of the `ROLE`s, the professional role template.
    pub role_template: &'a str,
    /// The `PARTY_REF.namespace` the `PERSON` and the `ROLE`s name each
    /// other in.
    pub namespace: &'a str,
    /// The terminology the roles are coded in.
    pub terminology: &'a str,
    /// The lifecycle state of the User's `PERSON`, which a version listing a
    /// new `ROLE` keeps.
    pub person_state: &'a str,
    /// The codes the User holds after the write.
    pub wanted: Vec<String>,
}

/// One role `ROLE` of a User in force: its container, its tree and body.
struct Held {
    vo_id: VoId,
    tree: crate::versioning::object_version_id::TreeId,
    body: Value,
}

impl FerroEhrService {
    /// The roles the `PERSON` `body` holds today: the coded names of the
    /// `ROLE`s of the professional role template it lists in `roles`, in force
    /// and not deleted, in code order.
    ///
    /// # Errors
    /// A storage failure.
    pub async fn user_roles(&self, body: &Value) -> Result<Vec<UserRole>, ServiceError> {
        let today = crate::party_binding::source::today();
        let mut roles: Vec<UserRole> = self
            .role_parties(body, &today)
            .await?
            .into_iter()
            .filter_map(|held| role_of(&held.body))
            .collect();
        roles.sort();
        roles.dedup();
        Ok(roles)
    }

    /// Makes the User `person` hold exactly the roles `write` asks for in its
    /// terminology: a new `ROLE` for each role it gains, listed in the
    /// `PERSON`'s `roles`, and the end of each it loses, in one demographic
    /// CONTRIBUTION as the client. Roles of another terminology are left as
    /// they are.
    ///
    /// # Errors
    /// The commit refusals and a storage failure.
    pub async fn set_user_roles(
        &self,
        person: VoId,
        write: &RoleWrite<'_>,
    ) -> Result<(), UserWriteError> {
        let today = crate::party_binding::source::today();
        let read = self
            .read_party(PartyKind::Person, person, None, None)
            .await?;
        let held: BTreeMap<String, Held> = self
            .role_parties(&read.body, &today)
            .await?
            .into_iter()
            .filter_map(|held| {
                role_of(&held.body)
                    .filter(|role| role.terminology == write.terminology)
                    .map(|role| (role.code, held))
            })
            .collect();
        let mut changes = Vec::new();
        let mut listed = read.body.clone();
        for (code, role) in &held {
            if !write.wanted.contains(code) {
                changes.push(self.ended(role, write.role_template, &today).await?);
                if let Some(roles) = listed.get_mut("roles").and_then(Value::as_array_mut) {
                    roles.retain(|reference| {
                        reference
                            .pointer("/id/value")
                            .and_then(Value::as_str)
                            .and_then(|value| VoId::from_uid_head(value).ok())
                            != Some(role.vo_id)
                    });
                }
            }
        }
        for code in &write.wanted {
            if held.contains_key(code) {
                continue;
            }
            let id = self.allocate_party_id();
            let mut body = self
                .professional_role_body(write.role_template, person, write.namespace)
                .await?;
            if let Some(object) = body.as_object_mut() {
                object.insert("name".to_owned(), coded(write.terminology, code));
                object.insert("time_validity".to_owned(), from(&today));
            }
            let state = self.first_state(&body, write.role_template).await?;
            typed_decode_gate(Kind::Role, &body, false)?;
            let reference = party_ref(write.namespace, "ROLE", id.id());
            if let Some(roles) = listed.as_object_mut().map(|object| {
                object
                    .entry("roles")
                    .or_insert_with(|| Value::Array(Vec::new()))
            }) && let Some(roles) = roles.as_array_mut()
            {
                roles.push(reference);
            }
            let envelope = WriteEnvelope {
                lifecycle_state: Some(state),
                ..WriteEnvelope::default()
            };
            changes.push(Change::create_allocated(id, Kind::Role, body, envelope));
        }
        if listed != read.body {
            let current = self
                .party_current(PartyKind::Person, person)
                .await?
                .ok_or_else(|| {
                    ServiceError::exception(format!("the PERSON {person} has no head"))
                })?;
            let envelope = WriteEnvelope {
                lifecycle_state: Some(write.person_state.to_owned()),
                ..WriteEnvelope::default()
            };
            changes.push(Change::modify(
                person,
                Kind::Person,
                listed,
                Some(current.tree),
                None,
                envelope,
            ));
        }
        if changes.is_empty() {
            return Ok(());
        }
        let commit = ScimCommit::new(write.binding_issuer, change_type::MODIFICATION);
        self.commit_scim(commit, changes).await.map(drop)
    }

    /// The next version of the role `role`, of `template`, ended before
    /// `today`.
    async fn ended(
        &self,
        role: &Held,
        template: &str,
        today: &str,
    ) -> Result<Change, ServiceError> {
        let mut body = role.body.clone();
        let lower = body
            .pointer("/time_validity/lower/value")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if let Some(object) = body.as_object_mut() {
            object.insert("time_validity".to_owned(), until(lower.as_deref(), today));
        }
        let state = self.first_state(&body, template).await?;
        let envelope = WriteEnvelope {
            lifecycle_state: Some(state),
            ..WriteEnvelope::default()
        };
        Ok(Change::modify(
            role.vo_id,
            Kind::Role,
            body,
            Some(role.tree),
            None,
            envelope,
        ))
    }

    /// The `ROLE`s of the professional role template the `PERSON` `body`
    /// lists in `roles`, in force on `today` and not deleted.
    async fn role_parties(&self, body: &Value, today: &str) -> Result<Vec<Held>, ServiceError> {
        let listed = body
            .get("roles")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let mut held = Vec::new();
        for reference in listed {
            // NOTE: a reference whose root is no UUID names no container of this store.
            let Some(role) = reference
                .pointer("/id/value")
                .and_then(Value::as_str)
                .and_then(|value| VoId::from_uid_head(value).ok())
            else {
                continue;
            };
            let Some(current) = self.party_current(PartyKind::Role, role).await? else {
                continue;
            };
            if current.deleted {
                continue;
            }
            let read = self.read_party(PartyKind::Role, role, None, None).await?;
            let professional = read
                .body
                .pointer("/archetype_details/template_id/value")
                .and_then(Value::as_str)
                .is_some_and(|template| self.binding.is_professional_role(template));
            if professional && in_force(read.body.get("time_validity"), today) {
                held.push(Held {
                    vo_id: role,
                    tree: current.tree,
                    body: read.body,
                });
            }
        }
        Ok(held)
    }
}

/// The role a `ROLE` `body` names: its `name` when that is a `DV_CODED_TEXT`
/// of a terminology other than the operation terminology.
fn role_of(body: &Value) -> Option<UserRole> {
    let name = body.get("name")?;
    if name.get("_type").and_then(Value::as_str) != Some("DV_CODED_TEXT") {
        return None;
    }
    let terminology = name
        .pointer("/defining_code/terminology_id/value")
        .and_then(Value::as_str)?;
    let code = name
        .pointer("/defining_code/code_string")
        .and_then(Value::as_str)?;
    (terminology != TERMINOLOGY_ID_ACCESS_CONTROL).then(|| UserRole {
        terminology: terminology.to_owned(),
        code: code.to_owned(),
    })
}

/// Whether the `DV_INTERVAL<DV_DATE>` `validity` holds the ISO 8601 date
/// `today`; an absent interval holds every date.
fn in_force(validity: Option<&Value>, today: &str) -> bool {
    let Some(validity) = validity else {
        return true;
    };
    let bound = |side: &str| {
        validity
            .pointer(&format!("/{side}/value"))
            .and_then(Value::as_str)
    };
    let included = |side: &str| {
        validity
            .get(format!("{side}_included"))
            .and_then(Value::as_bool)
            .unwrap_or(true)
    };
    let above = bound("lower").is_none_or(|lower| {
        if included("lower") {
            lower <= today
        } else {
            lower < today
        }
    });
    let below = bound("upper").is_none_or(|upper| {
        if included("upper") {
            today <= upper
        } else {
            today < upper
        }
    });
    above && below
}

/// The `DV_CODED_TEXT` of `code` in `terminology`, its value the code.
fn coded(terminology: &str, code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": terminology },
                "code_string": code } })
}

/// The validity of a role added on `today`: from that date, with no end.
fn from(today: &str) -> Value {
    json!({ "_type": "DV_INTERVAL",
            "lower": { "_type": "DV_DATE", "value": today },
            "lower_included": true, "lower_unbounded": false, "upper_unbounded": true })
}

/// The validity of a role from `lower` (unbounded when `None`) ended before
/// `today`.
fn until(lower: Option<&str>, today: &str) -> Value {
    match lower {
        Some(lower) => json!({ "_type": "DV_INTERVAL",
            "lower": { "_type": "DV_DATE", "value": lower },
            "upper": { "_type": "DV_DATE", "value": today },
            "lower_included": true, "upper_included": false,
            "lower_unbounded": false, "upper_unbounded": false }),
        None => json!({ "_type": "DV_INTERVAL",
            "upper": { "_type": "DV_DATE", "value": today },
            "upper_included": false, "lower_unbounded": true, "upper_unbounded": false }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_role_holds_within_its_validity() {
        assert!(in_force(None, "2026-11-14"));
        assert!(in_force(Some(&from("2026-11-14")), "2026-11-14"));
        assert!(!in_force(Some(&from("2026-11-15")), "2026-11-14"));
        let ended = until(Some("2026-11-01"), "2026-11-14");
        assert!(in_force(Some(&ended), "2026-11-13"));
        assert!(!in_force(Some(&ended), "2026-11-14"));
        assert!(!in_force(
            Some(&until(Some("2026-11-14"), "2026-11-14")),
            "2026-11-14"
        ));
    }

    #[test]
    fn a_role_is_a_coded_name_outside_the_operation_terminology() {
        let named = |name: Value| json!({ "_type": "ROLE", "name": name });
        assert_eq!(
            role_of(&named(coded("urn:example:roles", "NURSE"))),
            Some(UserRole {
                terminology: "urn:example:roles".to_owned(),
                code: "NURSE".to_owned()
            })
        );
        assert_eq!(
            role_of(&named(coded(TERMINOLOGY_ID_ACCESS_CONTROL, "NURSE"))),
            None
        );
        assert_eq!(
            role_of(&named(json!({ "_type": "DV_TEXT", "value": "Nurse" }))),
            None
        );
    }
}
