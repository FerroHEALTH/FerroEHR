// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The COMPOSITION read's response, and the read that judges its own row.
//!
//! One statement reads the object's access facts beside the version, and the
//! access evaluator judges the object before anything of the version leaves
//! the service (#5773). A refused read has read the version into memory and
//! returns nothing of it. No openEHR spec governs this: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): the COMPOSITION read serves the \
              stored canonical body"
)]

use openehr_base::prelude::ObjectVersionId;
use openehr_rm::draft::access_control::terminology::Refusal;
use serde_json::Value;

use crate::access_eval::condition::RequestAccess;
use crate::access_eval::procedure::Outcome;
use crate::error::ServiceError;
use crate::error::refusal::Missing;
use crate::ids::{EhrId, VoId};
use crate::service::FerroEhrService;
use crate::service::response::{RawServiceResponse, ReadBody};
use crate::storage::access_facts::visible;
use crate::storage::version_repo::judged::{JudgedAt, read_judged};
use crate::versioning::Kind;
use crate::versioning::object_version_id::{VersionIdError, components};
use crate::versioning::read::raw_version_read;

/// What a COMPOSITION read judged on its own row answers.
#[derive(Debug)]
pub enum JudgedRead {
    /// The store holds no such object of an EHR: the request is judged on
    /// its EHR alone, and the read answers the error.
    Unheld(ServiceError),
    /// The facts could not be read or judged: a store or settings fault.
    Unjudged(ServiceError),
    /// The evaluator reads the object as absent to the caller (`404`).
    Absent,
    /// The evaluator refuses the object (`403`) under `code`.
    Refused {
        /// The object's EHR, whose refusal grounds the answer names.
        ehr_id: EhrId,
        /// The refusal.
        code: Refusal,
    },
    /// The evaluator admits the object: the read's answer.
    Admitted(Result<RawServiceResponse, ServiceError>),
}

impl FerroEhrService {
    /// The COMPOSITION `vo_id` of `ehr_id` at `version`, or its latest,
    /// judged by `access` on the object's own facts, read in the one statement
    /// that reads the version, the refusal noted for the access record.
    pub async fn composition_judged(
        &self,
        access: &RequestAccess,
        ehr_id: EhrId,
        vo_id: VoId,
        version: Option<&ObjectVersionId>,
    ) -> JudgedRead {
        let at = match version.map(components).transpose() {
            Ok(Some((_, tree))) => JudgedAt::Tree(vo_id, tree),
            Ok(None) => JudgedAt::Current(vo_id),
            Err(error) => return JudgedRead::Admitted(Err(VersionIdError::into_argument(error))),
        };
        let missing =
            || ServiceError::not_found(Missing::Composition, format!("COMPOSITION {vo_id}"));
        let (facts, stored) = match read_judged(&self.pool, at).await {
            Ok(Some((facts, stored))) => (facts, stored),
            Ok(None) => return JudgedRead::Unheld(missing()),
            Err(error) => return JudgedRead::Unjudged(error.into()),
        };
        // A party's object is judged by no EHR: the request is judged on its EHR alone.
        let Some(object_ehr) = facts.ehr_id else {
            return JudgedRead::Unheld(missing());
        };
        let verdict = match self
            .ehr_verdict_at(access, object_ehr, &facts.revision)
            .await
        {
            Ok(verdict) => verdict,
            Err(error) => return JudgedRead::Unjudged(error),
        };
        match access.note(object_ehr, &verdict, &facts.item()) {
            Outcome::Admit => {}
            Outcome::Absent(_) => return JudgedRead::Absent,
            Outcome::Refused(code) => {
                return JudgedRead::Refused {
                    ehr_id: object_ehr,
                    code,
                };
            }
        }
        let served = stored
            .filter(visible)
            .map(|stored| raw_version_read(self.spec_profile, stored))
            .transpose()
            .and_then(|raw| self.composition_served(ehr_id, vo_id, raw))
            .and_then(|read| {
                if let (Some(addressed), Some(meta)) = (version, read.meta.as_ref()) {
                    super::ensure_addressed_version(addressed, &meta.uid)?;
                }
                Ok(read)
            });
        JudgedRead::Admitted(served)
    }

    /// The COMPOSITION response of `raw`, the version of `vo_id` a read found
    /// (see [`Self::read_composition_raw`]).
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] when `raw` is no COMPOSITION of `ehr_id`, and
    /// the identifier and body faults of the stored version.
    pub(super) fn composition_served(
        &self,
        ehr_id: EhrId,
        vo_id: VoId,
        raw: Option<crate::versioning::read::RawVersionRead>,
    ) -> Result<RawServiceResponse, ServiceError> {
        let raw = raw
            .filter(|r| r.read.ehr_id == Some(ehr_id) && r.read.kind == Kind::Composition)
            .ok_or_else(|| {
                ServiceError::not_found(Missing::Composition, format!("COMPOSITION {vo_id}"))
            })?;
        if raw.read.deleted() {
            return Ok(RawServiceResponse {
                body: ReadBody::Value(Value::Null),
                meta: None,
            });
        }
        let read = raw.read;
        let meta = self
            .version_meta(
                ehr_id,
                vo_id,
                &read.creating_system_id,
                read.tree,
                read.time_committed,
            )
            .with_origins(crate::versioning::origins::of_stored(
                read.origins.as_ref(),
                &read.canonical,
                &read.creating_system_id,
            ))
            .with_content(vec![read.content.clone()]);
        let Some(text) = raw.raw_json else {
            let stamped =
                self.with_uid(read.canonical, vo_id, &read.creating_system_id, read.tree)?;
            return Ok(RawServiceResponse {
                body: ReadBody::Value(stamped),
                meta: Some(meta),
            });
        };
        let uid = crate::versioning::object_version_id::object_version_id(
            vo_id,
            &read.creating_system_id,
            read.tree,
        );
        if carries_version_uid(&text, &uid) {
            return Ok(RawServiceResponse {
                body: ReadBody::RawJson(text),
                meta: Some(meta),
            });
        }
        let value: Value = serde_json::from_str(&text).map_err(|e| {
            ServiceError::exception(format!(
                "the stored body of COMPOSITION {vo_id} is not decodable JSON: {e}"
            ))
        })?;
        let stamped = self.with_uid(value, vo_id, &read.creating_system_id, read.tree)?;
        Ok(RawServiceResponse {
            body: ReadBody::Value(stamped),
            meta: Some(meta),
        })
    }
}

/// Whether `text`, a stored body in the compact form the commit writes
/// (`canonical_body_text`), carries `uid` as its stamped `OBJECT_VERSION_ID`
/// member, so the JSON read can serve it verbatim (#5755).
fn carries_version_uid(text: &str, uid: &str) -> bool {
    const OPEN: &str = r#""uid":{"_type":"OBJECT_VERSION_ID","value":""#;
    text.split(OPEN)
        .skip(1)
        .any(|rest| rest.strip_prefix(uid).is_some_and(|r| r.starts_with("\"}")))
}
