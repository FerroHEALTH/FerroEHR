// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The checks on what a system caller commits: only a client whose `ACTOR`
//! performs a `ROLE` granting `commit_composition` commits into an EHR, a COMPOSITION only of a template a grant lists, and
//! each COMPOSITION names a facility of that grant and is either a device act
//! of a `device` grant or a relayed person act of a `relay` grant.
//!
//! The grants are read from the policy parties per request through
//! [`Capabilities::current`], by the codes of the operation terminology
//! (`crate::party_binding::grants::operation`).
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

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): the canonical COMPOSITION body and \
              the grant credentials a system commit is judged against"
)]

use std::sync::Arc;

use serde_json::Value;
use sqlx::PgConnection;

use crate::error::{ServiceError, Violation};
use crate::ids::VoId;
use crate::party_binding::grants::capabilities::{Capabilities, Credentials, Grant};
use crate::party_binding::grants::operation::{
    DEVICE, DEVICE_IDENTIFIER_ISSUER, FACILITY_PARTS, ORIGIN, Operation, RELAY, SCOPES,
};
use crate::versioning::Kind;

/// The capabilities of the request's system caller for a commit of `kind`,
/// when the commit is a COMPOSITION's; `None` for a commit no system caller
/// makes, one of a demographic kind, or a granted client's commit of another
/// EHR kind.
///
/// # Errors
/// [`ServiceError::Forbidden`] (`403`) for a system caller holding no
/// `commit_composition` grant.
fn composing_grants(kind: Kind) -> Result<Option<Arc<Capabilities>>, ServiceError> {
    let Some(identity) = crate::versioning::committer::current_committer() else {
        return Ok(None);
    };
    if !identity.system || kind.is_demographic() {
        return Ok(None);
    }
    let capabilities = Capabilities::current();
    if !capabilities.holds(Operation::CommitComposition) {
        return Err(ServiceError::not_permitted(
            "a system caller commits into an EHR only under a commit_composition grant: a \
             CAPABILITY of a ROLE its client's ACTOR performs, coding \
             ferroehr-operations::commit_composition",
        ));
    }
    Ok((kind == Kind::Composition).then_some(capabilities))
}

/// Admits the content of a create or a modify of `kind` by the request's
/// system caller; any other caller passes.
///
/// # Errors
/// [`ServiceError::Forbidden`] (`403`) for a caller without a grant, a
/// template no grant lists, a person act under no `relay` grant
/// and a device act under no `device` grant;
/// [`ServiceError::Unprocessable`] (`422`) for an organisation composer, a
/// person act naming no professional by an identifier, and a COMPOSITION
/// naming no facility of an admitting grant.
pub(crate) fn admit_content(
    kind: Kind,
    canonical: &Value,
    template_id: Option<&str>,
) -> Result<(), ServiceError> {
    let Some(capabilities) = composing_grants(kind)? else {
        return Ok(());
    };
    let grants: Vec<Credentials<'_>> = capabilities
        .of(Operation::CommitComposition)
        .map(Grant::credentials)
        .collect();
    judge(&grants, canonical, template_of(canonical, template_id))
}

/// Judges `canonical`, of `template`, against the `commit_composition` grants
/// `grants`.
fn judge(
    grants: &[Credentials<'_>],
    canonical: &Value,
    template: Option<&str>,
) -> Result<(), ServiceError> {
    let writing = admit_template(grants, template)?;
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
    let devices: Vec<Credentials<'_>> = writing
        .iter()
        .copied()
        .filter(|grant| device_act(*grant, canonical))
        .collect();
    let admitting: Vec<Credentials<'_>> = if devices.is_empty() {
        let relays = of_origin(&writing, RELAY);
        if relays.is_empty() {
            return Err(ServiceError::not_permitted(
                "a COMPOSITION that is no device act records a person's act, which a system \
                 client commits only under a commit_composition grant whose origin is relay; \
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
        relays
    } else {
        let devices = of_origin(&devices, DEVICE);
        if devices.is_empty() {
            return Err(ServiceError::not_permitted(
                "a device act is committed only under a commit_composition grant whose origin \
                 is device",
            ));
        }
        devices
    };
    admit_facility(&admitting, canonical)
}

/// The grants of `grants` whose `origin` elements code `origin`.
fn of_origin<'a>(grants: &[Credentials<'a>], origin: &str) -> Vec<Credentials<'a>> {
    grants
        .iter()
        .copied()
        .filter(|grant| grant.words(ORIGIN).contains(&origin))
        .collect()
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
/// [`ServiceError::Forbidden`] (`403`) for a caller without a grant, or a
/// COMPOSITION of a template no grant lists; the storage
/// error of the template read.
pub(crate) async fn admit_delete(
    tx: &mut PgConnection,
    kind: Kind,
    vo_id: VoId,
) -> Result<(), ServiceError> {
    let Some(capabilities) = composing_grants(kind)? else {
        return Ok(());
    };
    let grants: Vec<Credentials<'_>> = capabilities
        .of(Operation::CommitComposition)
        .map(Grant::credentials)
        .collect();
    let template = crate::storage::version_repo::placement::head_template(tx, vo_id).await?;
    admit_template(&grants, template.as_deref()).map(drop)
}

/// The grants of `grants` that list `template`.
///
/// # Errors
/// [`ServiceError::Forbidden`] (`403`) when none does.
fn admit_template<'a>(
    grants: &[Credentials<'a>],
    template: Option<&str>,
) -> Result<Vec<Credentials<'a>>, ServiceError> {
    let writing: Vec<Credentials<'a>> = grants
        .iter()
        .copied()
        .filter(|grant| template.is_some_and(|template| grant.texts(SCOPES[0]).contains(&template)))
        .collect();
    if writing.is_empty() {
        return Err(ServiceError::not_permitted(
            "the system client may commit only a COMPOSITION of a template a \
             commit_composition grant lists in an element coded ferroehr-operations::template",
        ));
    }
    Ok(writing)
}

/// One facility a grant covers: an identifier issuer with the identifier,
/// or with a prefix of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Facility<'a> {
    issuer: &'a str,
    id: Option<&'a str>,
    prefix: Option<&'a str>,
}

impl Facility<'_> {
    /// Whether the facility identifier `id` of `issuer` is this facility.
    fn covers(&self, issuer: &str, id: &str) -> bool {
        self.issuer == issuer
            && (self.id == Some(id) || self.prefix.is_some_and(|prefix| id.starts_with(prefix)))
    }
}

/// The facilities `grant` covers: each element coded `facility` whose value
/// is a `DV_IDENTIFIER`, and each `CLUSTER` coded `facility` holding an
/// `issuer` with an `identifier` or an `identifier_prefix`.
fn facilities(grant: Credentials<'_>) -> Vec<Facility<'_>> {
    let identified = grant.named(SCOPES[1]).into_iter().filter_map(|item| {
        let value = item.get("value")?;
        (value.get("_type").and_then(Value::as_str) == Some("DV_IDENTIFIER")).then_some(())?;
        Some(Facility {
            issuer: value.get("issuer")?.as_str()?,
            id: Some(value.get("id")?.as_str()?),
            prefix: None,
        })
    });
    let clustered = grant.clusters(SCOPES[1]).into_iter().filter_map(|cluster| {
        Some(Facility {
            issuer: cluster.texts(FACILITY_PARTS[0]).first().copied()?,
            id: cluster.texts(FACILITY_PARTS[1]).first().copied(),
            prefix: cluster.texts(FACILITY_PARTS[2]).first().copied(),
        })
    });
    identified.chain(clustered).collect()
}

/// Refuses a COMPOSITION naming no facility of the grants `admitting`: in
/// `EVENT_CONTEXT.health_care_facility`, or with no context in
/// `feeder_audit.originating_system_audit.location`.
fn admit_facility(admitting: &[Credentials<'_>], canonical: &Value) -> Result<(), ServiceError> {
    let (facility, path) = facility_of(canonical);
    let covered: Vec<Facility<'_>> = admitting.iter().copied().flat_map(facilities).collect();
    if let Some((issuer, id)) = identifiers(facility)
        .find(|(issuer, id)| covered.iter().any(|listed| listed.covers(issuer, id)))
    {
        crate::system_log::access_context::record_facility(issuer, id);
        return Ok(());
    }
    Err(ServiceError::content_invalid(
        Violation::new(
            "carries no identifier of a facility a commit_composition grant of the client \
             covers (ferroehr-operations::facility); the server writes no facility (EHDS \
             Art. 13(4))",
        )
        .with_path(path),
    ))
}

/// The facility `canonical` names, with its path: its
/// `EVENT_CONTEXT.health_care_facility`, or with no context its
/// `feeder_audit.originating_system_audit.location`.
pub(crate) fn facility_of(canonical: &Value) -> (Option<&Value>, &'static str) {
    match canonical.get("context").filter(|c| !c.is_null()) {
        Some(context) => (
            context.get("health_care_facility"),
            "EVENT_CONTEXT.health_care_facility",
        ),
        None => (
            canonical.pointer("/feeder_audit/originating_system_audit/location"),
            "COMPOSITION.feeder_audit.originating_system_audit.location",
        ),
    }
}

/// Whether `canonical`'s composer is a `PARTY_IDENTIFIED` whose identifier of
/// an issuer `grant` lists as a `device_identifier_issuer` is the
/// `feeder_audit.originating_system_audit.system_id`: a device's registration.
fn device_act(grant: Credentials<'_>, canonical: &Value) -> bool {
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
    let issuers = grant.texts(DEVICE_IDENTIFIER_ISSUER);
    identifiers(Some(composer)).any(|(issuer, id)| id == system && issuers.contains(&issuer))
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
pub(crate) fn identifiers(party: Option<&Value>) -> impl Iterator<Item = (&str, &str)> {
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
    use crate::party_binding::policy_mark::TERMINOLOGY;

    fn coded(code: &str) -> Value {
        json!({ "_type": "DV_CODED_TEXT", "value": code,
                "defining_code": { "terminology_id": { "value": TERMINOLOGY },
                                   "code_string": code } })
    }

    fn element(code: &str, value: &Value) -> Value {
        json!({ "_type": "ELEMENT", "name": coded(code), "value": value })
    }

    fn text(code: &str, value: &str) -> Value {
        element(code, &json!({ "_type": "DV_TEXT", "value": value }))
    }

    fn cluster(code: &str, items: &[Value]) -> Value {
        json!({ "_type": "CLUSTER", "name": coded(code), "items": items })
    }

    /// A `commit_composition` grant of `origins`, writing `Lab result` at
    /// ward 7 of one issuer and any site of another.
    fn grant(origins: &[&str]) -> Grant {
        let mut items = vec![
            element("operation", &coded("commit_composition")),
            text("template", "Lab result"),
            cluster(
                "facility",
                &[
                    text("issuer", "urn:oid:2.999.1"),
                    text("identifier", "ward-7"),
                ],
            ),
            cluster(
                "facility",
                &[
                    text("issuer", "urn:oid:2.999.2"),
                    text("identifier_prefix", "site-"),
                ],
            ),
            text("device_identifier_issuer", "urn:oid:2.999.9"),
        ];
        items.extend(
            origins
                .iter()
                .map(|origin| element("origin", &coded(origin))),
        );
        Grant::new(
            Operation::CommitComposition,
            uuid::Uuid::nil(),
            Some(Arc::new(json!({ "_type": "ITEM_TREE", "items": items }))),
        )
    }

    fn party(issuer: &str, id: &str) -> Value {
        json!({ "_type": "PARTY_IDENTIFIED",
                "identifiers": [{ "_type": "DV_IDENTIFIER", "issuer": issuer, "id": id }] })
    }

    #[test]
    fn a_facility_matches_by_id_or_by_prefix_under_its_issuer() {
        let lab = grant(&[RELAY]);
        let lab = [lab.credentials()];
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
    fn a_facility_is_also_an_identifier_element() {
        let identified = Grant::new(
            Operation::CommitComposition,
            uuid::Uuid::nil(),
            Some(Arc::new(json!({ "_type": "ITEM_TREE", "items": [element(
                "facility",
                &json!({ "_type": "DV_IDENTIFIER", "issuer": "urn:oid:2.999.4", "id": "ward-1" })
            )] }))),
        );
        let with = |facility: Value| json!({ "context": { "health_care_facility": facility } });
        let grants = [identified.credentials()];
        assert!(admit_facility(&grants, &with(party("urn:oid:2.999.4", "ward-1"))).is_ok());
        assert!(admit_facility(&grants, &with(party("urn:oid:2.999.4", "ward-10"))).is_err());
    }

    #[test]
    fn a_composition_without_context_names_its_location_in_the_feeder_audit() {
        let lab = grant(&[RELAY]);
        let lab = [lab.credentials()];
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
        let device = grant(&[DEVICE]);
        let act = |system: &str| {
            json!({ "composer": party("urn:oid:2.999.9", "pump-1"),
                    "feeder_audit": { "originating_system_audit": { "system_id": system } } })
        };
        assert!(device_act(device.credentials(), &act("pump-1")));
        assert!(!device_act(device.credentials(), &act("pump-2")));
        let other_issuer = json!({ "composer": party("urn:oid:2.999.1", "pump-1"),
            "feeder_audit": { "originating_system_audit": { "system_id": "pump-1" } } });
        assert!(!device_act(device.credentials(), &other_issuer));
    }

    #[test]
    fn each_origin_admits_only_its_own_acts() {
        let at_ward = json!({ "health_care_facility": party("urn:oid:2.999.1", "ward-7") });
        let device_act = json!({ "composer": party("urn:oid:2.999.9", "pump-1"), "context": at_ward,
            "feeder_audit": { "originating_system_audit": { "system_id": "pump-1" } } });
        let person_act =
            json!({ "composer": party("urn:oid:2.999.5", "hp-1"), "context": at_ward });
        let relay = grant(&[RELAY]);
        let device = grant(&[DEVICE]);
        let template = Some("Lab result");
        assert!(judge(&[relay.credentials()], &person_act, template).is_ok());
        assert!(matches!(
            judge(&[relay.credentials()], &device_act, template),
            Err(ServiceError::Forbidden(_))
        ));
        assert!(judge(&[device.credentials()], &device_act, template).is_ok());
        assert!(matches!(
            judge(&[device.credentials()], &person_act, template),
            Err(ServiceError::Forbidden(_))
        ));
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
        let lab = grant(&[RELAY]);
        let lab = [lab.credentials()];
        assert!(admit_template(&lab, Some("Lab result")).is_ok());
        assert!(admit_template(&lab, Some("Vital signs")).is_err());
        assert!(admit_template(&lab, None).is_err());
    }
}
