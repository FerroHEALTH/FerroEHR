// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADL2 definitions: ADL2 artefacts (archetype /
//! template / `operational_template`) keyed by `ARCHETYPE_HRID`, on the
//! `adl2_artefact` store.
//!
//! Validation is the real `openehr-adl` engine (parse → AOM2 phase 1 → RM
//! phase 2 → specialisation phase 2 against the flat parent). The engine needs
//! an [`ArchetypeRepository`] to resolve specialisation parents +
//! `use_archetype` fillers; [`FerroEhrService::adl2_repository`] builds it
//! from the stored ADL2 sources (the registry is low-volume — an
//! admin surface — so parse-on-demand is cheap; the
//! `openehr-adl` phases degrade gracefully when a parent is unresolved).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): stored template/query artefacts served verbatim + \
              ADL/OPT wire envelopes"
)]

use std::sync::Arc;

use openehr_adl::artefact::ArchetypeRepository;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::hrid::raw_id_concept;
use openehr_adl::meta::{ArtefactSummary, summarize};
use openehr_adl::opt::{OptError, create_opt, unresolved_constituents};
use openehr_adl::parse::Dialect;
use openehr_adl::validate::catalogue::Severity;
use openehr_adl::validate::rm::{ProductionRmModel, production_model_governs};
use openehr_adl::validate::{validate_source, validate_source_integrity};
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetype;
use openehr_am::v2_4::aom2::archetype::operational_template::OperationalTemplate;
use openehr_base::validate::InvariantViolation;
use openehr_sdt::flat::example::{DetailLevel, ExampleType, apply_output_uid};
use openehr_sdt::flat::webtemplate::builder_v2_4::build_web_template_v2_4;

use crate::db::acquire::acquire;
use openehr_sdt::flat::webtemplate::model::WebTemplate;
use serde_json::Value;
use sqlx::Connection as _;
use sqlx::{PgConnection, Row};

use crate::service::FerroEhrService;
use crate::service::error::{ErrorChain, ServiceError, Violation};
use crate::service::list::Page;
use crate::service::refusal::Missing;

use super::adl2_dependents::adl2_write_lock;
use super::adl2_engine::{missing_constituents, not_compiled, on_engine_stack};
use super::adl2_resolution::{parse_stored, repository_of, resolve_stored, wire_reference};
use super::page_bounds;
use super::version_selector::check_version_selector;

/// The `WebTemplate`-cache key namespace for ADL2/OPT2 templates. The trailing
/// ASCII Unit Separator (`U+001F`) cannot appear in a grammar-legal
/// archetype/template id (printable ASCII —
/// `docs/specs/openehr/BASE/docs/base_types/master05-identification_package.adoc`
/// §Archetype Identifiers), so an ADL2 entry can never collide with an OPT 1.4
/// entry keyed by the plain identity-canonical form. No openEHR spec governs the
/// cache — our own design/extension.
const ADL2_CACHE_NS: &str = "adl2\u{1f}";

// ── domain logic (the ServiceError layer under the catalog) ──────────────────

impl FerroEhrService {
    /// Build the [`ArchetypeRepository`] of every stored ADL2 artefact, so the
    /// `openehr-adl` engine can resolve specialisation parents +
    /// `use_archetype` fillers when validating an upload or projecting an OPT:
    /// the engine resolves each reference to its stored release.
    ///
    /// NOTE: the repository is re-parsed on every call, this path being reached
    /// only for an ADL2 upload or an OPT projection, so a memoized copy would
    /// cost invalidation on every ADL2 write for no clinical-path gain. No
    /// openEHR spec governs this — our own design/extension.
    async fn adl2_repository(
        &self,
        conn: &mut PgConnection,
    ) -> Result<ArchetypeRepository, ServiceError> {
        let sources: Vec<String> =
            sqlx::query_scalar("SELECT adl FROM adl2_artefact ORDER BY hrid")
                .fetch_all(conn)
                .await?;
        on_engine_stack(move || {
            repository_of(sources.iter().map(String::as_str).filter_map(parse_stored))
        })
        .await
    }

    /// Validate one uploaded ADL2 source with the `openehr-adl` engine and store
    /// it (`adl2_persist`), returning its `ARCHETYPE_HRID`.
    ///
    /// The repository read, the validation and the write share one transaction
    /// under `adl2_write_lock`, which the delete takes too, so of an upload
    /// and a delete that race, the second gets the refusal it gets alone.
    ///
    /// Invalidity splits on the syntax/semantics line: an unparseable source
    /// (S-codes) is syntactically invalid content, the released `400` branch
    /// (`responses/400.yaml`, via `syntax_bad_request`), while an AOM2
    /// validation-phase failure (V-codes) on a parsed source is a
    /// `ValidationFailed` carrying the rule-code mnemonics, which the template
    /// upload answers `400` too
    /// (`operations/definition_template_adl2_upload.yaml` declares no `422`). The openEHR
    /// [`ProductionRmModel`] governs openEHR-published archetypes; a foreign or
    /// test model skips the RM pass, which would false-fire VCORM
    /// (`AOM2/master04.3` §Reference Model Type Matching). External term
    /// bindings are verified through the terminology-service resolver (VETDF,
    /// `master03` §Validity Rules; see `adl2_terminology_resolver`).
    /// This is `POST /definition/template/adl2`, which declares
    /// `409_template_already_exists.yaml` for an existing HRID.
    ///
    /// # Errors
    ///
    /// - Unparseable source → `BadRequest` (`400`).
    /// - AOM2-invalid source → `ValidationFailed` with the rule codes.
    /// - An artefact with the same HRID already stored → `Conflict` (`409`).
    /// - A database failure (`500`).
    pub async fn template_adl2_upload(&self, source: &str) -> Result<String, ServiceError> {
        let owned = source.to_owned();
        let archetype = on_engine_stack(move || parse_artefact(&owned, Dialect::Adl2))
            .await?
            .map_err(|errs| syntax_bad_request(&errs))?;
        let governed = production_model_governs(&archetype);
        // VETDF: external term bindings are verified against the terminology
        // service, pre-resolved here into the synchronous validator seam
        // (`master03` §Validity Rules; see [`AdlTerminologyResolver`]).
        let resolver = if governed {
            Some(self.adl2_terminology_resolver(&archetype).await)
        } else {
            None
        };
        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await?;
        adl2_write_lock(&mut tx).await?;
        let repo = self.adl2_repository(&mut tx).await?;
        let owned = source.to_owned();
        let issues = on_engine_stack(move || match resolver {
            Some(resolver) => validate_source(&owned, Some(&repo), &ProductionRmModel, &resolver),
            None => validate_source_integrity(&owned, Dialect::Adl2, Some(&repo)),
        })
        .await?
        .map_err(|errs| syntax_bad_request(&errs))?;

        let errors: Vec<InvariantViolation> = issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .map(|i| {
                // The rule-code mnemonic is the machine-readable key; the human
                // detail (and the archetype path where derivable) is the message.
                InvariantViolation::at(
                    i.code.mnemonic(),
                    match &i.path {
                        Some(p) => format!("{} (at {p})", i.message),
                        None => i.message.clone(),
                    },
                )
            })
            .collect();
        if !errors.is_empty() {
            return Err(ServiceError::ValidationFailed(errors));
        }
        let summary = summarize(&archetype);
        Self::adl2_persist(&mut tx, &summary, source).await?;
        tx.commit().await?;
        self.invalidate_archetype_lineage().await;
        Ok(summary.archetype_id)
    }

    /// Store a validated ADL2 artefact in `tx`. An artefact already stored under
    /// any case-variant of the same HRID is a `409` (the REST contract declares
    /// `409_template_already_exists.yaml`). The insert is verbatim (BASE
    /// master05 §Composite Identifiers and Case).
    ///
    /// The artefact's declared `specialize` parent travels with it into
    /// `parent_hrid`: it is the archetype-lineage edge AQL resolves a parent
    /// query through (BASE Artefact Identification
    /// `master07-referencing.adoc` §Supporting Archetype-based Querying: for a
    /// specialised archetype the lineage "can only be obtained from the
    /// operational form of the archetype"), and the engine has already
    /// extracted it into the [`ArtefactSummary`] during validation.
    async fn adl2_persist(
        tx: &mut PgConnection,
        summary: &ArtefactSummary,
        source: &str,
    ) -> Result<(), ServiceError> {
        let kind = store_kind(summary.kind);
        let stored: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM adl2_artefact WHERE lower(hrid) = lower($1))",
        )
        .bind(&summary.archetype_id)
        .fetch_one(&mut *tx)
        .await?;
        if stored {
            return Err(ServiceError::conflict(format!(
                "an ADL2 template with id '{}' already exists",
                summary.archetype_id
            )));
        }
        sqlx::query(
            "INSERT INTO adl2_artefact (hrid, kind, adl, parent_hrid) VALUES ($1, $2, $3, $4)",
        )
        .bind(&summary.archetype_id)
        .bind(kind)
        .bind(source)
        .bind(summary.parent_archetype_id.as_deref())
        .execute(&mut *tx)
        .await?;
        // Maintain the `template_ref` registry (the version.template_id FK
        // target) in the same transaction: a template-kind HRID is a commit
        // addressable wire identity (`clinical/0006_definitions.sql`
        // §template_ref).
        if matches!(kind, "template" | "operational_template") {
            sqlx::query(
                "INSERT INTO template_ref (template_id) VALUES ($1) ON CONFLICT DO NOTHING",
            )
            .bind(&summary.archetype_id)
            .execute(&mut *tx)
            .await?;
        }
        Ok(())
    }

    /// Resolve a wire `template_id` (`+` optional `version`) to the exact stored
    /// `ARCHETYPE_HRID`. A full HRID matches case-insensitively; any other
    /// `template_id` (`…concept.v1`, or `…concept` + a `version`) resolves by
    /// the engine's rule (BASE Artefact Identification
    /// `master07-referencing.adoc` §Referencing): the newest version carrying
    /// the given version parts, a release before its pre-releases, and a
    /// pre-release only by its full version (`template_id_adl2.yaml`: "a
    /// partial `template_id` will resolve to 'latest' major version").
    ///
    /// # Errors
    ///
    /// - A `version` that is no SEMVER version or prefix → `BadRequest`
    ///   (`400`), before any lookup.
    /// - No stored artefact matches → `NotFound` (`404`).
    pub(super) async fn adl2_resolve(
        &self,
        template_id: &str,
        version: Option<&str>,
    ) -> Result<String, ServiceError> {
        version.map(check_version_selector).transpose()?;
        // Fast path: an exact HRID (no explicit version filter).
        if version.is_none()
            && let Some(hrid) = sqlx::query_scalar::<_, String>(
                "SELECT hrid FROM adl2_artefact WHERE lower(hrid) = lower($1)",
            )
            .bind(template_id)
            .fetch_optional(&mut *acquire(&self.pool).await?)
            .await?
        {
            return Ok(hrid);
        }
        // Every other read resolves by the engine's rule over the (small)
        // registry.
        let hrids: Vec<String> = sqlx::query_scalar("SELECT hrid FROM adl2_artefact ORDER BY hrid")
            .fetch_all(&mut *acquire(&self.pool).await?)
            .await?;
        resolve_stored(&wire_reference(template_id, version), hrids).ok_or_else(|| {
            ServiceError::not_found(
                Missing::Template,
                format!(
                    "ADL2 template {template_id}{}",
                    version
                        .map(|v| format!(" at version {v}"))
                        .unwrap_or_default()
                ),
            )
        })
    }

    /// Project the stored ADL2 source into its `OperationalTemplateV2` canonical
    /// JSON (`GET …/adl2/{template_id}` with `Accept: application/json`).
    ///
    /// The OAS declares `OperationalTemplateV2` as an opaque `type: object` with
    /// no properties (`schemas/aom/OperationalTemplateV2.yaml`), so the AOM2
    /// canonical JSON of the operational template — a JSON object — satisfies it
    /// honestly. A stored `operational_template` is serialized as-is; any other
    /// kind is compiled to its OPT via `openehr_adl::opt::create_opt` first.
    ///
    /// # Errors
    ///
    /// - The stored source no longer parses → `Internal` (`500`; a stored source
    ///   was engine-valid at upload, so this is a server fault).
    /// - A constituent archetype is not stored → `ValidationFailed` (`422`,
    ///   one `VARXR` violation per missing archetype id).
    /// - The OPT cannot be compiled otherwise → `Unprocessable` (`422`).
    pub(super) async fn adl2_opt_json(&self, source: &str) -> Result<String, ServiceError> {
        let opt = self.adl2_operational_template(source).await?;
        Ok(openehr_its::json::to_canonical_json(&opt))
    }

    /// Generate an example COMPOSITION for a stored ADL2 template
    /// (`GET …/definition/template/adl2/{template_id}/example`).
    ///
    /// The stored source is resolved (`template_id` → HRID), parsed, compiled to
    /// its operational template (`create_opt`), turned into a `WebTemplate` by
    /// the `v2_4` front end
    /// ([`openehr_sdt::flat::webtemplate::builder_v2_4::build_web_template_v2_4`]),
    /// and walked into a canonical example COMPOSITION at the requested
    /// [`DetailLevel`] by the same generator the ADL 1.4 example endpoint uses.
    /// The `output` form ([`ExampleType::Output`]) carries a deterministic `uid`.
    /// Example generation is not spec-mandated; a generated example is validated
    /// by the template-independent RM-invariant and terminology pass
    /// ([`openehr_sdt::rm_instance::validate_rm_and_terminology`]).
    ///
    /// # Errors
    ///
    /// - [`ServiceError::NotFound`] (`404`) — no stored template matches
    ///   `template_id`.
    /// - [`ServiceError::Internal`] (`500`) — the stored source no longer parses
    ///   (it was engine-valid at upload, so this is a server fault).
    /// - [`ServiceError::ValidationFailed`] (`422`) — a constituent archetype
    ///   is not stored (one `VARXR` violation per missing archetype id).
    /// - [`ServiceError::Unprocessable`] (`422`) — the OPT cannot be compiled
    ///   otherwise or built into a `WebTemplate`.
    /// - [`ServiceError::BadRequest`] (`400`) — the template's root is neither a
    ///   COMPOSITION nor a party class, so no example exists for it.
    pub(super) async fn adl2_example(
        &self,
        template_id: &str,
        level: DetailLevel,
        kind: ExampleType,
    ) -> Result<Value, ServiceError> {
        let wt = self.web_template_adl2(template_id).await?;
        let mut composition = crate::templates::runtime::example_or_refusal(&wt, level)?;
        if kind == ExampleType::Output {
            apply_output_uid(&mut composition, &wt.template_id);
        }
        Ok(composition)
    }

    /// The [`WebTemplate`] of a stored ADL2 template: resolve `template_id` →
    /// HRID, fetch the source, compile it to its operational template, and build
    /// the Web Template with the `v2_4` front end
    /// ([`build_web_template_v2_4`]).
    /// The ADL2 twin of [`web_template`](Self::web_template) (which reads the
    /// ADL 1.4 OPT store), used by the example endpoint's FLAT/STRUCTURED
    /// negotiation.
    ///
    /// # Errors
    ///
    /// - [`ServiceError::NotFound`] (`404`) — no stored template matches.
    /// - [`ServiceError::Internal`] (`500`) — the stored source no longer parses.
    /// - [`ServiceError::Unprocessable`] (`422`) — the OPT cannot be compiled or
    ///   built into a `WebTemplate`.
    pub async fn web_template_adl2(&self, template_id: &str) -> Result<WebTemplate, ServiceError> {
        let opt = self.adl2_operational_template_for(template_id).await?;
        build_web_template_v2_4(&opt).map_err(|e| {
            ServiceError::content_invalid(
                Violation::new(format!(
                    "ADL2 template {template_id} could not be built into a WebTemplate: {e}"
                ))
                .with_source(e),
            )
        })
    }

    /// The (cached) [`WebTemplate`] for a stored ADL2/OPT2 template on the
    /// FLAT/STRUCTURED commit path, the ADL2 twin of
    /// [`web_template`](crate::service::FerroEhrService::web_template)'s
    /// OPT 1.4 resolution and reached as its fallback when a template id is not
    /// an ADL 1.4 template. The Web Template carries the AOM2
    /// archetype-conformance constraints ([`build_web_template_v2_4`]), so such
    /// a commit is constraint-checked exactly as an OPT 1.4 commit is.
    ///
    /// NOTE: the entry is cached under a dialect-namespaced key
    /// ([`ADL2_CACHE_NS`]) whose ASCII control separator no grammar-legal id can
    /// contain (BASE `docs/base_types/master05-identification_package.adoc`
    /// §Archetype Identifiers), so an OPT 1.4 and an ADL2 template sharing a
    /// `template_id` cannot collide. No openEHR spec governs the cache — our own
    /// design/extension.
    ///
    /// # Errors
    ///
    /// - [`ServiceError::Unprocessable`] (`422`) — no ADL2 template matches (an
    ///   unknown referenced template on a commit is a *semantic* error, matching
    ///   the OPT 1.4 side —
    ///   `docs/specs/openehr/ITS-REST/specifications/responses/422.yaml`) or the
    ///   stored OPT2 cannot be built into a `WebTemplate`.
    /// - [`ServiceError::Internal`] (`500`) — the stored ADL2 source no longer
    ///   parses (it was engine-valid at upload, so this is a server fault).
    /// - [`ServiceError::Database`] — a store read failed.
    pub(crate) async fn web_template_adl2_cached(
        &self,
        template_id: &str,
    ) -> Result<Arc<WebTemplate>, ServiceError> {
        let key = format!(
            "{ADL2_CACHE_NS}{}",
            crate::templates::identity::canonical_key(template_id)
        );
        if let Some(wt) = self.web_templates.get(&key).await {
            return Ok(wt);
        }
        // Compile the stored ADL2 source to its OPT2 (async) before the sync WT
        // build, mapping an unknown ADL2 template to the commit-path 422 the OPT
        // 1.4 side uses (`web_template`).
        let opt = match self.adl2_operational_template_for(template_id).await {
            Ok(opt) => opt,
            Err(ServiceError::NotFound(_)) => {
                return Err(ServiceError::content_invalid(Violation::new(format!(
                    "operational template not known: {template_id}"
                ))));
            }
            Err(e) => return Err(e),
        };
        self.web_templates
            .get_or_build(&key, || build_web_template_v2_4(&opt))
            .await
            .map_err(|e| {
                ServiceError::content_invalid(
                    Violation::new(format!(
                        "ADL2 template {template_id} could not be built into a WebTemplate: {}",
                        ErrorChain::new(&*e)
                    ))
                    .with_source(e),
                )
            })
    }

    /// Resolve a `template_id` to its stored ADL2 source and compile it to its
    /// operational template (OPT2). Shared by the example
    /// ([`web_template_adl2`](Self::web_template_adl2)) and commit
    /// ([`web_template_adl2_cached`](Self::web_template_adl2_cached)) paths.
    ///
    /// # Errors
    ///
    /// - [`ServiceError::NotFound`] (`404`) — no stored ADL2 template matches.
    /// - [`ServiceError::Internal`] (`500`) — the stored source no longer parses.
    /// - [`ServiceError::Unprocessable`] (`422`) — the OPT cannot be compiled.
    async fn adl2_operational_template_for(
        &self,
        template_id: &str,
    ) -> Result<OperationalTemplate, ServiceError> {
        let hrid = self.adl2_resolve(template_id, None).await?;
        let source = self.adl2_get(&hrid).await?;
        self.adl2_operational_template(&source).await
    }

    /// Compile stored ADL2 `source` to its operational template: a stored
    /// `operational_template` is parsed as-is; any other kind is flattened +
    /// compiled via `create_opt` (OPT2 master03). Both steps run on the
    /// engine stack ([`on_engine_stack`]).
    ///
    /// # Errors
    ///
    /// - The stored source no longer parses → `Internal` (`500`; a stored source
    ///   was engine-valid at upload, so this is a server fault).
    /// - A constituent archetype the template needs is not stored →
    ///   `ValidationFailed` (`422`), one `VARXR` violation per missing
    ///   archetype id ([`missing_constituents`]).
    /// - The OPT cannot be compiled otherwise (a cyclic constituent reference,
    ///   a lineage or nesting past the engine bound) → `Unprocessable`
    ///   (`422`).
    async fn adl2_operational_template(
        &self,
        source: &str,
    ) -> Result<OperationalTemplate, ServiceError> {
        let owned = source.to_owned();
        let archetype = on_engine_stack(move || parse_artefact(&owned, Dialect::Adl2))
            .await?
            .map_err(|errs| {
                ServiceError::exception(format!(
                    "stored ADL2 source no longer parses: {}",
                    join_syntax_errors(&errs)
                ))
            })?;
        if let Archetype::AuthoredArchetype(a) = &archetype
            && let AuthoredArchetype::OperationalTemplate(opt) = a.as_ref()
        {
            return Ok(opt.as_ref().clone());
        }
        let repo = self
            .adl2_repository(&mut *acquire(&self.pool).await?)
            .await?;
        on_engine_stack(move || {
            create_opt(&archetype, &repo).map_err(|e| match e {
                OptError::UnresolvedReference(_) => {
                    let missing = unresolved_constituents(&archetype, &repo);
                    if missing.is_empty() {
                        not_compiled(e)
                    } else {
                        missing_constituents(&missing)
                    }
                }
                other => not_compiled(other),
            })
        })
        .await?
    }

    /// The ADL2 source of the artefact with `ARCHETYPE_HRID` `an_id`; absent
    /// → `artefact_does_not_exist` (`404`).
    pub(super) async fn adl2_get(&self, an_id: &str) -> Result<String, ServiceError> {
        sqlx::query_scalar::<_, String>(
            "SELECT adl FROM adl2_artefact WHERE lower(hrid) = lower($1)",
        )
        .bind(an_id)
        .fetch_optional(&mut *acquire(&self.pool).await?)
        .await?
        .ok_or_else(|| ServiceError::not_found(Missing::Artefact, format!("ADL2 artefact {an_id}")))
    }

    /// The `ARCHETYPE_HRID`s of all stored ADL2 artefacts, paged in SQL.
    ///
    /// # Errors
    ///
    /// A database failure (`500`).
    pub async fn list_artefacts(&self, page: Page) -> Result<Vec<String>, ServiceError> {
        let (offset, limit) = page_bounds(page);
        Ok(sqlx::query_scalar::<_, String>(
            "SELECT hrid FROM adl2_artefact ORDER BY hrid OFFSET $1 LIMIT $2",
        )
        .bind(offset)
        .bind(limit)
        .fetch_all(&mut *acquire(&self.pool).await?)
        .await?)
    }

    /// The `ARCHETYPE_HRID`s of the stored ADL2 archetypes, paged in SQL.
    ///
    /// # Errors
    ///
    /// A database failure (`500`).
    pub async fn list_archetypes_adl2(&self, page: Page) -> Result<Vec<String>, ServiceError> {
        let (offset, limit) = page_bounds(page);
        Ok(sqlx::query_scalar::<_, String>(
            "SELECT hrid FROM adl2_artefact WHERE kind = 'archetype' \
             ORDER BY hrid OFFSET $1 LIMIT $2",
        )
        .bind(offset)
        .bind(limit)
        .fetch_all(&mut *acquire(&self.pool).await?)
        .await?)
    }

    /// Delete the ADL2 artefact `an_id` (case-insensitive).
    ///
    /// # Errors
    ///
    /// - No artefact with that HRID → [`Missing::Artefact`] (`404`).
    /// - A template committed versions use, or an artefact another stored one
    ///   depends on → `Conflict` (`409`; no openEHR spec governs the in-use
    ///   refusal, our own integrity design).
    /// - A database failure (`500`).
    pub async fn delete_artefact(&self, an_id: &str) -> Result<(), ServiceError> {
        // Resolve the stored (case-preserved) HRID + kind, count references,
        // and delete in ONE transaction, mirroring `opt_delete`; the
        // `version.template_id` → `template_ref` foreign key (NO ACTION)
        // remains the race-free backstop under a concurrent commit.
        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await?;
        adl2_write_lock(&mut tx).await?;
        let stored: Option<(String, String)> =
            sqlx::query_as("SELECT hrid, kind FROM adl2_artefact WHERE lower(hrid) = lower($1)")
                .bind(an_id)
                .fetch_optional(&mut *tx)
                .await?;
        let Some((hrid, kind)) = stored else {
            return Err(ServiceError::not_found(
                Missing::Artefact,
                format!("ADL2 artefact {an_id}"),
            ));
        };
        let is_template = matches!(kind.as_str(), "template" | "operational_template");
        if is_template {
            // Counted over BOTH storage tiers (see the ADL 1.4 twin): an
            // archived composition's template reference is invisible to the
            // `template_ref` foreign key, and deleting under it would make that
            // object unrestorable.
            let refs: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM version WHERE lower(template_id) = lower($1)",
            )
            .bind(&hrid)
            .fetch_one(&mut *tx)
            .await?;
            if refs > 0 {
                return Err(ServiceError::conflict(format!(
                    "template '{hrid}' is still referenced by {refs} committed version(s); \
                     delete those compositions before deleting the template"
                )));
            }
        }
        self.adl2_refuse_in_use(&mut tx, &hrid).await?;
        sqlx::query("DELETE FROM adl2_artefact WHERE hrid = $1")
            .bind(&hrid)
            .execute(&mut *tx)
            .await?;
        if is_template {
            // Deregister the wire address unless the OPT 1.4 store also claims
            // it (`template_ref` is the union of both dialects' addresses).
            sqlx::query(
                "DELETE FROM template_ref WHERE template_id = $1 AND NOT EXISTS \
                 (SELECT 1 FROM template_store WHERE lower(template_id) = lower($1))",
            )
            .bind(&hrid)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        if is_template {
            // Evict the built `WebTemplate` so a re-uploaded template is never
            // served from the deleted artefact's compiled form (the OPT 1.4
            // delete paths do the same; no openEHR spec governs the cache).
            self.web_templates
                .invalidate(&format!(
                    "{ADL2_CACHE_NS}{}",
                    crate::templates::identity::canonical_key(&hrid)
                ))
                .await;
        }
        self.invalidate_archetype_lineage().await;
        Ok(())
    }

    /// Total stored ADL2 artefacts.
    ///
    /// # Errors
    ///
    /// A database failure (`500`).
    pub async fn artefacts_count(&self) -> Result<i64, ServiceError> {
        Ok(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM adl2_artefact")
                .fetch_one(&mut *acquire(&self.pool).await?)
                .await?,
        )
    }

    /// Total stored ADL2 artefacts of kind `archetype`.
    ///
    /// # Errors
    ///
    /// A database failure (`500`).
    pub async fn archetypes_count_adl2(&self) -> Result<i64, ServiceError> {
        Ok(sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM adl2_artefact WHERE kind = 'archetype'",
        )
        .fetch_one(&mut *acquire(&self.pool).await?)
        .await?)
    }

    /// The wire list for `GET /definition/template/adl2`: the ADL2 templates and
    /// OPTs as `TemplateMetadata` objects
    /// (`schemas/definition/TemplateMetadata.yaml`:
    /// `{template_id, concept, archetype_id, created_timestamp}`). `archetype_id`
    /// is the stored `ARCHETYPE_HRID`; `concept` is its concept segment
    /// (`AOM2/master07.05` §Physical Archetype Identifier — the concept derives
    /// from the HRID, so no cADL parse is needed for the list). Lists the
    /// `template` and `operational_template` kinds (the "templates" under
    /// `/definition/template/adl2`), not source archetypes.
    pub(super) async fn adl2_template_list(&self, page: Page) -> Result<Vec<Value>, ServiceError> {
        let (offset, limit) = page_bounds(page);
        let rows = sqlx::query(
            "SELECT hrid, created_at FROM adl2_artefact \
             WHERE kind IN ('template', 'operational_template') \
             ORDER BY hrid OFFSET $1 LIMIT $2",
        )
        .bind(offset)
        .bind(limit)
        .fetch_all(&mut *acquire(&self.pool).await?)
        .await?;
        // NOT NULL columns decode infallibly on a healthy row; a decode fault
        // is a real storage error and surfaces instead of blanking the field.
        rows.iter()
            .map(|row| {
                let hrid: String = row.try_get("hrid")?;
                let created = row
                    .try_get::<jiff_sqlx::Timestamp, _>("created_at")?
                    .to_jiff()
                    .to_string();
                let concept = raw_id_concept(&hrid);
                Ok(serde_json::json!({
                    "template_id": hrid,
                    "concept": concept,
                    "archetype_id": hrid,
                    "created_timestamp": created,
                }))
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()
            .map_err(ServiceError::from)
    }
}

// ── stateless helpers ─────────────────────────────────────────────────────────

/// Join a parse failure's typed [`openehr_adl::error::SyntaxError`]s (S-codes)
/// into one detail string, each rendered `CODE at line L, column C: message`.
fn join_syntax_errors(errs: &[openehr_adl::error::SyntaxError]) -> String {
    errs.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// Map an ADL2 parse failure to a [`ServiceError::BadRequest`] carrying the
/// joined S-code details — an unparseable source is *syntactically invalid
/// content*, the released `400` branch declared on the upload
/// (`docs/specs/openehr/ITS-REST/specifications/responses/400.yaml`: "the
/// request could not be parsed or is invalid (e.g. … syntactically invalid …
/// content)"). AOM2 validation-phase failures (V-codes) on a *parsed* source
/// are [`ServiceError::ValidationFailed`] instead (see
/// [`FerroEhrService::template_adl2_upload`]), which the template upload answers `400` too.
fn syntax_bad_request(errs: &[openehr_adl::error::SyntaxError]) -> ServiceError {
    ServiceError::precondition(format!(
        "syntactically invalid ADL2 content: {}",
        join_syntax_errors(errs)
    ))
}

/// Map an artefact `kind` to the value the storage `kind` column accepts. The
/// AOM2 keyword set includes `template_overlay`, but the storage `kind` domain
/// is `{archetype, template, operational_template}` (our own design — no openEHR
/// spec governs the schema); an overlay is a specialising fragment of a
/// template, so it is stored under `template`. This keeps an ADL2 upload from
/// ever reaching a DB constraint (a malformed upload is refused at validation,
/// never a `500`).
fn store_kind(kind: &str) -> &str {
    match kind {
        "template_overlay" => "template",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use openehr_adl::hrid::{raw_id_lookup_key, raw_id_version};

    use super::*;

    #[test]
    fn template_overlay_folds_into_the_template_storage_kind() {
        // the storage `kind` domain excludes `template_overlay`; it is
        // stored as `template` so an upload never hits the DB CHECK.
        assert_eq!(store_kind("template_overlay"), "template");
        assert_eq!(store_kind("archetype"), "archetype");
        assert_eq!(store_kind("template"), "template");
        assert_eq!(store_kind("operational_template"), "operational_template");
    }

    #[test]
    fn hrid_concept_and_family_extract_from_the_identifier() {
        // The HRID grammar has ONE reading, owned by `openehr_adl::hrid`; this
        // pins that the template-resolution path reads identifiers through it.
        let hrid = "org.example::openEHR-EHR-COMPOSITION.vital_signs.v1.2.3";
        assert_eq!(raw_id_concept(hrid), "vital_signs");
        assert_eq!(
            raw_id_lookup_key(hrid),
            "openehr-ehr-composition.vital_signs"
        );
        assert_eq!(raw_id_version(hrid), "1.2.3");
        // the pre-release qualifier is not part of the release version
        let hrid = "openEHR-EHR-OBSERVATION.lab_result.v2.0.0-rc.1";
        assert_eq!(raw_id_concept(hrid), "lab_result");
        assert_eq!(raw_id_version(hrid), "2.0.0");
    }
}
