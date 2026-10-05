// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Derived-runtime resolution: the cached [`WebTemplate`] and example
//! COMPOSITION surfaces, thin over `openehr_sdt::flat`.
//!
//! Spec: `BASE/docs/architecture_overview/master10-archetypes.adoc`.
//! §Archetypes and Templates at Runtime gives a template two runtime functions,
//! validating data at capture and import against the RM and archetypes and
//! serving as the design basis for AQL paths; the validation and commit path
//! consumes [`FerroEhrService::web_template_for`]. §Deploying Archetypes and
//! Templates blesses a compiled near-runtime form "compiled into a near-runtime
//! form from the sharable openEHR form", which here is the [`WebTemplate`],
//! memoised in a `moka` cache.
//!
//! NOTE: the concrete `WebTemplate` JSON shape is not openEHR-normative, being
//! the Better `web-template` SDT format living in `openehr_sdt::flat`, so this
//! module stores, resolves and caches it without presenting it as canonical
//! openEHR; the builder's own id-sanitisation is a vendor rule, distinct from
//! the §Composite Identifiers and Case identity law applied to the cache key
//! (see [`crate::templates::identity`]).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): stored OPT/WebTemplate artefacts served verbatim \
              (families 1/8)"
)]

use std::sync::Arc;

use openehr_sdt::flat::example::{DetailLevel, ExampleType};
use openehr_sdt::flat::webtemplate::model::WebTemplate;
use serde_json::Value;

use super::identity;
use crate::service::FerroEhrService;
use crate::service::error::{ServiceError, Violation};

impl FerroEhrService {
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
    /// - [`ServiceError::Database`] — the store read failed.
    pub(crate) async fn web_template_for(
        &self,
        template_id: &str,
    ) -> Result<Arc<WebTemplate>, ServiceError> {
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
        let xml = match self.get_template_xml(template_id).await {
            Ok(xml) => xml,
            Err(ServiceError::NotFound(_)) => {
                return self.web_template_adl2_cached(template_id).await;
            }
            Err(e) => return Err(e),
        };

        self.build_cached_web_template(&key, template_id, &xml)
            .await
    }

    /// Generate an example COMPOSITION for a stored operational template
    /// (`GET /definition/template/adl1.4/{template_id}/example`).
    ///
    /// NOTE: example generation is not spec-mandated; it is a convenience
    /// surface, produced from the template's cached [`WebTemplate`] by
    /// [`example_composition`](openehr_sdt::flat::example::example_composition)
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
    ///   [`web_template_for`](Self::web_template_for) maps for an unknown
    ///   template on a *commit* path.
    /// - [`ServiceError::Unprocessable`] (→ `422`) — the template is stored but
    ///   cannot be built into a [`WebTemplate`].
    /// - [`ServiceError::Database`] — the store read failed.
    pub(crate) async fn template_example(
        &self,
        template_id: &str,
        level: DetailLevel,
        kind: ExampleType,
    ) -> Result<Value, ServiceError> {
        // Resolve existence first so an unknown id is a 404 (not the 422 the
        // WebTemplate cache maps for a commit-time unknown template).
        if !self.template_stored(template_id).await? {
            return Err(ServiceError::sm(
                crate::service::status::CallStatusType::TemplateDoesNotExist,
                format!("template {template_id}"),
            ));
        }
        let key = identity::canonical_key(template_id);
        let wt = if let Some(wt) = self.web_templates.get(&key).await {
            note_cache_event("hit");
            wt
        } else {
            note_cache_event("miss");
            let xml = self.get_template_xml(template_id).await?;
            self.build_cached_web_template(&key, template_id, &xml)
                .await?
        };
        let mut composition = openehr_sdt::flat::example::example_composition(&wt, level);
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
                    "operational template {template_id} could not be built into a WebTemplate: {e}"
                )))
            })
    }
}

/// Record one WebTemplate-cache hit/miss on the
/// [`crate::telemetry::metrics::WEBTEMPLATE_CACHE_EVENTS`] counter. No
/// openEHR spec governs this — our own observability design.
fn note_cache_event(event: &'static str) {
    crate::telemetry::metrics::metrics()
        .webtemplate_cache_events
        .add(1, &[opentelemetry::KeyValue::new("event", event)]);
}
