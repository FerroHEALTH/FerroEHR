// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The origins of data this deployment recognises, read from the policy
//! grants.
//!
//! Regulation (EU) 2025/327 Annex II 3.4 asks the storing components to
//! "support different retention periods and access rights that take into
//! account the origins and categories of electronic health data"
//! (`docs/law/eu/ehds/text.html`). FerroEHR records the origin of every version
//! at commit (`version.origins`, [`crate::versioning::origins`]): each
//! `FEEDER_AUDIT.originating_system_audit.system_id` the body carries, as the
//! sending system chose it (RM common, `FEEDER_AUDIT_DETAILS.system_id`). A
//! free-text value can key neither a retention period nor an access rule, so
//! an origin is recognised when a party holds a grant of
//! [`Operation::DataOrigin`] whose `origin` element names it, in force on the
//! request. A body with no feeder audit, committed by a caller holding such a
//! grant, is stamped with that grant's origin; any other direct commit keeps
//! this server's `system_id`.
//!
//! **No openEHR spec governs this — our own design/extension.**

use std::collections::BTreeSet;

use serde_json::Value;

use crate::error::ServiceError;
use crate::party_binding::grants::capabilities::Capabilities;
use crate::party_binding::grants::live::LiveGrants;
use crate::party_binding::grants::operation::{ORIGIN, Operation};

/// The origins every grant of [`Operation::DataOrigin`] in force at `now`
/// names, from `grants`' copy: the origins a retention period may be keyed
/// on.
#[must_use]
pub fn registered(grants: &LiveGrants, now: jiff::Timestamp) -> BTreeSet<String> {
    grants
        .all_in_force(Operation::DataOrigin, now)
        .iter()
        .flat_map(|(_, grant)| grant.credentials().identified(ORIGIN))
        .filter(|origin| !origin.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The origin a body with no `FEEDER_AUDIT` is stamped with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectOrigin {
    /// The origin of the committer's grant, or this server's `system_id`.
    One(String),
    /// The committer's grants name more than one origin, so a direct commit
    /// comes from none of them.
    Ambiguous(Vec<String>),
}

impl DirectOrigin {
    /// The direct origin of a caller holding `capabilities`: the one origin
    /// its grants of [`Operation::DataOrigin`] name, or `system_id` when they
    /// name none.
    #[must_use]
    pub fn of(capabilities: &Capabilities, system_id: &str) -> Self {
        let named: BTreeSet<&str> = capabilities
            .of(Operation::DataOrigin)
            .flat_map(|grant| grant.credentials().identified(ORIGIN))
            .filter(|origin| !origin.is_empty())
            .collect();
        let mut named: Vec<String> = named.into_iter().map(str::to_owned).collect();
        match named.len() {
            0 => Self::One(system_id.to_owned()),
            1 => Self::One(named.remove(0)),
            _ => Self::Ambiguous(named),
        }
    }

    /// The direct origin of the running request's caller
    /// ([`Capabilities::current`]).
    #[must_use]
    pub fn current(system_id: &str) -> Self {
        Self::of(&Capabilities::current(), system_id)
    }

    /// The origins this direct origin may stand for: the one, or each of the
    /// ambiguous ones, which an attribute check is asked about before the
    /// commit refuses.
    #[must_use]
    pub fn candidates(&self) -> Vec<String> {
        match self {
            Self::One(origin) => vec![origin.clone()],
            Self::Ambiguous(origins) => origins.clone(),
        }
    }

    /// The origin set `canonical` is stamped with: its feeder origins, or
    /// this direct origin when it carries none.
    ///
    /// # Errors
    /// [`ServiceError::Forbidden`] (`403`) when the body carries no feeder
    /// audit and the committer's grants name more than one origin.
    pub fn stamp(&self, canonical: &Value) -> Result<Vec<String>, ServiceError> {
        let fed = crate::versioning::origins::fed(canonical);
        if !fed.is_empty() {
            return Ok(fed.into_iter().collect());
        }
        match self {
            Self::One(origin) => Ok(vec![origin.clone()]),
            Self::Ambiguous(origins) => Err(ServiceError::not_permitted(format!(
                "the committer's data_origin grants name the origins {}, so a body with no \
                 FEEDER_AUDIT comes from none of them: leave the committer one origin, or \
                 name the origin in the body's FEEDER_AUDIT",
                origins.join(", ")
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;
    use uuid::Uuid;

    use super::*;
    use crate::party_binding::grants::capabilities::Grant;
    use crate::party_binding::policy_mark::TERMINOLOGY;

    fn origin_grant(origin: &str) -> Grant {
        let tree = json!({"_type": "ITEM_TREE", "items": [
            {"_type": "ELEMENT",
             "name": {"_type": "DV_CODED_TEXT", "value": ORIGIN,
                      "defining_code": {"terminology_id": {"value": TERMINOLOGY},
                                        "code_string": ORIGIN}},
             "value": {"_type": "DV_IDENTIFIER", "id": origin}}]});
        Grant::new(Operation::DataOrigin, Uuid::nil(), Some(Arc::new(tree)))
    }

    #[test]
    fn a_granted_origin_stamps_a_direct_commit_and_any_other_the_server() {
        let lab = Capabilities::new(None, vec![origin_grant("lab.example")]);
        let direct = DirectOrigin::of(&lab, "ferroehr");
        assert_eq!(direct, DirectOrigin::One("lab.example".to_owned()));
        assert_eq!(
            direct.stamp(&json!({"_type": "COMPOSITION"})).ok(),
            Some(vec!["lab.example".to_owned()])
        );
        assert_eq!(
            DirectOrigin::of(&Capabilities::default(), "ferroehr"),
            DirectOrigin::One("ferroehr".to_owned())
        );
    }

    #[test]
    fn two_origins_stamp_no_direct_commit_but_leave_a_fed_one() {
        let both = Capabilities::new(
            None,
            vec![origin_grant("lab.example"), origin_grant("pacs.example")],
        );
        let direct = DirectOrigin::of(&both, "ferroehr");
        assert_eq!(direct.candidates().len(), 2);
        assert!(matches!(
            direct.stamp(&json!({"_type": "COMPOSITION"})),
            Err(ServiceError::Forbidden(_))
        ));
        let fed = json!({"_type": "COMPOSITION", "feeder_audit": {"_type": "FEEDER_AUDIT",
            "originating_system_audit": {"system_id": "ris.example"}}});
        assert_eq!(direct.stamp(&fed).ok(), Some(vec!["ris.example".to_owned()]));
    }
}
