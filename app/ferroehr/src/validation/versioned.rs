// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The RM rules a commit judges against what the repository already holds:
//! the `VERSIONED_COMPOSITION` cross-version invariants, the resolvability of
//! a `FOLDER`'s local item references, and the template a COMPOSITION
//! declares.
//!
//! The service's direct write paths and the CONTRIBUTION path in
//! [`crate::versioning::contribution`] run them inside their own commit
//! transactions.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): service-surface signatures over stored canonical \
              fragments and dynamic shapes"
)]

use openehr_base::validate::InvariantViolation;
use serde_json::Value;
use sqlx::PgConnection;

use crate::error::{ServiceError, Violation};
use crate::ids::{EhrId, VoId};
use crate::validation::category::code;

/// `VERSIONED_COMPOSITION` cross-version invariants, enforced against the
/// FIRST stored version's root (RM ehr `versioned_composition.adoc`):
///
/// - `Archetype_node_id_valid`: every version's `data.archetype_node_id`
///   equals the first version's — a versioned composition cannot switch
///   archetype;
/// - `Persistent_validity`: every version's `is_persistent` equals the first
///   version's — the persistence category (`431|persistent|`) is fixed for
///   the container's life.
///
/// A violating modification is a `422` naming the invariant; the first-version
/// root read goes through `crate::storage::node_repo`. Both write flows run
/// this, the direct update path ([`crate::service::ehr::composition`]) inline and
/// the CONTRIBUTION path from [`crate::versioning::contribution`], each in its
/// own commit transaction.
///
/// # Errors
/// [`ServiceError::Unprocessable`] naming the violated invariant (→ 422);
/// [`ServiceError::Database`] if the first-version root read fails.
pub(crate) async fn check_versioned_composition_invariants(
    tx: &mut PgConnection,
    vo_id: VoId,
    canonical: &Value,
) -> Result<(), ServiceError> {
    // NOTE: no openEHR spec governs the `spec_profile` gate — our own
    // design/extension: this write-path read compares two root fields and
    // serves nothing, so it stays ungated.
    let first_root = crate::storage::node_repo::first_version_root(tx, vo_id).await?;
    check_versioned_composition_first_root(first_root, canonical)
}

/// The pure half of [`check_versioned_composition_invariants`]: compare the
/// incoming root against the first stored version's `(archetype_node_id,
/// category code)` pair, already read — the direct update path threads it off
/// its merged pre-read instead of paying a second statement.
///
/// # Errors
/// [`ServiceError::Unprocessable`] naming the violated invariant (→ 422).
pub(crate) fn check_versioned_composition_first_root(
    first_root: Option<(Option<String>, Option<String>)>,
    canonical: &Value,
) -> Result<(), ServiceError> {
    let Some((first_ani, first_category)) = first_root else {
        // No stored content version (e.g. every prior version deleted) — no
        // first-version root to compare against.
        return Ok(());
    };
    let incoming_ani = canonical.get("archetype_node_id").and_then(Value::as_str);
    if let (Some(stored), Some(incoming)) = (first_ani.as_deref(), incoming_ani)
        && stored != incoming
    {
        return Err(ServiceError::content_invalid(
            Violation::new(format!(
                "{incoming:?} differs from the versioned object's first version {stored:?}"
            ))
            .with_path("COMPOSITION.archetype_node_id")
            .with_invariant("VERSIONED_COMPOSITION.Archetype_node_id_valid"),
        ));
    }
    let incoming_category = canonical
        .pointer("/category/defining_code/code_string")
        .and_then(Value::as_str);
    if let (Some(stored), Some(incoming)) = (first_category.as_deref(), incoming_category)
        && (stored == code::PERSISTENT) != (incoming == code::PERSISTENT)
    {
        return Err(ServiceError::content_invalid(
            Violation::new(format!(
                "{incoming} changes the persistence of the versioned object \
                 (first version: {stored}) — is_persistent is fixed across versions"
            ))
            .with_path("COMPOSITION.category")
            .with_invariant("VERSIONED_COMPOSITION.Persistent_validity"),
        ));
    }
    Ok(())
}

/// The OPT `template_id` a COMPOSITION declares
/// (`archetype_details.template_id.value`), if any: the derivation both the
/// direct composition path and the CONTRIBUTION commit path stamp
/// `version.template_id` with (the template-delete 409 guard counts that
/// column — physical deletes never orphan committed data).
pub(crate) fn composition_template_id(composition: &Value) -> Option<&str> {
    composition
        .pointer("/archetype_details/template_id/value")
        .and_then(Value::as_str)
}

/// A `FOLDER.items` `OBJECT_REF` whose `namespace` claims THIS system,
/// collected by [`collect_local_item_refs`] for target resolution.
struct LocalItemRef {
    path: String,
    namespace: String,
    id_value: String,
}

/// Collect every `items` reference in the FOLDER tree whose `namespace` is
/// `local` or this server's own system id, with its RM instance path.
fn collect_local_item_refs(folder: &Value, system_id: &str, at: &str, out: &mut Vec<LocalItemRef>) {
    if let Some(items) = folder.get("items").and_then(Value::as_array) {
        for (i, item) in items.iter().enumerate() {
            let namespace = item
                .get("namespace")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if namespace != "local" && namespace != system_id {
                continue;
            }
            out.push(LocalItemRef {
                path: format!("{at}/items[{i}]"),
                namespace: namespace.to_owned(),
                id_value: item
                    .get("id")
                    .and_then(|id| id.get("value"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            });
        }
    }
    if let Some(folders) = folder.get("folders").and_then(Value::as_array) {
        for (i, sub) in folders.iter().enumerate() {
            collect_local_item_refs(sub, system_id, &format!("{at}/folders[{i}]"), out);
        }
    }
}

/// The versioned-object root of an `OBJECT_REF.id` value — the uid before any
/// `::` qualification — when it is a uid this store can hold.
fn item_ref_root(id_value: &str) -> Option<uuid::Uuid> {
    let root = id_value.split("::").next().unwrap_or(id_value);
    // NOTE: `Result → Option` is the decision here (reliability.md class): a
    // non-UUID root is a legitimately unresolvable id, reported by the caller
    // as an unresolvable reference — not a swallowed defect.
    uuid::Uuid::parse_str(root).ok()
}

/// Refuse a FOLDER commit whose `items` contain a reference that CLAIMS this
/// system but does not resolve to a versioned object in `ehr_id`.
///
/// Applies to every lifecycle state — an unresolvable local reference is
/// *wrong* data, not *missing* data, so the `553|incomplete|` relaxation (RM
/// common master06 §Incomplete Content: "data may be missing, but it may not
/// be wrong") never reaches it. The reference's `type` facet is deliberately
/// not judged: BASE admits any ancestor name and `ANY` there.
///
/// # Errors
/// [`ServiceError::ValidationFailed`] naming every unresolvable local
/// reference at its tree path (→ 422).
pub(crate) async fn check_folder_item_refs<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    ehr_id: EhrId,
    system_id: &str,
    folder: &Value,
) -> Result<(), ServiceError> {
    // NOTE: no released openEHR text constrains FOLDER.items resolvability
    // (BASE object_ref.adoc; no ITS-REST directory 422) — own-design safety
    // net, register AMB-211: refs claiming this system must resolve.
    let mut refs = Vec::new();
    collect_local_item_refs(folder, system_id, "", &mut refs);
    if refs.is_empty() {
        return Ok(());
    }
    let roots: Vec<uuid::Uuid> = refs
        .iter()
        .filter_map(|item| item_ref_root(&item.id_value))
        .collect();
    let known: std::collections::HashSet<uuid::Uuid> =
        crate::storage::ehr_repo::existing_vo_roots(executor, ehr_id, &roots)
            .await?
            .into_iter()
            .collect();
    let violations: Vec<InvariantViolation> = refs
        .iter()
        .filter(|item| !item_ref_root(&item.id_value).is_some_and(|root| known.contains(&root)))
        .map(|item| {
            InvariantViolation::at(
                &item.path,
                format!(
                    "is an OBJECT_REF claiming this system (namespace {:?}) whose target \
                     {:?} does not resolve to a versioned object in EHR {ehr_id}; a \
                     reference into this system must resolve — foreign-namespace \
                     references are accepted unchecked",
                    item.namespace, item.id_value
                ),
            )
        })
        .collect();
    if violations.is_empty() {
        return Ok(());
    }
    Err(ServiceError::ValidationFailed(violations))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{collect_local_item_refs, item_ref_root};

    /// The `items` walker collects exactly the THIS-system namespaces
    /// (`local` + the configured system id), skips foreign and `unknown`
    /// ones, and recurses with `/folders[i]/items[j]` paths.
    #[test]
    fn local_item_ref_walker_classifies_namespaces_and_paths() {
        let item = |namespace: &str, id: &str| {
            json!({
                "_type": "OBJECT_REF",
                "namespace": namespace,
                "type": "VERSIONED_COMPOSITION",
                "id": { "_type": "HIER_OBJECT_ID", "value": id }
            })
        };
        let folder = json!({
            "_type": "FOLDER",
            "name": { "_type": "DV_TEXT", "value": "root" },
            "items": [
                item("local", "a"),
                item("my.system.id", "b"),
                item("unknown", "c"),
                item("this.system", "d"),
            ],
            "folders": [{
                "_type": "FOLDER",
                "name": { "_type": "DV_TEXT", "value": "sub" },
                "items": [ item("local", "e") ]
            }]
        });
        let mut out = Vec::new();
        collect_local_item_refs(&folder, "this.system", "", &mut out);
        let collected: Vec<(&str, &str)> = out
            .iter()
            .map(|r| (r.path.as_str(), r.id_value.as_str()))
            .collect();
        assert_eq!(
            collected,
            [
                ("/items[0]", "a"),
                ("/items[3]", "d"),
                ("/folders[0]/items[0]", "e"),
            ]
        );
    }

    /// `item_ref_root` takes the uid before any `::` qualification and only
    /// accepts a uid this store can hold.
    #[test]
    fn item_ref_root_extracts_the_versioned_object_root() {
        let root = "0192b1c2-aaaa-7bbb-8ccc-0123456789ab";
        let versioned = format!("{root}::some.system::2");
        assert_eq!(
            item_ref_root(&versioned)
                .expect("qualified uid resolves")
                .to_string(),
            root
        );
        assert_eq!(
            item_ref_root(root).expect("bare uid resolves").to_string(),
            root
        );
        assert_eq!(item_ref_root("not-a-uuid"), None);
        assert_eq!(item_ref_root(""), None);
    }
}
