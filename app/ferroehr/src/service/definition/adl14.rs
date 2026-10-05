// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `I_DEFINITION_ADL14` (`i_definition_adl14.adoc`; `master04` §Archetypes and
//! Templates): ADL 1.4 source archetypes keyed by `ARCHETYPE_ID` (on
//! `archetype_store`) and OPTs keyed by `UUID` (on `template_store`).
//!
//! ADL 1.4 archetype validity is the `openehr-adl` engine judged as 1.4: an
//! upload parses via [`openehr_adl::assemble::parse_artefact`] in
//! [`openehr_adl::parse::Dialect::Adl14`] (the 1.4-shaped `openehr_am::v2_4`
//! model) and validates against the subset of the phase-1 catalogue
//! corresponding to the ADL 1.4 / AOM 1.4 standalone validity rules, plus VUNT,
//! the one 1.4 rule stated "according to the reference model"
//! ([`openehr_adl::validate::validate_adl14_source`]; ADL1.4 `master08` §Validity
//! Rules, `master05-cadl.adoc` §Internal References, AOM1.4 class invariants).
//! A 1.4 source is never validated post-conversion, which would change what is
//! being judged.
//!
//! An in-CDR 1.4 to ADL 2 migration is offered as a service capability
//! ([`FerroEhrService::adl14_convert_to_adl2`]), converting the stored 1.4 source
//! text through the `openehr_adl::adl14` converter.
//!
//! NOTE: no openEHR spec governs 1.4 to 2 conversion and the vendored ITS-REST
//! OAS declares no conversion operation — our own design/extension, service-level
//! only and never exposed on the wire.

use std::str::FromStr;

use openehr_adl::adl14::convert::{ConvertConfig, parse_and_convert};
use openehr_adl::adl14::log::ConversionLog;
use openehr_adl::error::SyntaxError;
use openehr_adl::validate::catalogue::Severity;
use openehr_adl::validate::rm::ProductionRmModel;
use openehr_adl::validate::{ValidationIssue, validate_adl14_source};
use openehr_base::prelude::ArchetypeId;
use openehr_base::validate::InvariantViolation;
use uuid::Uuid;

use crate::service::FerroEhrService;
use crate::service::error::{ServiceError, Violation};
use crate::service::list::Page;
use crate::service::status::{CallStatusType, SmError};

use super::{compile_pattern, page_bounds, paginate};

// ── SM Definitions native API (I_DEFINITION_ADL14) — the catalog contract ────

impl FerroEhrService {
    /// `has_archetype` — true if an ADL 1.4 archetype with id `an_id` is
    /// stored. Identity is compared case-insensitively (BASE master05
    /// §Composite Identifiers and Case).
    ///
    /// # Errors
    ///
    /// A database failure (`exception` → `500`).
    pub async fn has_archetype(&self, an_id: String) -> Result<bool, SmError> {
        Ok(self.archetype_exists(&an_id).await?)
    }

    /// `valid_archetype` — the ADL 1.4 phase-1 validity of a 1.4 source
    /// (`openehr-adl` engine, standalone: parse in the 1.4 dialect + the ADL 1.4
    /// / AOM 1.4 phase-1 subset). Stateless — validity is judged as 1.4, never
    /// post-conversion.
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
    pub fn valid_archetype(&self, adl: &str) -> Result<bool, SmError> {
        Ok(matches!(
            validate_adl14_source(adl, &ProductionRmModel),
            Ok(issues) if issues.iter().all(|i| i.severity != Severity::Error)
        ))
    }

    /// `upload_archetype` (`Post_has_archetype`) — validate a 1.4 archetype
    /// through the `openehr-adl` engine (judged as 1.4) and store it, replacing
    /// any existing one with the same id ("If an archetype with the same id
    /// already exists, replace it. The archetype must be valid to succeed." —
    /// `i_definition_adl14.adoc`).
    ///
    /// Identity is case-insensitive but storage case-preserving (BASE master05
    /// §Composite Identifiers and Case): the write removes any case-variant of
    /// the id in the same transaction, then inserts the source verbatim — so a
    /// re-upload with different case *replaces* rather than duplicates.
    ///
    /// Returns the stored `ARCHETYPE_ID`, the identity read out of the submitted
    /// source.
    ///
    /// NOTE: `i_definition_adl14.adoc` types the operation's return as void and
    /// no openEHR spec governs what a transport does with the stored identity —
    /// our own design/extension, so a caller can address the artefact without
    /// re-parsing what it sent.
    ///
    /// # Errors
    ///
    /// - Source that fails to parse (S-codes) or fails the ADL 1.4 phase-1
    ///   catalogue (V-codes) → `invalid_archetype` (`422`), the offending
    ///   rule-code mnemonics carried as the validation detail.
    /// - A database failure (`exception` → `500`).
    pub async fn upload_archetype(&self, adl: String) -> Result<String, SmError> {
        // NOTE: i_definition_adl14.adoc §upload_archetype .Errors declares
        // invalid_archetype for a semantically invalid archetype.
        archetype_validate(&adl)
            .map_err(|e| invalid_artefact_status(e, CallStatusType::InvalidArchetype))?;
        Ok(self.archetype_upload(&adl).await?)
    }

    /// Converts a stored ADL 1.4 archetype (by `ARCHETYPE_ID`) to ADL 2 source
    /// text via the `openehr_adl::adl14` converter.
    ///
    /// The stored artefact is 1.4 source text on `archetype_store`, which the
    /// converter consumes directly. A specialised source is base-converted,
    /// renumbered against its own codes; re-differentialisation against a
    /// converted and flattened parent is the converter's `differ`. Stored 1.4
    /// operational templates go through the sibling
    /// [`FerroEhrService::adl14_convert_opt_to_adl2`] instead.
    ///
    /// NOTE: no openEHR spec governs 1.4 to 2 conversion — our own
    /// design/extension, a service capability with no REST endpoint.
    ///
    /// # Errors
    ///
    /// - No archetype with that id → `artefact_does_not_exist` (`404`).
    /// - The stored source no longer converts (parse / unsupported kind), or
    ///   the conversion result carries a node with no ADL2 syntax →
    ///   `content_invalid` (`422`).
    /// - A database failure (`exception` → `500`).
    pub async fn adl14_convert_to_adl2(&self, an_id: String) -> Result<String, SmError> {
        let source = self.archetype_get(&an_id).await?;
        let mut log = ConversionLog::new();
        let converted =
            parse_and_convert(&source, &ConvertConfig::default(), &mut log).map_err(|e| {
                ServiceError::content_invalid(
                    Violation::new(format!("1.4 → 2 conversion failed: {e}")).with_source(e),
                )
            })?;
        openehr_adl::print::print(&converted).map_err(|e| {
            SmError::from(ServiceError::content_invalid(
                Violation::new(format!("1.4 → 2 conversion produced unprintable ADL2: {e}"))
                    .with_source(e),
            ))
        })
    }

    /// Convert a stored ADL 1.4 **operational template** (by `UUID`) to ADL2
    /// source text via the `openehr_adl::adl14` converter, returning one ADL2
    /// source per embedded archetype root (the top COMPOSITION root first, then
    /// each component in document order).
    ///
    /// A 1.4 OPT is specialisation-flattened: its `definition` is one
    /// `C_ARCHETYPE_ROOT` tree with the component archetypes embedded as nested
    /// `C_ARCHETYPE_ROOT` nodes, each with its own at-code space. The
    /// `super::opt14_convert` front end decomposes it into one scoped 1.4-shaped
    /// `v2_4` source per embedded root and converts each; the recovered
    /// composition structure stays on the front end's result.
    ///
    /// NOTE: no openEHR spec governs 1.4 to 2 conversion — our own
    /// design/extension, a service capability with no REST endpoint.
    ///
    /// # Errors
    ///
    /// - `an_opt_id` is not a parseable `UUID` → `precondition_violation`
    ///   (`400`).
    /// - No OPT with that id → `template_does_not_exist` (`404`).
    /// - The stored OPT no longer parses or does not convert → `content_invalid`
    ///   (`422`).
    /// - A database failure (`exception` → `500`).
    pub async fn adl14_convert_opt_to_adl2(
        &self,
        an_opt_id: String,
    ) -> Result<Vec<String>, SmError> {
        // `opt_get` names `template_does_not_exist` at construction and
        // `ServiceError` round-trips the granular status losslessly — no
        // boundary re-raise needed.
        let xml = self.opt_get(&an_opt_id).await?;
        let opt = openehr_its::opt14::from_xml(&xml).map_err(|e| {
            ServiceError::content_invalid(
                Violation::new(format!("stored OPT no longer parses: {e:?}")).with_source(e),
            )
        })?;
        let conversion = super::opt14_convert::convert_opt_to_adl2(&opt).map_err(|e| {
            ServiceError::content_invalid(
                Violation::new(format!("OPT 1.4 → 2 conversion failed: {e}")).with_source(e),
            )
        })?;
        Ok(conversion.roots.into_iter().map(|r| r.adl2).collect())
    }

    /// `get_archetype` — the ADL 1.4 source of the archetype with id `an_id`
    /// (interchange form). Identity is case-insensitive.
    ///
    /// # Errors
    ///
    /// - No archetype with that id → `artefact_does_not_exist` (`404`).
    /// - A database failure (`exception` → `500`).
    pub async fn get_archetype(&self, an_id: String) -> Result<String, SmError> {
        Ok(self.archetype_get(&an_id).await?)
    }

    /// `list_archetypes` — the ids of all stored ADL 1.4 archetypes, cursored
    /// by `page` (`master02-overview.adoc` §List Handling).
    ///
    /// # Errors
    ///
    /// A database failure (`exception` → `500`).
    pub async fn list_archetypes_adl14(&self, page: Page) -> Result<Vec<String>, SmError> {
        Ok(self.archetype_list(page).await?)
    }

    /// `list_matching_archetypes` — archetype ids matching `id_pattern` (a
    /// regex), cursored by `page`.
    ///
    /// # Errors
    ///
    /// - An uncompilable `id_pattern` → `invalid_id_pattern` (`400`).
    /// - A database failure (`exception` → `500`).
    pub async fn list_matching_archetypes(
        &self,
        id_pattern: String,
        page: Page,
    ) -> Result<Vec<String>, SmError> {
        Ok(self.archetype_list_matching(&id_pattern, page).await?)
    }

    /// `delete_archetype` (`Pre_artefact_exists` / `Post_archetype_removed`) —
    /// delete an archetype by id (case-insensitive).
    ///
    /// # Errors
    ///
    /// - No archetype with that id → `artefact_does_not_exist` (`404`).
    /// - A database failure (`exception` → `500`).
    pub async fn delete_archetype(&self, an_id: String) -> Result<(), SmError> {
        Ok(self.archetype_delete(&an_id).await?)
    }

    /// `archetypes_count` — total stored ADL 1.4 archetypes.
    ///
    /// # Errors
    ///
    /// A database failure (`exception` → `500`).
    pub async fn archetypes_count_adl14(&self) -> Result<i64, SmError> {
        Ok(self.archetype_count().await?)
    }

    /// `has_opt` — true if an OPT with `an_opt_id` (a `UUID`) is stored.
    ///
    /// # Errors
    ///
    /// - `an_opt_id` is not a parseable `UUID` → `precondition_violation`
    ///   (`400`).
    /// - A database failure (`exception` → `500`).
    pub async fn has_opt(&self, an_opt_id: String) -> Result<bool, SmError> {
        Ok(self.opt_exists(&an_opt_id).await?)
    }

    /// `valid_opt` — the OPT parses (`opt14::from_xml`) and passes the
    /// templates seam's structural check. Stateless.
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
    pub fn valid_opt(&self, opt_xml: &str) -> Result<bool, SmError> {
        Ok(valid_opt_xml(opt_xml))
    }

    /// `upload_opt` — store an OPT 1.4 canonical-XML template. Ingestion runs
    /// in the templates layer: `store_template` parses + structurally
    /// validates the OPT and stores it create-only.
    ///
    /// # Errors
    ///
    /// - Not well-formed XML → bad request (`400` — the released
    ///   "syntactically invalid … content" branch,
    ///   ITS-REST `responses/400.yaml`).
    /// - Well-formed but undecodable / structurally invalid OPT XML →
    ///   `invalid_template` (`422`).
    /// - A template with the same `template_id` already stored → conflict
    ///   (`409`).
    /// - A database failure (`exception` → `500`).
    pub async fn upload_opt(&self, opt_xml: String) -> Result<(), SmError> {
        // NOTE: i_definition_adl14.adoc §upload_opt .Errors declares
        // invalid_template for a semantically invalid operational template.
        self.store_template(&opt_xml)
            .await
            .map_err(|e| invalid_artefact_status(e, CallStatusType::InvalidTemplate))?;
        Ok(())
    }

    /// `get_opt` — the OPT 1.4 canonical XML of the OPT with `an_opt_id` (a
    /// `UUID`; interchange form).
    ///
    /// # Errors
    ///
    /// - `an_opt_id` is not a parseable `UUID` → `precondition_violation`
    ///   (`400`).
    /// - No OPT with that id → `template_does_not_exist` (`404`).
    /// - A database failure (`exception` → `500`).
    pub async fn get_opt(&self, an_opt_id: String) -> Result<String, SmError> {
        Ok(self.opt_get(&an_opt_id).await?)
    }

    /// `list_opts` — the ids (`UUID`s) of all stored OPTs, oldest first,
    /// cursored by `page`.
    ///
    /// # Errors
    ///
    /// A database failure (`exception` → `500`).
    pub async fn list_opts_adl14(&self, page: Page) -> Result<Vec<String>, SmError> {
        Ok(self.opt_list(page).await?)
    }

    /// `list_matching_opts` — OPTs whose `template_id` matches `id_pattern`
    /// (a regex), cursored by `page`.
    ///
    /// NOTE (spec defect): the SM types this `List<ARCHETYPE_ID>`
    /// though OPTs are UUID-keyed; we return the OPTs' `template_id` strings
    /// (the meaningful identifier a pattern is useful against).
    ///
    /// # Errors
    ///
    /// - An uncompilable `id_pattern` → `invalid_id_pattern` (`400`).
    /// - A database failure (`exception` → `500`).
    pub async fn list_matching_opts(
        &self,
        id_pattern: String,
        page: Page,
    ) -> Result<Vec<String>, SmError> {
        Ok(self.opt_list_matching(&id_pattern, page).await?)
    }

    /// `delete_opt` (`Pre_has_opt` / `Post_opt_removed`) — delete an OPT by
    /// `an_opt_id` (a `UUID`), evicting its derived-runtime (`WebTemplate`)
    /// cache entry.
    ///
    /// NOTE: `i_definition_adl14.adoc` §`delete_opt` is silent on committed data
    /// referencing the template, so the in-use refusal is our own integrity
    /// design, matching the admin wire delete
    /// ([`Self::admin_template_delete`]).
    ///
    /// # Errors
    ///
    /// - `an_opt_id` is not a parseable `UUID` → `precondition_violation`
    ///   (`400`).
    /// - No OPT with that id → `template_does_not_exist` (`404`).
    /// - A committed version still references the template →
    ///   `ServiceError::Conflict` (`409`), naming the reference count.
    /// - A database failure (`exception` → `500`).
    pub async fn delete_opt(&self, an_opt_id: String) -> Result<(), SmError> {
        Ok(self.opt_delete(&an_opt_id).await?)
    }

    /// `opts_count` — total stored OPTs.
    ///
    /// # Errors
    ///
    /// A database failure (`exception` → `500`).
    pub async fn opts_count_adl14(&self) -> Result<i64, SmError> {
        Ok(self.opt_count().await?)
    }
}

// ── domain logic (the ServiceError layer under the catalog) ──────────────────

impl FerroEhrService {
    /// True if an ADL 1.4 archetype with id `an_id` is stored
    /// (case-insensitive identity).
    async fn archetype_exists(&self, an_id: &str) -> Result<bool, ServiceError> {
        Ok(sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM archetype_store WHERE lower(archetype_id) = lower($1))",
        )
        .bind(an_id)
        .fetch_one(&self.pool)
        .await?)
    }

    /// Store a valid ADL 1.4 archetype, replacing any case-variant of the same
    /// id in the same transaction; invalid source → `invalid_archetype` (`422`).
    /// Returns the stored `ARCHETYPE_ID`.
    async fn archetype_upload(&self, adl: &str) -> Result<String, ServiceError> {
        let id = extract_archetype_id(adl).ok_or_else(|| {
            ServiceError::sm(
                CallStatusType::InvalidArchetype,
                "ADL 1.4 source is not a valid archetype (missing `archetype` \
                 header or a well-formed ARCHETYPE_ID)",
            )
        })?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM archetype_store WHERE lower(archetype_id) = lower($1)")
            .bind(&id.value)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO archetype_store (archetype_id, adl) VALUES ($1, $2)")
            .bind(&id.value)
            .bind(adl)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(id.value)
    }

    /// The ADL 1.4 source of the archetype with id `an_id`; absent →
    /// `artefact_does_not_exist` (`404`).
    async fn archetype_get(&self, an_id: &str) -> Result<String, ServiceError> {
        sqlx::query_scalar::<_, String>(
            "SELECT adl FROM archetype_store WHERE lower(archetype_id) = lower($1)",
        )
        .bind(an_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            ServiceError::sm(
                CallStatusType::ArtefactDoesNotExist,
                format!("archetype {an_id}"),
            )
        })
    }

    /// The ids of all stored ADL 1.4 archetypes, paged in SQL.
    async fn archetype_list(&self, page: Page) -> Result<Vec<String>, ServiceError> {
        let (offset, limit) = page_bounds(page);
        Ok(sqlx::query_scalar::<_, String>(
            "SELECT archetype_id FROM archetype_store ORDER BY archetype_id OFFSET $1 LIMIT $2",
        )
        .bind(offset)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Archetype ids matching `id_pattern` (regex; uncompilable →
    /// `invalid_id_pattern`, `400`), then paged.
    async fn archetype_list_matching(
        &self,
        id_pattern: &str,
        page: Page,
    ) -> Result<Vec<String>, ServiceError> {
        let re = compile_pattern(id_pattern)?;
        let all: Vec<String> =
            sqlx::query_scalar("SELECT archetype_id FROM archetype_store ORDER BY archetype_id")
                .fetch_all(&self.pool)
                .await?;
        Ok(paginate(all.into_iter().filter(|id| re.is_match(id)), page))
    }

    /// Delete an archetype by id (case-insensitive); absent →
    /// `artefact_does_not_exist` (`404`).
    async fn archetype_delete(&self, an_id: &str) -> Result<(), ServiceError> {
        let deleted =
            sqlx::query("DELETE FROM archetype_store WHERE lower(archetype_id) = lower($1)")
                .bind(an_id)
                .execute(&self.pool)
                .await?
                .rows_affected();
        if deleted == 0 {
            return Err(ServiceError::sm(
                CallStatusType::ArtefactDoesNotExist,
                format!("archetype {an_id}"),
            ));
        }
        Ok(())
    }

    /// Total stored ADL 1.4 archetypes.
    async fn archetype_count(&self) -> Result<i64, ServiceError> {
        Ok(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM archetype_store")
                .fetch_one(&self.pool)
                .await?,
        )
    }

    /// True if an OPT with `an_opt_id` (a `UUID`) is stored; unparseable UUID
    /// → `400`.
    async fn opt_exists(&self, an_opt_id: &str) -> Result<bool, ServiceError> {
        let id = parse_opt_uuid(an_opt_id)?;
        Ok(sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM template_store WHERE id = $1)",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?)
    }

    /// The OPT 1.4 canonical XML of the OPT with `an_opt_id` (a `UUID`);
    /// absent → `template_does_not_exist` (`404`); unparseable UUID
    /// → `400`.
    async fn opt_get(&self, an_opt_id: &str) -> Result<String, ServiceError> {
        let id = parse_opt_uuid(an_opt_id)?;
        sqlx::query_scalar::<_, String>("SELECT content FROM template_store WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| {
                ServiceError::sm(
                    CallStatusType::TemplateDoesNotExist,
                    format!("OPT {an_opt_id}"),
                )
            })
    }

    /// The OPT 1.4 canonical XML addressed by its `template_id` string (the
    /// ITS-REST wire address, unlike the SM's UUID-keyed [`opt_get`](Self::opt_get)).
    /// Spec-silent: `template_id` addressing is our own wire helper, the SM
    /// keys OPTs by `UUID`. Absent → `template_does_not_exist` (`404`).
    pub(super) async fn opt_get_by_template_id(
        &self,
        template_id: &str,
    ) -> Result<String, ServiceError> {
        // Identity of the TEMPLATE_ID is case-insensitive (BASE master05
        // §Composite Identifiers and Case).
        sqlx::query_scalar::<_, String>(
            "SELECT content FROM template_store WHERE lower(template_id) = lower($1)",
        )
        .bind(template_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            ServiceError::sm(
                CallStatusType::TemplateDoesNotExist,
                format!("OPT template_id {template_id}"),
            )
        })
    }

    /// The ids (`UUID`s) of all stored OPTs, oldest first, paged in SQL.
    async fn opt_list(&self, page: Page) -> Result<Vec<String>, ServiceError> {
        let (offset, limit) = page_bounds(page);
        let ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM template_store ORDER BY created_at, id OFFSET $1 LIMIT $2",
        )
        .bind(offset)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(ids.into_iter().map(|u| u.to_string()).collect())
    }

    /// OPTs whose `template_id` matches `id_pattern` (regex; uncompilable →
    /// `invalid_id_pattern`, `400`), then paged. Returns `template_id`
    /// strings.
    async fn opt_list_matching(
        &self,
        id_pattern: &str,
        page: Page,
    ) -> Result<Vec<String>, ServiceError> {
        let re = compile_pattern(id_pattern)?;
        let all: Vec<String> =
            sqlx::query_scalar("SELECT template_id FROM template_store ORDER BY template_id")
                .fetch_all(&self.pool)
                .await?;
        Ok(paginate(
            all.into_iter().filter(|tid| re.is_match(tid)),
            page,
        ))
    }

    /// Delete an OPT by `an_opt_id` (a `UUID`), evicting the deleted
    /// template's `WebTemplate` cache entry; absent →
    /// `template_does_not_exist` (`404`); still referenced by committed
    /// versions → `Conflict` (`409`, with the reference count); unparseable
    /// UUID → `400`.
    async fn opt_delete(&self, an_opt_id: &str) -> Result<(), ServiceError> {
        let id = parse_opt_uuid(an_opt_id)?;
        // Resolve, count references and delete in ONE transaction so the 409 is
        // consistent with the delete; the `version.template_id` →
        // `template_ref` foreign key stays the integrity guard under a
        // concurrent commit.
        let mut tx = self.pool.begin().await?;
        let template_id: Option<String> =
            sqlx::query_scalar("SELECT template_id FROM template_store WHERE id = $1")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
        let Some(template_id) = template_id else {
            return Err(ServiceError::sm(
                CallStatusType::TemplateDoesNotExist,
                format!("OPT {an_opt_id}"),
            ));
        };
        // Counted over BOTH storage tiers: the cold archival mirror carries no
        // `template_ref` foreign key, so an archived composition's reference is
        // invisible to the constraint and deleting under it would make that
        // object unrestorable (no openEHR spec governs the in-use refusal).
        let refs: i64 = sqlx::query_scalar("SELECT count(*) FROM version WHERE template_id = $1")
            .bind(&template_id)
            .fetch_one(&mut *tx)
            .await?;
        if refs > 0 {
            return Err(ServiceError::conflict(format!(
                "template '{template_id}' is still referenced by {refs} committed version(s); \
                 delete those compositions before deleting the template"
            )));
        }
        sqlx::query("DELETE FROM template_store WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        // Deregister the wire address unless a template-kind ADL2 artefact
        // also claims it (`template_ref` is the union of both dialects'
        // addresses; the FK blocks the deregistration if a concurrent commit
        // referenced it after the count above).
        sqlx::query(
            "DELETE FROM template_ref WHERE template_id = $1 AND NOT EXISTS \
             (SELECT 1 FROM adl2_artefact WHERE lower(hrid) = lower($1) \
              AND kind IN ('template', 'operational_template'))",
        )
        .bind(&template_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        // Uploads are create-only, so delete is the single cache-invalidation
        // point. The key is the identity-canonical form, so a case variant is
        // evicted too (BASE master05 §Composite Identifiers and Case).
        self.web_templates
            .invalidate(&crate::templates::identity::canonical_key(&template_id))
            .await;
        Ok(())
    }

    /// Total stored OPTs.
    async fn opt_count(&self) -> Result<i64, ServiceError> {
        Ok(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM template_store")
                .fetch_one(&self.pool)
                .await?,
        )
    }
}

// ── stateless validity helpers ────────────────────────────────────────────────

/// Validate an ADL 1.4 source through the `openehr-adl` engine, judged **as
/// 1.4** (`openehr_adl::validate::validate_adl14_source`). An unparseable
/// source (S-codes) or a catalogue failure (V-codes) is a
/// [`ServiceError::ValidationFailed`] carrying the rule-code mnemonics — the SM
/// `upload_archetype` maps it to `invalid_archetype` / `content_invalid`
/// (`422`, `i_definition_adl14.adoc`; ADL1.4 `master08` §Validity Rules).
///
/// The reference model is the generated openEHR RM 1.2.0
/// ([`ProductionRmModel`]), so the one ADL 1.4 validity rule that is stated
/// "according to the reference model" — VUNT, `use_node` type conformance
/// (`ADL1.4/master05-cadl.adoc` §Internal References L512-513) — is reached by
/// an upload instead of being unreachable behind a phase-1-only entry point.
fn archetype_validate(adl: &str) -> Result<(), ServiceError> {
    let issues =
        validate_adl14_source(adl, &ProductionRmModel).map_err(archetype_syntax_failure)?;
    let errors: Vec<InvariantViolation> = issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .map(issue_to_validation_error)
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(ServiceError::ValidationFailed(errors))
    }
}

/// Maps an ADL 1.4 archetype parse failure to a
/// [`ServiceError::ValidationFailed`] carrying the S-code mnemonics.
///
/// The archetype-provisioning operations are SM-only
/// (`i_definition_adl14.adoc` `upload_archetype` `invalid_archetype`; ITS-REST
/// 1.1.0 surfaces no archetype route), so no released wire status attaches and
/// the SM error is the whole contract.
/// Promote the generic `content_invalid` of a [`ServiceError`] crossing an
/// upload seam to the operation's declared artefact status.
///
/// `i_definition_adl14.adoc` declares `invalid_template` (§`upload_opt`
/// `.Errors`) and `invalid_archetype` (§`upload_archetype` `.Errors`) for a
/// semantically invalid artefact; every other status passes through
/// unchanged, so a duplicate-artefact conflict or a storage fault keeps its
/// own token.
fn invalid_artefact_status(e: ServiceError, status: CallStatusType) -> SmError {
    let mut sm = SmError::from(e);
    if sm.status == CallStatusType::ContentInvalid {
        sm.status = status;
    }
    sm
}

fn archetype_syntax_failure(errs: Vec<SyntaxError>) -> ServiceError {
    ServiceError::ValidationFailed(
        errs.into_iter()
            .map(|e| {
                InvariantViolation::at(
                    e.code.mnemonic(),
                    format!("{} (line {}, column {})", e.message, e.line, e.column),
                )
            })
            .collect(),
    )
}

/// Render one [`ValidationIssue`] as an [`InvariantViolation`]: the rule-code
/// mnemonic is the machine-readable key, the human detail (plus the archetype
/// path where derivable) is the message.
fn issue_to_validation_error(i: &ValidationIssue) -> InvariantViolation {
    InvariantViolation::at(
        i.code.mnemonic(),
        match &i.path {
            Some(p) => format!("{} (at {p})", i.message),
            None => i.message.clone(),
        },
    )
}

/// `valid_opt` core — the OPT parses (`opt14::from_xml`), passes the
/// templates seam's structural check, and passes the same artefact-validity
/// catalogue `upload_opt` enforces, so the two answers never diverge.
fn valid_opt_xml(opt_xml: &str) -> bool {
    crate::validation::validate_opt_structure(opt_xml).is_ok()
        && openehr_its::opt14::from_xml(opt_xml)
            .is_ok_and(|opt| crate::validation::validate_opt_artefact(&opt).is_ok())
}

/// Parse an OPT id UUID string; an unparseable value is a `400`.
#[expect(
    clippy::map_err_ignore,
    reason = "the mapped error already names the resource and echoes the \
              rejected token; the discarded `uuid::Error` adds only its own \
              wording, which is not part of the wire contract"
)]
fn parse_opt_uuid(an_opt_id: &str) -> Result<Uuid, ServiceError> {
    Uuid::parse_str(an_opt_id)
        .map_err(|_| ServiceError::precondition(format!("OPT id is not a UUID: {an_opt_id}")))
}

/// Extract the `ARCHETYPE_ID` from ADL 1.4 source: the source must begin with
/// the `archetype` keyword line (optionally `archetype (adl_version=…)`) and
/// carry a well-formed `ARCHETYPE_ID` (BASE `master05` §Archetype Identifiers)
/// on the next non-blank line.
fn extract_archetype_id(adl: &str) -> Option<ArchetypeId> {
    // Tolerate a leading UTF-8 BOM (present in the vendored .adl fixtures).
    let adl = adl.trim_start_matches('\u{feff}');
    let mut lines = adl.lines().map(str::trim).filter(|l| !l.is_empty());
    let header = lines.next()?;
    let is_header = header == "archetype"
        || header.starts_with("archetype ")
        || header.starts_with("archetype(");
    if !is_header {
        return None;
    }
    let id_line = lines.next()?;
    ArchetypeId::from_str(id_line).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archetype_id_extraction() {
        let adl = "\u{feff}archetype (adl_version=1.4)\n\
                   \topenEHR-EHR-COMPOSITION.prescription.v1\n\n\
                   concept\n\t[at0000]";
        assert_eq!(
            extract_archetype_id(adl).map(|a| a.value),
            Some("openEHR-EHR-COMPOSITION.prescription.v1".to_owned())
        );
        // No `archetype` header → not valid.
        assert!(extract_archetype_id("concept\n[at0000]").is_none());
        // Header but the id line is not a well-formed ARCHETYPE_ID.
        assert!(extract_archetype_id("archetype\n  not-an-archetype-id").is_none());
    }
}
