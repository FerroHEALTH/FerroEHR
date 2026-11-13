// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A SCIM Group and the `GROUP` that holds it, both ways.
//!
//! A Group's `displayName` is its `GROUP`'s `PARTY_IDENTITY`, its `externalId`
//! a `DV_IDENTIFIER` under the client's group issuer
//! ([`ferroehr::config::scim::group_issuer`]): the `group_identifier_issuer` of the
//! `[[auth.party]]` table whose `identifier_issuer` is the client's
//! `binding_issuer`, so the token's group claim of that client's people
//! matches it, or one derived from the `binding_issuer` when the table sets
//! none. Each member is served with its
//! `value`, the member User's `id`, its `$ref` and its `type`, and no
//! `display`, which RFC 7643 §4.2 leaves optional. No openEHR spec governs
//! SCIM: our own design.

use ferroehr::ids::VoId;
use ferroehr::party_binding::provisioned::GroupClient;
use ferroehr::service::demographic::group_provision::{GroupState, GroupWriter, ProvisionedGroup};
use serde_json::{Map, Value, json};

use crate::config::AppConfig;
use crate::scim::endpoint::{ScimClient, schemas};
use crate::scim::protocol::error::{ScimError, ScimType};
use crate::scim::protocol::etag::EntityTag;
use crate::scim::protocol::resource::{Container, Resource};
use crate::scim::protocol::urn;

/// The Group mapping of a deployment.
#[derive(Debug, Clone)]
pub(crate) struct GroupMap {
    namespace: String,
    max_members: usize,
}

impl GroupMap {
    /// The Group mapping of `config`, or `None` when the party mapping maps
    /// no group.
    ///
    /// # Errors
    /// The refusal naming what a mapped group lacks: the group template's
    /// `display_name` or the membership archetype.
    pub(crate) fn new(
        config: &AppConfig,
        scim: &ferroehr::config::scim::ScimConfig,
    ) -> Result<Option<Self>, String> {
        let binding = &config.party_binding;
        if !schemas::groups_mapped(binding) {
            return Ok(None);
        }
        let group = binding.group.as_ref().ok_or(
            "SCIM writes a Group as a GROUP of [authz.party_binding.group], which the mapping \
             does not name beside its membership archetype",
        )?;
        if group.display_name.is_none() {
            return Err(format!(
                "SCIM writes a Group's displayName at the display_name of \
                 [authz.party_binding.group], which the group template {:?} does not name",
                group.template_id
            ));
        }
        if binding.membership.is_none() {
            return Err(
                "SCIM writes each member of a Group as a PARTY_RELATIONSHIP of \
                 [authz.party_binding.membership], which the mapping does not name"
                    .to_owned(),
            );
        }
        Ok(Some(Self {
            namespace: binding.namespace.clone(),
            max_members: usize::try_from(scim.max_group_members).unwrap_or(usize::MAX),
        }))
    }

    /// The writer of `client`'s Groups.
    pub(crate) fn writer<'a>(&'a self, client: &'a ScimClient) -> GroupWriter<'a> {
        GroupWriter {
            client: client_of(client),
            namespace: &self.namespace,
            max_members: self.max_members,
        }
    }
}

/// Who `client`'s Group queries ask for.
pub(crate) fn client_of(client: &ScimClient) -> GroupClient<'_> {
    GroupClient {
        binding_issuer: &client.binding_issuer,
        group_issuer: &client.group_issuer,
    }
}

/// The full representation of `group` (RFC 7643 §4.2, §3.1), its members
/// and `meta` under `base_url`.
pub(crate) fn representation(group: &ProvisionedGroup, base_url: &str) -> Value {
    let mut object = Map::new();
    object.insert("schemas".to_owned(), json!([urn::CORE_GROUP]));
    object.insert("id".to_owned(), Value::from(group.row.party.to_string()));
    if let Some(external_id) = &group.row.external_id {
        object.insert("externalId".to_owned(), Value::from(external_id.clone()));
    }
    object.insert(
        "displayName".to_owned(),
        Value::from(group.row.display_name.clone()),
    );
    let members: Vec<Value> = group
        .members
        .iter()
        .map(|member| {
            json!({
                "value": member.to_string(),
                "$ref": format!("{base_url}/Users/{member}"),
                "type": "User",
            })
        })
        .collect();
    object.insert("members".to_owned(), Value::Array(members));
    let mut meta = Map::new();
    meta.insert("resourceType".to_owned(), Value::from("Group"));
    meta.insert("created".to_owned(), Value::from(group.created.to_string()));
    meta.insert(
        "lastModified".to_owned(),
        Value::from(group.last_modified.to_string()),
    );
    meta.insert(
        "location".to_owned(),
        Value::from(format!("{base_url}/Groups/{}", group.row.party)),
    );
    if let Some(tag) = entity_tag(group) {
        meta.insert("version".to_owned(), Value::from(tag.to_string()));
    }
    object.insert("meta".to_owned(), Value::Object(meta));
    Value::Object(object)
}

/// The entity tag of `group`: its version as a weak tag (RFC 7644 §3.14).
pub(crate) fn entity_tag(group: &ProvisionedGroup) -> Option<EntityTag> {
    EntityTag::weak(group.version_uid.clone()).ok()
}

/// What the Group `resource` asks to hold.
///
/// # Errors
/// A `400` `invalidValue` when `displayName` is missing or not a string, the
/// `externalId` is not a string, or a member names a Group (by its `type` or
/// `$ref`) or no id at all: nested Groups are refused.
pub(crate) fn state(resource: &Resource) -> Result<GroupState, ScimError> {
    let display_name = match resource.get(Container::Core, "displayName") {
        Some(Value::String(name)) if !name.trim().is_empty() => name.clone(),
        _ => {
            return Err(ScimError::bad_request(
                ScimType::InvalidValue,
                "A Group carries a displayName (RFC 7643 §4.2).",
            ));
        }
    };
    let external_id = match resource.get(Container::Core, "externalId") {
        None | Some(Value::Null) => None,
        Some(Value::String(id)) => Some(id.clone()),
        Some(_) => {
            return Err(ScimError::bad_request(
                ScimType::InvalidValue,
                "A Group's externalId is a string.",
            ));
        }
    };
    let listed = resource
        .get(Container::Core, "members")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut members = Vec::with_capacity(listed.len());
    for member in listed {
        members.push(member_id(member)?);
    }
    Ok(GroupState {
        display_name,
        external_id,
        members,
    })
}

/// The User container the member value `member` names.
fn member_id(member: &Value) -> Result<VoId, ScimError> {
    let names_group = member
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind.eq_ignore_ascii_case("Group"))
        || member
            .get("$ref")
            .and_then(Value::as_str)
            .is_some_and(|reference| reference.contains("/Groups/"));
    if names_group {
        return Err(ScimError::bad_request(
            ScimType::InvalidValue,
            "A Group's member is a User: nested Groups are not supported.",
        ));
    }
    let value = member.get("value").and_then(Value::as_str).unwrap_or("");
    // NOTE: a User's id is its container's UUID, so another value names no User, as an unknown id.
    value
        .parse::<uuid::Uuid>()
        .map(VoId)
        .map_err(|_error| unknown_member(value))
}

/// The refusal of a member naming no User of the client, the same for an
/// unknown id, a Group and another client's User.
fn unknown_member(value: &str) -> ScimError {
    ScimError::bad_request(
        ScimType::InvalidValue,
        format!("The member {value} is no User of this client."),
    )
}
