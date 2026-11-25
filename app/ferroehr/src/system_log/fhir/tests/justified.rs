// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The FHIR `AuditEvent` of an access that rested on a justification of the
//! access policy (#6587).

use super::*;

/// An access that rested on an emergency justification (EHDS Art. 11(5))
/// renders the `ehds-logging` entity that states it in words, beside the
/// declared purpose; an access that rested on a non-emergency justification,
/// or on none, renders no such entity, whatever purpose it declared.
#[test]
fn the_emergency_mark_renders_as_its_entity() {
    use crate::access_eval::justification::{Justification, Reason};
    let marking = ctx();
    let mut e = event(
        EventActionCode::Read,
        ObjectClass::Composition,
        EventOutcome::Success,
    );
    e.purpose = Some("ETREAT".to_owned());
    e.category = Some(Classifier::default().classify(&AccessedContent::Objects(Vec::new()), false));
    let mark = |v: &serde_json::Value| {
        v["entity"]
            .as_array()
            .expect("entities")
            .iter()
            .find(|entity| entity["name"] == ehds_logging::balp::EMERGENCY_ENTITY)
            .cloned()
    };
    let unmarked = to_fhir(&e, &marking, None).expect("render");
    assert!(mark(&unmarked).is_none(), "{unmarked}");
    let reason = Reason::new("unconscious on arrival").expect("a reason");
    e.justification = Some(Box::new(Justification {
        emergency: false,
        reason: reason.clone(),
    }));
    let justified = to_fhir(&e, &marking, None).expect("render");
    assert!(mark(&justified).is_none(), "{justified}");

    e.justification = Some(Box::new(Justification {
        emergency: true,
        reason,
    }));
    let rendered = to_fhir(&e, &marking, None).expect("render");
    let entity = mark(&rendered).expect("the mark");
    assert_eq!(
        entity["description"],
        ehds_logging::balp::EMERGENCY_DESCRIPTION
    );
    assert_eq!(
        agent(&rendered, "john doe")["purposeOfUse"][0]["coding"][0]["code"],
        "ETREAT",
        "the declared code stays on the requesting agent"
    );
}
