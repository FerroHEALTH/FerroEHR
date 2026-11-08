// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Query registration: the stored-query CRUD on the `stored_query` table.
//!
//! DEFINITION owns query *registration*; the
//! Query service only *resolves + executes* a stored query, so the stored-query
//! store lives here, keyed by a qualified query name and a version, both read
//! by the ITS-REST grammar (`openehr_its::rest::stored_query`).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): stored template/query artefacts served verbatim + \
              ADL/OPT wire envelopes"
)]

use std::sync::Arc;

use async_trait::async_trait;
use openehr_its::rest::stored_query::{QualifiedQueryName, QueryVersion, VersionPattern};
use serde_json::{Value, json};

use crate::aql::error::AqlError;
use crate::aql::ir::{ParamValue, Params};
use crate::aql::lineage::ArchetypeLineage;
use crate::aql::sql::{EhrAccessScope, SqlCtx};
use crate::aql::terminology::TerminologyExpander;
use crate::error::ServiceError;
use crate::error::refusal::Missing;
use crate::service::FerroEhrService;
use crate::storage::stored_query_repo::{self, StoredQueryRow};

/// The SEMVER a no-version store assigns a query. The no-version store path is
/// "stores a new query, or updates an existing query"
/// (`definition_query_store.yaml`), so it upserts at this default version.
///
/// NOTE: **no openEHR spec governs the minted value — our own design.** No
/// released ITS-REST sentence says which version a version-less store creates;
/// `specifications/docs/query/Qualified_query_name.md`'s "the system must use
/// the latest `version` with the supplied prefix" is a rule for USING a stored
/// query, not for storing one. The slot below is the lowest SEMVER a first store
/// can carry, which keeps the version-less form idempotent.
const DEFAULT_QUERY_VERSION: QueryVersion = QueryVersion::new(1, 0, 0);

/// The value every `$parameter` is bound to while a stored query is analysed.
///
/// SQL generation reads parameter values only to bind them, except a temporal
/// comparison bound, whose ISO-8601 shape it checks; this value has that shape
/// and is a valid string everywhere else, so no refusal at store time depends
/// on a value the caller supplies only at execution.
const STORE_TIME_PARAMETER: &str = "1970-01-01";

/// The terminology seam used while a stored query is analysed: it resolves
/// nothing and answers every `TERMINOLOGY()` call with a fixed result.
///
/// NOTE: QUERY master03 §TERMINOLOGY lets a resolution differ per execution,
/// so the store checks the query's shape and leaves resolution to execution.
#[derive(Debug)]
struct StoreTimeTerminology;

#[async_trait]
impl TerminologyExpander for StoreTimeTerminology {
    /// Answers an `expand` with one placeholder code.
    async fn expand(&self, _service_api: &str, _params_uri: &str) -> Result<Vec<String>, AqlError> {
        Ok(vec![STORE_TIME_PARAMETER.to_owned()])
    }

    /// Answers every Boolean operation with `true`.
    async fn boolean_operation(
        &self,
        _operation: &str,
        _service_api: &str,
        _params_uri: &str,
    ) -> Result<bool, AqlError> {
        Ok(true)
    }

    /// Answers a terminology URI with one placeholder code.
    async fn expand_uri(&self, _uri: &str) -> Result<Vec<String>, AqlError> {
        Ok(vec![STORE_TIME_PARAMETER.to_owned()])
    }
}

// ── the single-version delete ────────────────────────────────────────────────

impl FerroEhrService {
    /// Admin extension `DELETE /admin/query/{qualified_name}/{version}`:
    /// physically delete exactly one stored query (a single version row),
    /// case-insensitive on the qualified name (matching the PUT store path),
    /// exact on the version.
    ///
    /// NOTE: no openEHR spec governs this operation, the ITS-REST Admin API
    /// defining only EHR deletes — our own design/extension; it targets a
    /// single `(name, version)` row.
    ///
    /// # Errors
    /// - `versioned_object_does_not_exist` (`404`) — no stored query at that
    ///   name + version.
    /// - `exception` — a database fault.
    pub async fn admin_query_delete(
        &self,
        qualified_name: &QualifiedQueryName,
        version: QueryVersion,
    ) -> Result<(), ServiceError> {
        if !stored_query_repo::delete(&self.pool, qualified_name, version).await? {
            return Err(ServiceError::not_found(
                Missing::Artefact,
                format!("stored query {qualified_name} at version {version}"),
            ));
        }
        Ok(())
    }
}

// ── the stored-query store (CRUD used by the DEFINITION wire + Query service) ─

impl FerroEhrService {
    /// Store a stored query, returning the effective SEMVER it is stored at.
    ///
    /// The text must pass the analysis an execution runs before it touches the
    /// database ([`Self::analyse_stored_aql`]), so a query the engine refuses
    /// is refused here instead of on every later execution.
    ///
    /// - With an explicit `version` the `(name, version)` pair is immutable: an
    ///   already-existing pair, compared case-insensitively, is a `Conflict`
    ///   rather than an overwrite (ITS-REST overview
    ///   `Requests_and_responses.md` §status codes).
    /// - Without a version the query is stored or updated at
    ///   [`DEFAULT_QUERY_VERSION`], the unversioned store "stores a new query,
    ///   or updates an existing query".
    ///
    /// Identity is case-insensitive but storage case-preserving (BASE master05
    /// §Composite Identifiers and Case).
    ///
    /// # Errors
    ///
    /// - The reserved query-name `aql`, a non-exact SEMVER `version`, or a
    ///   text the AQL engine refuses → `BadRequest` (`400`).
    /// - An existing `(name, version)` pair → `Conflict` (`409`).
    /// - A database failure → `500`.
    pub(super) async fn store_query_version(
        &self,
        qualified_name: &QualifiedQueryName,
        version: Option<VersionPattern>,
        query_text: String,
    ) -> Result<QueryVersion, ServiceError> {
        // ITS-REST `Qualified_query_name.md` §NOTE: "The `query-name` value must
        // not be `aql` (case-insensitive), as that is a reserved name".
        qualified_name.check_registrable().map_err(|err| {
            ServiceError::precondition(format!(
                "`{qualified_name}`: {err} (ITS-REST Qualified_query_name)"
            ))
        })?;

        self.analyse_stored_aql(&query_text).await?;

        let Some(pattern) = version else {
            // No-version store: case-insensitive upsert at the default version.
            stored_query_repo::replace(
                &self.pool,
                qualified_name,
                DEFAULT_QUERY_VERSION,
                &query_text,
            )
            .await?;
            return Ok(DEFAULT_QUERY_VERSION);
        };

        // The versioned store requires an EXACT `major.minor.patch` (ITS-REST
        // `docs/query/Qualified_query_name.md`): the partial-prefix form is a
        // READ-resolution semantic over versions that already exist.
        // NOTE: no released sentence completes a prefix into a version to
        // create, so the refusal status is the docs text's own generic client
        // error (`Requests_and_responses.md` §"HTTP status codes", `400`).
        let Some(v) = pattern.exact() else {
            return Err(ServiceError::precondition(format!(
                "`{pattern}` is not an exact SEMVER version: the versioned store requires \
                 `major.minor.patch` (the {{major}}/{{major}}.{{minor}} prefix forms are \
                 read-side resolution patterns, parameters/path/version.yaml)"
            )));
        };

        // Versioned store: insert-only, immutable pair. A case-insensitive match
        // at this version is a 409 (BASE master05 §Composite Identifiers and
        // Case).
        if !stored_query_repo::insert(&self.pool, qualified_name, v, &query_text).await? {
            return Err(ServiceError::conflict(format!(
                "a stored query '{qualified_name}' at version '{v}' already exists"
            )));
        }
        Ok(v)
    }

    /// Runs the analysis an execution of `query_text` runs before it reaches
    /// the database, refusing a text the AQL engine refuses.
    ///
    /// The stages are the execution path's own: parse, the `TERMINOLOGY()`
    /// pre-pass, lowering to the IR, and SQL generation in both FROM shapes
    /// (flat and LIMIT-streaming). Nothing is executed: the pre-pass answers
    /// every `TERMINOLOGY()` call through [`StoreTimeTerminology`], and every
    /// `$parameter` is bound to [`STORE_TIME_PARAMETER`].
    ///
    /// # Errors
    ///
    /// `BadRequest` (`400`) for a text that does not parse, or that the
    /// engine refuses at lowering or SQL generation, with the refusal the
    /// execution would report.
    async fn analyse_stored_aql(&self, query_text: &str) -> Result<(), ServiceError> {
        let mut ast = openehr_query::parser::parse_str(query_text).map_err(|err| {
            ServiceError::bad_request(
                format!(
                    "stored query text is not valid AQL: {}",
                    crate::error::ErrorChain::new(&err)
                ),
                err,
            )
        })?;
        let refused = |err: AqlError| {
            ServiceError::bad_request(
                format!("the query engine refuses the stored query text: {err}"),
                err,
            )
        };
        crate::aql::terminology::expand_matches(&mut ast, &StoreTimeTerminology)
            .await
            .map_err(refused)?;
        let ir = crate::aql::lower_query(&ast, self.spec_profile).map_err(refused)?;
        let params = ir.params.iter().fold(Params::default(), |params, name| {
            params.with(
                name.clone(),
                ParamValue::Str(STORE_TIME_PARAMETER.to_owned()),
            )
        });
        // A `LIMIT` selects the streaming FROM shape when the query admits it
        // (`aql::sql::build`), so both shapes an execution can take are built.
        for limit in [None, Some(1)] {
            let ctx = SqlCtx {
                system_id: self.effective_system_id(),
                ehr_ids: Vec::new(),
                ehr_scope_kind: crate::aql::sql::EhrScopeKind::Named,
                subject_scope: None,
                ehr_access: EhrAccessScope::Unrestricted,
                limit,
                offset: None,
                archetype_lineage: Arc::new(ArchetypeLineage::default()),
            };
            crate::aql::sql::build(&ir, &params, &ctx).map_err(refused)?;
        }
        Ok(())
    }

    /// Retrieve a stored query: the highest stored version `version`
    /// selects (an exact version, or a `{major}` or `{major}.{minor}` prefix,
    /// `parameters/path/version.yaml`), or the latest when no version is
    /// given. Identity is case-insensitive.
    ///
    /// # Errors
    ///
    /// - No stored query matches → not-found (`404`).
    /// - A database or row-decode failure → `500`.
    pub(in crate::service) async fn get_stored_query(
        &self,
        qualified_name: &QualifiedQueryName,
        version: Option<&VersionPattern>,
    ) -> Result<Value, ServiceError> {
        let row = stored_query_repo::find(&self.pool, qualified_name, version)
            .await?
            .ok_or_else(|| {
                ServiceError::not_found(Missing::Artefact, format!("stored query {qualified_name}"))
            })?;
        Ok(stored_query_json(&row))
    }

    /// List stored queries whose qualified name starts with `name_pattern`
    /// (`definition_query_list.yaml`: the name is a **prefix**: a bare
    /// `org.openehr` prefix "will list all versions of all queries with names
    /// starting with `org.openehr`"; `None` lists every query, the empty
    /// pattern the operation calls a "wildcard"). All stored versions of
    /// every matching name are returned. The prefix match is case-insensitive,
    /// and a namespace-less pattern also matches under the `misc` namespace,
    /// the key a name without a namespace is stored under.
    ///
    /// # Errors
    ///
    /// A row-decode failure (a server fault) or a database failure (`500`).
    pub async fn query_list(
        &self,
        name_pattern: Option<&QualifiedQueryName>,
    ) -> Result<Vec<Value>, ServiceError> {
        Ok(stored_query_repo::list(&self.pool, name_pattern)
            .await?
            .iter()
            .map(stored_query_json)
            .collect())
    }
}

// ── row mapping ───────────────────────────────────────────────────────────────

/// The openEHR stored-query descriptor for one row (the ITS-REST wire shape).
fn stored_query_json(row: &StoredQueryRow) -> Value {
    let name = if row.reverse_domain_name.is_empty() {
        row.semantic_id.clone()
    } else {
        format!("{}::{}", row.reverse_domain_name, row.semantic_id)
    };
    json!({
        "name": name,
        "type": row.query_type,
        "version": row.semver,
        "saved": row.created_at.to_string(),
        "q": row.query_text,
    })
}

// ── formalism ────────────────────────────────────────────────────────────────

/// Parse a formalism name (case-insensitive name, optional `::version`, major
/// `"1"` when absent) and report whether it names AQL major-version 1 — the
/// only formalism the build can validate + store
/// (`parameters/query/query_type.yaml`). Shared with the wire seam.
pub(super) fn is_aql_v1(a_type: &str) -> bool {
    let (name, version) = match a_type.split_once("::") {
        Some((n, v)) => (n, Some(v)),
        None => (a_type, None),
    };
    if !name.trim().eq_ignore_ascii_case("aql") {
        return false;
    }
    let major = version
        .and_then(|v| v.trim().split('.').next())
        .filter(|s| !s.is_empty())
        .unwrap_or("1");
    major == "1"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formalism_case_and_version_equivalence() {
        // "AQL" ≡ "aql" ≡ "AQL::1".
        assert!(is_aql_v1("AQL"));
        assert!(is_aql_v1("aql"));
        assert!(is_aql_v1("AQL::1"));
        assert!(is_aql_v1("aql::1.0.3"));
        // Other formalisms / other major versions are not known here.
        assert!(!is_aql_v1("AQL::2"));
        assert!(!is_aql_v1("cql"));
        assert!(!is_aql_v1("sparql::1"));
    }

    /// The store path rejects a body that is not valid AQL (ECC-SQR-006/007):
    /// `store_query_version` guards on `openehr_query::parser::parse_str`, so a
    /// non-AQL or malformed body must fail that parse (→ `BadRequest` → `400`).
    #[test]
    fn store_time_aql_validation() {
        assert!(
            openehr_query::parser::parse_str("SELECT * FROM patients; -- SQL, not AQL").is_err()
        );
        assert!(openehr_query::parser::parse_str("SELECT FROM WHERE {{{ not valid aql").is_err());
        assert!(openehr_query::parser::parse_str("SELECT c FROM COMPOSITION c").is_ok());
    }
}
