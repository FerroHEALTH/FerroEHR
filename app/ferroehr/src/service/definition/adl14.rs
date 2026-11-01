// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADL 1.4 definitions: ADL 1.4 source archetypes keyed by `ARCHETYPE_ID` (on
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
use openehr_adl::adl14::opt::convert_opt_to_adl2;
use openehr_adl::error::SyntaxError;
use openehr_adl::validate::catalogue::Severity;
use openehr_adl::validate::rm::ProductionRmModel;
use openehr_adl::validate::{ValidationIssue, validate_adl14_source};
use openehr_base::prelude::ArchetypeId;
use openehr_base::validate::InvariantViolation;
use sqlx::Connection as _;

use crate::db::acquire::acquire;
use crate::service::FerroEhrService;
use crate::service::error::{ServiceError, Violation};
use crate::service::list::Page;
use crate::service::status::{ServiceStatus, StatusError};

use super::page_bounds;

// ── the native ADL 1.4 definition calls ─────────────────────────────────────

impl FerroEhrService {
    /// `has_archetype` — true if an ADL 1.4 archetype with id `an_id` is
    /// stored. Identity is compared case-insensitively (BASE master05
    /// §Composite Identifiers and Case).
    ///
    /// # Errors
    ///
    /// A database failure (`exception` → `500`).
    pub async fn has_archetype(&self, an_id: String) -> Result<bool, StatusError> {
        Ok(self.archetype_exists(&an_id).await?)
    }

    /// `upload_archetype` — validate a 1.4 archetype
    /// through the `openehr-adl` engine (judged as 1.4) and store it, replacing
    /// any existing one with the same id; only a valid archetype is stored.
    ///
    /// Identity is case-insensitive but storage case-preserving (BASE master05
    /// §Composite Identifiers and Case): the write removes any case-variant of
    /// the id in the same transaction, then inserts the source verbatim — so a
    /// re-upload with different case *replaces* rather than duplicates.
    ///
    /// Returns the stored `ARCHETYPE_ID`, the identity read out of the submitted
    /// source.
    ///
    /// NOTE: no openEHR spec governs what a transport does with the stored identity —
    /// our own design/extension, so a caller can address the artefact without
    /// re-parsing what it sent.
    ///
    /// # Errors
    ///
    /// - Source that fails to parse (S-codes) or fails the ADL 1.4 phase-1
    ///   catalogue (V-codes) → `invalid_archetype` (`422`), the offending
    ///   rule-code mnemonics carried as the validation detail.
    /// - A database failure (`exception` → `500`).
    pub async fn upload_archetype(&self, adl: String) -> Result<String, StatusError> {
        // NOTE: no specification names this status token: our own design for a
        // semantically invalid archetype.
        archetype_validate(&adl)
            .map_err(|e| invalid_artefact_status(e, ServiceStatus::InvalidArchetype))?;
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
    pub async fn adl14_convert_to_adl2(&self, an_id: String) -> Result<String, StatusError> {
        let source = self.archetype_get(&an_id).await?;
        let mut log = ConversionLog::default();
        let converted =
            parse_and_convert(&source, &ConvertConfig::default(), &mut log).map_err(|e| {
                ServiceError::content_invalid(
                    Violation::new(format!("1.4 → 2 conversion failed: {e}")).with_source(e),
                )
            })?;
        openehr_adl::print::print(&converted).map_err(|e| {
            StatusError::from(ServiceError::content_invalid(
                Violation::new(format!("1.4 → 2 conversion produced unprintable ADL2: {e}"))
                    .with_source(e),
            ))
        })
    }

    /// Convert a stored ADL 1.4 **operational template** (by its `template_id`)
    /// to ADL2 source text via the `openehr_adl::adl14` converter, returning one
    /// ADL2 source per embedded archetype root (the top COMPOSITION root first,
    /// then each component in document order).
    ///
    /// A 1.4 OPT is specialisation-flattened: its `definition` is one
    /// `C_ARCHETYPE_ROOT` tree with the component archetypes embedded as nested
    /// `C_ARCHETYPE_ROOT` nodes, each with its own at-code space.
    /// [`openehr_adl::adl14::opt::convert_opt_to_adl2`] decomposes it into one
    /// scoped 1.4-shaped `v2_4` source per embedded root and converts each.
    /// The `template_id` is matched case-insensitively (BASE master05
    /// §Composite Identifiers and Case).
    ///
    /// NOTE: no openEHR spec governs 1.4 to 2 conversion — our own
    /// design/extension, a service capability with no REST endpoint.
    ///
    /// # Errors
    ///
    /// - No OPT with that `template_id` → `template_does_not_exist` (`404`).
    /// - The stored OPT no longer parses or does not convert → `content_invalid`
    ///   (`422`).
    /// - A database failure (`exception` → `500`).
    pub async fn adl14_convert_opt_to_adl2(
        &self,
        template_id: String,
    ) -> Result<Vec<String>, StatusError> {
        let xml = self.opt_get_by_template_id(&template_id).await?;
        let opt = openehr_its::opt14::from_xml(&xml).map_err(|e| {
            ServiceError::content_invalid(
                Violation::new(format!("stored OPT no longer parses: {e:?}")).with_source(e),
            )
        })?;
        let conversion = convert_opt_to_adl2(&opt).map_err(|e| {
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
    pub async fn get_archetype(&self, an_id: String) -> Result<String, StatusError> {
        Ok(self.archetype_get(&an_id).await?)
    }

    /// `list_archetypes` — the ids of all stored ADL 1.4 archetypes, cursored
    /// by `page`.
    ///
    /// # Errors
    ///
    /// A database failure (`exception` → `500`).
    pub async fn list_archetypes_adl14(&self, page: Page) -> Result<Vec<String>, StatusError> {
        Ok(self.archetype_list(page).await?)
    }

    /// `delete_archetype` —
    /// delete an archetype by id (case-insensitive).
    ///
    /// # Errors
    ///
    /// - No archetype with that id → `artefact_does_not_exist` (`404`).
    /// - A database failure (`exception` → `500`).
    pub async fn delete_archetype(&self, an_id: String) -> Result<(), StatusError> {
        Ok(self.archetype_delete(&an_id).await?)
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
    pub async fn upload_opt(&self, opt_xml: String) -> Result<(), StatusError> {
        // NOTE: no specification names this status token: our own design for a
        // semantically invalid operational template.
        self.store_template(&opt_xml)
            .await
            .map_err(|e| invalid_artefact_status(e, ServiceStatus::InvalidTemplate))?;
        Ok(())
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
        .fetch_one(&mut *acquire(&self.pool).await?)
        .await?)
    }

    /// Store a valid ADL 1.4 archetype, replacing any case-variant of the same
    /// id in the same transaction; invalid source → `invalid_archetype` (`422`).
    /// Returns the stored `ARCHETYPE_ID`.
    async fn archetype_upload(&self, adl: &str) -> Result<String, ServiceError> {
        let id = extract_archetype_id(adl).ok_or_else(|| {
            ServiceError::new(
                ServiceStatus::InvalidArchetype,
                "ADL 1.4 source is not a valid archetype (missing `archetype` \
                 header or a well-formed ARCHETYPE_ID)",
            )
        })?;
        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await?;
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
        .fetch_optional(&mut *acquire(&self.pool).await?)
        .await?
        .ok_or_else(|| {
            ServiceError::new(
                ServiceStatus::ArtefactDoesNotExist,
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
        .fetch_all(&mut *acquire(&self.pool).await?)
        .await?)
    }

    /// Delete an archetype by id (case-insensitive); absent →
    /// `artefact_does_not_exist` (`404`).
    async fn archetype_delete(&self, an_id: &str) -> Result<(), ServiceError> {
        let deleted =
            sqlx::query("DELETE FROM archetype_store WHERE lower(archetype_id) = lower($1)")
                .bind(an_id)
                .execute(&mut *acquire(&self.pool).await?)
                .await?
                .rows_affected();
        if deleted == 0 {
            return Err(ServiceError::new(
                ServiceStatus::ArtefactDoesNotExist,
                format!("archetype {an_id}"),
            ));
        }
        Ok(())
    }

    /// The OPT 1.4 canonical XML addressed by its `template_id` string, the
    /// ITS-REST wire address. Absent → `template_does_not_exist` (`404`).
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
        .fetch_optional(&mut *acquire(&self.pool).await?)
        .await?
        .ok_or_else(|| {
            ServiceError::new(
                ServiceStatus::TemplateDoesNotExist,
                format!("OPT template_id {template_id}"),
            )
        })
    }
}

// ── stateless validity helpers ────────────────────────────────────────────────

/// Validate an ADL 1.4 source through the `openehr-adl` engine, judged **as
/// 1.4** (`openehr_adl::validate::validate_adl14_source`). An unparseable
/// source (S-codes) or a catalogue failure (V-codes) is a
/// [`ServiceError::ValidationFailed`] carrying the rule-code mnemonics —
/// `upload_archetype` maps it to `invalid_archetype` / `content_invalid`
/// (`422`; ADL1.4 `master08` §Validity Rules).
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

/// Promote the generic `content_invalid` of a [`ServiceError`] crossing an
/// upload seam to the operation's artefact status.
///
/// An OPT upload reports `invalid_template` and an archetype upload
/// `invalid_archetype` for a semantically invalid artefact; every other status passes through
/// unchanged, so a duplicate-artefact conflict or a storage fault keeps its
/// own token.
fn invalid_artefact_status(e: ServiceError, status: ServiceStatus) -> StatusError {
    let mut err = StatusError::from(e);
    if err.status == ServiceStatus::ContentInvalid {
        err.status = status;
    }
    err
}

/// Maps an ADL 1.4 archetype parse failure to a
/// [`ServiceError::ValidationFailed`] carrying the S-code mnemonics.
///
/// ITS-REST 1.1.0 surfaces no archetype route, so no released wire status
/// attaches: the `invalid_archetype` status is our own.
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
