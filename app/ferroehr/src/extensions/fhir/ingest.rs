// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The FHIR inbound-connector service glue: mapping store CRUD, inbound
//! ingest onto the EHR index + commit seam, and the read facade — the
//! platform-coupled half of the FHIR extension (the pure conversion core is
//! `ferroehr_ext::fhir`).

use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

use crate::ids::{EhrId, VoId};
use crate::service::FerroEhrService;
use crate::service::ehr_index::types::SubjectRef;
use crate::service::error::{ServiceError, Violation, internal_fault};
use crate::service::query::request::AqlQueryRequest;
use crate::service::response::ServiceResponse;
use crate::service::status::{CallStatusType, SmError};

use ferroehr_ext::fhir::mapping::FhirMappingDefinition;

/// The read-façade AQL: the version uid of every COMPOSITION of the mapped
/// template in scope. The template id binds as a parameter (no string
/// interpolation → no AQL injection).
///
/// NOTE: the engine-synthesized VERSION uid is selected (`v/uid/value`,
/// `<vo_id>::<system>::<ver>`) because a COMPOSITION variable's own
/// `c/uid/value` is a null RM leaf on the AQL read path — the reassembled body
/// carries no uid. The body is then loaded by that uid through the gated
/// versioned read seam ([`crate::versioning::read::read_version_by_ordinal`]),
/// the same seam the outbound emitter uses.
const FHIR_SEARCH_AQL: &str = "SELECT v/uid/value FROM EHR e \
     CONTAINS VERSION v CONTAINS COMPOSITION c \
     WHERE c/archetype_details/template_id/value = $templateId";

/// The read-façade query scope resolved from the `patient` parameter.
struct PatientScope {
    /// The EHR id to scope to when `patient` is a UUID.
    ehr_id: Option<Uuid>,
    /// The subject external id to scope to when `patient` is not a UUID.
    subject_scope: Option<String>,
    /// The subject external id to reconstruct each resource's
    /// `subject.reference` from (the looked-up EHR subject, or the param).
    subject_id: Option<String>,
}

impl FerroEhrService {
    /// Map a `fhir_mapping` row to its JSON record.
    fn mapping_row(row: &sqlx::postgres::PgRow) -> Result<Value, ServiceError> {
        let id: Uuid = row.try_get("id")?;
        let name: String = row.try_get("name")?;
        let resource_type: String = row.try_get("resource_type")?;
        let profile_url: Option<String> = row.try_get("profile_url")?;
        let template_id: String = row.try_get("template_id")?;
        let definition: Value = row.try_get("definition")?;
        let enabled: bool = row.try_get("enabled")?;
        let created_at = row
            .try_get::<jiff_sqlx::Timestamp, _>("created_at")?
            .to_jiff();
        Ok(json!({
            "id": id.to_string(),
            "name": name,
            "resource_type": resource_type,
            "profile_url": profile_url,
            "template_id": template_id,
            "definition": definition,
            "enabled": enabled,
            "created_at": created_at.to_string(),
        }))
    }

    /// List every stored mapping (newest first).
    async fn list_mappings(&self) -> Result<Vec<Value>, ServiceError> {
        let rows = sqlx::query(
            "SELECT id, name, resource_type, profile_url, template_id, definition, enabled, \
             created_at FROM fhir_mapping ORDER BY created_at DESC, id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(Self::mapping_row).collect()
    }

    /// Fetch one mapping by id, or `NotFound`.
    async fn get_mapping(&self, id: Uuid) -> Result<Value, ServiceError> {
        let row = sqlx::query(
            "SELECT id, name, resource_type, profile_url, template_id, definition, enabled, \
             created_at FROM fhir_mapping WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            ServiceError::sm(
                CallStatusType::VersionedObjectDoesNotExist,
                format!("FHIR mapping {id}"),
            )
        })?;
        Self::mapping_row(&row)
    }

    /// Create a mapping from a JSON body: `{name, enabled?, definition}` where
    /// `definition` is the [`FhirMappingDefinition`] (validated on upload, its
    /// `resource_type`/`profile_url`/`template_id` projected into columns). A
    /// duplicate name is a `Conflict`; an unknown `template_id` (FK) is a
    /// `BadRequest`.
    async fn create_mapping(&self, body: &Value) -> Result<Value, ServiceError> {
        let name = validated_name(body)?;
        let (definition, def) = validated_definition(body)?;
        let enabled = body.get("enabled").and_then(Value::as_bool).unwrap_or(true);
        let row = sqlx::query(
            "INSERT INTO fhir_mapping \
             (name, resource_type, profile_url, template_id, definition, enabled) \
             VALUES ($1, $2, $3, $4, $5, $6) \
             RETURNING id, name, resource_type, profile_url, template_id, definition, enabled, \
             created_at",
        )
        .bind(&name)
        .bind(&def.resource_type)
        .bind(def.profile_url.as_deref())
        .bind(&def.template_id)
        .bind(&definition)
        .bind(enabled)
        .fetch_one(&self.pool)
        .await
        .map_err(map_insert_error)?;
        Self::mapping_row(&row)
    }

    /// Replace a mapping's fields (name is immutable — it is the stable
    /// deployable identity). `NotFound` if the id is unknown.
    async fn update_mapping(&self, id: Uuid, body: &Value) -> Result<Value, ServiceError> {
        let (definition, def) = validated_definition(body)?;
        let enabled = body.get("enabled").and_then(Value::as_bool).unwrap_or(true);
        let row = sqlx::query(
            "UPDATE fhir_mapping \
             SET resource_type = $2, profile_url = $3, template_id = $4, definition = $5, \
             enabled = $6 WHERE id = $1 \
             RETURNING id, name, resource_type, profile_url, template_id, definition, enabled, \
             created_at",
        )
        .bind(id)
        .bind(&def.resource_type)
        .bind(def.profile_url.as_deref())
        .bind(&def.template_id)
        .bind(&definition)
        .bind(enabled)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_insert_error)?
        .ok_or_else(|| {
            ServiceError::sm(
                CallStatusType::VersionedObjectDoesNotExist,
                format!("FHIR mapping {id}"),
            )
        })?;
        Self::mapping_row(&row)
    }

    /// Delete a mapping by id. `NotFound` if the id is unknown.
    async fn delete_mapping(&self, id: Uuid) -> Result<(), ServiceError> {
        let deleted = sqlx::query("DELETE FROM fhir_mapping WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?
            .rows_affected();
        if deleted == 0 {
            return Err(ServiceError::sm(
                CallStatusType::VersionedObjectDoesNotExist,
                format!("FHIR mapping {id}"),
            ));
        }
        Ok(())
    }

    /// Resolve the enabled mapping definition for a resource type + optional
    /// profile: prefer an exact `profile_url` match, else the NULL-profile
    /// default. `None` when no enabled mapping matches.
    async fn resolve_mapping(
        &self,
        resource_type: &str,
        profile: Option<&str>,
    ) -> Result<Option<Value>, ServiceError> {
        // `profile_url = $2` is never true when $2 is NULL (SQL NULL semantics),
        // so a resource with no profile matches only the NULL-profile default;
        // ORDER BY (profile_url IS NULL) ASC puts an exact match ahead of it.
        let row = sqlx::query(
            "SELECT definition FROM fhir_mapping \
             WHERE resource_type = $1 AND enabled AND (profile_url = $2 OR profile_url IS NULL) \
             ORDER BY (profile_url IS NULL) LIMIT 1",
        )
        .bind(resource_type)
        .bind(profile)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|r| r.try_get::<Value, _>("definition").map_err(Into::into))
            .transpose()
    }

    /// Resolve-or-create the EHR whose `EHR_STATUS.subject` matches `subject`.
    async fn ensure_ehr_for_subject(&self, subject: SubjectRef) -> Result<EhrId, SmError> {
        let existing = self.get_ehrs_for_subject(subject.clone()).await?;
        if let Some(summary) = existing.first() {
            return Uuid::parse_str(&summary.ehr_id)
                .map(EhrId)
                .map_err(|e| internal_fault("read a stored EHR id", &e));
        }
        self.create_ehr_for_subject(subject, None).await
    }

    /// Every enabled mapping definition for a resource type, across all
    /// profiles (the read façade queries each). Ordered default-last so a
    /// profiled mapping's rows precede the type default's.
    async fn enabled_definitions_for_type(
        &self,
        resource_type: &str,
    ) -> Result<Vec<Value>, ServiceError> {
        let rows = sqlx::query(
            "SELECT definition FROM fhir_mapping WHERE resource_type = $1 AND enabled \
             ORDER BY (profile_url IS NULL), profile_url, id",
        )
        .bind(resource_type)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| r.try_get::<Value, _>("definition").map_err(Into::into))
            .collect()
    }

    /// Resolve the façade `patient` parameter to a query scope + the subject id
    /// used to reconstruct each resource's `subject.reference`. A UUID is an EHR
    /// id (its stored subject is looked up); anything else is a subject external
    /// id (matched by the engine's subject scope).
    async fn resolve_patient_scope(&self, patient: &str) -> Result<PatientScope, ServiceError> {
        if let Ok(ehr_id) = Uuid::parse_str(patient) {
            let subject_id =
                sqlx::query_scalar::<_, Option<String>>("SELECT subject_id FROM ehr WHERE id = $1")
                    .bind(ehr_id)
                    .fetch_optional(&self.pool)
                    .await?
                    .flatten();
            Ok(PatientScope {
                ehr_id: Some(ehr_id),
                subject_scope: None,
                subject_id,
            })
        } else {
            Ok(PatientScope {
                ehr_id: None,
                subject_scope: Some(patient.to_owned()),
                subject_id: Some(patient.to_owned()),
            })
        }
    }

    /// Assemble the FHIR `searchset` Bundle for the read façade: for each
    /// enabled mapping of `resource_type`, run its template-bound COMPOSITION
    /// query scoped to `patient`, reverse-map each hit, and collect the
    /// entries. A type with no enabled mapping yields an empty Bundle (not an
    /// error). `count` caps rows per mapping (`_count`).
    async fn fhir_search_bundle(
        &self,
        resource_type: &str,
        patient: &str,
        count: Option<i64>,
    ) -> Result<Value, SmError> {
        let scope = self.resolve_patient_scope(patient).await?;
        let mut entries: Vec<Value> = Vec::new();
        // One entry per COMPOSITION, never per mapping: two enabled mappings
        // sharing a template would serve one versioned object twice under one
        // fullUrl, which HL7 FHIR R4 bundle.html `bdl-7` forbids. Definitions
        // iterate in the ingest disposition's own precedence, so the mapping
        // that wins a composition is the one ingest would have picked.
        let mut seen: std::collections::HashSet<VoId> = std::collections::HashSet::new();
        for raw in self.enabled_definitions_for_type(resource_type).await? {
            let def: FhirMappingDefinition = serde_json::from_value(raw)
                .map_err(|e| internal_fault("read a stored FHIR mapping definition", &e))?;
            let wt = self.web_template_for(&def.template_id).await?;
            let request = AqlQueryRequest {
                ehr_ids: scope.ehr_id.map(|u| u.to_string()).into_iter().collect(),
                subject_scope: scope.subject_scope.clone(),
                fetch: count,
                parameters: std::iter::once((
                    "templateId".to_owned(),
                    Value::String(def.template_id.clone()),
                ))
                .collect(),
                ..AqlQueryRequest::default()
            };
            let outcome = self.execute_aql(FHIR_SEARCH_AQL, None, &request).await?;
            let rows = outcome
                .result_set
                .get("rows")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for row in &rows {
                // Row shape: [ <uid string> ] (SELECT v/uid/value). The
                // synthesized uid is `<vo_id>::<system>::<ver>`; the COMPOSITION
                // body carries no uid on the AQL read path, so the body is
                // loaded through the versioned read seam by uid (see the
                // FHIR_SEARCH_AQL NOTE).
                let Some(uid) = row.get(0).and_then(Value::as_str) else {
                    continue;
                };
                let mut parts = uid.split("::");
                let (Some(vo_id), _system, Some(sys_version)) = (
                    parts.next().and_then(|s| Uuid::parse_str(s).ok()).map(VoId),
                    parts.next(),
                    parts.next().and_then(|v| v.parse::<i32>().ok()),
                ) else {
                    continue;
                };
                if seen.contains(&vo_id) {
                    continue;
                }
                let Some(read) = crate::versioning::read::read_version_by_ordinal(
                    &self.pool,
                    self.spec_profile,
                    vo_id,
                    sys_version,
                )
                .await?
                else {
                    continue;
                };
                // A logically deleted version reassembles to Value::Null (no
                // node rows) — nothing to map.
                if read.canonical.is_null() || !read.canonical.is_object() {
                    continue;
                }
                seen.insert(vo_id);
                let mut fhir = ferroehr_ext::fhir::reverse::to_fhir(
                    resource_type,
                    &read.canonical,
                    &wt,
                    &def,
                    scope.subject_id.as_deref(),
                )
                .map_err(|e| internal_fault("render a stored COMPOSITION as FHIR", &e))?;
                // The versioned-object id (a UUID) is the FHIR logical id + the
                // `urn:uuid:` fullUrl.
                if let Value::Object(m) = &mut fhir {
                    m.insert("id".to_owned(), Value::String(vo_id.to_string()));
                }
                entries.push(json!({
                    "fullUrl": format!("urn:uuid:{vo_id}"),
                    "resource": fhir,
                }));
            }
        }
        // NOTE: `total` = the entries in THIS Bundle (a stateless connector,
        // not a FHIR Search engine: with `_count` it reports the page size);
        // no `Bundle.link` paging is emitted — our own design/extension.
        Ok(json!({
            "resourceType": "Bundle",
            "type": "searchset",
            "total": entries.len(),
            "entry": entries,
        }))
    }

    /// Reverse-map a committed COMPOSITION version for the **outbound emitter**:
    /// load the version at `(vo_id, sys_version)`, read its bound template from
    /// the canonical `archetype_details/template_id`, and for every enabled
    /// `fhir_mapping` on that template reverse-map it, returning
    /// `(resource_type, template_id, resource)` per mapping.
    ///
    /// Returns an empty vec (nothing to emit) when the version is absent, a
    /// logical delete (a deleted COMPOSITION has no content to map — a FHIR
    /// delete notification is out of the starter scope), carries no template,
    /// or its template has no enabled mapping. Reuses the gated versioned read
    /// seam ([`crate::versioning::read::read_version_by_ordinal`]) and the
    /// reverse transform.
    ///
    /// NOTE: the template is read from the COMPOSITION itself, as the read
    /// facade's AQL also does, rather than from `version.template_id`: the
    /// canonical body is the authoritative carrier
    /// (`archetype_details.template_id`).
    ///
    /// # Errors
    /// A database failure on the version/mapping/subject reads, or
    /// `Unprocessable` when a stored mapping definition no longer deserialises
    /// or the reverse transform fails (a stored mapping/template defect).
    #[cfg(feature = "events")]
    pub(crate) async fn fhir_outbound_messages(
        &self,
        ehr_id: Option<Uuid>,
        vo_id: VoId,
        sys_version: i32,
    ) -> Result<Vec<(String, String, Value)>, ServiceError> {
        // Load the exact committed version; skip absent / logically-deleted ones.
        let Some(read) = crate::versioning::read::read_version_by_ordinal(
            &self.pool,
            self.spec_profile,
            vo_id,
            sys_version,
        )
        .await?
        else {
            return Ok(Vec::new());
        };
        // A logically deleted version reassembles to Value::Null (no node rows).
        if read.canonical.is_null() || !read.canonical.is_object() {
            return Ok(Vec::new());
        }
        // The template the COMPOSITION was built against (its own self-description).
        let Some(template_id) = read
            .canonical
            .pointer("/archetype_details/template_id/value")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
        else {
            return Ok(Vec::new());
        };
        let rows = sqlx::query(
            "SELECT resource_type, definition FROM fhir_mapping \
             WHERE template_id = $1 AND enabled ORDER BY id",
        )
        .bind(&template_id)
        .fetch_all(&self.pool)
        .await?;
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        // The owning EHR's subject id reconstructs each resource's reference.
        let subject_id = match ehr_id {
            Some(id) => {
                sqlx::query_scalar::<_, Option<String>>("SELECT subject_id FROM ehr WHERE id = $1")
                    .bind(id)
                    .fetch_optional(&self.pool)
                    .await?
                    .flatten()
            }
            None => None,
        };
        let wt = self.web_template_for(&template_id).await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            let resource_type: String = row.try_get("resource_type")?;
            let definition: Value = row.try_get("definition")?;
            // A STORED mapping definition that no longer parses is a
            // server-side fault, not a client error: the caller supplied
            // nothing wrong, so it is an `exception` with the diagnostic traced.
            let def: FhirMappingDefinition = serde_json::from_value(definition).map_err(|e| {
                ServiceError::internal(
                    format!("read a stored FHIR mapping definition for {resource_type}"),
                    e,
                )
            })?;
            let resource = ferroehr_ext::fhir::reverse::to_fhir(
                &resource_type,
                &read.canonical,
                &wt,
                &def,
                subject_id.as_deref(),
            )
            .map_err(|e| {
                ServiceError::content_invalid(Violation::new(e.to_string()).with_source(e))
            })?;
            out.push((resource_type, template_id.clone(), resource));
        }
        Ok(out)
    }
}

/// Read + validate `name`: non-empty, `[A-Za-z0-9_.-]` (a clean, addressable
/// deployable identity).
fn validated_name(body: &Value) -> Result<String, ServiceError> {
    let name = body
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ServiceError::precondition("FHIR mapping requires a non-empty 'name'"))?;
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    {
        return Err(ServiceError::precondition(
            "FHIR mapping 'name' must match [A-Za-z0-9_.-]",
        ));
    }
    Ok(name.to_owned())
}

/// Validate the `definition` field: it must be present and deserialise into a
/// [`FhirMappingDefinition`]. Returns the raw JSON (stored verbatim) + the
/// parsed form (for the column projection).
fn validated_definition(body: &Value) -> Result<(Value, FhirMappingDefinition), ServiceError> {
    let raw = body
        .get("definition")
        .cloned()
        .ok_or_else(|| ServiceError::precondition("FHIR mapping requires a 'definition'"))?;
    let def: FhirMappingDefinition = serde_json::from_value(raw.clone()).map_err(|e| {
        ServiceError::bad_request(format!("invalid FHIR mapping definition: {e}"), e)
    })?;
    Ok((raw, def))
}

/// Map an INSERT/UPDATE failure: a unique-name violation is a `Conflict` (409);
/// a foreign-key violation (unknown `template_id`) is a `BadRequest` (400);
/// anything else is the underlying DB error.
fn map_insert_error(e: sqlx::Error) -> ServiceError {
    if let sqlx::Error::Database(db) = &e {
        if db.is_unique_violation() {
            return ServiceError::conflict("a FHIR mapping with that name exists");
        }
        if db.is_foreign_key_violation() {
            return ServiceError::precondition(
                "FHIR mapping references an unknown template_id (ingest the OPT first)",
            );
        }
    }
    ServiceError::Database(e)
}

impl FerroEhrService {
    /// List every stored FHIR mapping (newest first) as JSON records.
    ///
    /// # Errors
    /// [`SmError`] wrapping a database failure.
    pub async fn fhir_mapping_list(&self) -> Result<Vec<Value>, SmError> {
        Ok(self.list_mappings().await?)
    }

    /// Create a FHIR mapping from `{name, enabled?, definition}` (the
    /// definition validated on upload).
    ///
    /// # Errors
    /// `BadRequest` when `name` is missing/invalid, the `definition` is absent
    /// or malformed, or the `template_id` is unknown (FK); `Conflict` on a
    /// duplicate name; otherwise a database failure.
    pub async fn fhir_mapping_create(&self, a_mapping: Value) -> Result<Value, SmError> {
        Ok(self.create_mapping(&a_mapping).await?)
    }

    /// Fetch one FHIR mapping by id.
    ///
    /// # Errors
    /// `NotFound` when the id is unknown; otherwise a database failure.
    pub async fn fhir_mapping_get(&self, a_mapping_id: Uuid) -> Result<Value, SmError> {
        Ok(self.get_mapping(a_mapping_id).await?)
    }

    /// Replace a FHIR mapping's definition + `enabled` (the `name` is
    /// immutable — it is the deployable identity).
    ///
    /// # Errors
    /// `BadRequest` on a malformed definition or unknown `template_id`;
    /// `NotFound` when the id is unknown; otherwise a database failure.
    pub async fn fhir_mapping_update(
        &self,
        a_mapping_id: Uuid,
        a_mapping: Value,
    ) -> Result<Value, SmError> {
        Ok(self.update_mapping(a_mapping_id, &a_mapping).await?)
    }

    /// Delete a FHIR mapping by id.
    ///
    /// # Errors
    /// `NotFound` when the id is unknown; otherwise a database failure.
    pub async fn fhir_mapping_delete(&self, a_mapping_id: Uuid) -> Result<(), SmError> {
        Ok(self.delete_mapping(a_mapping_id).await?)
    }

    /// Resolve every cross-terminology translation `def` requests for
    /// `a_resource` through the terminology seam (`ConceptMap/$translate`),
    /// keyed for [`ferroehr_ext::fhir::mapping::build_flat`].
    ///
    /// Fail-closed: a mapping that declares `translate` on a deployment with
    /// no terminology provider for the route is a configuration fault (500),
    /// never a silent pass-through of the untranslated code. A provider that
    /// ANSWERS "no translation" is not a fault — the entry's own
    /// `required` flag decides between refusal and skip in `build_flat`.
    ///
    /// # Errors
    /// `precondition` when the mapping's translate entries are malformed
    /// against the resource; `Exception` when no provider serves a request's
    /// route or the provider call fails.
    async fn resolve_code_translations(
        &self,
        a_resource: &Value,
        def: &FhirMappingDefinition,
    ) -> Result<ferroehr_ext::fhir::mapping::CodeTranslations, SmError> {
        let requests = ferroehr_ext::fhir::mapping::collect_translations(a_resource, def)
            .map_err(|e| SmError::precondition(e.to_string()).with_source(e))?;
        let mut translations = ferroehr_ext::fhir::mapping::CodeTranslations::default();
        if requests.is_empty() {
            return Ok(translations);
        }
        let Some(router) = &self.terminology else {
            return Err(internal_fault(
                "translate a FHIR mapping's codes",
                &"the mapping declares translate but no terminology provider is configured",
            ));
        };
        for request in requests {
            let provider = request
                .route_terminology
                .as_deref()
                .and_then(|t| router.provider_for(t))
                .or_else(|| router.default_provider())
                .ok_or_else(|| {
                    internal_fault(
                        "translate a FHIR mapping's codes",
                        &format_args!(
                            "no terminology provider serves route '{}'",
                            request.route_terminology.as_deref().unwrap_or("default")
                        ),
                    )
                })?;
            if let Some(translated) = provider
                .translate(
                    &request.system,
                    &request.code,
                    &request.target_system,
                    request.concept_map.as_deref(),
                )
                .await?
            {
                translations.0.insert(
                    (request.system, request.code, request.target_system),
                    translated,
                );
            }
        }
        Ok(translations)
    }

    /// Ingest a FHIR resource: resolve the enabled mapping for
    /// `resource_type` + `profile`, resolve-or-create the target EHR from the
    /// resource's subject, build the COMPOSITION (FLAT → canonical), stamp
    /// `FEEDER_AUDIT` provenance, and commit through the NORMAL validated
    /// create path.
    ///
    /// # Errors
    /// `VersionedObjectDoesNotExist` (404) when no enabled mapping matches the
    /// resource type; `precondition` when the resource lacks the mapped
    /// subject or a required field; `ContentInvalid` (422) when the mapped
    /// FLAT does not build a valid COMPOSITION or the committed COMPOSITION
    /// fails validation; `Exception` when a stored mapping definition no
    /// longer deserialises.
    pub async fn fhir_ingest(
        &self,
        resource_type: String,
        profile: Option<String>,
        a_resource: Value,
    ) -> Result<ServiceResponse, SmError> {
        // 1. Resolve the mapping (no enabled match → 404).
        let def_value = self
            .resolve_mapping(&resource_type, profile.as_deref())
            .await?
            .ok_or_else(|| {
                SmError::new(
                    CallStatusType::VersionedObjectDoesNotExist,
                    format!("no enabled FHIR mapping for resource type '{resource_type}'"),
                )
            })?;
        let def: FhirMappingDefinition = serde_json::from_value(def_value)
            .map_err(|e| internal_fault("read a stored FHIR mapping definition", &e))?;

        // 2. Resolve-or-create the target EHR from the resource's subject.
        let subject = ferroehr_ext::fhir::mapping::extract_subject(&a_resource, &def)
            .map_err(|e| SmError::precondition(e.to_string()).with_source(e))?;
        // The mapping's output identity resolves through the EHR index as a
        // PERSON subject (the inbound connector's contract).
        let ehr_id = self
            .ensure_ehr_for_subject(SubjectRef::person(subject.id, subject.namespace))
            .await?;

        // 3. Resolve the mapping's cross-terminology translations through the
        //    terminology seam (FHIR ConceptMap/$translate), then build the
        //    FLAT map + the canonical COMPOSITION. `now` supplies the ctx/time
        //    default (ITS-REST simplified_formats master04 §Context) and the
        //    FEEDER_AUDIT provenance instant — one ingestion instant for both.
        let translations = self.resolve_code_translations(&a_resource, &def).await?;
        let wt = self.web_template_for(&def.template_id).await?;
        let flat = ferroehr_ext::fhir::mapping::build_flat(&a_resource, &def, &translations)
            .map_err(|e| SmError::precondition(e.to_string()).with_source(e))?;
        let now = ferroehr_ext::fhir::feeder_audit::now_iso();
        let mut composition = openehr_sdt::flat::convert::composition_from_flat(&flat, &wt, &now)
            .map_err(|e| {
            SmError::new(
                CallStatusType::ContentInvalid,
                format!("FHIR resource did not map to a valid COMPOSITION: {e}"),
            )
            .with_source(e)
        })?;

        // 4. Stamp FEEDER_AUDIT provenance.
        let feeder = ferroehr_ext::fhir::feeder_audit::feeder_audit(
            &resource_type,
            &ferroehr_ext::fhir::feeder_audit::resource_id(&a_resource, &resource_type),
            ferroehr_ext::fhir::feeder_audit::resource_version(&a_resource).as_deref(),
            &now,
        );
        ferroehr_ext::fhir::feeder_audit::inject_feeder_audit(&mut composition, feeder);

        // 5. Commit through the NORMAL validated path: the converter emits a
        //    canonical fragment, and re-typing it through the strict reader is
        //    what hands the commit seam a typed value. A resource that maps to
        //    an invalid COMPOSITION is rejected, never partially stored.
        let typed: openehr_rm::prelude::Composition =
            openehr_its::json::from_canonical_value(&composition).map_err(|e| {
                SmError::new(
                    CallStatusType::ContentInvalid,
                    format!("FHIR resource did not map to a valid COMPOSITION: {e}"),
                )
                .with_source(e)
            })?;
        // Boxed: the typed COMPOSITION envelope makes this future large enough
        // to matter on the stack of an async ingest path (clippy
        // `large_futures`).
        let committed = Box::pin(self.create_composition(
            ehr_id,
            crate::service::version_update::direct_envelope(typed),
        ))
        .await?;
        Ok(self.committed_response(ehr_id, &committed))
    }

    /// The ingest door's dry twin, FHIR R4 `$validate`: resolves the mapping,
    /// maps to FLAT, builds and provenance-stamps the COMPOSITION exactly as
    /// [`Self::fhir_ingest`] would, runs the commit path's own validation and
    /// commits nothing.
    ///
    /// Returns the FHIR `OperationOutcome` carrying the verdict: the validator's
    /// rejections verbatim as `error` issues, or the valid verdict plus the EHR
    /// disposition as `information` issues, the target EHR being resolved and
    /// never created.
    ///
    /// NOTE: no openEHR spec governs FHIR interop — our own extension; the
    /// wire convention is HL7 FHIR R4 `resource-operation-validate`
    /// (<https://hl7.org/fhir/R4/resource-operation-validate.html>).
    ///
    /// # Errors
    /// Operation-level failures mirror [`Self::fhir_ingest`]:
    /// `VersionedObjectDoesNotExist` (404) when no enabled mapping matches;
    /// `precondition` when the resource lacks the mapped subject or a
    /// required field; `Exception` on a broken stored definition or
    /// terminology fault. CONTENT failures are never errors — they are the
    /// verdict this operation exists to preview.
    pub async fn fhir_validate(
        &self,
        resource_type: String,
        profile: Option<String>,
        a_resource: Value,
    ) -> Result<Value, SmError> {
        // Steps 1–4 of the ingest path, verbatim: the dry run validates the
        // exact artifact the real commit would hand the validator.
        let def_value = self
            .resolve_mapping(&resource_type, profile.as_deref())
            .await?
            .ok_or_else(|| {
                SmError::new(
                    CallStatusType::VersionedObjectDoesNotExist,
                    format!("no enabled FHIR mapping for resource type '{resource_type}'"),
                )
            })?;
        let def: FhirMappingDefinition = serde_json::from_value(def_value)
            .map_err(|e| internal_fault("read a stored FHIR mapping definition", &e))?;
        let subject = ferroehr_ext::fhir::mapping::extract_subject(&a_resource, &def)
            .map_err(|e| SmError::precondition(e.to_string()).with_source(e))?;
        // Resolve — never create: the disposition is reported, not enacted.
        let disposition = match self
            .get_ehrs_for_subject(SubjectRef::person(
                subject.id.clone(),
                subject.namespace.clone(),
            ))
            .await?
            .first()
        {
            Some(existing) => format!("would commit into existing EHR {}", existing.ehr_id),
            None => format!(
                "would create a new EHR for subject '{}' (namespace '{}')",
                subject.id, subject.namespace
            ),
        };
        let translations = self.resolve_code_translations(&a_resource, &def).await?;
        let wt = self.web_template_for(&def.template_id).await?;
        let flat = ferroehr_ext::fhir::mapping::build_flat(&a_resource, &def, &translations)
            .map_err(|e| SmError::precondition(e.to_string()).with_source(e))?;
        let now = ferroehr_ext::fhir::feeder_audit::now_iso();
        let template_id = def.template_id.clone();

        // From here every failure is the VERDICT: exactly the refusals the
        // ingest path classes content-invalid (422), previewed instead of
        // enacted.
        let verdict = self
            .dry_run_verdict(&resource_type, &a_resource, &flat, &wt, &now)
            .await?;
        Ok(match verdict {
            None => serde_json::json!({
                "resourceType": "OperationOutcome",
                "issue": [
                    { "severity": "information", "code": "informational",
                      "diagnostics": format!(
                          "valid: the resource maps to a COMPOSITION under template \
                           '{template_id}' that passes commit validation; nothing was committed"
                      ) },
                    { "severity": "information", "code": "informational",
                      "diagnostics": disposition }
                ]
            }),
            Some(rejection) => serde_json::json!({
                "resourceType": "OperationOutcome",
                "issue": [
                    { "severity": "error", "code": "invalid", "diagnostics": rejection },
                    { "severity": "information", "code": "informational",
                      "diagnostics": disposition }
                ]
            }),
        })
    }

    /// The dry run's content verdict: `None` when the mapped COMPOSITION
    /// builds, re-types and passes the commit path's own validation (the
    /// validity-checking seam every commit runs); `Some(rejection)` carrying
    /// the refusal text verbatim otherwise.
    ///
    /// # Errors
    /// Only non-verdict service failures (template store or database faults)
    /// propagate; a validation refusal is the RETURN VALUE, never an error.
    async fn dry_run_verdict(
        &self,
        resource_type: &str,
        a_resource: &Value,
        flat: &serde_json::Map<String, Value>,
        wt: &openehr_sdt::flat::webtemplate::model::WebTemplate,
        now: &str,
    ) -> Result<Option<String>, SmError> {
        let mut composition = match openehr_sdt::flat::convert::composition_from_flat(flat, wt, now)
        {
            Ok(c) => c,
            Err(e) => {
                return Ok(Some(format!(
                    "FHIR resource did not map to a valid COMPOSITION: {e}"
                )));
            }
        };
        let feeder = ferroehr_ext::fhir::feeder_audit::feeder_audit(
            resource_type,
            &ferroehr_ext::fhir::feeder_audit::resource_id(a_resource, resource_type),
            ferroehr_ext::fhir::feeder_audit::resource_version(a_resource).as_deref(),
            now,
        );
        ferroehr_ext::fhir::feeder_audit::inject_feeder_audit(&mut composition, feeder);
        if let Err(e) = openehr_its::json::from_canonical_value::<openehr_rm::prelude::Composition>(
            &composition,
        ) {
            return Ok(Some(format!(
                "FHIR resource did not map to a valid COMPOSITION: {e}"
            )));
        }
        self.commit_rejection(crate::versioning::Kind::Composition, &composition)
            .await
    }

    /// The read façade: the FHIR `searchset` Bundle for `resource_type` scoped
    /// to `patient` (an EHR uuid or a subject external id), reverse-mapped from
    /// the stored COMPOSITIONs. `count` caps rows per mapping (`_count`).
    ///
    /// # Errors
    /// `Exception` when a stored mapping definition is invalid or the reverse
    /// transform fails; otherwise a database/query failure.
    pub async fn fhir_search(
        &self,
        resource_type: String,
        patient: String,
        count: Option<i64>,
    ) -> Result<Value, SmError> {
        self.fhir_search_bundle(&resource_type, &patient, count)
            .await
    }
}
