// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The COMPOSITION read's response, and the read that judges its own row.
//!
//! One statement reads the object's access facts beside the version, and the
//! access evaluator judges the object before anything of the version leaves
//! the service (#5773). A read by version id takes the version's body from
//! this replica's copy once one indexed read confirms its object's state
//! (#5778). A refused read has
//! read the version into memory and returns nothing of it. No openEHR spec
//! governs this: our own design.

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
use crate::service::ehr::version_copy::Held;
use crate::service::response::{RawServiceResponse, ReadBody};
use crate::storage::access_facts::{HeadFacts, visible};
use crate::storage::version_repo::judged::{JudgedAt, read_judged, read_marks};
use crate::storage::version_repo::read::StoredVersion;
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

/// A COMPOSITION version read before its EHR's verdict is final (#5959).
///
/// Nothing of it leaves the service until
/// [`FerroEhrService::composition_judged_on`] judges it.
pub struct VersionAhead(Result<Option<(HeadFacts, Option<StoredVersion>)>, Unread>);

/// Why a [`VersionAhead`] holds no version.
enum Unread {
    /// The version id is malformed: the read answers it as an argument error.
    Argument(ServiceError),
    /// The store did not answer.
    Store(ServiceError),
}

impl std::fmt::Debug for VersionAhead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // NOTE: our own design (#5959), the read version is clinical content and never formatted.
        let state = match &self.0 {
            Ok(Some(_)) => "held",
            Ok(None) => "unheld",
            Err(Unread::Argument(_)) => "argument",
            Err(Unread::Store(_)) => "unread",
        };
        f.debug_tuple("VersionAhead").field(&state).finish()
    }
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
        let (ahead, ()) = tokio::join!(
            self.composition_ahead(vo_id, version),
            self.care_ahead(access, ehr_id)
        );
        self.composition_judged_on(access, ehr_id, vo_id, version, ahead)
            .await
    }

    /// Reads the COMPOSITION `vo_id` at `version`, or its latest, before its
    /// EHR's verdict is final, for [`Self::composition_judged_on`]. It notes,
    /// records and returns nothing of the version (#5959).
    pub async fn composition_ahead(
        &self,
        vo_id: VoId,
        version: Option<&ObjectVersionId>,
    ) -> VersionAhead {
        let at = match version.map(components).transpose() {
            Ok(Some((_, tree))) => JudgedAt::Tree(vo_id, tree),
            Ok(None) => JudgedAt::Current(vo_id),
            Err(error) => {
                return VersionAhead(Err(Unread::Argument(VersionIdError::into_argument(error))));
            }
        };
        VersionAhead(self.version_judged(at).await.map_err(Unread::Store))
    }

    /// The COMPOSITION `vo_id` of `ehr_id` at `version`, or its latest, as
    /// `ahead` read it, judged by `access` on the object's own facts, the
    /// refusal noted for the access record.
    pub async fn composition_judged_on(
        &self,
        access: &RequestAccess,
        ehr_id: EhrId,
        vo_id: VoId,
        version: Option<&ObjectVersionId>,
        ahead: VersionAhead,
    ) -> JudgedRead {
        let missing =
            || ServiceError::not_found(Missing::Composition, format!("COMPOSITION {vo_id}"));
        let judged = match ahead.0 {
            Ok(judged) => Ok(judged),
            Err(Unread::Argument(error)) => return JudgedRead::Admitted(Err(error)),
            Err(Unread::Store(error)) => Err(error),
        };
        let (facts, stored) = match judged {
            Ok(Some((facts, stored))) => (facts, stored),
            Ok(None) => return JudgedRead::Unheld(missing()),
            Err(error) => return JudgedRead::Unjudged(error),
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

    /// The head facts of the object `at` names and the version it addresses,
    /// for a read by version id from the version copy once the store
    /// confirms it, otherwise read in one statement and kept in the copy.
    ///
    /// A held version is served only when one indexed read finds its object's
    /// head where it was read and its content digest unchanged; the
    /// restriction and access delay marks and the settings revision the read
    /// finds replace the held ones, so the evaluator judges the store's facts
    /// of this moment, as the full read would (owner decision on #5891).
    ///
    /// # Errors
    /// The storage error of either read.
    async fn version_judged(
        &self,
        at: JudgedAt,
    ) -> Result<Option<(HeadFacts, Option<StoredVersion>)>, ServiceError> {
        let JudgedAt::Tree(vo_id, tree) = at else {
            return Ok(read_judged(&self.pool, at)
                .await?
                .map(|j| (j.facts, j.stored)));
        };
        let key = (vo_id, tree);
        // NOTE: our own design (#6416), a copy read before reads selected episodes holds none.
        let held = self.version_copy.get(key).await.filter(|held| {
            held.facts.episodes.is_some() || !crate::access_eval::episodes::selected()
        });
        if let Some(held) = held {
            let marks = read_marks(&self.pool, vo_id, held.stored.sys_version).await?;
            if let Some(marks) = marks
                && marks.head == held.head
                && marks.digest.as_deref() == Some(held.digest.as_slice())
            {
                let mut facts = held.facts.clone();
                facts.revision = marks.revision.clone();
                let mut stored = held.stored.clone();
                stored.restricted_at = marks.restricted_at;
                stored.person_withheld = marks.person_withheld;
                stored.access_revision = marks.revision;
                return Ok(Some((facts, Some(stored))));
            }
            self.version_copy.forget(key).await;
        }
        let read = read_judged(&self.pool, at).await?;
        if let Some(judged) = &read
            && let (Some(stored), Some(digest)) = (&judged.stored, &judged.digest)
            && judged.facts.ehr_id.is_some()
        {
            let held = Held {
                facts: judged.facts.clone(),
                stored: stored.clone(),
                head: judged.head,
                digest: digest.clone(),
            };
            self.version_copy.keep(key, held).await;
        }
        Ok(read.map(|j| (j.facts, j.stored)))
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
        let body = self.served_body(vo_id, read, raw.raw_json)?;
        Ok(RawServiceResponse {
            body,
            meta: Some(meta),
        })
    }
}
