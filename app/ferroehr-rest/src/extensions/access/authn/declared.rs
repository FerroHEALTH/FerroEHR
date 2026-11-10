// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The callers the configuration declares: the system clients that commit
//! (`[[authz.system_client]]`) and the agreements a verified external person
//! or organisation of a `federation` issuer is admitted under
//! (`[[authz.federation.agreement]]`), only within the agreement of its
//! issuer and accessor, and only with the scopes the agreement reads.
//!
//! Its rights come from the agreement and never from a role (GDPR Art. 5(2)
//! and Art. 30(1)(b); `docs/law/eu/gdpr/text.html`), and its reads are
//! `patient/` scopes, so the launch-context confinement of SMART on openEHR
//! `master08-scopes.adoc` §Resource Scopes holds it to its token-bound
//! patient. What an external caller may do is our own design.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::response::Response;
use ferroehr::config::agreement::{AccessClass, Accessor, Agreement};
use ferroehr::config::clients::SystemClient;
use ferroehr::versioning::committer::ExternalCaller;
use ferroehr::versioning::committer::external::ExternalProfessional;
use ferrohealth_server::caller_rate::{self, CallerKey, CallerLimit};
use ferrohealth_smart::federation::{Attribute, External, Field};
use ferrohealth_smart::kind::CallerKind;
use ferrohealth_smart::resource::{Access, Caller};

use super::AuthError;

/// The agreement an external caller was admitted under, carried on its
/// [`Caller`] as an extension.
#[derive(Debug, Clone)]
pub(crate) struct Admitted(Arc<Held>);

/// One agreement as this node holds it: the table, where its issuer reads a
/// `chained` request's correlation id from, its caller rate, and whether its
/// issuer is a MyHealth@EU contact point.
#[derive(Debug)]
struct Held {
    agreement: Arc<Agreement>,
    correlation_source: Option<String>,
    limit: Option<CallerLimit>,
    contact_point: bool,
}

/// The scope tokens an external caller may hold beside the agreement's
/// `reads`: the OpenID Connect Core §5.4 and SMART launch requests, which
/// grant no access here.
const NEUTRAL: [&str; 5] = ["openid", "profile", "fhirUser", "online_access", "launch"];

/// The `[[authz.system_client]]` tables and the
/// `[[authz.federation.agreement]]` tables, by issuer and accessor.
#[derive(Debug, Default)]
pub(super) struct Declared {
    /// The system clients.
    pub(super) system_clients: Vec<Arc<SystemClient>>,
    /// The agreements, each with the jurisdiction it holds for; boot has
    /// refused a table without an accessor.
    agreements: BTreeMap<(String, Accessor), Arc<Held>>,
}

impl Declared {
    /// Returns the tables `config` declares.
    pub(super) fn of(config: &crate::config::AppConfig) -> Self {
        Self {
            system_clients: config
                .system_clients
                .iter()
                .cloned()
                .map(Arc::new)
                .collect(),
            agreements: config
                .agreements
                .iter()
                .filter_map(|table| {
                    let accessor = table.accessor?;
                    let agreement = Agreement {
                        jurisdiction: table.jurisdiction_of(&config.auth.shared),
                        ..table.clone()
                    };
                    let correlation_source = ferroehr::config::agreement::correlation_sources(
                        &config.auth.shared,
                        &table.issuer,
                    )
                    .map(|sources| {
                        sources
                            .iter()
                            .map(ferroehr::config::agreement::describe_source)
                            .collect::<Vec<_>>()
                            .join(" ")
                    });
                    let contact_point = config.auth.shared.federation.iter().any(|federation| {
                        federation.issuer == table.issuer && federation.contact_point == Some(true)
                    });
                    let held = Held {
                        agreement: Arc::new(agreement),
                        correlation_source,
                        limit: table.caller_rate.map(CallerLimit::new),
                        contact_point,
                    };
                    Some(((table.issuer.clone(), accessor), Arc::new(held)))
                })
                .collect(),
        }
    }

    /// Admits the caller of `access`: an external caller only under the
    /// agreement of its issuer and accessor, holding no scope beyond its
    /// `reads`, returned carrying [`Admitted`]; any other caller unchanged.
    ///
    /// A `chained` organisation request carries its correlation id in the field
    /// its issuer's `[[auth.federation]]` table maps, which the token attests;
    /// no request header stands in for it. Under an NL agreement, Wabvpz
    /// Art. 15f admits only a care provider and no occupational or insurance
    /// physician.
    ///
    /// # Errors
    /// [`AuthError::KindRefused`] (`403`) for an external caller without an
    /// agreement for its accessor kind, or holding a scope the agreement does
    /// not read, or making a `chained` request without an attested correlation
    /// id, or under an NL agreement asserting a refused UZI role or no care
    /// provider type.
    pub(super) fn admit(&self, access: Access) -> Result<Access, AuthError> {
        let Some(caller) = access.caller() else {
            return Ok(access);
        };
        let accessor = match caller.kind() {
            CallerKind::ExternalPerson(_) => Accessor::Person,
            CallerKind::ExternalOrganisation(_) => Accessor::Organisation,
            _ => return Ok(access),
        };
        let held = caller
            .issuer()
            .and_then(|issuer| self.agreements.get(&(issuer.to_owned(), accessor)))
            .ok_or_else(|| AuthError::KindRefused {
                code: "refused-no-agreement",
                message: format!(
                    "no [[authz.federation.agreement]] of this issuer admits the accessor \
                     {accessor}"
                ),
                principal: None,
            })?;
        let beyond = caller.scope().iter().any(|scope| {
            !NEUTRAL.contains(&scope.as_str())
                && !scope.starts_with("launch/")
                && !held.agreement.reads.iter().any(|read| read == scope)
        });
        if beyond {
            return Err(AuthError::KindRefused {
                code: "refused-scope-beyond-agreement",
                message: "an external caller holds only the patient/ scopes its agreement reads"
                    .to_owned(),
                principal: None,
            });
        }
        if held.agreement.access_class == Some(AccessClass::Chained)
            && caller.correlation_id().is_none()
        {
            return Err(AuthError::KindRefused {
                code: "refused-correlation-missing",
                message: "a chained organisation request carries no attested correlation id"
                    .to_owned(),
                principal: None,
            });
        }
        if held.agreement.jurisdiction == "NL"
            && let Some((code, message)) = super::wabvpz::refusal(caller.kind().external())
        {
            return Err(AuthError::KindRefused {
                code,
                message: message.to_owned(),
                principal: None,
            });
        }
        Ok(Access::Caller(Arc::new(
            Caller::clone(caller).with_extension(Admitted(Arc::clone(held))),
        )))
    }
}

/// The agreement the caller of `access` was admitted under, when it is an
/// external caller.
pub(crate) fn admitted(access: &Access) -> Option<&Arc<Agreement>> {
    held(access).map(|held| &held.agreement)
}

/// What this node holds of the agreement the caller of `access` was admitted
/// under.
fn held(access: &Access) -> Option<&Held> {
    access
        .caller()
        .and_then(Caller::extension::<Admitted>)
        .map(|admitted| admitted.0.as_ref())
}

/// Where the issuer of the caller of `access` reads a `chained` request's
/// correlation id from, as the access record names it.
pub(super) fn correlation_source(access: &Access) -> Option<&str> {
    held(access).and_then(|held| held.correlation_source.as_deref())
}

/// The agreement of the caller of `access` when its issuer is a MyHealth@EU
/// contact point.
pub(super) fn contact_point(access: &Access) -> Option<&Arc<Agreement>> {
    held(access)
        .filter(|held| held.contact_point)
        .map(|held| &held.agreement)
}

/// The refusal of an external caller of `principal` above its agreement's
/// `caller_rate`, keyed on its issuer's subject: `429` with `Retry-After`
/// from the family's caller limit, carrying the caller and its binding record
/// for the access log, which counts it into the caller's summary record.
/// `None` within the rate or for any other caller.
pub(super) fn over_rate(
    principal: &super::Principal,
    clients: &[Arc<SystemClient>],
) -> Option<Response> {
    let limit = held(&principal.access)?.limit.as_ref()?;
    let caller = CallerKey(Arc::from(principal.subject.as_str()));
    let wait = limit.take(&caller, tokio::time::Instant::now()).err()?;
    let mut refusal = caller_rate::refusal(wait);
    let committer = super::party::committer_identity(principal, clients);
    if let Some(mut binding) = super::record::of_request(principal, &committer) {
        binding
            .detail
            .insert("refusal".to_owned(), vec!["caller-rate".to_owned()]);
        refusal.extensions_mut().insert(binding);
    }
    refusal
        .extensions_mut()
        .insert(crate::system_log::refusals::OverRate);
    refusal.extensions_mut().insert(principal.clone());
    Some(refusal)
}

/// What the external caller of `access` may commit, when it is one.
pub(super) fn committing(access: &Access) -> Option<ExternalCaller> {
    let agreement = admitted(access)?;
    let external = access.caller().and_then(|caller| match caller.kind() {
        CallerKind::ExternalPerson(external) => Some(external.as_ref()),
        _ => None,
    });
    let contact_point = contact_point(access).is_some();
    Some(match (agreement.accessor, external) {
        (Some(Accessor::Person), Some(external)) => ExternalCaller::Person {
            writes: agreement.writes.clone(),
            professional: Box::new(ExternalProfessional {
                contact_point,
                ..professional(external)
            }),
            discard_window_hours: agreement
                .discard
                .then(|| agreement.discard_window_hours())
                .flatten(),
        },
        _ => ExternalCaller::Organisation,
    })
}

/// What `external`'s token asserts about the professional and the provider.
/// The name is the asserted full name (IHE IUA `subject_name`, eHDSI
/// `subject-id`), or the given and the family name joined by a space.
fn professional(external: &External) -> ExternalProfessional {
    let text = |field| {
        external
            .field(field)
            .and_then(Attribute::text)
            .map(str::to_owned)
    };
    let name = text(Field::ProfessionalName).or_else(|| {
        let parts: Vec<String> = [Field::ProfessionalGivenName, Field::ProfessionalFamilyName]
            .into_iter()
            .filter_map(text)
            .collect();
        (!parts.is_empty()).then(|| parts.join(" "))
    });
    ExternalProfessional {
        name,
        hp_identifier: text(Field::ProfessionalHpIdentifier),
        country_code: text(Field::ProfessionalCountryCode),
        issuing_authority: text(Field::ProfessionalIssuingAuthorityName),
        provider_identifier: text(Field::ProviderHealthcareProviderIdentifier),
        provider_authority: text(Field::ProviderIssuingAuthorityName),
        contact_point: false,
        name_id: external.name_id(),
        assertion_issuer: text(Field::AssertionIssuer),
        permissions: external.field(Field::ProfessionalPermissions).map_or_else(
            Vec::new,
            |permissions| {
                permissions
                    .values()
                    .into_iter()
                    .map(str::to_owned)
                    .collect()
            },
        ),
    }
}
