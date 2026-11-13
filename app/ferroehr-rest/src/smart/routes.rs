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
        if under("/ehr") && relative.ends_with("/access-delay/release") {
            // NOTE: EHDS Art. 3(3), releasing a held composition changes what the person
            // reads of it, so it needs the `u` grant its ABAC check (an update) names.
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
mod tests {
    #![expect(
        clippy::disallowed_types,
        reason = "a test builds a token's claim map directly; the claim set is RFC 7519-open"
    )]

    use std::sync::Arc;

    use ferrohealth_smart::resource::{Access, Caller, Honoured};

    use super::*;

    const BASE: &str = "/ferroehr/rest/openehr/v1";

    /// The permission letter `op` exercises, `r` for an operation without a row.
    fn permission_of_op(op: &str) -> Permission {
        operation(op).map_or(Permission::Read, Operation::permission)
    }

    fn routes() -> SmartRoutes {
        SmartRoutes::new(
            &crate::api::Namespaces::of(&ferroehr::config::rest::ApiConfig::default()),
            "__ferroehr.platform",
        )
    }

    fn principal(scope: &str, client: Option<&str>, honoured: Honoured) -> Principal {
        let mut claims = serde_json::Map::new();
        claims.insert("ehrId".to_owned(), serde_json::json!("ehr-1"));
        let caller = Caller::new(
            "alice".to_owned(),
            client.map(str::to_owned),
            scope,
            None,
            false,
            ferrohealth_smart::kind::CallerKind::Person,
        )
        .with_issuer("https://as.example".to_owned())
        .with_claims(claims);
        Principal::new(Access::Caller(Arc::new(caller)), Vec::new(), honoured)
    }

    /// Every generated ITS-REST operation carries a requirement: the family
    /// master08 §Resource Scopes names for it, the `composition-` family for
    /// the EHR's other data, the client list for the DEMOGRAPHIC group and the
    /// product scope for the ADMIN group. None is refused.
    #[test]
    fn every_generated_route_has_a_requirement() {
        use openehr_its::rest::generated::{admin, definition, demographic, ehr, query, system};
        let table = routes();
        for routes in [
            admin::ROUTES,
            definition::ROUTES,
            demographic::ROUTES,
            ehr::ROUTES,
            query::ROUTES,
            system::ROUTES,
        ] {
            for (method, path, op) in routes {
                let template = format!("{BASE}{}", crate::api::normalize_path(path));
                let method = method.parse::<Method>().expect("a method");
                let need = table.need(&method, Some(&template), &template);
                assert_ne!(need, Need::Now(Requirement::Refused), "{op}");
            }
        }
    }

    /// Every route the API router mounts, the extensions included, carries a
    /// requirement, so no route answers to a token that carries no grant for
    /// it merely because the table forgot it.
    #[test]
    fn every_mounted_route_has_a_requirement() {
        let table = routes();
        let document = crate::api::api_doc(&crate::api::Namespaces::of(
            &ferroehr::config::rest::ApiConfig::default(),
        ));
        for (path, item) in &document.paths.paths {
            let template = path.clone();
            for (method, operation) in [
                (Method::GET, item.get.as_ref()),
                (Method::PUT, item.put.as_ref()),
                (Method::POST, item.post.as_ref()),
                (Method::DELETE, item.delete.as_ref()),
                (Method::PATCH, item.patch.as_ref()),
            ] {
                if operation.is_none() {
                    continue;
                }
                let need = table.need(&method, Some(&template), &template);
                assert_ne!(
                    need,
                    Need::Now(Requirement::Refused),
                    "{method} {path} has no SMART on openEHR requirement"
                );
            }
        }
    }

    #[test]
    fn an_unknown_route_is_refused() {
        let table = routes();
        assert_eq!(
            table.need(&Method::GET, Some("/elsewhere"), "/elsewhere"),
            Need::Now(Requirement::Refused)
        );
        assert_eq!(
            table.need(&Method::GET, None, "/x"),
            Need::Now(Requirement::Refused)
        );
    }

    #[test]
    fn the_families_follow_the_operation() {
        let table = routes();
        let template = format!("{BASE}/definition/template/adl1.4/{{template_id}}");
        let path = format!("{BASE}/definition/template/adl1.4/Vitals.v1");
        assert_eq!(
            table.need(&Method::GET, Some(&template), &path),
            Need::Now(Requirement::OpenEhr {
                family: ResourceFamily::Template,
                permission: Permission::Read,
                resource: Some("Vitals.v1".to_owned()),
            })
        );
        // The ADL 2 conversion of a stored template, an extension route, is
        // held to the requirement of the released template read.
        let ext = "/ferroehr/rest/extensions/v1";
        let template = format!("{ext}/definition/template/adl1.4/{{template_id}}/adl2");
        let path = format!("{ext}/definition/template/adl1.4/Vitals.v1/adl2");
        assert_eq!(
            table.need(&Method::GET, Some(&template), &path),
            Need::Now(Requirement::OpenEhr {
                family: ResourceFamily::Template,
                permission: Permission::Read,
                resource: Some("Vitals.v1".to_owned()),
            })
        );
        let template = format!("{BASE}/ehr/{{ehr_id}}/composition/{{uid_based_id}}");
        assert_eq!(
            table.need(&Method::GET, Some(&template), &template),
            Need::Composition(Permission::Read)
        );
        let template = format!("{BASE}/ehr/{{ehr_id}}/ehr_status");
        assert_eq!(
            table.need(&Method::PUT, Some(&template), &template),
            Need::Now(ehr_data(Permission::Update))
        );
    }

    #[test]
    fn permission_mapping() {
        assert_eq!(permission_of_op("composition_create"), Permission::Create);
        assert_eq!(permission_of_op("composition_get"), Permission::Read);
        assert_eq!(permission_of_op("composition_update"), Permission::Update);
        assert_eq!(permission_of_op("composition_delete"), Permission::Delete);
        assert_eq!(
            permission_of_op("query_execute_stored_query"),
            Permission::Search
        );
        assert_eq!(
            permission_of_op("definition_template_adl1.4_upload"),
            Permission::Create
        );
        assert_eq!(
            permission_of_op("definition_query_store.yaml"),
            Permission::Create
        );
    }

    #[test]
    fn a_user_grant_meets_its_family_and_not_another() {
        let alice = principal("user/composition-*.r", None, Honoured::default());
        assert!(check(&ehr_data(Permission::Read), &alice, Some("ehr-9")).is_ok());
        assert!(check(&ehr_data(Permission::Create), &alice, Some("ehr-9")).is_err());
        let query = Requirement::OpenEhr {
            family: ResourceFamily::Aql,
            permission: Permission::Search,
            resource: None,
        };
        assert!(check(&query, &alice, None).is_err());
    }

    /// master07 §Context Selection: a `patient/` grant counts only for a bound
    /// issuer, and only on the EHR the token's `ehrId` names.
    #[test]
    fn a_patient_grant_reaches_the_launch_context_ehr_alone() {
        let bound = Honoured {
            backend: false,
            patient: true,
        };
        let patient = principal("patient/composition-*.r", None, bound);
        assert!(check(&ehr_data(Permission::Read), &patient, Some("ehr-1")).is_ok());
        assert!(check(&ehr_data(Permission::Read), &patient, Some("ehr-2")).is_err());
        assert!(check(&ehr_data(Permission::Read), &patient, None).is_err());
        let unbound = principal("patient/composition-*.r", None, Honoured::default());
        assert!(check(&ehr_data(Permission::Read), &unbound, Some("ehr-1")).is_err());
        // A user/ grant beside it widens the reach (the union of the grants).
        let both = principal("patient/composition-*.r user/composition-*.r", None, bound);
        assert!(check(&ehr_data(Permission::Read), &both, Some("ehr-2")).is_ok());
    }

    /// A subject granted `access_without_scope` whose token carries no
    /// resource scope meets every SMART on openEHR requirement; one without
    /// it, or one whose token carries resource scopes, gets only its scopes.
    #[test]
    fn a_scopeless_subject_meets_an_openehr_requirement_only_without_scopes() {
        let caller = |scope: &str, scopeless: bool| {
            let caller = Caller::new(
                "svc".to_owned(),
                None,
                scope,
                None,
                scopeless,
                ferrohealth_smart::kind::CallerKind::System,
            );
            Principal::new(
                Access::Caller(Arc::new(caller)),
                Vec::new(),
                Honoured::default(),
            )
        };
        let read = ehr_data(Permission::Read);
        assert!(check(&read, &caller("openid", true), Some("ehr-1")).is_ok());
        assert!(check(&read, &caller("openid", false), Some("ehr-1")).is_err());
        assert!(
            check(&read, &caller("user/template-*.r", true), Some("ehr-1")).is_err(),
            "a token carrying scopes gets only its scopes"
        );
        assert!(
            check(&routes().product(), &caller("openid", true), None).is_err(),
            "the allowlist reaches no product-scope surface"
        );
    }

    /// The DEMOGRAPHIC API has no family: a client granted
    /// `access_demographic` or the product scope reaches it, and no SMART
    /// grant does. The grant arm reads the caller's grants
    /// (`authn::gates::smart_refusal`); the requirement held is the product
    /// scope's, which no client name meets.
    #[test]
    fn the_demographic_api_needs_a_granted_client_or_the_product_scope() {
        let table = routes();
        let ext = "/ferroehr/rest/extensions/v1";
        let none = Honoured::default();
        for template in [
            format!("{BASE}/demographic/person/{{uid_based_id}}"),
            format!("{ext}/demographic/party/{{party_id}}/relationships"),
        ] {
            let Need::Demographic(requirement) =
                table.need(&Method::GET, Some(&template), &template)
            else {
                panic!("a demographic route reads the caller's grants: {template}");
            };
            assert_eq!(requirement, table.product());
            assert!(
                check(
                    &requirement,
                    &principal("__ferroehr.platform", None, none),
                    None
                )
                .is_ok()
            );
            for client in ["mpi", "app"] {
                assert!(
                    check(
                        &requirement,
                        &principal("system/composition-*.cruds", Some(client), none),
                        None
                    )
                    .is_err(),
                    "no client name meets it: {client}"
                );
            }
        }
    }

    /// The patient writes a standalone relationship; reading one is the DEMOGRAPHIC API's.
    #[test]
    fn a_patient_writes_a_standalone_relationship_and_reads_none() {
        let create = "/ferroehr/rest/extensions/v1/demographic/party_relationship".to_owned();
        let (table, one) = (routes(), format!("{create}/{{uid_based_id}}"));
        for (method, path) in [
            (Method::POST, &create),
            (Method::PUT, &one),
            (Method::DELETE, &one),
        ] {
            let need = table.need(&method, Some(path), path);
            assert_eq!(need, Need::Representation(table.product()), "{method}");
        }
        let read = table.need(&Method::GET, Some(&one), &one);
        assert!(matches!(read, Need::Demographic(_)));
    }

    /// The outbox feed needs its own scope: the product scope does not reach it,
    /// and the outbox scope reaches no other admin route.
    #[test]
    fn the_outbox_feed_needs_the_outbox_scope_alone() {
        let table = routes();
        let ext = "/ferroehr/rest/extensions/v1";
        let none = Honoured::default();
        let reader = principal(OUTBOX_SCOPE, Some("ferrobridge"), none);
        let platform = principal("__ferroehr.platform", None, none);
        for (method, template) in [
            (Method::GET, format!("{ext}/admin/outbox/{{reader}}")),
            (Method::POST, format!("{ext}/admin/outbox/{{reader}}/ack")),
        ] {
            let Need::Now(requirement) = table.need(&method, Some(&template), &template) else {
                panic!("the feed is checked at once");
            };
            assert_eq!(requirement, Requirement::Scope(OUTBOX_SCOPE.to_owned()));
            assert!(check(&requirement, &reader, None).is_ok());
            assert!(check(&requirement, &platform, None).is_err());
        }
        let report = format!("{ext}/admin/report/statistics");
        let Need::Now(requirement) = table.need(&Method::GET, Some(&report), &report) else {
            panic!("an admin route is checked at once");
        };
        assert!(
            check(&requirement, &reader, None).is_err(),
            "the outbox scope reaches no other admin route"
        );
    }

    /// master08 §Resource Scopes: `system/aql-*` "would grant access to all
    /// registered and ad-hoc AQL queries system-wide", so it counts only for a
    /// listed backend client.
    #[test]
    fn a_system_wide_aql_grant_counts_for_a_backend_client_alone() {
        let query = Requirement::OpenEhr {
            family: ResourceFamily::Aql,
            permission: Permission::Search,
            resource: None,
        };
        let backend = Honoured {
            backend: true,
            patient: false,
        };
        assert!(
            check(
                &query,
                &principal("system/aql-*.s", Some("etl"), backend),
                None
            )
            .is_ok()
        );
        assert!(
            check(
                &query,
                &principal("system/aql-*.s", Some("app"), Honoured::default()),
                None
            )
            .is_err()
        );
    }

    #[test]
    fn the_query_target_is_the_single_ehr_scope() {
        let query = Requirement::OpenEhr {
            family: ResourceFamily::Aql,
            permission: Permission::Search,
            resource: None,
        };
        let uri: Uri = "/x/query/aql?q=SELECT&ehr_id=ehr-7".parse().expect("uri");
        assert_eq!(
            target_ehr(&query, None, &uri, &HeaderMap::new()).as_deref(),
            Some("ehr-7")
        );
        let mut headers = HeaderMap::new();
        headers.insert("openehr-ehr-id", http::HeaderValue::from_static("ehr-8"));
        let bare: Uri = "/x/query/aql".parse().expect("uri");
        assert_eq!(
            target_ehr(&query, None, &bare, &headers).as_deref(),
            Some("ehr-8")
        );
    }
}
