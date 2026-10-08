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
//! The gate reads `[auth.oidc.assurance]` and `[auth.oidc.professional]`. On a
//! patient-data request it refuses a token below the configured level with the
//! RFC 9470 §3 step-up challenge (`401`, `insufficient_user_authentication`),
//! and a token that names no natural person with a `403`, unless the token is a
//! client token acting for a professional the configuration names a claim for.
//! On every request it yields the [`ActorAuthentication`] the access record
//! carries. A Basic credential carries neither a level nor an issuer-asserted
//! person, so it is refused on patient data whenever either block asks for
//! one. No openEHR spec governs any of this — our own design/extension.

use std::collections::BTreeMap;

use ferroehr::config::auth::{
    AssuranceLevel, AuthConfig, ClientTokenRule, OidcConfig, ProfessionalConfig,
};
use ferroehr::system_log::event::{ActingMode, ActorAuthentication};
use http::HeaderValue;

use super::{AuthMethod, Principal};
use crate::extensions::access::authz::roles::claim_string;

/// The API families that carry no patient data, as paths relative to the API
/// base path; every other matched route is a patient-data route.
///
/// Definitions (templates, archetypes, stored-query texts), terminology, the
/// aggregate counts of `/admin/report`, and the admin surfaces that manage
/// configuration, subscriptions, the retention policy, templates and
/// stored queries hold no record of a person. The list is closed on purpose: an
/// unlisted route, including any route added later, is judged patient data.
const NO_PATIENT_DATA: &[&str] = &[
    "/definition",
    "/terminology",
    "/admin/report",
    "/admin/config",
    "/admin/event_subscription",
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
    /// The credential names no natural person and no professional the client
    /// acts for: a `403`.
    NoNaturalPerson,
}

/// The configured patient-data gate of the one OIDC issuer.
#[derive(Debug)]
pub(crate) struct PatientDataGate {
    base_path: String,
    claim: String,
    levels: BTreeMap<String, AssuranceLevel>,
    minimum: Option<AssuranceLevel>,
    /// The values meeting [`Self::minimum`], space-separated, for
    /// `acr_values`.
    accepted: String,
    professional: Option<ProfessionalConfig>,
}

impl PatientDataGate {
    /// Builds the gate from the `[auth.oidc]` table, or `None` when no bearer
    /// mechanism is configured.
    #[must_use]
    pub(crate) fn new(auth: &AuthConfig, base_path: &str) -> Option<Self> {
        let OidcConfig {
            assurance,
            professional,
            ..
        } = auth.oidc.as_ref()?;
        Some(Self {
            base_path: base_path.to_owned(),
            claim: assurance.claim.clone(),
            levels: assurance.levels.clone(),
            minimum: assurance.minimum,
            accepted: assurance.accepted_values().join(" "),
            professional: professional.clone(),
        })
    }

    /// Whether the matched route (the full axum `MatchedPath`) serves patient
    /// data; a route outside the base path is judged patient data.
    fn is_patient_data(&self, matched: Option<&str>) -> bool {
        let Some(matched) = matched else {
            return false;
        };
        matched
            .strip_prefix(self.base_path.as_str())
            .is_none_or(is_patient_data_path)
    }

    /// Reads what the credential says about the person behind it.
    #[must_use]
    pub(crate) fn actor(&self, principal: &Principal) -> ActorAuthentication {
        if principal.method == AuthMethod::Basic {
            return ActorAuthentication::default();
        }
        let claims = &principal.claims;
        let assurance_value = claim_string(claims, &self.claim);
        let assurance_level = assurance_value
            .as_deref()
            .and_then(|value| self.levels.get(value).copied());
        let (mode, professional_id) = match &self.professional {
            None => (None, None),
            Some(professional) => {
                let person = claim_string(claims, &professional.claim);
                let client = person.is_none()
                    || (professional.client_tokens == ClientTokenRule::SubIsClient
                        && names_the_client(principal));
                if client {
                    let acting_for = professional
                        .acting_for_claim
                        .as_deref()
                        .and_then(|claim| claim_string(claims, claim));
                    (Some(ActingMode::Client), acting_for)
                } else {
                    (Some(ActingMode::Person), person)
                }
            }
        };
        ActorAuthentication {
            mode,
            assurance_level,
            assurance_value,
            professional_id,
        }
    }

    /// Judges an authenticated request: the [`ActorAuthentication`] for the
    /// record, or the reason a patient-data request is refused.
    ///
    /// # Errors
    /// [`PatientDataRefusal`] on a patient-data route whose credential misses
    /// the configured assurance level or names no natural person.
    pub(crate) fn judge(
        &self,
        principal: &Principal,
        matched: Option<&str>,
    ) -> Result<ActorAuthentication, (PatientDataRefusal, ActorAuthentication)> {
        let actor = self.actor(principal);
        if !self.is_patient_data(matched) {
            return Ok(actor);
        }
        if let Some(minimum) = self.minimum
            && actor.assurance_level.is_none_or(|level| level < minimum)
        {
            return Err((PatientDataRefusal::InsufficientAssurance, actor));
        }
        if self.professional.is_some() && actor.professional_id.is_none() {
            return Err((PatientDataRefusal::NoNaturalPerson, actor));
        }
        Ok(actor)
    }

    /// The RFC 9470 §3 step-up challenge naming the accepted values.
    #[must_use]
    pub(crate) fn step_up_challenge(&self) -> HeaderValue {
        let minimum = self.minimum.map_or("", AssuranceLevel::as_str);
        // Boot validation refuses a level value that is not a quotable token,
        // so the header is always valid; the fallback keeps the error code.
        HeaderValue::from_str(&format!(
            r#"Bearer realm="ferroehr", error="insufficient_user_authentication", error_description="patient data requires authentication at assurance level {minimum} or higher", acr_values="{}""#,
            self.accepted
        ))
        .unwrap_or_else(|_| {
            HeaderValue::from_static(
                r#"Bearer realm="ferroehr", error="insufficient_user_authentication""#,
            )
        })
    }
}

/// Whether the token's `sub` names the client application: RFC 9068 §2.2 has
/// `sub` "correspond to an identifier the authorization server uses to indicate
/// the client application" when no resource owner is involved, and the client
/// is `client_id` (RFC 9068 §2.2) or `azp` (OpenID Connect Core 1.0 §2).
fn names_the_client(principal: &Principal) -> bool {
    ["client_id", "azp"].iter().any(|claim| {
        claim_string(&principal.claims, claim).is_some_and(|client| client == principal.subject)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferroehr::config::auth::AssuranceConfig;
    use serde_json::{Value, json};

    const BASE: &str = "/ferroehr/rest/openehr/v1";

    fn bearer(claims: &Value) -> Principal {
        let claims = claims.as_object().cloned().expect("object");
        Principal {
            subject: claims
                .get("sub")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            scopes: Vec::new(),
            roles: vec!["USER".to_owned()],
            claims,
            method: AuthMethod::Bearer,
        }
    }

    fn gate(
        minimum: Option<AssuranceLevel>,
        professional: Option<ProfessionalConfig>,
    ) -> PatientDataGate {
        let auth = AuthConfig {
            oidc: Some(OidcConfig {
                issuer: "https://idp.example".to_owned(),
                audiences: vec!["ferroehr".to_owned()],
                assurance: AssuranceConfig {
                    levels: [
                        ("loa-low".to_owned(), AssuranceLevel::Low),
                        ("loa-substantial".to_owned(), AssuranceLevel::Substantial),
                        ("loa-high".to_owned(), AssuranceLevel::High),
                    ]
                    .into_iter()
                    .collect(),
                    minimum,
                    ..AssuranceConfig::default()
                },
                professional,
                ..OidcConfig::default()
            }),
            ..AuthConfig::default()
        };
        PatientDataGate::new(&auth, BASE).expect("gate")
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

    /// Off by default: no minimum and no professional block refuse nothing.
    #[test]
    fn an_unconfigured_gate_refuses_nothing() {
        let gate = gate(None, None);
        let actor = gate
            .judge(&bearer(&json!({ "sub": "anyone" })), Some(EHR))
            .expect("passes");
        assert_eq!(actor, ActorAuthentication::default());
    }

    /// RFC 9470 §3: a level below the minimum, an unmapped value and a missing
    /// claim are all insufficient on patient data, and pass elsewhere.
    #[test]
    fn a_level_below_the_minimum_is_refused_on_patient_data_only() {
        let gate = gate(Some(AssuranceLevel::Substantial), None);
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
        let gate = gate(Some(AssuranceLevel::Substantial), None);
        assert_eq!(
            gate.step_up_challenge(),
            HeaderValue::from_static(
                r#"Bearer realm="ferroehr", error="insufficient_user_authentication", error_description="patient data requires authentication at assurance level substantial or higher", acr_values="loa-high loa-substantial""#
            )
        );
    }

    /// A client token (`sub` equal to `client_id`) names no natural person and
    /// is refused on patient data unless it carries the acting-for claim.
    #[test]
    fn a_client_token_needs_a_professional_it_acts_for() {
        let without = gate(None, Some(ProfessionalConfig::default()));
        let client = json!({ "sub": "app-1", "client_id": "app-1" });
        let (refusal, actor) = without
            .judge(&bearer(&client), Some(EHR))
            .expect_err("refused");
        assert_eq!(refusal, PatientDataRefusal::NoNaturalPerson);
        assert_eq!(actor.mode, Some(ActingMode::Client));

        let with = gate(
            Some(AssuranceLevel::Substantial),
            Some(ProfessionalConfig {
                acting_for_claim: Some("act.sub".to_owned()),
                ..ProfessionalConfig::default()
            }),
        );
        let acting = json!({
            "sub": "app-1", "azp": "app-1", "acr": "loa-high",
            "act": { "sub": "prof-007" }
        });
        let actor = with.judge(&bearer(&acting), Some(EHR)).expect("passes");
        assert_eq!(
            actor,
            ActorAuthentication {
                mode: Some(ActingMode::Client),
                assurance_level: Some(AssuranceLevel::High),
                assurance_value: Some("loa-high".to_owned()),
                professional_id: Some("prof-007".to_owned()),
            }
        );
    }

    /// A person token names the person through the configured claim; under
    /// `claim_absent` only the claim's absence makes a client token.
    #[test]
    fn a_person_token_names_the_person() {
        let gate = gate(
            None,
            Some(ProfessionalConfig {
                claim: "professional_id".to_owned(),
                client_tokens: ClientTokenRule::ClaimAbsent,
                acting_for_claim: None,
            }),
        );
        let actor = gate
            .judge(
                &bearer(&json!({ "sub": "u1", "client_id": "u1", "professional_id": "uzi-1" })),
                Some(EHR),
            )
            .expect("passes");
        assert_eq!(actor.mode, Some(ActingMode::Person));
        assert_eq!(actor.professional_id.as_deref(), Some("uzi-1"));
        let refusal = gate
            .judge(&bearer(&json!({ "sub": "u1" })), Some(EHR))
            .expect_err("no professional claim");
        assert_eq!(refusal.0, PatientDataRefusal::NoNaturalPerson);
    }

    /// Basic carries no level and no issuer-asserted person, so either block
    /// refuses it on patient data.
    #[test]
    fn basic_is_refused_on_patient_data_when_either_block_asks() {
        let basic = Principal {
            subject: "alice".to_owned(),
            scopes: Vec::new(),
            roles: vec!["USER".to_owned()],
            claims: serde_json::Map::new(),
            method: AuthMethod::Basic,
        };
        assert_eq!(
            gate(Some(AssuranceLevel::Low), None)
                .judge(&basic, Some(EHR))
                .expect_err("refused")
                .0,
            PatientDataRefusal::InsufficientAssurance
        );
        assert_eq!(
            gate(None, Some(ProfessionalConfig::default()))
                .judge(&basic, Some(EHR))
                .expect_err("refused")
                .0,
            PatientDataRefusal::NoNaturalPerson
        );
        gate(
            Some(AssuranceLevel::Low),
            Some(ProfessionalConfig::default()),
        )
        .judge(&basic, Some(format!("{BASE}/terminology/x").as_str()))
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
