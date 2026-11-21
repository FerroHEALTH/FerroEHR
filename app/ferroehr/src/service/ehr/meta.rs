// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Shared version-metadata helpers: the glue every versioned kind
//! (`EHR_STATUS`, COMPOSITION, DIRECTORY) needs to turn a loaded
//! [`VersionRead`] into a wire [`ServiceResponse`] and its [`ResourceMeta`],
//! plus the commit-audit builder of a direct write.
//!
//! The `OBJECT_VERSION_ID` law is RM common
//! `master06-change_control_package.adoc` §Version Identification and BASE
//! `base_types/master05-identification_package.adoc` §Syntaxes; the
//! `Last-Modified`, `ETag` and `Location` derivation from a version's commit
//! audit is ITS-REST, carried in the [`ResourceMeta`] envelope. No openEHR spec
//! governs that envelope, or the `current_vo` storage seam
//! ([`crate::storage::version_repo`]) — our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once; stored-content serving"
)]

use crate::ids::{EhrId, VoId};
use crate::service::response::{RawServiceResponse, ReadBody, ResourceMeta, ServiceResponse};
use serde_json::Value;

use crate::error::ServiceError;
use crate::service::FerroEhrService;
use crate::versioning::Kind;
use crate::versioning::audit::AuditInput;
use crate::versioning::object_version_id::{TreeId, VersionIdError, object_version_id, version_id};
use crate::versioning::read::VersionRead;
use crate::versioning::wire::version_envelope;

impl FerroEhrService {
    /// The current version `(vo_id, VERSION_TREE_ID)` of an EHR's object of a
    /// given [`Kind`], if any — the trunk head the object's head row names.
    ///
    /// The `version` current-row read is a storage seam
    /// ([`crate::storage::version_repo::meta::current_vo`]; no openEHR spec governs
    /// the SQL — our own design).
    ///
    /// # Errors
    /// [`ServiceError::Database`] if the current-row read fails.
    pub(crate) async fn current_vo(
        &self,
        ehr_id: EhrId,
        kind: Kind,
    ) -> Result<Option<(VoId, TreeId)>, ServiceError> {
        Ok(
            crate::storage::version_repo::meta::current_vo(&self.pool, ehr_id, kind.as_str())
                .await?
                .map(|r| {
                    (
                        r.vo_id,
                        TreeId::from_columns(r.trunk_version, r.branch_number, r.branch_version),
                    )
                }),
        )
    }

    /// A metadata-only [`ServiceResponse`] for a write, built entirely from the
    /// commit result — the version identity and the server commit instant are
    /// already in [`Committed`](crate::versioning::change::Committed), so the write
    /// path never re-reads the row it just wrote (a representation response
    /// re-reads at the protocol layer). The body is `Value::Null` by contract:
    /// every write consumer uses only the metadata.
    pub(crate) fn committed_response(
        &self,
        ehr_id: EhrId,
        committed: &crate::versioning::change::Committed,
    ) -> ServiceResponse {
        let meta = self
            .version_meta(
                ehr_id,
                committed.vo_id,
                &committed.creating_system_id,
                committed.tree,
                committed.time_committed,
            )
            .with_content(committed.content_ids().into_iter().collect());
        ServiceResponse::new(Value::Null, meta)
    }

    /// The [`ResourceMeta`] for a versioned resource: the owning EHR plus the
    /// resource `OBJECT_VERSION_ID` (the `ETag` value + `Location` tail) and
    /// its commit time (the `Last-Modified`). `creating_system_id` is the
    /// stored per-version value — never re-derived from the live config — so a
    /// version's uid stays stable across a later system-id change (RM common
    /// master06 §Distributed Versioning).
    #[expect(
        clippy::unused_self,
        reason = "call-site ergonomics: every caller already holds the service, \
                  so an inherent method reads better than a free function"
    )]
    pub(crate) fn version_meta(
        &self,
        ehr_id: EhrId,
        vo_id: VoId,
        creating_system_id: &str,
        version: TreeId,
        at: jiff::Timestamp,
    ) -> ResourceMeta {
        ResourceMeta::new(
            ehr_id.to_string(),
            object_version_id(vo_id, creating_system_id, version),
        )
        .with_last_modified(at)
    }

    /// Ensure the versioned object's canonical JSON carries its `uid`
    /// (`OBJECT_VERSION_ID`, RM common master06 §Version Identification) so a
    /// bare read serves its wire identity.
    ///
    /// The write path already stamps the uid into the stored body
    /// (`crate::versioning::change` — `stamp_version_uid` runs before
    /// decomposition), so on a locally committed version this is a no-op:
    /// when the stored `uid` is an `OBJECT_VERSION_ID` whose value matches,
    /// the body passes through untouched. Only a body that lacks or mismatches
    /// it (e.g. a verbatim-imported foreign version) pays the re-stamp.
    #[expect(
        clippy::unused_self,
        reason = "call-site ergonomics: every caller already holds the service, \
                  so an inherent method reads better than a free function"
    )]
    pub(in crate::service) fn with_uid(
        &self,
        mut canonical: Value,
        vo_id: VoId,
        creating_system_id: &str,
        version: TreeId,
    ) -> Result<Value, VersionIdError> {
        if let Value::Object(map) = &mut canonical {
            let id = version_id(vo_id, creating_system_id, version)?;
            let already_stamped = map.get("uid").is_some_and(|uid| {
                uid.get("_type").and_then(Value::as_str) == Some("OBJECT_VERSION_ID")
                    && uid.get("value").and_then(Value::as_str) == Some(id.value())
            });
            if !already_stamped {
                map.insert("uid".to_owned(), openehr_its::json::to_canonical_value(&id));
            }
        }
        Ok(canonical)
    }

    /// The response of a version read whose body may still be the stored
    /// text ([`crate::versioning::read::read_raw`]): the text verbatim where the
    /// commit stamped the version's own `uid` into it, else its canonical value
    /// with the `uid` injected ([`Self::with_uid`]).
    ///
    /// # Errors
    /// The identifier fault of the stored version, and a stored body that is
    /// not JSON.
    pub(in crate::service) fn raw_version_response(
        &self,
        ehr_id: EhrId,
        vo_id: VoId,
        raw: crate::versioning::read::RawVersionRead,
    ) -> Result<RawServiceResponse, ServiceError> {
        let read = raw.read;
        let meta = self
            .version_meta(
                ehr_id,
                vo_id,
                &read.creating_system_id,
                read.tree,
                read.time_committed,
            )
            .with_content(vec![read.content.clone()]);
        Ok(RawServiceResponse {
            body: self.served_body(vo_id, read, raw.raw_json)?,
            meta: Some(meta),
        })
    }

    /// The version `version` of `vo_id`, of `kind`, in `ehr_id`, with its body
    /// kept as the stored text where nothing needs it parsed; `None` for a
    /// version of another EHR or kind, or none at all.
    ///
    /// # Errors
    /// The storage error of the read and the profile gate's refusal.
    pub(in crate::service) async fn tree_version_raw(
        &self,
        ehr_id: EhrId,
        kind: Kind,
        vo_id: VoId,
        version: TreeId,
    ) -> Result<Option<crate::versioning::read::RawVersionRead>, ServiceError> {
        Ok(crate::versioning::read::read_raw(
            &self.pool,
            self.spec_profile,
            crate::storage::version_repo::read::VersionAt::Tree(vo_id, version),
        )
        .await?
        .filter(|r| r.read.ehr_id == Some(ehr_id) && r.read.kind == kind))
    }

    /// The body a read serves of `read`, the version of `vo_id`: `raw_json`,
    /// its stored text, verbatim where it carries the version's own `uid`,
    /// else the canonical value with the `uid` injected; an `EHR_ACCESS`
    /// read by anyone but the controller carries no exclusions.
    ///
    /// # Errors
    /// The identifier fault of the stored version, and a stored body that is
    /// not JSON.
    pub(in crate::service) fn served_body(
        &self,
        vo_id: VoId,
        read: VersionRead,
        raw_json: Option<String>,
    ) -> Result<ReadBody, ServiceError> {
        // NOTE: draft access control decision procedure, Reading and Writing the Settings, only
        // the controller reads the exclusions of an EHR_ACCESS (#6077).
        let withhold = read.kind == Kind::EhrAccess && !self.settings_caller().administrator();
        let Some(mut text) = raw_json else {
            let mut canonical = read.canonical;
            if withhold {
                super::access_exclusions::withhold(&mut canonical);
            }
            return Ok(ReadBody::Value(self.with_uid(
                canonical,
                vo_id,
                &read.creating_system_id,
                read.tree,
            )?));
        };
        if withhold && let Some(withheld) = super::access_exclusions::withheld(&text)? {
            text = withheld;
        }
        let uid = object_version_id(vo_id, &read.creating_system_id, read.tree);
        if carries_version_uid(&text, &uid) {
            return Ok(ReadBody::RawJson(text));
        }
        let value: Value = serde_json::from_str(&text).map_err(|e| {
            ServiceError::exception(format!(
                "the stored body of {} {vo_id} is not decodable JSON: {e}",
                read.kind.as_str()
            ))
        })?;
        Ok(ReadBody::Value(self.with_uid(
            value,
            vo_id,
            &read.creating_system_id,
            read.tree,
        )?))
    }

    /// A [`ServiceResponse`] for the VERSION envelope of a loaded version, an
    /// `ORIGINAL_VERSION` or an `IMPORTED_VERSION` (RM common master06
    /// §Version and its Subtypes), with the version's `OBJECT_VERSION_ID` and
    /// commit instant as its metadata.
    ///
    /// # Errors
    /// The [`version_envelope`] refusals: a failed read-time signature check or
    /// an uninterpretable stored commit audit; [`ServiceError::NotFound`] for
    /// an `EHR_ACCESS` version carrying exclusions read by anyone but the
    /// controller.
    pub(in crate::service) fn envelope_response(
        &self,
        ehr_id: EhrId,
        read: &VersionRead,
    ) -> Result<ServiceResponse, ServiceError> {
        // NOTE: draft access control decision procedure, Reading and Writing the Settings, a
        // signed envelope is never served altered, so one carrying exclusions reads as absent.
        if read.kind == Kind::EhrAccess
            && !self.settings_caller().administrator()
            && super::access_exclusions::held(&read.canonical).is_some()
        {
            return Err(ServiceError::not_found(
                crate::error::refusal::Missing::ObjectVersion,
                format!("EHR_ACCESS version for EHR {ehr_id}"),
            ));
        }
        let meta = self
            .version_meta(
                ehr_id,
                read.vo_id,
                &read.creating_system_id,
                read.tree,
                read.time_committed,
            )
            .with_content(vec![read.content.clone()]);
        Ok(ServiceResponse::new(
            version_envelope(read, self.signer())?,
            meta,
        ))
    }

    /// The current `EHR_STATUS` version metadata of an EHR, or `None` when the
    /// EHR has no current `EHR_STATUS`.
    ///
    /// # Errors
    /// [`ServiceError`] if the metadata read fails.
    pub async fn ehr_status_latest_meta(
        &self,
        ehr_id: EhrId,
    ) -> Result<Option<ResourceMeta>, ServiceError> {
        let kind = Kind::EhrStatus;
        Ok(self
            .latest_version_meta_with_vo(ehr_id, kind)
            .await?
            .map(|(_, m)| m))
    }

    /// The current version's `vo_id` **and** its [`ResourceMeta`] for an
    /// EHR-owned object of `kind`, resolved and read in the same ONE
    /// metadata-only `version`⋈`audit` statement [`Self::ehr_status_latest_meta`]
    /// uses. Threading the `vo_id` back to the caller lets a following write
    /// skip re-resolving `(ehr_id, kind) → vo_id`, so the `If-Match` pre-read
    /// and the write share one `current_vo` resolution. The
    /// full-`OBJECT_VERSION_ID` compare (ITS-REST overview §Concurrency
    /// control) is unchanged.
    ///
    /// # Errors
    /// [`ServiceError::Database`] if the metadata read fails.
    pub(in crate::service) async fn latest_version_meta_with_vo(
        &self,
        ehr_id: EhrId,
        kind: Kind,
    ) -> Result<Option<(VoId, ResourceMeta)>, ServiceError> {
        let Some(m) = crate::storage::version_repo::meta::current_version_meta_by_kind(
            &self.pool,
            ehr_id,
            kind.as_str(),
        )
        .await?
        else {
            return Ok(None);
        };
        let tree = TreeId::from_columns(m.trunk_version, m.branch_number, m.branch_version);
        let meta = self.version_meta(
            ehr_id,
            m.vo_id,
            &m.creating_system_id,
            tree,
            m.time_committed,
        );
        Ok(Some((m.vo_id, meta)))
    }

    /// Build an [`AuditInput`] for a direct (single-object) write: the
    /// effective system id, the numeric `audit_change_type` code, a
    /// description, and the request's authenticated committer (RM common
    /// master04 §Audit Details).
    pub(in crate::service) fn audit(&self, change_type: &str, description: &str) -> AuditInput {
        AuditInput {
            system_id: self.effective_system_id(),
            change_type: change_type.to_owned(),
            description: Some(crate::versioning::audit::dv_text(description)),
            committer: crate::versioning::committer::request_committer(&self.system_id),
            attestation: None,
        }
    }
}

/// The [`ResourceMeta`] of a version-CONTAINER read (a `VERSIONED_OBJECT` body
/// or its `REVISION_HISTORY`): the `ETag` identity is the container uid
/// (ITS-REST overview `Requests_and_responses.md` §"`ETag` and Last-Modified"
/// names "`VERSIONED_OBJECT.uid.value`" as an `ETag` source) and the
/// `Last-Modified` instant is the newest held version's commit time (same §:
/// both headers "SHOULD be included in responses for VERSION,
/// `VERSIONED_OBJECT`, or other resources that have versioning or unique
/// state identifiers"; the value is "derived from
/// `VERSION.commit_audit.time_committed.value`").
pub(in crate::service) fn container_meta(
    ehr_id: EhrId,
    vo_id: VoId,
    last_modified: jiff::Timestamp,
) -> ResourceMeta {
    ResourceMeta::new(ehr_id.to_string(), vo_id.to_string()).with_last_modified(last_modified)
}

/// Whether `text`, a stored body in the compact form the commit writes
/// (`canonical_body_text`), carries `uid` as its root `OBJECT_VERSION_ID`, so
/// the JSON read can serve it verbatim (#5755).
///
/// It reads only the root members the codec writes before the uid (`_type`,
/// `name`, `archetype_node_id`; RM BMM LOCATABLE), so the cost is that short
/// prefix (#6198). A root uid elsewhere, or any other shape, answers `false`
/// and takes the parse path, which serves the same document.
fn carries_version_uid(text: &str, uid: &str) -> bool {
    const LEADING: [&[u8]; 3] = [b"_type", b"name", b"archetype_node_id"];
    let bytes = text.as_bytes();
    let Some(mut at) = expect(bytes, 0, b'{') else {
        return false;
    };
    loop {
        let Some((key, after_key)) = plain_string(bytes, at) else {
            return false;
        };
        let Some(value) = expect(bytes, after_key, b':') else {
            return false;
        };
        if key == b"uid" {
            return stamped_uid_at(bytes, value, uid.as_bytes());
        }
        if !LEADING.contains(&key) {
            return false;
        }
        let Some(next) = skip_value(bytes, value).and_then(|end| expect(bytes, end, b',')) else {
            return false;
        };
        at = next;
    }
}

/// The index past `byte` when it sits at `at`.
fn expect(bytes: &[u8], at: usize, byte: u8) -> Option<usize> {
    (bytes.get(at) == Some(&byte)).then_some(at + 1)
}

/// The content of the JSON string opening at `at` and the index past it;
/// `None` for one with an escape, which no root key the codec writes has.
fn plain_string(bytes: &[u8], at: usize) -> Option<(&[u8], usize)> {
    let start = expect(bytes, at, b'"')?;
    let len = bytes.get(start..)?.iter().position(|&b| b == b'"')?;
    let content = bytes.get(start..start + len)?;
    (!content.contains(&b'\\')).then_some((content, start + len + 1))
}

/// Whether the value at `at` is exactly `{"_type":"OBJECT_VERSION_ID","value":"<uid>"}`.
fn stamped_uid_at(bytes: &[u8], at: usize, uid: &[u8]) -> bool {
    const OPEN: &[u8] = br#"{"_type":"OBJECT_VERSION_ID","value":""#;
    bytes
        .get(at..)
        .and_then(|rest| rest.strip_prefix(OPEN))
        .and_then(|rest| rest.strip_prefix(uid))
        .and_then(|rest| rest.strip_prefix(br#""}"#))
        .is_some_and(|rest| matches!(rest.first(), Some(b',' | b'}')))
}

/// The index past the compact JSON value starting at `at`.
fn skip_value(bytes: &[u8], at: usize) -> Option<usize> {
    let mut depth = 0_usize;
    let mut in_string = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate().skip(at) {
        if in_string {
            match (escaped, b) {
                (true, _) => escaped = false,
                (false, b'\\') => escaped = true,
                (false, b'"') => {
                    in_string = false;
                    if depth == 0 {
                        return Some(i + 1);
                    }
                }
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' | b'[' => depth += 1,
            b'}' | b']' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            b',' if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
#[path = "meta_tests.rs"]
mod tests;
