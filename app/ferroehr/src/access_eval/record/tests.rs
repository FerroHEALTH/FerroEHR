// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access record's grounds, steps and care ground.

use openehr_rm::draft::access_control::terminology::ForeignCode;

use super::care::care_ground;
use super::*;
use crate::access_eval::care_tie::Grant;
use crate::party_binding::grants::caller::care_of::{CarerKind, ProviderKey};
use crate::party_binding::grants::caller::care_ref::Cover;
use crate::system_log::event::{EventActionCode, EventOutcome, ObjectClass};

/// The versions the stored entry's grounds name come back as the
/// policies every projection writes after the data permit, and a ground
/// defined elsewhere names none.
#[test]
fn the_stored_entry_gives_back_the_policies_of_its_grounds() {
    let mut event = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Composition,
        EventOutcome::Success,
    );
    "dr-b".clone_into(&mut event.user_id);
    event.ehr_id = Some("7d44b88c-4199-4bad-97dc-d78268e01398".to_owned());
    event.permit = Some("hdab-nl/2026/0042".to_owned());
    let ground = |name: &str, version: &str| AccessAuthorisation {
        name: Some(name.to_owned()),
        version: Some(version.to_owned()),
        ..authorisation(Authorisation::Consent)
    };
    let grounds = [
        ground(EHR_ACCESS, "ehr-access::cdr::2"),
        ground("agreement-of-the-region", "4"),
        ground(ACCESS_POLICY, "policy::cdr::3"),
        ground(EHR_ACCESS, "ehr-access::cdr::2"),
    ];
    event.decided = Some(Box::new(Decision {
        refused: None,
        policies: policies_of(&grounds),
        grounds: grounds.to_vec(),
        confidentiality: None,
    }));
    let retention = crate::access_log::retention::stated(
        None,
        &crate::system_log::retention::LogRetention::default(),
    );
    let entry =
        crate::access_log::entry::entry_of(&event, None, "cdr", retention).expect("an entry");
    let stored = crate::access_log::recorded::stored(&entry, 1).expect("it reads back");
    assert_eq!(
        stored.event.policies(),
        ["hdab-nl/2026/0042", "ehr-access::cdr::2", "policy::cdr::3"]
    );
}

/// The care ground names the carer a relationship covers for by the
/// issuer, id and kind its `covers_for` carries (#6171).
#[test]
fn the_care_ground_names_the_kind_of_the_carer_it_covers_for() {
    let covered = Uuid::now_v7().to_string();
    for kind in CarerKind::ALL {
        let grant = Grant {
            relationship: None,
            version: None,
            carer: Uuid::now_v7(),
            carer_kind: Some(CarerKind::Person),
            provider: ProviderKey::new("urn:example:providers", "hospital"),
            in_grace: false,
            covers_for: Some(Cover::new(ProviderKey::new("local", &covered), kind)),
            episode: None,
        };
        let role = ForeignCode {
            terminology: "HL7_ParticipationType".to_owned(),
            code: "RESP".to_owned(),
        };
        let tie = Tie {
            path: CarePath::Direct,
            role,
            grant: Some(std::sync::Arc::new(grant)),
        };
        let cover = care_ground(&tie, "local").and_then(|ground| ground.covers_for);
        let expected = DvIdentifier {
            issuer: Some("local".to_owned()),
            assigner: None,
            id: covered.clone(),
            r#type: Some(kind.rm_type().to_owned()),
        };
        assert_eq!(cover, Some(expected), "{kind:?}");
    }
}

/// Every refusal code belongs to the step that answers it.
#[test]
fn each_refusal_belongs_to_its_step() {
    let steps: Vec<(u8, &str)> = Refusal::ALL
        .iter()
        .map(|refusal| (Step::of(*refusal).number(), refusal.code()))
        .collect();
    for (number, code) in steps {
        let expected = match code {
            "refused-opt-out" => 1,
            "refused-restricted" => 2,
            "refused-consent-denied" => 3,
            "refused-purpose-missing" => 4,
            "refused-policy-denied" | "refused-conflict" => 5,
            "refused-sensitivity" | "refused-confidentiality" => 6,
            "refused-no-care-relationship" => 7,
            "refused-consent-required" => 8,
            "refused-not-subject"
            | "refused-not-represented"
            | "refused-consent-reference-missing"
            | "refused-excluded" => 0,
            other => panic!("an unknown refusal code {other}"),
        };
        assert_eq!(number, expected, "{code}");
    }
}
