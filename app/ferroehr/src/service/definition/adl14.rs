// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADL 1.4 source archetypes keyed by `ARCHETYPE_ID`, and the conversion of
//! a stored archetype or operational template to ADL 2. The operational
//! templates themselves are `opt14`.
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
//! A stored 1.4 archetype or operational template converts to ADL 2
//! ([`FerroEhrService::adl14_convert_to_adl2`],
//! [`FerroEhrService::adl14_convert_opt_to_adl2`]) through the
//! `openehr_adl::adl14` converter, which two read-only extension routes serve.
//!
//! NOTE: no openEHR spec governs 1.4 to 2 conversion and the vendored ITS-REST
//! OAS declares no conversion operation, so the conversion is our own design.

use std::collections::BTreeMap;
use std::str::FromStr;

use openehr_adl::adl14::convert::{ConvertConfig, ConvertError, convert};
use openehr_adl::adl14::log::ConversionLog;
use openehr_adl::adl14::opt::{ConvertedRoot, OptConversion, convert_opt_to_adl2};
use openehr_adl::adl14::specialised_codes::check_introduced_codes;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::error::SyntaxError;
use openehr_adl::meta::summarize;
use openehr_adl::parse::Dialect;
use openehr_adl::validate::catalogue::Severity;
use openehr_adl::validate::rm::ProductionRmModel;
use openehr_adl::validate::{ValidationIssue, validate_adl14_source};
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetype;
use openehr_base::prelude::ArchetypeId;
use openehr_base::validate::InvariantViolation;

use crate::error::refusal::{Invalid, Missing};
use crate::error::{ServiceError, Violation};
use crate::service::FerroEhrService;
use crate::service::list::Page;
use crate::storage::archetype_repo;

use super::page_bounds;

// ── the native ADL 1.4 definition calls ─────────────────────────────────────

impl FerroEhrService {
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
    pub async fn upload_archetype(&self, adl: String) -> Result<String, ServiceError> {
        // NOTE: no specification names this status token: our own design for a
        // semantically invalid archetype.
        archetype_validate(&adl).map_err(invalid_archetype)?;
        let id = extract_archetype_id(&adl).ok_or_else(|| ServiceError::Unprocessable {
            kind: Invalid::Archetype,
            violation: Violation::new(
                "ADL 1.4 source is not a valid archetype (missing `archetype` \
                     header or a well-formed ARCHETYPE_ID)",
            ),
        })?;
        archetype_repo::replace_archetype(&self.pool, &id.value, &adl).await?;
        Ok(id.value)
    }

    /// Converts a stored ADL 1.4 archetype (by `ARCHETYPE_ID`) to ADL 2: the
    /// ADL 2 source, the conversion log and the conversion details, as the one
    /// root of an [`OptConversion`] with no fill structure.
    ///
    /// The stored artefact is 1.4 source text on `archetype_store`, parsed in
    /// the 1.4 dialect and handed to [`convert`]. A specialised source is first
    /// checked against its stored parent's terminology, then base-converted,
    /// renumbered against its own codes. Identity is
    /// case-insensitive (BASE master05 §Composite Identifiers and Case).
    ///
    /// NOTE: no openEHR spec governs 1.4 to 2 conversion: our own design, served
    /// as the extension route `GET /definition/archetype/adl1.4/{archetype_id}/adl2`.
    ///
    /// # Errors
    ///
    /// - No archetype with that id → `artefact_does_not_exist` (`404`).
    /// - The stored source no longer converts (parse / unsupported kind), a
    ///   specialised source introduces a term code without the dots its depth
    ///   requires, or the conversion result carries a node with no ADL2
    ///   syntax → `content_invalid` (`422`).
    /// - A database failure (`exception` → `500`).
    pub async fn adl14_convert_to_adl2(
        &self,
        an_id: String,
    ) -> Result<OptConversion, ServiceError> {
        let source = self.get_archetype(&an_id).await?;
        let archetype = parse_artefact(&source, Dialect::Adl14)
            .map_err(|errs| conversion_refused(ConvertError::Parse(errs)))?;
        self.check_codes_against_stored_parent(&archetype).await?;
        let mut log = ConversionLog::default();
        let converted =
            convert(&archetype, &ConvertConfig::default(), &mut log).map_err(conversion_refused)?;
        let adl2 = openehr_adl::print::print(&converted).map_err(|e| {
            ServiceError::content_invalid(
                Violation::new(format!("1.4 → 2 conversion produced unprintable ADL2: {e}"))
                    .with_source(e),
            )
        })?;
        let conversion_details = if let Archetype::AuthoredArchetype(authored) = &converted
            && let AuthoredArchetype::AuthoredArchetype(data) = authored.as_ref()
        {
            data.description
                .as_ref()
                .and_then(|d| d.conversion_details.clone())
                .unwrap_or_default()
        } else {
            BTreeMap::new()
        };
        Ok(OptConversion {
            roots: vec![ConvertedRoot {
                archetype_id: an_id,
                adl2,
                log,
                conversion_details,
            }],
            structure: Vec::new(),
        })
    }

    /// Refuses a specialised 1.4 archetype whose new term codes lack the
    /// specialisation dots its depth requires, judged against its stored
    /// parent ([`check_introduced_codes`], AOM 1.4 `master07-ontology_package.adoc`
    /// §Specialisation Depth).
    ///
    /// NOTE: a parent that is not stored leaves no terminology to tell a new code from an
    /// inherited one, so the check passes; no openEHR spec governs that, our own design.
    ///
    /// # Errors
    ///
    /// - The check refuses the child, or the stored parent no longer parses →
    ///   `content_invalid` (`422`).
    /// - A database failure (`500`).
    async fn check_codes_against_stored_parent(
        &self,
        archetype: &Archetype,
    ) -> Result<(), ServiceError> {
        let Some(parent_id) = summarize(archetype).parent_archetype_id else {
            return Ok(());
        };
        let Some(parent_source) = archetype_repo::archetype_adl(&self.pool, &parent_id).await?
        else {
            return Ok(());
        };
        let parent = parse_artefact(&parent_source, Dialect::Adl14)
            .map_err(|errs| conversion_refused(ConvertError::Parse(errs)))?;
        check_introduced_codes(archetype, &parent).map_err(conversion_refused)
    }

    /// Converts a stored ADL 1.4 **operational template** (by its `template_id`)
    /// to ADL 2: one ADL 2 source per embedded archetype root (the top root
    /// first, then each component in document order), each with its conversion
    /// log and details, and the slot fills that joined them.
    ///
    /// [`convert_opt_to_adl2`] decomposes the flattened OPT into one scoped
    /// source per embedded root and converts each. The `template_id` is matched
    /// case-insensitively (BASE master05 §Composite Identifiers and Case).
    ///
    /// NOTE: no openEHR spec governs 1.4 to 2 conversion: our own design, served
    /// as the extension route `GET /definition/template/adl1.4/{template_id}/adl2`.
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
    ) -> Result<OptConversion, ServiceError> {
        let xml = self.template_adl14_get(&template_id).await?;
        let opt = openehr_its::opt14::from_xml(&xml).map_err(|e| {
            ServiceError::content_invalid(
                Violation::new(format!("stored OPT no longer parses: {e:?}")).with_source(e),
            )
        })?;
        convert_opt_to_adl2(&opt).map_err(|e| {
            ServiceError::content_invalid(
                Violation::new(format!("OPT 1.4 → 2 conversion failed: {e}")).with_source(e),
            )
        })
    }
}

// ── the archetype store ──────────────────────────────────────────────────────

impl FerroEhrService {
    /// The ADL 1.4 source of the archetype with id `an_id` (interchange form).
    /// Identity is case-insensitive.
    ///
    /// # Errors
    ///
    /// - No archetype with that id → [`Missing::Artefact`] (`404`).
    /// - A database failure (`500`).
    pub async fn get_archetype(&self, an_id: &str) -> Result<String, ServiceError> {
        archetype_repo::archetype_adl(&self.pool, an_id)
            .await?
            .ok_or_else(|| ServiceError::not_found(Missing::Artefact, format!("archetype {an_id}")))
    }

    /// The ids of all stored ADL 1.4 archetypes, paged in SQL.
    ///
    /// # Errors
    ///
    /// A database failure (`500`).
    pub async fn list_archetypes_adl14(&self, page: Page) -> Result<Vec<String>, ServiceError> {
        let (offset, limit) = page_bounds(page);
        Ok(archetype_repo::archetype_ids(&self.pool, offset, limit).await?)
    }

    /// Delete an archetype by id (case-insensitive).
    ///
    /// # Errors
    ///
    /// - No archetype with that id → [`Missing::Artefact`] (`404`).
    /// - A database failure (`500`).
    pub async fn delete_archetype(&self, an_id: &str) -> Result<(), ServiceError> {
        if archetype_repo::delete_archetype(&self.pool, an_id).await? {
            Ok(())
        } else {
            Err(ServiceError::not_found(
                Missing::Artefact,
                format!("archetype {an_id}"),
            ))
        }
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

/// Report a refused archetype as [`Invalid::Archetype`] (`422`), its
/// violations joined into one detail; every other refusal passes through.
fn invalid_archetype(e: ServiceError) -> ServiceError {
    match e.flattened() {
        ServiceError::Unprocessable {
            kind: Invalid::Content,
            violation,
        } => ServiceError::Unprocessable {
            kind: Invalid::Archetype,
            violation,
        },
        other => other,
    }
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

/// The `422` a 1.4 → 2 conversion failure answers.
fn conversion_refused(e: ConvertError) -> ServiceError {
    ServiceError::content_invalid(
        Violation::new(format!("1.4 → 2 conversion failed: {e}")).with_source(e),
    )
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
