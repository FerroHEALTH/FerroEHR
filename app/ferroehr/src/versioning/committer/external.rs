// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The committer of a verified external person, who has no `PERSON` here,
//! and the provider an external write names.
//!
//! RM common `org.openehr.rm.common.party_identified.adoc`: `PARTY_IDENTIFIED`
//! is "Used to describe parties where only identifiers may be known, and there
//! is no entry at all in the demographic system". EHDS Art. 13(4)
//! (`docs/law/eu/ehds/text.html`) asks the record to identify the professional
//! and the provider, so the committer carries the professional's identifier
//! under its issuing authority beside the login the federation issuer vouched
//! for, and the write names the provider in `EVENT_CONTEXT.health_care_facility`.
//! The `"<country_code>:<issuing_authority_name>"` issuer and its
//! normalisation are our own design (#5005). On the MyHealth@EU route the
//! professional's identifier is the HP Identity Assertion's `NameID` under the
//! assertion's issuer, typed by its format, because one string under two
//! formats names two identifiers (eHDSI SAML Profile 10.0.0 §2.1), and the
//! point of care adds the HCPO's `organization-id` (§2.3).

use ferrohealth_smart::federation::contact::NameId;
use openehr_rm::prelude::{DvIdentifier, PartyIdentified, PartyIdentifiedData, PartyProxy};
use unicode_normalization::UnicodeNormalization;

use super::CommitterIdentity;
use crate::error::ServiceError;

/// The `DV_IDENTIFIER.type` of an external professional's identifier on a
/// domestic route.
// NOTE: no openEHR spec governs the type string, our own design: the field name
// of Implementing Regulation (EU) 2026/2099 Annex Table 1.
pub const HP_IDENTIFIER_TYPE: &str = "hp_identifier";

/// What a verified external person's token asserts about the professional
/// and the healthcare provider, as asserted; the server checks each value
/// where it writes it.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ExternalProfessional {
    /// The name as one string: the asserted full name, or the given and the
    /// family name joined by a space.
    pub name: Option<String>,
    /// Table 1 `hp_identifier`.
    pub hp_identifier: Option<String>,
    /// Table 1 `country_code`.
    pub country_code: Option<String>,
    /// Table 1 `issuing_authority_name`.
    pub issuing_authority: Option<String>,
    /// Table 2 `healthcare_provider_identifier`.
    pub provider_identifier: Option<String>,
    /// Table 2 `issuing_authority_name`.
    pub provider_authority: Option<String>,
    /// Whether the token comes through a MyHealth@EU contact point, whose
    /// provider identifier is the HCPO's `organization-id`.
    pub contact_point: bool,
    /// The permissions the HP Identity Assertion carries (eHDSI SAML Profile
    /// 10.0.0 §2.3, `urn:oasis:names:tc:xspa:1.0:subject:hl7:permission`).
    pub permissions: Vec<String>,
    /// The HP Identity Assertion's `NameID` with its format, on the
    /// MyHealth@EU route.
    pub name_id: Option<NameId>,
    /// The HP Identity Assertion's issuer, on the MyHealth@EU route.
    pub assertion_issuer: Option<String>,
}

impl std::fmt::Debug for ExternalProfessional {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExternalProfessional")
            .finish_non_exhaustive()
    }
}

/// The committer of the external person `professional` whose token `identity` carries.
///
/// It is `PARTY_IDENTIFIED{name, identifiers = [the professional's
/// identifier, the token's issuer and subject]}` with no `external_ref`. The
/// professional's identifier is [`professional_identifier`].
///
/// # Errors
/// As [`professional_identifier`].
pub fn committer(
    identity: &CommitterIdentity,
    professional: &ExternalProfessional,
) -> Result<PartyProxy, ServiceError> {
    let mut identifiers =
        openehr_base::containers::NonEmptyVec::of(professional_identifier(professional)?);
    identifiers.push(DvIdentifier {
        issuer: identity.issuer.clone(),
        assigner: None,
        id: identity.subject.clone(),
        r#type: Some(identity.id_type.to_owned()),
    });
    Ok(PartyProxy::PartyIdentified(
        PartyIdentified::PartyIdentified(PartyIdentifiedData {
            external_ref: None,
            name: professional.name.clone(),
            identifiers: Some(identifiers),
        }),
    ))
}

/// The professional's identifier in an external committer.
///
/// On a domestic route it is `{issuer = "<country_code>:<issuing_authority_name>",
/// id = hp_identifier, type = "hp_identifier"}`. On the MyHealth@EU route it
/// is `{issuer = the IdA's issuer, id = the IdA NameID, type =
/// "hp_identifier:<NameID format>"}`.
///
/// # Errors
/// [`ServiceError::Forbidden`] (`403`) naming the field that is missing, a
/// country code that is no upper-case ISO 3166-1 alpha-2 code, or an issuing
/// authority that holds a `:`, a control or a bidi character.
pub fn professional_identifier(
    professional: &ExternalProfessional,
) -> Result<DvIdentifier, ServiceError> {
    if professional.contact_point {
        let name_id = professional.name_id.as_ref().ok_or_else(|| {
            refused(
                "assertion.name_id_format",
                "the contact point's token carries no NameID with one of the eHDSI SAML \
                 Profile 10.0.0 §2.1 formats",
            )
        })?;
        let issuer = required(professional.assertion_issuer.as_deref(), "assertion.issuer")?;
        return Ok(DvIdentifier {
            issuer: Some(issuer.to_owned()),
            assigner: None,
            id: name_id.value().to_owned(),
            r#type: Some(format!(
                "{HP_IDENTIFIER_TYPE}:{}",
                name_id.format().as_str()
            )),
        });
    }
    let hp_identifier = required(
        professional.hp_identifier.as_deref(),
        "professional.hp_identifier",
    )?;
    let country = country_code(required(
        professional.country_code.as_deref(),
        "professional.country_code",
    )?)?;
    let authority = authority(
        required(
            professional.issuing_authority.as_deref(),
            "professional.issuing_authority_name",
        )?,
        "professional.issuing_authority_name",
    )?;
    Ok(DvIdentifier {
        issuer: Some(format!("{country}:{authority}")),
        assigner: None,
        id: hp_identifier.to_owned(),
        r#type: Some(HP_IDENTIFIER_TYPE.to_owned()),
    })
}

/// The `(issuer, id)` the facility of an external write must carry: the
/// provider's identifier under its normalised issuing authority.
///
/// # Errors
/// [`ServiceError::Forbidden`] (`403`) naming a missing provider field, or an
/// issuing authority that holds a `:`, a control or a bidi character.
pub fn provider(professional: &ExternalProfessional) -> Result<(String, String), ServiceError> {
    let id = required(
        professional.provider_identifier.as_deref(),
        "provider.healthcare_provider_identifier",
    )?;
    let issuer = authority(
        required(
            professional.provider_authority.as_deref(),
            "provider.issuing_authority_name",
        )?,
        "provider.issuing_authority_name",
    )?;
    Ok((issuer, id.to_owned()))
}

/// The `DV_IDENTIFIER.issuer` of the HCPO's `organization-id` in a
/// facility's identifiers: the attribute name eHDSI SAML Profile 10.0.0 §2.3
/// gives the Healthcare Professional Organisation ID.
// NOTE: no openEHR spec governs the issuer string, our own design (#5005): the
// SAML attribute name names the identifier's scheme.
pub const ORGANIZATION_ID_ISSUER: &str = "urn:oasis:names:tc:xspa:1.0:subject:organization-id";

/// The permission a dispensation and its discard need (eHDSI SAML Profile
/// 10.0.0 §2.5: "Notification on Dispensation" and "Notification on
/// Dispensation Discard", PPD-046).
const DISPENSE: &str = "PPD-046";

/// The `(issuer, id)` pairs naming the point of care of an external write.
///
/// They are the provider's identifier under its issuing authority, and on the
/// MyHealth@EU route the HCPO's `organization-id` under
/// [`ORGANIZATION_ID_ISSUER`] beside it.
///
/// # Errors
/// As [`provider`].
pub fn point_of_care(
    professional: &ExternalProfessional,
) -> Result<Vec<(String, String)>, ServiceError> {
    let provider = provider(professional)?;
    let organization = professional
        .contact_point
        .then(|| (ORGANIZATION_ID_ISSUER.to_owned(), provider.1.clone()));
    Ok(std::iter::once(provider).chain(organization).collect())
}

/// Whether `professional` may record a dispensation or discard one.
///
/// A token carrying no permissions leaves the decision to the structural
/// role, and one carrying any must hold PPD-046 (eHDSI SAML Profile 10.0.0
/// §2.3, §2.5).
#[must_use]
pub fn may_dispense(professional: &ExternalProfessional) -> bool {
    professional.permissions.is_empty()
        || professional.permissions.iter().any(|permission| {
            permission == DISPENSE
                || permission
                    .strip_suffix(DISPENSE)
                    .is_some_and(|scheme| scheme.ends_with(':'))
        })
}

/// `value`, refused naming `field` when the token carries none or a blank one.
fn required<'a>(value: Option<&'a str>, field: &str) -> Result<&'a str, ServiceError> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| refused(field, "the federation token carries none"))
}

/// `code` when it is an ISO 3166-1 alpha-2 code in upper case.
// NOTE: no vetted crate lists the assigned codes; the shape is checked here and the
// `[[auth.federation]] countries` the classification enforces bound the values.
fn country_code(code: &str) -> Result<&str, ServiceError> {
    if code.len() == 2 && code.bytes().all(|b| b.is_ascii_uppercase()) {
        return Ok(code);
    }
    Err(refused(
        "professional.country_code",
        "an ISO 3166-1 alpha-2 code in upper case is required",
    ))
}

/// `name` NFC-normalised with its white space trimmed and collapsed, refused
/// naming `field` when it then holds a `:`, a control or a bidi character.
fn authority(name: &str, field: &str) -> Result<String, ServiceError> {
    let normalised: String = name.nfc().collect();
    let collapsed = normalised.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed
        .chars()
        .any(|c| c == ':' || c.is_control() || is_bidi_control(c))
    {
        return Err(refused(
            field,
            "an issuing authority holds no ':', control or bidi character",
        ));
    }
    Ok(collapsed)
}

/// Whether `c` is one of the Unicode bidirectional formatting characters
/// (Unicode Standard Annex #9 §2: ALM, LRM, RLM, the embeddings, overrides and
/// isolates).
const fn is_bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

/// The `403` refusing the external committer for `field`.
fn refused(field: &str, why: &str) -> ServiceError {
    ServiceError::not_permitted(format!(
        "{field}: {why}; the committer of a verified external person names the professional \
         by the identifier under its issuing authority (EHDS Art. 13(4))"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> CommitterIdentity {
        CommitterIdentity {
            subject: "hp-remote-1".to_owned(),
            id_type: "oauth2",
            issuer: Some("https://federation.example".to_owned()),
            client: None,
            person: false,
            party: None,
            representative: false,
            bound: None,
            system: false,
            agent: None,
            external: None,
        }
    }

    fn professional() -> ExternalProfessional {
        ExternalProfessional {
            name: Some("Dr Remote".to_owned()),
            hp_identifier: Some("hp-1".to_owned()),
            country_code: Some("DE".to_owned()),
            issuing_authority: Some("  Bundes\u{0061}\u{0308}rztekammer   Registry ".to_owned()),
            provider_identifier: Some("urn:oid:2.999.7".to_owned()),
            provider_authority: Some("Registry".to_owned()),
            contact_point: false,
            permissions: Vec::new(),
            name_id: None,
            assertion_issuer: None,
        }
    }

    fn identifiers(committer: &PartyProxy) -> Vec<(Option<&str>, &str, Option<&str>)> {
        let PartyProxy::PartyIdentified(PartyIdentified::PartyIdentified(party)) = committer else {
            panic!("an external committer is a PARTY_IDENTIFIED: {committer:?}");
        };
        assert!(party.external_ref.is_none());
        party
            .identifiers
            .iter()
            .flat_map(|ids| ids.iter())
            .map(|id| (id.issuer.as_deref(), id.id.as_str(), id.r#type.as_deref()))
            .collect()
    }

    #[test]
    fn the_committer_names_the_professional_and_the_vouching_login() {
        let committer = committer(&identity(), &professional()).expect("a committer");
        assert_eq!(
            identifiers(&committer),
            vec![
                (
                    Some("DE:Bundes\u{00E4}rztekammer Registry"),
                    "hp-1",
                    Some("hp_identifier")
                ),
                (
                    Some("https://federation.example"),
                    "hp-remote-1",
                    Some("oauth2")
                ),
            ]
        );
        let PartyProxy::PartyIdentified(PartyIdentified::PartyIdentified(party)) = committer else {
            panic!("a PARTY_IDENTIFIED");
        };
        assert_eq!(party.name.as_deref(), Some("Dr Remote"));
    }

    #[test]
    fn a_country_code_that_is_no_upper_case_alpha_2_code_is_refused_naming_the_field() {
        for code in ["de", "DEU", "D1", "", "ÄB"] {
            let asserted = ExternalProfessional {
                country_code: Some(code.to_owned()),
                ..professional()
            };
            let error = committer(&identity(), &asserted).expect_err(code);
            assert!(matches!(error, ServiceError::Forbidden(_)), "{code}");
            assert!(
                error.to_string().contains("professional.country_code"),
                "{code}: {error}"
            );
        }
    }

    #[test]
    fn an_authority_with_a_colon_a_control_or_a_bidi_character_is_refused() {
        for name in [
            "DE:Registry",
            "Reg\u{0007}istry",
            "Reg\u{202E}istry",
            "Reg\u{2066}istry",
        ] {
            let asserted = ExternalProfessional {
                issuing_authority: Some(name.to_owned()),
                ..professional()
            };
            let error = committer(&identity(), &asserted).expect_err(name);
            assert!(
                error
                    .to_string()
                    .contains("professional.issuing_authority_name"),
                "{error}"
            );
        }
    }

    #[test]
    fn a_missing_identifier_is_refused_naming_the_field() {
        let asserted = ExternalProfessional {
            hp_identifier: None,
            ..professional()
        };
        let error = committer(&identity(), &asserted).expect_err("no identifier");
        assert!(error.to_string().contains("professional.hp_identifier"));
    }

    #[test]
    fn the_provider_is_its_identifier_under_its_normalised_authority() {
        assert_eq!(
            provider(&professional()).expect("a provider"),
            ("Registry".to_owned(), "urn:oid:2.999.7".to_owned())
        );
        let none = ExternalProfessional {
            provider_identifier: None,
            ..professional()
        };
        assert!(
            provider(&none)
                .expect_err("no provider")
                .to_string()
                .contains("provider.healthcare_provider_identifier")
        );
    }

    #[test]
    fn the_point_of_care_adds_the_organization_id_on_the_contact_point_route() {
        let pair = ("Registry".to_owned(), "urn:oid:2.999.7".to_owned());
        assert_eq!(
            point_of_care(&professional()).expect("a point of care"),
            vec![pair.clone()]
        );
        let relayed = ExternalProfessional {
            contact_point: true,
            ..professional()
        };
        assert_eq!(
            point_of_care(&relayed).expect("a point of care"),
            vec![
                pair,
                (
                    ORGANIZATION_ID_ISSUER.to_owned(),
                    "urn:oid:2.999.7".to_owned()
                )
            ]
        );
    }

    #[test]
    fn a_dispensation_needs_ppd_046_only_where_permissions_are_asserted() {
        let with = |permissions: &[&str]| ExternalProfessional {
            permissions: permissions.iter().map(|p| (*p).to_owned()).collect(),
            ..professional()
        };
        assert!(may_dispense(&with(&[])));
        assert!(may_dispense(&with(&["PRD-010", "PPD-046"])));
        assert!(may_dispense(&with(&[
            "urn:oasis:names:tc:xspa:1.0:subject:hl7:permission:PPD-046"
        ])));
        assert!(!may_dispense(&with(&["PRD-010"])));
        assert!(!may_dispense(&with(&["XPPD-046"])));
    }

    #[test]
    fn a_contact_points_committer_needs_the_nameid_with_its_format() {
        let relayed = ExternalProfessional {
            contact_point: true,
            assertion_issuer: Some("https://ncp-b.example".to_owned()),
            ..professional()
        };
        let error = committer(&identity(), &relayed).expect_err("no NameID");
        assert!(
            error.to_string().contains("assertion.name_id_format"),
            "{error}"
        );
    }
}
