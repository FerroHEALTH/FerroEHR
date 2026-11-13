// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The grants read by one scope element (#5439): a `ROLE` whose capability
//! grants `data_origin`, `read_change_feed` or `scim_provision` is refused at
//! its commit unless it names the element that grant is read by exactly once,
//! a reader in the registry's grammar and a SCIM client with a token scope.
//! No openEHR spec gives a capability an authorization meaning: our own
//! design.

use ferroehr::error::ServiceError;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;

use crate::policy_fixtures::{capability, create, granted, identified, role, scope};

#[tokio::test]
async fn a_grant_read_by_one_element_names_it_once() {
    let db = testkit::db().await.expect("testkit database");
    let svc = crate::policy_fixtures::granting(FerroEhrService::new(
        &ferroehr::db::domain::DomainPools::from_shared(&db.pool()),
    ))
    .await;
    let (agent, _) = granted(
        &svc,
        "connected",
        &[capability(
            "scim_provision",
            &[
                scope("binding_issuer", "urn:example:login"),
                scope("token_scope", "ferroehr/scim"),
            ],
        )],
    )
    .await;
    for (refused, element) in [
        (capability("data_origin", &[]), "0 origin elements"),
        (
            capability(
                "data_origin",
                &[
                    identified("origin", "lab.example"),
                    scope("origin", "pacs.example"),
                ],
            ),
            "2 origin elements",
        ),
        (
            capability("data_origin", &[scope("origin", "lab example")]),
            "carries whitespace",
        ),
        (
            capability("read_change_feed", &[scope("reader", "Bad Name")]),
            "is not a reader name",
        ),
        (capability("read_change_feed", &[]), "0 reader elements"),
        (
            capability(
                "scim_provision",
                &[scope("binding_issuer", "urn:example:login")],
            ),
            "names no token_scope",
        ),
        (
            capability("scim_provision", &[scope("token_scope", "ferroehr/scim")]),
            "0 binding_issuer elements",
        ),
    ] {
        let error = create(&svc, PartyKind::Role, &role(agent, &[refused]))
            .await
            .expect_err(element);
        let ServiceError::BadRequest(refusal) = &error else {
            panic!("{element}: {error:?}");
        };
        assert!(refusal.to_string().contains(element), "{element}: {refusal}");
    }
    create(
        &svc,
        PartyKind::Role,
        &role(
            agent,
            &[
                capability("data_origin", &[identified("origin", "lab.example")]),
                capability("read_change_feed", &[scope("reader", "ferrobridge")]),
            ],
        ),
    )
    .await
    .expect("one element each commits");
}
