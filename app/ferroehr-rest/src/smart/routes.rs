// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The SMART on openEHR requirement of every ITS-REST route.
//!
//! The grammar is the ITS-REST "SMART on openEHR" draft (DEVELOPMENT status,
//! `docs/specs/openehr/ITS-REST/docs/smart_app_launch/master08-scopes.adoc`
//! §Resource Scopes): `<compartment>/<resource>.<permissions>` over three
//! resource families, `template-`, `composition-` and `aql-`. The
//! `ferrohealth-smart` crate parses the token's scopes and decides whether a
//! [`Requirement`] is met; this table is FerroEHR's declaration of which
//! requirement each operation carries.
//!
//! Where the draft is silent these mappings are ours (no openEHR spec governs
//! them: our own design, the crate's rules):
//!
//! - The EHR, its `EHR_STATUS`, its `DIRECTORY` and a `CONTRIBUTION` have no
//!   family of their own; they are the EHR's data, so each is held to the
//!   `composition-` family with the operation's permission over every template.
//! - A COMPOSITION operation names its template only in its body or in the
//!   stored version, so the policy-enforcement point checks it once the
//!   template is known ([`Need::Composition`]). A tag operation is metadata of
//!   the EHR's data and is held to every template.
//! - The DEMOGRAPHIC API has no family: a client whose `ACTOR` the CDR grants
//!   `access_demographic` ([`Need::Demographic`]), or a caller holding the
//!   product scope, reaches it. A patient's own credential also writes a
//!   standalone `PARTY_RELATIONSHIP` ([`Need::Representation`]), which the
//!   party protection holds to a representation of the patient. The ADMIN API needs the product scope, except the outbox
//!   feed, which needs [`OUTBOX_SCOPE`] alone, so a feed reader reaches no
//!   other admin route and no admin caller reads the feed by default.
//! - A `patient/` grant reaches only the EHR of the token's launch context
//!   (`master07 §Context Selection`): an EHR route's `{ehr_id}`, a query's
//!   single-EHR `ehr_id`; an operation that names no EHR is refused to it.
//! - A route the table does not know is refused, so a route mounted without a
//!   classification fails closed.

use std::collections::HashMap;

use ferrohealth_smart::guard::Requirement;
use ferrohealth_smart::openehr;
use ferrohealth_smart::openehr::smart_scopes::{Permission, ResourceFamily};
use http::{HeaderMap, Method, Uri};
use openehr_its::rest::decode::query_component_octets;

use crate::api::operation::{CompositionGate, Operation, operation};
use crate::extensions::access::authn::Principal;

/// The scope the outbox feed needs: SMART App Launch 2.2.0, Scopes and Launch
/// Context, §Extensibility gives an extension scope the form "any string
/// starting with `__`", as the product scope `__ferroehr.platform` has it.
pub(crate) const OUTBOX_SCOPE: &str = "__ferroehr.outbox";

/// When a route's requirement is checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Need {
    /// Checked by the authentication middleware, before the handler runs.
    Now(Requirement),
    /// A COMPOSITION operation needing `permission` on the `composition-`
    /// family, checked by the policy-enforcement point once the template is
    /// known.
    Composition(Permission),
    /// The DEMOGRAPHIC API: met by a client the CDR grants
    /// `access_demographic`, read from the caller's grants, and otherwise by
    /// the requirement held, the product scope.
    Demographic(Requirement),
    /// A write of a standalone `PARTY_RELATIONSHIP`: met as
    /// [`Need::Demographic`] is, and by the patient's own credential, whose
    /// write the party protection holds to a representation of the patient,
    /// the authorisation of a proxy.
    Representation(Requirement),
}

/// The generated ITS-REST group an operation belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    /// The EHR group.
    Ehr,
    /// The DEFINITION group.
    Definition,
    /// The DEMOGRAPHIC group.
    Demographic,
    /// The QUERY group.
    Query,
    /// The ADMIN group.
    Admin,
    /// The SYSTEM group.
    System,
}

/// The requirement table of one deployment.
#[derive(Debug)]
pub(crate) struct SmartRoutes {
    /// The generated operation and group of each `(method, full path)`.
    ops: HashMap<(Method, String), (&'static str, Group)>,
    /// The API namespaces.
    namespaces: crate::api::Namespaces,
    /// The product scope (`[auth] product_scope`).
    product_scope: String,
}

impl SmartRoutes {
    /// Returns the table for an API under `base_path`, with the product scope
    /// the deployment configured.
    pub(crate) fn new(namespaces: &crate::api::Namespaces, product_scope: &str) -> Self {
        let mut ops = HashMap::new();
        for (group, table) in [
            (Group::Ehr, openehr_its::rest::generated::ehr::ROUTES),
            (
                Group::Definition,
                openehr_its::rest::generated::definition::ROUTES,
            ),
            (
                Group::Demographic,
                openehr_its::rest::generated::demographic::ROUTES,
            ),
            (Group::Query, openehr_its::rest::generated::query::ROUTES),
            (Group::Admin, openehr_its::rest::generated::admin::ROUTES),
            (Group::System, openehr_its::rest::generated::system::ROUTES),
        ] {
            for (method, path, op) in table {
                let Ok(method) = method.parse::<Method>() else {
                    continue;
                };
                let full = format!("{}{}", namespaces.openehr, crate::api::normalize_path(path));
                ops.insert((method, full), (*op, group));
            }
        }
        Self {
            ops,
            namespaces: namespaces.clone(),
            product_scope: product_scope.to_owned(),
        }
    }

    /// Returns what a request with `method` on the matched route template
    /// `matched` needs; `path` is the request path the template matched.
    pub(crate) fn need(&self, method: &Method, matched: Option<&str>, path: &str) -> Need {
        let Some(matched) = matched else {
            return Need::Now(Requirement::Refused);
        };
        if let Some((op, group)) = self.ops.get(&(method.clone(), matched.to_owned())) {
            return self.need_of_op(op, *group, matched, path);
        }
        let relative = self.namespaces.relative(matched).unwrap_or(matched);
        if is_under(relative, "/demographic/party_relationship") && method != Method::GET {
            return Need::Representation(self.product());
        }
        if is_under(relative, "/demographic") {
            return self.demographic();
        }
        Need::Now(self.extension(method, relative, matched, path))
    }

    /// The requirement of a generated operation.
    fn need_of_op(&self, op: &str, group: Group, template: &str, path: &str) -> Need {
        let named = |name: &str| path_param(template, path, name);
        let row = operation(op);
        let permission = row.map_or(Permission::Read, Operation::permission);
        let requirement = match group {
            Group::Admin => self.product(),
            Group::Demographic => return self.demographic(),
            Group::System => Requirement::Caller,
            Group::Query => Requirement::OpenEhr {
                family: ResourceFamily::Aql,
                permission: Permission::Search,
                resource: named("qualified_query_name"),
            },
            Group::Definition if op.starts_with("definition_query_") => Requirement::OpenEhr {
                family: ResourceFamily::Aql,
                permission,
                resource: named("qualified_query_name"),
            },
            Group::Definition => Requirement::OpenEhr {
                family: ResourceFamily::Template,
                permission,
                resource: named("template_id"),
            },
            Group::Ehr if row.is_some_and(|row| row.composition != CompositionGate::None) => {
                return Need::Composition(permission);
            }
            Group::Ehr => ehr_data(permission),
        };
        Need::Now(requirement)
    }

    /// The requirement of an extension route, outside the generated tables,
    /// by its path relative to the API base.
    fn extension(
        &self,
        method: &Method,
        relative: &str,
        template: &str,
        path: &str,
    ) -> Requirement {
        let under = |root: &str| is_under(relative, root);
        if under("/admin/outbox") {
            return Requirement::Scope(OUTBOX_SCOPE.to_owned());
        }
        if under("/admin") {
            return self.product();
        }
        if under("/definition/archetype") || under("/definition/artefact") {
            // NOTE: master08 §Resource Scopes names no archetype family; an
            // archetype builds templates, so it is held to the template- family.
            return Requirement::OpenEhr {
                family: ResourceFamily::Template,
                permission: permission_of_method(method),
                resource: None,
            };
        }
        if under("/definition/template") {
            // NOTE: master08 §Resource Scopes, the template conversion reads the stored
            // template, so it carries the template- requirement of the released reads.
            return Requirement::OpenEhr {
                family: ResourceFamily::Template,
                permission: permission_of_method(method),
                resource: path_param(template, path, "template_id"),
            };
        }
        if under("/definition/query") {
            return Requirement::OpenEhr {
                family: ResourceFamily::Aql,
                permission: permission_of_method(method),
                resource: path_param(template, path, "qualified_query_name"),
            };
        }
        if under("/message/export") {
            return ehr_data(Permission::Read);
        }
        if under("/message/import") {
            return ehr_data(Permission::Create);
        }
        if under("/query/cohort") {
            return Requirement::OpenEhr {
                family: ResourceFamily::Aql,
                permission: Permission::Search,
                resource: None,
            };
        }
        if under("/terminology") || under("/fhir") {
            return Requirement::Caller;
        }
        if under("/ehr") && relative.split('/').any(|segment| segment == "access_log") {
            // NOTE: our own design (#5130), the access log has its own access control apart
            // from any clinical grant (the draft's recording rule 8), which its handler applies.
            return Requirement::Caller;
        }
        if under("/ehr") && relative.ends_with("/access-delay/release") {
            // NOTE: EHDS Art. 3(3), releasing a held composition changes what the person
            // reads of it, so it needs the `u` grant its patient-scope check (an update) names.
            return ehr_data(Permission::Update);
        }
        if under("/ehr") && path_param(template, path, "ehr_id").is_some() {
            return ehr_data(permission_of_method(method));
        }
        Requirement::Refused
    }

    /// The product scope's requirement.
    fn product(&self) -> Requirement {
        Requirement::Scope(self.product_scope.clone())
    }

    /// The DEMOGRAPHIC API's need: a client the CDR grants
    /// `access_demographic`, or a caller holding the product scope.
    fn demographic(&self) -> Need {
        Need::Demographic(self.product())
    }
}

/// Whether the API-relative path `relative` is `root` or below it.
fn is_under(relative: &str, root: &str) -> bool {
    relative
        .strip_prefix(root)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// The requirement on an EHR's data that names no template: the
/// `composition-` family over every template.
const fn ehr_data(permission: Permission) -> Requirement {
    Requirement::OpenEhr {
        family: ResourceFamily::Composition,
        permission,
        resource: None,
    }
}

/// The permission letter of an extension route, by its method.
fn permission_of_method(method: &Method) -> Permission {
    match *method {
        Method::POST => Permission::Create,
        Method::PUT | Method::PATCH => Permission::Update,
        Method::DELETE => Permission::Delete,
        _ => Permission::Read,
    }
}

/// The value of the `{name}` segment of `template` in `path`, decoded.
fn path_param(template: &str, path: &str, name: &str) -> Option<String> {
    let wanted = format!("{{{name}}}");
    template
        .split('/')
        .zip(path.split('/'))
        .find(|(segment, _value)| *segment == wanted)
        .and_then(|(_segment, value)| urlencoding::decode(value).ok())
        .map(std::borrow::Cow::into_owned)
}

/// The EHR a request addresses, for the `patient/` confinement: a query's
/// single-EHR scope (the `ehr_id` parameter or the `openehr-ehr-id` header,
/// ITS-REST `query/Request.md` §About the `ehr_id` parameter), otherwise the
/// route's `{ehr_id}` segment.
#[must_use]
pub(crate) fn target_ehr(
    requirement: &Requirement,
    matched: Option<&str>,
    uri: &Uri,
    headers: &HeaderMap,
) -> Option<String> {
    if let Requirement::OpenEhr {
        family: ResourceFamily::Aql,
        ..
    } = requirement
    {
        return uri
            .query()
            .and_then(|query| {
                query.split('&').find_map(|pair| {
                    let (key, value) = pair.split_once('=')?;
                    (key == "ehr_id")
                        .then(|| String::from_utf8(query_component_octets(value)).ok())?
                })
            })
            .or_else(|| {
                headers
                    .get("openehr-ehr-id")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned)
            });
    }
    matched.and_then(|template| path_param(template, uri.path(), "ehr_id"))
}

/// Checks `requirement` for `principal`, confines a `patient/` grant to the EHR
/// of the launch context, and confines a bound representative to the EHR of
/// the subject it represents.
///
/// A requirement on openEHR data that only `patient/` grants cover reaches the
/// one EHR the token's `ehrId` names (`master07 §Context Selection`;
/// `master08 §Resource Scopes`: "access is restricted to data within that
/// patient's EHR"), so `target` must name it.
///
/// A representative reaches only the EHR whose `EHR_STATUS` names the subject
/// it represents ([`Principal::represented`], #5086) on the EHR's data and its
/// queries, whatever its grants cover; a template is no patient's data.
///
/// A read under a federation agreement whose `consent` condition applies
/// needs the consent decision reference for that patient (Wabvpz Art. 1(j) and
/// Art. 15a(1), `docs/law/nl/wabvpz/text.html`): the token's mapped
/// `consent.reference`, which its `patient/` grants confine to that patient.
///
/// # Errors
/// The reason the debug log records: the grants do not meet the requirement,
/// a `patient/` grant addresses another EHR or none, a representative
/// addresses an EHR other than the represented subject's, or a
/// consent-conditioned read carries no consent decision reference.
pub(crate) fn check(
    requirement: &Requirement,
    principal: &Principal,
    target: Option<&str>,
) -> Result<(), &'static str> {
    if !requirement.met_by(&principal.access, principal.honoured) {
        return Err("the grants do not meet the route's requirement");
    }
    if let (
        Requirement::OpenEhr {
            family: ResourceFamily::Composition | ResourceFamily::Aql,
            ..
        },
        Some(represented),
    ) = (requirement, &principal.represented)
        && !target
            .zip(represented.ehr_id.as_deref())
            .is_some_and(|(target, ehr)| target.eq_ignore_ascii_case(ehr))
    {
        return Err("a representative reaches only the EHR of the subject it represents");
    }
    if let (Requirement::OpenEhr { permission, .. }, Some(agreement)) = (
        requirement,
        crate::extensions::access::authn::declared::admitted(&principal.access),
    ) && agreement.consent.is_some()
        && principal
            .access
            .caller()
            .and_then(ferrohealth_smart::resource::Caller::consent_reference)
            .is_none()
        && matches!(permission, Permission::Read | Permission::Search)
    {
        return Err("a consent-conditioned read carries no consent decision reference");
    }
    if let Requirement::OpenEhr {
        family,
        permission,
        resource,
    } = requirement
        && let Some(caller) = principal.access.caller()
    {
        let covering = openehr::covering(
            &caller.openehr_scopes(),
            *family,
            *permission,
            resource.as_deref(),
            principal.honoured,
        );
        // NOTE: a contact point's caller carries no launch context; the EHR of its TRC
        // patient stands in for it (eHDSI XCA Profile 8.0.1 §4.2.1).
        if openehr::confined(&covering)
            && !crate::extensions::access::authn::trc::is_bound(&principal.access)
        {
            match (target, principal.access.ehr_id()) {
                (Some(target), Some(launched)) if target == launched => {}
                _ => return Err("a patient grant reaches only the EHR of the launch context"),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
