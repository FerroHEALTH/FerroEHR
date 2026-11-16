// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A request's capabilities share the grant copy's client and credentials
//! with every other request's, never a copy of them (#5754).

use std::sync::Arc;

use openehr_base::prelude::TerminologyId;
use openehr_rm::prelude::{CodePhrase, DvCodedText, DvText, DvTextData};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::LiveGrants;
use crate::party_binding::grants::caller::store::Changed;
use crate::party_binding::grants::capabilities::Actor;
use crate::party_binding::grants::operation::{Operation, TEMPLATE};
use crate::party_binding::grants::store::{CapabilityRow, ClientRow, Dates, Loaded, Term};
use crate::party_binding::policy_mark::TERMINOLOGY;

const ISSUER: &str = "https://issuer.example";
const CLIENT: &str = "relay";

/// A copy holding one client `ACTOR` that performs a `ROLE` granting
/// `commit_composition` with a `template` credential.
#[expect(
    clippy::disallowed_types,
    reason = "a grant row stores its credentials as the canonical JSON tree the fixture builds"
)]
fn live() -> LiveGrants {
    let pool = PgPool::connect_lazy("postgres://localhost/unused").expect("a lazy pool");
    let live = LiveGrants::new(pool);
    let party = Uuid::from_u128(7);
    let name = openehr_its::json::to_canonical_value(&DvCodedText {
        value: TEMPLATE.to_owned(),
        hyperlink: None,
        formatting: None,
        mappings: None,
        language: None,
        encoding: None,
        defining_code: CodePhrase {
            terminology_id: TerminologyId {
                value: TERMINOLOGY.to_owned(),
            },
            code_string: TEMPLATE.to_owned(),
            preferred_term: None,
        },
    });
    let value = openehr_its::json::to_canonical_value(&DvText::DvText(DvTextData {
        value: "T.v1".to_owned(),
        hyperlink: None,
        formatting: None,
        mappings: None,
        language: None,
        encoding: None,
    }));
    let credentials =
        json!({"items": [{"archetype_node_id": "at1", "name": name, "value": value}]});
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

#[tokio::test]
async fn two_requests_share_the_client_and_the_credentials() {
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
        let [text] = grant.credentials().texts(TEMPLATE)[..] else {
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
