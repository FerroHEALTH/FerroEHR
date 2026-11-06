// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The administrator re-key of a `PERSON`'s binding key: the one deliberate
//! change of a key the protection otherwise holds immutable (RFC 7643 §2.2
//! `mutability: "immutable"`).
//!
//! It refuses a new key that is the administrator's own login or a key
//! already bound to another party, retires the old key in the same commit
//! that binds the new one, and writes one access record carrying both, so an
//! administrator cannot point a colleague's `PERSON` at their own login
//! unseen. No openEHR spec governs the binding: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once; stored-content serving"
)]

use serde_json::Value;

use crate::error::refusal::Missing;
use crate::error::{ServiceError, Violation};
use crate::ids::VoId;
use crate::party_binding::IdentifierSelector;
use crate::party_binding::protect::{Rekey, rekey};
use crate::service::FerroEhrService;
use crate::service::demographic::precondition::Precondition;
use crate::service::demographic::types::PartyKind;
use crate::service::version_update::WriteRequest;
use crate::system_log::event::{
    AccessDomain, AuditEvent, EventActionCode, EventOutcome, ObjectClass,
};

/// A binding key: its `DV_IDENTIFIER.issuer` and its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingKey {
    /// The `DV_IDENTIFIER.issuer`, the `identifier_issuer` of an
    /// `[[auth.party]]` table.
    pub issuer: String,
    /// The `DV_IDENTIFIER.id`.
    pub value: String,
}

impl FerroEhrService {
    /// Changes the binding key of the `PERSON` `party` to `to`, under an
    /// administrator's request.
    ///
    /// # Errors
    /// - `404` when `party` is no live `PERSON`.
    /// - `422` when `to` names no issuer or no value.
    /// - `409` when no party mapping binds its template, it carries no key
    ///   under `to.issuer`, or `to` is already bound to another party.
    /// - `403` when `to` is the requesting administrator's own login.
    /// - The refusals of a party update, and a storage failure.
    pub async fn rekey_party(&self, party: VoId, to: BindingKey) -> Result<(), ServiceError> {
        check_key(&to)?;
        let current = self
            .party_current(PartyKind::Person, party)
            .await?
            .filter(|current| !current.deleted)
            .ok_or_else(|| {
                ServiceError::not_found(Missing::VersionedObject, format!("PERSON {party}"))
            })?;
        let read = self
            .read_party(PartyKind::Person, party, Some(current.tree), None)
            .await?;
        let mut body = read.body;
        let template = body
            .pointer("/archetype_details/template_id/value")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let Some(selector) = self.binding.key_selector(&template).cloned() else {
            return Err(ServiceError::conflict(format!(
                "PERSON {party} is of no template [authz.party_binding.person] binds"
            )));
        };
        let to = BindingKey {
            value: canonical(selector.identifier_type(), &to.value),
            issuer: to.issuer,
        };
        refuse_own_login(&to)?;
        let from = rewrite(&mut body, &selector, &to).ok_or_else(|| {
            ServiceError::conflict(format!(
                "PERSON {party} carries no binding key of the issuer {:?} to re-key",
                to.issuer
            ))
        })?;
        let change = Rekey {
            party: party.0,
            from: Some((from.issuer.clone(), from.value.clone())),
            to: (to.issuer.clone(), to.value.clone()),
        };
        let precondition = Precondition::parse(None, Some(current.tree))?;
        rekey(
            change,
            Box::pin(self.commit_party_update(current, precondition, WriteRequest::new(body))),
        )
        .await
        .map_err(|e| match e.classified() {
            ServiceError::Conflict(_) => ServiceError::conflict(format!(
                "the binding key {:?} of the issuer {:?} is already bound to another party",
                to.value, to.issuer
            )),
            other => other,
        })?;
        self.record_rekey(party, &from, &to)
    }

    /// Records the re-key of `party` from `from` to `to`.
    fn record_rekey(
        &self,
        party: VoId,
        from: &BindingKey,
        to: &BindingKey,
    ) -> Result<(), ServiceError> {
        if !self.audit_enabled() {
            return Ok(());
        }
        let mut event = AuditEvent::new(
            EventActionCode::Update,
            ObjectClass::Demographic,
            EventOutcome::Success,
        );
        event.domain = AccessDomain::Demographic;
        event.object_id = Some(format!(
            "party:{}#binding-key:{}|{}>{}|{}",
            party.0, from.issuer, from.value, to.issuer, to.value
        ));
        // NOTE: a re-key outside a request (none is wired) names no reader.
        event.user_id = crate::versioning::committer::current_committer()
            .map(|committer| committer.subject)
            .unwrap_or_default();
        event.legal_basis = self.audit_legal_basis().map(str::to_owned);
        self.record_access(event)
    }
}

/// `value` in the canonical form its `identifier_type` declares: a GUID or
/// UUID in lower case, as the side table holds it.
fn canonical(identifier_type: &str, value: &str) -> String {
    if identifier_type.eq_ignore_ascii_case("guid") || identifier_type.eq_ignore_ascii_case("uuid")
    {
        value.to_ascii_lowercase()
    } else {
        value.to_owned()
    }
}

/// Refuses `to` when it is the requesting administrator's own login: the key
/// their own login is bound by.
fn refuse_own_login(to: &BindingKey) -> Result<(), ServiceError> {
    let own = crate::versioning::committer::current_committer()
        .and_then(|committer| committer.party)
        .is_some_and(|bound| {
            bound.identifier.issuer() == to.issuer
                && bound.identifier.value().eq_ignore_ascii_case(&to.value)
        });
    if own {
        return Err(ServiceError::not_permitted(
            "an administrator never re-keys a PERSON to their own login",
        ));
    }
    Ok(())
}

/// Replaces the binding key of `to.issuer` that `selector` selects in `body`
/// with `to`, returning the key it replaced; `None` when the body carries no
/// such key.
fn rewrite(body: &mut Value, selector: &IdentifierSelector, to: &BindingKey) -> Option<BindingKey> {
    let mut identifiers = Vec::new();
    let mut steps = selector.steps();
    if steps
        .last()
        .is_some_and(|(attribute, node)| attribute == "value" && node.is_none())
    {
        steps.pop();
    }
    collect(body, &steps, &mut identifiers);
    let mut replaced = None;
    for identifier in identifiers {
        let held = |name: &str| {
            identifier
                .get(name)
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        if held("_type").as_deref() != Some("DV_IDENTIFIER")
            || held("type").as_deref() != Some(selector.identifier_type())
            || held("issuer").as_deref() != Some(to.issuer.as_str())
        {
            continue;
        }
        let Some(value) = held("id") else {
            continue;
        };
        replaced.get_or_insert(BindingKey {
            issuer: to.issuer.clone(),
            value: canonical(selector.identifier_type(), &value),
        });
        if let Some(id) = identifier.get_mut("id") {
            *id = Value::String(to.value.clone());
        }
    }
    replaced
}

/// Collects the `value` of every `ELEMENT` at `steps` under `node`.
fn collect<'a>(
    node: &'a mut Value,
    steps: &[(String, Option<String>)],
    out: &mut Vec<&'a mut Value>,
) {
    let Some(((attribute, node_id), rest)) = steps.split_first() else {
        if let Some(value) = node.get_mut("value") {
            out.push(value);
        }
        return;
    };
    let Some(child) = node.get_mut(attribute.as_str()) else {
        return;
    };
    let children: Vec<&mut Value> = match child {
        Value::Array(items) => items.iter_mut().collect(),
        other => vec![other],
    };
    for child in children {
        let matches = node_id.as_deref().is_none_or(|wanted| {
            child.get("archetype_node_id").and_then(Value::as_str) == Some(wanted)
        });
        if matches {
            collect(child, rest, out);
        }
    }
}

/// Refuses a new key that names no issuer or no value.
fn check_key(to: &BindingKey) -> Result<(), ServiceError> {
    if to.issuer.trim().is_empty() || to.value.trim().is_empty() {
        return Err(ServiceError::content_invalid(
            Violation::new("a binding key names an issuer and a value").with_path("/"),
        ));
    }
    Ok(())
}
