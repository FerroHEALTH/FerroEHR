// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Who asks the access evaluator, and what: the request's caller, operation,
//! purpose and emergency, read once before dispatch.
//!
//! The caller's kind comes from what authentication admitted: the patient
//! and a person acting for them (EHDS Art. 3 and 4(2)), an access service
//! (an `access_service` grant in the CDR, or a verified caller of a
//! `federation` issuer; EHDS Art. 10), a system client, and every other
//! caller a professional. Its party, the roles of the `ROLE`s it performs and
//! its care teams are the demographic facts the request already holds
//! (`ferroehr::party_binding::grants`); a token's role claims drive no rule,
//! except that a caller admitted under a federation agreement adds the
//! professional roles and the facility types its verified credential
//! attests (`professional.hp_professional_role` and `provider.facility_type`,
//! eHDSI SAML Profile 10.0.0 §2.3), each a code of the system it names (#5691).
//! It acts for its care teams, the
//! provider its client acts for and the `GROUP`s that provider is a member
//! of (RM demographic `PARTY_RELATIONSHIP`, coded HL7 v3 `RoleClass#MBR`; #5575). The
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
use ferrohealth_smart::federation::{Attribute, Field};
use ferrohealth_smart::kind::CallerKind as SmartKind;
use ferrohealth_smart::openehr;
use ferrohealth_smart::openehr::smart_scopes::{
    Compartment, Permission, ResourceFamily, SmartScope,
};
use openehr_rm::draft::access_control::terminology::{
    AccessDefault, CallerKind, ForeignCode, OperationClass,
};
use uuid::Uuid;

use super::authn::attested::Attested;
use super::authn::trc::TrcPatientEhr;
use super::authn::{Principal, current_principal};
use super::authz::AuthzHandle;
use crate::api::operation::{Verb, operation};
use crate::state::AppState;
use ferroehr::telemetry::stage::{self, Stage};

/// The evaluator of the running request of `op`; `None` for the controller's
/// own operations (`admin_*`), which no directive binds. The read of the
/// change feed is judged: each event names EHR data, and the feed publishes
/// only what its reader may read (#5587).
pub(crate) fn request_access(state: &AppState, op: &str) -> Option<Arc<RequestAccess>> {
    let _stage = stage::enter(Stage::Access);
    if op.starts_with("admin_") && op != crate::api::admin::outbox::READ {
        return None;
    }
    let principal = current_principal();
    let principal = principal.as_deref();
    let backend = state.backend();
    let capabilities = Capabilities::current();
    let caller = caller_of(state, principal, &capabilities, Some(op));
    let kind = caller.kind;
    let purpose = ferroehr::system_log::access_context::current_purpose();
    let asking = Asking {
        operation: operation_class(op),
        emergency: emergency(state, principal, kind, purpose.as_deref()),
        purpose,
    };
    let server_default = state
        .authz()
        .as_deref()
        .map_or(EhrAccessDefault::Open, AuthzHandle::ehr_access_default);
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

/// The evaluator's caller of the running request, as the access log's
/// standing reads it: one source for who the caller is and which EHR a
/// confined caller reaches (#5614).
pub(crate) fn current_caller(state: &AppState) -> Caller {
    let principal = current_principal();
    caller_of(state, principal.as_deref(), &Capabilities::current(), None)
}

/// The evaluator's caller `principal` is, holding `capabilities`, asking
/// for `op`; with no operation, its grants are read as reaching more than
/// one patient's EHR.
fn caller_of(
    state: &AppState,
    principal: Option<&Principal>,
    capabilities: &Capabilities,
    op: Option<&str>,
) -> Caller {
    let settings = state.backend().settings_caller();
    let rbac = state
        .authz()
        .as_deref()
        .is_some_and(AuthzHandle::rbac_active);
    let kind = kind(
        principal,
        settings.subject()
            || principal.is_some_and(binds_the_person)
            || (rbac && principal.is_some_and(bound_person_alone)),
        capabilities,
    );
    let roles = principal.map_or_else(Vec::new, |p| p.roles.clone());
    let admin_role = state.authz().as_deref().map_or_else(
        || "ADMIN".to_owned(),
        |handle| handle.admin_role().to_owned(),
    );
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
    let patient_grants_only = op.is_some_and(|op| patient_grants_only(principal, op));
    Caller {
        kind,
        party: settings
            .party()
            .map(str::to_owned)
            .or_else(|| principal.and_then(own_person_id)),
        own_ehrs: own_ehrs(principal, kind, patient_grants_only),
        patient_grants_only,
        acts_for,
        administrator: roles.iter().any(|r| r.eq_ignore_ascii_case(&admin_role)),
        facts: principal.map_or_else(
            || capabilities.facts().clone(),
            |principal| {
                let (roles, types) = attested(principal);
                let facts = capabilities.facts().clone().with_attested(roles, types);
                match principal.access.extension::<Attested>() {
                    Some(Attested(attestation)) => facts.with_attestation(Arc::clone(attestation)),
                    None => facts,
                }
            },
        ),
        settings,
        lacks_consent_reference: principal.is_some_and(lacks_consent_reference),
    }
}

/// The professional roles and the organisation types the verified credential
/// of `principal` attests, when it was admitted under a federation agreement:
/// each coded value of `professional.hp_professional_role` and
/// `provider.facility_type` read as a code of the system it names. A value
/// that names no system is no code a rule can match, so it is left out.
fn attested(principal: &Principal) -> (Vec<ForeignCode>, Vec<ForeignCode>) {
    let external = super::authn::declared::admitted(&principal.access)
        .and(principal.access.caller())
        .and_then(|caller| caller.kind().external());
    let Some(external) = external else {
        return (Vec::new(), Vec::new());
    };
    let coded = |field: Field| -> Vec<ForeignCode> {
        match external.field(field) {
            Some(Attribute::Codes(codes)) => codes
                .iter()
                .filter_map(|code| {
                    // NOTE: our own design (#5691), a value with no system is legitimately no code
                    // of any terminology, so no rule matches it and a deny over it holds.
                    code.system.as_ref().map(|system| ForeignCode {
                        terminology: system.clone(),
                        code: code.code.clone(),
                    })
                })
                .collect(),
            Some(Attribute::Text(_)) | None => Vec::new(),
        }
    };
    (
        coded(Field::ProfessionalHpProfessionalRole),
        coded(Field::ProviderFacilityType),
    )
}

/// Whether `principal` was admitted under a federation agreement whose
/// `consent` condition applies and names no consent decision reference, the
/// token's mapped `consent.reference` (Wabvpz Art. 1(j) and Art. 15a(1),
/// `docs/law/nl/wabvpz/text.html`, for the condition the agreement carries).
fn lacks_consent_reference(principal: &Principal) -> bool {
    super::authn::declared::admitted(&principal.access)
        .is_some_and(|agreement| agreement.consent.is_some())
        && principal
            .access
            .caller()
            .and_then(ferrohealth_smart::resource::Caller::consent_reference)
            .is_none()
}

/// Whether `principal` is a person the party binding bound to a `PERSON`,
/// holding no role, for themself: under the role gate such a caller reaches
/// only the records whose `EHR_STATUS.subject` names that `PERSON`, as the
/// record's person (EHDS Art. 3), and the evaluator confines it to them.
///
/// No openEHR spec governs this: our own design (#6106).
pub(crate) fn bound_person_alone(principal: &Principal) -> bool {
    principal.roles.is_empty()
        && principal.represented.is_none()
        && principal.access.caller().is_some_and(|caller| {
            matches!(caller.kind(), SmartKind::Person)
                && caller
                    .party()
                    .and_then(ferrohealth_smart::party::Outcome::bound)
                    .is_some_and(|bound| bound.party.type_name() == "PERSON")
        })
}

/// Whether the login binding bound `principal` to a patient's `PERSON` as
/// the person themself ([`Principal::own`]): the evaluator confines it to the
/// records whose subject is that `PERSON`, whatever the role gate (#6106).
fn binds_the_person(principal: &Principal) -> bool {
    principal.own.is_some()
}

/// The id of the `PERSON` the login binding bound `principal` to as the
/// person themself, whose commits name no party (`PARTY_SELF`).
fn own_person_id(principal: &Principal) -> Option<String> {
    principal.own.as_ref().map(|party| party.id().to_owned())
}

/// The caller's kind (`access caller kind`).
fn kind(principal: Option<&Principal>, subject: bool, capabilities: &Capabilities) -> CallerKind {
    let Some(principal) = principal else {
        return CallerKind::Professional;
    };
    let smart = principal.access.caller();
    // NOTE: EHDS Art. 4(2), whoever acts for someone else is their representative whatever
    // issuer it logged in through, so the representation is read before the person (#5613).
    let represents =
        smart.is_some_and(|caller| matches!(caller.kind(), SmartKind::Representative { .. }));
    if represents || principal.represented.is_some() {
        return CallerKind::Representative;
    }
    // NOTE: EHDS Art. 3, the party binding is the one source of who a record's person is, so
    // a token's reading of the patient never makes its caller the subject (#6080).
    if subject {
        return CallerKind::Subject;
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

/// The EHRs a confined caller of `kind` reaches: for a representative, each
/// EHR whose `EHR_STATUS.subject` reference names the `PERSON` its recorded
/// representation `PARTY_RELATIONSHIP` names, which authentication resolved
/// ([`Principal::represented`]); otherwise the EHR of its SMART App Launch
/// patient context (`ehrId`, ITS-REST `master07` §Context Selection), or,
/// for a contact point's caller, the EHR of its TRC patient, which stands in
/// for the launch context its token does not carry (eHDSI XCA Profile 8.0.1
/// §4.2.1).
fn own_ehrs(
    principal: Option<&Principal>,
    kind: CallerKind,
    patient_grants_only: bool,
) -> Vec<EhrId> {
    let Some(principal) = principal else {
        return Vec::new();
    };
    if kind == CallerKind::Representative {
        // NOTE: our own design (#5609), a representative whose subject has no EHR, or one
        // authentication did not resolve, reaches none, so the evaluator refuses it on every EHR.
        return principal
            .represented
            .as_ref()
            .map_or_else(Vec::new, |represented| represented.ehr_ids.clone());
    }
    if kind != CallerKind::Subject && !patient_grants_only {
        return Vec::new();
    }
    let access = &principal.access;
    if let Some(TrcPatientEhr(ehr)) = access.extension::<TrcPatientEhr>() {
        return vec![*ehr];
    }
    // NOTE: our own design (#5586), a launch context that names no EHR id names no EHR, so
    // the evaluator refuses the confined caller on every EHR.
    access
        .ehr_id()
        .and_then(|ehr| ehr.parse().ok())
        .into_iter()
        .collect()
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
