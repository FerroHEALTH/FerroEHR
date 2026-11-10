// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The purposes of use and the roles an access token states, as the renderings
//! write them.

use super::{agent, ctx, event};
use crate::system_log::event::{EventActionCode, EventOutcome, ObjectClass};
use crate::system_log::fhir::to_fhir;

/// #5323: the purposes of use and the professional roles the access token
/// states reach the requesting agent of the BALP `AuditEvent` with their code
/// systems, beside the declared purpose and the roles the caller held; a
/// declared purpose the token also states is written once.
#[test]
fn the_token_purposes_and_roles_reach_the_requesting_agent() {
    use crate::system_log::event::{Coding, TokenAttributes};
    const ACT_REASON: &str = "http://terminology.hl7.org/CodeSystem/v3-ActReason";
    // The SNOMED CT concept the role is coded by (medical practitioner).
    const PRACTITIONER: &str = "158965000"; // privacy-allow: a SNOMED CT concept id, not a BSN
    let mut e = event(
        EventActionCode::Read,
        ObjectClass::Composition,
        EventOutcome::Success,
    );
    e.roles = vec!["CLINICIAN".to_owned()];
    e.purpose = Some("TREAT".to_owned());
    e.token = Some(TokenAttributes {
        purposes: vec![
            Coding {
                system: Some(ACT_REASON.to_owned()),
                code: "TREAT".to_owned(),
            },
            Coding {
                system: Some(ACT_REASON.to_owned()),
                code: "ETREAT".to_owned(),
            },
        ],
        roles: vec![Coding {
            system: Some("urn:oid:2.16.840.1.113883.6.96".to_owned()),
            code: PRACTITIONER.to_owned(),
        }],
    });
    let rendered = to_fhir(&e, &ctx(), None).expect("render");
    let user = agent(&rendered, "john doe");
    let purposes: Vec<(&str, &str)> = user["purposeOfUse"]
        .as_array()
        .expect("the purposes")
        .iter()
        .map(|purpose| {
            (
                purpose["coding"][0]["system"].as_str().unwrap_or_default(),
                purpose["coding"][0]["code"].as_str().unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(
        purposes,
        vec![(ACT_REASON, "TREAT"), (ACT_REASON, "ETREAT")],
        "{user}"
    );
    let roles = user["role"].to_string();
    assert!(
        roles.contains("CLINICIAN") && roles.contains(PRACTITIONER),
        "{roles}"
    );
}
