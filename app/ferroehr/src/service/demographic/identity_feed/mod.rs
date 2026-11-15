// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The PMIR Patient Identity Consumer: FerroEHR applies the Patient Identity
//! Registry's merges, unmerges and links to the patient `PERSON`s (#5693).
//!
//! FerroEHR subscribes with ITI-94 ([`subscribe`], PMIR 1.6.0 §2:3.94) and
//! receives each ITI-93 Mobile Patient Identity Feed message on its feed route
//! ([`receive`], §2:3.93). A message is stored in `party.pmir_message` before
//! it is acknowledged, and in the same transaction every identifier and record
//! subject it splits off a `PERSON` stops resolving on every replica
//! (`pmir_suppression`, the stamp and notification of the caller facts). One
//! replica then applies the messages in arrival order under a lease
//! ([`apply`]), each as one demographic CONTRIBUTION committed in the
//! transaction that marks it applied, so a crash between receipt and commit
//! loses nothing and a redelivery changes nothing.
//!
//! What a message does, read from its content alone:
//!
//! - a **merge** (the deprecated `Patient` with `active` false and a
//!   `replaced-by` link, §2:3.93.4.1.2.4) moves the record subjects of the
//!   merged identity's `PERSON` onto the survivor's and records them;
//! - an **unmerge** (a merged identity announced `active` with no
//!   `replaced-by` link) moves the recorded record subjects back; a message
//!   about a merged identity that states neither is refused, never guessed;
//! - an **update** or **create** makes the `PERSON`'s patient identifiers the
//!   ones the Registry announces: an identifier the announcement no longer
//!   lists is removed (an unlink), and one it adds is moved from the `PERSON`
//!   whose identity gave it up, or written when no `PERSON` carries it (a
//!   link);
//! - a **delete** removes every patient identifier of the `PERSON`.
//!
//! The Registry is the authority on identity; FerroEHR keeps its clinical
//! records and the record subjects that link them. PMIR leaves the Consumer's
//! handling to "its application capabilities" (§2:3.93.4.1.3), so every rule
//! here is our own design.

pub mod apply;
pub mod config;
pub mod receive;
pub mod subscribe;

mod body;
mod plan;

use std::fmt;

use serde::{Deserialize, Serialize};

impl crate::service::FerroEhrService {
    /// This service writing a patient identifier the PMIR feed links with
    /// the `DV_IDENTIFIER.type` its domain's `[[pix.identifier_domain]]`
    /// entry names.
    #[must_use]
    pub fn with_pmir_identifier_domains(mut self, domains: &[config::IdentifierDomain]) -> Self {
        self.pmir_domains = std::sync::Arc::new(
            domains
                .iter()
                .map(|domain| {
                    (
                        domain.system.trim().to_owned(),
                        domain.identifier_type.trim().to_owned(),
                    )
                })
                .collect(),
        );
        self
    }
}

/// The client an ITI-93 message was authenticated as: the issuer and
/// `client_id` of its access token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedSender {
    /// The token's issuer.
    pub issuer: String,
    /// The token's `client_id`.
    pub client_id: String,
}

/// The kind of change one history entry reports (§2:3.93.4.1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ChangeKind {
    /// A Patient Master Identity was created.
    Create,
    /// It was updated.
    Update,
    /// It was deleted.
    Delete,
    /// It was merged into another (§2:3.93.4.1.2.4).
    Merge,
}

/// One business identifier: its assigning authority and its value.
///
/// `Debug` shows the system and never the value.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Identifier {
    /// `Identifier.system`, the identifier's domain and the `DV_IDENTIFIER`
    /// issuer a patient `PERSON` carries it under.
    pub(crate) system: String,
    /// `Identifier.value`.
    pub(crate) value: String,
}

impl fmt::Debug for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Identifier")
            .field("system", &self.system)
            .finish_non_exhaustive()
    }
}

/// One change of a received message, as it is stored until applied.
///
/// `Debug` shows the kind and the shape, never an id or an identifier.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FeedEvent {
    /// What happened.
    pub(crate) kind: ChangeKind,
    /// The Registry's `Patient.id`.
    pub(crate) id: String,
    /// The business identifiers the `Patient` carries now, each with a
    /// system; empty for a delete.
    pub(crate) identifiers: Vec<Identifier>,
    /// `Patient.active` as the message states it; `None` when the message
    /// does not state it.
    pub(crate) active: Option<bool>,
    /// The surviving `Patient.id` a `replaced-by` link names.
    pub(crate) replaced_by: Option<String>,
}

impl fmt::Debug for FeedEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FeedEvent")
            .field("kind", &self.kind)
            .field("identifiers", &self.identifiers.len())
            .field("active", &self.active)
            .field("replaced", &self.replaced_by.is_some())
            .finish_non_exhaustive()
    }
}

/// Why a stored message is refused when it is applied: a fixed text naming
/// no value, which the message row and the log carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    /// A message about a merged identity states neither its merge (`active`
    /// false, `replaced-by` the survivor) nor its revert (`active` true, no
    /// `replaced-by` link).
    #[error("a message about a merged identity states neither its merge nor its revert")]
    MergedStateMissing,
    /// More than one `PERSON` carries the identifiers of one identity.
    #[error("more than one PERSON carries the identifiers of one identity")]
    Ambiguous,
    /// An identifier the announcement adds is held by a `PERSON` whose
    /// identity still holds it.
    #[error("an announced identifier is still held by another identity's PERSON")]
    Contested,
    /// A merge names a survivor the store holds no identity of, or that is
    /// itself merged.
    #[error("a merge names a survivor that is unknown or itself merged")]
    Survivor,
    /// A merged `PERSON`'s protected identifier cannot be moved: the message
    /// carries its value nowhere.
    #[error("a protected identifier cannot be moved: the message does not carry its value")]
    Unmovable,
    /// A linked identifier's domain has no `DV_IDENTIFIER` type: no
    /// `[[pix.identifier_domain]]` entry names it and the patient `PERSON`s
    /// carry it with none or more than one.
    #[error("a linked identifier's domain has no DV_IDENTIFIER type")]
    UnknownDomainType,
    /// The contribution the message makes does not commit.
    #[error("the contribution the message makes is refused by the commit checks")]
    Commit,
}

impl Refusal {
    /// The code stored on the message row.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::MergedStateMissing => "merged-state-missing",
            Self::Ambiguous => "ambiguous",
            Self::Contested => "contested",
            Self::Survivor => "survivor",
            Self::Unmovable => "unmovable",
            Self::UnknownDomainType => "unknown-domain-type",
            Self::Commit => "commit",
        }
    }
}
