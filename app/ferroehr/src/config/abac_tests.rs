// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The keys of the removed remote policy decision point are unknown keys.
//!
//! The remote decision point, `[authz.abac.remote]` with its endpoint tree,
//! and the `[authz.abac.policy.<name>]` bindings it read are gone (#5516).
//! FerroEHR is greenfield, so each is refused as any unknown key is, from the
//! file and from the environment. No openEHR spec governs authorization: our
//! own design.

use super::tests::{env, file_refusal};
use super::*;

/// Every key of the removed tree, by table: the table, whether it is an array
/// of tables, and its keys.
const REMOVED: &[(&str, bool, &[&str])] = &[
    (
        "authz.abac.remote.endpoint",
        false,
        &["url", "timeout", "connect_timeout"],
    ),
    (
        "authz.abac.remote.endpoint.retry",
        false,
        &["max_attempts", "initial_backoff", "max_backoff"],
    ),
    (
        "authz.abac.remote.endpoint.tls",
        false,
        &[
            "client_identity",
            "client_identity_file",
            "trust_roots_file",
            "min_version",
        ],
    ),
    (
        "authz.abac.remote.endpoint.credentials",
        false,
        &["user", "password", "password_file"],
    ),
    (
        "authz.abac.remote.endpoint.credentials.oauth",
        false,
        &[
            "issuer",
            "token_endpoint",
            "client_id",
            "client_auth",
            "client_secret",
            "client_secret_file",
            "private_key_file",
            "kid",
            "assertion_alg",
            "scope",
            "resource",
            "audience",
            "refresh_margin",
            "timeout",
            "assertion_audience",
            "dpop_key_file",
            "certificate_bound",
            "security_profile",
            "mtls_alias_hosts",
            "previous_private_key_file",
            "previous_key_overlap",
            "next_private_key_file",
            "authorization_details",
        ],
    ),
    (
        "authz.abac.remote.endpoint.credentials.oauth.tls",
        false,
        &[
            "client_identity",
            "client_identity_file",
            "trust_roots_file",
            "min_version",
        ],
    ),
    (
        "authz.abac.remote.endpoint.credentials.onward",
        false,
        &[
            "issuer",
            "kind",
            "profile",
            "token_endpoint",
            "client_id",
            "audiences",
            "indicators",
            "scope",
            "correlation_claim",
            "cache",
            "actor_assertion",
        ],
    ),
    (
        "authz.abac.remote.endpoint.credentials.onward.keys",
        false,
        &[
            "person_key",
            "tenant_claim",
            "client_claims",
            "impersonation_marker",
            "expect",
            "user_claims",
            "person_equality",
            "unstable_claims",
            "first_party_clients",
            "max_actor_chain",
            "act_represents_patient",
            "token_types",
            "required_claims",
            "adapter",
            "max_onward_lifetime",
            "onward_grant",
            "onward_indicator",
            "onward_parameters",
            "onward_client_id",
            "onward_subject_audience",
            "onward_preview",
        ],
    ),
    (
        "authz.abac.remote.endpoint.credentials.onward.keys.app_only",
        true,
        &[
            "claim",
            "value",
            "absent_claim",
            "subjects",
            "sub_is_client",
            "differs_from_iss",
        ],
    ),
    (
        "authz.abac.remote.endpoint.credentials.onward.keys.guest",
        true,
        &[
            "claim",
            "value",
            "absent_claim",
            "subjects",
            "sub_is_client",
            "differs_from_iss",
        ],
    ),
    (
        "authz.abac.remote.endpoint.credentials.onward.keys.delegates",
        true,
        &["issuer", "sub", "client"],
    ),
    (
        "authz.abac.remote.endpoint.credentials.onward.keys.representation",
        false,
        &["claim", "ground_claim"],
    ),
    (
        "authz.abac.remote.endpoint.credentials.onward.keys.scope_claim",
        false,
        &["claim", "form"],
    ),
    (
        "authz.abac.remote.endpoint.credentials.onward.keys.app_only_scope_claim",
        false,
        &["claim", "form"],
    ),
    (
        "authz.abac.remote.endpoint.credentials.onward.federation",
        false,
        &[
            "issuer",
            "contact_point",
            "mapping",
            "fields",
            "countries",
            "issuing_authorities",
            "provider_identifier_prefixes",
            "attests_authorization_details",
            "authorization_details_type",
            "organisation_types",
            "presentations_claim",
            "user_credential_types",
            "required_fields",
            "requires_sender_constraint",
            "exchanges_from",
            "person_mapping",
        ],
    ),
    (
        "authz.abac.remote.endpoint.credentials.onward.assurance",
        false,
        &["claim", "levels", "minimum"],
    ),
    ("authz.abac.policy.ehr", false, &["name", "parameters"]),
];

/// The removed table a key sits under, as the refusal names it.
fn removed_table(table: &str) -> &'static str {
    if table.starts_with("authz.abac.policy") {
        "policy"
    } else {
        "remote"
    }
}

#[test]
fn every_removed_key_is_unknown_in_the_file() {
    for (table, array, keys) in REMOVED {
        let header = if *array {
            format!("[[{table}]]")
        } else {
            format!("[{table}]")
        };
        for key in *keys {
            let refusal = file_refusal(&format!("{header}\n{key} = \"x\"\n"));
            assert!(
                refusal.contains(removed_table(table)),
                "{table}.{key} is refused as unknown: {refusal}"
            );
        }
    }
}

#[test]
fn every_removed_key_is_unknown_in_the_environment() {
    for (table, array, keys) in REMOVED {
        let mut path = table.replace('.', "__").to_uppercase();
        if *array {
            path.push_str("__0");
        }
        for key in *keys {
            let var = format!("FERROEHR__{path}__{}", key.to_uppercase());
            let refusal = assemble(None, &env(&[(var.as_str(), "x")]), &[])
                .expect_err(&var)
                .to_string();
            assert!(
                refusal.contains(&var) || refusal.contains(removed_table(table)),
                "{var} is refused as unknown: {refusal}"
            );
        }
    }
}

#[test]
fn the_removed_tables_are_unknown() {
    for table in [
        "authz.abac.remote",
        "authz.abac.remote.endpoint",
        "authz.abac.policy",
    ] {
        let refusal = file_refusal(&format!("[{table}]\n"));
        assert!(
            refusal.contains(removed_table(table)),
            "[{table}] is refused as unknown: {refusal}"
        );
    }
}

/// `remote` was the second value of `abac.engine`; `cedar` is its only one.
#[test]
fn the_remote_engine_is_unknown() {
    let refusal = file_refusal("[authz.abac]\nengine = \"remote\"\n");
    assert!(refusal.contains("remote"), "{refusal}");
    let refusal = assemble(
        None,
        &env(&[("FERROEHR__AUTHZ__ABAC__ENGINE", "remote")]),
        &[],
    )
    .expect_err("the remote engine")
    .to_string();
    assert!(refusal.contains("remote"), "{refusal}");
}
