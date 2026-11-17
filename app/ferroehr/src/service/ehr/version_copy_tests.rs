// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The version copy's weight, its drop and its bound (#5778).

use serde_json::Value;
use uuid::Uuid;

use super::{Held, Key, VersionCopy};
use crate::ids::{EhrId, VoId};
use crate::storage::access_facts::HeadFacts;
use crate::storage::access_projection::Revision;
use crate::storage::version_repo::read::StoredVersion;
use crate::versioning::object_version_id::TreeId;

/// The first trunk version of `vo_id` in `ehr_id` with a body of `body`.
fn held(ehr_id: EhrId, vo_id: VoId, body: &str) -> Held {
    let now = jiff::Timestamp::now();
    Held {
        facts: HeadFacts {
            vo_id,
            ehr_id: Some(ehr_id),
            template: Some("lab.v1".to_owned()),
            archetype: Some("openEHR-EHR-COMPOSITION.report.v1".to_owned()),
            origins: None,
            recorded: Some(now),
            revision: Revision::default(),
        },
        stored: StoredVersion {
            vo_id,
            kind: "COMPOSITION".to_owned(),
            ehr_id: Some(ehr_id),
            sys_version: 1,
            trunk_version: 1,
            branch_number: 0,
            branch_version: 0,
            preceding_version_uid: None,
            other_input_version_uids: Vec::new(),
            lifecycle_state: "532".to_owned(),
            creating_system_id: "ferroehr.local".to_owned(),
            contribution_id: Uuid::now_v7(),
            audit_system_id: "ferroehr.local".to_owned(),
            audit_change_type: "249".to_owned(),
            audit_description: None,
            audit_committer: Value::Null,
            audit_attestation: None,
            time_committed: now,
            template_id: Some("lab.v1".to_owned()),
            root_archetype: Some("openEHR-EHR-COMPOSITION.report.v1".to_owned()),
            signature: None,
            signature_client_supplied: false,
            wrapped_original: None,
            stable_compatible: Some(true),
            origins: None,
            restricted_at: None,
            head_committed_at: Some(now),
            person_withheld: false,
            access_revision: Revision::default(),
            canonical: Value::Null,
            canonical_text: Some(body.to_owned()),
            attestations_at_committal: Vec::new(),
            attestations_after_committal: Vec::new(),
        },
        head: 1,
        digest: vec![0; 32],
    }
}

/// The key of the first trunk version of `vo_id`.
fn first(vo_id: VoId) -> Key {
    (vo_id, TreeId::trunk(1))
}

#[test]
fn a_version_weighs_its_body() {
    let (ehr, vo) = (EhrId(Uuid::now_v7()), VoId(Uuid::now_v7()));
    let small = held(ehr, vo, "{}").bytes();
    let large = held(ehr, vo, &"x".repeat(3000)).bytes();
    assert!(large >= small + 2998, "{large} against {small}");
}

#[tokio::test]
async fn a_version_the_store_no_longer_confirms_drops() {
    let copy = VersionCopy::new();
    let ehr = EhrId(Uuid::now_v7());
    let (one, two) = (VoId(Uuid::now_v7()), VoId(Uuid::now_v7()));
    copy.keep(first(one), held(ehr, one, "{}")).await;
    copy.keep(first(two), held(ehr, two, "{}")).await;
    copy.forget(first(one)).await;
    assert!(copy.get(first(one)).await.is_none(), "the version drops");
    assert!(copy.get(first(two)).await.is_some(), "another stays");
}

#[tokio::test]
async fn the_bound_holds_the_bytes_of_the_versions() {
    let ehr = EhrId(Uuid::now_v7());
    let body = "x".repeat(4000);
    let one = held(ehr, VoId(Uuid::now_v7()), &body).bytes();
    let copy = VersionCopy::bounded(one * 3);
    for _ in 0..10 {
        let vo = VoId(Uuid::now_v7());
        copy.keep(first(vo), held(ehr, vo, &body)).await;
    }
    copy.inner.run_pending_tasks().await;
    let (versions, bytes) = copy.held();
    assert!(versions <= 3, "{versions} versions");
    assert!(bytes <= one * 3, "{bytes} within {}", one * 3);
}
