// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The rows party heads should hold, derived from what their bodies hold:
//! the identifier rows and memberships of `PERSON`s with the `ROLE`s they
//! list, the tombstones of deleted `PERSON`s, and the identifier rows of
//! `GROUP`s. No openEHR spec governs the side table: our own design.

use uuid::Uuid;

use super::{DELETED, Head, Scope};
use crate::party_binding::BindingMapping;
use crate::party_binding::rows::{IdentifierRow, Selectors};

/// The rows the `PERSON` heads `heads` hold that `scope` holds: their
/// identifier rows, their memberships as `(group, member)` pairs, and each
/// `ROLE` they list with the `PERSON` that lists it.
#[derive(Debug, Default)]
pub(super) struct PersonRows {
    pub(super) identifiers: Vec<IdentifierRow>,
    pub(super) memberships: Vec<(Uuid, Uuid)>,
    pub(super) listed: Vec<(Uuid, Uuid)>,
}

impl PersonRows {
    /// The rows of `heads` that `scope` holds, under `mapping`.
    pub(super) fn of(mapping: &BindingMapping, heads: &[Head], scope: Scope<'_>) -> Self {
        let mut rows = Self::default();
        for head in heads {
            if mapping.membership.is_some()
                && head.lifecycle_state != DELETED
                && let Some(facts) = &head.facts
            {
                rows.memberships
                    .extend(facts.groups.iter().map(|group| (*group, head.vo_id)));
            }
            let Some(facts) = mapping
                .persons
                .keys()
                .find_map(|template| head.live_of(template))
            else {
                continue;
            };
            for (selector, identifier) in &facts.identifiers {
                if !scope.holds(selector, &identifier.issuer) {
                    continue;
                }
                rows.identifiers.push(IdentifierRow {
                    party: head.vo_id,
                    selector,
                    issuer: identifier.issuer.clone(),
                    identifier_type: identifier.id_type.clone(),
                    value: if *selector == "key" {
                        identifier.canonical()
                    } else {
                        identifier.id.clone()
                    },
                    lifecycle_state: head.lifecycle_state.clone(),
                });
            }
            rows.listed
                .extend(facts.roles.iter().map(|role| (*role, head.vo_id)));
        }
        rows
    }
}

/// The selectors of the `PERSON` rows a write of `scope` holds, with the one
/// issuer it is limited to, and whether it leaves tombstones.
pub(super) fn person_selectors(scope: Scope<'_>) -> (Selectors<'_>, bool) {
    match scope {
        Scope::Heads { tombstones } => ((&PERSON_SELECTORS, None), tombstones),
        Scope::KeySpace(issuer) => ((&["key"], Some(issuer)), true),
    }
}

/// Every selector of a `PERSON`'s identifier rows.
const PERSON_SELECTORS: [&str; 3] = ["key", "user_name", "external_id"];

/// The tombstone rows of the deleted heads among `heads`: the binding keys
/// each carried in its last version with a body, of a mapped template.
pub(super) fn tombstones_of(mapping: &BindingMapping, heads: &[Head]) -> Vec<IdentifierRow> {
    let mut out = Vec::new();
    for head in heads.iter().filter(|h| h.lifecycle_state == DELETED) {
        let Some((lifecycle_state, facts)) = &head.last else {
            continue;
        };
        if !facts
            .template
            .as_ref()
            .is_some_and(|t| mapping.persons.contains_key(t))
        {
            continue;
        }
        for (_, identifier) in facts.identifiers.iter().filter(|(s, _)| *s == "key") {
            out.push(IdentifierRow {
                party: head.vo_id,
                selector: "key",
                issuer: identifier.issuer.clone(),
                identifier_type: identifier.id_type.clone(),
                value: identifier.canonical(),
                lifecycle_state: lifecycle_state.clone(),
            });
        }
    }
    out
}

/// Every selector of a `GROUP`'s identifier rows.
pub(super) const GROUP_SELECTORS: [&str; 2] = ["group_external_id", "group_display_name"];

/// The `externalId` and `displayName` rows the `GROUP` heads `heads` hold;
/// `None` when `mapping` maps no group template.
pub(super) fn group_rows(mapping: &BindingMapping, heads: &[Head]) -> Option<Vec<IdentifierRow>> {
    let group = mapping.group.as_ref()?;
    let mut identifiers = Vec::new();
    for head in heads {
        let Some(facts) = head.live_of(&group.template) else {
            continue;
        };
        let row = |selector, issuer: &str, identifier_type: &str, value: &str| IdentifierRow {
            party: head.vo_id,
            selector,
            issuer: issuer.to_owned(),
            identifier_type: identifier_type.to_owned(),
            value: value.to_owned(),
            lifecycle_state: head.lifecycle_state.clone(),
        };
        for id in &facts.group_external_ids {
            identifiers.push(row("group_external_id", &id.issuer, &id.id_type, &id.id));
        }
        for name in &facts.display_names {
            identifiers.push(row("group_display_name", "", "", name));
        }
    }
    Some(identifiers)
}
