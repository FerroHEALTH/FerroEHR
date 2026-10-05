// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `I_DEFINITION_QUERY` (`i_definition_query.adoc`; `master04` §Registered
//! Queries, §Query Formalism) + `QUERY_DESCRIPTOR` (`query_descriptor.adoc`),
//! and the stored-query CRUD it owns (on the `stored_query` table).
//!
//! DEFINITION owns query *registration* (`master04` §Registered Queries); the
//! Query service only *resolves + executes* a stored query, so the stored-query
//! store lives here, keyed by a qualified name (`reverse_domain_name ::
//! semantic_id`) and a semantic version.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): stored template/query artefacts served verbatim + \
              ADL/OPT wire envelopes"
)]

use serde_json::{Value, json};
use sqlx::Row;
use sqlx::postgres::PgRow;
use uuid::Uuid;

use crate::service::FerroEhrService;
use crate::service::definition::types::QueryDescriptor;
use crate::service::error::ServiceError;
use crate::service::list::Page;
use crate::service::status::{CallStatusType, SmError};

use super::{compile_pattern, page_bounds, paginate};

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

// ── SM Definitions native API (I_DEFINITION_QUERY) — the catalog contract ────

impl FerroEhrService {
    /// `has_query` — true if a query with the qualified name `a_query_name` is
    /// registered (the `"misc"` namespace is assumed when none is supplied; a
    /// three-part name's formalism segment is lifted out per `master04`
    /// §Registered Queries). Identity is case-insensitive (BASE
    /// master05 §Composite Identifiers and Case).
    ///
    /// # Errors
    ///
    /// A database failure (`exception` → `500`).
    pub async fn has_query(&self, a_query_name: String) -> Result<bool, SmError> {
        Ok(self.query_exists(&a_query_name).await?)
    }

    /// `valid_query` — `a_query_text` is a valid instance of formalism
    /// `a_type` (a successful `openehr_query` parse; only AQL major-version 1
    /// is a known formalism). Stateless.
    ///
    /// # Errors
    ///
    /// Never — the `Result` shape mirrors the SM catalog; validity is reported
    /// in the `Ok` boolean.
    #[expect(
        clippy::unused_self,
        clippy::unnecessary_wraps,
        reason = "the SM interface declares this call on the service and in the \
                  SM call-status `Result` shape; the protocol adapter invokes \
                  every SM call uniformly, so neither is dropped because this \
                  particular realization happens to be stateless and infallible"
    )]
    pub fn valid_query(&self, a_query_text: &str, a_type: &str) -> Result<bool, SmError> {
        Ok(valid_query_text(a_query_text, a_type))
    }

    /// `store_query` (Pre `valid_query`) — register a query, returning its
    /// [`QueryDescriptor`].
    ///
    /// # Errors
    ///
    /// - Query text that is not a valid instance of the effective formalism →
    ///   `invalid_query` (`422`).
    /// - Query text that fails the store-time AQL parse →
    ///   `precondition_violation` (`400`).
    /// - A database failure (`exception` → `500`).
    pub async fn store_query(
        &self,
        a_query_text: String,
        a_type: String,
        a_query_name: Option<String>,
    ) -> Result<QueryDescriptor, SmError> {
        Ok(self
            .register_query(a_query_text, &a_type, a_query_name)
            .await?)
    }

    /// `store_query_set (a_query_set_name: String [0..1]): UUID` — "Register
    /// a query set. TODO: determine details."
    ///
    /// NOTE: an explicit spec TODO with no defined semantics
    /// (`i_definition_query.adoc`) — `NotImplemented` (→ `501`) until the
    /// spec defines it.
    ///
    /// # Errors
    ///
    /// Always — `not_implemented` (`501`), unconditionally.
    #[expect(
        clippy::unused_self,
        reason = "the SM interface declares this call on the service; the \
                  protocol adapter invokes every SM call uniformly, so the \
                  receiver stays even where this realization ignores it"
    )]
    pub fn store_query_set(&self, _a_query_set_name: Option<String>) -> Result<String, SmError> {
        Err(SmError::new(
            CallStatusType::NotImplemented,
            "store_query_set is an SM TODO (i_definition_query.adoc): not implemented",
        ))
    }

    /// `list_queries` — all registered queries, as descriptors, cursored by
    /// `page`.
    ///
    /// # Errors
    ///
    /// - A row-decode failure on a `NOT NULL` column (a genuine server fault)
    ///   → `exception` (`500`).
    /// - A database failure (`exception` → `500`).
    pub async fn list_queries(&self, page: Page) -> Result<Vec<QueryDescriptor>, SmError> {
        Ok(self.stored_query_descriptors(page).await?)
    }

    /// `list_matching_queries` — registered queries whose qualified name
    /// matches `id_pattern` (regex) and whose referenced artefacts match
    /// `artefact_id_pattern` (regex, `None` = match any), cursored by `page`.
    ///
    /// # Errors
    ///
    /// - An uncompilable `id_pattern` or `artefact_id_pattern` →
    ///   `invalid_id_pattern` (`400`).
    /// - A row-decode failure (a genuine server fault) → `exception` (`500`).
    /// - A database failure (`exception` → `500`).
    pub async fn list_matching_queries(
        &self,
        id_pattern: String,
        artefact_id_pattern: Option<String>,
        page: Page,
    ) -> Result<Vec<QueryDescriptor>, SmError> {
        Ok(self
            .query_list_matching(&id_pattern, artefact_id_pattern.as_deref(), page)
            .await?)
    }

    /// `delete_query` (Pre `has_query` / Post `query_deleted`) — delete every
    /// version of the query with qualified name `a_query_name` (the SM keys
    /// deletion by *name*). Identity is case-insensitive.
    ///
    /// # Errors
    ///
    /// - No registered query with that name → `artefact_does_not_exist`
    ///   (`404`).
    /// - A database failure (`exception` → `500`).
    pub async fn delete_query(&self, a_query_name: String) -> Result<(), SmError> {
        Ok(self.query_delete(&a_query_name).await?)
    }

    /// `queries_count` — total count of queries.
    ///
    /// NOTE: counts distinct *qualified names* (a query with N stored
    /// versions counts once) — the natural reading of "total count of queries".
    ///
    /// # Errors
    ///
    /// A database failure (`exception` → `500`).
    pub async fn queries_count(&self) -> Result<i64, SmError> {
        Ok(self.query_count().await?)
    }
}

// ── domain logic (the ServiceError layer under the catalog) ──────────────────

impl FerroEhrService {
    /// True if a query with the qualified name `a_query_name` is registered
    /// (case-insensitive identity, `"misc"` default namespace).
    async fn query_exists(&self, a_query_name: &str) -> Result<bool, ServiceError> {
        let qualified = parse_qualified_name(a_query_name).qualified();
        let (rdn, semantic) = split_qualified(&qualified);
        Ok(sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM stored_query \
             WHERE lower(reverse_domain_name) = lower($1) AND lower(semantic_id) = lower($2))",
        )
        .bind(rdn)
        .bind(semantic)
        .fetch_one(&self.pool)
        .await?)
    }

    /// `store_query` core — register a query, returning its descriptor.
    ///
    /// The name is qualified (`"misc"` default) or generated (`misc::q_<uuid>`)
    /// when absent. A three-part `<ns>::<formalism>::<name>` name lifts its
    /// formalism out: the store key is the two-part `<ns>::<name>`
    /// and the name-borne formalism, when present, is the effective formalism —
    /// otherwise `a_type` is (`master04` §Query Formalism).
    ///
    /// NOTE (spec naming): the SM precondition names
    /// `is_valid_query` though the function is `valid_query`; we enforce
    /// `valid_query` and reject an invalid query as `invalid_query` (`422`).
    async fn register_query(
        &self,
        text: String,
        a_type: &str,
        name: Option<String>,
    ) -> Result<QueryDescriptor, ServiceError> {
        let (qualified, formalism) = match name {
            Some(n) => {
                let parsed = parse_qualified_name(&n);
                let formalism = parsed.formalism.clone();
                (parsed.qualified(), formalism)
            }
            None => (format!("misc::q_{}", Uuid::now_v7().simple()), None),
        };
        // The effective formalism is the name-borne one when a three-part name
        // supplied it, else the `a_type` parameter.
        let effective = formalism.as_deref().unwrap_or(a_type);
        if !valid_query_text(&text, effective) {
            return Err(ServiceError::sm(
                CallStatusType::InvalidQuery,
                "query text is not a valid instance of its formalism",
            ));
        }
        let version = self.store_query_version(&qualified, None, text).await?;
        self.query_descriptor(&qualified, &version).await
    }

    /// The descriptor of a single stored `(qualified name, version)` row.
    async fn query_descriptor(
        &self,
        qualified: &str,
        version: &str,
    ) -> Result<QueryDescriptor, ServiceError> {
        let (rdn, semantic) = split_qualified(qualified);
        let row = sqlx::query(
            "SELECT reverse_domain_name, semantic_id, semver, query_type, query_text, created_at \
             FROM stored_query \
             WHERE lower(reverse_domain_name) = lower($1) AND lower(semantic_id) = lower($2) \
             AND semver = $3",
        )
        .bind(rdn)
        .bind(semantic)
        .bind(version)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            ServiceError::sm(
                CallStatusType::ArtefactDoesNotExist,
                format!("stored query {qualified}"),
            )
        })?;
        descriptor_from_row(&row)
    }

    /// All registered queries as descriptors, paged in SQL.
    async fn stored_query_descriptors(
        &self,
        page: Page,
    ) -> Result<Vec<QueryDescriptor>, ServiceError> {
        let (offset, limit) = page_bounds(page);
        let rows = sqlx::query(
            "SELECT reverse_domain_name, semantic_id, semver, query_type, query_text, created_at \
             FROM stored_query \
             ORDER BY reverse_domain_name, semantic_id, string_to_array(semver, '.')::int[] \
             OFFSET $1 LIMIT $2",
        )
        .bind(offset)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(descriptor_from_row).collect()
    }

    /// Registered queries whose qualified name matches `id_pattern` and whose
    /// referenced artefacts match `artefact_id_pattern` (`None` = match any),
    /// then paged.
    ///
    /// NOTE: `artefact_id_pattern` is spec'd against
    /// "archetype / template identifiers referenced in the query". Until the AQL
    /// engine exposes a query's analysed FROM/CONTAINS artefact-id set
    /// (`openehr_query`), we approximate by regex-scanning the stored source
    /// text — a query matches when its source contains a substring matching the
    /// artefact pattern. Replacing the raw-text scan with that analysed set is
    /// the future AQL-surface work.
    async fn query_list_matching(
        &self,
        id_pattern: &str,
        artefact_id_pattern: Option<&str>,
        page: Page,
    ) -> Result<Vec<QueryDescriptor>, ServiceError> {
        let id_re = compile_pattern(id_pattern)?;
        let artefact_re = artefact_id_pattern.map(compile_pattern).transpose()?;
        let rows = sqlx::query(
            "SELECT reverse_domain_name, semantic_id, semver, query_type, query_text, created_at \
             FROM stored_query \
             ORDER BY reverse_domain_name, semantic_id, string_to_array(semver, '.')::int[]",
        )
        .fetch_all(&self.pool)
        .await?;
        // Decode every row up front so a decode failure surfaces (500) rather
        // than silently dropping a query from the match set.
        let descriptors = rows
            .iter()
            .map(descriptor_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        let matched = descriptors.into_iter().filter(|d| {
            id_re.is_match(&d.qualified_query_name)
                && artefact_re
                    .as_ref()
                    .is_none_or(|re| d.source.as_deref().is_some_and(|src| re.is_match(src)))
        });
        Ok(paginate(matched, page))
    }

    /// Delete exactly one stored-query version — the `(name, version)` row —
    /// case-insensitive on the qualified name (matching the PUT store path at
    /// [`Self::store_query_version`]), exact on `version`. Absent → 404.
    ///
    /// NOTE: no openEHR spec governs this admin surface (the ITS-REST Admin API
    /// defines only EHR deletes) — our own design/extension. It complements the
    /// SM [`Self::delete_query`] (which deletes every version by name) with a
    /// single-version delete, addressed exactly as
    /// `DELETE /admin/query/{qualified_name}/{version}`.
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
        .execute(&self.pool)
        .await?
        .rows_affected();
        if deleted == 0 {
            return Err(ServiceError::sm(
                CallStatusType::ArtefactDoesNotExist,
                format!("stored query {qualified_name} at version {version}"),
            ));
        }
        Ok(())
    }

    /// Delete every version of the query with qualified name `a_query_name`
    /// (case-insensitive); absent → `artefact_does_not_exist` (`404`).
    async fn query_delete(&self, a_query_name: &str) -> Result<(), ServiceError> {
        let qualified = parse_qualified_name(a_query_name).qualified();
        let (rdn, semantic) = split_qualified(&qualified);
        let deleted = sqlx::query(
            "DELETE FROM stored_query \
             WHERE lower(reverse_domain_name) = lower($1) AND lower(semantic_id) = lower($2)",
        )
        .bind(rdn)
        .bind(semantic)
        .execute(&self.pool)
        .await?
        .rows_affected();
        if deleted == 0 {
            return Err(ServiceError::sm(
                CallStatusType::ArtefactDoesNotExist,
                format!("stored query {a_query_name}"),
            ));
        }
        Ok(())
    }

    /// Total count of distinct qualified query names.
    async fn query_count(&self) -> Result<i64, ServiceError> {
        Ok(sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM \
             (SELECT DISTINCT reverse_domain_name, semantic_id FROM stored_query) t",
        )
        .fetch_one(&self.pool)
        .await?)
    }
}

// ── the stored-query store (CRUD used by the DEFINITION wire + Query service) ─

impl FerroEhrService {
    /// Store a stored query, returning the effective SEMVER it is stored at.
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
        // `GET /query/aql` route. Decomposed via the master04 schemes so a
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

        // Store-time AQL validation via the `openehr_query` parser: the
        // stored-query table only holds AQL (`query_type = 'AQL'`), so a non-AQL
        // or syntactically-invalid body is a `400 Bad Request`
        // (`definition_query_store` lists only `200`/`400`).
        if let Err(err) = openehr_query::parser::parse_str(&query_text) {
            return Err(ServiceError::bad_request(
                format!("stored query text is not valid AQL: {err}"),
                err,
            ));
        }

        // ONE canonical key for every surface (SM master04 §Registered Queries:
        // "If no namespace is supplied, the namespace \"misc\" is assumed"), so
        // the same identifier never resolves on one surface and 404s on another.
        let qualified = parse_qualified_name(qualified_name).qualified();
        let (rdn, semantic) = split_qualified(&qualified);
        let Some(v) = version else {
            // No-version store: case-insensitive upsert at the default version.
            let mut tx = self.pool.begin().await?;
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
        .fetch_one(&self.pool)
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
        .execute(&self.pool)
        .await?
        .rows_affected();
        if inserted == 0 {
            return Err(ServiceError::conflict(format!(
                "a stored query '{qualified_name}' at version '{v}' already exists"
            )));
        }
        Ok(v.to_owned())
    }

    /// Retrieve a stored query — an exact `version`, a SEMVER **prefix**
    /// (`{major}` or `{major}.{minor}` → the *highest* matching stored version,
    /// `parameters/path/version.yaml`), or the latest when no version
    /// is given. Identity is case-insensitive.
    pub(in crate::service) async fn get_stored_query(
        &self,
        qualified_name: &str,
        version: Option<&str>,
    ) -> Result<Value, ServiceError> {
        // The same canonical key as the store path (SM master04 misc default).
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
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            ServiceError::sm(
                CallStatusType::ArtefactDoesNotExist,
                format!("stored query {qualified_name}"),
            )
        })?;

        stored_query_json(&row)
    }

    /// List stored queries whose qualified name starts with `name_pattern`
    /// (`definition_query_list.yaml`: the name is a **prefix** — a bare
    /// `org.openehr` prefix "will list all versions of all queries with names
    /// starting with `org.openehr`"; empty ⇒ wildcard). All stored versions of
    /// every matching name are returned. The prefix match is case-insensitive.
    pub(super) async fn list_stored_queries(
        &self,
        name_pattern: &str,
    ) -> Result<Vec<Value>, ServiceError> {
        // A namespace-less pattern also matches its `misc::`-assumed
        // composition (SM master04 §Registered Queries: "If no namespace is
        // supplied, the namespace \"misc\" is assumed") — the canonical key
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
        .fetch_all(&self.pool)
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

/// Build a [`QueryDescriptor`] from a `stored_query` row (per
/// `query_descriptor.adoc`). The qualified name is `rdn::semantic` (or the bare
/// `semantic` when the domain is empty); `formalism` is the `query_type`
/// lowercased (`QUERY_DESCRIPTOR` spells AQL `"aql"`).
///
/// Every projected column is `NOT NULL` (`clinical/0006_definitions.sql`
/// §`stored_query`), so a decode failure is a genuine server fault: surface it
/// (`?` → `500`)
/// rather than silently blanking the descriptor field.
fn descriptor_from_row(row: &PgRow) -> Result<QueryDescriptor, ServiceError> {
    let rdn: String = row.try_get("reverse_domain_name")?;
    let semantic: String = row.try_get("semantic_id")?;
    let name = if rdn.is_empty() {
        semantic
    } else {
        format!("{rdn}::{semantic}")
    };
    let version: String = row.try_get("semver")?;
    let formalism: String = row.try_get::<String, _>("query_type")?.to_ascii_lowercase();
    let source: String = row.try_get("query_text")?;
    let registration_time = row
        .try_get::<jiff_sqlx::Timestamp, _>("created_at")?
        .to_jiff()
        .to_string();
    Ok(QueryDescriptor {
        qualified_query_name: name,
        version: Some(version),
        registration_time,
        formalism,
        source: Some(source),
    })
}

// ── qualified names (master04 §Registered Queries) ────────────────────────────

/// A qualified query name decomposed per `master04-definition_package.adoc`
/// §Registered Queries: `<namespace>::<query-name>` or the three-part
/// `<namespace>::<formalism>::<query-name>`.
struct QualifiedName {
    /// The namespace segment (`"misc"` when none was supplied — §Registered
    /// Queries, l.34).
    namespace: String,
    /// The formalism segment of a three-part name, if present (§Registered
    /// Queries scheme 2). Feeds `QUERY_DESCRIPTOR.formalism` / `query_type`.
    formalism: Option<String>,
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

/// Decompose a (possibly unqualified) query name per `master04` §Registered
/// Queries: apply the `"misc"` default namespace, then recognise the two-part
/// `<ns>::<name>` and three-part `<ns>::<formalism>::<name>` schemes. A
/// three-part name's middle segment is lifted out as the formalism (never left
/// folded into the name); a name of four or more segments keeps the
/// first segment as the namespace and the remainder as the (`::`-bearing) name.
fn parse_qualified_name(raw: &str) -> QualifiedName {
    let qualified = qualify(raw);
    // `qualify` guarantees a `::`, so the fallback arm is unreachable; it keeps
    // the decomposition total without a panic path.
    let (namespace, rest) = qualified.split_once("::").unwrap_or(("misc", &qualified));
    match rest.split_once("::") {
        // Exactly three segments: the middle one is the formalism.
        Some((formalism, name)) if !name.contains("::") => QualifiedName {
            namespace: namespace.to_owned(),
            formalism: Some(formalism.to_owned()),
            name: name.to_owned(),
        },
        // Two segments, or four and more: the whole remainder is the name.
        _ => QualifiedName {
            namespace: namespace.to_owned(),
            formalism: None,
            name: rest.to_owned(),
        },
    }
}

/// Apply the SM `"misc"` default namespace: a name with no `::` becomes
/// `misc::<name>` (`master04` §Registered Queries: "If no namespace is
/// supplied, the namespace `"misc"` is assumed").
fn qualify(name: &str) -> String {
    if name.contains("::") {
        name.to_owned()
    } else {
        format!("misc::{name}")
    }
}

/// Split an already two-part-canonical query name into `(reverse_domain_name,
/// semantic_id)` on the first `::`, so the SM and wire paths key `stored_query`
/// rows identically. Callers pass the [`QualifiedName::qualified`] form, which
/// is always two-part, so the formalism is never captured in `semantic_id`.
fn split_qualified(qualified_name: &str) -> (&str, &str) {
    qualified_name
        .split_once("::")
        .unwrap_or(("", qualified_name))
}

// ── formalism (master04 §Query Formalism) ─────────────────────────────────────

/// True if `a_query_text` is a valid instance of the formalism `a_type`.
///
/// Only AQL major-version 1 is a known formalism (`valid_query`; `master04`
/// §Query Formalism); any other formalism → `false` (the SM sanctions
/// "any other string value", which we reject typed since the store only holds
/// AQL). AQL validity is a successful `openehr_query` parse.
fn valid_query_text(text: &str, a_type: &str) -> bool {
    is_aql_v1(a_type) && openehr_query::parser::parse_str(text).is_ok()
}

/// Parse a formalism name per `master04` §Query Formalism (case-insensitive
/// name, optional `::version`, major `"1"` when absent) and report whether it
/// names AQL major-version 1 — the only formalism the build can validate +
/// store (`parameters/query/query_type.yaml`; the SM sanctions "any other
/// string value", which we reject typed). Shared with the wire seam.
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

/// Whether a `version` path parameter is a SEMVER *prefix* (`{major}` or
/// `{major}.{minor}`) rather than an exact `{major}.{minor}.{patch}` triple
/// (`parameters/path/version.yaml`).
fn is_partial_semver(version: &str) -> bool {
    let segments: Vec<&str> = version.split('.').collect();
    segments.len() < 3
        && segments
            .iter()
            .all(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
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
        // master04 §Registered Queries: no namespace ⇒ "misc".
        assert_eq!(qualify("all_over_50"), "misc::all_over_50");
        assert_eq!(qualify("ehr::x"), "ehr::x");
        assert_eq!(qualify("ns::aql::x"), "ns::aql::x");
    }

    #[test]
    fn three_part_name_lifts_the_formalism_out_of_the_name() {
        // master04 §Registered Queries scheme 2: <ns>::<formalism>::<name>.
        // The formalism segment must NOT be folded into the stored name
        // — it becomes the descriptor formalism and the store key is
        // the canonical two-part <ns>::<name>.
        let q = parse_qualified_name("task_planning::aql::chemotherapy_plans");
        assert_eq!(q.namespace, "task_planning");
        assert_eq!(q.formalism.as_deref(), Some("aql"));
        assert_eq!(q.name, "chemotherapy_plans");
        assert_eq!(q.qualified(), "task_planning::chemotherapy_plans");

        // Two-part names keep the whole remainder as the name, no formalism.
        let two = parse_qualified_name("ehr::all_over_50");
        assert_eq!(two.namespace, "ehr");
        assert_eq!(two.formalism, None);
        assert_eq!(two.name, "all_over_50");

        // Four and more segments: namespace + a `::`-bearing name, no
        // formalism lift.
        let four = parse_qualified_name("a::b::c::d");
        assert_eq!(four.namespace, "a");
        assert_eq!(four.formalism, None);
        assert_eq!(four.name, "b::c::d");

        // Unqualified ⇒ misc default, two-part canonical form.
        let bare = parse_qualified_name("all_over_50");
        assert_eq!(bare.qualified(), "misc::all_over_50");
    }

    #[test]
    fn formalism_case_and_version_equivalence() {
        // "AQL" ≡ "aql" ≡ "AQL::1" (master04 §Query Formalism).
        assert!(is_aql_v1("AQL"));
        assert!(is_aql_v1("aql"));
        assert!(is_aql_v1("AQL::1"));
        assert!(is_aql_v1("aql::1.0.3"));
        // Other formalisms / other major versions are not known here.
        assert!(!is_aql_v1("AQL::2"));
        assert!(!is_aql_v1("cql"));
        assert!(!is_aql_v1("sparql::1"));
    }

    #[test]
    fn valid_query_needs_aql_and_a_parse() {
        assert!(valid_query_text("SELECT c FROM COMPOSITION c", "aql"));
        // Wrong formalism short-circuits to false without parsing.
        assert!(!valid_query_text("SELECT c FROM COMPOSITION c", "cql"));
        // AQL but unparseable.
        assert!(!valid_query_text("this is not aql", "AQL"));
    }

    #[test]
    fn partial_semver_detection() {
        // `{major}` / `{major}.{minor}` prefixes (version.yaml).
        assert!(is_partial_semver("1"));
        assert!(is_partial_semver("1.0"));
        assert!(is_partial_semver("12.34"));
        // Exact triples and malformed tokens are not prefixes.
        assert!(!is_partial_semver("1.0.0"));
        assert!(!is_partial_semver("1..0"));
        assert!(!is_partial_semver("1.a"));
        assert!(!is_partial_semver(""));
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
