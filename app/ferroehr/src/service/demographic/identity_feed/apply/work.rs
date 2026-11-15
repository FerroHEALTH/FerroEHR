// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What one apply does to the patient `PERSON`s, change by change: the
//! bodies it edits, the identifiers it has moved so far, and the bindings and
//! merges it records on the apply's transaction. No specification governs
//! these rules: our own design (PMIR 1.6.0 §2:3.93.4.1.3).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): the stored canonical party bodies an \
              applied identity change edits and commits"
)]

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use sqlx::PgConnection;
use uuid::Uuid;

use super::super::body::{self, SubjectRef};
use super::super::plan;
use super::super::{ChangeKind, FeedEvent, Identifier, Refusal};
use super::{ApplyError, Claimed};
use crate::db::acquire::acquire;
use crate::error::ServiceError;
use crate::ids::VoId;
use crate::service::FerroEhrService;
use crate::service::demographic::types::PartyKind;
use crate::service::demographic::validate::Container;
use crate::versioning::Kind;
use crate::versioning::change::{Change, WriteEnvelope};
use crate::versioning::contribution::typed_decode_gate;
use crate::versioning::lifecycle::state;

/// A patient `PERSON` an apply edits.
#[derive(Debug)]
struct Person {
    template: String,
    tree: crate::versioning::object_version_id::TreeId,
    lifecycle: String,
    original: Value,
    body: Value,
    removes: bool,
}

/// One apply's view of the store: the bodies it edits and the identifiers it
/// has moved so far.
pub(super) struct Work<'a> {
    service: &'a FerroEhrService,
    message: &'a Claimed,
    path: String,
    persons: BTreeMap<Uuid, Person>,
    removed: BTreeSet<(Uuid, String, String)>,
    added: BTreeSet<(Uuid, String, String)>,
    released_types: BTreeMap<String, String>,
}

impl<'a> Work<'a> {
    /// The view of an apply of `message` on `service`, the patient
    /// identifiers at `path`.
    pub(super) fn new(service: &'a FerroEhrService, message: &'a Claimed, path: String) -> Self {
        Self {
            service,
            message,
            path,
            persons: BTreeMap::new(),
            removed: BTreeSet::new(),
            added: BTreeSet::new(),
            released_types: BTreeMap::new(),
        }
    }
}

impl Work<'_> {
    /// The announcement of `pix` in this message, other than a merge.
    fn announcement(&self, pix: &str) -> Option<&FeedEvent> {
        self.message
            .events
            .iter()
            .find(|event| event.id == pix && event.kind != ChangeKind::Merge)
    }

    /// Loads `person` for editing; `None` when it is deleted or gone.
    async fn person(&mut self, person: Uuid) -> Result<Option<&mut Person>, ApplyError> {
        if !self.persons.contains_key(&person) {
            let service = self.service;
            let Some(current) = service
                .party_current(PartyKind::Person, VoId(person))
                .await?
            else {
                return Ok(None);
            };
            if current.deleted {
                return Ok(None);
            }
            let read = service
                .read_party(PartyKind::Person, VoId(person), None, None)
                .await?;
            let mut conn = acquire(&service.demographic_pool)
                .await
                .map_err(ServiceError::from)?;
            let lifecycle: String =
                sqlx::query_scalar("SELECT lifecycle_state FROM vo_head WHERE vo_id = $1")
                    .bind(person)
                    .fetch_one(&mut *conn)
                    .await?;
            let template = read
                .body
                .pointer("/archetype_details/template_id/value")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            self.persons.insert(
                person,
                Person {
                    template,
                    tree: current.tree,
                    lifecycle,
                    original: read.body.clone(),
                    body: read.body,
                    removes: false,
                },
            );
        }
        Ok(self.persons.get_mut(&person))
    }

    /// Applies one change of the message.
    pub(super) async fn event(
        &mut self,
        tx: &mut PgConnection,
        event: &FeedEvent,
    ) -> Result<(), ApplyError> {
        if event.id.is_empty() {
            return Err(ApplyError::Refused(Refusal::Ambiguous));
        }
        match event.kind {
            ChangeKind::Merge => self.merge(tx, event).await,
            ChangeKind::Create | ChangeKind::Update | ChangeKind::Delete => {
                let known = plan::identity(tx, &event.id).await?;
                if let Some(survivor) = known.as_ref().and_then(|k| k.merged_into.clone()) {
                    match (event.active, event.replaced_by.as_deref()) {
                        (Some(false), Some(named)) if named == survivor => return Ok(()),
                        (Some(true), None) => self.unmerge(tx, &event.id).await?,
                        _ => return Err(ApplyError::Refused(Refusal::MergedStateMissing)),
                    }
                }
                self.announce(tx, event).await
            }
        }
    }

    /// Records the identity `pix` bound to `person`, merged into `merged_into`.
    async fn bind(
        tx: &mut PgConnection,
        pix: &str,
        person: Option<Uuid>,
        merged_into: Option<&str>,
    ) -> Result<(), ApplyError> {
        sqlx::query(
            "INSERT INTO pmir_identity (pix_id, person_id, merged_into) VALUES ($1, $2, $3) \
             ON CONFLICT (pix_id) DO UPDATE SET \
               person_id = COALESCE(EXCLUDED.person_id, pmir_identity.person_id), \
               merged_into = EXCLUDED.merged_into, updated_at = now()",
        )
        .bind(pix)
        .bind(person)
        .bind(merged_into)
        .execute(&mut *tx)
        .await?;
        Ok(())
    }

    /// A merge of `event.id` into the identity its `replaced-by` link names:
    /// the merged `PERSON`'s record subjects move onto the survivor's.
    async fn merge(&mut self, tx: &mut PgConnection, event: &FeedEvent) -> Result<(), ApplyError> {
        let engine = self.service.identifiers.as_deref();
        let Some(survivor) = event
            .replaced_by
            .clone()
            .filter(|s| !s.is_empty() && *s != event.id)
        else {
            return Err(ApplyError::Refused(Refusal::Survivor));
        };
        let known = plan::identity(tx, &event.id).await?;
        if known.as_ref().and_then(|k| k.merged_into.as_deref()) == Some(survivor.as_str()) {
            return Ok(());
        }
        let survivor_known = plan::identity(tx, &survivor).await?;
        if survivor_known
            .as_ref()
            .is_some_and(|k| k.merged_into.is_some())
        {
            return Err(ApplyError::Refused(Refusal::Survivor));
        }
        let named = plan::resolve(tx, engine, known.as_ref(), &event.identifiers, None).await?;
        let survivor_identifiers = self
            .announcement(&survivor)
            .map(|announced| announced.identifiers.clone())
            .unwrap_or_default();
        let survivor_person = plan::resolve(
            tx,
            engine,
            survivor_known.as_ref(),
            &survivor_identifiers,
            named,
        )
        .await?;
        // NOTE: PMIR 1.6.0 §2:3.93.4.1.2.4, the deprecated Patient may carry none of its former
        // identifiers; the survivor's announcement does, on a PERSON other than the survivor's.
        let merged = match (named, survivor_person) {
            (None, Some(survivor)) => {
                plan::resolve(tx, engine, None, &survivor_identifiers, Some(survivor)).await?
            }
            (named, _) => named,
        };
        let mut subjects: Vec<SubjectRef> = Vec::new();
        if let (Some(from), Some(to)) = (merged, survivor_person)
            && from != to
        {
            let mut moved = Vec::new();
            if let Some(person) = self.person(from).await? {
                subjects = body::record_subjects(&person.body);
                moved = body::take_record_subjects(&mut person.body, &subjects);
            }
            if let Some(person) = self.person(to).await? {
                body::put_record_subjects(&mut person.body, moved);
            }
        }
        Self::bind(tx, &survivor, survivor_person, None).await?;
        Self::bind(tx, &event.id, merged, Some(&survivor)).await?;
        sqlx::query(
            "INSERT INTO pmir_merge (id, message_id, merged_pix, survivor_pix, merged_person, \
             survivor_person, subjects) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(Uuid::now_v7())
        .bind(&self.message.message_id)
        .bind(&event.id)
        .bind(&survivor)
        .bind(merged)
        .bind(survivor_person)
        .bind(sqlx::types::Json(&subjects))
        .execute(&mut *tx)
        .await?;
        Ok(())
    }

    /// The unmerge of `pix`: the record subjects its standing merge moved go
    /// back to its `PERSON`, and it stands unmerged.
    async fn unmerge(&mut self, tx: &mut PgConnection, pix: &str) -> Result<(), ApplyError> {
        let Some(merge) = plan::standing_merge(tx, pix).await? else {
            return Err(ApplyError::Refused(Refusal::MergedStateMissing));
        };
        if let (Some(from), Some(to)) = (merge.survivor_person, merge.merged_person) {
            let mut moved = Vec::new();
            if let Some(person) = self.person(from).await? {
                moved = body::take_record_subjects(&mut person.body, &merge.subjects);
            }
            if let Some(person) = self.person(to).await? {
                body::put_record_subjects(&mut person.body, moved);
            }
        }
        sqlx::query("UPDATE pmir_merge SET reverted_by = $2 WHERE id = $1")
            .bind(merge.id)
            .bind(&self.message.message_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE pmir_identity SET merged_into = NULL, updated_at = now() WHERE pix_id = $1",
        )
        .bind(pix)
        .execute(&mut *tx)
        .await?;
        Ok(())
    }

    /// An update, a create or a delete of `event.id`: its `PERSON` carries the
    /// identifiers the Registry announces and no other.
    async fn announce(
        &mut self,
        tx: &mut PgConnection,
        event: &FeedEvent,
    ) -> Result<(), ApplyError> {
        let engine = self.service.identifiers.as_deref();
        let known = plan::identity(tx, &event.id).await?;
        let Some(holder) =
            plan::resolve(tx, engine, known.as_ref(), &event.identifiers, None).await?
        else {
            Self::bind(tx, &event.id, None, None).await?;
            return Ok(());
        };
        let announced: &[Identifier] = match event.kind {
            ChangeKind::Delete => &[],
            _ => &event.identifiers,
        };
        for row in plan::unannounced(tx, engine, holder, announced).await? {
            self.remove(holder, &row.issuer, &row.value, &row.identifier_type)
                .await?;
        }
        for identifier in announced {
            self.gain(tx, holder, identifier).await?;
        }
        if event.kind == ChangeKind::Delete {
            sqlx::query(
                "UPDATE pmir_identity SET person_id = NULL, updated_at = now() WHERE pix_id = $1",
            )
            .bind(&event.id)
            .execute(&mut *tx)
            .await?;
        } else {
            Self::bind(tx, &event.id, Some(holder), None).await?;
        }
        Ok(())
    }

    /// Removes the identifier `issuer` `value` (as stored) from `holder`.
    async fn remove(
        &mut self,
        holder: Uuid,
        issuer: &str,
        value: &str,
        identifier_type: &str,
    ) -> Result<(), ApplyError> {
        if !self
            .removed
            .insert((holder, issuer.to_owned(), value.to_owned()))
        {
            return Ok(());
        }
        self.released_types
            .insert(issuer.to_owned(), identifier_type.to_owned());
        let path = self.path.clone();
        if let Some(person) = self.person(holder).await? {
            person.removes |= body::remove_identifier(&mut person.body, &path, issuer, value);
        }
        Ok(())
    }

    /// Makes `holder` carry `identifier`: moved from the one `PERSON` whose
    /// identity gave it up, or written when no `PERSON` carries it.
    async fn gain(
        &mut self,
        tx: &mut PgConnection,
        holder: Uuid,
        identifier: &Identifier,
    ) -> Result<(), ApplyError> {
        let engine = self.service.identifiers.as_deref();
        let key = (holder, identifier.system.clone(), identifier.value.clone());
        if self.added.contains(&key) {
            return Ok(());
        }
        let rows: Vec<_> = plan::carried(tx, engine, identifier)
            .await?
            .into_iter()
            .filter(|row| {
                !self
                    .removed
                    .contains(&(row.holder, row.issuer.clone(), row.value.clone()))
            })
            .collect();
        if rows.iter().any(|row| row.holder == holder) {
            return Ok(());
        }
        let identifier_type = match rows.as_slice() {
            [] => match self.released_types.get(&identifier.system) {
                Some(known) => Some(known.clone()),
                None => plan::domain_type(tx, &identifier.system).await?,
            },
            [row] => {
                if !self.gives_up(tx, row.holder, identifier).await? {
                    return Err(ApplyError::Refused(Refusal::Contested));
                }
                let (from, issuer, value, kind) = (
                    row.holder,
                    row.issuer.clone(),
                    row.value.clone(),
                    row.identifier_type.clone(),
                );
                self.remove(from, &issuer, &value, &kind).await?;
                Some(kind)
            }
            _ => return Err(ApplyError::Refused(Refusal::Contested)),
        };
        let Some(identifier_type) = identifier_type else {
            tracing::warn!(
                issuer = %identifier.system,
                "an announced identifier of a domain no patient PERSON carries is not written: \
                 its DV_IDENTIFIER type is unknown"
            );
            return Ok(());
        };
        let value = if identifier_type.is_empty() {
            json!({ "_type": "DV_IDENTIFIER", "issuer": identifier.system,
                "id": identifier.value })
        } else {
            json!({ "_type": "DV_IDENTIFIER", "issuer": identifier.system,
                "id": identifier.value, "type": identifier_type })
        };
        let path = self.path.clone();
        let service = self.service;
        if let Some(person) = self.person(holder).await?
            && !body::push_identifier(&mut person.body, &path, &value)
        {
            person.body = service
                .mapped_party_body(Some(&person.body), &person.template, &[(path, value)])
                .await?;
        }
        self.added.insert(key);
        Ok(())
    }

    /// Whether the identity `from`'s `PERSON` is bound to gives up
    /// `identifier` in this message: it is merged, merged here, deleted, or
    /// announced without it.
    async fn gives_up(
        &self,
        tx: &mut PgConnection,
        from: Uuid,
        identifier: &Identifier,
    ) -> Result<bool, ApplyError> {
        let Some(pix) = plan::identity_of(tx, from).await? else {
            return Ok(false);
        };
        if plan::identity(tx, &pix)
            .await?
            .is_some_and(|identity| identity.merged_into.is_some())
        {
            return Ok(true);
        }
        Ok(self.announcement(&pix).is_some_and(|announced| {
            announced.kind == ChangeKind::Delete || !announced.identifiers.contains(identifier)
        }))
    }

    /// The versions the apply commits, the `PERSON`s that give something up
    /// first, each checked as a direct write is.
    pub(super) async fn changes(self) -> Result<Vec<Change>, ApplyError> {
        let mut edited: Vec<(Uuid, Person)> = self
            .persons
            .into_iter()
            .filter(|(_, person)| person.body != person.original)
            .collect();
        edited.sort_by_key(|(id, person)| (!person.removes, *id));
        let mut changes = Vec::with_capacity(edited.len());
        for (id, person) in edited {
            let incomplete = matches!(person.lifecycle.as_str(), state::INCOMPLETE);
            typed_decode_gate(Kind::Person, &person.body, incomplete)?;
            self.service
                .demographic_commit_check(
                    Kind::Person,
                    &person.body,
                    incomplete,
                    Some(Container::Existing(VoId(id))),
                )
                .await?;
            let envelope = WriteEnvelope {
                lifecycle_state: Some(person.lifecycle),
                ..WriteEnvelope::default()
            };
            changes.push(Change::modify(
                VoId(id),
                Kind::Person,
                person.body,
                Some(person.tree),
                None,
                envelope,
            ));
        }
        Ok(changes)
    }
}
