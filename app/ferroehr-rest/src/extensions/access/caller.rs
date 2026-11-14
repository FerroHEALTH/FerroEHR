// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Who asks the access evaluator, and what: the request's caller, operation,
//! purpose and emergency, read once before dispatch.
//!
//! The caller's kind comes from what authentication admitted: the patient
//! and a person acting for them (EHDS Art. 3 and 4(2)), an access service
//! (an `access_service` grant in the CDR, or a verified caller of a
//! `federation` issuer; EHDS Art. 10), a system client, and every other
//! caller a professional. Its party, clearances, permitted purposes and care
//! teams are the 5.7.1 demographic facts the request already holds
//! (`ferroehr::party_binding::grants`), and it acts for its care teams, the
//! provider its client acts for and the `GROUP`s that provider is a member
//! of (RM demographic `PARTY_RELATIONSHIP`, coded `member_of`; #5575). The
//! purpose is the declared code the audit layer accepted, and an emergency
//! is that code being one of `[audit] emergency_purpose_codes`, with
//! `[authz.rbac] emergency_access_role` for the break-glass steps (EHDS Art. 11(5)). No openEHR spec governs the
//! caller's kind or its facts: our own design (#5513).

use std::sync::Arc;

use ferroehr::access_eval::condition::{Asking, RecordFacts, RequestAccess};
use ferroehr::access_eval::procedure::{Caller, Emergency};
use ferroehr::config::authz::EhrAccessDefault;
use ferroehr::ids::EhrId;
use ferroehr::party_binding::grants::agreement::Accessor;
use ferroehr::party_binding::grants::capabilities::Capabilities;
use ferroehr::party_binding::grants::operation::Operation;
use ferrohealth_smart::kind::CallerKind as SmartKind;
use ferrohealth_smart::openehr;
use ferrohealth_smart::openehr::smart_scopes::{
    Compartment, Permission, ResourceFamily, SmartScope,
};
use openehr_rm::draft::access_control::terminology::{AccessDefault, CallerKind, OperationClass};
use uuid::Uuid;

use super::authn::trc::TrcPatientEhr;
use super::authn::{Principal, current_principal};
use crate::api::operation::{Verb, operation};
use crate::state::AppState;

/// The evaluator of the running request of `op`; `None` for the controller's
/// own operations (`admin_*`), which no directive binds. The read of the
/// change feed is judged: each event names EHR data, and the feed publishes
/// only what its reader may read (#5587).
pub(crate) fn request_access(state: &AppState, op: &str) -> Option<Arc<RequestAccess>> {
    if op.starts_with("admin_") && op != crate::api::admin::outbox::READ {
        return None;
    }
    let principal = current_principal();
    let principal = principal.as_ref();
    let backend = state.backend();
    let settings = backend.settings_caller();
    let capabilities = Capabilities::current();
    let kind = kind(principal, settings.subject(), &capabilities);
    let roles = principal.map_or_else(Vec::new, |p| p.roles.clone());
    let authz = state.authz();
    let (server_default, admin_role) = authz
        .as_deref()
        .map_or((EhrAccessDefault::Open, "ADMIN".to_owned()), |handle| {
            (handle.ehr_access_default(), handle.admin_role().to_owned())
        });
    let mut acts_for: Vec<String> = capabilities
        .facts()
        .care_teams()
        .iter()
        .map(|team| team.team().to_string())
        .collect();
    if let Some(provider) = capabilities
        .client()
        .and_then(|client| client.provider.as_ref())
    {
        acts_for.push(provider.party.to_string());
    }
    acts_for.extend(
        capabilities
            .facts()
            .provider_groups()
            .iter()
            .map(Uuid::to_string),
    );
    let patient_grants_only = patient_grants_only(principal, op);
    let caller = Caller {
        kind,
        party: settings.party().map(str::to_owned),
        own_ehr: own_ehr(principal, kind, patient_grants_only),
        patient_grants_only,
        acts_for,
        administrator: roles.iter().any(|r| r.eq_ignore_ascii_case(&admin_role)),
        roles,
        facts: capabilities.facts().clone(),
        settings,
    };
    let purpose = ferroehr::system_log::access_context::current_purpose();
    let asking = Asking {
        operation: operation_class(op),
        emergency: emergency(state, principal, kind, purpose.as_deref()),
        purpose,
    };
    let default = match server_default {
        EhrAccessDefault::Open => AccessDefault::Open,
        EhrAccessDefault::Restricted => AccessDefault::Restricted,
    };
    let (in_force, policy_version) = backend.access_in_force(default);
    let record = RecordFacts {
        policy_version,
        capabilities: capabilities
            .grants()
            .iter()
            .map(|grant| (grant.operation().code(), grant.role()))
            .collect(),
    };
    Some(Arc::new(
        RequestAccess::new(caller, asking, in_force)
            .with_record_facts(record)
            .with_stale_copies(backend.stale_ehr_access_copies()),
    ))
}

/// The caller's kind (`access caller kind`).
fn kind(principal: Option<&Principal>, subject: bool, capabilities: &Capabilities) -> CallerKind {
    let Some(principal) = principal else {
        return CallerKind::Professional;
    };
    let smart = principal.access.caller();
    if subject || smart.is_some_and(ferrohealth_smart::resource::Caller::is_the_person) {
        return CallerKind::Subject;
    }
    let represents =
        smart.is_some_and(|caller| matches!(caller.kind(), SmartKind::Representative { .. }));
    if represents || principal.represented.is_some() {
        return CallerKind::Representative;
    }
    let external = smart.is_some_and(|caller| {
        matches!(
            caller.kind(),
            SmartKind::ExternalPerson(_) | SmartKind::ExternalOrganisation(_)
        )
    });
    if external || capabilities.holds(Operation::AccessService) {
        return CallerKind::AccessService;
    }
    if smart.is_some_and(|caller| matches!(caller.kind(), SmartKind::System)) {
        return CallerKind::System;
    }
    CallerKind::Professional
}

/// The EHR a confined caller of `kind` reaches: for a representative, the
/// EHR whose `EHR_STATUS.subject` names the subject its recorded
/// representation `PARTY_RELATIONSHIP` names, which authentication resolved
/// ([`Principal::represented`]); otherwise the EHR of its SMART App Launch
/// patient context (`ehrId`, ITS-REST `master07` §Context Selection), or,
/// for a contact point's caller, the EHR of its TRC patient, which stands in
/// for the launch context its token does not carry (eHDSI XCA Profile 8.0.1
/// §4.2.1).
fn own_ehr(
    principal: Option<&Principal>,
    kind: CallerKind,
    patient_grants_only: bool,
) -> Option<EhrId> {
    let principal = principal?;
    if kind == CallerKind::Representative {
        // NOTE: our own design (#5609), a representative whose subject has no EHR, or one
        // authentication did not resolve, reaches none, so the evaluator refuses it on every EHR.
        return principal
            .represented
            .as_ref()
            .and_then(|represented| represented.ehr_id.as_deref())
            .and_then(|ehr| ehr.parse().ok());
    }
    if kind != CallerKind::Subject && !patient_grants_only {
        return None;
    }
    let access = &principal.access;
    if let Some(TrcPatientEhr(ehr)) = access.extension::<TrcPatientEhr>() {
        return Some(*ehr);
    }
    // NOTE: our own design (#5586), a launch context that names no EHR id names no EHR, so
    // the evaluator refuses the confined caller on every EHR.
    access.ehr_id()?.parse().ok()
}

/// Whether the caller's grants on the EHR data `op` reaches are `patient/`
/// grants alone (SMART on openEHR `master08` §Resource Scopes): it holds one
/// for the operation's family and permission, and no other grant covers
/// every resource of them. A grant of another compartment on some templates
/// leaves the caller confined, so no template it names widens its reach.
fn patient_grants_only(principal: Option<&Principal>, op: &str) -> bool {
    let Some((principal, caller)) =
        principal.and_then(|p| p.access.caller().map(|caller| (p, caller)))
    else {
        return false;
    };
    let (family, permission) = match operation(op) {
        Some(row) if row.verb == Verb::Search => (ResourceFamily::Aql, Permission::Search),
        Some(row) => (ResourceFamily::Composition, row.permission()),
        None => return false,
    };
    let scopes = caller.openehr_scopes();
    let patient = |scope: &SmartScope| matches!(scope, SmartScope::Resource(resource) if resource.compartment == Compartment::Patient);
    let holds = scopes.iter().any(|scope| {
        matches!(scope, SmartScope::Resource(resource)
            if resource.compartment == Compartment::Patient
                && resource.resource.family() == family
                && resource.permissions.contains(permission))
    });
    let wide = openehr::covering(&scopes, family, permission, None, principal.honoured)
        .iter()
        .any(|scope| !patient(scope));
    holds && !wide
}

/// The class of `op`: a query, an export, a read or a write.
fn operation_class(op: &str) -> OperationClass {
    // NOTE: our own design, the operation table names every ITS-REST and extension
    // operation, and one it does not name reads.
    match operation(op).map(|row| row.verb) {
        _ if op.contains("export") => OperationClass::Export,
        Some(Verb::Search) => OperationClass::Query,
        Some(Verb::Read) | None => OperationClass::Read,
        Some(Verb::Create | Verb::Update | Verb::Delete | Verb::Execute) => OperationClass::Write,
    }
}

/// What the caller declared in an emergency.
fn emergency(
    state: &AppState,
    principal: Option<&Principal>,
    kind: CallerKind,
    purpose: Option<&str>,
) -> Emergency {
    let declared = state.backend().is_emergency_purpose(purpose);
    let break_glass = state
        .config()
        .rbac
        .emergency_access_role
        .as_deref()
        .is_some_and(|role| {
            principal.is_some_and(|p| p.roles.iter().any(|held| held.eq_ignore_ascii_case(role)))
        });
    // NOTE: an external person may override only where its agreement sets
    // opt_out_override, and an organisation never; our own design under EHDS Art. 10(2).
    let overridable =
        principal.is_some_and(|p| match super::authn::declared::admitted(&p.access) {
            Some(agreement) => agreement.opt_out_override && agreement.accessor == Accessor::Person,
            None => kind == CallerKind::AccessService,
        });
    // NOTE: EHDS Art. 10(2) admits "a healthcare provider or health professional", so a
    // caller acting for the person is never admitted by the override.
    let opt_out_override = overridable && !principal.is_some_and(super::delay::acts_for_the_person);
    Emergency {
        declared,
        break_glass,
        opt_out_override,
    }
}
