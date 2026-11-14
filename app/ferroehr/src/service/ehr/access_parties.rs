// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The parties an `EHR_ACCESS` directive names, checked where the directive
//! is committed.
//!
//! The settings name parties in their gate-keepers and in their directives.
//! A directive names parties in two places: the actor scopes it applies to
//! (`ACCESS_ACTOR_SCOPE.party`, a `PARTY_REF`) and the party who instructed
//! it (`instructed_by`, a `PARTY_IDENTIFIED` or `PARTY_RELATED` with an
//! `external_ref`), in the directive and in each of its exceptions. The
//! access evaluator matches a caller against those references by the id it
//! reads off them ([`party_key`]), so a reference that names no party of the
//! demographic store can never match anyone: the directive would hold for
//! nobody, and a gate-keeper would lock the settings to nobody. Each write
//! path therefore refuses it with `422`, naming the directive or gate-keeper
//! and the party: the CONTRIBUTION and the controller's commit
//! through [`FerroEhrService::validate_for_commit`], an EHR Extract import and
//! an archive load through [`FerroEhrService::check_directive_parties`] on
//! the head version, the one they put in force. A
//! `PARTY_SELF` names the EHR's subject, whom `EHR_STATUS.subject`
//! identifies, so it is not looked up. No openEHR spec governs the scheme or
//! this check: our own design (#5517).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 10): EHR_ACCESS.settings is the RM-mandated \
              open slot (RM ehr access_control_settings.adoc — abstract, implementation-dependent \
              by specification)"
)]

use std::collections::BTreeMap;

use openehr_base::v1_3::base_types::identification::party_ref::PartyRef;
use openehr_rm::draft::access_control::access_control::access_directive::AccessDirective;
use openehr_rm::draft::access_control::access_control::access_directive_settings::AccessDirectiveSettings;
use openehr_rm::v1_2::common::generic::party_identified::PartyIdentified;
use openehr_rm::v1_2::common::generic::party_proxy::PartyProxy;
use serde_json::Value;

use crate::error::{ServiceError, Violation};
use crate::ids::VoId;
use crate::service::FerroEhrService;
use crate::service::ehr::access_rule::party_key;
use crate::service::ehr::validation::validate_ehr_access;
use crate::versioning::audit::text_value;

/// The parties a write lands beside the directives that may name them, by
/// container and RM type: the demographics chapter of an EHR Extract, or the
/// demographic records of an archive.
pub(in crate::service) type Landing = BTreeMap<VoId, String>;

/// One party reference of the settings.
struct Named<'a> {
    /// What holds the reference: `directive` or `gate-keeper`.
    holder: &'static str,
    /// The path of the holder in the settings.
    directive: String,
    /// The directive's name; `None` for a gate-keeper.
    name: Option<&'a str>,
    /// The attribute of the directive that holds the reference.
    attribute: String,
    /// The reference.
    party: &'a PartyRef,
}

impl FerroEhrService {
    /// Validates an `EHR_ACCESS` about to be committed: its RM structure and
    /// settings ([`validate_ehr_access`]), then every party its settings
    /// name ([`Self::check_directive_parties`]).
    ///
    /// # Errors
    /// The refusals of [`validate_ehr_access`] and
    /// [`Self::check_directive_parties`].
    pub(in crate::service) async fn validate_ehr_access_for_commit(
        &self,
        data: &Value,
        incomplete: bool,
    ) -> Result<(), ServiceError> {
        validate_ehr_access(data, incomplete)?;
        match data.get("settings").filter(|settings| !settings.is_null()) {
            Some(settings) => {
                self.check_directive_parties(settings, &Landing::new())
                    .await
            }
            None => Ok(()),
        }
    }

    /// Refuses `settings` whose directives or gate-keepers name a party that
    /// neither the demographic store nor `landing` holds as a party of the
    /// referenced type. A gate-keeper naming nobody would lock the settings
    /// to nobody, the same defect as a directive naming nobody.
    ///
    /// The settings have passed the scheme's checks
    /// ([`check_written_settings`](crate::service::ehr::access_settings::check_written_settings)).
    ///
    /// # Errors
    /// [`ServiceError::Unprocessable`] (`422`) naming the first such
    /// directive or gate-keeper, the attribute and the party; a [`ServiceError::BadRequest`]
    /// for settings that do not read as the scheme; the storage error of a
    /// party read.
    pub(in crate::service) async fn check_directive_parties(
        &self,
        settings: &Value,
        landing: &Landing,
    ) -> Result<(), ServiceError> {
        let read = serde_json::from_value::<AccessDirectiveSettings>(settings.clone())
            .map_err(|e| ServiceError::bad_request("EHR_ACCESS.settings do not read", e))?;
        let mut named = Vec::new();
        for (index, keeper) in read
            .gate_keepers
            .as_deref()
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            if let Some(party) = external_ref(keeper) {
                named.push(Named {
                    holder: "gate-keeper",
                    directive: format!("gate_keepers[{index}]"),
                    name: None,
                    attribute: "external_ref".to_owned(),
                    party,
                });
            }
        }
        for (index, directive) in read
            .directives
            .as_deref()
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            collect(&format!("directives[{index}]"), directive, &mut named);
        }
        for reference in named {
            if !self.holds(reference.party, landing).await? {
                return Err(unknown_party(&reference));
            }
        }
        Ok(())
    }

    /// Whether the demographic store or `landing` holds the party `party`
    /// references, as a party of its type.
    async fn holds(&self, party: &PartyRef, landing: &Landing) -> Result<bool, ServiceError> {
        // NOTE: a key that is no UUID names no party of the store, which keys parties by UUID.
        let Some(vo_id) = party_key(party).and_then(|key| key.parse().ok().map(VoId)) else {
            return Ok(false);
        };
        if let Some(rm_type) = landing.get(&vo_id) {
            return Ok(of_type(party, rm_type));
        }
        match self.party_kind(vo_id).await {
            Ok(kind) => Ok(of_type(party, kind.rm_type())),
            Err(ServiceError::NotFound(_)) => Ok(false),
            Err(error) => Err(error),
        }
    }
}

/// Every party reference of `directive` at `path` and of its exceptions.
fn collect<'a>(path: &str, directive: &'a AccessDirective, named: &mut Vec<Named<'a>>) {
    let name = text_value(&directive.name);
    for (index, scope) in directive
        .applies_to
        .as_deref()
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        if let Some(party) = &scope.party {
            named.push(Named {
                holder: "directive",
                directive: path.to_owned(),
                name: Some(name),
                attribute: format!("applies_to[{index}].party"),
                party,
            });
        }
    }
    if let Some(party) = directive.instructed_by.as_ref().and_then(external_ref) {
        named.push(Named {
            holder: "directive",
            directive: path.to_owned(),
            name: Some(name),
            attribute: "instructed_by.external_ref".to_owned(),
            party,
        });
    }
    for (index, exception) in directive
        .exceptions
        .as_deref()
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        collect(&format!("{path}.exceptions[{index}]"), exception, named);
    }
}

/// The party a `PARTY_IDENTIFIED` or `PARTY_RELATED` references; `None` for
/// one that references none and for `PARTY_SELF`, which names the subject.
fn external_ref(proxy: &PartyProxy) -> Option<&PartyRef> {
    match proxy {
        PartyProxy::PartyIdentified(PartyIdentified::PartyIdentified(data)) => {
            data.external_ref.as_ref()
        }
        PartyProxy::PartyIdentified(PartyIdentified::PartyRelated(data)) => {
            data.external_ref.as_ref()
        }
        PartyProxy::PartySelf(_) => None,
    }
}

/// Whether a reference of `party`'s type may name a party of RM type
/// `rm_type`: the type itself, `PARTY`, or `ACTOR` for any party but a `ROLE`
/// (BASE `party_ref.adoc`, `PARTY_REF.type`).
fn of_type(party: &PartyRef, rm_type: &str) -> bool {
    match party.r#type.as_str() {
        "PARTY" => true,
        "ACTOR" => rm_type != "ROLE",
        named => named == rm_type,
    }
}

/// The `422` of a directive or gate-keeper that names a party the
/// demographic store does not hold.
fn unknown_party(reference: &Named<'_>) -> ServiceError {
    let id = party_key(reference.party).unwrap_or_else(|| {
        "with an id that is neither a HIER_OBJECT_ID nor a GENERIC_ID".to_owned()
    });
    let name = reference
        .name
        .map_or_else(String::new, |name| format!(" ({name:?})"));
    ServiceError::content_invalid(
        Violation::new(format!(
            "EHR_ACCESS {} {}{name} names the {} {id} at {}, which the demographic store does \
             not hold",
            reference.holder, reference.directive, reference.party.r#type, reference.attribute
        ))
        .with_path(format!(
            "/settings/{}/{}",
            reference.directive.replace('.', "/"),
            reference.attribute.replace('.', "/")
        )),
    )
}
