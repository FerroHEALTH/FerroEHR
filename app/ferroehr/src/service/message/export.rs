// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! EHR-Extract export (SM `I_EHR_EXTRACT_SERVICE.export_ehrs` /
//! `export_ehr_extracts`) over the greenfield versioned store.
//!
//! Spec: SM `docs/specs/openehr/SM/docs/UML/classes/i_ehr_extract_service.adoc`
//! (`export_ehrs` / `export_ehr_extracts`); the RM EHR-Extract IM in
//! `docs/specs/openehr/RM/docs/ehr_extract/` — `master05-openehr_extract_package.adoc`
//! (`X_VERSIONED_OBJECT` and the five `X_VERSIONED_*` wrappers, §Demographic
//! Referencing) and `master09-semantics.adoc` §Creation Semantics (the
//! extract-building algorithm: primary set → `X_VERSIONED_COMPOSITION`,
//! demographic-reference resolution, `OBJECT_REF.namespace = "local"` rewrite,
//! multimedia include/exclude, `DV_LINK` following). The generated RM types are
//! in `crates/openehr-rm/src/ehr_extract/`.
//!
//! An exported `EXTRACT` is a canonical-JSON `Value` built directly over the
//! stored versions: each versioned object becomes one `OPENEHR_CONTENT_ITEM`
//! wrapping an `X_VERSIONED_<kind>` whose `versions` are the exact
//! `ORIGINAL_VERSION`s the read path serves ([`crate::versioning::wire::original_version`]),
//! so the extract content is byte-identical to what `GET .../versioned_*`
//! returns — then its content references are rewritten to the extract-local
//! namespace per the Creation-Semantics algorithm (see [`rewrite_content_refs`]).
//!
//! NOTE: the extract wrapper classes are `LOCATABLE`s whose
//! `archetype_node_id` is 1..1 (master05 class tables) yet are synthesized here
//! with no generating archetype, so the RM class token (`"EXTRACT"` and
//! siblings) is emitted as a self-descriptive placeholder rather than a
//! fabricated archetype id.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 3): EHR-Extract/TDD/dump-load compose over \
              verbatim stored content (RM common master06 §Copying)"
)]

use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

use crate::ids::{EhrId, VoId};
use crate::service::FerroEhrService;
use crate::service::error::ServiceError;
use crate::service::status::{CallStatusType, SmError};
use crate::system_log::event::EventActionCode;
use crate::versioning::read::{read_currents, read_version_by_ordinal};
use crate::versioning::wire::{original_version, revision_history, versioned_object};
use openehr_rm::v1_2::ehr_extract::common::extract_spec::ExtractSpec;

/// The extract-local reference namespace (`master09-semantics.adoc` §Creation
/// Semantics: "rewriting its `OBJECT_REFs` so that `namespace` = \"local\"").
const LOCAL_NS: &str = "local";

/// Which versions of a versioned object an extract includes, resolved from an
/// `EXTRACT_VERSION_SPEC` (`extract_version_spec.adoc`). The default (no spec)
/// is latest-only with data and no revision history.
#[derive(Debug, Clone, Copy)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "mirrors the four EXTRACT_VERSION_SPEC boolean flags 1:1; \
              collapsing them would diverge from the spec shape"
)]
struct VersionSelection {
    /// `EXTRACT_SPEC.include_multimedia` — when false, inline `DV_MULTIMEDIA`
    /// content (`data`) is stripped from exported version bodies
    /// (`master09-semantics.adoc` §Creation Semantics; the metadata + `uri`
    /// remain).
    include_multimedia: bool,
    /// `include_all_versions` — include every version, not just the latest.
    include_all: bool,
    /// `include_revision_history` — attach the full `REVISION_HISTORY`.
    include_revision_history: bool,
    /// `include_data` — include the version data; `false` ⇒ revision history
    /// only, `versions` empty (`X_VERSIONED_OBJECT.extract_version_count = 0`).
    include_data: bool,
}

impl VersionSelection {
    /// The `EXTRACT_VERSION_SPEC` default: latest-only, data included, no
    /// revision history (`extract_version_spec.adoc`: "By default, only latest
    /// versions are included").
    const fn latest_only() -> Self {
        Self {
            include_all: false,
            include_multimedia: true,
            include_revision_history: false,
            include_data: true,
        }
    }
}

/// The concrete RM versioned-object class per stored `kind` (RM ehr master04:
/// `VERSIONED_COMPOSITION` / `VERSIONED_EHR_STATUS` / `VERSIONED_FOLDER`).
///
/// EHR-scoped kinds only: the one caller resolves `kind` from an EHR-filtered
/// read (`ehr_versioned_objects` / `ehr_vo_kind`, both `WHERE ehr_id = $1`), and
/// demographic parties are not EHR-owned — the extract's demographics chapter
/// builds their `X_VERSIONED_PARTY` itself, with the system-scoped `owner_id` a
/// party container carries (`master09-semantics.adoc` §Creation Semantics:
/// "Create a demographics `EXTRACT_CHAPTER` and write the `PARTYs` in").
fn versioned_rm_type(kind: &str) -> &'static str {
    match kind {
        "COMPOSITION" => "VERSIONED_COMPOSITION",
        "EHR_STATUS" => "VERSIONED_EHR_STATUS",
        "FOLDER" => "VERSIONED_FOLDER",
        _ => "VERSIONED_OBJECT",
    }
}

/// The `X_VERSIONED_*` `_type` for a stored versioned-object `kind`
/// (`master05-openehr_extract_package.adoc`: the five data-oriented
/// `VERSIONED_OBJECT` wrappers). A kind with no dedicated wrapper (e.g.
/// `PARTY_RELATIONSHIP`, never EHR-scoped) falls back to the generic
/// `X_VERSIONED_OBJECT`.
fn x_versioned_type(kind: &str) -> &'static str {
    match kind {
        "COMPOSITION" => "X_VERSIONED_COMPOSITION",
        "EHR_STATUS" => "X_VERSIONED_EHR_STATUS",
        "EHR_ACCESS" => "X_VERSIONED_EHR_ACCESS",
        "FOLDER" => "X_VERSIONED_FOLDER",
        "AGENT" | "GROUP" | "ORGANISATION" | "PERSON" | "ROLE" => "X_VERSIONED_PARTY",
        _ => "X_VERSIONED_OBJECT",
    }
}

impl FerroEhrService {
    /// SM `export_ehrs(an_ehr_id)` — the whole-EHR, latest-only export as a
    /// single-element `List<EXTRACT>` (`i_ehr_extract_service.adoc`). A
    /// completed export is audited for non-repudiation (an outbound Extract
    /// communication → `EventActionCode::Read`).
    ///
    /// # Errors
    /// - `ehr_id_does_not_exist` — no EHR with `an_ehr_id` (`has_ehr` false).
    /// - `exception` — a database/codec fault while building the extract.
    pub async fn extract_ehrs(&self, an_ehr_id: EhrId) -> Result<Vec<Value>, SmError> {
        if !self.extract_ehr_exists(an_ehr_id).await? {
            return Err(SmError::ehr_not_found(format!(
                "no EHR with id {an_ehr_id}"
            )));
        }
        self.refuse_objected_export(an_ehr_id).await?;
        let extract = self.export_whole_ehr(an_ehr_id, 1).await?;
        self.emit_extract_audit(an_ehr_id, EventActionCode::Read)?;
        Ok(vec![extract])
    }

    /// SM `export_ehr_extracts(extract_spec)` — one `EXTRACT` per manifest
    /// entity, honouring `EXTRACT_VERSION_SPEC` + the item-list selector
    /// (`i_ehr_extract_service.adoc`; `master09-semantics.adoc` §Creation
    /// Semantics). One completed-export audit event is emitted per distinct
    /// exported EHR (outbound → `EventActionCode::Read`; non-repudiation).
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — an unsupported
    ///   `EXTRACT_VERSION_SPEC.commit_time_interval`, `include_data = false`
    ///   without `include_revision_history`, an `extract_type` outside the
    ///   openEHR extract-content-type group, `EXTRACT_SPEC.criteria` (AQL
    ///   selection — not supported this stage), a malformed
    ///   `item_list`/`ehr_id` value, or an entity naming neither `ehr_id` nor
    ///   `subject_id`.
    /// - `ehr_id_does_not_exist` — an entity's EHR/subject resolves to no EHR.
    /// - `versioned_object_does_not_exist` — an `item_list` version container
    ///   is not in the entity's EHR.
    /// - `exception` — a database/codec fault while building an extract.
    pub async fn export_ehr_extracts(
        &self,
        extract_spec: ExtractSpec,
    ) -> Result<Vec<Value>, SmError> {
        let sel = version_selection(&extract_spec)?;
        validate_extract_type(&extract_spec)?;
        let link_depth = extract_spec.link_depth;
        let criteria_present = !extract_spec.criteria.as_ref().is_none_or(Vec::is_empty);
        let spec_value = openehr_its::json::to_canonical_value(&extract_spec);

        let mut out = Vec::with_capacity(extract_spec.manifest.entities.len());
        let mut exported_ehrs: Vec<EhrId> =
            Vec::with_capacity(extract_spec.manifest.entities.len());
        for (idx, entity) in extract_spec.manifest.entities.iter().enumerate() {
            let ehr_id =
                resolve_entity_ehr(self, entity.ehr_id.as_deref(), entity.subject_id.as_deref())
                    .await?;

            self.refuse_objected_export(ehr_id).await?;

            let vo_kinds = self
                .entity_primary_set(ehr_id, entity, criteria_present, &extract_spec)
                .await?;

            let mut included: Vec<VoId> = vo_kinds.iter().map(|(vo, _)| *vo).collect();
            let mut items = Vec::with_capacity(vo_kinds.len());
            for (vo_id, kind) in vo_kinds {
                items.push(
                    self.build_openehr_content_item(ehr_id, vo_id, &kind, sel, true)
                        .await?,
                );
            }
            self.follow_links(ehr_id, &mut items, &mut included, sel, link_depth)
                .await?;
            let mut demographics = self.demographic_chapter_items(&items, sel).await?;
            rewrite_content_refs(&mut items);
            rewrite_content_refs(&mut demographics);
            let seq = i32::try_from(idx + 1).unwrap_or(i32::MAX);
            out.push(self.assemble_extract(items, &demographics, spec_value.clone(), seq));
            exported_ehrs.push(ehr_id);
        }
        exported_ehrs.sort_unstable();
        exported_ehrs.dedup();
        for ehr_id in exported_ehrs {
            self.emit_extract_audit(ehr_id, EventActionCode::Read)?;
        }
        Ok(out)
    }

    /// Follows `link_depth` levels of `DV_LINK` targets, appending each
    /// newly-reached versioned object as a non-primary content item.
    ///
    /// `EXTRACT_SPEC.link_depth`; `master09-semantics.adoc` §Creation
    /// Semantics: "for each instance of `DV_LINK` encountered … follow the
    /// links recursively … write the target Compositions in … set `is_primary`
    /// = False". Only same-EHR targets exist in this repository; a link
    /// outside it cannot be included.
    ///
    /// # Errors
    /// The content-item build errors, and storage errors from the kind lookup.
    async fn follow_links(
        &self,
        ehr_id: EhrId,
        items: &mut Vec<Value>,
        included: &mut Vec<VoId>,
        sel: VersionSelection,
        link_depth: i32,
    ) -> Result<(), SmError> {
        let mut depth = link_depth;
        while depth > 0 {
            let mut added = false;
            for target in link_target_uuids(items) {
                if included.contains(&target) {
                    continue;
                }
                let Some(kind) = self.ehr_vo_kind(ehr_id, target).await? else {
                    continue;
                };
                items.push(
                    self.build_openehr_content_item(ehr_id, target, &kind, sel, false)
                        .await?,
                );
                included.push(target);
                added = true;
            }
            if !added {
                break;
            }
            depth -= 1;
        }
        Ok(())
    }

    /// Whether an EHR with `ehr_id` exists (the `has_ehr` precondition of both
    /// export calls; `i_ehr_extract_service.adoc`).
    async fn extract_ehr_exists(&self, ehr_id: EhrId) -> Result<bool, ServiceError> {
        Ok(
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM ehr WHERE id = $1)")
                .bind(ehr_id)
                .fetch_one(&self.pool)
                .await?,
        )
    }

    /// The `(vo_id, kind)` of every versioned object currently in the EHR (one
    /// row per version container — its trunk head, `vo_head`), ordered by id
    /// for a deterministic extract.
    /// The primary set of one manifest entity: an explicit `item_list`; else
    /// the criteria queries (`master04-common_package.adoc` §`EXTRACT_SPEC`:
    /// criteria define "which items are to be retrieved from each entity's
    /// record"); else every VO of the EHR. `item_list` wins when both are
    /// given — master04 keeps the two mechanisms distinct ("only expected to
    /// be used when a specific identifier is known, rather than when items
    /// corresponding to filtering criteria are requested").
    ///
    /// # Errors
    /// The [`Self::criteria_primary_set`] refusals; `precondition_violation`
    /// for a malformed `item_list` entry; `versioned_object_does_not_exist`
    /// for an `item_list` container outside the entity's EHR.
    async fn entity_primary_set(
        &self,
        ehr_id: EhrId,
        entity: &openehr_rm::v1_2::ehr_extract::common::extract_entity_manifest::ExtractEntityManifest,
        criteria_present: bool,
        extract_spec: &ExtractSpec,
    ) -> Result<Vec<(VoId, String)>, SmError> {
        if entity.item_list.as_ref().is_none_or(Vec::is_empty) {
            if criteria_present {
                return self
                    .criteria_primary_set(
                        ehr_id,
                        extract_spec.criteria.as_deref().unwrap_or_default(),
                    )
                    .await;
            }
            return Ok(self.ehr_versioned_objects(ehr_id).await?);
        }
        let mut resolved = Vec::with_capacity(entity.item_list.as_ref().map_or(0, Vec::len));
        for obj_ref in entity.item_list.iter().flatten() {
            let value = openehr_its::json::to_canonical_value(obj_ref);
            let raw = value
                .pointer("/id/value")
                .and_then(Value::as_str)
                .ok_or_else(|| SmError::precondition("item_list OBJECT_REF has no id.value"))?;
            #[expect(
                clippy::map_err_ignore,
                reason = "the mapped error already echoes the rejected \
                      token; the discarded `uuid::Error` adds only \
                      its own wording, which is not part of the wire \
                      contract"
            )]
            let vo_id: VoId = raw.parse().map_err(|_| {
                SmError::precondition(format!(
                    "item_list id {raw:?} is not a version-container UUID"
                ))
            })?;
            let kind = self.ehr_vo_kind(ehr_id, vo_id).await?.ok_or_else(|| {
                SmError::new(
                    CallStatusType::VersionedObjectDoesNotExist,
                    format!("version container {vo_id} not found in EHR {ehr_id}"),
                )
            })?;
            resolved.push((vo_id, kind));
        }
        Ok(resolved)
    }

    /// The `$ehr`-bound criteria primary set: evaluate each
    /// `EXTRACT_SPEC.criteria` query against the entity's EHR and collect the
    /// version containers its result rows identify (RM `ehr_extract`
    /// `master04-common_package.adoc` §`EXTRACT_SPEC`: criteria "defines in the
    /// form of generic queries … which items are to be retrieved from each
    /// entity's record"; "Query expressions use variables such as $ehr to
    /// mean the current EHR from the `manifest` list").
    ///
    /// The released text names AQL as an example formalism and stops there, so
    /// the mechanics are our own design:
    /// - the formalism must be AQL (`DV_PARSABLE.formalism`, case-insensitive
    ///   `"aql"`), any other being a typed refusal rather than a silent skip;
    /// - the `$ehr` binding is realized twice, the engine call being scoped to
    ///   the entity's EHR and a literal `$ehr` parameter in the query text
    ///   binding to the EHR id;
    /// - a result row identifies a version container through any cell that is an
    ///   `OBJECT_VERSION_ID` or UID string or an object carrying `uid.value`,
    ///   and the union across rows and criteria in first-seen order is the
    ///   primary set, possibly empty;
    /// - the engine's SM population gate applies, so a non-queryable EHR yields
    ///   no rows (SM `i_query_service.adoc`); no openEHR spec relates EXTRACT
    ///   criteria to that gate.
    ///
    /// # Errors
    /// `precondition_violation` — a non-AQL formalism, or a criterion that
    /// does not parse/execute as AQL (the refusal names the criterion index).
    async fn criteria_primary_set(
        &self,
        ehr_id: EhrId,
        criteria: &[openehr_rm::v1_2::data_types::encapsulated::dv_parsable::DvParsable],
    ) -> Result<Vec<(VoId, String)>, SmError> {
        let mut out: Vec<(VoId, String)> = Vec::new();
        for (idx, criterion) in criteria.iter().enumerate() {
            if !criterion.formalism.eq_ignore_ascii_case("aql") {
                return Err(SmError::precondition(format!(
                    "EXTRACT_SPEC.criteria[{idx}] formalism {:?} is not supported: this \
                     service evaluates AQL criteria (master04-common_package.adoc \
                     names AQL as the openEHR query formalism)",
                    criterion.formalism
                )));
            }
            let request = crate::service::query::request::AqlQueryRequest {
                ehr_ids: vec![ehr_id.to_string()],
                parameters: std::iter::once(("ehr".to_owned(), json!(ehr_id.to_string())))
                    .collect(),
                ..crate::service::query::request::AqlQueryRequest::default()
            };
            let outcome = self
                .execute_ad_hoc_query(criterion.value.clone(), request)
                .await
                .map_err(|e| {
                    SmError::precondition(format!(
                        "EXTRACT_SPEC.criteria[{idx}] did not evaluate as AQL: {}",
                        e.message
                    ))
                })?;
            let empty = Vec::new();
            let rows = outcome
                .result_set
                .get("rows")
                .and_then(Value::as_array)
                .unwrap_or(&empty);
            for cell in rows.iter().filter_map(Value::as_array).flatten() {
                let candidate = match cell {
                    Value::String(s) => Some(s.as_str()),
                    Value::Object(_) => cell.pointer("/uid/value").and_then(Value::as_str),
                    _ => None,
                };
                let Some(raw) = candidate else { continue };
                let Ok(vo_id) = raw.split("::").next().unwrap_or(raw).parse::<VoId>() else {
                    continue;
                };
                if out.iter().any(|(v, _)| *v == vo_id) {
                    continue;
                }
                if let Some(kind) = self.ehr_vo_kind(ehr_id, vo_id).await? {
                    out.push((vo_id, kind));
                }
            }
        }
        Ok(out)
    }

    /// Refuse an Extract of an EHR whose subject objected to research
    /// processing, and of an EHR restricted as a whole.
    ///
    /// An Extract leaves this repository for another system, which is the
    /// surface a secondary-use consumer reads the record through, so both marks
    /// bite here. The refusal is loud rather than an empty Extract: a caller
    /// handed an Extract with nothing in it would record an absence of data
    /// where there is data the server declines to disclose. GDPR Art. 18(2) and
    /// Art. 21(6) (`docs/law/eu/gdpr/text.html`); no openEHR spec governs
    /// either mark — our own design/extension.
    ///
    /// # Errors
    /// [`ServiceError::Restricted`] (`403`) when either mark stands; the
    /// storage read error of the mark read.
    async fn refuse_objected_export(&self, ehr_id: EhrId) -> Result<(), ServiceError> {
        let Some(marks) = crate::storage::marks::ehr_marks(&self.pool, ehr_id).await? else {
            return Ok(());
        };
        if marks.restricted {
            return Err(ServiceError::restricted(&format!("EHR {ehr_id}")));
        }
        if marks.research_objected {
            return Err(ServiceError::research_objected(&format!("EHR {ehr_id}")));
        }
        Ok(())
    }

    /// Every version container of an EHR, restricted objects omitted.
    ///
    /// An Extract is a disclosure of the record to another system, which
    /// GDPR Art. 18(2) does not admit for a restricted object
    /// (`docs/law/eu/gdpr/text.html`): storage is the only processing left. The
    /// object is therefore absent from the Extract rather than failing the
    /// whole export, the same treatment the AQL result set gives it. No openEHR
    /// spec governs restriction of processing — our own design/extension.
    async fn ehr_versioned_objects(
        &self,
        ehr_id: EhrId,
    ) -> Result<Vec<(VoId, String)>, ServiceError> {
        let rows = sqlx::query(
            "SELECT version.vo_id, version.kind FROM version \
             JOIN vo_head h ON h.vo_id = version.vo_id AND h.trunk_head_sys_version = version.sys_version \
             WHERE version.ehr_id = $1 AND h.restricted_at IS NULL \
             ORDER BY version.vo_id",
        )
        .bind(ehr_id)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            out.push((r.try_get("vo_id")?, r.try_get("kind")?));
        }
        Ok(out)
    }

    /// The kind of a version container, if it belongs to `ehr_id` (resolves an
    /// `EXTRACT_ENTITY_MANIFEST.item_list` reference).
    async fn ehr_vo_kind(
        &self,
        ehr_id: EhrId,
        vo_id: VoId,
    ) -> Result<Option<String>, ServiceError> {
        Ok(sqlx::query_scalar(
            "SELECT version.kind FROM version \
             JOIN vo_head h ON h.vo_id = version.vo_id AND h.trunk_head_sys_version = version.sys_version \
             WHERE version.vo_id = $1 AND version.ehr_id = $2 AND h.restricted_at IS NULL",
        )
        .bind(vo_id)
        .bind(ehr_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// The `sys_version`s of a version container, in order, each flagged with
    /// whether it is the current trunk version (`vo_head.trunk_head_sys_version`).
    async fn vo_version_numbers(&self, vo_id: VoId) -> Result<Vec<(i32, bool)>, ServiceError> {
        let rows = sqlx::query(
            "SELECT sys_version, EXISTS(SELECT 1 FROM vo_head h \
                 WHERE h.vo_id = version.vo_id \
                   AND h.trunk_head_sys_version = version.sys_version) AS is_current \
             FROM version WHERE vo_id = $1 ORDER BY sys_version",
        )
        .bind(vo_id)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            out.push((r.try_get("sys_version")?, r.try_get("is_current")?));
        }
        Ok(out)
    }

    /// The stored-version counts of a set of parties, in ONE statement — the
    /// demographics chapter's `total_version_count` source (never a count round
    /// trip per party). Reads the demographic domain, where parties live.
    async fn vo_version_counts(
        &self,
        vo_ids: &[VoId],
    ) -> Result<std::collections::HashMap<VoId, i64>, ServiceError> {
        if vo_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let ids: Vec<Uuid> = vo_ids.iter().map(|v| v.0).collect();
        let rows = sqlx::query(
            "SELECT vo_id, count(*) AS n FROM version WHERE vo_id = ANY($1) GROUP BY vo_id",
        )
        .bind(&ids)
        .fetch_all(&self.demographic_pool)
        .await?;
        let mut out = std::collections::HashMap::with_capacity(rows.len());
        for r in rows {
            out.insert(r.try_get::<VoId, _>("vo_id")?, r.try_get::<i64, _>("n")?);
        }
        Ok(out)
    }

    /// Build one `OPENEHR_CONTENT_ITEM` wrapping the `X_VERSIONED_<kind>` for a
    /// version container (`master05-openehr_extract_package.adoc`). `versions`
    /// are the exact `ORIGINAL_VERSION`s the read path serves;
    /// `total_version_count` counts all stored versions, `extract_version_count`
    /// the included ones.
    async fn build_openehr_content_item(
        &self,
        ehr_id: EhrId,
        vo_id: VoId,
        kind: &str,
        sel: VersionSelection,
        is_primary: bool,
    ) -> Result<Value, ServiceError> {
        let all = self.vo_version_numbers(vo_id).await?;
        let total = i32::try_from(all.len()).unwrap_or(i32::MAX);

        // Which versions to serialise: none if data is excluded, else all or the
        // latest only (EXTRACT_VERSION_SPEC).
        let selected: Vec<i32> = if !sel.include_data {
            Vec::new()
        } else if sel.include_all {
            all.iter().map(|(sv, _)| *sv).collect()
        } else {
            all.iter()
                .filter(|(_, cur)| *cur)
                .map(|(sv, _)| *sv)
                .collect()
        };

        let mut versions = Vec::with_capacity(selected.len());
        for sv in &selected {
            let read = read_version_by_ordinal(&self.pool, self.spec_profile, vo_id, *sv)
                .await?
                .ok_or_else(|| {
                    ServiceError::sm(
                        CallStatusType::ObjectVersionDoesNotExist,
                        format!("version {vo_id}::{sv} for extract"),
                    )
                })?;
            let mut version = original_version(&read, self.signer())?;
            if !sel.include_multimedia {
                strip_inline_multimedia(&mut version);
            }
            versions.push(version);
        }
        let extract_version_count = i32::try_from(versions.len()).unwrap_or(i32::MAX);

        // uid / owner_id / time_created are the VERSIONED_OBJECT's own — reuse
        // the shared read builder so they match the /versioned_* surface.
        let (vo, _) = versioned_object(&self.pool, vo_id, ehr_id, versioned_rm_type(kind))
            .await?
            .ok_or_else(|| {
                ServiceError::sm(
                    CallStatusType::VersionedObjectDoesNotExist,
                    format!("versioned object {vo_id}"),
                )
            })?;
        let field = |name: &str| vo.get(name).cloned().unwrap_or(Value::Null);
        // TODO(#1695): build the EXTRACT tree from the generated
        // `openehr_rm::v1_2::ehr_extract` types (`Extract`, `ExtractChapter`,
        // `XVersionedObject`, `OpenehrContentItem`) instead of these JSON
        // literals. Blocked on the composition shape, not a missing type: every
        // branch here splices ALREADY-CANONICAL opaque fragments, so a typed
        // build would first have to decode each of them back into RM values.
        let mut x = json!({
            "_type": x_versioned_type(kind),
            "uid": field("uid"),
            "owner_id": field("owner_id"),
            "time_created": field("time_created"),
            "total_version_count": total,
            "extract_version_count": extract_version_count,
            "versions": versions,
        });
        if sel.include_revision_history
            && let Value::Object(map) = &mut x
        {
            map.insert(
                "revision_history".to_owned(),
                revision_history(&self.pool, ehr_id, vo_id).await?.0,
            );
        }

        Ok(json!({
            "_type": "OPENEHR_CONTENT_ITEM",
            "name": { "_type": "DV_TEXT", "value": kind },
            "archetype_node_id": "OPENEHR_CONTENT_ITEM",
            "is_primary": is_primary,
            "item": x,
        }))
    }

    /// The demographics-chapter items: every locally-held demographic PARTY
    /// referenced from the exported content via a `PARTY_REF` with namespace
    /// `demographic` becomes an `X_VERSIONED_PARTY` content item
    /// (`is_primary = false`) — `master09-semantics.adoc` §Creation Semantics
    /// ("Create a demographics `EXTRACT_CHAPTER` and write the `PARTYs` in";
    /// "obtain the target of the reference from the relevant service, and copy
    /// it to the … demographics … chapter"). Parties not held locally cannot be
    /// written in and are skipped. Collected from the ORIGINAL references,
    /// *before* the content refs are rewritten to `"local"`
    /// (`master09-semantics.adoc` orders reference resolution ahead of the
    /// namespace rewrite).
    async fn demographic_chapter_items(
        &self,
        content_items: &[Value],
        sel: VersionSelection,
    ) -> Result<Vec<Value>, ServiceError> {
        let mut party_ids = Vec::new();
        for item in content_items {
            collect_party_ids(item, &mut party_ids);
        }
        // One statement resolves every referenced party, one more counts their
        // stored versions — never a round-trip pair per party.
        let mut currents =
            read_currents(&self.demographic_pool, self.spec_profile, &party_ids).await?;
        let totals = self.vo_version_counts(&party_ids).await?;
        let mut out = Vec::new();
        for vo_id in party_ids {
            let Some(read) = currents.remove(&vo_id) else {
                continue; // not held locally — cannot be written in
            };
            if read.ehr_id.is_some() {
                continue; // an EHR-owned object, not a demographic party
            }
            let mut version = original_version(&read, self.signer())?;
            if !sel.include_multimedia {
                strip_inline_multimedia(&mut version);
            }
            let total = totals.get(&vo_id).copied().unwrap_or(0);
            out.push(json!({
                "_type": "OPENEHR_CONTENT_ITEM",
                "name": { "_type": "DV_TEXT", "value": "PARTY" },
                "archetype_node_id": "OPENEHR_CONTENT_ITEM",
                "is_primary": false,
                "item": {
                    "_type": "X_VERSIONED_PARTY",
                    "uid": { "_type": "HIER_OBJECT_ID", "value": vo_id.to_string() },
                    // RM demographic content stands alone, so the serving system
                    // is the owner — the shape the VERSIONED_PARTY container read
                    // serves, per the released `VersionedParty` example's
                    // `OBJECT_REF` in the ITS-REST demographic OAS.
                    "owner_id": {
                        "_type": "OBJECT_REF",
                        "namespace": "local",
                        "type": "SYSTEM",
                        "id": { "_type": "HIER_OBJECT_ID", "value": self.effective_system_id() }
                    },
                    "time_created": version
                        .get("commit_audit")
                        .and_then(|audit| audit.get("time_committed"))
                        .cloned()
                        .unwrap_or(Value::Null),
                    "total_version_count": i32::try_from(total).unwrap_or(i32::MAX),
                    "extract_version_count": 1,
                    "versions": [version],
                },
            }));
        }
        Ok(out)
    }

    /// Assemble the top-level `EXTRACT` from its (already namespace-rewritten)
    /// content items and the `EXTRACT_SPEC` that reflects the actual content
    /// (`extract.adoc`: the `specification` "might not be identical with the
    /// specification of the corresponding request").
    fn assemble_extract(
        &self,
        content_items: Vec<Value>,
        demographic_items: &[Value],
        specification: Value,
        sequence_nr: i32,
    ) -> Value {
        let mut extract = json!({
            "_type": "EXTRACT",
            "name": { "_type": "DV_TEXT", "value": "EHR Extract" },
            "archetype_node_id": "EXTRACT",
            "chapters": [ {
                "_type": "EXTRACT_CHAPTER",
                "name": { "_type": "DV_TEXT", "value": "openEHR content" },
                "archetype_node_id": "EXTRACT_CHAPTER",
                "items": [],
            } ],
            "time_created": {
                "_type": "DV_DATE_TIME",
                "value": jiff::Timestamp::now().to_string(),
            },
            "system_id": { "_type": "HIER_OBJECT_ID", "value": self.effective_system_id() },
            "sequence_nr": sequence_nr,
        });
        // The literal above created `chapters` with exactly one EXTRACT_CHAPTER
        // (the openEHR content chapter); fill its `items` through the map rather
        // than a panicking index-assign.
        if let Some(chapter) = extract
            .get_mut("chapters")
            .and_then(Value::as_array_mut)
            .and_then(|chapters| chapters.first_mut())
            .and_then(Value::as_object_mut)
        {
            chapter.insert("items".to_owned(), Value::Array(content_items));
        }
        // A demographics chapter carries the locally-held PARTYs referenced by
        // the content (`master09-semantics.adoc` §Creation Semantics: "Create a
        // demographics `EXTRACT_CHAPTER` and write the `PARTYs` in"); omitted
        // when nothing local is referenced.
        if !demographic_items.is_empty()
            && let Some(chapters) = extract.get_mut("chapters").and_then(Value::as_array_mut)
        {
            chapters.push(json!({
                "_type": "EXTRACT_CHAPTER",
                "name": { "_type": "DV_TEXT", "value": "demographics" },
                "archetype_node_id": "EXTRACT_CHAPTER",
                "items": demographic_items,
            }));
        }
        if let Some(obj) = extract.as_object_mut() {
            obj.insert("specification".to_owned(), specification);
        }
        extract
    }

    /// A synthetic `EXTRACT_SPEC` describing a whole-EHR, latest-only export (for
    /// `export_ehrs`, which takes no spec) — one entity keyed by the EHR id.
    fn whole_ehr_spec(ehr_id: EhrId) -> Value {
        json!({
            "_type": "EXTRACT_SPEC",
            "version_spec": {
                "_type": "EXTRACT_VERSION_SPEC",
                "include_all_versions": false,
                "include_revision_history": false,
                "include_data": true,
            },
            "manifest": {
                "_type": "EXTRACT_MANIFEST",
                "entities": [ {
                    "_type": "EXTRACT_ENTITY_MANIFEST",
                    "extract_id_key": ehr_id.to_string(),
                    "ehr_id": ehr_id.to_string(),
                    "other_ids": [],
                    "item_list": [],
                } ],
            },
            "extract_type": {
                "_type": "DV_CODED_TEXT",
                "value": "openehr-ehr",
                "defining_code": {
                    "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                    "code_string": "openehr-ehr",
                },
            },
            "include_multimedia": true,
            "priority": 0,
            "link_depth": 0,
            "criteria": [],
        })
    }

    /// Build a single whole-EHR `EXTRACT` (every versioned object of the EHR,
    /// latest version only) — the body of `export_ehrs`. The Creation-Semantics
    /// order is preserved: build content, resolve demographic references, then
    /// rewrite content refs to the extract-local namespace.
    async fn export_whole_ehr(
        &self,
        ehr_id: EhrId,
        sequence_nr: i32,
    ) -> Result<Value, ServiceError> {
        let sel = VersionSelection::latest_only();
        let vos = self.ehr_versioned_objects(ehr_id).await?;
        let mut items = Vec::with_capacity(vos.len());
        for (vo_id, kind) in vos {
            items.push(
                self.build_openehr_content_item(ehr_id, vo_id, &kind, sel, true)
                    .await?,
            );
        }
        let mut demographics = self.demographic_chapter_items(&items, sel).await?;
        rewrite_content_refs(&mut items);
        rewrite_content_refs(&mut demographics);
        Ok(self.assemble_extract(
            items,
            &demographics,
            Self::whole_ehr_spec(ehr_id),
            sequence_nr,
        ))
    }
}

/// Read the `EXTRACT_VERSION_SPEC` of a request into a [`VersionSelection`],
/// rejecting the not-yet-supported `commit_time_interval` selector and enforcing
/// the invariant that excluding data requires including revision history
/// (`extract_version_spec.adoc` `Includes_revision_history_valid`).
fn version_selection(spec: &ExtractSpec) -> Result<VersionSelection, SmError> {
    let Some(vs) = spec.version_spec.as_ref() else {
        return Ok(VersionSelection::latest_only());
    };
    // NOTE: commit-time-window version selection
    // (`extract_version_spec.adoc` `EXTRACT_VERSION_SPEC.commit_time_interval`)
    // is a typed reject, never a silent full export.
    if vs.commit_time_interval.is_some() {
        return Err(SmError::precondition(
            "EXTRACT_VERSION_SPEC.commit_time_interval is not supported in this stage",
        ));
    }
    // include_data = false ⇒ revision-history-only; the spec requires the
    // revision history to then be present.
    if !vs.include_data && !vs.include_revision_history {
        return Err(SmError::precondition(
            "include_data = false requires include_revision_history = true \
             (EXTRACT_VERSION_SPEC.Includes_revision_history_valid: data \
             excluded ⇒ revision-history-only)",
        ));
    }
    Ok(VersionSelection {
        include_multimedia: spec.include_multimedia,
        include_all: vs.include_all_versions,
        include_revision_history: vs.include_revision_history,
        include_data: vs.include_data,
    })
}

/// Recursively rewrite every `OBJECT_REF`-family reference in a version body so
/// `namespace = "local"` (`master09-semantics.adoc` §Creation Semantics:
/// "copy/serialise the Composition … rewriting its `OBJECT_REFs` so that
/// `namespace` = \"local\""). Within an extract every reference is local to the
/// extract, so its namespace is normalised regardless of the source namespace
/// (`demographic`, a remote system id, etc.). The `OBJECT_REF` subtypes carrying
/// a `namespace` are `OBJECT_REF`, `PARTY_REF`, `LOCATABLE_REF` and
/// `ACCESS_GROUP_REF` (BASE `base_types` `master05-identification_package.adoc`
/// §References).
fn rewrite_object_refs_local(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let is_ref = matches!(
                map.get("_type").and_then(Value::as_str),
                Some("OBJECT_REF" | "PARTY_REF" | "LOCATABLE_REF" | "ACCESS_GROUP_REF")
            );
            if is_ref && map.contains_key("namespace") {
                map.insert("namespace".to_owned(), Value::String(LOCAL_NS.to_owned()));
            }
            for v in map.values_mut() {
                rewrite_object_refs_local(v);
            }
        }
        Value::Array(items) => {
            for v in items {
                rewrite_object_refs_local(v);
            }
        }
        _ => {}
    }
}

/// Apply [`rewrite_object_refs_local`] to the version bodies of a chapter's
/// content items (`OPENEHR_CONTENT_ITEM.item.versions.as_deref().unwrap_or_default()[].data`) — the "copied /
/// serialised" content the Creation-Semantics algorithm rewrites. The
/// `X_VERSIONED_*` wrapper metadata (`uid` / `owner_id` / `contribution`) is
/// left as built.
fn rewrite_content_refs(content_items: &mut [Value]) {
    for item in content_items {
        if let Some(versions) = item
            .pointer_mut("/item/versions")
            .and_then(Value::as_array_mut)
        {
            for version in versions {
                if let Some(data) = version.get_mut("data") {
                    rewrite_object_refs_local(data);
                }
            }
        }
    }
}

/// Strip inline `DV_MULTIMEDIA` content (`data`) from a version body when
/// `EXTRACT_SPEC.include_multimedia = false` (`master09-semantics.adoc`
/// §Creation Semantics); the multimedia metadata and any `uri` reference remain.
fn strip_inline_multimedia(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if map.get("_type").and_then(Value::as_str) == Some("DV_MULTIMEDIA") {
                map.remove("data");
            }
            for v in map.values_mut() {
                strip_inline_multimedia(v);
            }
        }
        Value::Array(items) => {
            for v in items {
                strip_inline_multimedia(v);
            }
        }
        _ => {}
    }
}

/// Every UUID mentioned in a `LOCATABLE.links[].target` URI across the built
/// content items — the candidate same-EHR link targets for `link_depth`
/// following (`DV_EHR_URI` values carry the container/version uids).
fn link_target_uuids(items: &[Value]) -> Vec<VoId> {
    let mut out = Vec::new();
    for item in items {
        collect_link_targets(item, &mut out);
    }
    out
}

/// Walks one built content item, appending every distinct versioned-object
/// UUID a `PARTY_REF` in the `demographic` namespace names.
fn collect_party_ids(value: &Value, out: &mut Vec<VoId>) {
    match value {
        Value::Object(map) => {
            if map.get("_type").and_then(Value::as_str) == Some("PARTY_REF")
                && map.get("namespace").and_then(Value::as_str) == Some("demographic")
                && let Some(raw) = value.pointer("/id/value").and_then(Value::as_str)
                && let Ok(uuid) = raw.parse::<Uuid>()
                && !out.contains(&VoId(uuid))
            {
                out.push(VoId(uuid));
            }
            for v in map.values() {
                collect_party_ids(v, out);
            }
        }
        Value::Array(list) => {
            for v in list {
                collect_party_ids(v, out);
            }
        }
        _ => {}
    }
}

/// Walks one built content item, appending every distinct versioned-object
/// UUID its `LOCATABLE.links[].target` URIs mention.
fn collect_link_targets(value: &Value, out: &mut Vec<VoId>) {
    match value {
        Value::Object(map) => {
            if let Some(links) = map.get("links").and_then(Value::as_array) {
                for link in links {
                    push_link_target_uuids(link, out);
                }
            }
            for v in map.values() {
                collect_link_targets(v, out);
            }
        }
        Value::Array(list) => {
            for v in list {
                collect_link_targets(v, out);
            }
        }
        _ => {}
    }
}

/// Appends the versioned-object UUIDs one `DV_LINK` target URI names.
fn push_link_target_uuids(link: &Value, out: &mut Vec<VoId>) {
    let Some(uri) = link.pointer("/target/value").and_then(Value::as_str) else {
        return;
    };
    for token in uri.split(|c: char| !c.is_ascii_hexdigit() && c != '-') {
        // A DV_LINK target uri names a versioned object.
        if let Ok(uuid) = token.parse::<Uuid>()
            && !out.contains(&VoId(uuid))
        {
            out.push(VoId(uuid));
        }
    }
}

/// The RM-named string tokens `EXTRACT_SPEC.extract_type` may carry, beside
/// the TERM `extract_content_type` vocabulary codes.
///
/// The first five are the ones the RM names outright — RM `ehr_extract`
/// `master04-common_package.adoc` §Content Criteria Specification:
/// "`_extract_type_`: what kind of Extract this is, e.g. `|openehr-ehr|`,
/// `|openehr-demographic|`, `|openehr-synchronisation|`, `|openehr-generic|`,
/// `|generic-emr|`, etc". TERM `SupportTerminology/master03-terminology.adoc`
/// §Vocabularies additionally binds the attribute to the
/// `extract_content_type` group (concepts 803 "openEHR EHR" … 808 "other"),
/// validated against the bundle in [`validate_extract_type`] — the two value
/// spaces are never reconciled upstream, so both are accepted.
///
/// NOTE: the RM's "etc" (`master04-common_package.adoc` §Content Criteria
/// Specification) makes its list illustrative, and this service still validates,
/// an unrecognized type silently exporting an openEHR-EHR extract being a
/// payload that misdescribes itself; the accepted set is a superset of every
/// code the RM names and of the whole TERM group.
const EXTRACT_CONTENT_TYPES: [&str; 6] = [
    "openehr-ehr",
    "openehr-demographic",
    "openehr-synchronisation",
    "openehr-generic",
    "generic-emr",
    "other",
];

/// `EXTRACT_SPEC.extract_type` must be coded from the TERM
/// `extract_content_type` vocabulary (openEHR terminology, concepts 803–808 —
/// TERM `SupportTerminology/master03-terminology.adoc` §Vocabularies) or from
/// the RM-named tokens in [`EXTRACT_CONTENT_TYPES`].
fn validate_extract_type(spec: &ExtractSpec) -> Result<(), SmError> {
    let value = openehr_its::json::to_canonical_value(&spec.extract_type);
    let code = value
        .pointer("/defining_code/code_string")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let terminology = value
        .pointer("/defining_code/terminology_id/value")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if terminology == "openehr"
        && openehr_term::bundle::openehr().is_valid_code("extract_content_type", code)
    {
        return Ok(());
    }
    if EXTRACT_CONTENT_TYPES.contains(&code) {
        return Ok(());
    }
    Err(SmError::precondition(format!(
        "EXTRACT_SPEC.extract_type {code:?} is neither a member of the openEHR \
         extract_content_type vocabulary nor an RM-named extract content type \
         ({})",
        EXTRACT_CONTENT_TYPES.join(" | ")
    )))
}

/// Resolve an `EXTRACT_ENTITY_MANIFEST` to a concrete EHR id: prefer `ehr_id`,
/// else look the EHR up by `subject_id` (`master04-common_package.adoc`
/// `EXTRACT_ENTITY_MANIFEST`).
#[expect(
    clippy::map_err_ignore,
    reason = "the mapped error already names the resource and echoes the \
              rejected token; the discarded `uuid::Error` adds only its own \
              wording, which is not part of the wire contract"
)]
async fn resolve_entity_ehr(
    svc: &FerroEhrService,
    ehr_id: Option<&str>,
    subject_id: Option<&str>,
) -> Result<EhrId, SmError> {
    if let Some(raw) = ehr_id {
        let id: EhrId = raw.parse().map_err(|_| {
            SmError::precondition(format!(
                "EXTRACT_ENTITY_MANIFEST.ehr_id {raw:?} is not a UUID"
            ))
        })?;
        if svc.extract_ehr_exists(id).await? {
            return Ok(id);
        }
        return Err(SmError::ehr_not_found(format!("no EHR with id {id}")));
    }
    if let Some(subject) = subject_id {
        let id: Option<EhrId> = sqlx::query_scalar("SELECT id FROM ehr WHERE subject_id = $1")
            .bind(subject)
            .fetch_optional(&svc.pool)
            .await
            .map_err(ServiceError::from)?;
        return id
            .ok_or_else(|| SmError::ehr_not_found(format!("no EHR for subject_id {subject:?}")));
    }
    Err(SmError::precondition(
        "EXTRACT_ENTITY_MANIFEST identifies no entity (neither ehr_id nor subject_id)",
    ))
}
