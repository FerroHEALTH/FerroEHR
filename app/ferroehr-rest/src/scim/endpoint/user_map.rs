// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The SCIM User as the shared party mapping holds it: which data value of
//! the `PERSON` each attribute is, both ways.
//!
//! `userName`, `externalId` and the binding key (`loginIdentifier`) are the
//! `DV_IDENTIFIER`s of their selectors, issued by the client's
//! `binding_issuer`, so a client's key, `userName` and `externalId` are told
//! apart from another client's. Every other attribute
//! `[[authz.party_binding.person.attribute]]` maps is the data value at its
//! path: a `DV_TEXT`, a `DV_URI`, or a `DV_IDENTIFIER` of the attribute's
//! name. `active` is the lifecycle state (RM common
//! `master06-change_control_package.adoc` §Version Lifecycle): `complete` and
//! `incomplete` are active. No openEHR spec governs a mapping from SCIM to a
//! demographic template: our own design.

use ferroehr::config::binding::PartyBindingConfig;
use ferroehr::config::binding::person::{PersonAttribute, Selector};
use ferroehr::error::ServiceError;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::mapped_write::mapped_value;
use ferroehr::service::demographic::provision::ProvisionedUser;
use serde_json::{Map, Value};

use crate::scim::endpoint::schemas::{self, EXTENSION_URN, LIFECYCLE_STATE, MISSING_PATHS};
use crate::scim::protocol::error::{ScimError, ScimType};
use crate::scim::protocol::etag::EntityTag;
use crate::scim::protocol::path::AttrPath;
use crate::scim::protocol::resource::{Container, Resource};
use crate::scim::protocol::urn;

/// Where a mapped attribute's value sits in a User.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    /// The attribute `name` of `schema` (the core schema when `None`), or its
    /// sub-attribute `sub`.
    Plain {
        schema: Option<String>,
        name: String,
        sub: Option<String>,
    },
    /// The sub-attribute `sub` of the value of the multi-valued `name` whose
    /// `type` is `kind` (RFC 7644 §3.4.2.2 `valuePath`, `emails[type eq
    /// "work"].value`).
    Typed {
        name: String,
        kind: String,
        sub: String,
    },
}

/// One mapped attribute: where it sits in a User, and its path per
/// template.
#[derive(Debug, Clone)]
struct Mapped {
    target: Target,
    /// The attribute's last name, the `DV_IDENTIFIER.type` it is written as.
    identifier_type: String,
    /// The mapping table, with its path per template.
    attribute: PersonAttribute,
}

/// The User mapping of a deployment.
#[derive(Debug, Clone)]
pub(crate) struct UserMap {
    template_id: String,
    role_template: String,
    namespace: String,
    key: Selector,
    user_name: Selector,
    external_id: Option<Selector>,
    attributes: Vec<Mapped>,
}

impl UserMap {
    /// The User mapping of `binding`.
    ///
    /// # Errors
    /// The refusal naming what the mapping lacks: a default person template
    /// with a `user_name` selector, the professional role, or a mapped SCIM
    /// path this mapping reads.
    pub(crate) fn new(binding: &PartyBindingConfig) -> Result<Self, String> {
        let template = binding
            .person()
            .template
            .iter()
            .find(|template| template.default)
            .ok_or("SCIM creates Users with the default [[authz.party_binding.person.template]], and none sets default = true")?;
        let user_name = template.user_name.clone().ok_or_else(|| {
            format!(
                "SCIM writes userName at the user_name selector, which the default template {:?} \
                 does not name",
                template.template_id
            )
        })?;
        let role_template = binding
            .professional_role
            .as_ref()
            .map(|role| role.template_id.clone())
            .ok_or("SCIM creates each User's professional ROLE, so it needs [authz.party_binding.professional_role]")?;
        let mut attributes = Vec::new();
        for attribute in &binding.person().attribute {
            let Some(scim) = attribute.scim.as_deref() else {
                continue;
            };
            if scim == ferroehr::config::binding::person::LOGIN_IDENTIFIER {
                continue;
            }
            let target = target(scim).ok_or_else(|| {
                format!("the SCIM attribute {scim:?} is no attribute path this endpoint maps")
            })?;
            let identifier_type = match &target {
                Target::Plain { name, sub, .. } => sub.clone().unwrap_or_else(|| name.clone()),
                Target::Typed { name, .. } => name.clone(),
            };
            attributes.push(Mapped {
                target,
                identifier_type,
                attribute: attribute.clone(),
            });
        }
        Ok(Self {
            template_id: template.template_id.clone(),
            role_template,
            namespace: binding.namespace.clone(),
            key: template.key.clone(),
            user_name,
            external_id: template.external_id.clone(),
            attributes,
        })
    }

    /// The template new Users' `PERSON`s are created with.
    pub(crate) fn template_id(&self) -> &str {
        &self.template_id
    }

    /// The template of the professional `ROLE`.
    pub(crate) fn role_template(&self) -> &str {
        &self.role_template
    }

    /// The `PARTY_REF.namespace` the `PERSON` and `ROLE` name each other in.
    pub(crate) fn namespace(&self) -> &str {
        &self.namespace
    }

    /// The data value of each mapped path of a new User's `PERSON`, from the
    /// create body `resource` of the client of `issuer`.
    ///
    /// # Errors
    /// A `400` `invalidValue` when the body carries no `loginIdentifier`, a
    /// mapped value is not a string, or the template draws no data value a
    /// text fills at a mapped path; a storage failure as a `500`.
    pub(crate) async fn values(
        &self,
        service: &FerroEhrService,
        resource: &Resource,
        issuer: &str,
    ) -> Result<Vec<(String, Value)>, ScimError> {
        let login = text(resource.get(
            Container::Extension(EXTENSION_URN),
            schemas::LOGIN_IDENTIFIER,
        ))?
        .ok_or_else(|| {
            ScimError::bad_request(
                ScimType::InvalidValue,
                format!(
                    "A User carries the {}:{} its login binds by.",
                    EXTENSION_URN,
                    schemas::LOGIN_IDENTIFIER
                ),
            )
        })?;
        let user_name = text(resource.get(Container::Core, "userName"))?.ok_or_else(|| {
            ScimError::bad_request(ScimType::InvalidValue, "A User carries a 'userName'.")
        })?;
        let mut wanted = vec![
            (
                self.key.path.clone(),
                login,
                self.key.identifier_type.clone(),
            ),
            (
                self.user_name.path.clone(),
                user_name,
                self.user_name.identifier_type.clone(),
            ),
        ];
        if let Some(external_id) = text(resource.get(Container::Core, "externalId"))? {
            let selector = self.external_id.as_ref().ok_or_else(|| {
                ScimError::bad_request(
                    ScimType::InvalidValue,
                    "This provider maps no 'externalId'.",
                )
            })?;
            wanted.push((
                selector.path.clone(),
                external_id,
                selector.identifier_type.clone(),
            ));
        }
        for mapped in &self.attributes {
            let Some(path) = mapped.attribute.path_for(&self.template_id) else {
                continue;
            };
            if let Some(value) = text(read(resource, &mapped.target))? {
                wanted.push((path.to_owned(), value, mapped.identifier_type.clone()));
            }
        }
        let mut values = Vec::with_capacity(wanted.len());
        for (path, value, identifier_type) in wanted {
            let data = service
                .mapped_text_value(&self.template_id, &path, &value, (issuer, &identifier_type))
                .await
                .map_err(|error| service_refusal(&error))?;
            values.push((path, data));
        }
        Ok(values)
    }

    /// The mapped values to set and the mapped paths to clear in the
    /// `PERSON` `body` of the client of `issuer`, for its User to go from
    /// `old` to `new`.
    ///
    /// # Errors
    /// A `400` `mutability` when the binding key changes; the
    /// [`Self::values`] refusals of a new value.
    pub(crate) async fn changes(
        &self,
        service: &FerroEhrService,
        body: &Value,
        (old, new): (&Resource, &Resource),
        issuer: &str,
    ) -> Result<
        (
            Vec<(String, Value)>,
            Vec<(String, Option<(String, String)>)>,
        ),
        ScimError,
    > {
        let ours = Container::Extension(EXTENSION_URN);
        if text(old.get(ours, schemas::LOGIN_IDENTIFIER))?
            != text(new.get(ours, schemas::LOGIN_IDENTIFIER))?
        {
            return Err(ScimError::bad_request(
                ScimType::Mutability,
                "The binding key is immutable (RFC 7643 §2.2).",
            ));
        }
        let template = template_of(body);
        let mut set = Vec::new();
        let mut clear = Vec::new();
        let mut selectors = vec![("userName", &self.user_name)];
        if let Some(external_id) = &self.external_id {
            selectors.push(("externalId", external_id));
        }
        for (name, selector) in selectors {
            let (before, after) = (
                text(old.get(Container::Core, name))?,
                text(new.get(Container::Core, name))?,
            );
            let slot = (selector.identifier_type.clone(), issuer.to_owned());
            match after {
                Some(after) if Some(&after) != before.as_ref() => {
                    set.push((selector.path.clone(), after, slot.0));
                }
                None if before.is_some() => clear.push((selector.path.clone(), Some(slot))),
                _ => {}
            }
        }
        for mapped in &self.attributes {
            let Some(path) = mapped.attribute.path_for(template) else {
                continue;
            };
            let (before, after) = (
                text(read(old, &mapped.target))?,
                text(read(new, &mapped.target))?,
            );
            match after {
                Some(after) if Some(&after) != before.as_ref() => {
                    set.push((path.to_owned(), after, mapped.identifier_type.clone()));
                }
                None if before.is_some() => {
                    let slot = (mapped.identifier_type.clone(), issuer.to_owned());
                    let identifier = mapped_value(body, path, Some((&slot.0, &slot.1)))
                        .is_some()
                        .then_some(slot);
                    clear.push((path.to_owned(), identifier));
                }
                _ => {}
            }
        }
        let mut values = Vec::with_capacity(set.len());
        for (path, value, identifier_type) in set {
            let data = service
                .mapped_text_value(template, &path, &value, (issuer, &identifier_type))
                .await
                .map_err(|error| service_refusal(&error))?;
            values.push((path, data));
        }
        Ok((values, clear))
    }

    /// The representation of `user` of the client of `issuer` (RFC 7643
    /// §8.1), with its `meta` naming it under `base_url` (RFC 7643 §3.1).
    pub(crate) fn representation(
        &self,
        user: &ProvisionedUser,
        base_url: &str,
        issuer: &str,
    ) -> Value {
        let mut attributes = Map::new();
        let mut schemas = vec![Value::from(urn::CORE_USER)];
        attributes.insert("id".to_owned(), Value::from(user.row.party.to_string()));
        if let Some(external_id) = &user.row.external_id {
            attributes.insert("externalId".to_owned(), Value::from(external_id.clone()));
        }
        if let Some(user_name) = &user.row.user_name {
            attributes.insert("userName".to_owned(), Value::from(user_name.clone()));
        }
        let state = user.row.lifecycle_state.as_str();
        attributes.insert(
            "active".to_owned(),
            Value::Bool(matches!(state, "532" | "553")),
        );
        let template = template_of(&user.body);
        for mapped in &self.attributes {
            let Some(text) = mapped
                .attribute
                .path_for(template)
                .and_then(|path| {
                    attribute_value(&user.body, path, (&mapped.identifier_type, issuer))
                })
                .and_then(served_text)
            else {
                continue;
            };
            write(&mut attributes, &mut schemas, &mapped.target, text);
        }
        let mut ours = Map::new();
        ours.insert(
            schemas::LOGIN_IDENTIFIER.to_owned(),
            Value::from(user.row.login_identifier.clone()),
        );
        if let Some(named) = state_name(state) {
            ours.insert(LIFECYCLE_STATE.to_owned(), Value::from(named));
        }
        if !user.missing.is_empty() {
            ours.insert(MISSING_PATHS.to_owned(), Value::from(user.missing.clone()));
        }
        attributes.insert(EXTENSION_URN.to_owned(), Value::Object(ours));
        schemas.push(Value::from(EXTENSION_URN));
        attributes.insert("meta".to_owned(), meta(user, base_url));
        let mut body = Map::new();
        body.insert("schemas".to_owned(), Value::Array(schemas));
        body.extend(attributes);
        Value::Object(body)
    }
}

/// The template the `PERSON` `body` names.
fn template_of(body: &Value) -> &str {
    body.pointer("/archetype_details/template_id/value")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

/// The data value of a mapped attribute at `path` in the `PERSON` `body`:
/// the identifier of its `slot` (type and issuer) when the path holds one,
/// and otherwise the first value that is no identifier, so a path the
/// selectors share never serves another attribute's identifier.
fn attribute_value<'a>(body: &'a Value, path: &str, slot: (&str, &str)) -> Option<&'a Value> {
    mapped_value(body, path, Some(slot)).or_else(|| {
        mapped_value(body, path, None)
            .filter(|value| value.get("_type").and_then(Value::as_str) != Some("DV_IDENTIFIER"))
    })
}

/// The entity tag of `user`: its version as a weak tag (RFC 7644 §3.14).
pub(crate) fn entity_tag(user: &ProvisionedUser) -> Option<EntityTag> {
    EntityTag::weak(user.version_uid.clone()).ok()
}

/// The `meta` of `user` (RFC 7643 §3.1).
fn meta(user: &ProvisionedUser, base_url: &str) -> Value {
    let mut meta = Map::new();
    meta.insert("resourceType".to_owned(), Value::from("User"));
    meta.insert("created".to_owned(), Value::from(user.created.to_string()));
    meta.insert(
        "lastModified".to_owned(),
        Value::from(user.last_modified.to_string()),
    );
    meta.insert(
        "location".to_owned(),
        Value::from(format!("{base_url}/Users/{}", user.row.party)),
    );
    if let Some(tag) = entity_tag(user) {
        meta.insert("version".to_owned(), Value::from(tag.to_string()));
    }
    Value::Object(meta)
}

/// The name FerroEHR's extension gives a lifecycle state that is not
/// `complete`.
fn state_name(code: &str) -> Option<&'static str> {
    match code {
        "553" => Some("incomplete"),
        "800" => Some("inactive"),
        "801" => Some("abandoned"),
        _ => None,
    }
}

/// The target a mapped SCIM attribute path names, or `None` when it is no
/// attribute path or `valuePath` of the `type eq` form.
fn target(scim: &str) -> Option<Target> {
    if let Some((name, rest)) = scim.split_once('[') {
        let (filter, sub) = rest.split_once("].")?;
        let mut words = filter.split_whitespace();
        let (attribute, op, value) = (words.next()?, words.next()?, words.next()?);
        if words.next().is_some()
            || !attribute.eq_ignore_ascii_case("type")
            || !op.eq_ignore_ascii_case("eq")
        {
            return None;
        }
        let kind = value.strip_prefix('"')?.strip_suffix('"')?;
        return Some(Target::Typed {
            name: name.to_owned(),
            kind: kind.to_owned(),
            sub: sub.to_owned(),
        });
    }
    let path = AttrPath::parse(scim).ok()?;
    Some(Target::Plain {
        schema: path.schema,
        name: path.name.as_str().to_owned(),
        sub: path.sub_attr.map(|sub| sub.as_str().to_owned()),
    })
}

/// The container a schema URN's attributes sit in.
fn container(schema: Option<&str>) -> Container<'_> {
    match schema {
        Some(schema) if !urn::same(schema, urn::CORE_USER) => Container::Extension(schema),
        _ => Container::Core,
    }
}

/// The value at `target` in `resource`.
fn read<'a>(resource: &'a Resource, target: &Target) -> Option<&'a Value> {
    match target {
        Target::Plain { schema, name, sub } => {
            let value = resource.get(container(schema.as_deref()), name)?;
            match sub {
                Some(sub) => value.get(sub),
                None => Some(value),
            }
        }
        Target::Typed { name, kind, sub } => resource
            .get(Container::Core, name)?
            .as_array()?
            .iter()
            .find(|item| {
                item.get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|t| t.eq_ignore_ascii_case(kind))
            })?
            .get(sub),
    }
}

/// Writes `text` at `target` in `attributes`, adding an extension's URN to
/// `schemas`.
fn write(
    attributes: &mut Map<String, Value>,
    schemas: &mut Vec<Value>,
    target: &Target,
    text: String,
) {
    match target {
        Target::Plain { schema, name, sub } => {
            let holder = match container(schema.as_deref()) {
                Container::Core => &mut *attributes,
                Container::Extension(urn) => {
                    if !schemas.iter().any(|s| s.as_str() == Some(urn)) {
                        schemas.push(Value::from(urn));
                    }
                    let Value::Object(holder) = attributes
                        .entry(urn.to_owned())
                        .or_insert_with(|| Value::Object(Map::new()))
                    else {
                        return;
                    };
                    holder
                }
            };
            match sub {
                None => {
                    holder.insert(name.clone(), Value::from(text));
                }
                Some(sub) => {
                    if let Value::Object(complex) = holder
                        .entry(name.clone())
                        .or_insert_with(|| Value::Object(Map::new()))
                    {
                        complex.insert(sub.clone(), Value::from(text));
                    }
                }
            }
        }
        Target::Typed { name, kind, sub } => {
            if let Value::Array(values) = attributes
                .entry(name.clone())
                .or_insert_with(|| Value::Array(Vec::new()))
            {
                let mut value = Map::new();
                value.insert("type".to_owned(), Value::from(kind.clone()));
                value.insert(sub.clone(), Value::from(text));
                values.push(Value::Object(value));
            }
        }
    }
}

/// The text a SCIM string attribute serves of the data value `value`: a
/// `DV_TEXT`'s or a `DV_URI`'s `value`, a `DV_IDENTIFIER`'s `id`.
fn served_text(value: &Value) -> Option<String> {
    let field = match value.get("_type").and_then(Value::as_str)? {
        "DV_IDENTIFIER" => "id",
        "DV_TEXT" | "DV_CODED_TEXT" | "DV_URI" | "DV_EHR_URI" => "value",
        _ => return None,
    };
    value.get(field).and_then(Value::as_str).map(str::to_owned)
}

/// The string `value` holds, `None` when it is absent or null.
///
/// # Errors
/// A `400` `invalidValue` when it is not a string.
fn text(value: Option<&Value>) -> Result<Option<String>, ScimError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(ScimError::bad_request(
            ScimType::InvalidValue,
            "A mapped attribute holds a string.",
        )),
    }
}

/// The SCIM error a demographic refusal answers (RFC 7644 §3.12).
pub(crate) fn service_refusal(error: &ServiceError) -> ScimError {
    use http::StatusCode;
    match error {
        ServiceError::Conflict(_) => ScimError::uniqueness(error.to_string()),
        // NOTE: RFC 7644 §3.12 `detail` is human-readable, so a refused
        // content names the rule it breaks.
        ServiceError::Unprocessable { violation, .. } => {
            ScimError::bad_request(ScimType::InvalidValue, violation.to_string())
        }
        ServiceError::BadRequest(_) | ServiceError::ValidationFailed(_) => {
            ScimError::bad_request(ScimType::InvalidValue, error.to_string())
        }
        ServiceError::NotFound(_) => ScimError::new(StatusCode::NOT_FOUND, "Resource not found."),
        ServiceError::Forbidden(_) => ScimError::new(StatusCode::FORBIDDEN, error.to_string()),
        ServiceError::VersionConflict(_) => ScimError::new(
            StatusCode::PRECONDITION_FAILED,
            "The version does not match.",
        ),
        _ => {
            tracing::error!(
                error = %ferroehr::error::ErrorChain::new(error),
                "a SCIM request failed in the demographic service"
            );
            ScimError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "The provider could not complete the request.",
            )
        }
    }
}
