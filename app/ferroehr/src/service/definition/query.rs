// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Query registration: the stored-query CRUD on the `stored_query` table.
//!
//! DEFINITION owns query *registration*; the
//! Query service only *resolves + executes* a stored query, so the stored-query
//! store lives here, keyed by a qualified name (`reverse_domain_name ::
//! semantic_id`) and a semantic version.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): stored template/query artefacts served verbatim + \
              ADL/OPT wire envelopes"
)]

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use sqlx::Connection as _;
use sqlx::Row;
use sqlx::postgres::PgRow;

use crate::aql::error::AqlError;
use crate::aql::ir::{ParamValue, Params};
use crate::aql::lineage::ArchetypeLineage;
use crate::aql::sql::{EhrAccessScope, SqlCtx};
use crate::aql::terminology::TerminologyExpander;
use crate::db::acquire::acquire;
use crate::service::FerroEhrService;
use crate::service::error::ServiceError;
use crate::service::refusal::Missing;

use super::version_selector::{check_version_selector, is_partial_semver};

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
const DEFAULT_QUERY_VERSION: &str = "1.0.0";

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
    /// Delete exactly one stored-query version — the `(name, version)` row —
    /// case-insensitive on the qualified name (matching the PUT store path at
    /// [`Self::store_query_version`]), exact on `version`. Absent → 404.
    ///
    /// NOTE: no openEHR spec governs this admin surface (the ITS-REST Admin API
    /// defines only EHR deletes) — our own design/extension. It is addressed
    /// exactly as `DELETE /admin/query/{qualified_name}/{version}`.
    pub(crate) async fn delete_stored_query_version(
        &self,
        qualified_name: &str,
        version: &str,
    ) -> Result<(), ServiceError> {
        let qualified = parse_qualified_name(qualified_name).qualified();
        let (rdn, semantic) = split_qualified(&qualified);
        let deleted = sqlx::query(
            "DELETE FROM stored_query \
             WHERE lower(reverse_domain_name) = lower($1) \
               AND lower(semantic_id) = lower($2) AND semver = $3",
        )
        .bind(rdn)
        .bind(semantic)
        .bind(version)
        .execute(&mut *acquire(&self.pool).await?)
        .await?
        .rows_affected();
        if deleted == 0 {
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
        qualified_name: &str,
        version: Option<&str>,
        query_text: String,
    ) -> Result<String, ServiceError> {
        // The bare query-name segment must not be the reserved name `aql`,
        // case-insensitively (ITS-REST query Qualified_query_name §NOTE: "The
        // `query-name` value must not be `aql` (case-insensitive), as that is
        // a reserved name") — it would collide with the ad-hoc
        // `GET /query/aql` route. Decomposed via the name schemes so a
        // three-part name's *formalism* segment (`ns::aql::name`) is never
        // mistaken for the query-name.
        if parse_qualified_name(qualified_name)
            .name
            .eq_ignore_ascii_case("aql")
        {
            return Err(ServiceError::precondition(format!(
                "`{qualified_name}`: the query-name `aql` is reserved \
                 (ITS-REST Qualified_query_name)"
            )));
        }

        self.analyse_stored_aql(&query_text).await?;

        // ONE canonical key for every surface (a bare name takes the `misc`
        // namespace), so the same identifier never resolves on one surface and
        // 404s on another.
        let qualified = parse_qualified_name(qualified_name).qualified();
        let (rdn, semantic) = split_qualified(&qualified);
        let Some(v) = version else {
            // No-version store: case-insensitive upsert at the default version.
            let mut conn = acquire(&self.pool).await?;
            let mut tx = conn.begin().await?;
            sqlx::query(
                "DELETE FROM stored_query \
                 WHERE lower(reverse_domain_name) = lower($1) \
                 AND lower(semantic_id) = lower($2) AND semver = $3",
            )
            .bind(rdn)
            .bind(semantic)
            .bind(DEFAULT_QUERY_VERSION)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO stored_query \
                 (reverse_domain_name, semantic_id, semver, query_type, query_text) \
                 VALUES ($1, $2, $3, 'AQL', $4)",
            )
            .bind(rdn)
            .bind(semantic)
            .bind(DEFAULT_QUERY_VERSION)
            .bind(query_text)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            return Ok(DEFAULT_QUERY_VERSION.to_owned());
        };

        // The versioned store requires an EXACT `major.minor.patch` (ITS-REST
        // `docs/query/Qualified_query_name.md`): the partial-prefix form is a
        // READ-resolution semantic over versions that already exist, and storing
        // one verbatim would break the surface's SEMVER ordering.
        // NOTE: no released sentence completes a prefix into a version to
        // create, so the refusal status is the docs text's own generic client
        // error (`Requests_and_responses.md` §"HTTP status codes", `400`).
        if !is_exact_semver(v) {
            return Err(ServiceError::precondition(format!(
                "`{v}` is not an exact SEMVER version: the versioned store requires \
                 `major.minor.patch` (the {{major}}/{{major}}.{{minor}} prefix forms are \
                 read-side resolution patterns, parameters/path/version.yaml)"
            )));
        }

        // Versioned store: insert-only, immutable pair. A case-insensitive match
        // at this version is a 409 (BASE master05 §Composite Identifiers and
        // Case). The exact-case insert stays race-safe via `ON CONFLICT (rdn,
        // semantic, semver) DO NOTHING` — 0 affected rows also means the pair
        // already exists → 409.
        let exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM stored_query \
             WHERE lower(reverse_domain_name) = lower($1) \
             AND lower(semantic_id) = lower($2) AND semver = $3)",
        )
        .bind(rdn)
        .bind(semantic)
        .bind(v)
        .fetch_one(&mut *acquire(&self.pool).await?)
        .await?;
        if exists {
            return Err(ServiceError::conflict(format!(
                "a stored query '{qualified_name}' at version '{v}' already exists"
            )));
        }
        let inserted = sqlx::query(
            "INSERT INTO stored_query \
             (reverse_domain_name, semantic_id, semver, query_type, query_text) \
             VALUES ($1, $2, $3, 'AQL', $4) \
             ON CONFLICT (reverse_domain_name, semantic_id, semver) DO NOTHING",
        )
        .bind(rdn)
        .bind(semantic)
        .bind(v)
        .bind(query_text)
        .execute(&mut *acquire(&self.pool).await?)
        .await?
        .rows_affected();
        if inserted == 0 {
            return Err(ServiceError::conflict(format!(
                "a stored query '{qualified_name}' at version '{v}' already exists"
            )));
        }
        Ok(v.to_owned())
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
                    crate::service::error::ErrorChain::new(&err)
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

    /// Retrieve a stored query — an exact `version`, a SEMVER **prefix**
    /// (`{major}` or `{major}.{minor}` → the *highest* matching stored version,
    /// `parameters/path/version.yaml`), or the latest when no version
    /// is given. Identity is case-insensitive.
    ///
    /// # Errors
    ///
    /// - A `version` that is no SEMVER version or prefix → `BadRequest`
    ///   (`400`), before any lookup.
    /// - No stored query matches → not-found (`404`).
    /// - A database or row-decode failure → `500`.
    pub(in crate::service) async fn get_stored_query(
        &self,
        qualified_name: &str,
        version: Option<&str>,
    ) -> Result<Value, ServiceError> {
        version.map(check_version_selector).transpose()?;
        // The same canonical key as the store path (the `misc` default).
        let qualified = parse_qualified_name(qualified_name).qualified();
        let (rdn, semantic) = split_qualified(&qualified);
        let row = match version {
            Some(v) if is_partial_semver(v) => sqlx::query(
                // Prefix match on a dot boundary (`1` → `1.x.y`, `1.0` →
                // `1.0.x`), highest version first. `left(...)` (not LIKE) avoids
                // pattern-metacharacter escaping.
                "SELECT reverse_domain_name, semantic_id, semver, query_type, query_text, \
                 created_at FROM stored_query \
                 WHERE lower(reverse_domain_name) = lower($1) AND lower(semantic_id) = lower($2) \
                 AND (semver = $3 OR left(semver, length($3) + 1) = $3 || '.') \
                 ORDER BY string_to_array(semver, '.')::int[] DESC LIMIT 1",
            )
            .bind(rdn)
            .bind(semantic)
            .bind(v),
            Some(v) => sqlx::query(
                "SELECT reverse_domain_name, semantic_id, semver, query_type, query_text, \
                 created_at FROM stored_query \
                 WHERE lower(reverse_domain_name) = lower($1) AND lower(semantic_id) = lower($2) \
                 AND semver = $3",
            )
            .bind(rdn)
            .bind(semantic)
            .bind(v),
            None => sqlx::query(
                "SELECT reverse_domain_name, semantic_id, semver, query_type, query_text, \
                 created_at FROM stored_query \
                 WHERE lower(reverse_domain_name) = lower($1) AND lower(semantic_id) = lower($2) \
                 ORDER BY string_to_array(semver, '.')::int[] DESC LIMIT 1",
            )
            .bind(rdn)
            .bind(semantic),
        }
        .fetch_optional(&mut *acquire(&self.pool).await?)
        .await?
        .ok_or_else(|| {
            ServiceError::not_found(Missing::Artefact, format!("stored query {qualified_name}"))
        })?;

        stored_query_json(&row)
    }

    /// List stored queries whose qualified name starts with `name_pattern`
    /// (`definition_query_list.yaml`: the name is a **prefix** — a bare
    /// `org.openehr` prefix "will list all versions of all queries with names
    /// starting with `org.openehr`"; empty ⇒ wildcard). All stored versions of
    /// every matching name are returned. The prefix match is case-insensitive.
    ///
    /// # Errors
    ///
    /// A row-decode failure (a server fault) or a database failure (`500`).
    pub async fn query_list(&self, name_pattern: &str) -> Result<Vec<Value>, ServiceError> {
        // A namespace-less pattern also matches its `misc::`-assumed
        // composition — the canonical key
        // every surface stores under.
        let rows = sqlx::query(
            "SELECT reverse_domain_name, semantic_id, semver, query_type, query_text, created_at \
             FROM stored_query \
             WHERE left(lower(CASE WHEN reverse_domain_name = '' THEN semantic_id \
                             ELSE reverse_domain_name || '::' || semantic_id END), \
                        length($1)) = lower($1) \
                OR ($1 NOT LIKE '%::%' AND \
                    left(lower(reverse_domain_name || '::' || semantic_id), \
                         length('misc::' || $1)) = lower('misc::' || $1)) \
             ORDER BY reverse_domain_name, semantic_id, string_to_array(semver, '.')::int[]",
        )
        .bind(name_pattern)
        .fetch_all(&mut *acquire(&self.pool).await?)
        .await?;
        rows.iter().map(stored_query_json).collect()
    }
}

// ── row mapping ───────────────────────────────────────────────────────────────

/// The openEHR stored-query descriptor for one row (the ITS-REST wire shape).
///
/// Every projected column is `NOT NULL` (`clinical/0006_definitions.sql`
/// §`stored_query`), so a decode failure is a genuine server fault, not an
/// empty field:
/// surface it (`?` → `500`) rather than silently blanking the value.
fn stored_query_json(row: &PgRow) -> Result<Value, ServiceError> {
    let rdn = row.try_get::<String, _>("reverse_domain_name")?;
    let semantic = row.try_get::<String, _>("semantic_id")?;
    let name = if rdn.is_empty() {
        semantic
    } else {
        format!("{rdn}::{semantic}")
    };
    let saved = row
        .try_get::<jiff_sqlx::Timestamp, _>("created_at")?
        .to_jiff()
        .to_string();
    Ok(json!({
        "name": name,
        "type": row.try_get::<String, _>("query_type")?,
        "version": row.try_get::<String, _>("semver")?,
        "saved": saved,
        "q": row.try_get::<String, _>("query_text")?,
    }))
}

// ── qualified names ──────────────────────────────────────────────────────────

/// A qualified query name decomposed as ITS-REST `Qualified_query_name.md`
/// spells it, `<namespace>::<query-name>`, or the three-part
/// `<namespace>::<formalism>::<query-name>`.
struct QualifiedName {
    /// The namespace segment (`"misc"` when none was supplied — §Registered
    /// Queries, l.34).
    namespace: String,
    /// The bare query-name segment (the store key, never carrying the
    /// formalism).
    name: String,
}

impl QualifiedName {
    /// The canonical two-part `<namespace>::<query-name>` — the form the
    /// stored-query store keys on (so a three-part input round-trips to the
    /// same row and the formalism is never folded into the name).
    fn qualified(&self) -> String {
        format!("{}::{}", self.namespace, self.name)
    }
}

/// Decompose a (possibly unqualified) query name: apply the `"misc"` default
/// namespace, then recognise the two-part
/// `<ns>::<name>` and three-part `<ns>::<formalism>::<name>` schemes. A
/// three-part name's middle segment is the formalism, which is never folded
/// into the name; a name of four or more segments keeps the
/// first segment as the namespace and the remainder as the (`::`-bearing) name.
fn parse_qualified_name(raw: &str) -> QualifiedName {
    let qualified = qualify(raw);
    // `qualify` guarantees a `::`, so the fallback arm is unreachable; it keeps
    // the decomposition total without a panic path.
    let (namespace, rest) = qualified.split_once("::").unwrap_or(("misc", &qualified));
    match rest.split_once("::") {
        // Exactly three segments: the middle one is the formalism.
        Some((_formalism, name)) if !name.contains("::") => QualifiedName {
            namespace: namespace.to_owned(),
            name: name.to_owned(),
        },
        // Two segments, or four and more: the whole remainder is the name.
        _ => QualifiedName {
            namespace: namespace.to_owned(),
            name: rest.to_owned(),
        },
    }
}

/// Apply the `"misc"` default namespace: a name with no `::` becomes
/// `misc::<name>`. No openEHR wire spec names a default: our own design.
fn qualify(name: &str) -> String {
    if name.contains("::") {
        name.to_owned()
    } else {
        format!("misc::{name}")
    }
}

/// Split an already two-part-canonical query name into `(reverse_domain_name,
/// semantic_id)` on the first `::`, so the native and wire paths key `stored_query`
/// rows identically. Callers pass the [`QualifiedName::qualified`] form, which
/// is always two-part, so the formalism is never captured in `semantic_id`.
fn split_qualified(qualified_name: &str) -> (&str, &str) {
    qualified_name
        .split_once("::")
        .unwrap_or(("", qualified_name))
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

/// Whether a `version` value is an EXACT numeric `major.minor.patch` triple —
/// the only form the versioned store accepts (the prefix forms of
/// `parameters/path/version.yaml` are read-side resolution patterns; see
/// [`FerroEhrService::store_query_version`]).
fn is_exact_semver(version: &str) -> bool {
    let segments: Vec<&str> = version.split('.').collect();
    segments.len() == 3
        && segments
            .iter()
            .all(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualify_applies_misc_default() {
        // No namespace ⇒ "misc".
        assert_eq!(qualify("all_over_50"), "misc::all_over_50");
        assert_eq!(qualify("ehr::x"), "ehr::x");
        assert_eq!(qualify("ns::aql::x"), "ns::aql::x");
    }

    #[test]
    fn three_part_name_lifts_the_formalism_out_of_the_name() {
        // The three-part scheme: <ns>::<formalism>::<name>.
        // The formalism segment must NOT be folded into the stored name: the
        // store key is the canonical two-part <ns>::<name>.
        let q = parse_qualified_name("task_planning::aql::chemotherapy_plans");
        assert_eq!(q.namespace, "task_planning");
        assert_eq!(q.name, "chemotherapy_plans");
        assert_eq!(q.qualified(), "task_planning::chemotherapy_plans");

        // Two-part names keep the whole remainder as the name, no formalism.
        let two = parse_qualified_name("ehr::all_over_50");
        assert_eq!(two.namespace, "ehr");
        assert_eq!(two.name, "all_over_50");

        // Four and more segments: namespace + a `::`-bearing name, no
        // formalism lift.
        let four = parse_qualified_name("a::b::c::d");
        assert_eq!(four.namespace, "a");
        assert_eq!(four.name, "b::c::d");

        // Unqualified ⇒ misc default, two-part canonical form.
        let bare = parse_qualified_name("all_over_50");
        assert_eq!(bare.qualified(), "misc::all_over_50");
    }

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
