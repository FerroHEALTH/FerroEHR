// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The authenticated-committer request context.
//!
//! Every CONTRIBUTION carries a committer (`AUDIT_DETAILS.committer` `1..1`,
//! RM common `master04-generic_package` §Audit Details): when a request is
//! authenticated, the audit is attributed to that principal; an internal or
//! unauthenticated write falls back to the system identity. The protocol
//! adapter publishes the identity for the request's task scope via
//! [`with_committer`] and the platform reads it via [`current_committer`] —
//! the transport (a task-local) is spec-silent: no openEHR spec governs this
//! — our own design.

use openehr_base::prelude::{GenericId, ObjectId, PartyRef};
use openehr_rm::prelude::{PartyProxy, PartySelf};
use tokio::task_local;

/// The authenticated identity a protocol adapter attributes commits to.
#[derive(Debug, Clone)]
pub struct CommitterIdentity {
    /// The authenticated subject (user name / token subject).
    pub subject: String,
    /// The `DV_IDENTIFIER.type` discriminator for the identity's origin
    /// (`"basic"` / `"oauth2"`).
    pub id_type: &'static str,
    /// Who issued [`Self::subject`], when the authenticating party is not this
    /// server — the validated token issuer for an OAuth2/OIDC principal
    /// (`iss`). `None` for a locally-held credential (Basic), whose issuer IS
    /// this deployment.
    ///
    /// NOTE: no openEHR spec governs the value — our own design/extension.
    /// `DV_IDENTIFIER.issuer` is the "authority which issues the kind of id
    /// used in the id field of this object" (RM `data_types`
    /// `UML/classes/org.openehr.rm.data_types.dv_identifier.adoc` §Attributes);
    /// for a subject minted by an identity provider that authority is the
    /// provider, not this CDR, and which concrete string names it is spec-silent.
    pub issuer: Option<String>,
    /// The client the request came through, which the `[origins]` register
    /// maps to the origin a direct commit is stamped with: the Basic user, or
    /// an access token's `client_id`, failing that its `azp` (RFC 9068 §2.2).
    pub client: Option<String>,
    /// Whether the identity is the record's person, or their representative,
    /// writing through a patient-context credential: a commit it makes is
    /// marked as inserted by the person, and it may alter only what such a
    /// commit inserted (EHDS Art. 5, `docs/law/eu/ehds/text.html`).
    pub person: bool,
}

task_local! {
    static REQUEST_COMMITTER: Option<CommitterIdentity>;
}

/// The committer identity published for the current request, if any.
///
/// Outside a request scope (background tasks, tests) this is `None` and the
/// audit falls back to the system identity.
#[must_use]
pub fn current_committer() -> Option<CommitterIdentity> {
    REQUEST_COMMITTER.try_with(Clone::clone).ok().flatten()
}

/// Whether the current request's committer is a person writing in their own
/// EHR ([`CommitterIdentity::person`]); `false` outside a request.
#[must_use]
pub fn person_commits() -> bool {
    current_committer().is_some_and(|identity| identity.person)
}

/// The committer of a write a person makes in their own EHR: `PARTY_SELF`, its
/// `external_ref` naming the login that committed it.
///
/// RM common master04 §Audit Details: committer is a `PARTY_PROXY`, "allowing
/// for `PARTY_SELF` to be used when the committer is the record subject", and
/// a `PARTY_PROXY.external_ref` is a reference to "identification information
/// for this party, in an external system" (RM common
/// `UML/classes/org.openehr.rm.common.party_proxy.adoc`). This is how EHDS Art.
/// 5 makes what a person inserts "clearly distinguishable".
// NOTE: no openEHR spec governs which `PARTY_REF` names a login; ours is
// namespace `local`, type `PERSON`, a `GENERIC_ID` of the subject in its issuer's scheme.
#[must_use]
pub fn person_committer(identity: CommitterIdentity) -> PartyProxy {
    PartyProxy::PartySelf(PartySelf {
        external_ref: Some(PartyRef {
            namespace: "local".to_owned(),
            r#type: "PERSON".to_owned(),
            id: ObjectId::GenericId(GenericId {
                value: identity.subject,
                scheme: identity
                    .issuer
                    .unwrap_or_else(|| crate::service::ehr::meta::LOCAL_ISSUER.to_owned()),
            }),
        }),
    })
}

/// The committer a commit records: the person's own [`person_committer`] when
/// the request's committer is a person ([`person_commits`]), else `supplied`.
///
/// A person's committer is the server's statement, so whatever committer the
/// client supplied is replaced (EHDS Art. 5: what the person inserts is
/// "clearly distinguishable"); our own design, as no openEHR spec governs it.
#[must_use]
pub fn attributed(supplied: PartyProxy) -> PartyProxy {
    match current_committer() {
        Some(identity) if identity.person => person_committer(identity),
        _ => supplied,
    }
}

/// Run `fut` with `identity` published as the request's committer.
pub async fn with_committer<F>(identity: Option<CommitterIdentity>, fut: F) -> F::Output
where
    F: Future,
{
    REQUEST_COMMITTER.scope(identity, fut).await
}

/// The committer a write records when the request supplied none: the
/// authenticated caller, or the system identity when there is none.
///
/// A person writing in their own EHR is their [`person_committer`]. Any other
/// caller is a `PARTY_IDENTIFIED` named by its subject, with one
/// `DV_IDENTIFIER` whose `issuer` is the token issuer for a Bearer caller and
/// this deployment's `ferroehr` for a Basic one ([`CommitterIdentity::issuer`]).
/// `DV_IDENTIFIER.issuer` is the "authority which issues the kind of id used
/// in the id field of this object" (RM `data_types`
/// `UML/classes/org.openehr.rm.data_types.dv_identifier.adoc` §Attributes), and
/// `AUDIT_DETAILS.committer` is `1..1` (RM common master04 §Audit Details).
/// Every default committer the server writes, in the service and in a protocol
/// adapter, comes from here.
// NOTE: no openEHR spec governs which string names the issuing authority; our own design.
#[must_use]
pub fn default_committer(identity: Option<CommitterIdentity>) -> PartyProxy {
    let party = match identity {
        Some(identity) if identity.person => return person_committer(identity),
        Some(identity) => openehr_rm::prelude::PartyIdentifiedData {
            external_ref: None,
            name: Some(identity.subject.clone()),
            identifiers: Some(openehr_base::containers::NonEmptyVec::of(
                openehr_rm::prelude::DvIdentifier {
                    issuer: Some(
                        identity
                            .issuer
                            .unwrap_or_else(|| crate::service::ehr::meta::LOCAL_ISSUER.to_owned()),
                    ),
                    assigner: None,
                    id: identity.subject,
                    r#type: Some(identity.id_type.to_owned()),
                },
            )),
        },
        None => openehr_rm::prelude::PartyIdentifiedData {
            external_ref: None,
            name: Some(crate::service::SYSTEM_COMMITTER_NAME.to_owned()),
            identifiers: None,
        },
    };
    PartyProxy::PartyIdentified(openehr_rm::prelude::PartyIdentified::PartyIdentified(party))
}

/// The default committer of the current request ([`default_committer`] over
/// [`current_committer`]).
#[must_use]
pub fn request_committer() -> PartyProxy {
    default_committer(current_committer())
}

#[cfg(test)]
mod tests {
    use super::{CommitterIdentity, default_committer};
    use openehr_rm::prelude::{PartyIdentified, PartyProxy};

    fn caller(id_type: &'static str, issuer: Option<&str>) -> CommitterIdentity {
        CommitterIdentity {
            subject: "alice".to_owned(),
            id_type,
            issuer: issuer.map(str::to_owned),
            client: None,
            person: false,
        }
    }

    /// The `(issuer, id, type)` of the one identifier a default committer carries.
    fn identifier(committer: &PartyProxy) -> (Option<&str>, &str, Option<&str>) {
        let PartyProxy::PartyIdentified(PartyIdentified::PartyIdentified(party)) = committer else {
            panic!("a default committer is a PARTY_IDENTIFIED: {committer:?}");
        };
        let ids = party.identifiers.as_ref().expect("one identifier");
        assert_eq!(ids.len(), 1);
        let id = ids.iter().next().expect("one identifier");
        (id.issuer.as_deref(), id.id.as_str(), id.r#type.as_deref())
    }

    #[test]
    fn a_basic_caller_is_issued_by_this_deployment() {
        let committer = default_committer(Some(caller("basic", None)));
        assert_eq!(
            identifier(&committer),
            (Some("ferroehr"), "alice", Some("basic"))
        );
    }

    #[test]
    fn a_bearer_caller_is_issued_by_its_token_issuer() {
        let committer = default_committer(Some(caller("oauth2", Some("https://idp.example"))));
        assert_eq!(
            identifier(&committer),
            (Some("https://idp.example"), "alice", Some("oauth2"))
        );
    }

    #[test]
    fn no_caller_is_the_system_identity() {
        let PartyProxy::PartyIdentified(PartyIdentified::PartyIdentified(party)) =
            default_committer(None)
        else {
            panic!("the system committer is a PARTY_IDENTIFIED");
        };
        assert_eq!(
            party.name.as_deref(),
            Some(crate::service::SYSTEM_COMMITTER_NAME)
        );
        assert!(party.identifiers.is_none());
    }
}
