// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The Commit tab's staged-change model and the CONTRIBUTION envelope it
//! assembles.
//!
//! Component-free and unit-tested, compiled on BOTH targets: the browser
//! stages and checks a change, the server fn assembles the body it posts,
//! and both answer from this one module.
//!
//! The envelope is the RELAXED `NewContribution` the released operation defines
//! (ITS-REST `specifications/operations/contribution_create.yaml` +
//! `specifications/schemas/ehr/NewContribution.yaml`): `versions[]` of
//! `UPDATE_VERSION` (`preceding_version_uid`?, `lifecycle_state`, `data`,
//! `commit_audit`) plus the change-set `audit`. `audit` and every
//! `versions[i].commit_audit` are `UPDATE_AUDIT` objects — "Clients SHOULD send
//! `_type: \"UPDATE_AUDIT\"`" (`specifications/schemas/common/UpdateAudit.yaml`)
//! — each requiring `change_type` + `committer`.
//!
//! Two codings are fixed here, both from the openEHR support terminology
//! (`docs/specs/openehr/TERM/docs/SupportTerminology/master04-representation.adoc`
//! §`terminology`): the `audit_change_type` group for a member's change type and
//! `532|complete|` of the `version_lifecycle_state` group for every member the
//! viewer authors — it commits whole documents, never an `incomplete` or
//! `deleted` member.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694)"
)]

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The `version_lifecycle_state` code every viewer-authored member carries.
const LIFECYCLE_COMPLETE: &str = "532";

/// Which openEHR change one staged row commits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StagedKind {
    /// A brand-new COMPOSITION — a member with no `preceding_version_uid`.
    CompositionCreate,
    /// A new version of an existing COMPOSITION.
    CompositionAmend,
    /// A new version of the EHR's `EHR_STATUS`.
    StatusModify,
}

impl StagedKind {
    /// Returns the operator-facing name of this kind.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::CompositionCreate => "Composition — create",
            Self::CompositionAmend => "Composition — amend",
            Self::StatusModify => "EHR status — modify",
        }
    }

    /// Returns the `<select>` option value for this kind.
    #[must_use]
    pub fn as_value(self) -> &'static str {
        match self {
            Self::CompositionCreate => "create",
            Self::CompositionAmend => "amend",
            Self::StatusModify => "status",
        }
    }

    /// Returns the kind a `<select>` option value names, defaulting to
    /// [`StagedKind::CompositionCreate`] for anything unrecognized.
    #[must_use]
    pub fn from_value(value: &str) -> Self {
        match value {
            "amend" => Self::CompositionAmend,
            "status" => Self::StatusModify,
            _ => Self::CompositionCreate,
        }
    }

    /// Whether a member of this kind supersedes an existing version.
    #[must_use]
    pub fn supersedes(self) -> bool {
        !matches!(self, Self::CompositionCreate)
    }

    /// The change types the wire accepts for this kind, most natural first.
    ///
    /// A member with no `preceding_version_uid` may only be a `249|creation|`
    /// ("the modification type does not match the operation - i.e. first
    /// version of a MODIFICATION" is the released `400` trigger, ITS-REST
    /// `specifications/responses/400_CONTRIBUTION.yaml`), and an `EHR_STATUS`
    /// member is accepted only as a modification or an amendment (the official
    /// accepted-combination matrix, CNF
    /// `docs/specs/openehr/CNF/docs/platform_test_schedule/master08*`
    /// §`EHR_STATUS` CONTRIBUTION Commit Data Sets).
    #[must_use]
    pub fn change_types(self) -> &'static [ChangeType] {
        match self {
            Self::CompositionCreate => &[ChangeType::Creation],
            Self::CompositionAmend => &[ChangeType::Amendment, ChangeType::Modification],
            Self::StatusModify => &[ChangeType::Modification, ChangeType::Amendment],
        }
    }

    /// The change type a freshly picked kind starts on.
    #[must_use]
    pub fn default_change_type(self) -> ChangeType {
        match self.change_types().first() {
            Some(first) => *first,
            None => ChangeType::Modification,
        }
    }
}

/// A code of the openEHR `audit_change_type` group, as a member's change type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeType {
    /// `249|creation|`.
    Creation,
    /// `250|amendment|`.
    Amendment,
    /// `251|modification|`.
    Modification,
}

impl ChangeType {
    /// Returns the `audit_change_type` code string.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Creation => "249",
            Self::Amendment => "250",
            Self::Modification => "251",
        }
    }

    /// Returns the group's rubric for this code — the `DV_CODED_TEXT.value`.
    #[must_use]
    pub fn rubric(self) -> &'static str {
        match self {
            Self::Creation => "creation",
            Self::Amendment => "amendment",
            Self::Modification => "modification",
        }
    }

    /// Returns the change type a rubric names, defaulting to
    /// [`ChangeType::Modification`] for anything unrecognized.
    #[must_use]
    pub fn from_rubric(rubric: &str) -> Self {
        match rubric {
            "creation" => Self::Creation,
            "amendment" => Self::Amendment,
            _ => Self::Modification,
        }
    }
}

/// One pending change in the staging area.
///
/// Viewer-session state only: the list lives in the tab's component state, so
/// navigating away discards it (the viewer stores nothing of its own). Every
/// field is fixed-size-safe so the row crosses the server-fn boundary on the
/// 32-bit WASM target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StagedChange {
    /// The row's stable identity — the staging sequence number, carried IN the
    /// row so `<For>` keys on a datum rather than a position.
    pub seq: u32,
    /// Which change this row commits.
    pub kind: StagedKind,
    /// The `commit_audit.change_type` this member commits under.
    pub change_type: ChangeType,
    /// The `OBJECT_VERSION_ID` this member supersedes; empty for a create.
    pub preceding_version_uid: String,
    /// The operator-facing name of what this row changes.
    pub target: String,
    /// The member's `data` — a canonical-JSON document, as text.
    pub body: String,
}

/// Returns the label a staged COMPOSITION create carries.
///
/// The picked template id when there is one; otherwise the body's own
/// `archetype_details.template_id.value` (a canonical COMPOSITION carries it —
/// RM `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.archetyped.adoc`),
/// then its `name.value`, then a bare type name.
#[must_use]
pub fn create_target_label(template_id: &str, body: &str) -> String {
    let picked = template_id.trim();
    if !picked.is_empty() {
        return picked.to_owned();
    }
    let Ok(doc) = serde_json::from_str::<Value>(body) else {
        return "new COMPOSITION".to_owned();
    };
    let from_template = doc
        .get("archetype_details")
        .and_then(|a| a.get("template_id"))
        .and_then(|t| t.get("value"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !from_template.is_empty() {
        return from_template.to_owned();
    }
    let from_name = doc
        .get("name")
        .and_then(|n| n.get("value"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if from_name.is_empty() {
        "new COMPOSITION".to_owned()
    } else {
        from_name.to_owned()
    }
}

/// Check one staged change before it enters the list or the envelope.
///
/// Everything beyond these three structural facts is the CDR's call — its
/// diagnostic is rendered verbatim rather than second-guessed BFF-side.
///
/// # Errors
/// The operator-facing complaint when the body is blank, is not a JSON object
/// (`versions[i].data` is a `_type`-discriminated COMPOSITION / `EHR_STATUS` /
/// FOLDER object — ITS-REST `specifications/schemas/ehr/UVersionable.yaml`),
/// when a superseding member carries no `preceding_version_uid`, or when a
/// create carries one.
pub fn check(change: &StagedChange) -> Result<(), String> {
    let body = change.body.trim();
    if body.is_empty() {
        return Err("the document is empty — paste or load the body to commit".to_owned());
    }
    let value: Value = serde_json::from_str(body).map_err(|e| format!("not valid JSON: {e}"))?;
    if !value.is_object() {
        return Err("the document must be a JSON object — a canonical openEHR document".to_owned());
    }
    let preceding = change.preceding_version_uid.trim();
    if change.kind.supersedes() && preceding.is_empty() {
        return Err(
            "no preceding version is known yet — pick the target and wait for it to load"
                .to_owned(),
        );
    }
    if !change.kind.supersedes() && !preceding.is_empty() {
        return Err(
            "a creation commits a NEW versioned object, so it carries no preceding version"
                .to_owned(),
        );
    }
    Ok(())
}

/// Returns the envelope `audit.change_type` for a change set.
///
/// All-creation reads as a creation, anything else as a modification: the
/// CONTRIBUTION audit's change type is the aggregate of its members' and "may
/// sometimes be approximate, and is not expected to be used as a computable
/// value" (RM
/// `docs/specs/openehr/RM/docs/common/master06-change_control_package.adoc`
/// §Contributions).
#[must_use]
pub fn envelope_change_type(changes: &[StagedChange]) -> ChangeType {
    if changes
        .iter()
        .all(|change| change.change_type == ChangeType::Creation)
    {
        ChangeType::Creation
    } else {
        ChangeType::Modification
    }
}

/// Assemble the `NewContribution` envelope the staging area posts.
///
/// `committer` names the `PARTY_IDENTIFIED` on the change-set audit and on
/// every member's `commit_audit` (both are `UPDATE_AUDIT` objects requiring
/// `change_type` + `committer` — ITS-REST
/// `specifications/schemas/common/UpdateAudit.yaml`). A blank `description`
/// omits the optional attribute rather than sending an empty `DV_TEXT`.
///
/// # Errors
/// The operator-facing complaint when the change set is empty, when the
/// committer is blank, or when a member fails [`check`] — prefixed with the
/// row it belongs to, because the CDR's own refusal does not name a member
/// index.
pub fn contribution_body(
    changes: &[StagedChange],
    committer: &str,
    description: &str,
) -> Result<String, String> {
    if changes.is_empty() {
        return Err("stage at least one change before committing".to_owned());
    }
    let committer = committer.trim();
    if committer.is_empty() {
        return Err("a committer is required on a CONTRIBUTION audit".to_owned());
    }
    let mut versions = Vec::with_capacity(changes.len());
    for (position, change) in changes.iter().enumerate() {
        check(change).map_err(|complaint| {
            format!(
                "change {} ({} — {}): {complaint}",
                position.saturating_add(1),
                change.kind.label(),
                change.target
            )
        })?;
        versions.push(update_version(change, committer)?);
    }
    let mut audit = update_audit(envelope_change_type(changes), committer);
    let description = description.trim();
    if !description.is_empty()
        && let Some(object) = audit.as_object_mut()
    {
        drop(object.insert(
            "description".to_owned(),
            json!({ "_type": "DV_TEXT", "value": description }),
        ));
    }
    serde_json::to_string(&json!({ "versions": versions, "audit": audit }))
        .map_err(|e| format!("the contribution could not be serialized: {e}"))
}

/// One `UPDATE_VERSION` member of the envelope.
fn update_version(change: &StagedChange, committer: &str) -> Result<Value, String> {
    let data: Value = serde_json::from_str(change.body.trim())
        .map_err(|e| format!("the document is not valid JSON: {e}"))?;
    let mut member = json!({
        "lifecycle_state": coded_text(LIFECYCLE_COMPLETE, "complete"),
        "data": data,
        "commit_audit": update_audit(change.change_type, committer),
    });
    let preceding = change.preceding_version_uid.trim();
    if !preceding.is_empty()
        && let Some(object) = member.as_object_mut()
    {
        drop(object.insert(
            "preceding_version_uid".to_owned(),
            json!({ "_type": "OBJECT_VERSION_ID", "value": preceding }),
        ));
    }
    Ok(member)
}

/// An `UPDATE_AUDIT` carrying `change_type` + `committer`.
fn update_audit(change_type: ChangeType, committer: &str) -> Value {
    json!({
        "_type": "UPDATE_AUDIT",
        "change_type": coded_text(change_type.code(), change_type.rubric()),
        "committer": { "_type": "PARTY_IDENTIFIED", "name": committer },
    })
}

/// A `DV_CODED_TEXT` over the `openehr` terminology.
fn coded_text(code: &str, rubric: &str) -> Value {
    json!({
        "_type": "DV_CODED_TEXT",
        "value": rubric,
        "defining_code": {
            "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
            "code_string": code,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::{
        ChangeType, StagedChange, StagedKind, check, contribution_body, create_target_label,
        envelope_change_type,
    };
    use serde_json::Value;

    /// A minimal canonical COMPOSITION body, as the CDR's own template example
    /// serves it.
    const COMPOSITION: &str = r#"{
        "_type": "COMPOSITION",
        "name": {"_type": "DV_TEXT", "value": "Minimal"},
        "archetype_node_id": "openEHR-EHR-COMPOSITION.minimal.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": {"_type": "ARCHETYPE_ID", "value": "openEHR-EHR-COMPOSITION.minimal.v1"},
            "rm_version": "1.2.0",
            "template_id": {"_type": "TEMPLATE_ID", "value": "minimal_evaluation.en.v1"}
        }
    }"#;

    /// The EHR's served `EHR_STATUS`, verbatim.
    const STATUS: &str = r#"{
        "_type": "EHR_STATUS",
        "uid": {"_type": "OBJECT_VERSION_ID", "value": "01a0::ferroehr.local::1"},
        "is_queryable": true,
        "is_modifiable": true
    }"#;

    fn create() -> StagedChange {
        StagedChange {
            seq: 1,
            kind: StagedKind::CompositionCreate,
            change_type: ChangeType::Creation,
            preceding_version_uid: String::new(),
            target: "minimal_evaluation.en.v1".to_owned(),
            body: COMPOSITION.to_owned(),
        }
    }

    fn status_modify() -> StagedChange {
        StagedChange {
            seq: 2,
            kind: StagedKind::StatusModify,
            change_type: ChangeType::Modification,
            preceding_version_uid: "01a0::ferroehr.local::1".to_owned(),
            target: "EHR_STATUS".to_owned(),
            body: STATUS.to_owned(),
        }
    }

    #[test]
    fn the_envelope_is_the_wire_shape_the_cdr_accepts() {
        // The exact body verified first-hand against a composed CDR: a
        // COMPOSITION creation plus an EHR_STATUS modification, answered 201
        // with two OBJECT_REFs.
        let body = contribution_body(
            &[create(), status_modify()],
            "Dr Viewer",
            "Encounter recorded; status refreshed",
        )
        .expect("the envelope assembles");
        let doc: Value = serde_json::from_str(&body).expect("assembled JSON");

        let versions = doc["versions"].as_array().expect("versions array");
        assert_eq!(versions.len(), 2);

        // Member 1: a creation, no preceding version.
        assert_eq!(versions[0]["lifecycle_state"]["value"], "complete");
        assert_eq!(
            versions[0]["lifecycle_state"]["defining_code"]["code_string"],
            "532"
        );
        assert_eq!(
            versions[0]["lifecycle_state"]["defining_code"]["terminology_id"]["value"],
            "openehr"
        );
        assert_eq!(versions[0]["data"]["_type"], "COMPOSITION");
        assert!(versions[0].get("preceding_version_uid").is_none());
        assert_eq!(versions[0]["commit_audit"]["_type"], "UPDATE_AUDIT");
        assert_eq!(
            versions[0]["commit_audit"]["change_type"]["value"],
            "creation"
        );
        assert_eq!(
            versions[0]["commit_audit"]["change_type"]["defining_code"]["code_string"],
            "249"
        );
        assert_eq!(
            versions[0]["commit_audit"]["committer"]["_type"],
            "PARTY_IDENTIFIED"
        );
        assert_eq!(
            versions[0]["commit_audit"]["committer"]["name"],
            "Dr Viewer"
        );

        // Member 2: a modification naming the version it supersedes.
        assert_eq!(
            versions[1]["preceding_version_uid"]["_type"],
            "OBJECT_VERSION_ID"
        );
        assert_eq!(
            versions[1]["preceding_version_uid"]["value"],
            "01a0::ferroehr.local::1"
        );
        assert_eq!(versions[1]["data"]["_type"], "EHR_STATUS");
        assert_eq!(
            versions[1]["commit_audit"]["change_type"]["defining_code"]["code_string"],
            "251"
        );

        // The change-set audit: UPDATE_AUDIT, the aggregate change type, the
        // committer, the description.
        assert_eq!(doc["audit"]["_type"], "UPDATE_AUDIT");
        assert_eq!(doc["audit"]["change_type"]["value"], "modification");
        assert_eq!(
            doc["audit"]["change_type"]["defining_code"]["code_string"],
            "251"
        );
        assert_eq!(doc["audit"]["committer"]["name"], "Dr Viewer");
        assert_eq!(
            doc["audit"]["description"]["value"],
            "Encounter recorded; status refreshed"
        );
        assert_eq!(doc["audit"]["description"]["_type"], "DV_TEXT");
        // The envelope carries no client-supplied uid: the CDR mints one.
        assert!(doc.get("uid").is_none());
    }

    #[test]
    fn an_all_creation_change_set_carries_a_creation_envelope_audit() {
        let mut second = create();
        second.seq = 7;
        assert_eq!(
            envelope_change_type(&[create(), second.clone()]),
            ChangeType::Creation
        );
        let body = contribution_body(&[create(), second], "u", "").expect("assembles");
        let doc: Value = serde_json::from_str(&body).expect("assembled JSON");
        assert_eq!(doc["audit"]["change_type"]["value"], "creation");
        // A blank description omits the optional attribute entirely.
        assert!(doc["audit"].get("description").is_none());
    }

    #[test]
    fn an_amendment_member_carries_its_own_code() {
        let mut amend = status_modify();
        amend.kind = StagedKind::CompositionAmend;
        amend.change_type = ChangeType::Amendment;
        amend.body = COMPOSITION.to_owned();
        let body = contribution_body(&[amend], "u", "").expect("assembles");
        let doc: Value = serde_json::from_str(&body).expect("assembled JSON");
        assert_eq!(
            doc["versions"][0]["commit_audit"]["change_type"]["defining_code"]["code_string"],
            "250"
        );
        assert_eq!(
            doc["versions"][0]["commit_audit"]["change_type"]["value"],
            "amendment"
        );
    }

    #[test]
    fn an_empty_change_set_or_a_blank_committer_is_refused_before_any_round_trip() {
        assert!(contribution_body(&[], "u", "").is_err());
        let message = contribution_body(&[create()], "   ", "").expect_err("blank committer");
        assert!(message.contains("committer"), "{message}");
    }

    #[test]
    fn a_failing_member_names_its_row_because_the_cdr_refusal_does_not() {
        let mut broken = create();
        broken.body = "[]".to_owned();
        let message =
            contribution_body(&[status_modify(), broken], "u", "").expect_err("a broken member");
        assert!(message.starts_with("change 2 ("), "{message}");
        assert!(message.contains("must be a JSON object"), "{message}");
    }

    #[test]
    fn check_refuses_the_three_structural_defects() {
        let mut blank = create();
        blank.body = "  ".to_owned();
        assert!(check(&blank).expect_err("blank body").contains("empty"));

        let mut malformed = create();
        malformed.body = "{".to_owned();
        assert!(
            check(&malformed)
                .expect_err("malformed body")
                .contains("not valid JSON")
        );

        let mut unseeded = status_modify();
        unseeded.preceding_version_uid = String::new();
        assert!(
            check(&unseeded)
                .expect_err("no preceding version")
                .contains("preceding version")
        );

        let mut creation_with_preceding = create();
        creation_with_preceding.preceding_version_uid = "01a0::sys::1".to_owned();
        assert!(
            check(&creation_with_preceding)
                .expect_err("a creation may not supersede")
                .contains("NEW versioned object")
        );

        assert!(check(&create()).is_ok());
        assert!(check(&status_modify()).is_ok());
    }

    #[test]
    fn every_kind_offers_only_change_types_the_wire_accepts() {
        assert_eq!(
            StagedKind::CompositionCreate.change_types(),
            &[ChangeType::Creation]
        );
        assert_eq!(
            StagedKind::CompositionAmend.change_types(),
            &[ChangeType::Amendment, ChangeType::Modification]
        );
        // The official EHR_STATUS accepted matrix is modification | amendment.
        assert_eq!(
            StagedKind::StatusModify.change_types(),
            &[ChangeType::Modification, ChangeType::Amendment]
        );
        assert_eq!(
            StagedKind::StatusModify.default_change_type(),
            ChangeType::Modification
        );
        assert!(!StagedKind::CompositionCreate.supersedes());
        assert!(StagedKind::CompositionAmend.supersedes());
        assert!(StagedKind::StatusModify.supersedes());
    }

    #[test]
    fn kind_and_change_type_round_trip_through_their_select_values() {
        for kind in [
            StagedKind::CompositionCreate,
            StagedKind::CompositionAmend,
            StagedKind::StatusModify,
        ] {
            assert_eq!(StagedKind::from_value(kind.as_value()), kind);
        }
        assert_eq!(
            StagedKind::from_value("nonsense"),
            StagedKind::CompositionCreate
        );
        for change_type in [
            ChangeType::Creation,
            ChangeType::Amendment,
            ChangeType::Modification,
        ] {
            assert_eq!(ChangeType::from_rubric(change_type.rubric()), change_type);
        }
        assert_eq!(
            ChangeType::from_rubric("nonsense"),
            ChangeType::Modification
        );
    }

    #[test]
    fn a_create_row_is_labelled_by_the_pick_then_the_body() {
        assert_eq!(create_target_label("picked.v1", COMPOSITION), "picked.v1");
        assert_eq!(
            create_target_label("", COMPOSITION),
            "minimal_evaluation.en.v1"
        );
        assert_eq!(
            create_target_label("", r#"{"name":{"value":"Vitals"}}"#),
            "Vitals"
        );
        assert_eq!(create_target_label("", "{}"), "new COMPOSITION");
        assert_eq!(create_target_label("", "not json"), "new COMPOSITION");
    }
}
