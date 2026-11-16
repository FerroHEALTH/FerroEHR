// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A request's capabilities share the grant copy's client and credentials
//! with every other request's, never a copy of them (#5754).

use std::sync::Arc;

use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::LiveGrants;
use crate::party_binding::grants::caller::store::Changed;
use crate::party_binding::grants::capabilities::Actor;
use crate::party_binding::grants::operation::Operation;
use crate::party_binding::grants::store::{CapabilityRow, ClientRow, Dates, Loaded, Term};
use crate::party_binding::policy_mark::TERMINOLOGY;

const ISSUER: &str = "https://issuer.example";
const CLIENT: &str = "relay";

/// A copy holding one client `ACTOR` that performs a `ROLE` granting
/// `commit_composition` with a `template` credential.
fn live() -> LiveGrants {
    let pool = PgPool::connect_lazy("postgres://localhost/unused").expect("a lazy pool");
    let live = LiveGrants::new(pool);
    let party = Uuid::from_u128(7);
    let credentials = json!({"_type": "ITEM_TREE", "items": [
        {"_type": "ELEMENT", "archetype_node_id": "at1",
         "name": {"_type": "DV_CODED_TEXT", "value": "template",
                  "defining_code": {"terminology_id": {"value": TERMINOLOGY},
                                    "code_string": "template"}},
         "value": {"_type": "DV_TEXT", "value": "T.v1"}}
    ]});
    live.install(Loaded {
        revision: 1,
        clients: vec![ClientRow {
            party_id: party,
            issuer: ISSUER.to_owned(),
            client_id: CLIENT.to_owned(),
            lifecycle_state: "532".to_owned(),
            provider_id: None,
            provider: None,
        }],
        capabilities: vec![CapabilityRow {
            role_id: Uuid::from_u128(8),
            performer_id: party,
            operation: Operation::CommitComposition.code().to_owned(),
            term: Term {
                lifecycle_state: "532".to_owned(),
                role_dates: Some(Dates {
                    lower: None,
                    upper: None,
                }),
                dates: Some(Dates {
                    lower: None,
                    upper: None,
                }),
                not_before: None,
                not_after: None,
            },
            credentials: Some(Arc::new(credentials)),
        }],
        facts: Changed::default(),
    });
    live
}

#[test]
fn two_requests_share_the_client_and_the_credentials() {
    let live = live();
    let actor = Actor::Client {
        issuer: ISSUER,
        client_id: CLIENT,
    };
    let now = jiff::Timestamp::now();
    let first = live.capabilities(actor, now);
    let second = live.capabilities(actor, now);
    let template = |capabilities: &crate::party_binding::grants::capabilities::Capabilities| {
        let grant = capabilities
            .of(Operation::CommitComposition)
            .next()
            .expect("the grant is in force");
        let [text] = grant.credentials().texts("template")[..] else {
            panic!("one template credential");
        };
        text.as_ptr()
    };
    assert_eq!(
        template(&first),
        template(&second),
        "both requests read the one credentials tree the copy holds"
    );
    let (Some(a), Some(b)) = (first.client(), second.client()) else {
        panic!("the client ACTOR resolves by its issuer and client_id");
    };
    assert!(
        std::ptr::eq(a, b),
        "both requests read the one client the copy holds"
    );
    assert!(
        live.capabilities(
            Actor::Client {
                issuer: ISSUER,
                client_id: "other",
            },
            now
        )
        .client()
        .is_none(),
        "another client_id of the issuer names no ACTOR"
    );
}
