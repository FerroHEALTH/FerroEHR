// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ABAC request this CDR puts to the policy decision point of
//! `ferrohealth-authz`.
//!
//! An operation maps onto a [`ResourceKind`] and an [`AccessMode`], and the PEP
//! resolves the attributes a policy reads: the caller's organization and
//! patient claims, and the patient and template of every object the operation
//! touches. [`model`] declares the same names to the Cedar engine, so the
//! schema a deployment's policies are validated against is the request this
//! module builds. The engine decides every combination of a set attribute and
//! permits only when each is permitted. No openEHR specification governs
//! authorization: our own design.

use ferrohealth_authz::cedar::Model;
use ferrohealth_authz::request::{Attr, Request, RequestError, Subject};

/// The resource family a clinical operation acts on. Derived from the
/// operation-id prefix by [`crate::extensions::access::authz::classify::kind_of`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResourceKind {
    /// An EHR (create / get / get-by-subject).
    Ehr,
    /// An `EHR_STATUS` or a versioned-EHR_STATUS read.
    EhrStatus,
    /// A COMPOSITION or a versioned-composition read.
    Composition,
    /// A CONTRIBUTION.
    Contribution,
    /// An AQL query execution (ad-hoc or stored).
    Query,
    /// A DIRECTORY/FOLDER. Gated only when `abac.check_directory` is set, so a
    /// deployment opts into it explicitly.
    Directory,
}

/// Every resource kind, in declaration order.
pub const KINDS: [ResourceKind; 6] = [
    ResourceKind::Ehr,
    ResourceKind::EhrStatus,
    ResourceKind::Composition,
    ResourceKind::Contribution,
    ResourceKind::Query,
    ResourceKind::Directory,
];

impl ResourceKind {
    /// The canonical key this kind uses in the `abac.policy` config map and in
    /// the Cedar action names.
    #[must_use]
    pub const fn config_key(self) -> &'static str {
        match self {
            ResourceKind::Ehr => "ehr",
            ResourceKind::EhrStatus => "ehr_status",
            ResourceKind::Composition => "composition",
            ResourceKind::Contribution => "contribution",
            ResourceKind::Query => "query",
            ResourceKind::Directory => "directory",
        }
    }

    /// The resource type the policy request names, the Cedar entity type.
    #[must_use]
    pub const fn resource_type(self) -> &'static str {
        match self {
            ResourceKind::Ehr => "Ehr",
            ResourceKind::EhrStatus => "EhrStatus",
            ResourceKind::Composition => "Composition",
            ResourceKind::Contribution => "Contribution",
            ResourceKind::Query => "Query",
            ResourceKind::Directory => "Directory",
        }
    }

    /// The kind whose [`Self::config_key`] is `key`.
    #[must_use]
    pub fn from_config_key(key: &str) -> Option<Self> {
        KINDS.into_iter().find(|kind| kind.config_key() == key)
    }
}

/// The access mode of an operation (the Cedar *action* axis): op ids map
/// onto `ResourceKind × AccessMode` rather than 96 raw op ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessMode {
    /// A create/commit.
    Create,
    /// A read/get.
    Read,
    /// An update/modify.
    Update,
    /// A delete.
    Delete,
    /// A query execution.
    Execute,
}

/// Every access mode, in declaration order.
pub const MODES: [AccessMode; 5] = [
    AccessMode::Create,
    AccessMode::Read,
    AccessMode::Update,
    AccessMode::Delete,
    AccessMode::Execute,
];

impl AccessMode {
    /// The lower-case mode name used to compose the Cedar action id
    /// (`composition.create`, `query.execute`, …).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            AccessMode::Create => "create",
            AccessMode::Read => "read",
            AccessMode::Update => "update",
            AccessMode::Delete => "delete",
            AccessMode::Execute => "execute",
        }
    }
}

/// The action id of a (kind, mode) pair, such as `composition.create`.
#[must_use]
pub fn action_id(kind: ResourceKind, mode: AccessMode) -> String {
    format!("{}.{}", kind.config_key(), mode.as_str())
}

/// The subject attribute carrying the caller's organization claim.
pub const ORGANIZATION: &str = "organization";
/// The subject and resource attribute naming a patient.
pub const PATIENT: &str = "patient";
/// The resource attribute naming a template.
pub const TEMPLATE: &str = "template";
/// The context attribute naming the generated operation.
pub const OPERATION_ID: &str = "operation_id";

/// The names the Cedar schema declares.
///
/// The principal's organization and patient, one resource type per
/// [`ResourceKind`] with its patient and template, one action per kind and
/// mode, and the operation id as context.
#[must_use]
pub fn model() -> Model {
    let mut model = Model::new()
        .subject_attribute(ORGANIZATION)
        .subject_attribute(PATIENT)
        .context(OPERATION_ID);
    for kind in KINDS {
        model = model.resource(kind.resource_type(), [PATIENT, TEMPLATE]);
        for mode in MODES {
            model = model.action(action_id(kind, mode), [kind.resource_type()]);
        }
    }
    model
}

/// The attributes the PEP resolved for one operation.
#[derive(Debug, Clone)]
pub struct AuthzRequest<'a> {
    /// The generated operation id, the `operation_id` context attribute.
    pub operation_id: &'a str,
    /// The resource family.
    pub kind: ResourceKind,
    /// The access mode (the action axis).
    pub access: AccessMode,
    /// The authenticated caller's subject identifier (`sub` / Basic username),
    /// so a policy can name who is asking (NIST SP 800-162 §2.2).
    pub subject: &'a str,
    /// The caller's roles, upper-cased as the RBAC gate sees them.
    pub roles: &'a [String],
    /// The caller's OAuth2 scopes, verbatim.
    pub scopes: &'a [String],
    /// The caller's organization (resolved `abac.organization_claim`), if any.
    pub organization: Option<String>,
    /// The caller's patient (resolved `abac.patient_claim`), the principal's
    /// `patient`; the PEP leaves it unset for the remote decision point, which
    /// reads a parameter the resource lacks from the subject.
    pub subject_patient: Option<String>,
    /// The patient of the touched objects (one, or a set for a query).
    pub patient: Option<Attr>,
    /// The template of the touched objects (one, or a set for a contribution
    /// or a query).
    pub template: Option<Attr>,
}

impl AuthzRequest<'_> {
    /// The subject the policy request names.
    #[must_use]
    pub fn subject(&self) -> Subject {
        let mut subject = Subject::new(self.subject)
            .with_roles(self.roles.to_vec())
            .with_scopes(self.scopes.to_vec());
        if let Some(organization) = &self.organization {
            subject = subject.with_attribute(ORGANIZATION, organization.clone());
        }
        if let Some(patient) = &self.subject_patient {
            subject = subject.with_attribute(PATIENT, patient.clone());
        }
        subject
    }

    /// The policy request of `subject`, which [`Self::subject`] returns.
    ///
    /// # Errors
    ///
    /// [`RequestError::EmptySet`] for a patient or template set with no value.
    pub fn to_request<'s>(&self, subject: &'s Subject) -> Result<Request<'s>, RequestError> {
        let mut request = Request::new(
            subject,
            action_id(self.kind, self.access),
            self.kind.resource_type(),
        )
        .with_context(OPERATION_ID, self.operation_id);
        if let Some(patient) = &self.patient {
            request = request.with_attribute(PATIENT, patient.clone())?;
        }
        if let Some(template) = &self.template {
            request = request.with_attribute(TEMPLATE, template.clone())?;
        }
        Ok(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req<'a>(patient: Option<Attr>, template: Option<Attr>) -> AuthzRequest<'a> {
        AuthzRequest {
            operation_id: "composition_create",
            kind: ResourceKind::Composition,
            access: AccessMode::Create,
            subject: "test-subject",
            roles: &[],
            scopes: &[],
            organization: Some("org1".to_owned()),
            subject_patient: None,
            patient,
            template,
        }
    }

    #[test]
    fn the_model_declares_every_kind_and_mode() {
        assert_eq!(model().validate(), Ok(()));
        let schema = model().schema_source();
        assert!(schema.contains("action \"composition.create\""), "{schema}");
        assert!(schema.contains("entity EhrStatus"), "{schema}");
        assert!(
            schema.contains("context: { operation_id: String }"),
            "{schema}"
        );
    }

    #[test]
    fn the_request_carries_the_kind_mode_and_attributes() {
        let r = req(
            Some(Attr::Set(vec!["p1".to_owned(), "p2".to_owned()])),
            Some(Attr::One("t1".to_owned())),
        );
        let subject = r.subject();
        let request = r.to_request(&subject).expect("a request");
        assert_eq!(request.action(), "composition.create");
        assert_eq!(request.resource_type(), "Composition");
        assert_eq!(request.combinations().len(), 2);
        assert_eq!(
            subject.attributes().get(ORGANIZATION).map(String::as_str),
            Some("org1")
        );
        assert!(!subject.attributes().contains_key(PATIENT));
    }

    #[test]
    fn an_empty_set_is_refused_before_any_engine_runs() {
        let r = req(Some(Attr::Set(vec![])), None);
        let subject = r.subject();
        assert!(matches!(
            r.to_request(&subject),
            Err(RequestError::EmptySet { .. })
        ));
    }

    #[test]
    fn a_config_key_names_its_kind() {
        for kind in KINDS {
            assert_eq!(ResourceKind::from_config_key(kind.config_key()), Some(kind));
        }
        assert_eq!(ResourceKind::from_config_key("folder"), None);
    }
}
