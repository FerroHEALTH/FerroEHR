// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The patient-data authentication gate: the assurance level a bearer token
//! carries and the natural person it names.
//!
//! EHDS Annex II 3.1 asks an EHR system used by health professionals for
//! "reliable mechanisms for the identification and authentication of health
//! professionals", and Annex II 3.2(b) asks the access record for "the specific
//! natural person or persons having accessed" the data
//! (`docs/law/eu/ehds/text.html Annex II 3`). Implementing Regulation (EU)
//! 2026/2099 Art. 6(3) puts the cross-border floor at assurance level
//! substantial, high from 26 March 2032
//! (`docs/law/eu/cross-border-identification-2026-2099/text.html Art. 6(3)`).
//!
//! The gate reads `[authz.assurance]`. On a patient-data request it refuses a
//! token below the configured level with the RFC 9470 §3 step-up challenge
//! (`401`, `insufficient_user_authentication`). On every request it yields the
//! [`ActorAuthentication`] the access record carries: the person behind the
//! token as `ferrohealth-smart` classified the caller, a delegated token
//! recorded as the client acting for its person. A Basic credential carries
//! no level, so it is refused on patient data whenever a minimum is set. No
//! openEHR spec governs any of this — our own design/extension.

use ferroehr::system_log::event::{ActingMode, ActorAuthentication};
use ferrohealth_smart::assurance::AssuranceSection;
use ferrohealth_smart::kind::CallerKind;
use http::HeaderValue;

use super::Principal;

/// The API families that carry no patient data, as paths relative to the API
/// base path; every other matched route is a patient-data route.
///
/// Definitions (templates, archetypes, stored-query texts), terminology, the
/// aggregate counts of `/admin/report`, and the admin surfaces that manage
/// configuration, the retention policy, templates and
/// stored queries hold no record of a person. The list is closed on purpose: an
/// unlisted route, including any route added later, is judged patient data.
const NO_PATIENT_DATA: &[&str] = &[
    "/definition",
    "/terminology",
    "/admin/report",
    "/admin/config",
    "/admin/retention/policy",
    "/admin/template",
    "/admin/query",
];

/// Whether a matched route, relative to the API base path, serves patient
/// data.
///
/// The base-path root (the System API manifest) carries none; every family in
/// [`NO_PATIENT_DATA`] carries none; everything else does.
#[must_use]
pub(crate) fn is_patient_data_path(relative: &str) -> bool {
    if relative.is_empty() || relative == "/" {
        return false;
    }
    !NO_PATIENT_DATA.iter().any(|root| {
        relative
            .strip_prefix(root)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    })
}

/// Why the gate refused a patient-data request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PatientDataRefusal {
    /// The credential's assurance level is missing, unmapped or below the
    /// minimum: a `401` step-up challenge (RFC 9470 §3).
    InsufficientAssurance,
}

/// The configured patient-data gate.
#[derive(Debug)]
pub(crate) struct PatientDataGate {
    namespaces: crate::api::Namespaces,
    realm: String,
    assurance: AssuranceSection,
}

impl PatientDataGate {
    /// Builds the gate from `[authz.assurance]`, naming `realm` in its
    /// challenge, or `None` when no bearer token is admitted (`bearer` false).
    #[must_use]
    pub(crate) fn new(
        bearer: bool,
        assurance: &AssuranceSection,
        namespaces: crate::api::Namespaces,
        realm: &str,
    ) -> Option<Self> {
        bearer.then(|| Self {
            namespaces,
            realm: realm.to_owned(),
            assurance: assurance.clone(),
        })
    }

    /// Whether the matched route (the full axum `MatchedPath`) serves patient
    /// data; a route outside the API namespaces is judged patient data.
    pub(super) fn serves_patient_data(&self, matched: Option<&str>) -> bool {
        let Some(matched) = matched else {
            return false;
        };
        self.namespaces
            .relative(matched)
            .is_none_or(is_patient_data_path)
    }

    /// Reads what the credential says about the person behind it; a Basic
    /// credential says nothing.
    #[must_use]
    pub(crate) fn actor(&self, principal: &Principal) -> ActorAuthentication {
        let Some(caller) = principal.access.caller() else {
            return ActorAuthentication::default();
        };
        let assessment = self.assurance.assess(caller.claims());
        let mode = match caller.kind() {
            CallerKind::Guest(inner) => acting(inner),
            kind => acting(kind),
        };
        ActorAuthentication {
            mode,
            assurance_level: assessment.level,
            assurance_value: assessment.value,
            professional_id: matches!(mode, Some(ActingMode::Person | ActingMode::Delegated))
                .then(|| caller.person_key().map(str::to_owned))
                .flatten(),
        }
    }

    /// Judges an authenticated request: the [`ActorAuthentication`] for the
    /// record, or the reason a patient-data request is refused.
    ///
    /// # Errors
    /// [`PatientDataRefusal`] on a patient-data route whose credential misses
    /// the configured assurance level.
    pub(crate) fn judge(
        &self,
        principal: &Principal,
        matched: Option<&str>,
    ) -> Result<ActorAuthentication, (PatientDataRefusal, ActorAuthentication)> {
        let actor = self.actor(principal);
        if !self.serves_patient_data(matched) {
            return Ok(actor);
        }
        if let Some(minimum) = self.assurance.minimum
            && actor.assurance_level.is_none_or(|level| level < minimum)
        {
            return Err((PatientDataRefusal::InsufficientAssurance, actor));
        }
        Ok(actor)
    }

    /// The RFC 9470 §3 step-up challenge naming the accepted values.
    #[must_use]
    pub(crate) fn step_up_challenge(&self) -> HeaderValue {
        // Boot validation refuses a level value the challenge cannot carry, so
        // the header is always valid; the fallback keeps the error code.
        self.assurance
            .step_up()
            .ok()
            .and_then(|refusal| refusal.challenge(Some(&self.realm)))
            .and_then(|value| HeaderValue::from_str(&value).ok())
            .unwrap_or_else(|| {
                HeaderValue::from_static(r#"Bearer error="insufficient_user_authentication""#)
            })
    }
}

/// How a caller of `kind` acts for the person its token names: as that
/// person, or as a client the person delegated to; `None` for a caller that
/// names no person of this deployment.
const fn acting(kind: &CallerKind) -> Option<ActingMode> {
    match kind {
        CallerKind::Person | CallerKind::Representative { .. } => Some(ActingMode::Person),
        CallerKind::Delegated { .. } => Some(ActingMode::Delegated),
        CallerKind::System => Some(ActingMode::System),
        CallerKind::ExternalPerson(_) | CallerKind::ExternalOrganisation(_) => {
            Some(ActingMode::External)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ferrohealth_smart::assurance::AssuranceLevel;
    use ferrohealth_smart::resource::{Access, Caller, Honoured};
    use serde_json::{Value, json};

    use super::*;

    const BASE: &str = "/ferroehr/rest/openehr/v1";

    fn bearer(claims: &Value) -> Principal {
        as_kind(claims, CallerKind::Person)
    }

    /// A bearer principal of `kind` whose person key is its `sub`.
    fn as_kind(claims: &Value, kind: CallerKind) -> Principal {
        let claims = claims.as_object().cloned().expect("object");
        let subject = claims
            .get("sub")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let caller = Caller::new(subject.clone(), None, "", None, false, kind)
            .with_person_key(Some(subject), None)
            .with_claims(claims);
        Principal::new(
            Access::Caller(Arc::new(caller)),
            vec!["USER".to_owned()],
            Honoured::default(),
        )
    }

    fn gate(minimum: Option<AssuranceLevel>) -> PatientDataGate {
        let assurance = AssuranceSection {
            levels: [
                ("loa-low".to_owned(), AssuranceLevel::Low),
                ("loa-substantial".to_owned(), AssuranceLevel::Substantial),
                ("loa-high".to_owned(), AssuranceLevel::High),
            ]
            .into_iter()
            .collect(),
            minimum,
            ..AssuranceSection::default()
        };
        PatientDataGate::new(
            true,
            &assurance,
            crate::api::Namespaces::of(&ferroehr::config::rest::RestConfig::default()),
            "ferroehr",
        )
        .expect("gate")
    }

    const EHR: &str = "/ferroehr/rest/openehr/v1/ehr/{ehr_id}";

    /// The patient-data families against the closed non-patient list.
    #[test]
    fn patient_data_paths_are_everything_but_the_closed_list() {
        for patient in [
            "/ehr",
            "/ehr/{ehr_id}/composition/{uid_based_id}",
            "/query/aql",
            "/query/{qualified_query_name}",
            "/demographic/person/{uid_based_id}",
            "/admin/ehr/{ehr_id}",
            "/admin/load",
            "/message/export",
            "/fhir/r4/AuditEvent",
            "/admin/retention/due",
            "/admin/configuration-of-something-new",
        ] {
            assert!(is_patient_data_path(patient), "{patient}");
        }
        for not_patient in [
            "",
            "/",
            "/definition/template/adl1.4",
            "/definition/query/{qualified_query_name}",
            "/terminology/{terminology_id}",
            "/admin/report/contribution/count",
            "/admin/config",
            "/admin/template/{template_id}",
            "/admin/query/{qualified_query_name}/{version}",
            "/admin/retention/policy",
        ] {
            assert!(!is_patient_data_path(not_patient), "{not_patient}");
        }
    }

    /// Off by default: no minimum refuses nothing, and a system caller is
    /// recorded as one and names no person.
    #[test]
    fn an_unconfigured_gate_refuses_nothing() {
        let gate = gate(None);
        let actor = gate
            .judge(
                &as_kind(&json!({ "sub": "anyone" }), CallerKind::System),
                Some(EHR),
            )
            .expect("passes");
        assert_eq!(
            actor,
            ActorAuthentication {
                mode: Some(ActingMode::System),
                ..ActorAuthentication::default()
            }
        );
    }

    /// RFC 9470 §3: a level below the minimum, an unmapped value and a missing
    /// claim are all insufficient on patient data, and pass elsewhere.
    #[test]
    fn a_level_below_the_minimum_is_refused_on_patient_data_only() {
        let gate = gate(Some(AssuranceLevel::Substantial));
        for claims in [
            json!({ "sub": "p", "acr": "loa-low" }),
            json!({ "sub": "p", "acr": "unmapped" }),
            json!({ "sub": "p" }),
        ] {
            let refusal = gate
                .judge(&bearer(&claims), Some(EHR))
                .expect_err("refused");
            assert_eq!(refusal.0, PatientDataRefusal::InsufficientAssurance);
            gate.judge(
                &bearer(&claims),
                Some(format!("{BASE}/definition/template/adl1.4").as_str()),
            )
            .expect("a definition route is not patient data");
        }
        for acr in ["loa-substantial", "loa-high"] {
            let actor = gate
                .judge(&bearer(&json!({ "sub": "p", "acr": acr })), Some(EHR))
                .expect("at or above the minimum passes");
            assert_eq!(actor.assurance_value.as_deref(), Some(acr));
        }
    }

    /// The step-up challenge carries the RFC 9470 §3 error code and the
    /// accepted values in map order.
    #[test]
    fn the_step_up_challenge_names_the_accepted_values() {
        let gate = gate(Some(AssuranceLevel::Substantial));
        assert_eq!(
            gate.step_up_challenge(),
            HeaderValue::from_static(
                r#"Bearer realm="ferroehr", error="insufficient_user_authentication", acr_values="loa-high loa-substantial""#
            )
        );
    }

    /// A delegated token is recorded as delegated, naming its person, and a
    /// person's own token as that person, each by the issuer profile's
    /// person key.
    #[test]
    fn the_caller_kind_names_the_person() {
        let substantial = gate(Some(AssuranceLevel::Substantial));
        let delegated = as_kind(
            &json!({ "sub": "prof-007", "acr": "loa-high" }),
            CallerKind::Delegated { actor: None },
        );
        assert_eq!(
            substantial.judge(&delegated, Some(EHR)).expect("passes"),
            ActorAuthentication {
                mode: Some(ActingMode::Delegated),
                assurance_level: Some(AssuranceLevel::High),
                assurance_value: Some("loa-high".to_owned()),
                professional_id: Some("prof-007".to_owned()),
            }
        );
        let person = gate(None)
            .judge(&bearer(&json!({ "sub": "uzi-1" })), Some(EHR))
            .expect("passes");
        assert_eq!(person.mode, Some(ActingMode::Person));
        assert_eq!(person.professional_id.as_deref(), Some("uzi-1"));
    }

    /// Basic carries no level, so a minimum refuses it on patient data only.
    #[test]
    fn basic_is_refused_on_patient_data_under_a_minimum() {
        let basic = Principal::new(
            Access::Caller(Arc::new(Caller::basic("alice".to_owned(), ""))),
            vec!["USER".to_owned()],
            Honoured::default(),
        );
        let gate = gate(Some(AssuranceLevel::Low));
        assert_eq!(
            gate.judge(&basic, Some(EHR)).expect_err("refused").0,
            PatientDataRefusal::InsufficientAssurance
        );
        gate.judge(&basic, Some("/ferroehr/rest/extensions/v1/terminology/x"))
            .expect("not patient data");
    }

    /// Every generated route agrees with the audit classification: patient
    /// data is exactly the routes audited against a person's record, plus
    /// query execution.
    #[test]
    fn the_path_rule_agrees_with_the_audit_classification() {
        use ferroehr::system_log::event::{EventActionCode, ObjectClass};
        for table in [
            openehr_its::rest::generated::ehr::ROUTES,
            openehr_its::rest::generated::definition::ROUTES,
            openehr_its::rest::generated::demographic::ROUTES,
            openehr_its::rest::generated::query::ROUTES,
            openehr_its::rest::generated::admin::ROUTES,
            openehr_its::rest::generated::system::ROUTES,
        ] {
            for (_method, path, op) in table {
                let (action, class) = crate::system_log::classify::audit_for(op).expect("audited");
                let expected = match class {
                    ObjectClass::Ehr
                    | ObjectClass::Composition
                    | ObjectClass::Contribution
                    | ObjectClass::Directory
                    | ObjectClass::Demographic
                    | ObjectClass::Extract => true,
                    ObjectClass::Query => action == EventActionCode::Execute,
                    ObjectClass::Template
                    | ObjectClass::ApplicationActivity
                    | ObjectClass::Authentication => false,
                };
                let normalized = crate::api::normalize_path(path);
                assert_eq!(
                    is_patient_data_path(&normalized),
                    expected,
                    "{op} at {normalized}"
                );
            }
        }
    }
}
