// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHDS Art. 8 restriction and Art. 10 opt-out as directives of the EHR's
//! own `EHR_ACCESS`, and the marks the read paths filter on.
//!
//! Regulation (EU) 2025/327 Art. 8 lets a person restrict the access of
//! health professionals to all or parts of their data, invisibly to them, and
//! Art. 10 lets a Member State's law give an opt-out from access through the
//! access services, reversibly (`docs/law/eu/ehds/text.html`). Each is a
//! `restriction` or `opt_out` directive of the `access_directives` settings
//! (`DirectiveKind` of the access control terminology), so it is versioned
//! with the `EHR_ACCESS` (RM ehr `ehr_access.adoc` §`EHR_ACCESS` Class) and
//! travels in an EHR Extract.
//!
//! A request is judged by the access evaluator, which reads every attribute of
//! a directive for the caller who asks ([`crate::access_eval`]). The marks of
//! [`crate::storage::access_marks`] serve the readers that have no caller to
//! judge: the event outbox withholds what they mark. Every `EHR_ACCESS`
//! commit, import, archive load and repair projects the directives in force
//! onto them in its own transaction ([`project`]), and every boot projects
//! every stored `EHR_ACCESS` again. With no caller, the projection reads a
//! directive as binding everyone: a data scope it cannot place covers the
//! whole EHR, and a directive's callers, operations, purposes and exceptions
//! narrow nothing.
//!
//! No openEHR spec governs either right, the directives or the projection: our
//! own design (#5487).

use openehr_base::v1_3::base_types::identification::object_id::ObjectId;
use openehr_base::v1_3::base_types::identification::object_ref::ObjectRef;
use openehr_base::v1_3::base_types::identification::uid_based_id::UidBasedId;
use openehr_rm::draft::access_control::access_control::access_directive::AccessDirective;
use openehr_rm::draft::access_control::terminology::{DirectiveKind, Term as _};
use serde_json::Value;
use sqlx::PgConnection;

use crate::db::acquire::acquire;
use crate::error::ServiceError;
use crate::ids::{EhrId, VoId};
use crate::service::FerroEhrService;
use crate::service::ehr::access_settings::EhrAccessSettings;
use crate::storage::access_marks::{self, Marks};

/// The marks the settings of one EHR imply at `now`.
///
/// Settings held closed (they do not read as the scheme) restrict and opt out
/// the whole EHR, since a privacy control that cannot be read is never read
/// as permissive.
#[must_use]
pub(crate) fn marks_of(settings: Option<&EhrAccessSettings>, now: jiff::Timestamp) -> Marks {
    let directives = match settings {
        None => return Marks::default(),
        Some(EhrAccessSettings::HeldClosed) => {
            return Marks {
                opted_out: true,
                restricted_ehr: true,
                restricted_objects: Vec::new(),
            };
        }
        Some(EhrAccessSettings::Directives(settings)) => {
            settings.directives.as_deref().unwrap_or_default()
        }
    };
    let mut marks = Marks::default();
    for directive in directives.iter().filter(|d| in_force(d, now)) {
        if DirectiveKind::OptOut.is(&directive.kind) {
            marks.opted_out = true;
        } else if DirectiveKind::Restriction.is(&directive.kind) {
            match restricted_objects(directive) {
                Some(objects) => marks.restricted_objects.extend(objects),
                None => marks.restricted_ehr = true,
            }
        }
    }
    marks.restricted_objects.sort_unstable();
    marks.restricted_objects.dedup();
    marks
}

/// Why the projection refuses `settings`, naming the first defect: a
/// restriction whose data scope holds a reference that names no versioned
/// object, which neither the marks nor the evaluator could place.
#[must_use]
pub(crate) fn refused(settings: &EhrAccessSettings) -> Option<String> {
    let EhrAccessSettings::Directives(settings) = settings else {
        return None;
    };
    let directives = settings.directives.as_deref().unwrap_or_default();
    for (index, directive) in directives.iter().enumerate() {
        let restriction = DirectiveKind::Restriction.is(&directive.kind);
        let scopes = directive.data.as_deref().unwrap_or_default();
        for (at, scope) in scopes.iter().enumerate().filter(|_| restriction) {
            let objects = scope.objects.as_deref().unwrap_or_default();
            if let Some((place, object)) = objects
                .iter()
                .enumerate()
                .find(|(_, object)| object_id(object).is_none())
            {
                return Some(format!(
                    "directives[{index}].data[{at}].objects[{place}]: {} names no versioned object",
                    object_text(object)
                ));
            }
        }
    }
    None
}

/// Whether `directive` is in force at `now`: it is unless its `time_validity`
/// has a readable upper bound that has passed.
fn in_force(directive: &AccessDirective, now: jiff::Timestamp) -> bool {
    let Some(validity) = &directive.time_validity else {
        return true;
    };
    if validity.upper_unbounded {
        return true;
    }
    let Some(upper) = validity
        .upper
        .as_ref()
        .and_then(|upper| crate::party_binding::grants::derive::instant(&upper.value))
    else {
        return true;
    };
    if validity.upper_included {
        now <= upper
    } else {
        now < upper
    }
}

/// The versioned objects a directive's data scope names, or `None` when its
/// scope is the whole EHR or holds anything but objects this server can place.
fn restricted_objects(directive: &AccessDirective) -> Option<Vec<uuid::Uuid>> {
    let scopes = directive.data.as_deref()?;
    let mut objects = Vec::new();
    for scope in scopes {
        let only_objects = scope.templates.is_none()
            && scope.archetypes.is_none()
            && scope.categories.is_none()
            && scope.sensitivities.is_none()
            && scope.origins.is_none()
            && scope.recorded.is_none();
        if !only_objects {
            return None;
        }
        for object in scope.objects.as_deref().unwrap_or_default() {
            objects.push(object_id(object)?);
        }
    }
    (!objects.is_empty()).then_some(objects)
}

/// The id text of an `OBJECT_REF` whose id is a `HIER_OBJECT_ID` or an
/// `OBJECT_VERSION_ID`; `None` for a reference of any other id class.
fn id_text(object: &ObjectRef) -> Option<&str> {
    match object {
        ObjectRef::ObjectRef(data) => match &data.id {
            ObjectId::HierObjectId(id) => Some(id.value()),
            ObjectId::ObjectVersionId(id) => Some(id.value()),
            _ => None,
        },
        ObjectRef::LocatableRef(locatable) => match &locatable.id {
            UidBasedId::HierObjectId(id) => Some(id.value()),
            UidBasedId::ObjectVersionId(id) => Some(id.value()),
        },
        ObjectRef::AccessGroupRef(_) | ObjectRef::PartyRef(_) => None,
    }
}

/// The versioned-object id an `OBJECT_REF` names, by its id or the head of its
/// version id; `None` for a reference to anything else.
pub(crate) fn object_id(object: &ObjectRef) -> Option<uuid::Uuid> {
    id_text(object)
        .and_then(|text| VoId::from_uid_head(text).ok())
        .map(|vo_id| vo_id.0)
}

/// An `OBJECT_REF` as a refusal names it: its id, or its class.
fn object_text(object: &ObjectRef) -> String {
    id_text(object).map_or_else(
        || "a reference to a party or an access group".to_owned(),
        |text| format!("{text:?}"),
    )
}

/// Reads the current `EHR_ACCESS` settings of `ehr_id` on `tx`.
async fn current_settings(
    tx: &mut PgConnection,
    ehr_id: EhrId,
) -> Result<Option<EhrAccessSettings>, ServiceError> {
    let body =
        crate::storage::version_repo::ehr_access::current_ehr_access_body(tx, ehr_id).await?;
    let Some(body) = body else {
        return Ok(None);
    };
    let access: Value = serde_json::from_str(&body)
        .map_err(|e| ServiceError::internal("read a stored EHR_ACCESS body", e))?;
    Ok(EhrAccessSettings::held_or_read(&access))
}

/// Projects the directives in force in the current `EHR_ACCESS` of `ehr_id`
/// onto its marks, in the caller's transaction.
///
/// # Errors
/// [`ServiceError::BadRequest`] (`400`) naming the first defect [`refused`]
/// finds, which rolls the commit back; the storage errors of the read and the
/// write.
pub(crate) async fn project(tx: &mut PgConnection, ehr_id: EhrId) -> Result<(), ServiceError> {
    let settings = current_settings(tx, ehr_id).await?;
    if let Some(defect) = settings.as_ref().and_then(refused) {
        return Err(ServiceError::bad_request(
            format!("EHR_ACCESS.settings.{defect}"),
            Unprojectable,
        ));
    }
    let marks = marks_of(settings.as_ref(), jiff::Timestamp::now());
    access_marks::write(tx, ehr_id, &marks).await?;
    Ok(())
}

/// Settings the projection cannot honour as written.
#[derive(Debug, thiserror::Error)]
#[error("the restriction and opt_out directives cannot be projected as written")]
struct Unprojectable;

impl FerroEhrService {
    /// Projects the directives of every EHR whose current `EHR_ACCESS`
    /// carries settings onto its marks, each EHR in its own transaction, and
    /// returns how many it projected: the boot's refill after a migration
    /// cleared the marks.
    ///
    /// Settings `project` would refuse are projected here as far as they
    /// read, an unplaceable object scope covering the whole EHR, and logged,
    /// since they are already stored.
    ///
    /// # Errors
    /// The storage errors of the read and the writes.
    pub async fn refill_access_marks(&self) -> Result<usize, ServiceError> {
        let bodies =
            crate::storage::version_repo::ehr_access::ehr_access_bodies_with_settings(&self.pool)
                .await?;
        let now = jiff::Timestamp::now();
        for (ehr_id, body) in &bodies {
            let access: Value = serde_json::from_str(body)
                .map_err(|e| ServiceError::internal("read a stored EHR_ACCESS body", e))?;
            let settings = EhrAccessSettings::held_or_read(&access);
            if let Some(defect) = settings.as_ref().and_then(refused) {
                tracing::warn!(%ehr_id, %defect, "stored directives the projection refuses are projected fail-closed");
            }
            let mut conn = acquire(&self.pool).await?;
            let mut tx = sqlx::Connection::begin(&mut *conn).await?;
            access_marks::write(&mut tx, *ehr_id, &marks_of(settings.as_ref(), now)).await?;
            tx.commit().await?;
        }
        Ok(bodies.len())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::party_binding::policy_mark::TERMINOLOGY;

    const OBJECT: &str = "8849182c-82ad-4088-a07f-48ead4180515";

    fn term(code: &str) -> Value {
        json!({ "_type": "DV_CODED_TEXT", "value": code,
                "defining_code": { "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": TERMINOLOGY },
                    "code_string": code } })
    }

    /// A directive of `kind` that an emergency lifts, with `extra` members.
    fn directive(kind: &str, extra: &Value) -> Value {
        let mut d = json!({
            "_type": "ACCESS_DIRECTIVE",
            "name": { "_type": "DV_TEXT", "value": "Directive" },
            "archetype_node_id": "at0001",
            "kind": term(kind),
            "effect": term(if kind == "consent" { "permit" } else { "deny" }),
            "emergency_override": true
        });
        if let (Some(d), Some(extra)) = (d.as_object_mut(), extra.as_object()) {
            d.extend(extra.clone());
        }
        d
    }

    fn settings(directives: &[Value]) -> EhrAccessSettings {
        let mut settings = json!({ "_type": "ACCESS_DIRECTIVE_SETTINGS",
                                   "default_access": term("open") });
        if !directives.is_empty() {
            settings["directives"] = json!(directives);
        }
        let access = json!({ "_type": "EHR_ACCESS", "settings": settings });
        EhrAccessSettings::from_ehr_access(&access)
            .expect("the settings read")
            .expect("settings")
    }

    fn objects(ids: &[&str]) -> Value {
        let refs: Vec<Value> = ids
            .iter()
            .map(|id| {
                json!({ "_type": "OBJECT_REF", "namespace": "local",
                              "type": "VERSIONED_COMPOSITION",
                              "id": { "_type": "HIER_OBJECT_ID", "value": id } })
            })
            .collect();
        json!({ "data": [{ "objects": refs }] })
    }

    fn validity(upper: &str) -> Value {
        json!({ "time_validity": { "_type": "DV_INTERVAL",
            "upper": { "_type": "DV_DATE_TIME", "value": upper },
            "lower_unbounded": true, "upper_unbounded": false,
            "lower_included": false, "upper_included": true } })
    }

    fn now() -> jiff::Timestamp {
        "2026-11-13T12:00:00Z".parse().expect("an instant")
    }

    #[test]
    fn no_settings_and_no_directives_mark_nothing() {
        assert_eq!(marks_of(None, now()), Marks::default());
        assert_eq!(marks_of(Some(&settings(&[])), now()), Marks::default());
        let consent = directive("consent", &json!({}));
        assert_eq!(
            marks_of(Some(&settings(&[consent])), now()),
            Marks::default()
        );
    }

    #[test]
    fn a_whole_record_restriction_and_an_opt_out_mark_the_ehr() {
        let marks = marks_of(
            Some(&settings(&[
                directive("restriction", &json!({})),
                directive("opt_out", &json!({})),
            ])),
            now(),
        );
        assert!(marks.restricted_ehr && marks.opted_out);
        assert_eq!(marks.restricted_objects, Vec::<uuid::Uuid>::new());
    }

    #[test]
    fn an_object_scoped_restriction_marks_its_objects_once() {
        let version = format!("{OBJECT}::local.ehrbase.org::2");
        let marks = marks_of(
            Some(&settings(&[
                directive("restriction", &objects(&[OBJECT])),
                directive("restriction", &objects(&[&version])),
            ])),
            now(),
        );
        assert!(!marks.restricted_ehr && !marks.opted_out);
        assert_eq!(
            marks.restricted_objects,
            vec![OBJECT.parse::<uuid::Uuid>().expect("a uuid")]
        );
    }

    /// A data scope the marks cannot place covers the whole EHR, and so does a
    /// stored reference that names no object.
    #[test]
    fn a_scope_beyond_objects_restricts_the_whole_ehr() {
        let category = json!({ "data": [{ "objects": objects(&[OBJECT])["data"][0]["objects"],
                                          "templates": ["lab.v1"] }] });
        let foreign = json!({ "data": [{ "objects": [{ "_type": "PARTY_REF",
            "namespace": "demographic", "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": OBJECT } }] }] });
        for extra in [category, foreign] {
            let marks = marks_of(Some(&settings(&[directive("restriction", &extra)])), now());
            assert!(marks.restricted_ehr, "{extra}");
        }
    }

    /// A directive whose validity ended marks nothing; one ending later, or
    /// whose end does not read, is in force.
    #[test]
    fn a_directive_is_in_force_until_its_validity_ends() {
        let ended = directive("opt_out", &validity("2026-11-01T00:00:00Z"));
        assert!(!marks_of(Some(&settings(&[ended])), now()).opted_out);
        for upper in ["2026-12-01T00:00:00Z", "2026-12"] {
            let running = directive("opt_out", &validity(upper));
            assert!(
                marks_of(Some(&settings(&[running])), now()).opted_out,
                "{upper}"
            );
        }
    }

    #[test]
    fn settings_held_closed_restrict_and_opt_out_the_whole_ehr() {
        let marks = marks_of(Some(&EhrAccessSettings::HeldClosed), now());
        assert!(marks.restricted_ehr && marks.opted_out);
    }

    /// A directive no emergency may lift projects like any other: the
    /// evaluator honours `emergency_override` for each caller (#5513).
    #[test]
    fn a_directive_no_emergency_lifts_is_projected() {
        let mut absolute = directive("opt_out", &json!({}));
        absolute["emergency_override"] = json!(false);
        let mut restriction = directive("restriction", &json!({}));
        restriction["emergency_override"] = json!(false);
        let named = settings(&[restriction, absolute]);
        assert_eq!(refused(&named), None);
        let marks = marks_of(Some(&named), now());
        assert!(marks.restricted_ehr && marks.opted_out);
    }

    /// A restriction naming an object by an id that names no versioned object
    /// is refused naming the reference.
    #[test]
    fn an_object_reference_that_names_no_object_is_refused() {
        let defect = refused(&settings(&[directive(
            "restriction",
            &objects(&["not-a-uuid"]),
        )]))
        .expect("refused");
        assert!(
            defect.starts_with("directives[0].data[0].objects[0]: "),
            "{defect}"
        );
        assert!(defect.contains("not-a-uuid"), "{defect}");
    }

    /// No configuration names an access service: it is a grant of
    /// `access_service`, which the default ceiling does not allow, and the
    /// removed role list is refused from the environment (#5521).
    #[test]
    fn an_access_service_is_a_grant_and_the_removed_role_list_is_refused() {
        use crate::party_binding::grants::operation::{GrantCeiling, Operation};
        assert!(!GrantCeiling::default().allows(Operation::AccessService));
        let env: std::collections::HashMap<String, String> = [(
            "FERROEHR__AUTHZ__RBAC__ACCESS_SERVICE_ROLES",
            "EHDS-PORTAL,EHDS-PROFESSIONAL",
        )]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        let refused = crate::config::assemble(None, &env, &[])
            .expect_err("the removed key is unknown")
            .to_string();
        assert!(refused.contains("ACCESS_SERVICE_ROLES"), "{refused}");
    }
}
