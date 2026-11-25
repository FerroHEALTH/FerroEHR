// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The content check of an `ACCESS_POLICY` that holds as a model: its
//! template and the codes it names against the value sets it binds.
//!
//! A policy whose `archetype_details.template_id` names a template is
//! validated against it as any archetyped data is (draft access control
//! package, Profiles). A code of a bound attribute is checked against the
//! bound value set on the terminology server; an unbound kinship takes the
//! openEHR Terminology group `subject relationship`, an unbound organisation
//! type is refused since it has no default code system, and a concealed level
//! is a level of `HL7_Confidentiality` (draft access control Terminology,
//! Value Sets). An agreement of `attested_care` that sets an `issuer` names
//! the container of an agreement `ROLE` whose partner holds its client
//! identifiers under that issuer (owner decision on #6078). A policy that breaks either is refused `422` naming the
//! attribute, as content that "was well-formed but was unable to be
//! followed due to semantic errors" (ITS-REST overview
//! `Requests_and_responses.md` §HTTP status codes). Which failure is `400`
//! and which `422` is our own design over our own model.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the template pass reads the canonical body"
)]

use serde_json::Value;

use openehr_base::validate::InvariantViolation;
use openehr_rm::draft::access_control::access_control::access_actor_scope::AccessActorScope;
use openehr_rm::draft::access_control::access_control::access_policy::AccessPolicy;
use openehr_rm::draft::access_control::terminology::{Confidentiality, ValueSetBinding};
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;

use crate::access_policy::care::CareFacts;
use crate::error::ServiceError;
use crate::service::FerroEhrService;
use crate::service::demographic::template::{TemplateFit, fit, refusal};

/// The RM class of a policy, the root class its template must have.
const RM_CLASS: &str = "ACCESS_POLICY";

/// The openEHR terminology, whose `subject relationship` group codes an
/// unbound kinship.
const OPENEHR: &str = "openehr";

/// The `subject relationship` group of the openEHR terminology.
const SUBJECT_RELATIONSHIP: &str = "subject_relationship";

/// HL7 `v3-ActReason` by `HL7_` and its code system name: the code system
/// of an unbound care purpose (draft access control Terminology, Value Sets).
const ACT_REASON: &str = "HL7_ActReason";

impl FerroEhrService {
    /// Refuse `policy`, read from `body`, when it breaks its template or
    /// names a code outside a value set it binds.
    ///
    /// # Errors
    /// - `validation_failed` (`422`) naming each attribute that breaks the
    ///   template, each duplicated binding, each code outside its value set,
    ///   and each bound code no terminology server can check (none
    ///   configured, or one that fails under `on_error = "refuse"`).
    /// - the [`Self::web_template`] refusals (`422`) for a template that is
    ///   not stored or does not build.
    pub(crate) async fn access_policy_content(
        &self,
        body: &Value,
        policy: &AccessPolicy,
    ) -> Result<(), ServiceError> {
        if let Some(template_id) = policy
            .archetype_details
            .as_ref()
            .and_then(|details| details.template_id.as_ref())
        {
            let wt = self.web_template(&template_id.value).await?;
            if let TemplateFit::Incomplete(missing) = fit(Some(RM_CLASS), body, &wt)? {
                return Err(refusal(missing));
            }
        }
        let mut wrong = duplicate_bindings(policy);
        let facts = CareFacts::of(Some(policy));
        for (index, level) in policy
            .concealed_confidentialities
            .as_deref()
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            if let Err(error) = Confidentiality::read(level) {
                wrong.push(InvariantViolation::at(
                    format!("/concealed_confidentialities/{index}"),
                    error.to_string(),
                ));
            }
        }
        for (at, rule) in policy
            .conflict_rules
            .as_deref()
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            for (index, kinship) in rule
                .kinships
                .as_deref()
                .unwrap_or_default()
                .iter()
                .enumerate()
            {
                let path = format!("/conflict_rules/{at}/kinships/{index}");
                wrong.extend(self.kinship(facts, &path, kinship).await?);
            }
            let scopes = rule.applies_to.as_deref().unwrap_or_default();
            let path = format!("/conflict_rules/{at}/applies_to");
            wrong.extend(self.organisation_types(facts, &path, scopes).await);
        }
        for (at, justification) in policy
            .justifications
            .as_deref()
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            let scopes = justification.applies_to.as_deref().unwrap_or_default();
            let path = format!("/justifications/{at}/applies_to");
            wrong.extend(self.organisation_types(facts, &path, scopes).await);
            wrong.extend(self.care_roles(facts, &path, scopes).await);
        }
        for (at, rule) in policy
            .rules
            .as_deref()
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            let scopes = rule.applies_to.as_deref().unwrap_or_default();
            let path = format!("/rules/{at}/applies_to");
            wrong.extend(self.organisation_types(facts, &path, scopes).await);
            wrong.extend(self.care_roles(facts, &path, scopes).await);
            for (index, exception) in rule
                .exceptions
                .as_deref()
                .unwrap_or_default()
                .iter()
                .enumerate()
            {
                let path = format!("/rules/{at}/exceptions/{index}/applies_to");
                let scopes = exception.applies_to.as_deref().unwrap_or_default();
                wrong.extend(self.organisation_types(facts, &path, scopes).await);
                wrong.extend(self.care_roles(facts, &path, scopes).await);
            }
        }
        wrong.extend(self.relationship_codes(facts, policy).await);
        wrong.extend(self.care_purposes(facts, policy).await);
        wrong.extend(self.attested_issuers(facts).await?);
        if wrong.is_empty() {
            Ok(())
        } else {
            Err(ServiceError::ValidationFailed(wrong))
        }
    }

    /// The refusals of the care roles and organisation types
    /// `relationship_validity` names and of the organisation types the scopes
    /// of `relationship_writers` name (#6073).
    async fn relationship_codes(
        &self,
        facts: CareFacts<'_>,
        policy: &AccessPolicy,
    ) -> Vec<InvariantViolation> {
        let mut wrong = Vec::new();
        for (at, entry) in policy
            .relationship_validity
            .as_deref()
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            for (index, role) in entry
                .roles
                .as_deref()
                .unwrap_or_default()
                .iter()
                .enumerate()
            {
                let path = format!("/relationship_validity/{at}/roles/{index}");
                wrong.extend(self.care_role(facts, &path, role).await);
            }
            for (index, kind) in entry
                .organisation_types
                .as_deref()
                .unwrap_or_default()
                .iter()
                .enumerate()
            {
                let path = format!("/relationship_validity/{at}/organisation_types/{index}");
                wrong.extend(self.organisation_type(facts, &path, kind).await);
            }
        }
        for (at, writer) in policy
            .relationship_writers
            .as_deref()
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            let path = format!("/relationship_writers/{at}/applies_to");
            wrong.extend(
                self.organisation_types(facts, &path, &writer.applies_to)
                    .await,
            );
        }
        wrong
    }

    /// The refusals of the agreements `attested_care` names with an
    /// `issuer` that is not the federation issuer of the partner whose
    /// agreement `ROLE` the `id` names; an `id` with no issuer is not read
    /// here, and names no agreement when it names no `ROLE`.
    async fn attested_issuers(
        &self,
        facts: CareFacts<'_>,
    ) -> Result<Vec<InvariantViolation>, ServiceError> {
        let mut wrong = Vec::new();
        for (index, named) in facts.attested_agreements().iter().enumerate() {
            let Some(issuer) = named.issuer.as_deref() else {
                continue;
            };
            let issuers = match uuid::Uuid::parse_str(named.id.trim()) {
                Ok(role) => {
                    crate::party_binding::grants::store::agreement_issuers(
                        &self.demographic_pool,
                        role,
                    )
                    .await?
                }
                Err(_) => Vec::new(),
            };
            if !issuers.iter().any(|held| held == issuer) {
                wrong.push(InvariantViolation::at(
                    format!("/attested_care/{index}/issuer"),
                    "the issuer is not the federation issuer of the partner whose agreement ROLE \
                     the id names"
                        .to_owned(),
                ));
            }
        }
        Ok(wrong)
    }

    /// The refusal of the kinship at `path`: outside the bound value set,
    /// or, unbound, outside the openEHR `subject relationship` group.
    async fn kinship(
        &self,
        facts: CareFacts<'_>,
        path: &str,
        kinship: &DvCodedText,
    ) -> Result<Option<InvariantViolation>, ServiceError> {
        if let Some(url) = facts.value_set(ValueSetBinding::Kinship) {
            return Ok(self.in_value_set(path, url, kinship).await);
        }
        let phrase = &kinship.defining_code;
        let held = phrase.terminology_id.value == OPENEHR
            && self
                .value_set_validate(OPENEHR, SUBJECT_RELATIONSHIP, &phrase.code_string, None)
                .await?;
        Ok((!held).then(|| {
            InvariantViolation::at(
                path,
                format!(
                    "{}::{} is no code of the openEHR subject relationship group, which codes a \
                     kinship the policy binds no value set for",
                    phrase.terminology_id.value, phrase.code_string
                ),
            )
        }))
    }

    /// The refusals of the care roles `scopes` name, at `path`: each outside
    /// the value set the policy binds for `care_role`, or, unbound, of a
    /// terminology other than `HL7_ParticipationType` (draft access control
    /// Terminology, Value Sets; #6075).
    async fn care_roles(
        &self,
        facts: CareFacts<'_>,
        path: &str,
        scopes: &[AccessActorScope],
    ) -> Vec<InvariantViolation> {
        let mut wrong = Vec::new();
        for (index, scope) in scopes.iter().enumerate() {
            for (at, role) in scope.care_roles.iter().flatten().enumerate() {
                let path = format!("{path}/{index}/care_roles/{at}");
                wrong.extend(self.care_role(facts, &path, role).await);
            }
        }
        wrong
    }

    /// The refusals of `care_purposes`: each outside the value set the policy
    /// binds for `care_purpose`, or, unbound, of a terminology other than
    /// `HL7_ActReason` (draft access control Terminology, Value Sets).
    async fn care_purposes(
        &self,
        facts: CareFacts<'_>,
        policy: &AccessPolicy,
    ) -> Vec<InvariantViolation> {
        let mut wrong = Vec::new();
        for (index, purpose) in policy.care_purposes.iter().flatten().enumerate() {
            let path = format!("/care_purposes/{index}");
            if let Some(url) = facts.value_set(ValueSetBinding::CarePurpose) {
                wrong.extend(self.in_value_set(&path, url, purpose).await);
                continue;
            }
            let phrase = &purpose.defining_code;
            if phrase.terminology_id.value != ACT_REASON {
                wrong.push(InvariantViolation::at(
                    path,
                    format!(
                        "{}::{} is no code of {ACT_REASON}, which codes a care purpose the policy \
                         binds no value set for",
                        phrase.terminology_id.value, phrase.code_string
                    ),
                ));
            }
        }
        wrong
    }

    /// The refusals of the organisation types `scopes` name, at `path`.
    async fn organisation_types(
        &self,
        facts: CareFacts<'_>,
        path: &str,
        scopes: &[AccessActorScope],
    ) -> Vec<InvariantViolation> {
        let mut wrong = Vec::new();
        for (index, scope) in scopes.iter().enumerate() {
            if let Some(kind) = &scope.organisation_type {
                let at = format!("{path}/{index}/organisation_type");
                wrong.extend(self.organisation_type(facts, &at, kind).await);
            }
        }
        wrong
    }

    /// The refusal of the organisation type `kind` at `path`: outside the
    /// value set the policy binds, or, unbound, any type at all, since the
    /// attribute has no default code system: "none: the controller binds one
    /// before a rule names a type" (draft access control Terminology, Value
    /// Sets).
    async fn organisation_type(
        &self,
        facts: CareFacts<'_>,
        path: &str,
        kind: &DvCodedText,
    ) -> Option<InvariantViolation> {
        if let Some(url) = facts.value_set(ValueSetBinding::OrganisationType) {
            return self.in_value_set(path, url, kind).await;
        }
        let phrase = &kind.defining_code;
        Some(InvariantViolation::at(
            path,
            format!(
                "{}::{} names an organisation type, and the policy binds no organisation_type \
                 value set to check it against",
                phrase.terminology_id.value, phrase.code_string
            ),
        ))
    }

    /// The refusal of `coded` at `path` when the value set at `url` does not
    /// hold it, or no terminology server can say so.
    pub(crate) async fn in_value_set(
        &self,
        path: &str,
        url: &str,
        coded: &DvCodedText,
    ) -> Option<InvariantViolation> {
        let phrase = &coded.defining_code;
        let (terminology, code) = (&phrase.terminology_id.value, &phrase.code_string);
        let Some(server) = self.terminology_server() else {
            return Some(InvariantViolation::at(
                path,
                format!(
                    "{terminology}::{code} is bound to the value set {url}, and no terminology \
                     server is configured to check it"
                ),
            ));
        };
        match server
            .value_set_validate(terminology, url, code, None)
            .await
        {
            Ok(true) => None,
            Ok(false) => Some(InvariantViolation::at(
                path,
                format!("{terminology}::{code} is not in the value set {url}"),
            )),
            Err(error) if server.fail_on_error() => Some(InvariantViolation::at(
                path,
                format!(
                    "the value set {url} could not be checked and terminology.on_error is \
                     `refuse`: {}",
                    error.message()
                ),
            )),
            Err(error) => {
                tracing::warn!(
                    value_set = %url,
                    error = %error.message(),
                    "an access policy value set could not be checked; accepting the code \
                     (terminology.on_error is `accept`)"
                );
                None
            }
        }
    }
}

/// The refusal of each binding a value set of `policy` repeats: one
/// attribute is bound to one value set.
fn duplicate_bindings(policy: &AccessPolicy) -> Vec<InvariantViolation> {
    let bindings = policy.value_sets.as_deref().unwrap_or_default();
    bindings
        .iter()
        .enumerate()
        .filter(|(index, bound)| {
            bindings
                .iter()
                .take(*index)
                .any(|before| before.binding.defining_code == bound.binding.defining_code)
        })
        .map(|(index, bound)| {
            InvariantViolation::at(
                format!("/value_sets/{index}/binding"),
                format!(
                    "{} is bound twice: one attribute is bound to one value set",
                    bound.binding.defining_code.code_string
                ),
            )
        })
        .collect()
}
