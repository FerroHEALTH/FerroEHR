// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The projection of each EHR's `EHR_ACCESS` settings: the facts the access
//! gate and the access evaluator read, written in the transaction that
//! commits the version and rebuilt by a refill.
//!
//! `EHR_ACCESS` is the access-decision authority of an EHR (RM ehr
//! `org.openehr.rm.ehr.ehr_access.adoc` §`EHR_ACCESS` Class), and every change
//! to it is a version in a contribution (RM common
//! `master06-change_control_package.adoc` §Committal and Audits). The commit
//! of a version therefore replaces the EHR's rows
//! ([`crate::storage::access_projection`]): its settings, one
//! [`DirectiveFacts`] per directive and exception, and one [`LabelFacts`] per
//! label. Settings that do not read as the `access_directives` scheme project
//! as held closed, with no directive or label. A change to an EHR's settings
//! notifies every replica when it commits, so each drops its copy
//! (`service::ehr::access::EhrAccessCache`). No openEHR spec governs
//! the projection: our own design (#5509).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 10): EHR_ACCESS.settings is the RM-mandated \
              open slot (RM ehr access_control_settings.adoc — abstract, implementation-dependent \
              by specification)"
)]

use openehr_base::v1_3::base_types::identification::object_id::ObjectId;
use openehr_base::v1_3::base_types::identification::object_ref::ObjectRef;
use openehr_rm::draft::access_control::access_control::access_directive::AccessDirective;
use openehr_rm::draft::access_control::access_control::access_directive_settings::AccessDirectiveSettings;
use openehr_rm::draft::access_control::access_control::access_label::AccessLabel;
use openehr_rm::draft::access_control::terminology::{
    CallerKind, DirectiveKind, Effect, ForeignCode, ForeignKind, NotForeign, OperationClass,
    OutsideGroup, Relationship, Term,
};
use openehr_rm::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
use openehr_rm::v1_2::data_types::quantity::dv_interval::DvInterval;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use serde_json::Value;
use sqlx::PgConnection;

use crate::error::ServiceError;
use crate::ids::EhrId;
use crate::service::FerroEhrService;
use crate::service::ehr::access_rule::party_key;
use crate::service::ehr::access_settings::EhrAccessSettings;
use crate::storage::access_projection::{self as store, DirectiveRow, LabelRow, Rows, Source};
use crate::storage::version_repo::ehr_access::{self as heads, CurrentAccess};
use crate::versioning::Kind;
use crate::versioning::change::{Change, Committed};

/// The facts of one `ACCESS_DIRECTIVE` or exception.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectiveFacts {
    /// The position in a depth-first walk of the settings, from 0.
    pub ordinal: i32,
    /// The ordinal of the directive this one is an exception of.
    pub parent: Option<i32>,
    /// `ACCESS_DIRECTIVE.kind`.
    pub kind: DirectiveKind,
    /// `ACCESS_DIRECTIVE.effect`.
    pub effect: Effect,
    /// `ACCESS_DIRECTIVE.emergency_override`.
    pub emergency_override: bool,
    /// The party id values of `applies_to`.
    pub parties: Vec<String>,
    /// The roles of `applies_to`, by their code phrase.
    pub roles: Vec<ForeignCode>,
    /// The relationships of `applies_to`.
    pub relationships: Vec<Relationship>,
    /// The caller kinds of `applies_to`.
    pub caller_kinds: Vec<CallerKind>,
    /// `ACCESS_DIRECTIVE.operations`.
    pub operations: Vec<OperationClass>,
    /// `ACCESS_DIRECTIVE.purposes`.
    pub purposes: Vec<ForeignCode>,
    /// The categories of the data scopes.
    pub categories: Vec<ForeignCode>,
    /// The sensitivities of the data scopes.
    pub sensitivities: Vec<ForeignCode>,
    /// `ACCESS_DIRECTIVE.time_validity`.
    pub validity: Validity,
}

/// One bound of a [`Validity`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bound {
    /// The instant.
    pub at: jiff::Timestamp,
    /// Whether the instant itself is inside.
    pub included: bool,
}

/// The instants a directive is in force.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Validity {
    /// From and until the bounds; an absent bound is unbounded.
    Within {
        /// The lower bound.
        from: Option<Bound>,
        /// The upper bound.
        until: Option<Bound>,
    },
    /// A bound is not a full date and time with an offset, so no instant
    /// can be judged against it.
    Unreadable,
}

/// The facts of one `ACCESS_LABEL`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelFacts {
    /// The position in `ACCESS_DIRECTIVE_SETTINGS.labels`.
    pub ordinal: i32,
    /// The `OBJECT_REF.namespace` of the labelled object.
    pub namespace: String,
    /// The `OBJECT_REF.type`.
    pub r#type: String,
    /// The `OBJECT_REF.id` value.
    pub id: String,
    /// `ACCESS_LABEL.sensitivities`.
    pub sensitivities: Vec<ForeignCode>,
}

/// The facts of one EHR's settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    /// Every directive and exception, depth-first.
    pub directives: Vec<DirectiveFacts>,
    /// Every label.
    pub labels: Vec<LabelFacts>,
}

/// A coded value of the settings that the facts cannot hold.
#[derive(Debug, thiserror::Error)]
pub enum Unprojectable {
    /// A code outside the access control group its attribute is bound to.
    #[error(transparent)]
    Outside(#[from] OutsideGroup),
    /// A code of the access control terminology where a jurisdiction's
    /// code belongs.
    #[error(transparent)]
    NotForeign(#[from] NotForeign),
    /// More directives or labels than an ordinal can count.
    #[error("the settings hold more entries than an ordinal counts")]
    TooMany(#[from] std::num::TryFromIntError),
}

impl Facts {
    /// The facts of `settings`.
    ///
    /// # Errors
    /// [`Unprojectable`] for a code outside its group, which the write check
    /// refuses, so stored settings that hold one are held closed.
    pub fn of(settings: &AccessDirectiveSettings) -> Result<Self, Unprojectable> {
        let mut directives = Vec::new();
        for directive in settings.directives.as_deref().unwrap_or_default() {
            walk(directive, None, &mut directives)?;
        }
        let labels = settings
            .labels
            .as_deref()
            .unwrap_or_default()
            .iter()
            .enumerate()
            .map(|(ordinal, label)| label_facts(i32::try_from(ordinal)?, label))
            .collect::<Result<_, Unprojectable>>()?;
        Ok(Self { directives, labels })
    }
}

/// Pushes the facts of `directive`, an exception of `parent`, and of its
/// exceptions after it.
fn walk(
    directive: &AccessDirective,
    parent: Option<i32>,
    into: &mut Vec<DirectiveFacts>,
) -> Result<(), Unprojectable> {
    let ordinal = i32::try_from(into.len())?;
    let scopes = directive.applies_to.as_deref().unwrap_or_default();
    let data = directive.data.as_deref().unwrap_or_default();
    into.push(DirectiveFacts {
        ordinal,
        parent,
        kind: DirectiveKind::read_coded(&directive.kind)?,
        effect: Effect::read_coded(&directive.effect)?,
        emergency_override: directive.emergency_override,
        parties: scopes
            .iter()
            .filter_map(|scope| scope.party.as_ref().and_then(party_key))
            .collect(),
        roles: scopes
            .iter()
            .filter_map(|scope| scope.role.as_ref().map(code_phrase))
            .collect(),
        relationships: terms(
            scopes
                .iter()
                .filter_map(|scope| scope.relationship.as_ref()),
        )?,
        caller_kinds: terms(scopes.iter().filter_map(|scope| scope.caller_kind.as_ref()))?,
        operations: terms(directive.operations.as_deref().unwrap_or_default().iter())?,
        purposes: foreign(
            ForeignKind::Purpose,
            directive.purposes.as_deref().unwrap_or_default().iter(),
        )?,
        categories: foreign(
            ForeignKind::Category,
            data.iter()
                .flat_map(|scope| scope.categories.as_deref().unwrap_or_default()),
        )?,
        sensitivities: foreign(
            ForeignKind::Sensitivity,
            data.iter()
                .flat_map(|scope| scope.sensitivities.as_deref().unwrap_or_default()),
        )?,
        validity: directive.time_validity.as_ref().map_or(
            Validity::Within {
                from: None,
                until: None,
            },
            validity,
        ),
    });
    for exception in directive.exceptions.as_deref().unwrap_or_default() {
        walk(exception, Some(ordinal), into)?;
    }
    Ok(())
}

/// The facts of `label` at `ordinal`.
fn label_facts(ordinal: i32, label: &AccessLabel) -> Result<LabelFacts, Unprojectable> {
    let (namespace, r#type, id) = match &label.target {
        ObjectRef::AccessGroupRef(target) => {
            (&target.namespace, &target.r#type, id_value(&target.id))
        }
        ObjectRef::LocatableRef(target) => (
            &target.namespace,
            &target.r#type,
            target.id.value().to_owned(),
        ),
        ObjectRef::PartyRef(target) => (&target.namespace, &target.r#type, id_value(&target.id)),
        ObjectRef::ObjectRef(target) => (&target.namespace, &target.r#type, id_value(&target.id)),
    };
    Ok(LabelFacts {
        ordinal,
        namespace: namespace.clone(),
        r#type: r#type.clone(),
        id,
        sensitivities: foreign(ForeignKind::Sensitivity, label.sensitivities.iter())?,
    })
}

/// The value of an `OBJECT_ID`.
fn id_value(id: &ObjectId) -> String {
    match id {
        ObjectId::ArchetypeId(id) => id.value.clone(),
        ObjectId::GenericId(id) => id.value.clone(),
        ObjectId::HierObjectId(id) => id.value().to_owned(),
        ObjectId::ObjectVersionId(id) => id.value().to_owned(),
        ObjectId::TemplateId(id) => id.value.clone(),
        ObjectId::TerminologyId(id) => id.value.clone(),
    }
}

/// The terms of one access control group `codes` name.
fn terms<'a, T: Term>(
    codes: impl Iterator<Item = &'a DvCodedText>,
) -> Result<Vec<T>, OutsideGroup> {
    codes.map(T::read_coded).collect()
}

/// The codes of `kind` `codes` name.
fn foreign<'a>(
    kind: ForeignKind,
    codes: impl Iterator<Item = &'a DvCodedText>,
) -> Result<Vec<ForeignCode>, NotForeign> {
    codes.map(|coded| ForeignCode::read(kind, coded)).collect()
}

/// The code phrase of `coded`, of whatever terminology it names.
fn code_phrase(coded: &DvCodedText) -> ForeignCode {
    ForeignCode {
        terminology: coded.defining_code.terminology_id.value.clone(),
        code: coded.defining_code.code_string.clone(),
    }
}

/// The instants `interval` holds.
fn validity(interval: &DvInterval<DvDateTime>) -> Validity {
    // NOTE: our own design, a bound without a full date, time and offset names no instant.
    let bound = |unbounded: bool, at: Option<&DvDateTime>, included: bool| match at {
        Some(at) if !unbounded => at
            .value
            .parse::<jiff::Timestamp>()
            .ok()
            .map(|at| Some(Bound { at, included })),
        _ => Some(None),
    };
    match (
        bound(
            interval.lower_unbounded,
            interval.lower.as_ref(),
            interval.lower_included,
        ),
        bound(
            interval.upper_unbounded,
            interval.upper.as_ref(),
            interval.upper_included,
        ),
    ) {
        (Some(from), Some(until)) => Validity::Within { from, until },
        _ => Validity::Unreadable,
    }
}

impl Validity {
    /// The `tstzrange` literal; `None` when unreadable.
    fn range(self) -> Option<String> {
        let Self::Within { from, until } = self else {
            return None;
        };
        let lower = from.map_or(("(", String::new()), |b| {
            (if b.included { "[" } else { "(" }, b.at.to_string())
        });
        let upper = until.map_or((")", String::new()), |b| {
            (if b.included { "]" } else { ")" }, b.at.to_string())
        });
        Some(format!("{}{},{}{}", lower.0, lower.1, upper.1, upper.0))
    }
}

/// A code phrase as `terminology::code`.
fn text(code: &ForeignCode) -> String {
    format!("{}::{}", code.terminology, code.code)
}

/// The codes of `terms`.
fn codes<T: Term>(terms: &[T]) -> Vec<&'static str> {
    terms.iter().map(|term| term.code()).collect()
}

impl From<&DirectiveFacts> for DirectiveRow {
    fn from(facts: &DirectiveFacts) -> Self {
        Self {
            ordinal: facts.ordinal,
            parent: facts.parent,
            kind: facts.kind.code(),
            effect: facts.effect.code(),
            emergency_override: facts.emergency_override,
            parties: facts.parties.clone(),
            roles: facts.roles.iter().map(text).collect(),
            relationships: codes(&facts.relationships),
            caller_kinds: codes(&facts.caller_kinds),
            operations: codes(&facts.operations),
            purposes: facts.purposes.iter().map(text).collect(),
            categories: facts.categories.iter().map(text).collect(),
            sensitivities: facts.sensitivities.iter().map(text).collect(),
            validity: facts.validity.range(),
        }
    }
}

impl From<&LabelFacts> for LabelRow {
    fn from(facts: &LabelFacts) -> Self {
        Self {
            ordinal: facts.ordinal,
            target_namespace: facts.namespace.clone(),
            target_type: facts.r#type.clone(),
            target_id: facts.id.clone(),
            sensitivities: facts.sensitivities.iter().map(text).collect(),
        }
    }
}

/// The rows the settings of `access`, an `EHR_ACCESS` in canonical JSON,
/// project as.
fn rows_of(access: Option<&Value>) -> Rows {
    let Some((access, settings)) = access.and_then(|access| {
        access
            .get("settings")
            .filter(|settings| !settings.is_null())
            .map(|settings| (access, settings))
    }) else {
        return Rows::None;
    };
    let read = match EhrAccessSettings::from_ehr_access(access) {
        Ok(Some(EhrAccessSettings::Directives(read))) => *read,
        Ok(_) => return held_closed(),
        Err(error) => return unprojected(&error),
    };
    let facts = match Facts::of(&read) {
        Ok(facts) => facts,
        Err(error) => return unprojected(&error),
    };
    Rows::Settings {
        settings: store::SettingsRow {
            default_access: EhrAccessSettings::Directives(Box::new(read))
                .default_access()
                .code(),
            settings: Some(settings.to_string()),
        },
        directives: facts.directives.iter().map(DirectiveRow::from).collect(),
        labels: facts.labels.iter().map(LabelRow::from).collect(),
    }
}

/// The rows of settings that do not project, logged with `error`.
fn unprojected(error: &(dyn std::error::Error + 'static)) -> Rows {
    tracing::error!(
        error,
        "EHR_ACCESS settings do not project; the EHR is held closed"
    );
    held_closed()
}

/// The rows of an EHR held closed.
fn held_closed() -> Rows {
    Rows::Settings {
        settings: store::SettingsRow {
            default_access: EhrAccessSettings::HeldClosed.default_access().code(),
            settings: None,
        },
        directives: Vec::new(),
        labels: Vec::new(),
    }
}

/// What a commit of an `EHR_ACCESS` version projects, captured from its
/// change before the change is written.
#[derive(Debug)]
pub(crate) enum AccessCommit {
    /// The version's `EHR_ACCESS` body; `None` for a logical deletion.
    Body(Option<Value>),
}

impl AccessCommit {
    /// What `change` projects; `None` for a change of another kind and for
    /// the first version of an `EHR_ACCESS` with no settings, which replaces
    /// no row.
    pub(crate) fn of(change: &Change) -> Option<Self> {
        let has_settings =
            |canonical: &Value| canonical.get("settings").is_some_and(|s| !s.is_null());
        match change {
            Change::Create {
                kind: Kind::EhrAccess,
                canonical,
                ..
            } => has_settings(canonical).then(|| Self::Body(Some(canonical.clone()))),
            Change::Modify {
                kind: Kind::EhrAccess,
                canonical,
                ..
            } => Some(Self::Body(Some(canonical.clone()))),
            Change::Delete {
                kind: Kind::EhrAccess,
                ..
            } => Some(Self::Body(None)),
            _ => None,
        }
    }

    /// Replaces the rows of `ehr_id` with the projection of the committed
    /// version, on the commit's transaction.
    ///
    /// # Errors
    /// The storage error of the write.
    pub(crate) async fn project(
        self,
        tx: &mut PgConnection,
        ehr_id: EhrId,
        committed: &Committed,
    ) -> Result<(), ServiceError> {
        let Self::Body(body) = self;
        let source = Source {
            ehr_id,
            vo_id: committed.vo_id,
            sys_version: committed.sys_version,
        };
        store::replace(tx, source, &rows_of(body.as_ref())).await?;
        Ok(())
    }
}

/// Replaces the rows of `access` with the projection of its body.
///
/// # Errors
/// [`ServiceError`] when the stored body is not JSON, and the storage error
/// of the write.
async fn project_stored(tx: &mut PgConnection, access: CurrentAccess) -> Result<(), ServiceError> {
    let body = access
        .body
        .as_deref()
        .map(serde_json::from_str::<Value>)
        .transpose()
        .map_err(|e| ServiceError::internal("read a stored EHR_ACCESS body", e))?;
    let source = Source {
        ehr_id: access.ehr_id,
        vo_id: access.vo_id,
        sys_version: access.sys_version,
    };
    store::replace(tx, source, &rows_of(body.as_ref())).await?;
    Ok(())
}

/// Replaces the rows of `ehr_id` with the projection of its current
/// `EHR_ACCESS` version, on `tx`: the path of a write that lands versions
/// without the commit's projection, such as an import.
///
/// # Errors
/// The storage errors of the read and the write.
pub(crate) async fn project_current(
    tx: &mut PgConnection,
    ehr_id: EhrId,
) -> Result<(), ServiceError> {
    match heads::current_ehr_access(tx, ehr_id).await? {
        Some(access) => project_stored(tx, access).await,
        None => Ok(()),
    }
}

/// Which rebuild [`FerroEhrService::rebuild_ehr_access_projection`] makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rebuild {
    /// Fill the projection once: nothing when it was filled before.
    Fill,
    /// Fill it again from the stored versions.
    Refill,
}

impl FerroEhrService {
    /// Rebuilds the projection from every EHR's current `EHR_ACCESS`
    /// version and drops every replica's copy of the settings: a
    /// [`Rebuild::Fill`] only when it was never filled (the first boot after
    /// the projection's migration), a [`Rebuild::Refill`] always. It holds the
    /// lock that orders it with every commit's write of the rows.
    ///
    /// # Errors
    /// The storage errors of the reads and the writes.
    pub async fn rebuild_ehr_access_projection(
        &self,
        rebuild: Rebuild,
    ) -> Result<(), ServiceError> {
        let mut conn = crate::db::acquire::acquire(&self.pool).await?;
        let mut tx = sqlx::Connection::begin(&mut *conn).await?;
        store::lock(&mut tx).await?;
        if rebuild == Rebuild::Fill && store::filled(&mut tx).await? {
            return Ok(());
        }
        store::clear(&mut tx).await?;
        let current = heads::every_current_with_settings(&mut tx).await?;
        let count = current.len();
        for access in current {
            project_stored(&mut tx, access).await?;
        }
        store::mark_filled(&mut tx).await?;
        tx.commit().await?;
        self.ehr_access.forget_every();
        tracing::info!(ehrs = count, "the EHR_ACCESS projection is filled");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::party_binding::policy_mark::TERMINOLOGY;

    /// A term of the access control groups.
    fn term(code: &str) -> Value {
        json!({ "_type": "DV_CODED_TEXT", "value": code,
                "defining_code": { "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": TERMINOLOGY },
                    "code_string": code } })
    }

    /// A code of the jurisdiction's terminology `terminology`.
    fn foreign_term(terminology: &str, code: &str) -> Value {
        json!({ "_type": "DV_CODED_TEXT", "value": code,
                "defining_code": { "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": terminology },
                    "code_string": code } })
    }

    /// A directive of `kind` and `effect`, with `extra` members.
    fn directive(kind: &str, effect: &str, extra: &Value) -> Value {
        let mut d = json!({
            "_type": "ACCESS_DIRECTIVE",
            "name": { "_type": "DV_TEXT", "value": "Directive" },
            "archetype_node_id": "at0001",
            "kind": term(kind),
            "effect": term(effect),
            "emergency_override": false
        });
        if let (Some(d), Some(extra)) = (d.as_object_mut(), extra.as_object()) {
            d.extend(extra.clone());
        }
        d
    }

    /// Settings of `default` with `directives` and `labels`.
    fn settings(default: &str, directives: Vec<Value>, labels: Vec<Value>) -> Value {
        let mut s =
            json!({ "_type": "ACCESS_DIRECTIVE_SETTINGS", "default_access": term(default) });
        if !directives.is_empty() {
            s["directives"] = Value::Array(directives);
        }
        if !labels.is_empty() {
            s["labels"] = Value::Array(labels);
        }
        s
    }

    /// An `EHR_ACCESS` carrying `settings`.
    fn access(settings: Value) -> Value {
        let mut access = json!({ "_type": "EHR_ACCESS" });
        access["settings"] = settings;
        access
    }

    const DOCTOR: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f81";

    #[test]
    fn a_directive_and_its_exception_project_depth_first() {
        let exception = directive(
            "consent",
            "permit",
            &json!({ "applies_to": [{ "_type": "ACCESS_ACTOR_SCOPE",
                "party": { "_type": "PARTY_REF", "namespace": "local", "type": "PERSON",
                           "id": { "_type": "HIER_OBJECT_ID", "value": DOCTOR } } }] }),
        );
        let restriction = directive(
            "restriction",
            "deny",
            &json!({
                "applies_to": [{ "_type": "ACCESS_ACTOR_SCOPE", "caller_kind": term("professional") }],
                "operations": [term("read"), term("query")],
                "data": [{ "_type": "ACCESS_DATA_SCOPE",
                           "categories": [foreign_term("EHDS", "mental-health")] }],
                "time_validity": { "_type": "DV_INTERVAL",
                    "lower": { "_type": "DV_DATE_TIME", "value": "2026-11-01T00:00:00Z" },
                    "lower_unbounded": false, "upper_unbounded": true,
                    "lower_included": true, "upper_included": false },
                "exceptions": [exception]
            }),
        );
        let label = json!({ "_type": "ACCESS_LABEL",
            "target": { "_type": "OBJECT_REF", "namespace": "local", "type": "VERSIONED_COMPOSITION",
                        "id": { "_type": "HIER_OBJECT_ID", "value": DOCTOR } },
            "sensitivities": [foreign_term("v3-ActCode", "HIV")] });
        let Rows::Settings {
            settings: row,
            directives,
            labels,
        } = rows_of(Some(&access(settings(
            "restricted",
            vec![restriction],
            vec![label],
        ))))
        else {
            panic!("the settings project rows");
        };
        assert_eq!(row.default_access, "restricted");
        assert!(row.settings.is_some(), "the settings are kept");
        assert_eq!(directives.len(), 2, "the directive and its exception");
        let (deny, permit) = (&directives[0], &directives[1]);
        assert_eq!((deny.ordinal, deny.parent), (0, None));
        assert_eq!((deny.kind, deny.effect), ("restriction", "deny"));
        assert_eq!(deny.caller_kinds, ["professional"]);
        assert_eq!(deny.operations, ["read", "query"]);
        assert_eq!(deny.categories, ["EHDS::mental-health"]);
        assert_eq!(deny.validity.as_deref(), Some("[2026-11-01T00:00:00Z,)"));
        assert_eq!((permit.ordinal, permit.parent), (1, Some(0)));
        assert_eq!(permit.parties, [DOCTOR]);
        assert_eq!(permit.validity.as_deref(), Some("(,)"));
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].target_id, DOCTOR);
        assert_eq!(labels[0].sensitivities, ["v3-ActCode::HIV"]);
    }

    /// A bound that names no instant leaves the validity unreadable, which
    /// the evaluator never reads as permissive.
    #[test]
    fn a_bound_with_no_offset_is_unreadable() {
        let deny = directive(
            "consent",
            "deny",
            &json!({ "time_validity": { "_type": "DV_INTERVAL",
                "upper": { "_type": "DV_DATE_TIME", "value": "2026-11-01T00:00:00" },
                "lower_unbounded": true, "upper_unbounded": false,
                "lower_included": false, "upper_included": true } }),
        );
        let Rows::Settings { directives, .. } =
            rows_of(Some(&access(settings("open", vec![deny], vec![]))))
        else {
            panic!("the settings project rows");
        };
        assert_eq!(directives[0].validity, None);
    }

    /// Settings that do not read hold the EHR closed, and no settings
    /// project no row.
    #[test]
    fn unreadable_settings_project_held_closed() {
        let bogus =
            json!({ "_type": "ACCESS_DIRECTIVE_SETTINGS", "default_access": term("bogus") });
        assert_eq!(
            rows_of(Some(&access(bogus))),
            held_closed(),
            "settings outside the scheme are held closed"
        );
        assert_eq!(rows_of(None), Rows::None);
        assert_eq!(rows_of(Some(&access(Value::Null))), Rows::None);
    }
}
