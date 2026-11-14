// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A Basic user is a credential alone (#5438): it holds no scope or role of
//! its own, and `[authz.rbac] first_admin` names the one Basic user that holds
//! `admin_role` from the configuration. No openEHR spec governs
//! authentication mechanics: our own design.

use super::*;

#[tokio::test]
async fn valid_basic() {
    let p = basic_only()
        .await
        .authenticate(&headers("Basic YWxpY2U6cHc="))
        .await
        .expect("ok");
    assert_eq!(p.principal.subject, "alice");
    assert_eq!(p.principal.method, AuthMethod::Basic);
    assert!(
        p.principal.scopes.is_empty(),
        "a Basic user is a credential alone; its scopes are grants of its PERSON"
    );
    assert_eq!(
        p.principal.roles,
        Vec::<String>::new(),
        "a Basic user granted no role holds none"
    );
    assert!(
        p.fresh,
        "a first Basic verification is a genuine authentication"
    );
}

/// The roles `alice` holds under `config`.
async fn roles_of_alice(config: &AppConfig) -> Vec<String> {
    Authenticator::new(config, None, memory(), test_support::standing())
        .await
        .expect("a Basic-only authenticator")
        .authenticate(&headers("Basic YWxpY2U6cHc="))
        .await
        .expect("ok")
        .principal
        .roles
}

/// `[authz.rbac] first_admin` is the one Basic user that holds
/// `admin_role` from the configuration, upper-cased; another Basic user
/// holds no role of its own.
#[tokio::test]
async fn the_first_admin_holds_the_admin_role_upper_cased() {
    let mut config = basic_config(60);
    config.rbac.admin_role = "admin".to_owned();
    config.rbac.first_admin = Some("alice".to_owned());
    assert_eq!(roles_of_alice(&config).await, vec!["ADMIN".to_owned()]);
    config.rbac.first_admin = Some("bob".to_owned());
    let roles = roles_of_alice(&config).await;
    assert!(roles.is_empty(), "{roles:?}");
}
