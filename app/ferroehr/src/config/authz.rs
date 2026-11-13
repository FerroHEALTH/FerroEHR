// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Authorization configuration ([`AuthzConfig`]).
//!
//! The `[authz]` section of the one server configuration tree, with no loader of
//! its own. No openEHR spec governs RBAC or ABAC, so this is our own design.
//!
//! Two sections: the coarse RBAC keys (`rbac.*`, always evaluated when auth is
//! enabled) and the opt-in ABAC keys (`abac.*`, master switch `abac.enabled`,
//! default `false`). Every field has a default, so an all-defaults
//! [`AuthzConfig`] is valid (RBAC on, `ADMIN`/`USER` roles, ABAC off).
//!
//! NOTE: the management surface is NOT governed here — `[management]`
//! (`[management.endpoints]`, one level per endpoint) is its single authority,
//! enforced by the per-route guard in `ferroehr-rest`.

use std::collections::BTreeMap;

use ferrohealth_secret::secret_url::SecretUrl;
use serde::{Deserialize, Serialize};

use crate::config::abac_rules::RemoteConfig;
use crate::party_binding::grants::operation::GrantCeiling;

/// The server-wide disposition for an EHR that carries no
/// `ACCESS_CONTROL_SETTINGS` of its own — the `ehr_access_default` tri-state's
/// two states.
///
/// NOTE: no openEHR spec governs this — our own design/extension; ITS-REST
/// mandates no authorization scheme, and RM `master07` only defines the
/// per-EHR object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum EhrAccessDefault {
    /// Any caller the coarse layers already admitted reaches the EHR. The
    /// default, because it is what every existing deployment runs.
    #[default]
    Open,
    /// A setting-less EHR is reachable only by a caller holding
    /// `rbac.admin_role`.
    ///
    /// The admin carve-out is deliberate: a plain deny would make an EHR that
    /// carries no settings unreachable by anyone, including the operator who
    /// would author the settings that fix it — a default-deny posture nobody
    /// can climb out of is an outage, not a control.
    Restricted,
}

/// The coarse role-based access-control settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "trust")))]
pub struct RbacConfig {
    /// Master switch of the coarse role gate, active only when auth is enabled
    /// (`FERROEHR__AUTHZ__RBAC__ENABLED`, default `true`).
    ///
    /// Disabling it lets every authenticated caller use every enabled surface,
    /// which the deployment posture reports as `rbac_off`. The per-EHR keys
    /// [`Self::ehr_access_default`] and [`Self::admin_role`] apply either way.
    pub enabled: bool,
    /// The role required for Admin-class operations (default `ADMIN`).
    pub admin_role: String,
    /// The role marking a principal as read-only (default `READONLY`): a caller
    /// carrying it is refused on every write operation even when it also holds
    /// granting roles (a restriction overrides a grant). Supports the
    /// conformance catalogue's authorization-separation cases.
    pub readonly_role: String,
    /// The role that may read the access log for ONE subject at a time
    /// (`FERROEHR__AUTHZ__RBAC__SUBJECT_AUDIT_ROLE`, default unset).
    ///
    /// A portal that shows a person who accessed their record (GDPR Art. 15
    /// with Recital 63, `docs/law/eu/gdpr/oj.html`, EHDS Art. 9, for Dutch deployments Wabvpz Art. 15e)
    /// needs the ITI-81 retrieval for that person and nobody else. A caller
    /// holding this role may call `GET /fhir/r4/AuditEvent` only with the
    /// `patient` parameter, the subject's opaque pseudonym; without it the call
    /// is refused, and the unscoped retrieval stays with `admin_role`. Unset,
    /// the log is admin-only, as before (#3240). No openEHR spec governs the
    /// access log — our own design/extension.
    pub subject_audit_role: Option<String>,
    /// The role that may reach data under an EHDS Art. 8 access restriction in
    /// an emergency (`FERROEHR__AUTHZ__RBAC__EMERGENCY_ACCESS_ROLE`, default
    /// unset).
    ///
    /// EHDS Art. 11(5) lets access to restricted data be granted where
    /// necessary to protect the vital interests of the person
    /// (`docs/law/eu/ehds/text.html`). A caller holding this role that
    /// declares one of `[audit] emergency_purpose_codes` reads the restricted
    /// data, and its access record carries the emergency mark. Unset, nobody
    /// may. No openEHR spec governs restriction of access — our own
    /// design/extension.
    pub emergency_access_role: Option<String>,
    /// The roles of an EHDS Art. 10 access-service caller, held to an opt-out
    /// (`FERROEHR__AUTHZ__RBAC__ACCESS_SERVICE_ROLES`; empty, the default, holds nobody).
    pub access_service_roles: Vec<String>,
    /// The role that may read a party's protected national identifiers
    /// (`FERROEHR__AUTHZ__RBAC__IDENTIFIER_READER_ROLE`, default unset).
    ///
    /// `GET /admin/party/identifiers` expands the identifiers
    /// `[privacy.identifier_protection]` seals, and every read is an
    /// access record. The route sits in the ADMIN API and also needs this
    /// role, so holding `admin_role` alone does not reveal a national
    /// identifier. Unset, nobody may call it (#3776). No openEHR spec governs
    /// this — our own design/extension.
    pub identifier_reader_role: Option<String>,
    /// The role that may import an EHR or an EHR extract
    /// (`message_import_ehr`, `message_import_ehr_extract`; default unset).
    // TODO(#5005): require the role on the two import operations.
    pub import_role: Option<String>,
    /// The issuers whose role claims may grant `admin_role`,
    /// `emergency_access_role`, `import_role` and `identifier_reader_role`,
    /// each a `persons` issuer; empty, the default, lets no token grant them.
    /// A policy grant of `import_ehr`, `import_ehr_extract` or
    /// `read_identifiers` is a second source. Each renders with any userinfo
    /// masked.
    pub role_issuers: Vec<SecretUrl>,
    /// What an EHR carrying no `ACCESS_CONTROL_SETTINGS` admits (default
    /// `open`).
    ///
    /// A newly created EHR carries none, so this is the disposition that
    /// actually governs most records. `restricted` lets a deployment choose
    /// object-level default-deny ONCE, instead of authoring a settings object
    /// per EHR — which was the only way to reach that posture, and is the
    /// asymmetry this key removes (OWASP Insecure Direct Object Reference
    /// Prevention Cheat Sheet: an unpredictable id is not itself an access
    /// control).
    ///
    /// It applies whether or not [`Self::enabled`] is set, because the per-EHR
    /// gate runs without the coarse one; with authentication off nothing could
    /// enforce `restricted`, so that combination is refused at boot.
    pub ehr_access_default: EhrAccessDefault,
}

impl Default for RbacConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            admin_role: "ADMIN".to_owned(),
            readonly_role: "READONLY".to_owned(),
            subject_audit_role: None,
            emergency_access_role: None,
            access_service_roles: Vec::new(),
            identifier_reader_role: None,
            import_role: None,
            role_issuers: Vec::new(),
            ehr_access_default: EhrAccessDefault::Open,
        }
    }
}

impl RbacConfig {
    /// The privileged roles, upper-cased: `admin_role`,
    /// `emergency_access_role`, `import_role` and `identifier_reader_role`,
    /// which a `hold_roles` grant never gives.
    #[must_use]
    pub fn privileged_roles(&self) -> Vec<String> {
        [
            Some(&self.admin_role),
            self.emergency_access_role.as_ref(),
            self.import_role.as_ref(),
            self.identifier_reader_role.as_ref(),
        ]
        .into_iter()
        .flatten()
        .map(|role| role.trim().to_uppercase())
        .collect()
    }
}

/// The ABAC policy engine selector (`abac.engine`). `cedar` is the
/// embedded default; `remote` delegates to an external PDP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum AbacEngineKind {
    /// Embedded Cedar (`cedar-policy`), its policy set from the Cedar admin
    /// register.
    #[default]
    Cedar,
    /// The remote policy-decision-point (`abac.remote.url`).
    Remote,
}

/// One flat parameter a remote PDP body may carry: the resolved
/// attribute keys, exactly `organization` / `patient` / `template` /
/// `category` / `origin` on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum AbacParam {
    /// The caller's organization (from `abac.organization_claim`).
    Organization,
    /// The patient the resource is about (from `abac.patient_claim` / EHR subject).
    Patient,
    /// The resource's template id.
    Template,
    /// The EHDS categories of the resource's data (EHDS Annex II 3.4).
    Category,
    /// The origins of the resource's data (EHDS Annex II 3.4, a `data_origin` grant).
    Origin,
}

impl AbacParam {
    /// The key this attribute takes in the flat PDP request body — the wire name
    /// a policy author writes their rules against, so it is part of the
    /// deployment's contract with its PDP.
    #[must_use]
    pub const fn wire_key(self) -> &'static str {
        match self {
            AbacParam::Organization => "organization",
            AbacParam::Patient => "patient",
            AbacParam::Template => "template",
            AbacParam::Category => "category",
            AbacParam::Origin => "origin",
        }
    }
}

/// A per-resource-kind policy binding: the policy name appended to the
/// remote PDP base URL, and which resolved parameters its request body carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "infrastructure")))]
pub struct PolicyRule {
    /// The policy name (the path segment appended to `abac.remote.url`).
    pub name: String,
    /// The parameters the PDP body carries; empty = an unparameterized policy.
    #[serde(default)]
    pub parameters: Vec<AbacParam>,
}

/// The fine-grained attribute-based access-control settings.
///
/// Master switch `enabled` defaults to `false`, so an all-defaults config
/// preserves today's authentication-plus-RBAC behaviour until an operator
/// opts in. The table keeps `enabled` where other optional features use the
/// table's presence, because `organization_claim` applies while it is off:
/// the access log records the organisation a caller acts for.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "infrastructure")))]
pub struct AbacConfig {
    /// Master ABAC switch (default `false`).
    pub enabled: bool,
    /// Which PDP engine to use (default `cedar`).
    pub engine: AbacEngineKind,
    /// The JWT claim carrying the caller's organization (default
    /// `organization_id`); resolved opportunistically (absent → no attribute).
    pub organization_claim: String,
    /// The JWT claim carrying the patient id (default `patient_id`); its presence
    /// enables the local subject gate. Set to empty to disable the gate.
    pub patient_claim: String,
    /// The subject namespace patient-claim ids are issued in
    /// (`EHR_STATUS.subject.external_ref.namespace`, RM ehr master04 §EHR
    /// Status), default unset.
    ///
    /// Set, a claim names the EHR subject whose (id, namespace) pair equals
    /// (claim, this value). Unset, a claim names the subject holding that id
    /// only when no other namespace holds the same id, and matches nothing
    /// otherwise. No openEHR spec governs the claim — our own extension.
    pub patient_namespace: Option<String>,
    /// Remote-PDP settings.
    pub remote: RemoteConfig,
    /// Whether DIRECTORY (FOLDER) operations are submitted to the PDP
    /// (default `false`).
    ///
    /// An engine-independent switch: the alternative — inferring the opt-in from
    /// a `directory` entry in [`Self::policy`] — is a remote-PDP-shaped map, so a
    /// Cedar deployment could only enable the check by inventing a policy name
    /// its engine never reads. No openEHR spec governs authorization — our own
    /// design/extension.
    pub check_directory: bool,
    /// Per-resource-kind policy bindings, keyed by the canonical kind name
    /// (`ehr`, `ehr_status`, `composition`, `contribution`, `query`,
    /// `directory`).
    ///
    /// With `engine = remote` every kind the enforcement point consults needs an
    /// entry ([`REQUIRED_REMOTE_POLICY_KINDS`](crate::config::abac_rules::REQUIRED_REMOTE_POLICY_KINDS)), checked at boot: a runtime
    /// "kind not configured" branch can only fail closed, and a deny on
    /// live traffic is a worse answer than a refusal to start.
    pub policy: BTreeMap<String, PolicyRule>,
}

impl Default for AbacConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            engine: AbacEngineKind::Cedar,
            organization_claim: "organization_id".to_owned(),
            patient_claim: "patient_id".to_owned(),
            patient_namespace: None,
            remote: RemoteConfig::default(),
            check_directory: false,
            policy: BTreeMap::new(),
        }
    }
}

impl AbacConfig {
    /// The patient claim, or `None` when blank (the subject gate is then off).
    #[must_use]
    pub fn patient_claim(&self) -> Option<&str> {
        let c = self.patient_claim.trim();
        (!c.is_empty()).then_some(c)
    }

    /// The patient namespace, or `None` when unset or blank.
    #[must_use]
    pub fn patient_namespace(&self) -> Option<&str> {
        self.patient_namespace
            .as_deref()
            .map(str::trim)
            .filter(|ns| !ns.is_empty())
    }

    /// The organization claim, or `None` when blank.
    #[must_use]
    pub fn organization_claim(&self) -> Option<&str> {
        let c = self.organization_claim.trim();
        (!c.is_empty()).then_some(c)
    }
}

/// The ceiling on what the policy grants may grant (`[authz.grants]`).
///
/// A grant is an operation of FerroEHR's operation terminology
/// (`ferroehr-operations`) coded in the credentials of a `CAPABILITY`, which
/// an administrator manages through the Demographic API. This key is the
/// trust bound the party store cannot raise: a commit of a capability coding
/// an operation it does not list is refused, and one already stored is never
/// honoured, so a compromised administrator account grants no more than the
/// operator allows. No openEHR spec gives a capability an authorization
/// meaning: our own design.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "trust")))]
#[serde(default, deny_unknown_fields)]
pub struct GrantsConfig {
    /// The operations the party store may grant
    /// (`FERROEHR__AUTHZ__GRANTS__ALLOWED`, comma-separated).
    ///
    /// The operations are `commit_composition`, `import_ehr`,
    /// `import_ehr_extract`, `export_ehr_extract`, `federated_person_access`,
    /// `federated_organisation_access`, `read_subject_audit`,
    /// `access_service`, `read_identifiers`, `hold_roles`, `hold_scopes` and
    /// `create_at_login`. The default allows only `commit_composition`, which
    /// lets a client write the templates its grant names and nothing else: an
    /// import, an export, a federation agreement, every privilege and the
    /// creation of a `PERSON` at a first login reach beyond one system's own
    /// writes, so each is allowed by name. An empty list lets the store grant
    /// nothing.
    pub allowed: Vec<crate::party_binding::grants::operation::Operation>,
}

impl Default for GrantsConfig {
    fn default() -> Self {
        Self {
            allowed: vec![crate::party_binding::grants::operation::Operation::CommitComposition],
        }
    }
}

impl GrantsConfig {
    /// The ceiling these operations set, reserving the privileged roles of
    /// `rbac` ([`RbacConfig::privileged_roles`]).
    #[must_use]
    pub fn ceiling(&self, rbac: &RbacConfig) -> GrantCeiling {
        GrantCeiling::new(self.allowed.iter().copied()).with_reserved_roles(rbac.privileged_roles())
    }
}

/// Authorization configuration: the coarse RBAC gate plus the opt-in ABAC
/// policy layer.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct AuthzConfig {
    /// Role-based access control.
    pub rbac: RbacConfig,
    /// Attribute-based access control (opt-in).
    pub abac: AbacConfig,
    /// The authentication assurance a patient-data request must carry
    /// (`[authz.assurance]`, off by default).
    #[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "trust")))]
    pub assurance: ferrohealth_smart::assurance::AssuranceSection,
    /// The party binding: the provider, the namespace and the Basic users'
    /// keys (`[authz.party_binding]`).
    pub party_binding: crate::config::binding::PartyBindingConfig,
    /// The system clients that commit (`[[authz.system_client]]`).
    pub system_client: Vec<crate::config::clients::SystemClient>,
    /// The federation agreements (`[authz.federation]`).
    pub federation: crate::config::agreement::FederationConfig,
    /// The operations the policy grants may grant (`[authz.grants]`).
    pub grants: GrantsConfig,
}

/// A boot-time authorization-configuration error — a hard error that must
/// abort startup rather than silently mis-gate.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AuthzConfigError {
    /// A required role name was blank.
    #[error("authz.rbac.{0} must not be empty")]
    EmptyRole(&'static str),
    /// A policy key does not name a known resource kind.
    #[error(
        "authz.abac.policy.{0} is not a known resource kind \
         (ehr, ehr_status, composition, contribution, query, directory)"
    )]
    UnknownPolicyKind(String),
    /// A `template` parameter was configured for `ehr`/`ehr_status`, whose
    /// resources carry no template.
    #[error("authz.abac.policy.{0} must not take the `template` parameter")]
    TemplateParamIllegal(&'static str),
    /// `engine = remote` without a configured base URL.
    #[error("authz.abac.remote.url is required when abac.engine = remote")]
    RemoteServerMissing,
    /// The remote base URL does not end with `/` (the policy name is appended);
    /// carries the URL with its `userinfo` masked.
    #[error("authz.abac.remote.url must end with '/' (got {0:?})")]
    RemoteServerTrailingSlash(String),
    /// `engine = remote` without a policy for a kind the enforcement point
    /// consults — the runtime branch can only fail closed, so it is refused here.
    #[error(
        "authz.abac.policy.{0} is required when abac.engine = remote: every resource kind the \
         enforcement point consults must name a policy, or its requests can only be denied"
    )]
    RemotePolicyMissing(&'static str),
}

impl AuthzConfig {
    /// Validate the configuration at boot. All defaults are valid.
    ///
    /// # Errors
    /// [`AuthzConfigError`] when a role name is blank or an ABAC setting does
    /// not hold.
    pub fn validate(&self) -> Result<(), AuthzConfigError> {
        if self.rbac.admin_role.trim().is_empty() {
            return Err(AuthzConfigError::EmptyRole("admin_role"));
        }
        if self.rbac.readonly_role.trim().is_empty() {
            return Err(AuthzConfigError::EmptyRole("readonly_role"));
        }
        if self.abac.enabled {
            self.abac.validate()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::config::abac_rules::REQUIRED_REMOTE_POLICY_KINDS;

    #[test]
    fn defaults_are_valid() {
        let c = AuthzConfig::default();
        assert!(c.rbac.enabled);
        assert_eq!(c.rbac.admin_role, "ADMIN");
        assert_eq!(c.rbac.readonly_role, "READONLY");
        assert!(c.validate().is_ok());
    }

    /// `user_role`, retired in v5.6 and past its one release, is an unknown
    /// key.
    #[test]
    fn user_role_is_an_unknown_key() {
        let err = toml::from_str::<AuthzConfig>("[rbac]\nuser_role = \"USER\"\n")
            .expect_err("an unknown key");
        assert!(err.to_string().contains("user_role"), "{err}");
    }

    #[test]
    fn blank_admin_role_rejected() {
        let c = AuthzConfig {
            rbac: RbacConfig {
                admin_role: "  ".to_owned(),
                ..RbacConfig::default()
            },
            ..AuthzConfig::default()
        };
        assert_eq!(c.validate(), Err(AuthzConfigError::EmptyRole("admin_role")));
    }

    #[test]
    fn blank_readonly_role_rejected() {
        let c = AuthzConfig {
            rbac: RbacConfig {
                readonly_role: "  ".to_owned(),
                ..RbacConfig::default()
            },
            ..AuthzConfig::default()
        };
        assert_eq!(
            c.validate(),
            Err(AuthzConfigError::EmptyRole("readonly_role"))
        );
    }

    #[test]
    fn abac_disabled_by_default_and_skips_validation() {
        let c = AuthzConfig::default();
        assert!(!c.abac.enabled);
        assert_eq!(c.abac.engine, AbacEngineKind::Cedar);
        // A misconfigured-but-disabled ABAC section is not validated.
        let c = AuthzConfig {
            abac: AbacConfig {
                enabled: false,
                engine: AbacEngineKind::Remote,
                remote: RemoteConfig {
                    ..RemoteConfig::default()
                },
                ..AbacConfig::default()
            },
            ..AuthzConfig::default()
        };
        assert!(c.validate().is_ok());
    }

    #[test]
    fn template_param_illegal_on_ehr_and_ehr_status() {
        for kind in ["ehr", "ehr_status"] {
            let mut policy = BTreeMap::new();
            policy.insert(
                kind.to_owned(),
                PolicyRule {
                    name: "p".to_owned(),
                    parameters: vec![AbacParam::Template],
                },
            );
            let c = AuthzConfig {
                abac: AbacConfig {
                    enabled: true,
                    engine: AbacEngineKind::Cedar,
                    policy,
                    ..AbacConfig::default()
                },
                ..AuthzConfig::default()
            };
            assert!(matches!(
                c.validate(),
                Err(AuthzConfigError::TemplateParamIllegal(_))
            ));
        }
    }

    #[test]
    fn unknown_policy_kind_rejected() {
        let mut policy = BTreeMap::new();
        policy.insert(
            "nonsense".to_owned(),
            PolicyRule {
                name: "p".to_owned(),
                parameters: vec![],
            },
        );
        let c = AuthzConfig {
            abac: AbacConfig {
                enabled: true,
                policy,
                ..AbacConfig::default()
            },
            ..AuthzConfig::default()
        };
        assert_eq!(
            c.validate(),
            Err(AuthzConfigError::UnknownPolicyKind("nonsense".to_owned()))
        );
    }

    /// A `policy` entry for every kind [`REQUIRED_REMOTE_POLICY_KINDS`] names.
    fn full_remote_policy_map() -> BTreeMap<String, PolicyRule> {
        REQUIRED_REMOTE_POLICY_KINDS
            .iter()
            .map(|kind| {
                (
                    (*kind).to_owned(),
                    PolicyRule {
                        name: format!("{kind}-access"),
                        parameters: vec![],
                    },
                )
            })
            .collect()
    }

    #[test]
    fn remote_server_must_be_present_and_slash_terminated() {
        let base = AbacConfig {
            enabled: true,
            engine: AbacEngineKind::Remote,
            policy: full_remote_policy_map(),
            ..AbacConfig::default()
        };
        let missing = AuthzConfig {
            abac: base.clone(),
            ..AuthzConfig::default()
        };
        assert_eq!(
            missing.validate(),
            Err(AuthzConfigError::RemoteServerMissing)
        );

        let no_slash = AuthzConfig {
            abac: AbacConfig {
                remote: RemoteConfig {
                    endpoint: ferrohealth_smart::config::endpoint::EndpointSection {
                        url: SecretUrl::new("http://pdp:3001/exec"),
                        ..RemoteConfig::default().endpoint
                    },
                },
                ..base.clone()
            },
            ..AuthzConfig::default()
        };
        assert!(matches!(
            no_slash.validate(),
            Err(AuthzConfigError::RemoteServerTrailingSlash(_))
        ));

        let ok = AuthzConfig {
            abac: AbacConfig {
                remote: RemoteConfig {
                    endpoint: ferrohealth_smart::config::endpoint::EndpointSection {
                        url: SecretUrl::new("http://pdp:3001/exec/"),
                        ..RemoteConfig::default().endpoint
                    },
                },
                ..base
            },
            ..AuthzConfig::default()
        };
        assert!(ok.validate().is_ok());
    }

    /// A remote PDP with no policy for a kind the enforcement point consults can
    /// only be answered by a fail-closed deny at runtime, which would break live
    /// traffic; the misconfiguration is therefore refused at boot. No openEHR
    /// spec governs authorization — our own design/extension.
    #[test]
    fn remote_engine_without_a_policy_per_kind_is_a_boot_error() {
        let remote = |policy: BTreeMap<String, PolicyRule>| AuthzConfig {
            abac: AbacConfig {
                enabled: true,
                engine: AbacEngineKind::Remote,
                remote: RemoteConfig {
                    endpoint: ferrohealth_smart::config::endpoint::EndpointSection {
                        url: SecretUrl::new("http://pdp:3001/exec/"),
                        ..RemoteConfig::default().endpoint
                    },
                },
                policy,
                ..AbacConfig::default()
            },
            ..AuthzConfig::default()
        };

        // An empty map names the first missing kind.
        assert_eq!(
            remote(BTreeMap::new()).validate(),
            Err(AuthzConfigError::RemotePolicyMissing(
                REQUIRED_REMOTE_POLICY_KINDS[0]
            ))
        );

        // Each kind is required in turn: drop exactly one from a full map.
        for kind in REQUIRED_REMOTE_POLICY_KINDS {
            let mut policy = full_remote_policy_map();
            policy.remove(kind);
            assert_eq!(
                remote(policy).validate(),
                Err(AuthzConfigError::RemotePolicyMissing(kind)),
                "a remote PDP with no {kind} policy must not boot"
            );
        }

        // The full map boots; `directory` is required only with the opt-in on.
        assert!(remote(full_remote_policy_map()).validate().is_ok());
        let mut with_directory_check = remote(full_remote_policy_map());
        with_directory_check.abac.check_directory = true;
        assert_eq!(
            with_directory_check.validate(),
            Err(AuthzConfigError::RemotePolicyMissing("directory"))
        );

        // The Cedar engine reads its policies from the Cedar register, so the
        // map is not required there.
        let cedar = AuthzConfig {
            abac: AbacConfig {
                enabled: true,
                engine: AbacEngineKind::Cedar,
                ..AbacConfig::default()
            },
            ..AuthzConfig::default()
        };
        assert!(cedar.validate().is_ok());
    }

    /// DIRECTORY gating is its own switch, not an inference from the remote-PDP
    /// policy map, so it is off by default and settable under either engine.
    #[test]
    fn check_directory_defaults_off_and_is_engine_independent() {
        assert!(!AbacConfig::default().check_directory);
        let cedar = AuthzConfig {
            abac: AbacConfig {
                enabled: true,
                engine: AbacEngineKind::Cedar,
                check_directory: true,
                ..AbacConfig::default()
            },
            ..AuthzConfig::default()
        };
        assert!(
            cedar.validate().is_ok(),
            "the Cedar engine needs no policy-map entry to gate DIRECTORY"
        );
    }

    /// The Cedar engine takes its policy set from the Cedar register, so it
    /// needs no configuration of its own, and `[authz.abac.cedar]` is an
    /// unknown table.
    #[test]
    fn cedar_needs_no_configuration() {
        let c = AuthzConfig {
            abac: AbacConfig {
                enabled: true,
                engine: AbacEngineKind::Cedar,
                ..AbacConfig::default()
            },
            ..AuthzConfig::default()
        };
        assert_eq!(c.validate(), Ok(()));
        assert!(
            toml::from_str::<AbacConfig>("[cedar]\npolicy_dir = \"/etc/ferroehr\"").is_err(),
            "[authz.abac.cedar] is no longer a configuration table"
        );
    }

    #[test]
    fn remote_timeouts_default() {
        let c = AbacConfig::default();
        assert_eq!(
            c.remote.endpoint.connect_timeout,
            Some(ferrohealth_config::units::Duration::from_secs(2))
        );
        assert_eq!(
            c.remote.endpoint.timeout,
            ferrohealth_config::units::Duration::from_secs(5)
        );
    }

    /// Every section here carries a container-level `#[serde(default)]`, which
    /// serde fills from the struct's own [`Default`] — so the `Default` impl is
    /// the single source of defaults and no field needs its own default
    /// attribute. A partially-specified table must therefore leave every
    /// unmentioned key at its `Default` value.
    #[test]
    fn a_partial_table_falls_back_to_the_default_impl() {
        let parsed: AuthzConfig = toml::from_str(
            "[rbac]\nadmin_role = \"OPERATOR\"\n\n[abac]\nenabled = true\n\n\
             [abac.remote.endpoint]\nconnect_timeout = \"750ms\"\n",
        )
        .expect("partial authz table");

        let default = AuthzConfig::default();
        // The specified keys took the file's values.
        assert_eq!(parsed.rbac.admin_role, "OPERATOR");
        assert!(parsed.abac.enabled);
        assert_eq!(
            parsed.abac.remote.endpoint.connect_timeout,
            Some(ferrohealth_config::units::Duration::from_millis(750))
        );
        // Every unmentioned key came from the Default impl, not from
        // `<field type>::default()` (which would be `false`/`0`/empty).
        assert_eq!(parsed.rbac.enabled, default.rbac.enabled);
        assert_eq!(parsed.rbac.readonly_role, default.rbac.readonly_role);
        assert_eq!(parsed.abac.engine, default.abac.engine);
        assert_eq!(
            parsed.abac.organization_claim,
            default.abac.organization_claim
        );
        assert_eq!(parsed.abac.patient_claim, default.abac.patient_claim);
        assert_eq!(
            parsed.abac.patient_namespace,
            default.abac.patient_namespace
        );
        assert_eq!(parsed.abac.check_directory, default.abac.check_directory);
    }
}
