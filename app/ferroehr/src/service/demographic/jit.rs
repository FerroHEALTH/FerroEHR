// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Just-in-time creation: the provisional `PERSON` a first login with no
//! party creates, performing the professional `ROLE`, in one demographic
//! CONTRIBUTION.
//!
//! The `PERSON` is committed `553|incomplete|`, which RM common
//! `master06-change_control_package.adoc` §Incomplete Content allows for
//! "incomplete demographic details", with the change type `249|creation|`,
//! a description naming just-in-time creation and the issuer, and the
//! person's own committer: their name, their login identifier and an
//! `external_ref` naming the new `PERSON`, whose id is allocated before the
//! commit. The binding key row is written in the same transaction, so the
//! binding side table's unique key index makes two first requests of one
//! login commit one `PERSON`. When a login creates a party is our own design.

use openehr_base::prelude::{HierObjectId, ObjectId, PartyRef};
use openehr_rm::prelude::DvIdentifier;

use crate::error::ServiceError;
use crate::ids::VoId;
use crate::service::FerroEhrService;
use crate::service::demographic::provision::NewUser;
use crate::versioning::lifecycle;

/// The `DV_IDENTIFIER.type` of a committer's login identifier read from an
/// access token, as the protocol adapter names an OAuth 2.0 principal.
const LOGIN_TYPE: &str = "oauth2";

/// A provisional `PERSON` to create at a first login.
///
/// The `Debug` rendering names the templates and the issuers, never a value.
#[derive(Clone)]
pub struct ProvisionalPerson {
    /// The default person template.
    pub template_id: String,
    /// The professional `ROLE` template.
    pub role_template: String,
    /// The `PARTY_REF.namespace` the `PERSON`, its `ROLE` and its committer
    /// name each other in.
    pub namespace: String,
    /// The access token's issuer, which the description and the committer's
    /// login identifier name.
    pub login_issuer: String,
    /// The `identifier_issuer` of the issuer's `[[auth.party]]` table, which
    /// issues the binding key and the `userName`.
    pub key_issuer: String,
    /// The binding key: its selector's path, the key claim's value and the
    /// selector's `DV_IDENTIFIER.type`.
    pub key: (String, String, String),
    /// Each other mapped path with its text and the `DV_IDENTIFIER.type` an
    /// identifier there is written as: the `userName`, the names and every
    /// attribute the mapping marks for just-in-time creation.
    pub texts: Vec<(String, String, String)>,
    /// The committer's name.
    pub name: Option<String>,
    /// The healthcare provider the person acts for, as `(issuer, id)`.
    pub provider: Option<(String, String)>,
}

impl std::fmt::Debug for ProvisionalPerson {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProvisionalPerson")
            .field("template_id", &self.template_id)
            .field("role_template", &self.role_template)
            .field("login_issuer", &self.login_issuer)
            .field("key_issuer", &self.key_issuer)
            .finish_non_exhaustive()
    }
}

impl FerroEhrService {
    /// Creates the provisional `PERSON` `person` with its professional
    /// `ROLE` in one demographic CONTRIBUTION, returning the `PERSON`'s
    /// container.
    ///
    /// The write is the one the key's issuer owns
    /// ([`crate::party_binding::protect::provisioning`]), so a SCIM client
    /// of that `binding_issuer` completes the `PERSON` later.
    ///
    /// # Errors
    /// A `422` of a text the template draws no data value for, or of a body
    /// that does not fit its template even incomplete; the `409` of a binding
    /// key or `userName` another live party holds; a storage failure.
    pub async fn create_provisional_person(
        &self,
        person: ProvisionalPerson,
    ) -> Result<VoId, ServiceError> {
        let (key_path, key, key_type) = &person.key;
        let login =
            crate::party_binding::derive::Identifier::new(&person.key_issuer, key_type, key)
                .canonical();
        let mut values = Vec::with_capacity(person.texts.len() + 1);
        for (path, text, identifier_type) in std::iter::once((key_path, key, key_type)).chain(
            person
                .texts
                .iter()
                .map(|(path, text, identifier_type)| (path, text, identifier_type)),
        ) {
            let value = self
                .mapped_text_value(
                    &person.template_id,
                    path,
                    text,
                    (&person.key_issuer, identifier_type),
                )
                .await?;
            values.push((path.clone(), value));
        }
        let user = NewUser {
            template_id: person.template_id,
            namespace: person.namespace.clone(),
            values,
        };
        let (id, parties) = self
            .person_with_role(
                &person.role_template,
                user,
                Some(lifecycle::state::INCOMPLETE),
            )
            .await?;
        // NOTE: the local source names each party by its container's UUID, so the
        // new container is the PERSON the binding binds the login to.
        let external_ref = HierObjectId::new(id.to_string()).ok().map(|uid| PartyRef {
            namespace: person.namespace,
            r#type: "PERSON".to_owned(),
            id: ObjectId::HierObjectId(uid),
        });
        let login = DvIdentifier {
            issuer: Some(person.login_issuer.clone()),
            assigner: None,
            id: login,
            r#type: Some(LOGIN_TYPE.to_owned()),
        };
        let committer = crate::versioning::committer::bound::professional(
            external_ref,
            person.name,
            login,
            person.provider,
        );
        let description = description(&person.login_issuer);
        Box::pin(crate::party_binding::protect::provisioning(
            person.key_issuer,
            self.create_parties(parties, &description, Some(committer)),
        ))
        .await?;
        Ok(id)
    }

    /// The provisional `PERSON`s created more than `days` days ago, with
    /// when each was created, oldest first.
    ///
    /// # Errors
    /// [`ServiceError`] on a storage failure.
    pub async fn provisional_persons(
        &self,
        days: u32,
    ) -> Result<Vec<(VoId, jiff::Timestamp)>, ServiceError> {
        let before = jiff::Timestamp::now()
            .saturating_sub(jiff::SignedDuration::from_hours(i64::from(days) * 24))
            .map_err(|e| ServiceError::internal("computing the report cut-off", e))?;
        let mut conn = crate::db::acquire::acquire(&self.demographic_pool).await?;
        Ok(crate::party_binding::provisioned::provisional_before(&mut conn, before).await?)
    }
}

/// The audit description of the just-in-time creation at a first login of
/// the issuer `login_issuer`, which the pre-flight's provenance rule reads
/// back ([`crate::party_binding::check`]).
#[must_use]
pub fn description(login_issuer: &str) -> String {
    format!("just-in-time creation at a first login of the issuer {login_issuer}")
}
