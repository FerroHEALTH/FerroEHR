// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The party binding of FerroEHR: `[authz.party_binding]`, and the boot rules
//! that hold the shared `[auth]` section, the binding and FerroEHR's per-issuer
//! tables together.
//!
//! `ferrohealth-smart` owns the issuer kinds and profiles, `[[auth.party]]` and
//! `[[auth.federation]]`, and refuses what breaks its own rules. FerroEHR adds
//! the provider it acts for, the `PARTY_REF.namespace` a bound party is
//! written under, the key each Basic user is bound by, and the rules no single
//! table can see: every `persons` issuer is bound, no two issuers share an
//! audience, and each FerroEHR table names a trusted issuer of its kind once.
//! No openEHR spec governs configuration: our own design.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use ferrohealth_smart::config::Resolved;
use ferrohealth_smart::kind::IssuerKind;
use ferrohealth_smart::party::binding::PartyBinding;
use ferrohealth_smart::party::config::PartyError;
use ferrohealth_smart::party::source::{Lookup, PartySource, Query, SourceError};
use serde::de::value::{MapAccessDeserializer, SeqAccessDeserializer};
use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

use crate::party_binding::shipped;

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
    /// (EHDS Art. 13(4), Annex II 3.2(a)): one provider, or one table per
    /// tenant.
    pub provider: Option<Providers>,
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
    /// The representation `PARTY_RELATIONSHIP` archetype.
    pub representation: Option<person::RepresentationSection>,
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
            representation: self.representation.as_ref(),
            patient: self.patient.as_ref(),
        }
    }
}

impl Default for PartyBindingConfig {
    fn default() -> Self {
        Self {
            provider: None,
            // NOTE: ITS-REST overview, Requests_and_responses.md §openehr-version, names
            // `committer.external_ref.namespace="demographic"` in its example.
            namespace: "demographic".to_owned(),
            basic_user: Vec::new(),
            // The default mapping names the shipped definitions of
            // `deploy/definitions/`, which every boot stores when absent.
            person: Some(person::PersonSection {
                template: vec![person::PersonTemplate {
                    template_id: shipped::PROFESSIONAL_PERSON.to_owned(),
                    key: selector(IDENTIFIER, "login"),
                    user_name: Some(selector(IDENTIFIER, "scim_user_name")),
                    external_id: Some(selector(IDENTIFIER, "scim_external_id")),
                    default: true,
                }],
                attribute: vec![
                    attribute(
                        "name.formatted",
                        true,
                        "/identities[id4]/details[id5]/items[id6]",
                    ),
                    attribute(
                        "name.givenName",
                        false,
                        "/identities[id4]/details[id5]/items[id7]",
                    ),
                    attribute(
                        "name.familyName",
                        false,
                        "/identities[id4]/details[id5]/items[id8]",
                    ),
                    attribute(
                        "emails[type eq \"work\"].value",
                        false,
                        "/contacts[id9]/addresses[id10]/details[id11]/items[id12]",
                    ),
                ],
            }),
            professional_role: Some(person::RoleSection {
                template_id: shipped::PROFESSIONAL_ROLE.to_owned(),
            }),
            group: Some(person::GroupSection {
                template_id: shipped::CARE_TEAM.to_owned(),
                external_id: selector(IDENTIFIER, "scim_external_id"),
                display_name: Some("/identities[id4]/details[id5]/items[id6]".to_owned()),
            }),
            membership: Some(person::MembershipSection {
                archetype_id: shipped::MEMBERSHIP.to_owned(),
            }),
            representation: Some(person::RepresentationSection {
                archetype_id: shipped::REPRESENTATION.to_owned(),
                relationship_codes: BTreeMap::from([
                    ("at10".to_owned(), "254".to_owned()),
                    ("at11".to_owned(), "264".to_owned()),
                ]),
            }),
            patient: Some(person::PatientSection {
                template_id: shipped::PATIENT.to_owned(),
                identifier: IDENTIFIER.to_owned(),
                name: Some("/identities[id6]/details[id7]/items[id8]".to_owned()),
            }),
            issuer: Vec::new(),
            provisional_report_after: ferrohealth_config::units::Duration::from_days(30),
        }
    }
}

/// The identifier element of each default `PERSON` and `GROUP` definition.
const IDENTIFIER: &str = "/details[id2]/items[id3]";

/// The selector of the identifiers of `identifier_type` at `path`.
fn selector(path: &str, identifier_type: &str) -> person::Selector {
    person::Selector {
        path: path.to_owned(),
        identifier_type: identifier_type.to_owned(),
    }
}

/// The attribute mapping the SCIM attribute `scim` to `path` of the default
/// professional's `PERSON`.
fn attribute(scim: &str, name: bool, path: &str) -> person::PersonAttribute {
    person::PersonAttribute {
        scim: Some(scim.to_owned()),
        oidc: None,
        jit: false,
        name,
        path: BTreeMap::from([(shipped::PROFESSIONAL_PERSON.to_owned(), path.to_owned())]),
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

/// One `[[authz.party_binding.provider]]` table: the provider of the tokens
/// whose issuer profile's `tenant_claim` carries `tenant`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "trust")))]
pub struct TenantProvider {
    /// The `tenant_claim` value the table holds for.
    pub tenant: String,
    /// The issuer of the provider's identifier.
    pub issuer: String,
    /// The provider's identifier.
    pub id: String,
}

/// The `provider` key: one table, or an array of tables keyed by tenant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum Providers {
    /// `[authz.party_binding.provider]`: the one provider of the deployment.
    One(ProviderId),
    /// `[[authz.party_binding.provider]]`: one provider per tenant.
    ByTenant(Vec<TenantProvider>),
}

impl<'de> Deserialize<'de> for Providers {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(ProvidersVisitor)
    }
}

/// Reads the table form or the array form of `provider`.
struct ProvidersVisitor;

impl<'de> Visitor<'de> for ProvidersVisitor {
    type Value = Providers;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "an [authz.party_binding.provider] table or [[authz.party_binding.provider]] tables",
        )
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Providers, A::Error> {
        ProviderId::deserialize(MapAccessDeserializer::new(map)).map(Providers::One)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Providers, A::Error> {
        Vec::<TenantProvider>::deserialize(SeqAccessDeserializer::new(seq)).map(Providers::ByTenant)
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
    /// The `identifier_issuer` of the `[[auth.party]]` table the key belongs
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
}

/// Returns the party binding the configuration describes, looking logins up
/// in `source`.
///
/// The vendor adapters `[auth.idp]` builds are registered from `resolved`,
/// so a profile naming one without its table is refused; every Basic user of
/// the section must map to a key.
///
/// # Errors
/// [`BindingError`] naming the table, the adapter or the Basic user refused.
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
        built.check_basic(store)?;
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
    unbound_persons(resolved, &mut errors);
    key_types(binding, resolved, &mut errors);
    shared_audiences(resolved, &mut errors);
    providers(binding, resolved, &mut errors);
    errors.extend(jit::errors(binding, resolved));
    if let Err(e) = resolved.check_role_issuers("authz.rbac.role_issuers", &authz.rbac.role_issuers)
    {
        errors.push(crate::error::ErrorChain::new(&e).to_string());
    }
    errors
}

/// Refuses a `persons` issuer with no `[[auth.party]]` table: binding is how
/// FerroEHR attributes a person, with no unbound mode.
fn unbound_persons(resolved: &Resolved, errors: &mut Vec<String>) {
    let bound: BTreeSet<&str> = resolved.party.iter().map(|t| t.issuer.expose()).collect();
    for (issuer, kind) in &resolved.kinds {
        if *kind == IssuerKind::Persons && !bound.contains(issuer.as_str()) {
            errors.push(format!(
                "the persons issuer {issuer:?} has no [[auth.party]] table: every persons issuer \
                 binds its logins to a party"
            ));
        }
    }
}

/// Refuses an `[[auth.party]]` table whose `identifier_type` no person
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
            "the [[auth.party]] table of {:?} looks logins up under identifier_type {:?}, \
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

/// Refuses a missing provider, a namespace outside the `OBJECT_REF` pattern,
/// and a per-tenant provider list that leaves a bound issuer uncovered.
fn providers(binding: &PartyBindingConfig, resolved: &Resolved, errors: &mut Vec<String>) {
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
             the issuer and id of the healthcare provider this deployment acts for, or one \
             [[authz.party_binding.provider]] table per tenant"
                .to_owned(),
        ),
        Some(Providers::One(provider)) => {
            if provider.issuer.trim().is_empty() || provider.id.trim().is_empty() {
                errors.push("authz.party_binding.provider needs both issuer and id".to_owned());
            }
        }
        Some(Providers::ByTenant(tables)) => tenants(tables, resolved, errors),
    }
}

/// The rules of the per-tenant provider tables.
fn tenants(tables: &[TenantProvider], resolved: &Resolved, errors: &mut Vec<String>) {
    let mut seen = BTreeSet::new();
    for table in tables {
        if table.tenant.trim().is_empty()
            || table.issuer.trim().is_empty()
            || table.id.trim().is_empty()
        {
            errors.push(
                "each [[authz.party_binding.provider]] table needs tenant, issuer and id"
                    .to_owned(),
            );
        }
        if !seen.insert(table.tenant.as_str()) {
            errors.push(format!(
                "two [[authz.party_binding.provider]] tables name the tenant {:?}",
                table.tenant
            ));
        }
    }
    let bound: BTreeSet<&str> = resolved.party.iter().map(|t| t.issuer.expose()).collect();
    for settings in &resolved.issuers {
        if bound.contains(settings.issuer.as_str())
            && settings
                .profile
                .tenant_claim
                .as_deref()
                .is_none_or(|c| c.trim().is_empty())
        {
            errors.push(format!(
                "the bound issuer {:?} names no tenant_claim, so no \
                 [[authz.party_binding.provider]] table covers its logins",
                settings.issuer
            ));
        }
    }
}

impl crate::config::FerroEhrConfig {
    /// The boot refusals of the party binding, the federation agreements and
    /// `[authz.import]`; the section rules of `[auth]` itself are
    /// [`crate::config::auth::validate`]'s.
    pub(crate) fn identity_errors(&self) -> Vec<String> {
        let mut errors =
            crate::config::clients::import_errors(&self.authz.import, &self.api.system_id);
        let Ok(resolved) = crate::config::auth::resolve(&self.auth.shared) else {
            return errors;
        };
        if !resolved.enabled {
            return errors;
        }
        errors.extend(self::errors(&self.authz, &resolved));
        let protected: &[String] = self
            .privacy
            .identifier_protection
            .as_ref()
            .map_or(&[], |protection| protection.schemes.as_slice());
        errors.extend(
            self.authz
                .party_binding
                .sections()
                .errors(&key_issuers(&resolved), protected),
        );
        errors.extend(crate::config::agreement::errors(
            &self.authz.federation.agreement,
            &self.auth.shared,
            &resolved,
            self.audit.store.horizon(),
        ));
        errors
    }
}

/// The `identifier_issuer` of every `[[auth.party]]` table: the issuers a
/// binding key is held under.
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
pub(crate) fn one_provider() -> Providers {
    Providers::One(ProviderId {
        issuer: "urn:ura".to_owned(),
        id: "1".to_owned(),
    })
}

/// What the party binding binds, as the boot line and `/management/info`
/// state it.
///
/// It carries the issuers with their kind and identifier issuer, the
/// provider, and every federation agreement with its accessor and legal
/// ground.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BindingPosture {
    /// Every trusted issuer.
    pub issuers: Vec<BoundIssuer>,
    /// The provider the deployment acts for, when declared.
    pub provider: Option<Providers>,
    /// The `PARTY_REF.namespace` a bound party is written under.
    pub namespace: String,
    /// The federation agreements.
    pub agreements: Vec<AgreementPosture>,
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

/// One federation agreement of the binding posture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgreementPosture {
    /// The federation issuer.
    pub issuer: String,
    /// Who it admits.
    pub accessor: Option<crate::config::agreement::Accessor>,
    /// Its legal ground, rendered.
    pub legal_ground: Option<String>,
}

impl BindingPosture {
    /// Returns the posture `config` declares.
    #[must_use]
    pub fn of(config: &crate::config::FerroEhrConfig) -> Self {
        let party = &config.auth.shared.party;
        let issuers = config
            .auth
            .shared
            .issuers()
            .iter()
            .map(|entry| BoundIssuer {
                issuer: entry.issuer.redacted(),
                kind: entry.kind,
                identifier_issuer: party
                    .iter()
                    .find(|table| table.issuer == entry.issuer)
                    .map(|table| table.identifier_issuer.clone()),
            })
            .collect();
        let agreements = config
            .authz
            .federation
            .agreement
            .iter()
            .map(|agreement| AgreementPosture {
                issuer: agreement.issuer.redacted(),
                accessor: agreement.accessor,
                legal_ground: agreement.legal_ground.as_ref().map(ToString::to_string),
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
            agreements,
            unassured_basic,
        }
    }

    /// Logs the posture: one line for the binding, one per agreement, a
    /// `warn` for every agreement that runs without a named person, and a
    /// `warn` while a mapped Basic user exists with no assurance minimum.
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
        let provider = match &self.provider {
            None => "-".to_owned(),
            Some(Providers::One(provider)) => provider.to_string(),
            Some(Providers::ByTenant(tables)) => format!("{} tenants", tables.len()),
        };
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
        for agreement in &self.agreements {
            let accessor = agreement
                .accessor
                .map_or_else(|| "-".to_owned(), |a| a.to_string());
            let ground = agreement.legal_ground.as_deref().unwrap_or("-");
            if agreement.accessor == Some(crate::config::agreement::Accessor::Organisation) {
                tracing::warn!(
                    issuer = %agreement.issuer,
                    accessor = %accessor,
                    legal_ground = %ground,
                    "a federation agreement admits an organisation without a named person"
                );
            } else {
                tracing::info!(
                    issuer = %agreement.issuer,
                    accessor = %accessor,
                    legal_ground = %ground,
                    "federation agreement configured"
                );
            }
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
            ("FERROEHR__AUTHZ__RBAC__IMPORT_ROLE", "IMPORTER"),
            ("FERROEHR__AUTHZ__RBAC__ROLE_ISSUERS", "https://a,https://b"),
            ("FERROEHR__AUTHZ__IMPORT__TRUSTED_SYSTEMS", "sys-a"),
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
            Some(Providers::One(ProviderId {
                issuer: "urn:ura".to_owned(),
                id: "1234".to_owned(),
            }))
        );
        assert_eq!(c.authz.rbac.import_role.as_deref(), Some("IMPORTER"));
        assert_eq!(
            c.authz.rbac.role_issuers,
            [
                ferrohealth_secret::secret_url::SecretUrl::new("https://a"),
                ferrohealth_secret::secret_url::SecretUrl::new("https://b")
            ]
        );
        assert_eq!(c.authz.import.trusted_systems, ["sys-a"]);
    }
}
