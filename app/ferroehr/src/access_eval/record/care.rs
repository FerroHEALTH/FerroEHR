// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The care relationship an access rested on, in the form the draft access
//! log names it: an `ACCESS_STORED_CARE_GROUND` for a stored relationship,
//! an `ACCESS_ATTESTED_CARE_GROUND` for one a verified credential attests
//! (draft access control decision procedure, Refusals; #6173).

use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_base::v1_3::base_types::identification::object_id::ObjectId;
use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
use openehr_base::v1_3::base_types::identification::party_ref::PartyRef;
use openehr_rm::draft::access_control::terminology::{Term as _, coded_text};
use openehr_rm::draft::access_log::access_log::access_attested_care_ground::AccessAttestedCareGround;
use openehr_rm::draft::access_log::access_log::access_stored_care_ground::AccessStoredCareGround;
use openehr_rm::v1_2::data_types::basic::dv_identifier::DvIdentifier;

use crate::access_eval::care_tie::Tie;
use crate::party_binding::grants::caller::attestation::Attestation;
use crate::party_binding::grants::caller::care_of::{CarerKind, ProviderKey};

/// The care relationship `tie` rests on as the access log names it: its
/// role, its path, its carer, the version that holds it, whether grace
/// admitted the access, the carer it covers for and its provider (draft
/// access control decision procedure, Refusals); `None` for a tie read
/// without its relationship. The carer is named under `namespace`, the one
/// this CDR names its parties under.
pub(super) fn care_ground(tie: &Tie, namespace: &str) -> Option<AccessStoredCareGround> {
    let grant = tie.grant.as_deref()?;
    let identifier = |key: &ProviderKey, r#type: Option<&str>| DvIdentifier {
        issuer: Some(key.issuer.clone()),
        assigner: None,
        id: key.id.clone(),
        r#type: r#type.map(str::to_owned),
    };
    Some(AccessStoredCareGround {
        role: coded_text(&tie.role.terminology, &tie.role.code, &tie.role.code),
        path: tie.path.coded(),
        carer: PartyRef {
            namespace: namespace.to_owned(),
            r#type: grant
                .carer_kind
                .map_or("PARTY", CarerKind::rm_type)
                .to_owned(),
            // NOTE: BASE HIER_OBJECT_ID, a UUID always reads as one.
            id: ObjectId::HierObjectId(HierObjectId::new(grant.carer.to_string()).ok()?),
        },
        relationship: grant
            .version
            .as_deref()
            .and_then(|version| version.parse::<ObjectVersionId>().ok()),
        in_grace: grant.in_grace,
        covers_for: grant
            .covers_for
            .as_ref()
            .map(|cover| identifier(&cover.key, Some(cover.kind.rm_type()))),
        provider: identifier(&grant.provider, Some("ORGANISATION")),
        episode: grant.episode.as_ref().map(|episode| DvIdentifier {
            issuer: Some(episode.issuer.clone()),
            assigner: None,
            id: episode.id.clone(),
            r#type: None,
        }),
    })
}

/// The attested relationship `tie` rests on as the access log names it: its
/// role, its path, the assertion of `attestation` and the agreement it was
/// attested under, named by the container of the agreement's `ROLE` and the
/// partner's federation issuer (draft decision procedure, Refusals).
pub(super) fn attested_ground(tie: &Tie, attestation: &Attestation) -> AccessAttestedCareGround {
    AccessAttestedCareGround {
        role: coded_text(&tie.role.terminology, &tie.role.code, &tie.role.code),
        path: tie.path.coded(),
        assertion: attestation.assertion_ref(),
        agreement: DvIdentifier {
            issuer: Some(attestation.agreement.issuer.clone()),
            assigner: None,
            id: attestation.agreement.role.to_string(),
            r#type: Some("ROLE".to_owned()),
        },
    }
}
