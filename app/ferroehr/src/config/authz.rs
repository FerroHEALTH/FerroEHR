// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Authorization configuration ([`AuthzConfig`]).
//!
//! The `[authz]` section of the one server configuration tree, with no loader of
//! its own. No openEHR spec governs RBAC or ABAC, the SM placing authorization
//! out of band, so this is our own design.
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
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The server-wide disposition for an EHR that carries no
/// `ACCESS_CONTROL_SETTINGS` of its own — the `ehr_access_default` tri-state's
/// two states.
///
/// NOTE: no openEHR spec governs this — our own design/extension; the SM places
/// authorization out of band (SM `openehr_platform/master02-overview.adoc`
/// §General Assumptions), and RM `master07` only defines the per-EHR object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
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
#[serde(default, deny_unknown_fields)]
pub struct RbacConfig {
    /// Master switch; the coarse role gate is active only when auth is enabled
    /// (`FERROEHR__AUTHZ__RBAC__ENABLED`, default `true`). Disabling restores
    /// authentication-only behaviour.
    pub enabled: bool,
    /// The role required for Admin-class operations (default `ADMIN`).
    pub admin_role: String,
    /// The baseline clinical role (default `USER`).
    pub user_role: String,
    /// The role marking a principal as read-only (default `READONLY`): a caller
    /// carrying it is refused on every write operation even when it also holds
    /// granting roles (a restriction overrides a grant). Supports the CNF
    /// SEC-BASIC authorization-separation profile.
    pub readonly_role: String,
    /// The role that may read the access log for ONE subject at a time
    /// (`FERROEHR__AUTHZ__RBAC__SUBJECT_AUDIT_ROLE`, default unset).
    ///
    /// A portal that shows a person who accessed their record (GDPR Art. 15
    /// with Recital 63, EHDS Art. 9, for Dutch deployments Wabvpz Art. 15e)
    /// needs the ITI-81 retrieval for that person and nobody else. A caller
    /// holding this role may call `GET /fhir/r4/AuditEvent` only with the
    /// `patient` parameter, the subject's opaque pseudonym; without it the call
    /// is refused, and the unscoped retrieval stays with `admin_role`. Unset,
    /// the log is admin-only, as before (#3240). No openEHR spec governs the
    /// access log — our own design/extension.
    pub subject_audit_role: Option<String>,
    /// JWT claim paths mined for roles, in resolution order.
    ///
    /// Defaults to the carriers RFC 9068 §2.2.3.1 names for conveying
    /// authorization state — `roles`, `groups`, `entitlements` (`roles` and
    /// `entitlements` being SCIM attributes, RFC 7643 §4.1.2) — followed by the
    /// widely deployed nested `realm_access.roles`. An issuer that nests them
    /// differently is configuration, not a code change.
    ///
    /// `scope` is deliberately NOT a default: an OAuth2 scope grants a client
    /// delegated authority (RFC 6749 §3.3) and asserts nothing about the
    /// subject's roles, so reading it as one makes the "at least one role" gate
    /// vacuous for every OIDC token.
    pub role_claims: Vec<String>,
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
    pub ehr_access_default: EhrAccessDefault,
}

impl Default for RbacConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            admin_role: "ADMIN".to_owned(),
            user_role: "USER".to_owned(),
            readonly_role: "READONLY".to_owned(),
            subject_audit_role: None,
            role_claims: vec![
                "roles".to_owned(),
                "groups".to_owned(),
                "entitlements".to_owned(),
                "realm_access.roles".to_owned(),
            ],
            ehr_access_default: EhrAccessDefault::Open,
        }
    }
}

/// The ABAC policy engine selector (`abac.engine`). `cedar` is the
/// embedded default; `remote` delegates to an external PDP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AbacEngineKind {
    /// Embedded Cedar (`cedar-policy`), policies from `abac.cedar.policy_dir`.
    #[default]
    Cedar,
    /// The remote policy-decision-point (`abac.remote.server`).
    Remote,
}

/// One flat parameter a remote PDP body may carry: the resolved
/// attribute keys, exactly `organization` / `patient` / `template` on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbacParam {
    /// The caller's organization (from `abac.organization_claim`).
    Organization,
    /// The patient the resource is about (from `abac.patient_claim` / EHR subject).
    Patient,
    /// The resource's template id.
    Template,
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
        }
    }
}

/// A per-resource-kind policy binding: the policy name appended to the
/// remote PDP base URL, and which resolved parameters its request body carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyRule {
    /// The policy name (the path segment appended to `abac.remote.server`).
    pub name: String,
    /// The parameters the PDP body carries; empty = an unparameterized policy.
    #[serde(default)]
    pub parameters: Vec<AbacParam>,
}

/// The embedded-Cedar engine settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CedarConfig {
    /// Directory of `*.cedar` policy files (required when `engine = cedar`).
    pub policy_dir: Option<PathBuf>,
    /// Optional periodic hot-reload interval (seconds); off when unset.
    pub reload_secs: Option<u64>,
}

/// The remote-PDP client settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RemoteConfig {
    /// The PDP base URL; the policy name is appended, so it must end with `/`
    /// (required when `engine = remote`).
    pub server: Option<String>,
    /// TCP connect timeout in milliseconds (default 2000): without one, a PDP
    /// that blackholes packets parks the request until the OS TCP timeout.
    pub connect_timeout_ms: u64,
    /// Whole-request timeout in milliseconds (default 5000).
    pub request_timeout_ms: u64,
}

impl Default for RemoteConfig {
    fn default() -> Self {
        Self {
            server: None,
            connect_timeout_ms: 2_000,
            // 5 s: the whole-request budget the discovery and terminology
            // clients use, so one outbound-HTTP posture spans the server.
            request_timeout_ms: 5_000,
        }
    }
}

/// The fine-grained attribute-based access-control settings.
///
/// Master switch `enabled` defaults to `false`, so an all-defaults config
/// preserves today's authentication-plus-RBAC behaviour until an operator
/// opts in.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
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
    /// Embedded-Cedar settings.
    pub cedar: CedarConfig,
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
    /// entry ([`REQUIRED_REMOTE_POLICY_KINDS`]), checked at boot: a runtime
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
            cedar: CedarConfig::default(),
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

    /// The organization claim, or `None` when blank.
    #[must_use]
    pub fn organization_claim(&self) -> Option<&str> {
        let c = self.organization_claim.trim();
        (!c.is_empty()).then_some(c)
    }
}

/// Authorization configuration: the coarse RBAC gate plus the opt-in ABAC
/// policy layer.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuthzConfig {
    /// Role-based access control.
    pub rbac: RbacConfig,
    /// Attribute-based access control (opt-in).
    pub abac: AbacConfig,
}

/// A boot-time authorization-configuration error — a hard error that must
/// abort startup rather than silently mis-gate.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AuthzConfigError {
    /// A required role name was blank.
    #[error("authz.rbac.{0} must not be empty")]
    EmptyRole(&'static str),
    /// The role-claim path list was empty or contained a blank entry.
    #[error("authz.rbac.role_claims must be non-empty and contain no blank paths")]
    RoleClaims,
    /// A policy key does not name a known resource kind.
    #[error(
        "authz.abac.policy.{0} is not a known resource kind \
         (ehr, ehr_status, composition, contribution, query, directory)"
    )]
    UnknownPolicyKind(String),
    /// A `template` parameter was configured for `ehr`/`ehr_status` — illegal
    /// A missing entry is refused at boot rather than at the first request, so a
    /// deployment cannot discover the gap from a 500 in production.
    #[error("authz.abac.policy.{0} must not take the `template` parameter")]
    TemplateParamIllegal(&'static str),
    /// `engine = remote` without a configured base URL.
    #[error("authz.abac.remote.server is required when abac.engine = remote")]
    RemoteServerMissing,
    /// The remote base URL does not end with `/` (the policy name is appended).
    #[error("authz.abac.remote.server must end with '/' (got {0:?})")]
    RemoteServerTrailingSlash(String),
    /// `engine = cedar` without a configured policy directory.
    #[error("authz.abac.cedar.policy_dir is required when abac.engine = cedar")]
    CedarDirMissing,
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
    /// [`AuthzConfigError`] when a role name is blank or the role-claim path list
    /// is empty / contains a blank path.
    pub fn validate(&self) -> Result<(), AuthzConfigError> {
        if self.rbac.admin_role.trim().is_empty() {
            return Err(AuthzConfigError::EmptyRole("admin_role"));
        }
        if self.rbac.user_role.trim().is_empty() {
            return Err(AuthzConfigError::EmptyRole("user_role"));
        }
        if self.rbac.readonly_role.trim().is_empty() {
            return Err(AuthzConfigError::EmptyRole("readonly_role"));
        }
        if self.rbac.role_claims.is_empty()
            || self.rbac.role_claims.iter().any(|c| c.trim().is_empty())
        {
            return Err(AuthzConfigError::RoleClaims);
        }
        if self.abac.enabled {
            self.abac.validate()?;
        }
        Ok(())
    }
}

/// The canonical resource-kind keys accepted in the `abac.policy` map, and the
/// two on which a `template` parameter is illegal.
const POLICY_KINDS: [&str; 6] = [
    "ehr",
    "ehr_status",
    "composition",
    "contribution",
    "query",
    "directory",
];

/// The resource kinds a remote PDP must have a policy for.
///
/// `directory` is absent deliberately: DIRECTORY gating is the separate
/// [`AbacConfig::check_directory`] opt-in, so its policy is required only when
/// that switch is on.
pub const REQUIRED_REMOTE_POLICY_KINDS: [&str; 5] =
    ["ehr", "ehr_status", "composition", "contribution", "query"];

impl AbacConfig {
    /// Validate the ABAC section at boot (hard errors). Only called when
    /// `enabled`.
    fn validate(&self) -> Result<(), AuthzConfigError> {
        self.validate_policy_rules()?;
        match self.engine {
            AbacEngineKind::Remote => self.validate_remote_engine(),
            AbacEngineKind::Cedar => {
                if self.cedar.policy_dir.is_none() {
                    return Err(AuthzConfigError::CedarDirMissing);
                }
                Ok(())
            }
        }
    }

    /// Every configured policy rule names a known resource kind, and no rule
    /// asks for a parameter its kind cannot carry.
    ///
    /// The `template` parameter is a property of committed content, so an
    /// EHR-level or EHR_STATUS-level rule can never be given one.
    fn validate_policy_rules(&self) -> Result<(), AuthzConfigError> {
        for (kind, rule) in &self.policy {
            if !POLICY_KINDS.contains(&kind.as_str()) {
                return Err(AuthzConfigError::UnknownPolicyKind(kind.clone()));
            }
            if !rule.parameters.contains(&AbacParam::Template) {
                continue;
            }
            match kind.as_str() {
                "ehr" => return Err(AuthzConfigError::TemplateParamIllegal("ehr")),
                "ehr_status" => return Err(AuthzConfigError::TemplateParamIllegal("ehr_status")),
                _ => {}
            }
        }
        Ok(())
    }

    /// The remote-PDP engine's own requirements: a base server URL with a
    /// trailing slash, and a policy for every resource kind the PEP consults
    /// (an unconfigured kind denies, and the missing rule is a boot error
    /// rather than a silent refusal at request time).
    fn validate_remote_engine(&self) -> Result<(), AuthzConfigError> {
        let server = self
            .remote
            .server
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .ok_or(AuthzConfigError::RemoteServerMissing)?;
        if !server.ends_with('/') {
            return Err(AuthzConfigError::RemoteServerTrailingSlash(
                server.to_owned(),
            ));
        }
        for kind in REQUIRED_REMOTE_POLICY_KINDS {
            if !self.policy.contains_key(kind) {
                return Err(AuthzConfigError::RemotePolicyMissing(kind));
            }
        }
        if self.check_directory && !self.policy.contains_key("directory") {
            return Err(AuthzConfigError::RemotePolicyMissing("directory"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        let c = AuthzConfig::default();
        assert!(c.rbac.enabled);
        assert_eq!(c.rbac.admin_role, "ADMIN");
        assert_eq!(c.rbac.user_role, "USER");
        assert_eq!(c.rbac.readonly_role, "READONLY");
        // The RFC 9068 §2.2.3.1 carriers, in order. `scope` is deliberately
        // absent: an OAuth2 scope grants a client delegated authority
        // (RFC 6749 §3.3) and asserts nothing about the subject's roles.
        assert_eq!(
            c.rbac.role_claims,
            vec![
                "roles".to_owned(),
                "groups".to_owned(),
                "entitlements".to_owned(),
                "realm_access.roles".to_owned(),
            ]
        );
        assert!(c.validate().is_ok());
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
    fn empty_role_claims_rejected() {
        let c = AuthzConfig {
            rbac: RbacConfig {
                role_claims: vec![],
                ..RbacConfig::default()
            },
            ..AuthzConfig::default()
        };
        assert_eq!(c.validate(), Err(AuthzConfigError::RoleClaims));
        let c = AuthzConfig {
            rbac: RbacConfig {
                role_claims: vec!["ok".to_owned(), "  ".to_owned()],
                ..RbacConfig::default()
            },
            ..AuthzConfig::default()
        };
        assert_eq!(c.validate(), Err(AuthzConfigError::RoleClaims));
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
                    server: None,
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
                    cedar: CedarConfig {
                        policy_dir: Some(PathBuf::from("/tmp/policies")),
                        reload_secs: None,
                    },
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
                cedar: CedarConfig {
                    policy_dir: Some(PathBuf::from("/tmp/p")),
                    reload_secs: None,
                },
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
                    server: Some("http://pdp:3001/exec".to_owned()),
                    ..RemoteConfig::default()
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
                    server: Some("http://pdp:3001/exec/".to_owned()),
                    ..RemoteConfig::default()
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
                    server: Some("http://pdp:3001/exec/".to_owned()),
                    ..RemoteConfig::default()
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

        // The Cedar engine reads its policies from disk, so the map is not
        // required there.
        let cedar = AuthzConfig {
            abac: AbacConfig {
                enabled: true,
                engine: AbacEngineKind::Cedar,
                cedar: CedarConfig {
                    policy_dir: Some(PathBuf::from("/etc/ferroehr/policies")),
                    reload_secs: None,
                },
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
                cedar: CedarConfig {
                    policy_dir: Some(PathBuf::from("/etc/ferroehr/policies")),
                    reload_secs: None,
                },
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

    #[test]
    fn cedar_requires_policy_dir() {
        let c = AuthzConfig {
            abac: AbacConfig {
                enabled: true,
                engine: AbacEngineKind::Cedar,
                ..AbacConfig::default()
            },
            ..AuthzConfig::default()
        };
        assert_eq!(c.validate(), Err(AuthzConfigError::CedarDirMissing));
    }

    #[test]
    fn remote_timeouts_default() {
        let c = AbacConfig::default();
        assert_eq!(c.remote.connect_timeout_ms, 2000);
        assert_eq!(c.remote.request_timeout_ms, 5000);
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
             [abac.remote]\nconnect_timeout_ms = 750\n",
        )
        .expect("partial authz table");

        let default = AuthzConfig::default();
        // The specified keys took the file's values.
        assert_eq!(parsed.rbac.admin_role, "OPERATOR");
        assert!(parsed.abac.enabled);
        assert_eq!(parsed.abac.remote.connect_timeout_ms, 750);
        // Every unmentioned key came from the Default impl, not from
        // `<field type>::default()` (which would be `false`/`0`/empty).
        assert_eq!(parsed.rbac.enabled, default.rbac.enabled);
        assert_eq!(parsed.rbac.user_role, default.rbac.user_role);
        assert_eq!(parsed.rbac.readonly_role, default.rbac.readonly_role);
        assert_eq!(parsed.rbac.role_claims, default.rbac.role_claims);
        assert_eq!(parsed.abac.engine, default.abac.engine);
        assert_eq!(
            parsed.abac.organization_claim,
            default.abac.organization_claim
        );
        assert_eq!(parsed.abac.patient_claim, default.abac.patient_claim);
        assert_eq!(parsed.abac.check_directory, default.abac.check_directory);
        assert_eq!(
            parsed.abac.remote.request_timeout_ms,
            default.abac.remote.request_timeout_ms
        );
    }
}
