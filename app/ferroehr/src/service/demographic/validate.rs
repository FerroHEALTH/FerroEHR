// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Inbound-body validation for the demographic chapter — the RM
//! PARTY/ACTOR/ROLE/`PARTY_RELATIONSHIP` invariants and the BASE
//! `PARTY_REF`/`OBJECT_REF` rules enforced at the write boundary
//! (`valid_content` → `422`, `i_demographic_service.adoc §create_party`).
//!
//! Spec oracles:
//! - `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.demographic.party.adoc`
//!   (`Identities_valid`, `Contacts_valid`, `Relationships_validity`,
//!   `Uid_mandatory`), `…demographic.actor.adoc` (`Roles_valid`),
//!   `…demographic.role.adoc` (`Capabilities_valid`, `performer`),
//!   `…demographic.party_relationship.adoc` (`source`/`target`),
//! - `docs/specs/openehr/BASE/docs/UML/classes/org.openehr.base.base_types.party_ref.adoc`
//!   (`PARTY_REF.Type_validity`) +
//!   `…object_ref.adoc` (`OBJECT_REF.namespace`),
//! - `docs/specs/openehr/RM/docs/demographic/master02` (§Modelling of Parties
//!   and Relationships — relationship refs denote the party's version
//!   container).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once; stored-content serving"
)]

use openehr_base::prelude::PartyRef;
use openehr_base::v1_3::base_types::identification::lexical::composite_ids_equal;
use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;
use openehr_base::validate::Validate;
use serde_json::Value;

use crate::service::error::{ServiceError, Violation};

/// The RM `_type` a `PARTY_RELATIONSHIP` versioned object stores.
const RELATIONSHIP_RM_TYPE: &str = "PARTY_RELATIONSHIP";

/// Validate one inbound `PARTY_REF` value against the BASE class checks —
/// `PARTY_REF.Type_validity` and the inherited `OBJECT_REF.namespace` rule.
///
/// The rules themselves are NOT restated here: the reference is decoded into
/// the generated [`PartyRef`] and judged by that class's single spec-cited
/// realization in `openehr-base`
/// (`base_types/identification/party_ref_impl.rs`), so this service and the
/// whole-instance RM pass can never give two answers about the same ref.
/// `context` names the referencing attribute for the `422` message.
///
/// Decoding also enforces the mandatory `OBJECT_REF` attributes (`id`,
/// `namespace`, `type` are all 1..1 —
/// `docs/specs/openehr/BASE/docs/UML/classes/org.openehr.base.base_types.object_ref.adoc`
/// §Attributes), which a partial hand check cannot see.
fn validate_party_ref(reference: &Value, context: &str) -> Result<(), ServiceError> {
    let typed = openehr_its::json::from_canonical_value::<PartyRef>(reference).map_err(|e| {
        ServiceError::content_invalid(
            Violation::new("is not a well-formed PARTY_REF")
                .with_path(context)
                .with_decode_failure(&e),
        )
    })?;
    let violations = typed.invariants();
    if !violations.is_empty() {
        // The class's own `InvariantViolation`s travel on as data; only the
        // first is rendered.
        return Err(ServiceError::content_invalid(
            Violation::new("fails its BASE class invariants")
                .with_path(context)
                .with_causes(violations.into_iter().take(1).collect()),
        ));
    }
    Ok(())
}

/// Structurally validate a candidate party body of concrete RM type `rm_type`:
/// deserialize into the corresponding `openehr_rm` demographic type (a type
/// mismatch → `422`) and enforce the enforceable PARTY/ACTOR/ROLE invariants.
/// `Uid_mandatory` is met by the server injecting the `uid` on read (mirroring
/// the COMPOSITION service), so an incoming body need not carry one.
///
/// This is the raw-body lane's gate, the CONTRIBUTION route, whose payload has
/// never been through a typed door. The direct routes call [`party_invariants`]
/// instead, having decoded the body as the routed kind's concrete RM type before
/// the commit.
///
/// # Errors
/// [`ServiceError::BadRequest`] when the strict reader refuses the body;
/// [`ServiceError::Unprocessable`] when it violates an enforceable
/// PARTY/ACTOR/ROLE invariant.
pub(crate) fn party_check(
    rm_type: &str,
    data: &Value,
    incomplete: bool,
) -> Result<(), ServiceError> {
    use openehr_rm::prelude::{Agent, Group, Organisation, Person, Role};
    let typed = match rm_type {
        "AGENT" => openehr_its::json::from_canonical_value::<Agent>(data).map(drop),
        "GROUP" => openehr_its::json::from_canonical_value::<Group>(data).map(drop),
        "ORGANISATION" => openehr_its::json::from_canonical_value::<Organisation>(data).map(drop),
        "PERSON" => openehr_its::json::from_canonical_value::<Person>(data).map(drop),
        "ROLE" => openehr_its::json::from_canonical_value::<Role>(data).map(drop),
        other => {
            return Err(ServiceError::content_invalid(
                Violation::new(format!("not a demographic party type: {other:?}"))
                    .with_path("_type"),
            ));
        }
    };
    // A body that DOES construct is still fully judged; one that does not is
    // handed to the relaxed whole-instance pass below, which enforces
    // everything except presence.
    // NOTE: on a `553|incomplete|` commit the TYPED construction is not the gate
    // — the generated party types make mandatory attributes structural, and RM
    // common master06 §Incomplete Content lifts precisely those bounds.
    if !incomplete {
        // NOTE: the released `responses/422.yaml` scopes 422 to content that
        // "could be converted to a resource" — a body the strict reader
        // refuses is the 400 row, as on every direct commit route.
        typed.map_err(|e| {
            ServiceError::bad_request(format!("invalid canonical JSON body: {e}"), e)
        })?;
    }
    party_invariants(rm_type, data, incomplete)
}

/// The party invariants that are NOT facts of the typed decode — everything
/// [`party_check`] does apart from constructing the RM value.
///
/// The direct routes enter here: their body is already a constructed value of
/// this very class, so the decode half has nothing left to judge, while these
/// rules remain live because they are stated over the RAW JSON (a root's
/// `ARCHETYPED` presence, and the "present implies non-empty" list bounds that
/// an absent and a present-empty list both deserialize into the same `Vec`).
///
/// # Errors
/// [`ServiceError::Unprocessable`] naming the violated rule.
pub(super) fn party_invariants(
    rm_type: &str,
    data: &Value,
    incomplete: bool,
) -> Result<(), ServiceError> {
    // PARTY is unconditionally an archetype root (`demographic.party.adoc`
    // §Invariants `Is_archetype_root`), so `Archetyped_valid`'s "a root MUST
    // carry ARCHETYPED" direction applies here — the one a per-node pass cannot
    // express. Root identity and `Links_valid` belong to the whole-instance
    // pass run below.
    if let Some(obj) = data.as_object() {
        crate::service::ehr::validation::validate_root_locatable(obj, rm_type)?;
    }

    // NOTE: Identities_valid (1..* → NonEmptyVec) and the present-implies-
    // non-empty family hold by construction at the strict typed door; the
    // `553|incomplete|` lane skips typed construction (master06 §Incomplete).

    // Relationships_validity, second arm (party.adoc): every inline
    // relationship's `source` must reference THIS party's CONTAINER id — RM
    // demographic master02 §Party Relationships requires `HIER_OBJECT_ID`s
    // there, while a served party's `uid` is the three-part
    // `OBJECT_VERSION_ID`, so it is reduced to its `object_id`.
    if let (Some(uid), Some(relationships)) = (
        data.pointer("/uid/value").and_then(Value::as_str),
        data.get("relationships").and_then(Value::as_array),
    ) {
        let container_id = ObjectVersionId::new(uid).map_or_else(
            |_| uid.to_owned(),
            |version_id| version_id.object_id().value().into_owned(),
        );
        for (i, rel) in relationships.iter().enumerate() {
            let source = rel.pointer("/source/id/value").and_then(Value::as_str);
            if source.is_some_and(|s| !composite_ids_equal(s, &container_id)) {
                return Err(ServiceError::content_invalid(
                    Violation::new(format!(
                        "must reference this party's version container \
                         ({container_id}) — relationships are stored under their \
                         source"
                    ))
                    .with_path(format!("{rm_type}.relationships[{i}].source"))
                    .with_invariant("PARTY.Relationships_validity"),
                ));
            }
        }
    }

    // enforce PARTY_REF.Type_validity on the demographic refs a party
    // body carries. ACTOR.roles is `List<PARTY_REF>` (actor.adoc); ROLE.performer
    // is `PARTY_REF` (role.adoc). BASE `party_ref.adoc §Type_validity`.
    if let Some(roles) = data.get("roles").and_then(Value::as_array) {
        for (i, role_ref) in roles.iter().enumerate() {
            validate_party_ref(role_ref, &format!("{rm_type}.roles[{i}]"))?;
        }
    }
    if rm_type == "ROLE"
        && let Some(performer) = data.get("performer").filter(|v| !v.is_null())
    {
        validate_party_ref(performer, "ROLE.performer")?;
    }

    // The checks above are root-scoped, but the RM class invariants bind every
    // node, so this whole-instance pass judges an identity, contact or nested
    // CLUSTER below the root by the rules a COMPOSITION's nodes get.
    crate::service::ehr::validation::validate_rm_invariants_for_commit(data, rm_type, incomplete)
}

/// Structurally validate a candidate `PARTY_RELATIONSHIP` body: deserialize into
/// the `openehr_rm` type (a strict-reader refusal → `400`), enforce that both
/// `source` and `target` `PARTY_REF`s are present continuant refs, and enforce
/// their `PARTY_REF.Type_validity`. `uid` need not be supplied — the server
/// injects it on read, mirroring the PARTY / COMPOSITION services.
///
/// # Errors
/// [`ServiceError::BadRequest`] when the strict reader refuses the body;
/// [`ServiceError::Unprocessable`] when a ref rule fails.
pub(crate) fn relationship_check(data: &Value, incomplete: bool) -> Result<(), ServiceError> {
    use openehr_base::prelude::ObjectId;
    use openehr_rm::prelude::PartyRelationship;
    // The decode is the validating ACT: `source`/`target` are mandatory
    // `PARTY_REF`s on the RM type, so once it succeeds their presence and shape
    // are facts of the type. A body that does NOT construct is handed to the
    // relaxed whole-instance pass, which enforces everything except presence.
    // NOTE: on a `553|incomplete|` commit the decode is not the gate —
    // mandatory `source`/`target` may be absent (RM common master06 §Incomplete
    // Content), which is exactly what the decode refuses.
    let decoded = openehr_its::json::from_canonical_value::<PartyRelationship>(data);
    let typed = match decoded {
        Ok(typed) => Some(typed),
        Err(_) if incomplete => None,
        // NOTE: the released `responses/422.yaml` scopes 422 to content that
        // "could be converted to a resource" — a body the strict reader
        // refuses is the 400 row, as on every direct commit route.
        Err(e) => {
            return Err(ServiceError::bad_request(
                format!("invalid canonical JSON body: {e}"),
                e,
            ));
        }
    };
    for (field, reference) in typed
        .iter()
        .flat_map(|t| [("source", &t.source), ("target", &t.target)])
    {
        // The refs denote the Version CONTAINER of a Party — an OBJECT_REF
        // carrying a HIER_OBJECT_ID (the continuant), never an
        // OBJECT_VERSION_ID (one particular version) — RM demographic
        // master02 §Modelling of Parties and Relationships.
        if matches!(reference.id, ObjectId::ObjectVersionId(_)) {
            return Err(ServiceError::content_invalid(
                Violation::new(
                    "must identify the party's version container (HIER_OBJECT_ID), \
                     not one version (OBJECT_VERSION_ID)",
                )
                .with_path(format!("PARTY_RELATIONSHIP.{field}.id"))
                .with_invariant("RM demographic master02"),
            ));
        }
        // PARTY_REF.Type_validity + OBJECT_REF.namespace (BASE
        // `party_ref.adoc` / `object_ref.adoc`), run on the ALREADY-DECODED
        // ref: no second decode, so no second way to answer the same question.
        let violations = reference.invariants();
        if !violations.is_empty() {
            return Err(ServiceError::content_invalid(
                Violation::new("fails its BASE class invariants")
                    .with_path(format!("PARTY_RELATIONSHIP.{field}"))
                    .with_causes(violations.into_iter().take(1).collect()),
            ));
        }
    }
    // `PARTY_RELATIONSHIP` is a LOCATABLE too
    // (`RM/docs/UML/classes/org.openehr.rm.demographic.party_relationship.adoc`),
    // so the same whole-instance RM class-invariant pass applies.
    crate::service::ehr::validation::validate_rm_invariants_for_commit(
        data,
        RELATIONSHIP_RM_TYPE,
        incomplete,
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A complete, RM-valid `PARTY_IDENTITY` (its LOCATABLE `name` +
    /// `archetype_node_id` and the mandatory `details` are 1..1 on the
    /// generated type).
    fn identity() -> Value {
        json!({
            "_type": "PARTY_IDENTITY",
            "name": { "_type": "DV_TEXT", "value": "legal name" },
            "archetype_node_id": "at0004",
            "details": {
                "_type": "ITEM_TREE",
                "name": { "_type": "DV_TEXT", "value": "details" },
                "archetype_node_id": "at0005",
                "items": []
            }
        })
    }

    fn person(identities: &Value) -> Value {
        // Root-valid per PARTY `Is_archetype_root` (party.adoc): the
        // ARCHETYPED block present, the root archetype_node_id equal to its
        // archetype_id — party_check enforces both before the PARTY rules.
        json!({
            "_type": "PERSON",
            "name": { "_type": "DV_TEXT", "value": "person" },
            "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
            "archetype_details": {
                "_type": "ARCHETYPED",
                "archetype_id": { "_type": "ARCHETYPE_ID",
                                   "value": "openEHR-DEMOGRAPHIC-PERSON.person.v1" },
                "rm_version": "1.1.0"
            },
            "identities": identities
        })
    }

    /// PARTY is unconditionally an archetype root (`party.adoc`
    /// `Is_archetype_root`): a commit without `archetype_details`, or with a
    /// root `archetype_node_id` differing from the declared `archetype_id`,
    /// is a 422 (`locatable.adoc` `Archetyped_valid` + `§archetype_node_id`).
    #[test]
    fn party_root_locatable_rules_are_enforced() {
        let mut no_details = person(&json!([identity()]));
        no_details
            .as_object_mut()
            .unwrap()
            .remove("archetype_details");
        let msg = match party_check("PERSON", &no_details, false) {
            Err(ServiceError::Unprocessable { violation: v, .. }) => v,
            other => panic!("expected Unprocessable, got {other:?}"),
        };
        assert_eq!(msg.path(), Some("PERSON.archetype_details"));

        // The root-identity half is the whole-instance pass's
        // (`openehr_rm::v1_2::validate::check_archetyped_valid`), so it
        // surfaces as the structured `ValidationFailed` report.
        let mut mismatched = person(&json!([identity()]));
        mismatched["archetype_node_id"] = json!("openEHR-DEMOGRAPHIC-PERSON.other.v1");
        let err = party_check("PERSON", &mismatched, false)
            .expect_err("a contradicting root identity is rejected");
        assert!(
            format!("{err:?}").contains("archetype_node_id"),
            "got {err:?}"
        );
    }

    /// `PARTY.Identities_valid` is enforced BY THE DECODE — `identities` is
    /// `NonEmptyVec<PARTY_IDENTITY>` on the generated party types, so an empty
    /// or absent list has no representation. The refusal is asserted here (it
    /// must not weaken); the service carries no second check of the same rule.
    /// The refusal class is `BadRequest`: the released `responses/422.yaml`
    /// scopes 422 to content that "could be converted to a resource", and an
    /// unconstructible body is the 400 row on every commit route.
    #[test]
    fn identities_valid_is_enforced() {
        for bad in [person(&json!([])), json!({ "_type": "PERSON" })] {
            let err = party_check("PERSON", &bad, false)
                .expect_err("an unconstructible body must be refused");
            let api = openehr_its::rest::runtime::ApiError::from(err);
            assert_eq!(
                api.status(),
                http::StatusCode::BAD_REQUEST,
                "an unconstructible body must be 400, got {api:?}"
            );
            assert!(
                api.to_string().contains("invalid canonical JSON body"),
                "the decode is the enforcement point, got {api}"
            );
        }
        party_check("PERSON", &person(&json!([identity()])), false)
            .expect("a party with one identity is valid");
    }

    #[test]
    fn present_but_empty_lists_are_rejected() {
        let mut body = person(&json!([identity()]));
        body["contacts"] = json!([]);
        let err = party_check("PERSON", &body, false)
            .expect_err("present-but-empty contacts must refuse (#1730 parse class)");
        assert!(
            format!("{err:?}").contains("contacts")
                && format!("{err:?}").contains("at least one member"),
            "got {err:?}"
        );
    }

    /// a `PARTY_REF` whose `type` is outside the legal set is a `422`
    /// (`PARTY_REF.Type_validity`); a legal supertype (`ACTOR`) is accepted.
    #[test]
    fn party_ref_type_validity_is_enforced() {
        let good = json!({ "_type": "PARTY_REF", "namespace": "demographic",
            "type": "PERSON", "id": { "_type": "HIER_OBJECT_ID", "value": "x" } });
        validate_party_ref(&good, "ctx").expect("PERSON is a legal PARTY_REF type");
        let actor = json!({ "_type": "PARTY_REF", "namespace": "local",
            "type": "ACTOR", "id": { "_type": "HIER_OBJECT_ID", "value": "x" } });
        validate_party_ref(&actor, "ctx").expect("ACTOR supertype is legal");
        let bad_type = json!({ "_type": "PARTY_REF", "namespace": "demographic",
            "type": "COMPOSITION", "id": { "_type": "HIER_OBJECT_ID", "value": "x" } });
        match validate_party_ref(&bad_type, "ctx") {
            Err(ServiceError::Unprocessable { violation: v, .. }) => assert!(
                v.causes()
                    .iter()
                    .any(|c| c.message.contains("Type_validity")),
                "causes must carry Type_validity, got {:?}",
                v.causes()
            ),
            other => panic!("expected Type_validity 422, got {other:?}"),
        }
    }

    /// The accepting twin for the universal supertype `ANY`: the demographic
    /// write boundary judges `PARTY_REF.type` by the one realization in
    /// `openehr-base`, whose adjudicated set admits `ANY` (the value the CNF
    /// positive commit corpus writes into `PARTY_IDENTIFIED.external_ref` —
    /// `CNF/tests/platform/robot/_resources/test_data_sets/compositions/TDD/persistent_minimal.en.v1__full.xml`).
    /// Before this crate consumed that realization it kept its own list and
    /// refused `ANY`, so the two disagreed.
    #[test]
    fn party_ref_any_supertype_is_accepted() {
        let any = json!({ "_type": "PARTY_REF", "namespace": "local",
            "type": "ANY", "id": { "_type": "HIER_OBJECT_ID", "value": "x" } });
        validate_party_ref(&any, "ctx").expect("ANY is admitted by PARTY_REF.Type_validity");
    }

    /// an `OBJECT_REF.namespace` that is empty or violates the standard
    /// regex is a `422` (`OBJECT_REF.namespace`).
    #[test]
    fn object_ref_namespace_legality_is_enforced() {
        let bad_ns = json!({ "_type": "PARTY_REF", "namespace": "1nope",
            "type": "PERSON", "id": { "_type": "HIER_OBJECT_ID", "value": "x" } });
        match validate_party_ref(&bad_ns, "ctx") {
            Err(ServiceError::Unprocessable { violation: v, .. }) => {
                assert!(
                    v.causes()
                        .iter()
                        .any(|c| c.message.contains("Namespace_valid")),
                    "causes must carry Namespace_valid, got {:?}",
                    v.causes()
                );
            }
            other => panic!("expected namespace 422, got {other:?}"),
        }
    }

    /// The mandatory `OBJECT_REF` attributes (`id`, `namespace`, `type` all
    /// 1..1 — BASE `object_ref.adoc` §Attributes) are enforced by decoding the
    /// reference into the generated type, so a ref missing one is a `422`
    /// instead of passing a partial hand check.
    #[test]
    fn party_ref_mandatory_attributes_are_enforced() {
        let no_id = json!({ "_type": "PARTY_REF", "namespace": "local", "type": "PERSON" });
        assert!(matches!(
            validate_party_ref(&no_id, "ctx"),
            Err(ServiceError::Unprocessable { .. })
        ));
        let no_type = json!({ "_type": "PARTY_REF", "namespace": "local",
            "id": { "_type": "HIER_OBJECT_ID", "value": "x" } });
        assert!(matches!(
            validate_party_ref(&no_type, "ctx"),
            Err(ServiceError::Unprocessable { .. })
        ));
    }

    /// A ROLE.performer / ACTOR.roles ref is checked for `Type_validity`.
    #[test]
    fn role_performer_ref_is_validated() {
        let role = |performer_type: &str| {
            json!({
                "_type": "ROLE",
                "name": { "_type": "DV_TEXT", "value": "role" },
                "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.role.v1",
                "archetype_details": { "_type": "ARCHETYPED",
                    "archetype_id": { "_type": "ARCHETYPE_ID",
                                       "value": "openEHR-DEMOGRAPHIC-ROLE.role.v1" },
                    "rm_version": "1.1.0" },
                "identities": [identity()],
                "performer": { "_type": "PARTY_REF", "namespace": "demographic",
                    "type": performer_type,
                    "id": { "_type": "HIER_OBJECT_ID", "value": "performer-id" } }
            })
        };
        party_check("ROLE", &role("PERSON"), false).expect("a legal performer ref is accepted");
        match party_check("ROLE", &role("COMPOSITION"), false) {
            Err(ServiceError::Unprocessable { violation: v, .. }) => assert!(
                v.causes()
                    .iter()
                    .any(|c| c.message.contains("Type_validity")),
                "causes must carry Type_validity, got {:?}",
                v.causes()
            ),
            other => panic!("expected performer Type_validity 422, got {other:?}"),
        }
    }

    /// The whole-instance RM class-invariant pass reaches BELOW a party root:
    /// `ARCHETYPED.Rm_version_valid` (RM common
    /// `org.openehr.rm.common.archetyped.adoc` §Invariants) and
    /// `LOCATABLE.Links_valid` (`…common.locatable.adoc` §Invariants) on a
    /// nested identity are both 422s, and the valid twins are accepted.
    #[test]
    fn party_rm_invariants_below_the_root_are_enforced() {
        party_check("PERSON", &person(&json!([identity()])), false)
            .expect("the baseline person is valid");

        let mut empty_rm_version = person(&json!([identity()]));
        empty_rm_version["archetype_details"]["rm_version"] = json!("");
        let err = party_check("PERSON", &empty_rm_version, false)
            .expect_err("an empty rm_version must be refused");
        assert!(
            format!("{err:?}").contains("Rm_version_valid"),
            "the refusal should name the invariant, got {err:?}"
        );

        let mut nested_links = person(&json!([identity()]));
        nested_links["identities"][0]
            .as_object_mut()
            .unwrap()
            .insert("links".into(), json!([]));
        let err = party_check("PERSON", &nested_links, false)
            .expect_err("an empty links list must be refused");
        assert!(
            format!("{err:?}").contains("links")
                && format!("{err:?}").contains("at least one member"),
            "the refusal names the empty container (#1730 parse class), got {err:?}"
        );
    }
}
