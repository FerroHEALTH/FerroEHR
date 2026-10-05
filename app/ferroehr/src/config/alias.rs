// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The reserved-namespace allowlist, the section registry, the two permanent
//! conventional aliases, and the list-typed key registry. No openEHR spec governs
//! configuration — our own design.
//!
//! There is deliberately NO alias-remapping layer here: an unrecognized
//! variable in the reserved namespace is a boot error carrying the uniform
//! spelling it should have had (`strict`), never a silently-honoured
//! synonym.

/// Reserved-namespace names that are NOT configuration keys and must pass the
/// strict env sweep untouched: the config-file pointer, the healthcheck arg,
/// the build-time vars, and the compose/infra parameterization that can leak
/// into the container environment. These keep a single `_` by design, which is
/// what distinguishes them from configuration keys — `FERROEHR_CONFIG` is a
/// file pointer, not a config key, so it never joins the uniform `FERROEHR__`
/// grammar.
pub(super) const ALLOWLIST: &[&str] = &[
    "FERROEHR_CONFIG",
    "FERROEHR_HEALTHCHECK_URL",
    "FERROEHR_BUILD_EPOCH",
    "FERROEHR_RUSTC",
    "FERROEHR_IMAGE",
    "FERROEHR_POSTGRES_IMAGE",
    "FERROEHR_VIEWER_IMAGE",
    "FERROEHR_EHRBASE_IMAGE",
    "FERROEHR_EHRBASE_POSTGRES_IMAGE",
    "FERROEHR_PORT",
    "FERROEHR_DB_PORT",
    "FERROEHR_S3_PORT",
    "FERROEHR_VIEWER_PORT",
    "FERROEHR_TERMINOLOGY_PORT",
    "FERROEHR_PGP_PORT",
    "FERROEHR_PGP_DB_PORT",
    "FERROEHR_EHRBASE_PORT",
    "FERROEHR_CPUS",
    "FERROEHR_MEM",
    "FERROEHR_DB_CPUS",
    "FERROEHR_DB_MEM",
    "FERROEHR_TERMINOLOGY_CPUS",
    "FERROEHR_TERMINOLOGY_MEM",
];

/// The top-level configuration names (the section tables plus the scalar
/// `spec_profile`) — the did-you-mean candidate set for an unknown
/// `FERROEHR__<SECTION>__…` variable.
pub(super) const SECTIONS: &[&str] = &[
    // Top-level scalar keys (no section table) — `spec_profile` maps to
    // FERROEHR__SPEC_PROFILE with no further segments.
    "spec_profile",
    "server",
    "db",
    "storage",
    "log",
    "telemetry",
    "auth",
    "authz",
    "admin",
    "smart",
    "management",
    "signing",
    "licence",
    "query",
    "events",
    "fhir",
    "terminology",
    "multimedia",
    "audit",
    "privacy",
    "demographic",
    "cohort",
    "usage_report",
];

/// The two PERMANENT conventional aliases — 12-factor ecosystem names every
/// deployment platform already sets; they sit BELOW their `FERROEHR__` forms
/// within the env layer.
/// `(external_name, canonical FERROEHR__ env form)`.
///
/// A third conventional name, `PORT` (the container-platform port
/// convention), is handled in the loader directly: its value is only the
/// port half, so it expands to a `0.0.0.0:<PORT>` bind instead of mapping
/// 1:1 onto a canonical form.
pub(super) const CONVENTIONAL: &[(&str, &str)] = &[
    ("DATABASE_URL", "FERROEHR__DB__URL"),
    ("RUST_LOG", "FERROEHR__LOG__FILTER"),
];

/// List-typed key paths — env values for these are comma-separated
/// (`config`'s `with_list_parse_key`), so a scalar value containing a comma is
/// never mis-split.
///
/// NOTE: every `Vec`-typed key an operator can set through the environment must
/// be listed here, or its env form is not merely mis-split but REFUSED —
/// `invalid type: string …, expected a sequence`. Two shipped keys were missing
/// and unreachable from the environment until a live probe hit the refusal.
/// Keys inside arrays-of-tables (a Basic user's `roles`) are file-only by the
/// env grammar and are deliberately absent.
pub(super) const LIST_KEYS: &[&str] = &[
    "auth.oidc.audiences",
    "auth.oidc.algorithms",
    "authz.rbac.role_claims",
    "smart.endpoints.token_endpoint_auth_methods_supported",
    "smart.endpoints.grant_types_supported",
    "smart.endpoints.response_types_supported",
    "smart.endpoints.code_challenge_methods_supported",
    "smart.endpoints.scopes_supported",
    "smart.endpoints.capabilities",
    "signing.retired_key_paths",
    "privacy.subject_namespaces",
    "privacy.identifier_scan.rules",
    "privacy.identifier_scan.patterns",
    "demographic.identifier_protection.schemes",
    // `authz.abac.policy` is a MAP, so the env grammar reaches into it the way
    // it reaches `terminology.external.providers.<name>`. The resource kinds are the
    // closed set the enforcement point consults (`ResourceKind`), so each is
    // named rather than pattern-matched — `with_list_parse_key` takes exact
    // paths, and a wildcard here would be a second grammar to keep true.
    "authz.abac.policy.ehr.parameters",
    "authz.abac.policy.ehr_status.parameters",
    "authz.abac.policy.composition.parameters",
    "authz.abac.policy.contribution.parameters",
    "authz.abac.policy.query.parameters",
    "authz.abac.policy.directory.parameters",
];

/// The `FERROEHR__<SECTION>__<TAIL>` env name for a dotted config key path —
/// the uniform `__` grammar in the file→env direction, pinned by the tests below.
/// Test-only for now: the deserialize-error enrichment reports serde leaf
/// fields (not full paths), so it cannot reconstruct env provenance yet.
#[cfg(test)]
#[must_use]
pub(super) fn env_name_for(key_path: &str) -> String {
    let tail = key_path
        .split('.')
        .map(str::to_ascii_uppercase)
        .collect::<Vec<_>>()
        .join("__");
    format!("FERROEHR__{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_name_reconstruction() {
        assert_eq!(
            env_name_for("db.max_connections"),
            "FERROEHR__DB__MAX_CONNECTIONS"
        );
        assert_eq!(
            env_name_for("auth.oidc.issuer"),
            "FERROEHR__AUTH__OIDC__ISSUER"
        );
    }
}
