// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The composer of a COMPOSITION a bound professional commits (#6130).
//!
//! RM `COMPOSITION.composer` is "the person primarily responsible for the
//! content of the Composition" (`org.openehr.rm.composition.composition.adoc`),
//! and `AUDIT_DETAILS.committer` the "user who committed the item"
//! (`org.openehr.rm.common.audit_details.adoc`). The two differ when a scribe
//! records for a physician, so the server writes the committer
//! ([`super::bound::server_committer`]) and holds the composer: it names the
//! professional, by the judgement the committer gets ([`super::bound::judge`]),
//! or, under a `compose_for` grant, another `PERSON` of the provider the
//! professional acts for; a modify may also keep the composer of the version
//! it replaces, which claims nothing new while the committer records who
//! corrected it. The placeholder naming the caller is filled with
//! the professional's `PARTY_IDENTIFIED` before the body is rendered,
//! digested and stored; any other composer is kept as stated or refused,
//! never rewritten. EHDS Art. 13(4) (`docs/law/eu/ehds/text.html`) asks the
//! record to identify the professional and the provider. No openEHR spec
//! governs who may name whom as composer: the rule and its refusal are our
//! own design.

use openehr_base::prelude::ObjectId;
use openehr_rm::prelude::{PartyIdentified, PartyIdentifiedData, PartyProxy};
use sqlx::PgConnection;

use super::{CommitterIdentity, REQUEST_COMMITTER, bound};
use crate::error::ServiceError;
use crate::ids::VoId;
use crate::party_binding::grants::capabilities::Capabilities;
use crate::party_binding::grants::operation::Operation;
use crate::versioning::Kind;
use crate::versioning::body::Body;
use crate::versioning::object_version_id::TreeId;

/// What the judgement makes of a stated composer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    /// The composer is kept as stated.
    Keep,
    /// The placeholder is filled with the professional's `PARTY_IDENTIFIED`.
    Fill,
}

/// What [`settle`] leaves to decide.
#[derive(Debug)]
pub(crate) enum Settled {
    /// The composer is settled: filled, kept, or no rule applies.
    Done,
    /// The composer is the bound professional's to state only when it is the
    /// composer of the version a modify replaces ([`confirm_unchanged`]); a
    /// create is refused.
    UnlessUnchanged(Box<PartyProxy>),
}

/// Settles the composer of a COMPOSITION `body` of `kind` the request's
/// caller commits: for a bound professional the placeholder naming the
/// caller is filled, and a composer naming anyone else is admitted only
/// under a `compose_for` grant or, on a modify, when it is unchanged; any
/// other caller and kind pass unchanged.
///
/// # Errors
/// [`ServiceError::Forbidden`] (`403`) naming `COMPOSITION.composer` for a
/// composer that cannot be read.
pub(crate) fn settle(kind: Kind, body: &mut Body) -> Result<Settled, ServiceError> {
    if kind != Kind::Composition {
        return Ok(Settled::Done);
    }
    // NOTE: outside a request scope there is no caller to hold the composer to.
    REQUEST_COMMITTER
        .try_with(
            |identity| match identity.as_ref().filter(|i| professional(i)) {
                Some(identity) => settle_for(identity, body),
                None => Ok(Settled::Done),
            },
        )
        .unwrap_or(Ok(Settled::Done))
}

/// Admits the composer `stated` of a modify of `vo_id` when it equals the
/// composer of the version `preceding` it replaces.
///
/// # Errors
/// [`ServiceError::Forbidden`] (`403`) naming `COMPOSITION.composer` for any
/// other composer; the storage error of the read.
pub(crate) async fn confirm_unchanged(
    tx: &mut PgConnection,
    vo_id: VoId,
    preceding: TreeId,
    stated: &PartyProxy,
) -> Result<(), ServiceError> {
    let previous =
        crate::storage::version_repo::placement::composer_of(tx, vo_id, preceding.columns())
            .await?;
    // NOTE: a stored composer that does not read as a PARTY_PROXY is no composer
    // the modify keeps, so the refusal stands.
    let unchanged = previous
        .and_then(|text| openehr_its::json::from_canonical_json::<PartyProxy>(&text).ok())
        .is_some_and(|previous| previous == *stated);
    if unchanged { Ok(()) } else { Err(refused()) }
}

/// Whether `identity` is a professional bound to a `PERSON` through a token:
/// no person or representative in their own record, no system, external or
/// patient-portal caller, and no Basic user, which keeps its stated composer.
fn professional(identity: &CommitterIdentity) -> bool {
    identity.id_type == "oauth2"
        && identity.party.is_some()
        && !identity.person
        && !identity.representative
        && !identity.system
        && identity.external.is_none()
        && identity
            .bound
            .as_ref()
            .is_none_or(|details| details.portal.is_none())
}

/// [`settle`] for the bound professional `identity`.
fn settle_for(identity: &CommitterIdentity, body: &mut Body) -> Result<Settled, ServiceError> {
    let Some(ours) = bound::server_committer(identity) else {
        return Ok(Settled::Done);
    };
    let verdict = match body.composer() {
        // NOTE: RM common master06 §Incomplete Content, a 553|incomplete| body may state no
        // composer yet, and one that names nobody names no one else.
        None => return Ok(Settled::Done),
        Some(Err(_)) => return Err(refused()),
        Some(Ok(stated)) => match judge(&stated, &ours, identity, || {
            Capabilities::current().holds(Operation::ComposeFor)
        }) {
            Ok(verdict) => verdict,
            Err(_) => return Ok(Settled::UnlessUnchanged(Box::new(stated.into_owned()))),
        },
    };
    if verdict == Verdict::Fill {
        body.set_composer(ours);
    }
    Ok(Settled::Done)
}

/// Judges the composer `stated` against `ours`, the professional's
/// `PARTY_IDENTIFIED`, for the caller `identity`, asking `compose_for`
/// whether a `compose_for` grant is in force only when it decides.
///
/// The placeholder naming the caller is filled; a composer equal to ours,
/// or stating nothing ours does not, is kept; one naming another `PERSON`
/// of the caller's provider is kept under the grant.
fn judge(
    stated: &PartyProxy,
    ours: &PartyProxy,
    identity: &CommitterIdentity,
    compose_for: impl FnOnce() -> bool,
) -> Result<Verdict, ServiceError> {
    if stated == ours {
        return Ok(Verdict::Keep);
    }
    let (
        PartyProxy::PartyIdentified(PartyIdentified::PartyIdentified(party)),
        PartyProxy::PartyIdentified(PartyIdentified::PartyIdentified(own)),
    ) = (stated, ours)
    else {
        return Err(refused());
    };
    if bound::placeholder(party) {
        return if bound::names_caller(party, identity) {
            Ok(Verdict::Fill)
        } else {
            Err(refused())
        };
    }
    if bound::states_nothing_more(party, own)
        || (another_person(party, own, identity) && compose_for())
    {
        return Ok(Verdict::Keep);
    }
    Err(refused())
}

/// Whether `party` names, by its `external_ref`, a `PERSON` of the party
/// store the caller is bound in other than `own`'s, and names the caller's
/// provider: by its provider identifier when the caller acts for one, and by
/// none when it does not.
fn another_person(
    party: &PartyIdentifiedData,
    own: &PartyIdentifiedData,
    identity: &CommitterIdentity,
) -> bool {
    let Some(reference) = party.external_ref.as_deref() else {
        return false;
    };
    let details = identity.bound.as_ref();
    let person = reference.r#type == "PERSON"
        && details.is_some_and(|details| details.namespace == reference.namespace)
        && matches!(reference.id, ObjectId::HierObjectId(_))
        && own.external_ref.as_deref() != Some(reference);
    let provider = details.and_then(|details| details.provider.as_ref());
    let mut named = false;
    let consistent = party
        .identifiers
        .iter()
        .flat_map(|ids| ids.iter())
        .filter(|id| id.r#type.as_deref() == Some(bound::PROVIDER_TYPE))
        .all(|id| {
            named = true;
            provider.is_some_and(|(issuer, value)| {
                id.issuer.as_ref() == Some(issuer) && &id.id == value && id.assigner.is_none()
            })
        });
    person && consistent && named == provider.is_some()
}

/// The refusal of a composer the bound professional may not state.
pub(crate) fn refused() -> ServiceError {
    ServiceError::not_permitted(
        "COMPOSITION.composer: a bound professional's COMPOSITION names the professional as its \
         composer; send the placeholder naming the authenticated caller, which the server \
         fills, a composer that states nothing the server's does not, or, under a compose_for \
         grant, another PERSON of the same provider by its external_ref (RM COMPOSITION.composer; \
         EHDS Art. 13(4))",
    )
}

#[cfg(test)]
mod tests {
    use openehr_base::prelude::{HierObjectId, PartyRef};
    use openehr_rm::prelude::DvIdentifier;

    use super::*;
    use crate::versioning::committer::bound::BoundDetails;

    const ISSUER: &str = "https://login.example";
    const PROVIDER: (&str, &str) = ("urn:oid:2.999.1", "12345678");
    const OWN: &str = "0190a6b2-0000-7000-8000-000000000001";
    const OTHER: &str = "0190a6b2-0000-7000-8000-000000000002";

    fn identity(provider: bool) -> CommitterIdentity {
        CommitterIdentity {
            subject: "sub-a".to_owned(),
            id_type: "oauth2",
            issuer: Some(ISSUER.to_owned()),
            client: Some("app".to_owned()),
            person: false,
            party: None,
            representative: false,
            bound: Some(BoundDetails {
                name: Some("Dr Jane Roe".to_owned()),
                provider: provider.then(|| (PROVIDER.0.to_owned(), PROVIDER.1.to_owned())),
                namespace: "demographic".to_owned(),
                portal: None,
            }),
            system: false,
            agent: None,
            external: None,
        }
    }

    fn reference(id: &str) -> PartyRef {
        PartyRef {
            namespace: "demographic".to_owned(),
            r#type: "PERSON".to_owned(),
            id: ObjectId::HierObjectId(HierObjectId::new(id.to_owned()).expect("a uuid")),
        }
    }

    fn identifier(issuer: &str, id: &str, kind: Option<&str>) -> DvIdentifier {
        DvIdentifier {
            issuer: Some(issuer.to_owned()),
            assigner: None,
            id: id.to_owned(),
            r#type: kind.map(str::to_owned),
        }
    }

    fn party(
        external_ref: Option<PartyRef>,
        name: Option<&str>,
        identifiers: Vec<DvIdentifier>,
    ) -> PartyProxy {
        let mut ids = identifiers.into_iter();
        let identifiers = ids.next().map(|first| {
            let mut all = openehr_base::containers::NonEmptyVec::of(first);
            ids.for_each(|id| all.push(id));
            all
        });
        PartyProxy::PartyIdentified(PartyIdentified::PartyIdentified(PartyIdentifiedData {
            external_ref: external_ref.map(Box::new),
            name: name.map(str::to_owned),
            identifiers,
        }))
    }

    fn ours(provider: bool) -> PartyProxy {
        bound::professional(
            Some(reference(OWN)),
            Some("Dr Jane Roe".to_owned()),
            identifier(ISSUER, "key-a", Some("oauth2")),
            provider.then(|| (PROVIDER.0.to_owned(), PROVIDER.1.to_owned())),
        )
    }

    fn provider_id() -> DvIdentifier {
        identifier(PROVIDER.0, PROVIDER.1, Some(bound::PROVIDER_TYPE))
    }

    fn verdict(stated: &PartyProxy, provider: bool, granted: bool) -> Option<Verdict> {
        judge(stated, &ours(provider), &identity(provider), || granted).ok()
    }

    #[test]
    fn the_placeholder_naming_the_caller_is_filled() {
        let placeholder = party(None, None, vec![identifier(ISSUER, "sub-a", None)]);
        assert_eq!(verdict(&placeholder, true, false), Some(Verdict::Fill));
        let someone = party(None, None, vec![identifier(ISSUER, "sub-b", None)]);
        assert_eq!(verdict(&someone, true, true), None);
    }

    #[test]
    fn a_composer_stating_nothing_more_is_kept() {
        assert_eq!(verdict(&ours(true), true, false), Some(Verdict::Keep));
        for stated in [
            party(Some(reference(OWN)), None, Vec::new()),
            party(None, Some("Dr Jane Roe"), vec![provider_id()]),
            party(
                Some(reference(OWN)),
                Some("Dr Jane Roe"),
                vec![identifier(ISSUER, "key-a", None), provider_id()],
            ),
        ] {
            assert_eq!(
                verdict(&stated, true, false),
                Some(Verdict::Keep),
                "{stated:?}"
            );
        }
    }

    #[test]
    fn a_composer_saying_more_than_the_server_knows_is_refused() {
        for stated in [
            party(None, Some("Dr Fake"), Vec::new()),
            party(
                None,
                Some("Dr Jane Roe"),
                vec![identifier("urn:x", "9", None)],
            ),
            party(Some(reference(OWN)), Some("Dr Other"), Vec::new()),
            PartyProxy::PartySelf(openehr_rm::prelude::PartySelf { external_ref: None }),
        ] {
            assert_eq!(verdict(&stated, true, true), None, "{stated:?}");
        }
    }

    #[test]
    fn another_person_of_the_provider_takes_the_grant() {
        let colleague = party(Some(reference(OTHER)), Some("Dr B"), vec![provider_id()]);
        assert_eq!(verdict(&colleague, true, true), Some(Verdict::Keep));
        assert_eq!(verdict(&colleague, true, false), None);
        // The provider is named when the caller acts for one, and never another.
        let unnamed = party(Some(reference(OTHER)), Some("Dr B"), Vec::new());
        assert_eq!(verdict(&unnamed, true, true), None);
        let elsewhere = party(
            Some(reference(OTHER)),
            None,
            vec![identifier(PROVIDER.0, "99", Some(bound::PROVIDER_TYPE))],
        );
        assert_eq!(verdict(&elsewhere, true, true), None);
        // With no provider, the colleague names none.
        assert_eq!(verdict(&unnamed, false, true), Some(Verdict::Keep));
        assert_eq!(verdict(&colleague, false, true), None);
        // A reference to no PERSON of the bound namespace is no colleague.
        let mut foreign = reference(OTHER);
        foreign.namespace = "elsewhere".to_owned();
        assert_eq!(
            verdict(&party(Some(foreign), None, vec![provider_id()]), true, true),
            None
        );
        let mut organisation = reference(OTHER);
        organisation.r#type = "ORGANISATION".to_owned();
        assert_eq!(
            verdict(
                &party(Some(organisation), None, vec![provider_id()]),
                true,
                true
            ),
            None
        );
    }

    #[test]
    fn the_grant_is_asked_only_when_it_decides() {
        let asked = std::cell::Cell::new(false);
        let ask = || {
            asked.set(true);
            true
        };
        assert!(judge(&ours(true), &ours(true), &identity(true), ask).is_ok());
        assert!(!asked.get());
    }

    #[test]
    fn an_unbound_caller_is_not_held() {
        let mut caller = identity(true);
        assert!(
            !professional(&caller),
            "an unbound caller keeps its composer"
        );
        caller.system = true;
        assert!(!professional(&caller));
    }
}
