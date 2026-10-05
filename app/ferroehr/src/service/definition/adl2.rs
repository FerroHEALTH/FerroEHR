// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `I_DEFINITION_ADL2` (`i_definition_adl2.adoc`): ADL2 artefacts (archetype /
//! template / `operational_template`) keyed by `ARCHETYPE_HRID`, on the
//! `adl2_artefact` store.
//!
//! Validation is the real `openehr-adl` engine (parse → AOM2 phase 1 → RM
//! phase 2 → specialisation phase 2 against the flat parent). The engine needs
//! an [`ArchetypeRepository`] to resolve specialisation parents +
//! `use_archetype` fillers; [`FerroEhrService::adl2_repository`] builds one by
//! parsing every stored ADL2 source (the registry is low-volume — SM
//! `I_DEFINITION_ADL2` is an admin surface — so parse-on-demand is cheap; the
//! `openehr-adl` phases degrade gracefully when a parent is unresolved).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): stored template/query artefacts served verbatim + \
              ADL/OPT wire envelopes"
)]

use std::collections::HashMap;
use std::sync::Arc;

use openehr_adl::artefact::ArchetypeRepository;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::hrid::{raw_id_concept, raw_id_lookup_key, raw_id_version};
use openehr_adl::meta::{ArtefactSummary, summarize};
use openehr_adl::opt::create_opt;
use openehr_adl::parse::Dialect;
use openehr_adl::validate::bindings::{TerminologyResolver, external_term_bindings};
use openehr_adl::validate::catalogue::Severity;
use openehr_adl::validate::rm::{ProductionRmModel, production_model_governs};
use openehr_adl::validate::{validate_source, validate_source_integrity};
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetype;
use openehr_am::v2_4::aom2::archetype::operational_template::OperationalTemplate;
use openehr_base::validate::InvariantViolation;
use openehr_sdt::flat::example::{DetailLevel, ExampleType, apply_output_uid, example_composition};
use openehr_sdt::flat::webtemplate::builder_v2_4::build_web_template_v2_4;

use crate::service::definition::binding_uri::code_system_and_code;
use crate::service::terminology::fhir::TermExistence;
use openehr_sdt::flat::webtemplate::model::WebTemplate;
use serde_json::Value;
use sqlx::Row;

use crate::service::FerroEhrService;
use crate::service::error::{ServiceError, Violation};
use crate::service::list::Page;
use crate::service::status::{CallStatusType, SmError};

use super::{compile_pattern, page_bounds, paginate};

/// The `WebTemplate`-cache key namespace for ADL2/OPT2 templates. The trailing
/// ASCII Unit Separator (`U+001F`) cannot appear in a grammar-legal
/// archetype/template id (printable ASCII —
/// `docs/specs/openehr/BASE/docs/base_types/master05-identification_package.adoc`
/// §Archetype Identifiers), so an ADL2 entry can never collide with an OPT 1.4
/// entry keyed by the plain identity-canonical form. No openEHR spec governs the
/// cache — our own design/extension.
const ADL2_CACHE_NS: &str = "adl2\u{1f}";

/// A memoised [`TerminologyResolver`] for ADL2 VETDF validation.
///
/// `openehr-adl` is a network-free spec engine whose VETDF check consults a
/// synchronous [`TerminologyResolver`] seam, while a terminology lookup is
/// asynchronous. The service therefore pre-resolves every external term binding
/// of the uploaded archetype against its terminology service
/// ([`FerroEhrService::term_existence`], on the system and code the binding
/// URI names) and hands the validator this memoised map.
///
/// `code_exists` returns `Some(true)`/`Some(false)` for a binding the service
/// could answer, and `None` for one it could not (no external provider
/// configured, a code system the server does not serve, or a transport
/// fault) — matching the
/// VETDF "subject to tool accessibility; … no verification was possible"
/// carve-out (AM ADL2 `master03-archetype_package.adoc` §Validity Rules).
#[derive(Debug, Default)]
struct AdlTerminologyResolver {
    /// `(terminology_id, external target)` → the target's existence, for every
    /// binding the terminology service could answer.
    resolved: HashMap<(String, String), bool>,
}

impl TerminologyResolver for AdlTerminologyResolver {
    fn code_exists(&self, terminology_id: &str, code: &str) -> Option<bool> {
        self.resolved
            .get(&(terminology_id.to_owned(), code.to_owned()))
            .copied()
    }
}

// ── SM Definitions native API (I_DEFINITION_ADL2) — the catalog contract ─────

impl FerroEhrService {
    /// `has_artefact` — true if an ADL2 artefact with `ARCHETYPE_HRID` `an_id`
    /// is stored. HRID identity is case-insensitive (BASE master05 §Composite
    /// Identifiers and Case).
    ///
    /// # Errors
    ///
    /// A database failure (`exception` → `500`).
    pub async fn has_artefact(&self, an_id: String) -> Result<bool, SmError> {
        Ok(self.adl2_exists(&an_id).await?)
    }

    /// `valid_artefact` — the AOM2 phase-1 basic-integrity validity of ADL2
    /// source (`openehr-adl` engine, standalone: parse + phase 1 with no
    /// registry). Parent-dependent checks (VACSD against a stored parent) run at
    /// `upload_artefact`, where the registry is available. Stateless.
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
    pub fn valid_artefact(&self, adl2: &str) -> Result<bool, SmError> {
        Ok(matches!(
            validate_source_integrity(adl2, Dialect::Adl2, None),
            Ok(issues) if issues.iter().all(|i| i.severity != Severity::Error)
        ))
    }

    /// `upload_artefact` (Pre `valid_artefact`, Post `has_artefact`) — validate a
    /// full ADL2 artefact through the `openehr-adl` engine and store it.
    ///
    /// An existing artefact with the same `ARCHETYPE_HRID` is a conflict, never
    /// replaced: the released REST API answers `409` (ITS-REST definition OAS,
    /// `POST /definition/template/adl2` → `409_template_already_exists`), and
    /// the wire is the API oracle where the SM's `i_definition_adl2.adoc`
    /// wording ("replace it") differs.
    ///
    /// # Errors
    ///
    /// - Source that fails to parse → bad request (`400`, syntactically
    ///   invalid content — `responses/400.yaml`); source that fails an AOM2
    ///   validation phase → `invalid_artefact` (`422`, via [`ServiceError`]).
    /// - A database failure (`exception` → `500`).
    pub async fn upload_artefact(&self, adl2: String) -> Result<(), SmError> {
        let summary = self.adl2_validate(&adl2).await?;
        self.adl2_persist(&summary, &adl2, true).await?;
        Ok(())
    }

    /// `get_artefact` — the ADL2 source of the artefact with `ARCHETYPE_HRID`
    /// `an_id` (interchange form). Identity is case-insensitive.
    ///
    /// # Errors
    ///
    /// - No artefact with that HRID → `artefact_does_not_exist` (`404`).
    /// - A database failure (`exception` → `500`).
    pub async fn get_artefact(&self, an_id: String) -> Result<String, SmError> {
        Ok(self.adl2_get(&an_id).await?)
    }

    /// `list_artefacts` — the `ARCHETYPE_HRID`s of all stored ADL2 artefacts,
    /// cursored by `page`.
    ///
    /// # Errors
    ///
    /// A database failure (`exception` → `500`).
    pub async fn list_artefacts(&self, page: Page) -> Result<Vec<String>, SmError> {
        Ok(self.adl2_list(page).await?)
    }

    /// `list_archetypes` — the HRIDs of stored ADL2 artefacts of kind
    /// `archetype`, cursored by `page`.
    ///
    /// # Errors
    ///
    /// A database failure (`exception` → `500`).
    pub async fn list_archetypes_adl2(&self, page: Page) -> Result<Vec<String>, SmError> {
        Ok(self.adl2_list_by_kind("archetype", page).await?)
    }

    /// `list_templates` — the HRIDs of stored ADL2 artefacts of kind
    /// `template`, cursored by `page`.
    ///
    /// # Errors
    ///
    /// A database failure (`exception` → `500`).
    pub async fn list_templates_adl2(&self, page: Page) -> Result<Vec<String>, SmError> {
        Ok(self.adl2_list_by_kind("template", page).await?)
    }

    /// `list_opts` — the HRIDs of stored ADL2 artefacts of kind
    /// `operational_template`, cursored by `page`.
    ///
    /// # Errors
    ///
    /// A database failure (`exception` → `500`).
    pub async fn list_opts_adl2(&self, page: Page) -> Result<Vec<String>, SmError> {
        Ok(self.adl2_list_by_kind("operational_template", page).await?)
    }

    /// `list_matching_artefacts` — HRIDs matching `id_pattern` (a regex),
    /// cursored by `page`.
    ///
    /// # Errors
    ///
    /// - An uncompilable `id_pattern` → `invalid_id_pattern` (`400`).
    /// - A database failure (`exception` → `500`).
    pub async fn list_matching_artefacts(
        &self,
        id_pattern: String,
        page: Page,
    ) -> Result<Vec<String>, SmError> {
        Ok(self.adl2_list_matching(&id_pattern, page).await?)
    }

    /// `delete_artefact` — delete the ADL2 artefact with `ARCHETYPE_HRID`
    /// `an_id` (case-insensitive).
    ///
    /// # Errors
    ///
    /// - No artefact with that HRID → `artefact_does_not_exist` (`404`).
    /// - A database failure (`exception` → `500`).
    pub async fn delete_artefact(&self, an_id: String) -> Result<(), SmError> {
        Ok(self.adl2_delete(&an_id).await?)
    }

    /// `artefacts_count` — total stored ADL2 artefacts.
    ///
    /// # Errors
    ///
    /// A database failure (`exception` → `500`).
    pub async fn artefacts_count(&self) -> Result<i64, SmError> {
        Ok(self.adl2_count().await?)
    }

    /// `archetypes_count` — total stored ADL2 artefacts of kind `archetype`.
    ///
    /// # Errors
    ///
    /// A database failure (`exception` → `500`).
    pub async fn archetypes_count_adl2(&self) -> Result<i64, SmError> {
        Ok(self.adl2_count_by_kind("archetype").await?)
    }

    /// `templates_count` — total stored ADL2 artefacts of kind `template`.
    ///
    /// # Errors
    ///
    /// A database failure (`exception` → `500`).
    pub async fn templates_count(&self) -> Result<i64, SmError> {
        Ok(self.adl2_count_by_kind("template").await?)
    }

    /// `opts_count` — total stored ADL2 artefacts of kind
    /// `operational_template`.
    ///
    /// # Errors
    ///
    /// A database failure (`exception` → `500`).
    pub async fn opts_count_adl2(&self) -> Result<i64, SmError> {
        Ok(self.adl2_count_by_kind("operational_template").await?)
    }
}

// ── domain logic (the ServiceError layer under the catalog) ──────────────────

impl FerroEhrService {
    /// True if an ADL2 artefact with `ARCHETYPE_HRID` `an_id` is stored
    /// (case-insensitive identity).
    pub(super) async fn adl2_exists(&self, an_id: &str) -> Result<bool, ServiceError> {
        Ok(sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM adl2_artefact WHERE lower(hrid) = lower($1))",
        )
        .bind(an_id)
        .fetch_one(&self.pool)
        .await?)
    }

    /// Build an in-memory [`ArchetypeRepository`] from every stored ADL2 source,
    /// so the `openehr-adl` engine can resolve specialisation parents +
    /// `use_archetype` fillers when validating an upload or projecting an OPT.
    ///
    /// A stored source that no longer parses is skipped rather than failing the
    /// whole build; every source was engine-validated at its own upload.
    ///
    /// NOTE: the repository is re-parsed on every call, this path being reached
    /// only for an ADL2 upload or an OPT projection, so a memoized copy would
    /// cost invalidation on every ADL2 write for no clinical-path gain. No
    /// openEHR spec governs this — our own design/extension.
    async fn adl2_repository(&self) -> Result<ArchetypeRepository, ServiceError> {
        let sources: Vec<String> = sqlx::query_scalar("SELECT adl FROM adl2_artefact")
            .fetch_all(&self.pool)
            .await?;
        on_engine_stack(move || {
            let mut repo = ArchetypeRepository::new();
            for src in &sources {
                if let Ok(archetype) = parse_artefact(src, Dialect::Adl2) {
                    repo.insert(archetype);
                }
            }
            repo
        })
        .await
    }

    /// Validate one uploaded ADL2 source with the `openehr-adl` engine and return
    /// its identity [`ArtefactSummary`].
    ///
    /// Invalidity splits on the syntax/semantics line: an unparseable source
    /// (S-codes) is syntactically invalid content, the released `400` branch
    /// (`responses/400.yaml`, via [`syntax_bad_request`]), while an AOM2
    /// validation-phase failure (V-codes) on a parsed source is a
    /// `ValidationFailed` (`422`) rendering the ITS-REST `Error` object with the
    /// rule-code mnemonics as `validationErrors[]`. The openEHR
    /// [`ProductionRmModel`] governs openEHR-published archetypes; a foreign or
    /// test model skips the RM pass, which would false-fire VCORM
    /// (`AOM2/master04.3` §Reference Model Type Matching). External term
    /// bindings are verified through the terminology-service resolver (VETDF,
    /// `master03` §Validity Rules; see [`Self::adl2_terminology_resolver`]).
    async fn adl2_validate(&self, source: &str) -> Result<ArtefactSummary, ServiceError> {
        let owned = source.to_owned();
        let archetype = on_engine_stack(move || parse_artefact(&owned, Dialect::Adl2))
            .await?
            .map_err(|errs| syntax_bad_request(&errs))?;
        let repo = self.adl2_repository().await?;
        let governed = production_model_governs(&archetype);
        // VETDF: external term bindings are verified against the terminology
        // service, pre-resolved here into the synchronous validator seam
        // (`master03` §Validity Rules; see [`AdlTerminologyResolver`]).
        let resolver = if governed {
            Some(self.adl2_terminology_resolver(&archetype).await)
        } else {
            None
        };
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
        Ok(summarize(&archetype))
    }

    /// Pre-resolve every external term binding of `archetype` against the
    /// terminology service, building the [`AdlTerminologyResolver`] the VETDF
    /// check consults (see its docs — the seam is synchronous, a lookup is not).
    ///
    /// A binding target is a URI in the IHTSDO model (`…/id/<code>`), so it is
    /// taken apart into the FHIR `(system, code)` the server is asked about
    /// ([`code_system_and_code`]); a target of another shape is asked as
    /// written under the binding's own terminology id. Only a definite answer
    /// lands in the resolver: exists, or absent from a code system the server
    /// serves. A binding the service cannot verify — no configured external
    /// provider, a code system the server does not serve, a transport fault —
    /// is left unresolved and logged, so VETDF is not raised for it
    /// (`master03` §Validity Rules: "codes for inaccessible terminologies
    /// should be flagged with a warning indicating that no verification was
    /// possible"). Only genuinely external terminologies are consulted; the
    /// archetype-internal `local`/`openehr` ids are excluded by
    /// [`external_term_bindings`] (their keys are covered by VTTBK/VTCBK).
    async fn adl2_terminology_resolver(&self, archetype: &Archetype) -> AdlTerminologyResolver {
        let mut resolved = HashMap::new();
        for binding in external_term_bindings(archetype) {
            let (system, code) = code_system_and_code(&binding.target)
                .unwrap_or_else(|| (binding.terminology_id.clone(), binding.target.clone()));
            match self
                .term_existence(&binding.terminology_id, &system, &code)
                .await
            {
                Ok(TermExistence::Exists) => {
                    resolved.insert((binding.terminology_id, binding.target), true);
                }
                Ok(TermExistence::Absent) => {
                    resolved.insert((binding.terminology_id, binding.target), false);
                }
                Ok(TermExistence::Unverifiable(reason)) => tracing::warn!(
                    terminology = %binding.terminology_id,
                    target = %binding.target,
                    system = %system,
                    reason,
                    "VETDF: no verification was possible for this term binding; accepted unverified"
                ),
                Err(e) => tracing::warn!(
                    terminology = %binding.terminology_id,
                    target = %binding.target,
                    system = %system,
                    error = %e,
                    "VETDF: the terminology server could not be asked; accepted unverified"
                ),
            }
        }
        AdlTerminologyResolver { resolved }
    }

    /// Store a validated ADL2 artefact. With `replace`, any case-variant of the
    /// same HRID is removed first (SM `upload_artefact` replace semantics); the
    /// insert is then verbatim (BASE master05 §Composite Identifiers and Case).
    ///
    /// The artefact's declared `specialize` parent travels with it into
    /// `parent_hrid`: it is the archetype-lineage edge AQL resolves a parent
    /// query through (AM `Identification` master07 §Supporting Archetype-based
    /// Querying — for a specialised archetype the lineage "can only be obtained
    /// from the operational form of the archetype"), and the engine has already
    /// extracted it into the [`ArtefactSummary`] during validation.
    async fn adl2_persist(
        &self,
        summary: &ArtefactSummary,
        source: &str,
        replace: bool,
    ) -> Result<(), ServiceError> {
        let kind = store_kind(summary.kind);
        let mut tx = self.pool.begin().await?;
        if replace {
            sqlx::query("DELETE FROM adl2_artefact WHERE lower(hrid) = lower($1)")
                .bind(&summary.archetype_id)
                .execute(&mut *tx)
                .await?;
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
        // §template_ref). A
        // replace that DEMOTES a template to an archetype deregisters the id
        // unless the OPT 1.4 store also claims it — and the FK blocks the
        // deregistration loudly when committed versions still reference it.
        if matches!(kind, "template" | "operational_template") {
            sqlx::query(
                "INSERT INTO template_ref (template_id) VALUES ($1) ON CONFLICT DO NOTHING",
            )
            .bind(&summary.archetype_id)
            .execute(&mut *tx)
            .await?;
        } else if replace {
            sqlx::query(
                "DELETE FROM template_ref WHERE template_id = $1 AND NOT EXISTS \
                 (SELECT 1 FROM template_store WHERE lower(template_id) = lower($1))",
            )
            .bind(&summary.archetype_id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        self.invalidate_archetype_lineage().await;
        Ok(())
    }

    /// The wire upload path (`POST /definition/template/adl2`): validate through
    /// the engine, reject a duplicate HRID with a `409` (the REST contract
    /// declares `409_template_already_exists.yaml`, diverging from the SM native
    /// replace), then store. Returns the stored `ARCHETYPE_HRID`.
    ///
    /// # Errors
    ///
    /// - Unparseable source → `BadRequest` (`400`).
    /// - AOM2-invalid source → `ValidationFailed` (`422` with rule codes).
    /// - An ADL2 artefact with the same HRID already stored → `Conflict`
    ///   (`409`).
    /// - A database failure (`500`).
    pub(super) async fn adl2_wire_upload(&self, source: &str) -> Result<String, ServiceError> {
        let summary = self.adl2_validate(source).await?;
        if self.adl2_exists(&summary.archetype_id).await? {
            return Err(ServiceError::conflict(format!(
                "an ADL2 template with id '{}' already exists",
                summary.archetype_id
            )));
        }
        self.adl2_persist(&summary, source, false).await?;
        Ok(summary.archetype_id)
    }

    /// Resolve a wire `template_id` (`+` optional `version`) to the exact stored
    /// `ARCHETYPE_HRID`. A full HRID matches case-insensitively; a partial
    /// `template_id` (`…concept.v1`, or `…concept` + a `version` prefix)
    /// resolves to the **highest** stored version whose family +
    /// `{major}[.{minor}[.{patch}]]` prefix match (`template_id_adl2.yaml` — "a
    /// partial `template_id` will resolve to 'latest' major version";
    /// `version.yaml` — "a pattern as partial prefix … highest matching").
    ///
    /// # Errors
    ///
    /// No stored artefact matches → `NotFound` (`404`).
    pub(super) async fn adl2_resolve(
        &self,
        template_id: &str,
        version: Option<&str>,
    ) -> Result<String, ServiceError> {
        // Fast path: an exact HRID (no explicit version filter).
        if version.is_none()
            && let Some(hrid) = sqlx::query_scalar::<_, String>(
                "SELECT hrid FROM adl2_artefact WHERE lower(hrid) = lower($1)",
            )
            .bind(template_id)
            .fetch_optional(&self.pool)
            .await?
        {
            return Ok(hrid);
        }
        // Partial resolution over the (small) registry: match the family and the
        // version prefix, then take the highest release version.
        let family = raw_id_lookup_key(template_id);
        let want_version = version.map(str::to_owned).or_else(|| {
            let v = raw_id_version(template_id);
            (!v.is_empty()).then(|| v.to_owned())
        });
        let hrids: Vec<String> = sqlx::query_scalar("SELECT hrid FROM adl2_artefact")
            .fetch_all(&self.pool)
            .await?;
        hrids
            .into_iter()
            .filter(|h| raw_id_lookup_key(h) == family)
            .filter(|h| {
                want_version
                    .as_deref()
                    .is_none_or(|want| version_prefix_matches(raw_id_version(h), want))
            })
            .max_by(|a, b| cmp_version(raw_id_version(a), raw_id_version(b)))
            .ok_or_else(|| {
                ServiceError::sm(
                    CallStatusType::TemplateDoesNotExist,
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
    /// - The OPT cannot be compiled (an unresolved constituent reference) →
    ///   `Unprocessable` (`422`).
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
    /// - [`ServiceError::Unprocessable`] (`422`) — the OPT cannot be compiled (an
    ///   unresolved constituent reference) or built into a `WebTemplate`.
    pub(super) async fn adl2_example(
        &self,
        template_id: &str,
        level: DetailLevel,
        kind: ExampleType,
    ) -> Result<Value, ServiceError> {
        let wt = self.web_template_adl2(template_id).await?;
        let mut composition = example_composition(&wt, level);
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
    /// [`web_template_for`](crate::service::FerroEhrService::web_template_for)'s
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
        // 1.4 side uses (`web_template_for`).
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
                        "ADL2 template {template_id} could not be built into a WebTemplate: {e}"
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
    /// - The OPT cannot be compiled (an unresolved or cyclic constituent
    ///   reference, a lineage or nesting past the engine bound) →
    ///   `Unprocessable` (`422`).
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
        let repo = self.adl2_repository().await?;
        on_engine_stack(move || create_opt(&archetype, &repo))
            .await?
            .map_err(|e| {
                ServiceError::content_invalid(
                    Violation::new(format!("cannot compile operational template: {e}"))
                        .with_source(e),
                )
            })
    }

    /// The ADL2 source of the artefact with `ARCHETYPE_HRID` `an_id`; absent
    /// → `artefact_does_not_exist` (`404`).
    pub(super) async fn adl2_get(&self, an_id: &str) -> Result<String, ServiceError> {
        sqlx::query_scalar::<_, String>(
            "SELECT adl FROM adl2_artefact WHERE lower(hrid) = lower($1)",
        )
        .bind(an_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            ServiceError::sm(
                CallStatusType::ArtefactDoesNotExist,
                format!("ADL2 artefact {an_id}"),
            )
        })
    }

    /// The `ARCHETYPE_HRID`s of all stored ADL2 artefacts, paged in SQL.
    async fn adl2_list(&self, page: Page) -> Result<Vec<String>, ServiceError> {
        let (offset, limit) = page_bounds(page);
        Ok(sqlx::query_scalar::<_, String>(
            "SELECT hrid FROM adl2_artefact ORDER BY hrid OFFSET $1 LIMIT $2",
        )
        .bind(offset)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// The `ARCHETYPE_HRID`s of stored ADL2 artefacts of one concrete `kind`,
    /// paged in SQL.
    async fn adl2_list_by_kind(&self, kind: &str, page: Page) -> Result<Vec<String>, ServiceError> {
        let (offset, limit) = page_bounds(page);
        Ok(sqlx::query_scalar::<_, String>(
            "SELECT hrid FROM adl2_artefact WHERE kind = $1 ORDER BY hrid OFFSET $2 LIMIT $3",
        )
        .bind(kind)
        .bind(offset)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// HRIDs matching `id_pattern` (regex; uncompilable →
    /// `invalid_id_pattern`, `400`), then paged.
    async fn adl2_list_matching(
        &self,
        id_pattern: &str,
        page: Page,
    ) -> Result<Vec<String>, ServiceError> {
        let re = compile_pattern(id_pattern)?;
        let all: Vec<String> = sqlx::query_scalar("SELECT hrid FROM adl2_artefact ORDER BY hrid")
            .fetch_all(&self.pool)
            .await?;
        Ok(paginate(all.into_iter().filter(|id| re.is_match(id)), page))
    }

    /// Delete the ADL2 artefact with `ARCHETYPE_HRID` `an_id`
    /// (case-insensitive); absent → `artefact_does_not_exist` (`404`); a
    /// template-kind artefact still referenced by committed versions →
    /// `Conflict` (`409`, with the reference count — the same
    /// never-orphan-clinical-data guard the OPT 1.4 delete carries; no openEHR
    /// spec governs the in-use refusal, our own integrity design).
    async fn adl2_delete(&self, an_id: &str) -> Result<(), ServiceError> {
        // Resolve the stored (case-preserved) HRID + kind, count references,
        // and delete in ONE transaction, mirroring `opt_delete`; the
        // `version.template_id` → `template_ref` foreign key (NO ACTION)
        // remains the race-free backstop under a concurrent commit.
        let mut tx = self.pool.begin().await?;
        let stored: Option<(String, String)> =
            sqlx::query_as("SELECT hrid, kind FROM adl2_artefact WHERE lower(hrid) = lower($1)")
                .bind(an_id)
                .fetch_optional(&mut *tx)
                .await?;
        let Some((hrid, kind)) = stored else {
            return Err(ServiceError::sm(
                CallStatusType::ArtefactDoesNotExist,
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

    /// Total ADL2 artefacts.
    async fn adl2_count(&self) -> Result<i64, ServiceError> {
        Ok(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM adl2_artefact")
                .fetch_one(&self.pool)
                .await?,
        )
    }

    /// Total ADL2 artefacts of one concrete `kind`.
    async fn adl2_count_by_kind(&self, kind: &str) -> Result<i64, ServiceError> {
        Ok(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM adl2_artefact WHERE kind = $1")
                .bind(kind)
                .fetch_one(&self.pool)
                .await?,
        )
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
        .fetch_all(&self.pool)
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
/// are the semantic `422` branch instead (see [`FerroEhrService::adl2_validate`]).
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
/// ever reaching a DB constraint (a malformed upload is a `422` at validation,
/// never a `500`).
fn store_kind(kind: &str) -> &str {
    match kind {
        "template_overlay" => "template",
        other => other,
    }
}

/// Whether `full` (a `major.minor.patch` release) matches the SEMVER `prefix`
/// (`1`, `1.2`, or `1.2.3` — `version.yaml`: "an exact version … or a pattern
/// as partial prefix"). Each supplied prefix component must equal the
/// corresponding `full` component.
fn version_prefix_matches(full: &str, prefix: &str) -> bool {
    let mut want = prefix.split('.').filter(|p| !p.is_empty() && *p != "*");
    let mut have = full.split('.');
    want.all(|w| have.next() == Some(w))
}

/// Compare two numeric release versions (`major.minor.patch`) component-wise so
/// the highest matching version can be selected (`Ordering::Equal` on ties).
fn cmp_version(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |v: &str| -> Vec<u64> { v.split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    parse(a).cmp(&parse(b))
}

/// The stack the AOM engine runs on.
///
/// Parsing, flattening and validating an archetype recurse over the artefact
/// and over the stored repository (slot fillers, specialisation parents), and
/// a well-stocked store crossed a tokio worker's 2 MiB stack (#3062). The
/// reservation is virtual: pages are committed only as the stack grows.
const ENGINE_STACK_BYTES: usize = 256 * 1024 * 1024;

/// Run `f` on a dedicated thread with [`ENGINE_STACK_BYTES`] of stack and await
/// its result without blocking the runtime.
///
/// # Errors
/// [`ServiceError::Internal`] when the thread cannot be spawned or ends without
/// delivering a result (a panic inside `f` surfaces as that, never as an abort
/// of the process).
async fn on_engine_stack<T, F>(f: F) -> Result<T, ServiceError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("adl2-engine".to_owned())
        .stack_size(ENGINE_STACK_BYTES)
        .spawn(move || {
            // The receiver is gone only when the request was cancelled, in
            // which case the result has no reader left.
            drop(tx.send(f()));
        })
        .map_err(|e| ServiceError::internal("spawning the ADL2 engine thread", e))?;
    rx.await
        .map_err(|e| ServiceError::internal("the ADL2 engine thread ended without a result", e))
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn version_prefix_matches_semver_prefix() {
        assert!(version_prefix_matches("1.2.3", "1"));
        assert!(version_prefix_matches("1.2.3", "1.2"));
        assert!(version_prefix_matches("1.2.3", "1.2.3"));
        assert!(version_prefix_matches("1.2.3", "*"));
        assert!(!version_prefix_matches("1.2.3", "2"));
        assert!(!version_prefix_matches("1.2.3", "1.3"));
    }

    #[test]
    fn cmp_version_orders_numerically() {
        use std::cmp::Ordering;
        assert_eq!(cmp_version("1.2.10", "1.2.9"), Ordering::Greater);
        assert_eq!(cmp_version("2.0.0", "1.9.9"), Ordering::Greater);
        assert_eq!(cmp_version("1.0.0", "1.0.0"), Ordering::Equal);
    }
}
