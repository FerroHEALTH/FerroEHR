// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The checks on what a system caller commits: only a client an
//! `[[authz.system_client]]` table declares commits into an EHR, a
//! COMPOSITION only of a template its `writes` lists, and each COMPOSITION
//! names a facility of its `facilities` and is either a device act or a
//! relayed person act.
//!
//! EHDS Art. 13(4) (`docs/law/eu/ehds/text.html`) asks the record to
//! "identify the health professional and healthcare provider that carried out
//! such registration or update", so the server checks the facility and never
//! fills it. RM `COMPOSITION.composer` is "The person primarily responsible
//! for the content of the Composition" (`org.openehr.rm.composition.composition.adoc`);
//! `EVENT_CONTEXT.health_care_facility` names the facility of the event, and
//! `FEEDER_AUDIT_DETAILS.location` the "particular site/facility within an
//! organisation which handled the item" for a COMPOSITION with no context
//! (`org.openehr.rm.common.feeder_audit_details.adoc`). Naming the device in
//! `composer` for a registration no professional made, and what a system
//! caller may do, are our own design.

use serde_json::Value;
use sqlx::PgConnection;

use crate::config::clients::{Origin, SystemClient};
use crate::error::{ServiceError, Violation};
use crate::ids::VoId;
use crate::versioning::Kind;

/// The client table of the request's system caller for a commit of `kind`,
/// when the commit is a COMPOSITION's; `None` for a commit no system caller
/// makes, one of a demographic kind, or a declared client's commit of
/// another EHR kind.
///
/// # Errors
/// [`ServiceError::Forbidden`] (`403`) for a system caller whose client no
/// `[[authz.system_client]]` table declares.
fn composing_client(kind: Kind) -> Result<Option<std::sync::Arc<SystemClient>>, ServiceError> {
    let Some(identity) = crate::versioning::committer::current_committer() else {
        return Ok(None);
    };
    if !identity.system || kind.is_demographic() {
        return Ok(None);
    }
    let client = identity.system_client.ok_or_else(|| {
        ServiceError::not_permitted(
            "a system caller commits into an EHR only as a client an [[authz.system_client]] \
             table declares, with the provider it acts for (EHDS Art. 13(4))",
        )
    })?;
    Ok((kind == Kind::Composition).then_some(client))
}

/// Admits the content of a create or a modify of `kind` by the request's
/// system caller; any other caller passes.
///
/// # Errors
/// [`ServiceError::Forbidden`] (`403`) for an undeclared client, a template
/// outside its `writes`, a person act from a client without the `relay`
/// origin and a device act from one without `device`;
/// [`ServiceError::Unprocessable`] (`422`) for an organisation composer, a
/// person act naming no professional by an identifier, and a COMPOSITION
/// naming no facility of the client's `facilities`.
pub(crate) fn admit_content(
    kind: Kind,
    canonical: &Value,
    template_id: Option<&str>,
) -> Result<(), ServiceError> {
    let Some(client) = composing_client(kind)? else {
        return Ok(());
    };
    admit_template(&client, template_of(canonical, template_id))?;
    if organisation_composer(canonical) {
        return Err(ServiceError::content_invalid(
            Violation::new(
                "names an organisation, and the composer is the person primarily responsible \
                 for the content; a performing organisation goes into \
                 feeder_audit.originating_system_audit.provider or a participation",
            )
            .with_path("COMPOSITION.composer"),
        ));
    }
    if device_act(&client, canonical) {
        if !client.origins.contains(&Origin::Device) {
            return Err(ServiceError::not_permitted(
                "a device act is committed only by a client whose [[authz.system_client]] \
                 origins include device",
            ));
        }
    } else {
        if !client.origins.contains(&Origin::Relay) {
            return Err(ServiceError::not_permitted(
                "a COMPOSITION that is no device act records a person's act, which a system \
                 client commits only when its [[authz.system_client]] origins include relay; \
                 a local person who acted commits under a token of their own",
            ));
        }
        if !names_professional(canonical) {
            return Err(ServiceError::content_invalid(
                Violation::new(
                    "a relayed person act names the professional by an identifier, in the \
                     composer or in the performer of a participation (EHDS Art. 13(4))",
                )
                .with_path("COMPOSITION.composer"),
            ));
        }
    }
    admit_facility(&client, canonical)
}

/// The template a COMPOSITION is of: `template_id` as the change carries it,
/// or its body's `archetype_details.template_id`.
pub(crate) fn template_of<'a>(
    canonical: &'a Value,
    template_id: Option<&'a str>,
) -> Option<&'a str> {
    template_id.or_else(|| {
        canonical
            .pointer("/archetype_details/template_id/value")
            .and_then(Value::as_str)
    })
}

/// Admits a logical deletion of `vo_id`, of `kind`, by the request's system
/// caller; any other caller passes.
///
/// # Errors
/// [`ServiceError::Forbidden`] (`403`) for an undeclared client or a
/// COMPOSITION of a template outside its `writes`; the storage error of the
/// template read.
pub(crate) async fn admit_delete(
    tx: &mut PgConnection,
    kind: Kind,
    vo_id: VoId,
) -> Result<(), ServiceError> {
    let Some(client) = composing_client(kind)? else {
        return Ok(());
    };
    let template = crate::storage::version_repo::placement::head_template(tx, vo_id).await?;
    admit_template(&client, template.as_deref())
}

/// Refuses a COMPOSITION of `template` that `client`'s `writes` does not list.
fn admit_template(client: &SystemClient, template: Option<&str>) -> Result<(), ServiceError> {
    if template.is_some_and(|template| client.writes.iter().any(|listed| listed == template)) {
        return Ok(());
    }
    Err(ServiceError::not_permitted(
        "the system client may commit only a COMPOSITION of a template its \
         [[authz.system_client]] writes lists",
    ))
}

/// Refuses a COMPOSITION naming no facility of `client`'s `facilities`: in
/// `EVENT_CONTEXT.health_care_facility`, or with no context in
/// `feeder_audit.originating_system_audit.location`.
fn admit_facility(client: &SystemClient, canonical: &Value) -> Result<(), ServiceError> {
    let (facility, path) = match canonical.get("context").filter(|c| !c.is_null()) {
        Some(context) => (
            context.get("health_care_facility"),
            "EVENT_CONTEXT.health_care_facility",
        ),
        None => (
            canonical.pointer("/feeder_audit/originating_system_audit/location"),
            "COMPOSITION.feeder_audit.originating_system_audit.location",
        ),
    };
    if let Some((issuer, id)) = identifiers(facility).find(|(issuer, id)| {
        client.facilities.iter().any(|listed| {
            listed.issuer == *issuer
                && (listed.id.as_deref() == Some(*id)
                    || listed
                        .id_prefix
                        .as_deref()
                        .is_some_and(|prefix| id.starts_with(prefix)))
        })
    }) {
        crate::system_log::access_context::record_facility(issuer, id);
        return Ok(());
    }
    Err(ServiceError::content_invalid(
        Violation::new(
            "carries no identifier of a facility the client's [[authz.system_client]] \
             facilities list; the server writes no facility (EHDS Art. 13(4))",
        )
        .with_path(path),
    ))
}

/// Whether `canonical`'s composer is a `PARTY_IDENTIFIED` whose identifier of
/// one of `client`'s `device_identifier_issuers` is the
/// `feeder_audit.originating_system_audit.system_id`: a device's registration.
fn device_act(client: &SystemClient, canonical: &Value) -> bool {
    let Some(composer) = canonical.get("composer").filter(|composer| {
        composer.get("_type").and_then(Value::as_str) == Some("PARTY_IDENTIFIED")
    }) else {
        return false;
    };
    let Some(system) = canonical
        .pointer("/feeder_audit/originating_system_audit/system_id")
        .and_then(Value::as_str)
    else {
        return false;
    };
    identifiers(Some(composer)).any(|(issuer, id)| {
        id == system
            && client
                .device_identifier_issuers
                .iter()
                .any(|listed| listed == issuer)
    })
}

/// Whether `canonical`'s composer references an `ORGANISATION`.
fn organisation_composer(canonical: &Value) -> bool {
    canonical
        .pointer("/composer/external_ref/type")
        .and_then(Value::as_str)
        == Some("ORGANISATION")
}

/// Whether `canonical` names a professional by an identifier: its composer,
/// or the performer of one of its context's participations, is a
/// `PARTY_IDENTIFIED` carrying one.
fn names_professional(canonical: &Value) -> bool {
    let identified = |party: Option<&Value>| {
        party.is_some_and(|party| {
            party.get("_type").and_then(Value::as_str) == Some("PARTY_IDENTIFIED")
                && identifiers(Some(party)).next().is_some()
        })
    };
    identified(canonical.get("composer"))
        || canonical
            .pointer("/context/participations")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|participation| identified(participation.get("performer")))
}

/// Every `(issuer, id)` of `party`'s `identifiers`, skipping one without an
/// issuer.
fn identifiers(party: Option<&Value>) -> impl Iterator<Item = (&str, &str)> {
    party
        .and_then(|party| party.get("identifiers"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|identifier| {
            Some((
                identifier.get("issuer")?.as_str()?,
                identifier.get("id")?.as_str()?,
            ))
        })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::config::clients::Facility;

    fn client(origins: &[Origin]) -> SystemClient {
        SystemClient {
            issuer: "https://machines.example".to_owned(),
            client_id: "lab".to_owned(),
            facilities: vec![
                Facility {
                    issuer: "urn:oid:2.999.1".to_owned(),
                    id: Some("ward-7".to_owned()),
                    id_prefix: None,
                },
                Facility {
                    issuer: "urn:oid:2.999.2".to_owned(),
                    id: None,
                    id_prefix: Some("site-".to_owned()),
                },
            ],
            writes: vec!["Lab result".to_owned()],
            origins: origins.to_vec(),
            device_identifier_issuers: vec!["urn:oid:2.999.9".to_owned()],
            ..SystemClient::default()
        }
    }

    fn party(issuer: &str, id: &str) -> Value {
        json!({ "_type": "PARTY_IDENTIFIED",
                "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": issuer, "id": id }] })
    }

    #[test]
    fn a_facility_matches_by_id_or_by_prefix_under_its_issuer() {
        let lab = client(&[Origin::Relay]);
        let with = |facility: Value| json!({ "context": { "health_care_facility": facility } });
        assert!(admit_facility(&lab, &with(party("urn:oid:2.999.1", "ward-7"))).is_ok());
        assert!(admit_facility(&lab, &with(party("urn:oid:2.999.2", "site-3"))).is_ok());
        for refused in [
            party("urn:oid:2.999.1", "ward-8"),
            party("urn:oid:2.999.3", "ward-7"),
            party("urn:oid:2.999.1", "site-3"),
        ] {
            assert!(admit_facility(&lab, &with(refused)).is_err());
        }
    }

    #[test]
    fn a_composition_without_context_names_its_location_in_the_feeder_audit() {
        let lab = client(&[Origin::Relay]);
        let located = json!({ "feeder_audit": { "originating_system_audit": {
            "system_id": "lab", "location": party("urn:oid:2.999.1", "ward-7") } } });
        assert!(admit_facility(&lab, &located).is_ok());
        let unlocated = json!({ "context": null, "feeder_audit": {
            "originating_system_audit": { "system_id": "lab" } } });
        match admit_facility(&lab, &unlocated) {
            Err(ServiceError::Unprocessable { violation, .. }) => assert_eq!(
                violation.path(),
                Some("COMPOSITION.feeder_audit.originating_system_audit.location")
            ),
            other => panic!("expected 422, got {other:?}"),
        }
    }

    #[test]
    fn a_device_act_names_the_device_in_the_composer_and_the_feeder_audit() {
        let device = client(&[Origin::Device]);
        let act = |system: &str| {
            json!({ "composer": party("urn:oid:2.999.9", "pump-1"),
                    "feeder_audit": { "originating_system_audit": { "system_id": system } } })
        };
        assert!(device_act(&device, &act("pump-1")));
        assert!(!device_act(&device, &act("pump-2")));
        let other_issuer = json!({ "composer": party("urn:oid:2.999.1", "pump-1"),
            "feeder_audit": { "originating_system_audit": { "system_id": "pump-1" } } });
        assert!(!device_act(&device, &other_issuer));
    }

    #[test]
    fn a_professional_is_named_in_the_composer_or_a_participation() {
        assert!(names_professional(
            &json!({ "composer": party("urn:oid:2.999.5", "hp-1") })
        ));
        let named = json!({
            "composer": { "_type": "PARTY_IDENTIFIED", "name": "Lab" },
            "context": { "participations": [{ "performer": party("urn:oid:2.999.5", "hp-1") }] }
        });
        assert!(names_professional(&named));
        assert!(!names_professional(
            &json!({ "composer": { "_type": "PARTY_IDENTIFIED", "name": "A clinician" } })
        ));
        assert!(!names_professional(
            &json!({ "composer": { "_type": "PARTY_SELF" } })
        ));
    }

    #[test]
    fn only_a_listed_template_is_written() {
        let lab = client(&[Origin::Relay]);
        assert!(admit_template(&lab, Some("Lab result")).is_ok());
        assert!(admit_template(&lab, Some("Vital signs")).is_err());
        assert!(admit_template(&lab, None).is_err());
    }
}
