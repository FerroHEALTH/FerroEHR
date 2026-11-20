// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The controller's exclusions in `EHR_ACCESS.settings`: written and read by
//! the controller alone.
//!
//! The draft access control decision procedure, Reading and Writing the
//! Settings: "`_marking_` and `_exclusions_` MUST be changed only by the
//! controller. A version committed by anyone else MUST carry them
//! unchanged", and the subject, a representative and a gate-keeper read the
//! settings "never `_exclusions_`". FerroEHR's controller is a caller holding
//! `admin_role`. A version another writer commits carries the current
//! exclusions forward: a writer who cannot read them sends none, and the
//! commit writes them in; one that sends exclusions other than the current
//! ones is refused whole. Every read of an `EHR_ACCESS` by anyone but the
//! controller leaves them out. No openEHR spec governs who the controller
//! is: our own design (#6077).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 10): EHR_ACCESS.settings is the RM-mandated \
              open slot (RM ehr access_control_settings.adoc — abstract, implementation-dependent \
              by specification)"
)]

use openehr_base::serde_support::OpenSubtype;
use serde_json::Value;

use crate::error::ServiceError;
use crate::versioning::Kind;
use crate::versioning::audit::AuditInput;
use crate::versioning::body::Body;
use crate::versioning::change::Change;

/// The member of the settings that holds the exclusions.
const EXCLUSIONS: &str = "exclusions";

/// The exclusions the stored `EHR_ACCESS` canonical JSON `access` carries;
/// `None` when it carries none.
#[must_use]
pub fn held(access: &Value) -> Option<&Value> {
    access
        .get("settings")
        .and_then(|settings| settings.get(EXCLUSIONS))
        .filter(|exclusions| !exclusions.is_null())
}

/// Carries `current`, the exclusions of the current version, into each
/// `EHR_ACCESS` version of `changes`, which a caller who is no controller
/// commits.
///
/// # Errors
/// [`ServiceError::Forbidden`] (`403`) for a version that names exclusions
/// other than the current ones, or that drops its settings while the current
/// version holds exclusions; the error of a settings carrier that does not
/// rebuild.
pub(in crate::service) fn carry(
    current: Option<&Value>,
    changes: &mut [(AuditInput, Change)],
) -> Result<(), ServiceError> {
    for (_, change) in changes.iter_mut() {
        let (Change::Create {
            kind: Kind::EhrAccess,
            body,
            ..
        }
        | Change::Modify {
            kind: Kind::EhrAccess,
            body,
            ..
        }) = change
        else {
            continue;
        };
        carry_into(current, body)?;
    }
    Ok(())
}

/// [`carry`] into one version's `body`.
fn carry_into(current: Option<&Value>, body: &mut Body) -> Result<(), ServiceError> {
    match body {
        Body::EhrAccess(access) => {
            let Some(carrier) = access.settings.as_ref() else {
                return unchanged(current, None);
            };
            let sent = carrier.member(EXCLUSIONS).filter(|e| !e.is_null());
            if sent.is_some() {
                return unchanged(current, sent);
            }
            let Some(current) = current else {
                return Ok(());
            };
            let mut members = carrier.members().clone();
            members.insert(EXCLUSIONS.to_owned(), current.clone());
            let rebuilt = OpenSubtype::new(carrier.type_name(), members)
                .map_err(|e| ServiceError::internal("carry the controller's exclusions", e))?;
            access.settings = Some(rebuilt);
            Ok(())
        }
        Body::Canonical(value) => {
            let Some(settings) = value.get_mut("settings").filter(|s| s.is_object()) else {
                return unchanged(current, None);
            };
            let sent = settings.get(EXCLUSIONS).filter(|e| !e.is_null()).cloned();
            if sent.is_some() {
                return unchanged(current, sent.as_ref());
            }
            if let (Some(current), Some(settings)) = (current, settings.as_object_mut()) {
                settings.insert(EXCLUSIONS.to_owned(), current.clone());
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Refuses a version whose exclusions, `sent`, are not the current ones.
fn unchanged(current: Option<&Value>, sent: Option<&Value>) -> Result<(), ServiceError> {
    if current == sent {
        return Ok(());
    }
    Err(ServiceError::not_permitted(
        "only the controller changes the exclusions of EHR_ACCESS.settings; a version anyone \
         else commits carries them unchanged (draft access control decision procedure, Reading \
         and Writing the Settings)",
    ))
}

/// The `EHR_ACCESS` canonical JSON text `body` without the controller's
/// exclusions, for a reader who is no controller; `None` when it carries
/// none, so the stored text is served as it is.
///
/// # Errors
/// [`ServiceError::Internal`] when a stored body is no JSON.
pub fn withheld(body: &str) -> Result<Option<String>, ServiceError> {
    if !body.contains("\"exclusions\"") {
        return Ok(None);
    }
    let mut access: Value = serde_json::from_str(body)
        .map_err(|e| ServiceError::internal("read a stored EHR_ACCESS body", e))?;
    let removed = access
        .get_mut("settings")
        .and_then(Value::as_object_mut)
        .and_then(|settings| settings.remove(EXCLUSIONS));
    if removed.is_none() {
        return Ok(None);
    }
    serde_json::to_string(&access)
        .map(Some)
        .map_err(|e| ServiceError::internal("render an EHR_ACCESS body", e))
}

/// `access` without the controller's exclusions, for a reader who is no
/// controller.
pub fn withhold(access: &mut Value) {
    if let Some(settings) = access.get_mut("settings").and_then(Value::as_object_mut) {
        settings.remove(EXCLUSIONS);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn access(exclusions: Option<Value>) -> Value {
        let mut settings = json!({ "_type": "ACCESS_DIRECTIVE_SETTINGS",
            "default_access": { "_type": "DV_CODED_TEXT", "value": "open",
                "defining_code": { "terminology_id": { "value": "openehr_access" },
                    "code_string": "open" } } });
        if let Some(exclusions) = exclusions {
            settings["exclusions"] = exclusions;
        }
        json!({ "_type": "EHR_ACCESS", "archetype_node_id": "openEHR-EHR-EHR_ACCESS.generic.v1",
                "name": { "_type": "DV_TEXT", "value": "EHR access" }, "settings": settings })
    }

    fn excluding(who: &str) -> Value {
        json!([{ "_type": "ACCESS_DIRECTIVE", "archetype_node_id": "id1",
                 "name": { "_type": "DV_TEXT", "value": format!("not {who}") } }])
    }

    #[test]
    fn a_version_with_no_exclusions_carries_the_current_ones() {
        let current = excluding("the ex-partner");
        let mut body = Body::Canonical(access(None));
        carry_into(Some(&current), &mut body).expect("carried");
        let Body::Canonical(carried) = body else {
            panic!("canonical")
        };
        assert_eq!(held(&carried), Some(&current));
    }

    #[test]
    fn a_version_naming_other_exclusions_is_refused() {
        let current = excluding("the ex-partner");
        for sent in [excluding("the neighbour"), excluding("the ex-partner")] {
            let same = sent == current;
            let result = carry_into(Some(&current), &mut Body::Canonical(access(Some(sent))));
            assert_eq!(result.is_ok(), same, "{result:?}");
        }
        let added = carry_into(
            None,
            &mut Body::Canonical(access(Some(excluding("anyone")))),
        );
        assert!(
            matches!(added, Err(ServiceError::Forbidden(_))),
            "{added:?}"
        );
        let mut dropped = access(None);
        dropped["settings"] = Value::Null;
        let dropped = carry_into(Some(&current), &mut Body::Canonical(dropped));
        assert!(
            matches!(dropped, Err(ServiceError::Forbidden(_))),
            "{dropped:?}"
        );
    }

    #[test]
    fn a_reader_who_is_no_controller_reads_no_exclusions() {
        let text = access(Some(excluding("the ex-partner"))).to_string();
        let withheld = withheld(&text).expect("reads").expect("withheld");
        assert!(!withheld.contains("exclusions"), "{withheld}");
        assert!(withheld.contains("default_access"), "{withheld}");
        assert_eq!(withheld_none(&access(None).to_string()), None);
    }

    fn withheld_none(text: &str) -> Option<String> {
        withheld(text).expect("reads")
    }
}
