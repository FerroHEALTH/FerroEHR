// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The copy of created EHRs holds at most its cap, and the create's answer
//! of an EHR the cap evicted is the store's body, byte for byte (#5926).

use super::created_ehrs;
use crate::memory::held;
use crate::service::FerroEhrService;
use crate::service::version_update::WriteRequest;

#[tokio::test]
async fn the_cap_holds_and_an_evicted_create_answers_the_stored_body() {
    let db = testkit::db().await.expect("testkit database");
    let mut svc = FerroEhrService::new(&crate::db::domain::DomainPools::from_shared(&db.pool()));
    let cap = 2;
    svc.created_ehr_repr = created_ehrs(cap);
    let mut created = Vec::new();
    for _ in 0..8 {
        let (ehr_id, _) = Box::pin(svc.create_ehr_meta(WriteRequest::new(None)))
            .await
            .expect("the EHR is created");
        created.push(ehr_id);
    }
    svc.created_ehr_repr.run_pending_tasks().await;
    let kept = held(&svc.created_ehr_repr);
    assert!(kept.entries <= cap, "{kept:?} within {cap}");
    assert!(
        kept.entries > 0 && kept.bytes > 0,
        "{kept:?} keeps the newest"
    );
    for ehr_id in created {
        let answered = svc
            .ehr_created_object(ehr_id)
            .await
            .expect("the create's answer");
        let stored = svc.ehr_read(ehr_id).await.expect("the stored EHR").body;
        assert_eq!(answered, stored, "EHR {ehr_id}: the cap changes no answer");
    }
}
