// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The stored form of a CONTRIBUTION member (#6003): an unsigned member is
//! stored as its typed canonical rendering, the bytes the direct commit of the
//! same COMPOSITION stores, and a member its client signed keeps the bytes it
//! was received as, so that signature still covers them. No openEHR spec
//! governs the stored form beyond canonical JSON: our own design.

use ferroehr::ids::EhrId;
use ferroehr::service::FerroEhrService;
use serde_json::{Value, json};

use crate::fixtures::{commit_audit, composition, first_version_uid, uv, vo_of};

/// `value` without the `_type` of every object whose slot declares one
/// concrete type, the members a client may leave out.
fn without_implied_types(value: &mut Value) {
    const IMPLIED: [&str; 4] = [
        "ARCHETYPED",
        "ARCHETYPE_ID",
        "CODE_PHRASE",
        "TERMINOLOGY_ID",
    ];
    match value {
        Value::Object(map) => {
            if map
                .get("_type")
                .and_then(Value::as_str)
                .is_some_and(|t| IMPLIED.contains(&t))
            {
                map.shift_remove("_type");
            }
            map.values_mut().for_each(without_implied_types);
        }
        Value::Array(items) => items.iter_mut().for_each(without_implied_types),
        _ => {}
    }
}

/// A CONTRIBUTION of one creation member carrying `data`, signed by its
/// client with `signature` when one is given.
fn contribution(data: &Value, signature: Option<&str>) -> Value {
    let mut member = json!({
        "data": data,
        "lifecycle_state": crate::fixtures::lifecycle("532"),
        "commit_audit": commit_audit("249", "creation", "T"),
    });
    if let Some(signature) = signature {
        member["signature"] = json!(signature);
    }
    json!({ "versions": [member], "audit": commit_audit("249", "creation", "T") })
}

/// The stored body of the COMPOSITION `vo` of `ehr_id` as the JSON read
/// serves it, without its version `uid`, in its stored member order.
async fn stored(svc: &FerroEhrService, ehr_id: EhrId, vo: &str) -> (String, Value) {
    let read = svc
        .read_composition_raw(
            ehr_id,
            ferroehr::ids::VoId(vo.parse().expect("a vo uuid")),
            None,
        )
        .await
        .expect("the composition reads");
    let mut body = read.body.into_value().expect("the stored body is JSON");
    body.as_object_mut().expect("an object").shift_remove("uid");
    (serde_json::to_string(&body).expect("serializes"), body)
}

/// An unsigned member that leaves out the `_type` its slots imply stores the
/// bytes the direct commit of the same COMPOSITION stores.
#[tokio::test]
async fn an_unsigned_member_stores_what_the_direct_commit_stores() {
    let (_db, svc) = crate::fixtures::service().await;
    let ehr_id = crate::fixtures::create_ehr(&svc).await;
    let mut sent = composition("obs");
    without_implied_types(&mut sent);
    let resp = svc
        .create_ehr_contribution(ehr_id, contribution(&sent, None))
        .await
        .expect("the contribution commits");
    let via_contribution = first_version_uid(&resp.body);
    let direct = svc
        .create_composition(ehr_id, uv(&composition("obs"), "249", None))
        .await
        .expect("the direct commit")
        .vo_id
        .0
        .to_string();
    let (contributed, _) = stored(&svc, ehr_id, vo_of(&via_contribution)).await;
    let (committed, _) = stored(&svc, ehr_id, &direct).await;
    assert_eq!(contributed, committed);
    assert!(
        contributed.contains(r#""_type":"CODE_PHRASE""#),
        "{contributed}"
    );
}

/// A member its client signed keeps the bytes it was received as, and its
/// signature is stored verbatim, so the signature still covers what is served.
#[tokio::test]
async fn a_signed_member_keeps_its_received_bytes() {
    let (_db, svc) = crate::fixtures::service().await;
    let ehr_id = crate::fixtures::create_ehr(&svc).await;
    let mut sent = composition("signed");
    without_implied_types(&mut sent);
    let resp = svc
        .create_ehr_contribution(ehr_id, contribution(&sent, Some("client-signature")))
        .await
        .expect("the contribution commits");
    let ovid = first_version_uid(&resp.body);
    let (_, body) = stored(&svc, ehr_id, vo_of(&ovid)).await;
    assert_eq!(body, sent);
    assert_eq!(
        serde_json::to_string(&body).unwrap(),
        serde_json::to_string(&sent).unwrap(),
        "the member order is the client's"
    );
    let envelope = svc
        .composition_version_envelope(ehr_id, ovid.parse().expect("ovid"))
        .await
        .expect("the version reads");
    assert_eq!(envelope.body["signature"], "client-signature");
    let mut data = envelope.body["data"].clone();
    data.as_object_mut().expect("an object").shift_remove("uid");
    assert_eq!(
        openehr_rm::v1_2::common::change_control::version_impl::canonical_form_of_json_with_data(
            &json!({}),
            &data
        )
        .unwrap(),
        openehr_rm::v1_2::common::change_control::version_impl::canonical_form_of_json_with_data(
            &json!({}),
            &sent
        )
        .unwrap(),
        "the signed content is the received content"
    );
}
