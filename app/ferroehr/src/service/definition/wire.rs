// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ITS-REST wire-shaped DEFINITION extension methods: the rich shapes the
//! `DEFINITION` API group returns (template summaries, the example
//! COMPOSITION, `StoredQuery` descriptors, glob filters) that the SM
//! `I_DEFINITION_*` interfaces do not express. Native error types only
//! (`SmError`, or `ServiceError` where a structured per-code validation body is
//! carried — the ADL2 upload), so this layer stays protocol-free; the route
//! wiring is the ITS-REST layer's concern. The retrieval/store behaviour rides
//! on the SM logic in the sibling interface files.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): stored template/query artefacts served verbatim + \
              ADL/OPT wire envelopes"
)]

use regex::Regex;
use serde_json::Value;

use openehr_sdt::flat::example::{DetailLevel, ExampleType};

use crate::service::FerroEhrService;
use crate::service::definition::types::{Adl2Template, TemplateListFilter};
use crate::service::error::ServiceError;
use crate::service::list::Page;
use crate::service::status::SmError;

use super::{paginate, query::is_aql_v1};

impl FerroEhrService {
    /// `POST /definition/template/adl1.4` — parse + store an OPT 1.4
    /// canonical-XML template through the templates layer; the wire `201` body
    /// is the created template summary.
    ///
    /// # Errors
    ///
    /// - Unparseable / structurally invalid OPT XML → `invalid_template`
    ///   (`422`).
    /// - A template with the same `template_id` already stored → conflict
    ///   (`409`).
    /// - A database failure (`exception` → `500`).
    pub async fn template_adl14_upload(&self, opt_xml: String) -> Result<Value, SmError> {
        Ok(self.store_template(&opt_xml).await?)
    }

    /// `GET /definition/template/adl1.4/{template_id}` — the OPT 1.4 canonical
    /// XML addressed by its `template_id` string (the ITS-REST wire address;
    /// the SM keys OPTs by `UUID`). Identity is case-insensitive.
    ///
    /// # Errors
    ///
    /// - No OPT with that `template_id` → `template_does_not_exist` (`404`).
    /// - A database failure (`exception` → `500`).
    pub async fn template_adl14_get(&self, template_id: String) -> Result<String, SmError> {
        Ok(self.opt_get_by_template_id(&template_id).await?)
    }

    /// `GET /definition/template/adl1.4` — the stored template summaries,
    /// filtered by the wire's `filter_template_id`/`concept`/`version` globs
    /// (`*` wildcard) and cursored by `offset`/`fetch`
    /// (`docs/specs/openehr/ITS-REST/specifications/operations/definition_template_adl1.4_list.yaml`).
    ///
    /// NOTE: the version value derives from the `template_id`'s `.vN` axis
    /// (`crate::templates::identity::template_version`), which is the template's
    /// whole version and lifecycle mechanism; no parallel lifecycle-state model
    /// exists.
    ///
    /// With `version` absent the listing collapses to the latest version of each
    /// template, and any glob lists every matching stored version. The ITS-REST
    /// docs text is silent, so the released OAS grounds it: "Filter by version
    /// (e.g. `1.2.*` or use `*` for all versions), taken from `template_id`; if
    /// missing, then only the latest version will be returned"
    /// (`specifications/parameters/query/filter_version.yaml`).
    ///
    /// # Errors
    ///
    /// A database failure (`exception` → `500`).
    pub async fn template_adl14_list(
        &self,
        filter: TemplateListFilter,
        page: Page,
    ) -> Result<Vec<Value>, SmError> {
        Ok(filter_templates(
            self.template_summaries().await?,
            &filter,
            page,
        ))
    }

    /// `GET /definition/template/adl1.4/{template_id}/example` — an example
    /// COMPOSITION built from the template's `WebTemplate` by the templates
    /// layer. `kind`/`detail_level` are the released-OAS
    /// `example_type`/`example_detail_level` enums
    /// (`docs/specs/openehr/ITS-REST/computable/OAS/definition-codegen.openapi.yaml`
    /// §`components.parameters`).
    ///
    /// # Errors
    ///
    /// - An out-of-enum `detail_level` or `kind` value →
    ///   `precondition_violation` (`400`).
    /// - No template with that `template_id` → `template_does_not_exist`
    ///   (`404`).
    /// - A database failure (`exception` → `500`).
    pub async fn template_adl14_example(
        &self,
        template_id: String,
        detail_level: Option<String>,
        kind: Option<String>,
    ) -> Result<Value, SmError> {
        let level =
            DetailLevel::from_query(detail_level.as_deref()).map_err(SmError::precondition)?;
        let kind = ExampleType::from_query(kind.as_deref()).map_err(SmError::precondition)?;
        Ok(self.template_example(&template_id, level, kind).await?)
    }

    /// `GET /definition/template/adl2/{template_id}/example` — an example
    /// COMPOSITION built from the ADL2 template's `WebTemplate` (the `v2_4` front
    /// end feeding the shared example generator). `kind`/`detail_level` are the
    /// `example_type`/`example_detail_level` query enums.
    ///
    /// # Errors
    ///
    /// - An out-of-enum `detail_level` or `kind` value → `BadRequest` (`400`).
    /// - No template with that `template_id` → `NotFound` (`404`).
    /// - The stored template cannot be compiled/built → `Unprocessable` (`422`).
    /// - A database failure (`500`).
    pub async fn template_adl2_example(
        &self,
        template_id: String,
        detail_level: Option<String>,
        kind: Option<String>,
    ) -> Result<Value, ServiceError> {
        let level =
            DetailLevel::from_query(detail_level.as_deref()).map_err(ServiceError::precondition)?;
        let kind = ExampleType::from_query(kind.as_deref()).map_err(ServiceError::precondition)?;
        self.adl2_example(&template_id, level, kind).await
    }

    /// `POST /definition/template/adl2` — validate ADL2 operational-template
    /// source (text/plain) through the `openehr-adl` engine, store it, and
    /// return the stored `ARCHETYPE_HRID`; the dispatcher builds `Location` +
    /// the `Prefer` body from it (`201_Template_adl2_upload`).
    ///
    /// Returns [`ServiceError`] rather than `SmError` so a semantic-validation
    /// failure keeps its structured per-code violations for the ITS-REST `Error`
    /// body. Duplicate handling diverges by surface: the REST contract declares
    /// `409_template_already_exists.yaml` while the SM native `upload_artefact`
    /// replaces (`i_definition_adl2.adoc`), so an existing HRID is a `409` here.
    ///
    /// # Errors
    ///
    /// - Unparseable source → `BadRequest` (`400`,
    ///   `definition_template_adl2_upload.yaml` → `responses/400.yaml`).
    /// - AOM2-invalid source → `ValidationFailed` (`422` with the rule-code
    ///   mnemonics).
    /// - An ADL2 artefact with the same HRID already stored → `Conflict`
    ///   (`409`).
    /// - A database failure (`500`).
    pub async fn template_adl2_upload(&self, source: String) -> Result<String, ServiceError> {
        self.adl2_wire_upload(&source).await
    }

    /// `GET /definition/template/adl2/{template_id}` (and the deprecated
    /// `…/{template_id}/{version}`) — the stored ADL2 source, resolved from a
    /// full or partial `template_id` (`+` optional SEMVER `version`). Served as
    /// `text/plain` (`200_Template_adl2_retrieved.yaml` body `oneOf:
    /// [OperationalTemplateV2, string]`, example = ADL2 source): the stored
    /// source is returned verbatim (lossless). The resolved `ARCHETYPE_HRID`
    /// travels with the payload as the served artefact's identity (the wire
    /// `ETag` source — see [`Adl2Template`]).
    ///
    /// # Errors
    ///
    /// - No stored template matches → `NotFound` (`404`).
    /// - A database failure (`500`).
    pub async fn template_adl2_source(
        &self,
        template_id: String,
        version: Option<String>,
    ) -> Result<Adl2Template, ServiceError> {
        let hrid = self.adl2_resolve(&template_id, version.as_deref()).await?;
        let payload = self.adl2_get(&hrid).await?;
        Ok(Adl2Template { hrid, payload })
    }

    /// `GET /definition/template/adl2/{template_id}` with `Accept:
    /// application/json` — the `OperationalTemplateV2` canonical-JSON projection
    /// of the resolved template (`200_Template_adl2_retrieved.yaml`,
    /// `application/json` → `OperationalTemplateV2`).
    ///
    /// # Errors
    ///
    /// - No stored template matches → `NotFound` (`404`).
    /// - The OPT cannot be compiled → `Unprocessable` (`422`).
    /// - A database failure (`500`).
    pub async fn template_adl2_opt_json(
        &self,
        template_id: String,
        version: Option<String>,
    ) -> Result<Adl2Template, ServiceError> {
        let hrid = self.adl2_resolve(&template_id, version.as_deref()).await?;
        let source = self.adl2_get(&hrid).await?;
        let payload = self.adl2_opt_json(&source).await?;
        Ok(Adl2Template { hrid, payload })
    }

    /// `GET /definition/template/adl2` — the ADL2 twin of
    /// [`template_adl14_list`](Self::template_adl14_list). The store yields
    /// `TemplateMetadata` rows (`template_id`, `concept`, `archetype_id`,
    /// `created_timestamp`); `template_id`/`concept`/`version` globs +
    /// `offset`/`fetch` are honoured over the full set here.
    ///
    /// # Errors
    ///
    /// A database failure (`exception` → `500`).
    pub async fn template_adl2_list(
        &self,
        filter: TemplateListFilter,
        page: Page,
    ) -> Result<Vec<Value>, SmError> {
        Ok(filter_templates(
            self.adl2_template_list(Page::all()).await?,
            &filter,
            page,
        ))
    }

    /// `GET /definition/query/{qualified_query_name}` — all stored versions of
    /// every query whose qualified name starts with `qualified_query_name`
    /// (a case-insensitive prefix; empty ⇒ all —
    /// `definition_query_list.yaml`).
    ///
    /// # Errors
    ///
    /// - A row-decode failure on a `NOT NULL` column (a genuine server fault)
    ///   → `exception` (`500`).
    /// - A database failure (`exception` → `500`).
    pub async fn query_list(&self, qualified_query_name: String) -> Result<Vec<Value>, SmError> {
        Ok(self.list_stored_queries(&qualified_query_name).await?)
    }

    /// `GET /definition/query/{qualified_query_name}/{version}` — one stored
    /// query at an exact version or a SEMVER prefix (`{major}` /
    /// `{major}.{minor}` → the highest matching stored version,
    /// `parameters/path/version.yaml`).
    ///
    /// # Errors
    ///
    /// - No stored query matching that name + version → not-found (`404`).
    /// - A row-decode failure (a genuine server fault) → `exception` (`500`).
    /// - A database failure (`exception` → `500`).
    pub async fn query_version_get(
        &self,
        qualified_query_name: String,
        version: String,
    ) -> Result<Value, SmError> {
        Ok(self
            .get_stored_query(&qualified_query_name, Some(&version))
            .await?)
    }

    /// `PUT /definition/query/{qualified_query_name}[/{version}]`: stores a
    /// query under its qualified name.
    ///
    /// `query_type` is the query's formalism (`QUERY_DESCRIPTOR.formalism`,
    /// default `AQL`, case-insensitive); the build validates and stores AQL
    /// only, so a non-AQL formalism is a distinct unsupported-formalism `400`
    /// and AQL bodies fall through to the store-time syntactic check. Returns
    /// the effective SEMVER the store wrote at, so the dispatcher's `Location`
    /// names exactly the stored resource (`headers/Location_Query.yaml`). The
    /// store response itself is bodyless.
    ///
    /// # Errors
    ///
    /// - A non-AQL `query_type` → `precondition_violation` (`400`).
    /// - A body that fails the AQL parse → `precondition_violation` (`400`).
    /// - With an explicit `version`, a non-exact SEMVER → `precondition_violation`
    ///   (`400`), and an already-existing `(name, version)` pair → conflict
    ///   (`409`, `409_StoredQuery_version.yaml`).
    /// - A database failure (`exception` → `500`).
    pub async fn query_store(
        &self,
        qualified_query_name: String,
        version: Option<String>,
        query_type: String,
        body: String,
    ) -> Result<String, SmError> {
        if !is_aql_v1(&query_type) {
            return Err(SmError::precondition(format!(
                "unsupported query formalism `{query_type}`: only AQL is supported for \
                 stored queries (parameters/query/query_type.yaml)"
            )));
        }
        Ok(self
            .store_query_version(&qualified_query_name, version.as_deref(), body)
            .await?)
    }
}

/// Filter + paginate a list of template descriptors per the wire query params.
///
/// `template_id` glob-matches the `template_id` field, `concept` glob-matches
/// the `concept` field (a record lacking a filtered field does not match), and
/// `version` glob-matches the id's VERSION AXIS — "Filter by version (e.g.
/// `1.2.*` or use `*` for all versions), taken from `template_id`" (the
/// released OAS `parameters/query/filter_version.yaml`, grounding docs-text
/// silence) — so an id without a version axis survives only a glob admitting
/// the empty axis (a bare `*`). With `version` ABSENT the survivors collapse
/// to the latest version of each template ("if missing, then only the latest
/// version will be returned" — the same source); `page` then applies
/// `offset`/`fetch` (`parameters/query/filter_template_id.yaml` — "supports
/// wildcards `*`"; `master02-overview.adoc` §List Handling).
fn filter_templates(list: Vec<Value>, filter: &TemplateListFilter, page: Page) -> Vec<Value> {
    let tid = filter.template_id.as_deref().map(glob_to_regex);
    let concept = filter.concept.as_deref().map(glob_to_regex);
    let version = filter.version.as_deref().map(glob_to_regex);
    let matches = |re: &Option<Regex>, field: Option<&str>| match re {
        None => true,
        Some(re) => field.is_some_and(|v| re.is_match(v)),
    };
    let filtered = list
        .into_iter()
        .filter(|row| {
            let template_id = row.get("template_id").and_then(Value::as_str);
            let concept_field = row.get("concept").and_then(Value::as_str);
            let version_axis = template_id
                .and_then(split_version_axis)
                .map(|(_, axis)| axis)
                .unwrap_or_default();
            matches(&tid, template_id)
                && matches(&concept, concept_field)
                && matches(&version, Some(&version_axis))
        })
        .collect();
    let effective = if version.is_none() {
        collapse_to_latest(filtered)
    } else {
        filtered
    };
    paginate(effective.into_iter(), page)
}

/// Collapse a filtered template listing to the latest version of each
/// template — the ABSENT-`version` wire behaviour
/// (`parameters/query/filter_version.yaml`: "if missing, then only the latest
/// version will be returned").
///
/// "The same template" means the same base identifier once the `.vN` version
/// axis (`crate::templates::identity::template_version`) is stripped, compared
/// case-insensitively like every template identity
/// (`crate::templates::identity::canonical_key`). Version axes order
/// numerically segment by segment (`1.10` > `1.2`); on a numeric-prefix tie
/// the longer, more precise axis wins (`1.0` > `1`).
///
/// NOTE: no openEHR spec relates an id without a version axis to a versioned
/// sibling or orders numerically equal axes, so an unversioned id stays its own
/// identity and is always listed and the prefix-tie rule is our own
/// deterministic tiebreak — our own design/extension.
fn collapse_to_latest(rows: Vec<Value>) -> Vec<Value> {
    // The best (row index, version axis) seen per base identity, in first-seen
    // order so the collapse is stable with respect to the store's listing.
    let mut best: Vec<(String, usize, String)> = Vec::new();
    let mut keep: Vec<Option<Value>> = Vec::new();
    for row in rows {
        let idx = keep.len();
        let Some((base, axis)) = row
            .get("template_id")
            .and_then(Value::as_str)
            .and_then(split_version_axis)
        else {
            // No template_id (already unmatched by any filter) or no version
            // axis: its own identity, always listed.
            keep.push(Some(row));
            continue;
        };
        keep.push(Some(row));
        match best.iter_mut().find(|(b, _, _)| *b == base) {
            None => best.push((base, idx, axis)),
            Some((_, held_idx, held_axis)) => {
                // Both indices come from the `enumerate` above, so they address
                // slots this loop already pushed; fetched rather than indexed so
                // the bookkeeping is the only thing that has to stay correct.
                let drop_idx = if version_axis_gt(&axis, held_axis) {
                    let superseded = *held_idx;
                    *held_idx = idx;
                    *held_axis = axis;
                    superseded
                } else {
                    idx
                };
                if let Some(slot) = keep.get_mut(drop_idx) {
                    *slot = None;
                }
            }
        }
    }
    keep.into_iter().flatten().collect()
}

/// Split a template id into its case-folded base identity and its version
/// axis; `None` when the id carries no `.vN` axis.
fn split_version_axis(template_id: &str) -> Option<(String, String)> {
    let axis = crate::templates::identity::template_version(template_id)?;
    let trimmed = template_id.trim();
    let base = trimmed
        .get(..trimmed.len() - (".v".len() + axis.len()))
        .unwrap_or(trimmed);
    Some((crate::templates::identity::canonical_key(base), axis))
}

/// `true` iff version axis `a` orders strictly after `b`: dotted segments
/// compare numerically (digits-only by construction, so longer segment ⇒
/// larger number; equal length ⇒ lexicographic), and on a numeric-prefix tie
/// the axis with more segments wins.
fn version_axis_gt(a: &str, b: &str) -> bool {
    let seg = |s: &str| s.split('.').map(str::to_owned).collect::<Vec<_>>();
    let (a, b) = (seg(a), seg(b));
    for (x, y) in a.iter().zip(&b) {
        match (x.len(), x.as_str()).cmp(&(y.len(), y.as_str())) {
            std::cmp::Ordering::Greater => return true,
            std::cmp::Ordering::Less => return false,
            std::cmp::Ordering::Equal => {}
        }
    }
    a.len() > b.len()
}

/// Compile a glob pattern (`*` wildcard, per `filter_template_id.yaml`) into an
/// anchored regex; all other characters are matched literally. A bare `*`
/// matches everything; an empty pattern matches only the empty string.
fn glob_to_regex(pattern: &str) -> Regex {
    let escaped = pattern
        .split('*')
        .map(regex::escape)
        .collect::<Vec<_>>()
        .join(".*");
    #[expect(
        clippy::expect_used,
        reason = "every segment went through regex::escape and the only \
                  unescaped metacharacters are the `.*` this function itself \
                  inserts, so the Err arm is unreachable"
    )]
    Regex::new(&format!("^{escaped}$")).expect("a glob-derived pattern should always compile")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(ids: &[&str]) -> Vec<Value> {
        ids.iter()
            .map(|id| serde_json::json!({ "template_id": id, "concept": id }))
            .collect()
    }

    fn ids(rows: &[Value]) -> Vec<&str> {
        rows.iter()
            .filter_map(|r| r.get("template_id").and_then(Value::as_str))
            .collect()
    }

    #[test]
    fn absent_version_collapses_to_the_latest_axis() {
        // filter_version.yaml: "if missing, then only the latest version will
        // be returned" — one row per base id, the highest `.vN` axis.
        let out = filter_templates(
            rows(&[
                "Encounter.v1",
                "Encounter.v2",
                "Encounter.v1.9",
                "Vital Signs.v0",
            ]),
            &TemplateListFilter::default(),
            Page::all(),
        );
        assert_eq!(ids(&out), vec!["Encounter.v2", "Vital Signs.v0"]);
    }

    #[test]
    fn version_axes_order_numerically_not_lexically() {
        // `1.10` > `1.9`, and `2` > `1.999`.
        let out = filter_templates(
            rows(&["T.v1.9", "T.v1.10", "U.v1.999", "U.v2"]),
            &TemplateListFilter::default(),
            Page::all(),
        );
        assert_eq!(ids(&out), vec!["T.v1.10", "U.v2"]);
    }

    #[test]
    fn base_identity_is_case_insensitive_and_prefix_ties_prefer_precision() {
        // `ENCOUNTER.V2` vs `Encounter.v1`: same base identity
        // (identity::canonical_key) — but note template_version only
        // recognizes the lowercase `.v` axis, so the uppercase id is its own
        // unversioned identity and both survive. Same-case bases collapse;
        // a numeric-prefix tie keeps the more precise axis (`1.0` > `1`).
        let out = filter_templates(
            rows(&["Report.v1", "report.v1.0"]),
            &TemplateListFilter::default(),
            Page::all(),
        );
        assert_eq!(ids(&out), vec!["report.v1.0"]);
    }

    #[test]
    fn unversioned_ids_are_their_own_identity_and_always_listed() {
        // No spec relates `Foo` to `Foo.v1` — the unversioned id stays listed
        // beside the latest versioned sibling.
        let out = filter_templates(
            rows(&["Foo", "Foo.v1", "Foo.v3", "Foo.verified"]),
            &TemplateListFilter::default(),
            Page::all(),
        );
        assert_eq!(ids(&out), vec!["Foo", "Foo.v3", "Foo.verified"]);
    }

    #[test]
    fn star_version_glob_lists_every_stored_version() {
        // `*` — "use `*` for all versions" (filter_version.yaml): no collapse.
        let all = rows(&["Encounter.v1", "Encounter.v2"]);
        let out = filter_templates(
            all.clone(),
            &TemplateListFilter {
                version: Some("*".into()),
                ..TemplateListFilter::default()
            },
            Page::all(),
        );
        assert_eq!(out, all);
    }

    #[test]
    fn concrete_version_glob_filters_without_collapsing() {
        // The OAS's own shape (`1.*`, filter_version.yaml) against the ids'
        // version AXES: every matching version, no collapse.
        let out = filter_templates(
            rows(&["T.v1.0", "T.v1.5", "T.v2.0"]),
            &TemplateListFilter {
                version: Some("1.*".into()),
                ..TemplateListFilter::default()
            },
            Page::all(),
        );
        assert_eq!(ids(&out), vec!["T.v1.0", "T.v1.5"]);
    }

    #[test]
    fn exact_version_glob_matches_the_axis_alone() {
        // The glob's subject is the version AXIS ("taken from template_id" —
        // filter_version.yaml), never the whole template_id: matching against
        // the whole id made `?version=1.0` match nothing while T.v1.0 was
        // stored.
        let out = filter_templates(
            rows(&["T.v1.0", "T.v1.5"]),
            &TemplateListFilter {
                version: Some("1.0".into()),
                ..TemplateListFilter::default()
            },
            Page::all(),
        );
        assert_eq!(ids(&out), vec!["T.v1.0"]);
    }

    #[test]
    fn unmatched_version_glob_yields_an_empty_list() {
        let out = filter_templates(
            rows(&["T.v1.0"]),
            &TemplateListFilter {
                version: Some("9.9.9".into()),
                ..TemplateListFilter::default()
            },
            Page::all(),
        );
        assert!(out.is_empty());
    }

    #[test]
    fn an_unversioned_id_survives_only_a_glob_admitting_the_empty_axis() {
        // A bare `*` means "all versions" (filter_version.yaml) and keeps
        // axis-less ids listed; any concrete glob has no axis to match.
        let all = rows(&["Plain", "T.v1.0"]);
        let star = filter_templates(
            all.clone(),
            &TemplateListFilter {
                version: Some("*".into()),
                ..TemplateListFilter::default()
            },
            Page::all(),
        );
        assert_eq!(ids(&star), vec!["Plain", "T.v1.0"]);
        let concrete = filter_templates(
            all,
            &TemplateListFilter {
                version: Some("1.*".into()),
                ..TemplateListFilter::default()
            },
            Page::all(),
        );
        assert_eq!(ids(&concrete), vec!["T.v1.0"]);
    }

    #[test]
    fn glob_matches_literally_with_star_wildcard() {
        // `*` is the only wildcard (filter_template_id.yaml); everything else
        // is literal — regex metacharacters must not leak through.
        assert!(glob_to_regex("*").is_match("anything at all"));
        assert!(glob_to_regex("IPS*").is_match("IPS v1"));
        assert!(!glob_to_regex("IPS*").is_match("not IPS"));
        assert!(glob_to_regex("a.b").is_match("a.b"));
        assert!(!glob_to_regex("a.b").is_match("aXb"));
        assert!(glob_to_regex("").is_match(""));
        assert!(!glob_to_regex("").is_match("x"));
    }
}
