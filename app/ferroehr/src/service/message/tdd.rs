// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! TDD (Template Data Document) import — SM `I_TDD_SERVICE.import_tdd` /
//! `import_tdds` (`docs/specs/openehr/SM/docs/UML/classes/i_tdd_service.adoc`).
//!
//! A TDD is a template-namespaced XML instance of a COMPOSITION: the root
//! element is named after the operational template and carries a `template_id`
//! attribute in the Ocean/Marand templates namespace
//! (`http://schemas.oceanehr.com/templates`); structural nodes use the template
//! node names and leaves are `rm:`-namespaced canonical RM value fragments (see
//! the corpus at
//! `docs/specs/openehr/CNF/tests/platform/robot/_resources/test_data_sets/compositions/TDD/`).
//! The document does **not** carry the archetype-node-ids, `xsi:type`s or RM
//! structural attribute names a COMPOSITION needs — those come from the OPT.
//!
//! ## Pipeline
//!
//! 1. **Envelope** — parse the XML, require the templates namespace + a
//!    `template_id`, verify the target EHR exists (`has_ehr`), and resolve the
//!    referenced operational template (an unknown template is
//!    `template_does_not_exist`, which also rejects the corpus
//!    `..__invalid_opt_doesnt_exist` case; a stored-but-unbuildable template
//!    surfaces through [`FerroEhrService::web_template_for`]).
//! 2. **Body conversion** — the OPT-guided TDD-body → canonical-COMPOSITION walk
//!    ([`openehr_sdt::flat::tdd::from_tdd`]): the template node names are matched to the
//!    `WebTemplate` node tree to supply `archetype_node_id`s, re-materialise the
//!    `HISTORY`/`EVENT`/`ITEM_TREE`/`ELEMENT` wrappers the template compacts, and
//!    parse each `rm:`-namespaced leaf into its RM datatype. A body that does not
//!    conform to the template is a typed `precondition_violation`.
//! 3. **Commit** — the produced canonical COMPOSITION goes through the normal
//!    validated [`FerroEhrService::create_composition`] path (`WebTemplate` +
//!    RM-invariant + terminology validation, contribution/audit — RM common
//!    master06 §Contributions), returning its `OBJECT_VERSION_ID`. A validation
//!    failure is `content_invalid` — never a silent partial COMPOSITION.
//!
//! NOTE: `i_tdd_service.adoc` declares no `import_tdds` signature, so it is a
//! design-filled `(UUID, Vec<String>) -> Vec<String>` and all-or-nothing: every
//! TDD is parsed and converted before any is committed, so one unconvertible TDD
//! rejects the whole batch. A flagged extension of the SM interface.
//!
//! This module reaches the templates layer through
//! [`FerroEhrService::web_template_for`] / [`FerroEhrService::get_template_xml`]
//! and the validated commit through [`FerroEhrService::create_composition`].

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 3): EHR-Extract/TDD/dump-load compose over \
              verbatim stored content (RM common master06 §Copying)"
)]

use quick_xml::Reader;
use quick_xml::events::Event;
use serde_json::Value;

use crate::ids::EhrId;
use crate::service::FerroEhrService;
use crate::service::error::ServiceError;
use crate::service::status::{CallStatusType, SmError};
use crate::service::version_update::direct_envelope;

/// Re-type a converted TDD body as the RM `COMPOSITION` the commit seam takes.
///
/// `from_tdd` emits a canonical COMPOSITION fragment; the strict canonical
/// reader is what turns it into the typed value (a conversion that produced
/// anything else is `content_invalid`, refused before any commit).
fn typed_composition(composition: &Value) -> Result<openehr_rm::prelude::Composition, SmError> {
    openehr_its::json::from_canonical_value(composition).map_err(|e| {
        SmError::new(
            CallStatusType::ContentInvalid,
            format!("the TDD did not convert to a valid COMPOSITION: {e}"),
        )
        .with_source(e)
    })
}

/// The Ocean/Marand operational-template-data XML namespace every TDD root
/// declares (the corpus TDD instances use exactly this default `xmlns`).
const TDD_TEMPLATE_NS: &str = "http://schemas.oceanehr.com/templates";

/// The structural facts read from a TDD document's root element.
struct TddEnvelope {
    /// The operational template the TDD instantiates (root `template_id`
    /// attribute), e.g. `persistent_minimal.en.v1`.
    template_id: String,
}

/// Parse the TDD XML envelope: locate the root element, require the Ocean
/// templates namespace, and read its `template_id`. A document that is not
/// well-formed XML, is not in the templates namespace, or carries no
/// `template_id` is a typed rejection (`content_invalid` /
/// `precondition_violation`).
fn parse_tdd_envelope(tdd: &str) -> Result<TddEnvelope, SmError> {
    let mut reader = Reader::from_str(tdd);

    loop {
        match reader.read_event() {
            // The first element start is the TDD root.
            Ok(Event::Start(e) | Event::Empty(e)) => {
                let mut template_id: Option<String> = None;
                let mut namespace: Option<String> = None;
                for attr in e.attributes() {
                    let attr = attr.map_err(|err| {
                        SmError::new(
                            CallStatusType::ContentInvalid,
                            format!("TDD root has malformed XML attributes: {err}"),
                        )
                        .with_source(err)
                    })?;
                    let key = attr.key.as_ref();
                    // The default-namespace declaration `xmlns="..."` (the
                    // TDD root declares the templates namespace as default).
                    if key == "xmlns" {
                        namespace = Some(attr.value.as_ref().to_owned());
                    } else if key == "template_id" {
                        template_id = Some(attr.value.as_ref().to_owned());
                    }
                }

                match namespace.as_deref() {
                    Some(TDD_TEMPLATE_NS) => {}
                    other => {
                        return Err(SmError::precondition(format!(
                            "TDD root is not in the templates namespace {TDD_TEMPLATE_NS:?} \
                             (found {other:?}); the payload is not a Template Data Document"
                        )));
                    }
                }
                let template_id = template_id.filter(|t| !t.is_empty()).ok_or_else(|| {
                    SmError::precondition("TDD root carries no non-empty template_id attribute")
                })?;
                return Ok(TddEnvelope { template_id });
            }
            Ok(Event::Eof) => {
                return Err(SmError::new(
                    CallStatusType::ContentInvalid,
                    "TDD payload has no root element (empty or not XML)",
                ));
            }
            // Skip the XML declaration, comments, leading text, etc.
            Ok(_) => {}
            Err(err) => {
                return Err(SmError::new(
                    CallStatusType::ContentInvalid,
                    format!("TDD payload is not well-formed XML: {err}"),
                )
                .with_source(err));
            }
        }
    }
}

impl FerroEhrService {
    /// SM `import_tdd`: convert a single TDD and commit it through the
    /// validated [`Self::create_composition`] path. Returns the created
    /// COMPOSITION's `OBJECT_VERSION_ID`.
    ///
    /// NOTE: this commits a PLAIN LOCAL `ORIGINAL_VERSION`, not the
    /// `IMPORTED_VERSION`/`ORIGINAL_VERSION` pair RM ehr `master04-ehr_package.adoc`
    /// §Versioning Scenarios sketches for its Case 2: nothing binds `import_tdd`
    /// to that scenario (SM `UML/classes/i_tdd_service.adoc`: "Import a TDD"),
    /// and the pair is unconstructible from a TDD — `IMPORTED_VERSION.item` is an
    /// `ORIGINAL_VERSION` whose `uid`, `contribution` and `commit_audit` are all
    /// mandatory, so minting them would fabricate the "faithful copy of its
    /// original" RM common `master06-change_control_package.adoc` §Copying requires.
    ///
    /// The spec's own mechanism for feeder provenance is the one master04 gives
    /// immediately after the case list, and it is content, not versioning: "the
    /// `AUDIT_DETAILS` is always used to document the addition of information
    /// locally, regardless of where it has come from. If there is a need to
    /// record original audit details (via the `COMPOSITION._feeder_audit_`),
    /// they become part of the content of the versioned object." A converted
    /// TDD carrying `COMPOSITION.feeder_audit` therefore keeps its feeder
    /// provenance through this path, inside the committed content, while the
    /// commit's own `AUDIT_DETAILS` documents the local addition.
    ///
    /// # Errors
    /// - `content_invalid` — the payload is not well-formed XML / has no root,
    ///   or the produced COMPOSITION fails WebTemplate/RM/terminology
    ///   validation at commit.
    /// - `precondition_violation` (`400`) — the root is not in the templates
    ///   namespace, carries no `template_id`, or the body does not conform to
    ///   the operational template.
    /// - `ehr_id_does_not_exist` — no EHR with `an_ehr_id` (`has_ehr` false).
    /// - `template_does_not_exist` — the referenced operational template is not
    ///   provisioned.
    /// - `exception` — a database fault while checking/committing.
    pub async fn import_tdd(&self, an_ehr_id: EhrId, tdd: String) -> Result<String, SmError> {
        let composition = self.prepare_one_tdd(an_ehr_id, &tdd).await?;
        // The validated commit path (WebTemplate + RM-invariant + terminology
        // validation, contribution/audit).
        // Boxed: the typed COMPOSITION envelope makes this future large enough
        // to matter on the stack (clippy `large_futures`).
        let resp = Box::pin(
            self.create_composition(an_ehr_id, direct_envelope(typed_composition(&composition)?)),
        )
        .await?;
        Ok(resp.version_uid())
    }

    /// The `import_tdds` extension: convert and commit a batch of TDDs,
    /// all-or-nothing — every TDD is parsed and converted before any is
    /// committed, so a single unconvertible TDD rejects the whole batch with
    /// nothing committed. Returns the created `OBJECT_VERSION_ID`s in input
    /// order; an EMPTY batch returns an empty list.
    ///
    /// The target-EHR precondition is checked for EVERY batch, the empty one
    /// included: `an_ehr_id` is a parameter of the operation, not of its
    /// members, so an unknown EHR is `ehr_id_does_not_exist` even when there
    /// is no member to carry the check.
    ///
    /// # Errors
    /// - `ehr_id_does_not_exist` — no EHR with `an_ehr_id`, whatever the batch
    ///   holds.
    /// - Otherwise as [`Self::import_tdd`], for any TDD in the batch (a
    ///   conversion failure rejects the batch before any commit).
    pub async fn import_tdds(
        &self,
        an_ehr_id: EhrId,
        tdds: Vec<String>,
    ) -> Result<Vec<String>, SmError> {
        self.require_tdd_target_ehr(an_ehr_id).await?;
        let mut prepared = Vec::with_capacity(tdds.len());
        for tdd in &tdds {
            prepared.push(self.prepare_one_tdd(an_ehr_id, tdd).await?);
        }
        let mut ids = Vec::with_capacity(prepared.len());
        for composition in prepared {
            // The validated commit path (as in `import_tdd`).
            // Boxed, as in `import_tdd` (clippy `large_futures`).
            let resp =
                Box::pin(self.create_composition(
                    an_ehr_id,
                    direct_envelope(typed_composition(&composition)?),
                ))
                .await?;
            ids.push(resp.version_uid());
        }
        Ok(ids)
    }

    /// Validate a TDD's envelope against a target EHR and its referenced
    /// template, then convert its body to a canonical COMPOSITION (no commit).
    ///
    /// Envelope failures are typed (`ehr_id_does_not_exist`,
    /// `template_does_not_exist`); a body that does not conform to the template
    /// is `precondition_violation`. Splitting prepare from commit is what lets
    /// [`Self::import_tdds`] convert a whole batch before committing any.
    async fn prepare_one_tdd(&self, ehr_id: EhrId, tdd: &str) -> Result<Value, SmError> {
        let envelope = parse_tdd_envelope(tdd)?;

        self.require_tdd_target_ehr(ehr_id).await?;

        // Precondition: the referenced operational template is provisioned. An
        // unknown template_id is `template_does_not_exist` (this rejects the
        // corpus `..__invalid_opt_doesnt_exist` TDD) — probed by EXISTS, so the
        // stored XML never moves for the check. A stored-but-unbuildable
        // template surfaces through `web_template_for` as content_invalid.
        if !self.template_stored(&envelope.template_id).await? {
            return Err(SmError::from(ServiceError::sm(
                CallStatusType::TemplateDoesNotExist,
                format!("template {}", envelope.template_id),
            )));
        }
        let web_template = self.web_template_for(&envelope.template_id).await?;

        // OPT-guided body → canonical COMPOSITION. A body that does not conform
        // to the template is a typed precondition_violation (never a silent
        // partial COMPOSITION).
        openehr_sdt::flat::tdd::from_tdd(tdd, &web_template).map_err(|e| {
            SmError::precondition(format!(
                "TDD body does not conform to operational template {:?}: {e}",
                envelope.template_id
            ))
            .with_source(e)
        })
    }

    /// The shared target-EHR precondition of both import operations (`has_ehr`
    /// — `i_tdd_service.adoc` takes `an_ehr_id` on the operation, not on the
    /// document).
    async fn require_tdd_target_ehr(&self, ehr_id: EhrId) -> Result<(), SmError> {
        let ehr_exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM ehr WHERE id = $1)")
            .bind(ehr_id)
            .fetch_one(&self.pool)
            .await
            .map_err(ServiceError::from)?;
        if ehr_exists {
            Ok(())
        } else {
            Err(SmError::ehr_not_found(format!("no EHR with id {ehr_id}")))
        }
    }
}
