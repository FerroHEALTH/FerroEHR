// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The OPT 1.4 operational templates keyed by their wire `template_id`: the
//! upload, retrieval and listing behind the ITS-REST
//! `/definition/template/adl1.4` surface
//! (`ITS-REST/specifications/definition.openapi.yaml`), and the derived
//! [`WebTemplate`] every FLAT and STRUCTURED commit validates against.
//!
//! # Spec basis
//!
//! - (`AM/docs/OPT2/master02-overview.adoc` §Purpose of the OPT,
//!   §Types of OPT): the stored artefact is the compiled OPT; "a production
//!   EHR … can safely run only using guaranteed *validated* templates", hence
//!   the parse, structural and artefact-validity gates before an insert.
//! - (`AM/docs/AOM2/master10-templates.adoc` §Template Identifiers): a
//!   `TEMPLATE_ID` identifies a template; equality follows the §Composite
//!   Identifiers and Case rule (see [`crate::templates::identity`]).
//! - (`BASE/docs/architecture_overview/master10-archetypes.adoc` §Archetypes and
//!   Templates at Runtime, §Deploying Archetypes and Templates): a template
//!   validates data at capture and import, and may be "compiled into a
//!   near-runtime form from the sharable openEHR form", which here is the
//!   [`WebTemplate`], memoised in a `moka` cache.
//!
//! NOTE: OPT 1.4 has no normative prose chapter, so the canonical XML ingested
//! here is governed by the ITS-XML v1 Template XSD plus AOM 1.4.
//!
//! NOTE: the `WebTemplate` JSON shape is the Better `web-template` SDT format in
//! `openehr_sdt::flat`, not openEHR-normative, so it is cached and served
//! without being presented as canonical openEHR.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): stored OPT/WebTemplate artefacts served verbatim \
              (families 1/8)"
)]

use std::sync::Arc;

use openehr_its::opt14::types::OperationalTemplate;
use openehr_sdt::flat::example::{DetailLevel, ExampleType};
use openehr_sdt::flat::webtemplate::model::WebTemplate;
use serde_json::{Value, json};

use crate::error::refusal::{Invalid, Missing};
use crate::error::{ErrorChain, ServiceError, Violation};
use crate::service::FerroEhrService;
use crate::storage::template_repo::{self, NewTemplate, TemplateRow};
use crate::templates::identity;

impl FerroEhrService {
    /// Store an OPT 1.4 operational template from its canonical XML, returning
    /// the stored template's metadata descriptor (the ITS-REST template list
    /// shape — the `adl1.4` upload response body).
    ///
    /// The XML is parsed to validate it is a well-formed OPT and to pull the
    /// `template_id` (the unique key), `concept` and root archetype id, then run
    /// through the standalone-artefact validity catalogue before any row is
    /// written.
    ///
    /// Operational templates are immutable on the `adl1.4` upload endpoint:
    /// re-uploading an existing `template_id`, a case variant counting as the
    /// same id under §Composite Identifiers and Case, is a `Conflict` rather
    /// than a silent overwrite (ITS-REST
    /// `responses/409_template_already_exists.yaml`; CNF Robot case
    /// `I_DEFINITION_ADL14.upload_opt-valid_opt_twice_conflict`).
    ///
    /// # Errors
    ///
    /// - [`ServiceError::BadRequest`] (→ `400`) — the payload is not
    ///   well-formed XML (ITS-REST `responses/400.yaml`: "syntactically
    ///   invalid … content").
    /// - [`ServiceError::Unprocessable`] — well-formed XML that does not
    ///   decode as an OPT 1.4 document, or a decoded OPT with an empty
    ///   `template_id` or `concept`; the REST upload answers it `400`, the only
    ///   refusal `operations/definition_template_adl1.4_upload.yaml` declares.
    /// - The structural gate
    ///   (`crate::validation::validate_opt_structure`) and the
    ///   AOM2/08 artefact-validity catalogue
    ///   (`crate::validation::validate_opt_artefact`,
    ///   `AM/docs/AOM2/master08-validation.adoc`) propagate their own
    ///   typed rejections.
    /// - [`ServiceError::Unprocessable`] — the OPT does not build into a
    ///   Web Template, checked before the insert so such a template is never
    ///   stored; the REST upload answers it `400`.
    /// - [`ServiceError::Conflict`] (→ `409`) — a template with the same
    ///   `template_id` (case-insensitively) already exists.
    /// - [`ServiceError::Storage`] — the insert itself failed.
    pub(super) async fn store_template(&self, xml: &str) -> Result<Value, ServiceError> {
        let opt = parse_opt(xml)?;
        // Structural well-formedness the tolerant codec would otherwise accept
        // (foreign / duplicated top-level elements).
        crate::validation::structure::validate_opt_structure(xml)?;

        // The AOM2/08 standalone-artefact validity catalogue (VCOC/VACMCO,
        // VATID/VTLC, VTTBK/VTCBK, VCORM/VCARM/VCAEX/VCACA/VCAM → `422` carrying
        // the AOM2 rule code in `validationErrors[]`) is owned by the validation layer
        // (`crate::validation::opt`; spec `AM/docs/AOM2/master08-validation.adoc`).
        crate::validation::validate_opt_artefact(&opt)?;

        let template_id = opt.template_id.value.clone();
        // NOTE: no specification names this status token: our own design for a
        // semantically invalid operational template.
        if template_id.trim().is_empty() {
            return Err(ServiceError::Unprocessable {
                kind: Invalid::Template,
                violation: Violation::new("is missing from the operational template")
                    .with_path("template_id"),
            });
        }
        // `concept` is a mandatory `OPERATIONAL_TEMPLATE` attribute; an empty one
        // is a malformed OPT (CNF `removed_mandatory_elements/…removed_concept_value`).
        if opt.concept.trim().is_empty() {
            return Err(ServiceError::Unprocessable {
                kind: Invalid::Template,
                violation: Violation::new("of the operational template is empty")
                    .with_path("concept"),
            });
        }
        // NOTE: `operations/definition_template_adl1.4_upload.yaml` answers only
        // 201, 400 and 409, so a template the server cannot serve as a Web
        // Template is refused before it is stored, never after.
        let web_template = openehr_sdt::flat::webtemplate::builder::build_web_template(&opt)
            .map_err(|e| ServiceError::Unprocessable {
                kind: Invalid::Template,
                violation: Violation::new(format!(
                    "the operational template could not be built into a Web Template: {}",
                    ErrorChain::new(&e)
                )),
            })?;
        let root_archetype = &opt.definition.archetype_id.value;
        let row = template_repo::insert_template(
            &self.pool,
            NewTemplate {
                template_id: &template_id,
                concept: Some(&opt.concept),
                root_archetype: (!root_archetype.trim().is_empty())
                    .then_some(root_archetype.as_str()),
                content: xml,
            },
        )
        .await?
        .ok_or_else(|| {
            ServiceError::conflict(format!(
                "an operational template with template_id '{template_id}' already exists"
            ))
        })?;

        // The create path needs no `WebTemplate`-cache invalidation: the insert
        // never overwrites and `web_template` caches only successful builds,
        // so a freshly stored template_id can have no pre-existing entry. The
        // cache is invalidated where a template's lifetime ends, in the delete.
        // The Web Template built above is cached, so the upload's representation
        // reads no row back.
        let key = identity::canonical_key(&template_id);
        if let Err(error) = self
            .web_templates
            .get_or_build(&key, || Ok(web_template))
            .await
        {
            tracing::warn!(template_id, %error, "the uploaded Web Template was not cached");
        }
        Ok(template_descriptor(&row))
    }

    /// `GET /definition/template/adl1.4/{template_id}`: the OPT 1.4 canonical
    /// XML addressed by its `template_id` string, the ITS-REST wire address,
    /// matched case-insensitively (BASE master05 §Composite Identifiers and
    /// Case).
    ///
    /// # Errors
    ///
    /// - No OPT with that `template_id` → [`Missing::Template`] (`404`).
    /// - A database failure (`500`).
    pub async fn template_adl14_get(&self, template_id: &str) -> Result<String, ServiceError> {
        template_repo::template_xml(&self.pool, template_id)
            .await?
            .ok_or_else(|| {
                ServiceError::not_found(Missing::Template, format!("OPT template_id {template_id}"))
            })
    }

    /// Resolve the (cached) [`WebTemplate`] for a stored operational template,
    /// building it from the stored OPT 1.4 XML on first use.
    ///
    /// The cache is keyed by the §Composite Identifiers and Case canonical form
    /// of `template_id`, so case variants of one stored template resolve
    /// to a single cached [`WebTemplate`].
    ///
    /// # Errors
    ///
    /// - [`ServiceError::Unprocessable`] (ITS-REST `422`, not `NotFound`) when
    ///   the template is not in the store: on a composition commit an unknown
    ///   referenced template is a semantic error (`responses/422.yaml`, CNF
    ///   Robot case `I_EHR_COMPOSITION.create_composition-event_bad_opt`).
    /// - [`ServiceError::Unprocessable`] when the stored XML fails to build
    ///   into a [`WebTemplate`].
    /// - [`ServiceError::Storage`] — the store read failed.
    pub async fn web_template(&self, template_id: &str) -> Result<Arc<WebTemplate>, ServiceError> {
        // Key the cache on the identity-canonical form so a case variant resolves
        // to the single entry for the same template.
        let key = identity::canonical_key(template_id);

        // A resident WebTemplate is served without touching `template_store`:
        // composition validation calls this once per commit, so a warm template
        // makes every later commit a pure in-memory hit (no openEHR spec governs
        // the cache — our own design).
        if let Some(wt) = self.web_templates.get(&key).await {
            note_cache_event("hit");
            return Ok(wt);
        }
        note_cache_event("miss");

        // On a miss the stored OPT XML is loaded once, and an id that is not an
        // ADL 1.4 template falls back to the ADL2/OPT2 store so a FLAT/STRUCTURED
        // commit against an ADL2-registered template still resolves. Only after
        // BOTH stores miss is the id unknown (the commit-path 422).
        let Some(xml) = template_repo::template_xml(&self.pool, template_id).await? else {
            return self.web_template_adl2_cached(template_id).await;
        };
        self.build_cached_web_template(&key, template_id, &xml)
            .await
    }

    /// Generate an example COMPOSITION, or an example party for a demographic
    /// template, for a stored operational template
    /// (`GET /definition/template/adl1.4/{template_id}/example`).
    ///
    /// NOTE: example generation is not spec-mandated; it is a convenience
    /// surface, produced from the template's cached [`WebTemplate`] by
    /// [`example_instance`](openehr_sdt::flat::example::example_instance)
    /// at the requested [`DetailLevel`], with a deterministic `uid` for the
    /// `output` ([`ExampleType::Output`]) form.
    ///
    /// Existence is probed on every call so a template another replica deleted
    /// is not served from a stale cache entry, but the probe is `EXISTS`-shaped
    /// and the stored XML moves only for a cold-cache build.
    ///
    /// # Errors
    ///
    /// - [`ServiceError::NotFound`] (→ ITS-REST `404`,
    ///   `responses/404_unknown_template_id.yaml`) — unknown `template_id`,
    ///   matching the `adl1.4/{template_id}` GET surface rather than the `422`
    ///   [`web_template`](Self::web_template) maps for an unknown
    ///   template on a *commit* path.
    /// - [`ServiceError::Unprocessable`] (→ `422`) — the template is stored but
    ///   cannot be built into a [`WebTemplate`].
    /// - [`ServiceError::BadRequest`] (→ `400`) — the template's root is neither
    ///   a COMPOSITION nor a party class, or a party template constrains no
    ///   identity, so no example exists for it (see [`example_or_refusal`]).
    /// - [`ServiceError::Internal`] (→ `500`) — the generated document did not
    ///   build, a server fault.
    /// - [`ServiceError::Storage`] — the store read failed.
    pub(super) async fn template_example(
        &self,
        template_id: &str,
        level: DetailLevel,
        kind: ExampleType,
    ) -> Result<Value, ServiceError> {
        let unknown =
            || ServiceError::not_found(Missing::Template, format!("template {template_id}"));
        // Resolve existence first so an unknown id is a 404 (not the 422 the
        // WebTemplate cache maps for a commit-time unknown template).
        if !template_repo::template_exists(&self.pool, template_id).await? {
            return Err(unknown());
        }
        let key = identity::canonical_key(template_id);
        let wt = if let Some(wt) = self.web_templates.get(&key).await {
            note_cache_event("hit");
            wt
        } else {
            note_cache_event("miss");
            let xml = template_repo::template_xml(&self.pool, template_id)
                .await?
                .ok_or_else(unknown)?;
            self.build_cached_web_template(&key, template_id, &xml)
                .await?
        };
        let mut composition = example_or_refusal(&wt, level)?;
        if kind == ExampleType::Output {
            openehr_sdt::flat::example::apply_output_uid(&mut composition, template_id);
        }
        Ok(composition)
    }

    /// Build the [`WebTemplate`] for `xml` and cache it under `key` (the
    /// §Composite Identifiers and Case canonical form of `template_id`).
    ///
    /// The expensive build is single-flighted by the cache's `get_or_build`
    /// (one build per key under contention); only a *successful* build is
    /// cached, so no negative entry can shadow a later upload.
    ///
    /// # Errors
    ///
    /// [`ServiceError::Unprocessable`] (→ `422`) — the stored XML does not
    /// re-parse as an OPT or the OPT does not build into a [`WebTemplate`]
    /// (a stored-but-unbuildable template).
    async fn build_cached_web_template(
        &self,
        key: &str,
        template_id: &str,
        xml: &str,
    ) -> Result<Arc<WebTemplate>, ServiceError> {
        self.web_templates
            .get_or_build(key, || {
                let opt = openehr_its::opt14::from_xml(xml)
                    .map_err(openehr_sdt::flat::error::FlatError::OptParse)?;
                openehr_sdt::flat::webtemplate::builder::build_web_template(&opt)
            })
            .await
            .map_err(|e| {
                ServiceError::content_invalid(Violation::new(format!(
                    "operational template {template_id} could not be built into a WebTemplate: {}",
                    ErrorChain::new(&*e)
                )))
            })
    }
}

/// The openEHR template descriptor of one stored template (the ITS-REST
/// template list shape).
///
/// The `version` field is the ITS-REST `TemplateMetadata.version` (optional +
/// `deprecated`; `definition-codegen.openapi.yaml`
/// §components.schemas.TemplateMetadata). It is **not** stored — it is a pure
/// function of `template_id` (its `.vN` axis; `filter_version`: "taken from
/// `template_id`"), so it is derived here rather than denormalised into a
/// column (see [`crate::templates::identity::template_version`]) and emitted
/// only when the id carries a version, matching the schema's optional field.
pub(super) fn template_descriptor(row: &TemplateRow) -> Value {
    let mut descriptor = json!({
        "template_id": row.template_id,
        "concept": row.concept,
        "archetype_id": row.root_archetype,
        "created_timestamp": row.created_at.to_string(),
    });
    // `version` is optional (deprecated) — include it only when derivable
    // from the id, never as an explicit `null`.
    if let Some(version) = identity::template_version(&row.template_id)
        && let Some(obj) = descriptor.as_object_mut()
    {
        obj.insert("version".to_owned(), Value::String(version));
    }
    descriptor
}

/// Record one WebTemplate-cache hit/miss on the
/// [`crate::telemetry::metrics::WEBTEMPLATE_CACHE_EVENTS`] counter. No
/// openEHR spec governs this — our own observability design.
fn note_cache_event(event: &'static str) {
    crate::telemetry::metrics::metrics()
        .webtemplate_cache_events
        .add(1, &[opentelemetry::KeyValue::new("event", event)]);
}

/// The example instance of `wt` at `level`, or the refusal the example
/// operation answers.
///
/// ITS-REST `definition_template_adl1.4_example_get` answers an instance of a
/// versioned-object class (`responses/200_Template_example_retrieved.yaml`
/// lists COMPOSITION and the five parties): a COMPOSITION template gets an
/// example COMPOSITION and a demographic template an example party of its root
/// class. The operation allows a `400` for what the server does not generate;
/// a template rooted at an ENTRY or a SECTION admits none of those classes, so
/// it is refused rather than served an instance its root class does not
/// declare (#3780). A party template that constrains no `PARTY_IDENTITY` is
/// refused the same way, since RM demographic §PARTY `Identities_valid` admits
/// no party without one.
///
/// # Errors
///
/// [`ServiceError::BadRequest`] (`400`) naming the root for a template rooted
/// at neither a COMPOSITION nor a party class, or naming the class of a party
/// template that constrains no identity; [`ServiceError::Internal`] (`500`)
/// when the generated document does not build.
pub(super) fn example_or_refusal(
    wt: &WebTemplate,
    level: DetailLevel,
) -> Result<Value, ServiceError> {
    match openehr_sdt::flat::example::example_instance(wt, level) {
        Ok(instance) => Ok(instance),
        Err(
            e @ (openehr_sdt::flat::error::FlatError::NoExampleForRoot { .. }
            | openehr_sdt::flat::error::FlatError::NoExampleIdentity { .. }),
        ) => Err(ServiceError::bad_request(e.to_string(), e)),
        Err(e) => Err(ServiceError::internal("generate the template example", e)),
    }
}

/// Parse OPT 1.4 canonical XML into an [`OperationalTemplate`].
///
/// Failure classification follows the ITS-REST status split: a payload that is
/// not well-formed XML at all is *syntactically invalid content* — the released
/// `400` branch (`docs/specs/openehr/ITS-REST/specifications/responses/400.yaml`:
/// "the request could not be parsed or is invalid (e.g. … syntactically
/// invalid … content)") — while well-formed XML that does not decode as an OPT
/// is a semantic error on the artefact (the overview status table's `422` row,
/// `docs/specs/openehr/ITS-REST/specifications/docs/overview/
/// Requests_and_responses.md` §HTTP status codes; no template operation
/// declares `422`, so the semantic branch is the adjudicated handling).
///
/// # Errors
///
/// - [`ServiceError::BadRequest`] (→ ITS-REST `400`) when the payload is not
///   well-formed XML (including an empty document).
/// - [`ServiceError::Unprocessable`] (→ ITS-REST `422`) when well-formed XML
///   does not decode as an OPT 1.4 `OPERATIONAL_TEMPLATE` document.
fn parse_opt(xml: &str) -> Result<OperationalTemplate, ServiceError> {
    require_well_formed_xml(xml)?;
    openehr_its::opt14::from_xml(xml).map_err(|e| {
        // NOTE: no specification names this status token: our own design for a
        // semantically invalid operational template.
        ServiceError::Unprocessable {
            kind: Invalid::Template,
            violation: Violation::new(format!("invalid OPT 1.4 XML: {e}")),
        }
    })
}

/// The well-formedness gate ahead of the OPT decode: scan the document with a
/// bare `quick_xml::Reader` and reject anything that is not one well-formed
/// XML document with a root element. This isolates the released `400` branch
/// (syntactically invalid content) from the semantic `422` branch the tolerant
/// codec reports past it.
fn require_well_formed_xml(xml: &str) -> Result<(), ServiceError> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut saw_root = false;
    let mut open = 0_usize;
    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Eof) => break,
            Ok(quick_xml::events::Event::Start(_)) => {
                saw_root = true;
                open = open.saturating_add(1);
            }
            Ok(quick_xml::events::Event::Empty(_)) => saw_root = true,
            Ok(quick_xml::events::Event::End(_)) => open = open.saturating_sub(1),
            Ok(_) => {}
            Err(e) => {
                return Err(ServiceError::precondition(format!(
                    "syntactically invalid XML content: {e}"
                )));
            }
        }
    }
    // NOTE: XML 1.0 §2.1 `document ::= prolog element Misc*`; an element still open
    // at end of input is no document, so a truncated upload is the 400 branch.
    if open > 0 {
        return Err(ServiceError::precondition(format!(
            "syntactically invalid XML content: the document ends with {open} element(s) unclosed"
        )));
    }
    if saw_root {
        Ok(())
    } else {
        Err(ServiceError::precondition(
            "syntactically invalid XML content: the document has no root element".to_owned(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::require_well_formed_xml;
    use crate::error::ServiceError;
    use crate::templates::identity;

    #[test]
    fn a_truncated_document_is_a_bad_request() {
        let truncated =
            r#"<template xmlns="http://schemas.openehr.org/v1"><template_id><value>vdef"#;
        let err = require_well_formed_xml(truncated).expect_err("truncated XML");
        assert!(matches!(err, ServiceError::BadRequest(_)), "{err:?}");
    }

    #[test]
    fn a_closed_document_passes() {
        require_well_formed_xml("<template><a/><b>x</b></template>").expect("well-formed");
    }

    #[test]
    fn store_lookup_is_case_insensitive_by_law() {
        // The store SQL boundary compares `lower(template_id) = lower($1)`, i.e.
        // the §Composite Identifiers and Case canonical form. This guards
        // the invariant that the identity module and the SQL boundary agree.
        assert_eq!(
            identity::canonical_key("Vital-Signs.v1"),
            identity::canonical_key("vital-signs.V1"),
        );
    }
}
