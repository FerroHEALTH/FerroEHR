// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The party binding of FerroEHR: `[authz.party_binding]`, and the boot rules
//! that hold the shared `[auth]` section, the binding and FerroEHR's per-issuer
//! tables together.
//!
//! `ferrohealth-smart` owns the issuers, `[auth.issuer.<name>.party]` and
//! `[auth.issuer.<name>.federation]`, and refuses what breaks its own rules.
//! FerroEHR adds the provider it acts for, the `PARTY_REF.namespace` a bound
//! party is written under, the key each Basic user is bound by, and the rules
//! no single table can see: no two issuers share an audience, and each
//! FerroEHR table names a trusted issuer once.
//! No openEHR spec governs configuration: our own design.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use ferrohealth_smart::config::Resolved;
use ferrohealth_smart::kind::IssuerKind;
use ferrohealth_smart::party::binding::PartyBinding;
use ferrohealth_smart::party::config::PartyError;
use ferrohealth_smart::party::source::{Lookup, PartySource, Query, SourceError};
use serde::{Deserialize, Serialize};

mod defaults;
pub mod jit;
pub mod person;

/// The `[authz.party_binding]` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "trust")))]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-must-set" = ["provider.issuer", "provider.id"])))]
pub struct PartyBindingConfig {
    /// The healthcare provider this deployment acts for, which every access
    /// record of a bound person and every version a person commits name
    /// (EHDS Art. 13(4), Annex II 3.2(a)).
    pub provider: Option<ProviderId>,
    /// The `PARTY_REF.namespace` a bound party is written under, held to the
    /// `OBJECT_REF.namespace` pattern.
    #[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "optional")))]
    pub namespace: String,
    /// The key each Basic user is bound by (`[[authz.party_binding.basic_user]]`).
    pub basic_user: Vec<BasicUserKey>,
    /// The person templates and attributes of the shared party mapping. Each
    /// mapping section defaults to FerroEHR's default demographic definitions,
    /// which every boot stores when no definition of their id is stored (the
    /// book's "Default demographic definitions" page); a deployment with
    /// templates of its own sets the section, and stores its templates before
    /// boot.
    pub person: Option<person::PersonSection>,
    /// The professional `ROLE` template.
    pub professional_role: Option<person::RoleSection>,
    /// The provisioned `GROUP` template.
    pub group: Option<person::GroupSection>,
    /// The membership `PARTY_RELATIONSHIP` archetype.
    pub membership: Option<person::MembershipSection>,
    /// The patient's `PERSON` template.
    pub patient: Option<person::PatientSection>,
    /// How FerroEHR reads each `persons` issuer's tokens when it creates a
    /// provisional `PERSON` at a first login
    /// (`[[authz.party_binding.issuer]]`).
    pub issuer: Vec<jit::IssuerBinding>,
    /// The age past which `ferroehr party-binding check` reports a
    /// provisional `PERSON`.
    #[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "optional")))]
    pub provisional_report_after: ferrohealth_config::units::Duration,
}

impl PartyBindingConfig {
    /// The person section, empty when the table sets an empty one.
    #[must_use]
    pub fn person(&self) -> &person::PersonSection {
        self.person.as_ref().unwrap_or(&person::NO_PERSON)
    }

    /// The mapping sections of the table.
    #[must_use]
    pub fn sections(&self) -> person::Sections<'_> {
        person::Sections {
            person: self.person(),
            professional_role: self.professional_role.as_ref(),
            group: self.group.as_ref(),
            membership: self.membership.as_ref(),
            patient: self.patient.as_ref(),
        }
    }
}

/// A healthcare provider, named by an identifier: its issuer and its id (in
/// the Netherlands the URA).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "trust")))]
pub struct ProviderId {
    /// The issuer of the identifier.
    pub issuer: String,
    /// The identifier.
    pub id: String,
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}|{}", self.issuer, self.id)
    }
}

/// One `[[authz.party_binding.basic_user]]` table: the binding key a Basic
/// user is looked up by.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "trust")))]
pub struct BasicUserKey {
    /// The `[[auth.basic.users]]` user-id.
    pub user: String,
    /// The `identifier_issuer` of the `[auth.issuer.<name>.party]` table the key belongs
    /// to.
    pub key_issuer: String,
    /// The binding key the user's `PERSON` carries.
    pub key: String,
}

/// Why the party binding cannot be built.
#[derive(Debug, thiserror::Error)]
pub enum BindingError {
    /// The shared crate refuses a party table or a Basic user's key.
    #[error(transparent)]
    Party(#[from] PartyError),
    /// A Basic user maps to no key while an `[auth.issuer.<name>.party]` table binds
    /// logins, so the user resolves to no `PERSON`.
    #[error(
        "beside an party table a Basic user is a person's credential: map it with \
         [[authz.party_binding.basic_user]] to the key of its PERSON, which SCIM provisions \
         once the operator grant (`ferroehr party-binding grant --operation scim_provision`) \
         admits the SCIM client"
    )]
    BasicUnmapped(#[source] PartyError),
}

/// Returns the party binding the configuration describes, looking logins up
/// in `source`.
///
/// Every Basic user of the section must map to a key.
///
/// # Errors
/// [`BindingError`] naming the table or the Basic user refused;
/// [`BindingError::BasicUnmapped`] names the operator grant that bootstraps
/// the `PERSON` an unmapped Basic user needs.
pub fn party_binding(
    binding: &PartyBindingConfig,
    resolved: &Resolved,
    source: Arc<dyn PartySource>,
) -> Result<PartyBinding, BindingError> {
    let mut built = PartyBinding::new(source).with_resolved(resolved)?;
    for user in &binding.basic_user {
        built = built.with_basic_user(&user.user, &user.key_issuer, &user.key)?;
    }
    if let Some(store) = &resolved.basic {
        built
            .check_basic(store)
            .map_err(BindingError::BasicUnmapped)?;
    }
    Ok(built)
}

/// The source of a binding built only to check the configuration: it is
/// never resolved, and a lookup fails.
struct CheckOnly;

impl PartySource for CheckOnly {
    fn lookup<'a>(
        &'a self,
        _query: &'a Query<'a>,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<Lookup, SourceError>> + Send + 'a>> {
        Box::pin(async {
            Err(SourceError::new(
                "the party binding was built to check the configuration",
            ))
        })
    }
}

/// The boot refusals of the party binding and of the per-issuer tables, each
/// naming its key; none when `[auth]` is off.
#[must_use]
pub fn errors(authz: &crate::config::authz::AuthzConfig, resolved: &Resolved) -> Vec<String> {
    let mut errors = Vec::new();
    if !resolved.enabled {
        return errors;
    }
    let binding = &authz.party_binding;
    if let Err(e) = party_binding(binding, resolved, Arc::new(CheckOnly)) {
        errors.push(format!(
            "authz.party_binding: {}",
            crate::error::ErrorChain::new(&e)
        ));
    }
    let users: BTreeSet<&str> = resolved
        .basic
        .as_ref()
        .map(|store| store.users().collect())
        .unwrap_or_default();
    for mapping in binding
        .basic_user
        .iter()
        .filter(|m| !users.contains(m.user.as_str()))
    {
        errors.push(format!(
            "an [[authz.party_binding.basic_user]] table maps {:?}, which no \
             [[auth.basic.users]] entry names",
            mapping.user
        ));
    }
    key_types(binding, resolved, &mut errors);
    shared_audiences(resolved, &mut errors);
    providers(binding, &mut errors);
    errors.extend(jit::errors(binding, resolved));
    if let Err(e) = resolved.role_issuers("authz.rbac.role_issuers", &authz.rbac.role_issuers) {
        errors.push(crate::error::ErrorChain::new(&e).to_string());
    }
    errors
}

/// Refuses an `[auth.issuer.<name>.party]` table whose `identifier_type` no person
/// template's `key` selector writes: the lookup matches the type exactly
/// (`party_binding::lookup::LOOKUP`), and SCIM, just-in-time creation and the
/// refill write the key under the template's type, so no login of the issuer
/// would ever match a party.
fn key_types(binding: &PartyBindingConfig, resolved: &Resolved, errors: &mut Vec<String>) {
    let written: BTreeSet<&str> = binding
        .person()
        .template
        .iter()
        .map(|template| template.key.identifier_type.as_str())
        .collect();
    for table in &resolved.party {
        if written.contains(table.identifier_type.as_str()) {
            continue;
        }
        let writes = if written.is_empty() {
            "no [[authz.party_binding.person.template]] is declared, so no key is written"
                .to_owned()
        } else {
            format!("the person templates' keys are of the types {written:?}")
        };
        errors.push(format!(
            "the party table of {:?} looks logins up under identifier_type {:?}, \
             which no person template's key carries: {writes}, so no login of the issuer \
             matches a party",
            table.issuer, table.identifier_type
        ));
    }
}

/// Refuses two issuers of this node that share an audience, so one audience
/// is one issuer and one token form.
fn shared_audiences(resolved: &Resolved, errors: &mut Vec<String>) {
    let mut by_audience: BTreeMap<&str, &str> = BTreeMap::new();
    for settings in &resolved.issuers {
        for audience in &settings.audience {
            if let Some(first) = by_audience.insert(audience, &settings.issuer)
                && first != settings.issuer
            {
                errors.push(format!(
                    "the issuers {first:?} and {:?} share the audience {audience:?}: each issuer \
                     of this node has an audience of its own",
                    settings.issuer
                ));
            }
        }
    }
}

/// Refuses a missing provider and a namespace outside the `OBJECT_REF`
/// pattern.
fn providers(binding: &PartyBindingConfig, errors: &mut Vec<String>) {
    if !openehr_base::v1_3::base_types::identification::object_ref_impl::namespace_valid(
        &binding.namespace,
    ) {
        errors.push(format!(
            "authz.party_binding.namespace {:?} does not match the OBJECT_REF.namespace pattern \
             [a-zA-Z][a-zA-Z0-9_.:/&?=+-]* (BASE base_types, OBJECT_REF.namespace)",
            binding.namespace
        ));
    }
    match &binding.provider {
        None => errors.push(
            "authz.party_binding.provider is required: set [authz.party_binding.provider] with \
             the issuer and id of the healthcare provider this deployment acts for"
                .to_owned(),
        ),
        Some(provider) => {
            if provider.issuer.trim().is_empty() || provider.id.trim().is_empty() {
                errors.push("authz.party_binding.provider needs both issuer and id".to_owned());
            }
        }
    }
}

impl crate::config::FerroEhrConfig {
    /// The boot refusals of the party binding; the section rules of `[auth]`
    /// itself are [`crate::config::auth::validate`]'s.
    pub(crate) fn identity_errors(&self) -> Vec<String> {
        let mut errors = Vec::new();
        let Ok(resolved) = self.resolve_auth() else {
            return errors;
        };
        if !resolved.enabled {
            return errors;
        }
        errors.extend(self::errors(&self.authz, &resolved));
        errors.extend(
            self.authz
                .party_binding
                .sections()
                .errors(&key_issuers(&resolved)),
        );
        errors
    }
}

/// The person mapping the server derives the side table under (#5622).
///
/// # Errors
/// [`crate::party_binding::SelectorError`] when a selector path does not compile, which
/// the configuration check reports first.
pub fn installed_mapping(
    sections: &person::Sections<'_>,
    resolved: &Resolved,
    admin_role: &str,
) -> Result<crate::party_binding::BindingMapping, crate::party_binding::SelectorError> {
    sections.binding_mapping(&key_issuers(resolved), admin_role)
}

/// The issuers a binding key is held under: every party table's
/// `identifier_issuer`.
///
/// The server compiles the person mapping from this one set, and the SCIM
/// gate admits a `scim_provision` grant whose `binding_issuer` is in it. No
/// openEHR spec governs resolving a login to a party: our own design.
#[must_use]
pub fn key_issuers(resolved: &Resolved) -> Vec<String> {
    let mut issuers: Vec<String> = resolved
        .party
        .iter()
        .map(|table| table.identifier_issuer.clone())
        .collect();
    issuers.sort();
    issuers.dedup();
    issuers
}

/// Returns the one provider `urn:ura|1`, the provider the configuration
/// tests declare.
#[cfg(test)]
pub(crate) fn one_provider() -> ProviderId {
    ProviderId {
        issuer: "urn:ura".to_owned(),
        id: "1".to_owned(),
    }
}

/// What the party binding binds, as the boot line and `/management/info`
/// state it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BindingPosture {
    /// Every trusted issuer.
    pub issuers: Vec<BoundIssuer>,
    /// The provider the deployment acts for, when declared.
    pub provider: Option<ProviderId>,
    /// The `PARTY_REF.namespace` a bound party is written under.
    pub namespace: String,
    /// The federation agreements in force, read from the CDR; empty in the
    /// posture the configuration alone states.
    pub agreements: Vec<crate::party_binding::grants::trust::AgreementPosture>,
    /// The mapped Basic users, while `[authz.assurance] minimum` is unset: a
    /// Basic credential carries no assurance level, so nothing refuses them
    /// on patient data.
    #[serde(skip)]
    pub unassured_basic: Vec<String>,
}

/// One trusted issuer of the binding posture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BoundIssuer {
    /// The issuer, its URL `userinfo` masked.
    pub issuer: String,
    /// Its kind, when declared.
    pub kind: Option<IssuerKind>,
    /// The identifier issuer its logins are bound under, for a bound issuer.
    pub identifier_issuer: Option<String>,
}

impl BindingPosture {
    /// Returns the posture `config` declares, each issuer's kind and
    /// identifier issuer as `[auth]` resolves them.
    #[must_use]
    pub fn of(config: &crate::config::FerroEhrConfig) -> Self {
        // NOTE: no openEHR spec governs this: the posture is logged after the configuration check, which refuses a section that does not resolve.
        let resolved = config.resolve_auth().ok();
        let issuers = config
            .auth
            .shared
            .issuers()
            .map(|(_, entry)| {
                let issuer = entry.issuer.expose();
                BoundIssuer {
                    issuer: entry.issuer.redacted(),
                    kind: resolved
                        .as_ref()
                        .and_then(|r| r.kinds.get(issuer).copied())
                        .or(entry.kind),
                    identifier_issuer: resolved.as_ref().and_then(|r| {
                        r.party
                            .iter()
                            .find(|table| table.issuer.expose() == issuer)
                            .map(|table| table.identifier_issuer.clone())
                    }),
                }
            })
            .collect();
        let unassured_basic = if config.authz.assurance.minimum.is_none() {
            config
                .authz
                .party_binding
                .basic_user
                .iter()
                .map(|mapping| mapping.user.clone())
                .collect()
        } else {
            Vec::new()
        };
        Self {
            issuers,
            provider: config.authz.party_binding.provider.clone(),
            namespace: config.authz.party_binding.namespace.clone(),
            agreements: Vec::new(),
            unassured_basic,
        }
    }

    /// Logs the posture: one line for the binding and a `warn` while a mapped
    /// Basic user exists with no assurance minimum. The agreements are
    /// logged once the grants are read
    /// ([`crate::party_binding::grants::live::LiveGrants::log_agreements`]).
    pub fn log(&self) {
        let issuers: Vec<String> = self
            .issuers
            .iter()
            .map(|bound| {
                let kind = bound.kind.map_or("-", IssuerKind::as_str);
                let key = bound.identifier_issuer.as_deref().unwrap_or("-");
                format!("{} ({kind}, {key})", bound.issuer)
            })
            .collect();
        let provider = self
            .provider
            .as_ref()
            .map_or_else(|| "-".to_owned(), ToString::to_string);
        tracing::info!(
            issuers = %issuers.join(", "),
            provider = %provider,
            namespace = %self.namespace,
            "party binding configured"
        );
        if !self.unassured_basic.is_empty() {
            tracing::warn!(
                users = %self.unassured_basic.join(", "),
                "mapped Basic users sign in with no assurance level while \
                 [authz.assurance] minimum is unset"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    /// The configuration of the environment `pairs` alone.
    fn assemble_ok(pairs: &[(&str, &str)]) -> crate::config::FerroEhrConfig {
        let env: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        crate::config::assemble(None, &env, &[]).expect("assembles")
    }

    /// The assurance block, the party binding and the role keys are reachable
    /// from the env grammar (the `levels` map is file-only: its keys are
    /// issuer URIs).
    #[test]
    fn env_mapping_reaches_the_assurance_and_party_binding_blocks() {
        let c = assemble_ok(&[
            ("FERROEHR__AUTHZ__ASSURANCE__CLAIM", "loa"),
            ("FERROEHR__AUTHZ__ASSURANCE__MINIMUM", "high"),
            ("FERROEHR__AUTHZ__PARTY_BINDING__NAMESPACE", "hr"),
            (
                "FERROEHR__AUTHZ__PARTY_BINDING__PROVIDER__ISSUER",
                "urn:ura",
            ),
            ("FERROEHR__AUTHZ__PARTY_BINDING__PROVIDER__ID", "1234"),
            (
                "FERROEHR__AUTHZ__PARTY_BINDING__PROVISIONAL_REPORT_AFTER",
                "14d",
            ),
            (
                "FERROEHR__AUTHZ__RBAC__EMERGENCY_ACCESS_ROLE",
                "BREAK_GLASS",
            ),
            ("FERROEHR__AUTHZ__RBAC__ROLE_ISSUERS", "a,b"),
        ]);
        assert_eq!(c.authz.assurance.claim, "loa");
        assert_eq!(
            c.authz.assurance.minimum,
            Some(ferrohealth_smart::assurance::AssuranceLevel::High)
        );
        assert_eq!(c.authz.party_binding.namespace, "hr");
        assert_eq!(
            c.authz.party_binding.provisional_report_after,
            ferrohealth_config::units::Duration::from_days(14)
        );
        assert_eq!(
            c.authz.party_binding.provider,
            Some(ProviderId {
                issuer: "urn:ura".to_owned(),
                id: "1234".to_owned(),
            })
        );
        assert_eq!(
            c.authz.rbac.emergency_access_role.as_deref(),
            Some("BREAK_GLASS")
        );
        assert_eq!(c.authz.rbac.role_issuers, ["a", "b"]);
    }
}
