// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Authorization configuration ([`AuthzConfig`]).
//!
//! The `[authz]` section of the one server configuration tree, with no loader of
//! its own. No openEHR spec governs RBAC, so this is our own design.
//!
//! The coarse RBAC keys (`rbac.*`, always evaluated when auth is enabled), the
//! assurance floor, the party binding and the grant ceiling. Access decisions
//! on clinical data are the access evaluator's, over the `access_directives`
//! of each `EHR_ACCESS` and the controller's `ACCESS_POLICY`, and take no key
//! here. Every field has a default, so an all-defaults [`AuthzConfig`] is
//! valid (RBAC on, `ADMIN`/`READONLY` roles).
//!
//! NOTE: the management surface is NOT governed here — `[management]`
//! (`[management.endpoints]`, one level per endpoint) is its single authority,
//! enforced by the per-route guard in `ferroehr-rest`.

use ferrohealth_secret::secret_url::SecretUrl;
use serde::{Deserialize, Serialize};

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
    /// The issuers whose role claims may grant `admin_role` and
    /// `emergency_access_role`, each a `persons` issuer; empty, the default,
    /// lets no token grant them. Each renders with any userinfo masked.
    pub role_issuers: Vec<SecretUrl>,
    /// The Basic user of `[[auth.basic.users]]` that holds `admin_role` and
    /// `[auth] product_scope` from this file
    /// (`FERROEHR__AUTHZ__RBAC__FIRST_ADMIN`, default unset).
    ///
    /// No policy grant gives `admin_role`, so this user is the administrator
    /// who writes the first policy parties through the Demographic API.
    /// Unset, no Basic user holds `admin_role`. Our own design.
    pub first_admin: Option<String>,
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
            emergency_access_role: None,
            role_issuers: Vec::new(),
            first_admin: None,
            ehr_access_default: EhrAccessDefault::Open,
        }
    }
}

impl RbacConfig {
    /// The privileged roles, upper-cased: `admin_role` and
    /// `emergency_access_role`, which a `hold_roles` grant never gives.
    #[must_use]
    pub fn privileged_roles(&self) -> Vec<String> {
        [Some(&self.admin_role), self.emergency_access_role.as_ref()]
            .into_iter()
            .flatten()
            .map(|role| role.trim().to_uppercase())
            .collect()
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
    /// The operations are the codes of `ferroehr-operations`
    /// ([`crate::party_binding::grants::operation::Operation`]). The default
    /// allows only `commit_composition`, which lets a client write the
    /// templates its grant names and nothing else: every other operation
    /// reaches beyond one system's own writes, so each is allowed by name. An
    /// empty list lets the store grant nothing.
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

/// Authorization configuration: the coarse RBAC gate, the assurance floor, the
/// party binding and the grant ceiling.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct AuthzConfig {
    /// Role-based access control.
    pub rbac: RbacConfig,
    /// The authentication assurance a patient-data request must carry
    /// (`[authz.assurance]`, off by default).
    #[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "trust")))]
    pub assurance: ferrohealth_smart::assurance::AssuranceSection,
    /// The party binding: the provider, the namespace and the Basic users'
    /// keys (`[authz.party_binding]`).
    pub party_binding: crate::config::binding::PartyBindingConfig,
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
}

impl AuthzConfig {
    /// Validate the configuration at boot. All defaults are valid.
    ///
    /// # Errors
    /// [`AuthzConfigError`] when a role name is blank.
    pub fn validate(&self) -> Result<(), AuthzConfigError> {
        if self.rbac.admin_role.trim().is_empty() {
            return Err(AuthzConfigError::EmptyRole("admin_role"));
        }
        if self.rbac.readonly_role.trim().is_empty() {
            return Err(AuthzConfigError::EmptyRole("readonly_role"));
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

    /// Every section here carries a container-level `#[serde(default)]`, which
    /// serde fills from the struct's own [`Default`] — so the `Default` impl is
    /// the single source of defaults and no field needs its own default
    /// attribute. A partially-specified table must therefore leave every
    /// unmentioned key at its `Default` value.
    #[test]
    fn a_partial_table_falls_back_to_the_default_impl() {
        let parsed: AuthzConfig =
            toml::from_str("[rbac]\nadmin_role = \"OPERATOR\"\n\n[grants]\nallowed = []\n")
                .expect("partial authz table");

        let default = AuthzConfig::default();
        // The specified keys took the file's values.
        assert_eq!(parsed.rbac.admin_role, "OPERATOR");
        assert_eq!(parsed.grants.allowed, []);
        // Every unmentioned key came from the Default impl, not from
        // `<field type>::default()` (which would be `false`/`0`/empty).
        assert_eq!(parsed.rbac.enabled, default.rbac.enabled);
        assert_eq!(parsed.rbac.readonly_role, default.rbac.readonly_role);
        assert_eq!(
            parsed.rbac.emergency_access_role,
            default.rbac.emergency_access_role
        );
        assert_eq!(
            parsed.rbac.ehr_access_default,
            default.rbac.ehr_access_default
        );
    }

    /// `[authz.abac]` is gone with every key it held: the access evaluator
    /// takes no switch, engine or claim (#5515).
    #[test]
    fn the_abac_table_is_unknown() {
        let err = toml::from_str::<AuthzConfig>("[abac]\n").expect_err("an unknown table");
        assert!(err.to_string().contains("unknown field `abac`"), "{err}");
    }
}
