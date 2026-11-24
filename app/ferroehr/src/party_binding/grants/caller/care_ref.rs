// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The identifiers a care relationship names beside its provider: the carer
//! it covers for and the episode of care it is scoped to (draft access
//! control package, Care Relationships and Episodes of Care).

use openehr_rm::draft::access_control::terminology::Term as _;
use openehr_rm::draft::access_control::terminology::care::CareElement;
use openehr_rm::v1_2::data_types::basic::dv_identifier::DvIdentifier;
use uuid::Uuid;

use super::care_of::{CarerKind, ProviderKey, named_element};
use crate::party_binding::derive::container;
use crate::versioning::episode::EpisodeKey;
use crate::versioning::tree::Tree;

/// The episode of care a care relationship is scoped to: the `issuer` and
/// `id` of its `episode` `DV_IDENTIFIER`, and their [`EpisodeKey`] (draft
/// access control package, Episodes of Care: `_type_` is not compared).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Episode {
    /// The key a placement is matched by.
    pub key: EpisodeKey,
    /// The identifier's `issuer`.
    pub issuer: String,
    /// The identifier's `id`.
    pub id: String,
}

impl Episode {
    /// The episode `issuer` gives the identifier `id`.
    #[must_use]
    pub fn new(issuer: impl Into<String>, id: impl Into<String>) -> Self {
        let (issuer, id) = (issuer.into(), id.into());
        Self {
            key: EpisodeKey::of(&issuer, &id),
            issuer,
            id,
        }
    }
}

/// The carer a care relationship covers for, as the caller fact keeps it:
/// the `issuer` and `id` of its `covers_for` `DV_IDENTIFIER` and the kind its
/// `type` names.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Cover {
    /// The identifier's `issuer` and `id`.
    pub key: ProviderKey,
    /// What the carer is, as the identifier's `type` names it.
    pub kind: CarerKind,
}

impl Cover {
    /// The carer `key` names, of `kind`.
    #[must_use]
    pub fn new(key: ProviderKey, kind: CarerKind) -> Self {
        Self { key, kind }
    }
}

/// The carer a care relationship covers for, as its `covers_for` names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CoversFor {
    /// The identifier, as the caller fact stores it.
    pub(crate) cover: Cover,
    /// The container its `id` names.
    pub(crate) carer: Uuid,
}

/// The carer the care relationship `body` covers for: its `covers_for`
/// `DV_IDENTIFIER`, in the provider's form, its `issuer` the namespace of the
/// carer's reference, its `id` the carer's container and its `type` the
/// carer's kind; `None` when it names none (draft access control package,
/// Care Relationships). Whether that container is held, and as that kind, is
/// judged at the commit ([`crate::versioning::party_refs`]).
///
/// # Errors
/// Why it cannot be read: a `covers_for` that is no `DV_IDENTIFIER` with an
/// issuer, a container id and the type of a carer.
pub(crate) fn covers_for<'a, T: Tree<'a>>(body: T) -> Result<Option<CoversFor>, String> {
    let Some(value) = named_element(body, CareElement::CoversFor.code()) else {
        return Ok(None);
    };
    let identifier: DvIdentifier = openehr_its::json::from_canonical_value(&value.to_value())
        .map_err(|error| format!("its covers_for is no DV_IDENTIFIER: {error}"))?;
    let issuer = identifier.issuer.filter(|issuer| !issuer.trim().is_empty());
    let kind = identifier.r#type.as_deref().and_then(CarerKind::of);
    match (issuer, container(&identifier.id), kind) {
        (Some(issuer), Some(carer), Some(kind)) => Ok(Some(CoversFor {
            cover: Cover::new(ProviderKey::new(issuer, identifier.id), kind),
            carer,
        })),
        _ => Err(
            "its covers_for names the carer it replaces by the namespace of its reference as \
             the issuer, its container as the id and its kind (PERSON, ROLE, GROUP or \
             ORGANISATION) as the type"
                .to_owned(),
        ),
    }
}

/// The episode of care the care relationship `body` is scoped to: its
/// `episode` `DV_IDENTIFIER`, with an `issuer` and an `id`; `None` when it
/// names none (draft access control package, Episodes of Care).
///
/// # Errors
/// Why it cannot be read: an `episode` that is no `DV_IDENTIFIER` with a
/// non-blank issuer and id.
pub(crate) fn episode<'a, T: Tree<'a>>(body: T) -> Result<Option<Episode>, String> {
    let Some(value) = named_element(body, CareElement::Episode.code()) else {
        return Ok(None);
    };
    let identifier: DvIdentifier = openehr_its::json::from_canonical_value(&value.to_value())
        .map_err(|error| format!("its episode is no DV_IDENTIFIER: {error}"))?;
    match identifier.issuer.filter(|issuer| !issuer.trim().is_empty()) {
        Some(issuer) if !identifier.id.trim().is_empty() => {
            Ok(Some(Episode::new(issuer, identifier.id)))
        }
        _ => Err("its episode names the episode of care by an issuer and an id".to_owned()),
    }
}
