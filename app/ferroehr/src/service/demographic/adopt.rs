// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `ferroehr party-binding adopt`: the professional `ROLE` an administrator
//! gives each prepared `PERSON` of an existing deployment, so it binds.
//!
//! For each listed `PERSON` it commits a `ROLE` of the mapping's role
//! template with the `PERSON` as `performer` and a new `PERSON` version
//! listing it in `roles` (RM demographic `org.openehr.rm.demographic.actor.adoc`,
//! `…role.adoc`), all in one demographic CONTRIBUTION under the
//! administrator's committer, and writes one access record per adopted
//! `PERSON`. It refuses, naming them, every `PERSON` whose key the provenance
//! rule of the pre-flight does not accept ([`crate::party_binding::check`]),
//! so a key planted before the upgrade is never adopted in silence. No
//! openEHR spec governs a deployment's migration: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once"
)]

use openehr_base::prelude::{HierObjectId, ObjectId, PartyRef};
use openehr_rm::prelude::{PartyIdentified, PartyIdentifiedData, PartyProxy};
use serde::Serialize;
use serde_json::Value;
use sqlx::Connection as _;
use uuid::Uuid;

use crate::db::acquire::acquire;
use crate::error::ServiceError;
use crate::error::refusal::{Clash, Refusal};
use crate::ids::VoId;
use crate::party_binding::check::{self, CheckError, CheckInput};
use crate::service::FerroEhrService;
use crate::service::demographic::mapped_write::party_ref;
use crate::service::demographic::support;
use crate::service::demographic::types::PartyKind;
use crate::system_log::event::{
    AccessDomain, AuditEvent, EventActionCode, EventOutcome, ObjectClass,
};
use crate::versioning::audit::{AuditInput, change_type, dv_text};
use crate::versioning::change::{self, Change, WriteEnvelope};
use crate::versioning::contribution::typed_decode_gate;
use crate::versioning::lifecycle::state;

/// The audit description of an adoption.
const DESCRIPTION: &str = "party-binding adopt: the professional ROLE of a prepared PERSON";

/// What an adoption commits, and as whom.
#[derive(Debug, Clone, Default)]
pub struct AdoptInput {
    /// The provenance rule's input: the SCIM clients, the administrators
    /// (the first one named commits), the declared migrations and the key
    /// types.
    pub check: CheckInput,
    /// The `PERSON`s to adopt.
    pub parties: Vec<VoId>,
    /// The `PARTY_REF.namespace` the `PERSON` and `ROLE` name each other in.
    pub namespace: String,
}

/// What an adoption committed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AdoptReport {
    /// The administrators as named, the first one the committer.
    pub administrators: Vec<check::NamedAdministrator>,
    /// The CONTRIBUTION, absent when every `PERSON` was adopted already.
    pub contribution: Option<Uuid>,
    /// Each adopted `PERSON` with its new professional `ROLE`.
    pub adopted: Vec<AdoptedPerson>,
    /// The listed `PERSON`s that perform a professional `ROLE` already.
    pub already: Vec<Uuid>,
}

/// One adopted `PERSON`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AdoptedPerson {
    /// The `PERSON`.
    pub party: Uuid,
    /// Its new professional `ROLE`.
    pub role: Uuid,
}

impl FerroEhrService {
    /// Gives each `PERSON` of `input` its professional `ROLE`, in one
    /// demographic CONTRIBUTION committed by the first administrator named.
    ///
    /// # Errors
    /// - `409` when no mapping names a person template and a role template,
    ///   no administrator is named, a selector value two `PERSON`s carry
    ///   stands, or a listed `PERSON` is not a live key-bearing `PERSON` of
    ///   a mapped template in `532|complete|` or `553|incomplete|`.
    /// - `403` naming every listed `PERSON` whose key fails the provenance
    ///   rule.
    /// - The refusals of the commit, and a storage failure.
    pub async fn adopt_persons(&self, input: &AdoptInput) -> Result<AdoptReport, ServiceError> {
        let Some(role_template) = self.binding.professional_role().map(str::to_owned) else {
            return Err(ServiceError::conflict(
                "no [authz.party_binding.professional_role] is configured, so no ROLE makes a \
                 PERSON bindable",
            ));
        };
        let parties: Vec<Uuid> = input.parties.iter().map(|p| p.0).collect();
        let (administrators, candidates) = check::candidates(
            &self.demographic_pool,
            &self.binding,
            &input.check,
            &parties,
        )
        .await
        .map_err(refusal)?;
        let Some(admin) = administrators.first().map(|a| a.party) else {
            return Err(ServiceError::conflict(
                "an adoption is committed by an administrator: name one with --admin or \
                 --admin-key",
            ));
        };
        let unkeyed: Vec<String> = candidates
            .iter()
            .filter(|c| !c.keyed)
            .map(|c| c.party.to_string())
            .collect();
        if !unkeyed.is_empty() {
            return Err(ServiceError::conflict(format!(
                "no live PERSON of a mapped template carrying a binding key is {}",
                unkeyed.join(", ")
            )));
        }
        let failing: Vec<String> = candidates
            .iter()
            .filter(|c| !c.professional && !c.passes)
            .map(|c| c.party.to_string())
            .collect();
        if !failing.is_empty() {
            return Err(ServiceError::not_permitted(format!(
                "the binding key of PERSON {} was introduced by a committer that is no SCIM \
                 client, named administrator or declared migration, so it is not adopted",
                failing.join(", ")
            )));
        }
        let mut report = AdoptReport {
            administrators,
            ..AdoptReport::default()
        };
        let mut changes = Vec::new();
        let creation = self.adopt_audit(change_type::CREATION, admin, &input.namespace);
        let modification = self.adopt_audit(change_type::MODIFICATION, admin, &input.namespace);
        for candidate in &candidates {
            if candidate.professional {
                report.already.push(candidate.party);
                continue;
            }
            let (role, person) = self
                .adoption(VoId(candidate.party), &role_template, &input.namespace)
                .await?;
            report.adopted.push(AdoptedPerson {
                party: candidate.party,
                role: role.0,
            });
            changes.push((creation.clone(), role.1));
            changes.push((modification.clone(), person));
        }
        if changes.is_empty() {
            return Ok(report);
        }
        let mut conn = acquire(&self.demographic_pool).await?;
        let mut tx = conn.begin().await?;
        let committed = change::commit_contribution(
            &mut tx,
            None,
            None,
            &modification,
            changes,
            Vec::new(),
            &self.signing_ctx(),
        )
        .await?;
        tx.commit().await?;
        report.contribution = Some(committed.id);
        for adopted in &report.adopted {
            self.record_adoption(admin, adopted)?;
        }
        Ok(report)
    }

    /// The `ROLE` creation, with its container, and the `PERSON` version
    /// listing it, that adopt the `PERSON` `person`.
    async fn adoption(
        &self,
        person: VoId,
        role_template: &str,
        namespace: &str,
    ) -> Result<((Uuid, Change), Change), ServiceError> {
        let current = self
            .party_current(PartyKind::Person, person)
            .await?
            .filter(|current| !current.deleted)
            .ok_or_else(|| ServiceError::conflict(format!("PERSON {person} is deleted")))?;
        let lifecycle = self.head_lifecycle(person).await?;
        if !matches!(lifecycle.as_str(), state::COMPLETE | state::INCOMPLETE) {
            return Err(ServiceError::conflict(format!(
                "PERSON {person} is inactive or abandoned (RM common master06 §Version \
                 Lifecycle): reactivate it before it is adopted"
            )));
        }
        let mut body = self
            .read_party(PartyKind::Person, person, Some(current.tree), None)
            .await?
            .body;
        let role_id = self.allocate_party_id();
        let role_vo = role_id.id();
        let role = self
            .professional_role_body(role_template, person, namespace)
            .await?;
        let role_state = self.first_state(&role, role_template).await?;
        let reference = party_ref(namespace, "ROLE", role_vo);
        if let Some(object) = body.as_object_mut() {
            match object.get_mut("roles") {
                Some(Value::Array(roles)) => roles.push(reference),
                _ => {
                    object.insert("roles".to_owned(), Value::Array(vec![reference]));
                }
            }
        }
        for (kind, data, incomplete) in [
            (PartyKind::Role, &role, role_state == state::INCOMPLETE),
            (PartyKind::Person, &body, lifecycle == state::INCOMPLETE),
        ] {
            let kind = support::kind_of(kind);
            typed_decode_gate(kind, data, incomplete)?;
            self.validate_for_commit(kind, data, incomplete).await?;
        }
        let role_change = Change::create_allocated(
            role_id,
            support::kind_of(PartyKind::Role),
            role,
            WriteEnvelope {
                lifecycle_state: Some(role_state),
                ..WriteEnvelope::default()
            },
        );
        let person_change = Change::modify(
            person,
            support::kind_of(PartyKind::Person),
            body,
            Some(current.tree),
            None,
            WriteEnvelope {
                lifecycle_state: Some(lifecycle),
                ..WriteEnvelope::default()
            },
        );
        Ok(((role_vo.0, role_change), person_change))
    }

    /// The audit of an adoption: committed by the administrator `admin`,
    /// named by a reference to its `PERSON` (RM common master04 §Audit
    /// Details, `PARTY_IDENTIFIED.external_ref`).
    fn adopt_audit(&self, change_type: &str, admin: Uuid, namespace: &str) -> AuditInput {
        AuditInput {
            system_id: self.effective_system_id(),
            change_type: change_type.to_owned(),
            description: Some(dv_text(DESCRIPTION)),
            committer: PartyProxy::PartyIdentified(PartyIdentified::PartyIdentified(
                PartyIdentifiedData {
                    external_ref: Some(PartyRef {
                        namespace: namespace.to_owned(),
                        r#type: "PERSON".to_owned(),
                        id: ObjectId::HierObjectId(HierObjectId::from(admin)),
                    }),
                    name: None,
                    identifiers: None,
                },
            )),
            attestation: None,
        }
    }

    /// Records the adoption `adopted` by the administrator `admin`; nothing
    /// while auditing is off.
    fn record_adoption(&self, admin: Uuid, adopted: &AdoptedPerson) -> Result<(), ServiceError> {
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
            "party:{}#professional-role:{}",
            adopted.party, adopted.role
        ));
        event.user_id = admin.to_string();
        event.legal_basis = self.audit_legal_basis().map(str::to_owned);
        self.record_access(event)
    }
}

/// The service refusal of a pre-flight error: a duplicate or an unknown
/// administrator key is a `409`, every other error a storage failure.
fn refusal(e: CheckError) -> ServiceError {
    match e {
        CheckError::Storage(source) => ServiceError::Storage(source),
        other => ServiceError::Conflict(
            Refusal::new(Clash::State, "the PERSONs cannot be adopted").with_source(other),
        ),
    }
}
