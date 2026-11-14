//! probe
use crate::common::EXT;
use crate::hospital::build::world;
use crate::hospital::world::{ROOT, call};
use serde_json::json;

#[tokio::test]
async fn probe() {
    let w = Box::pin(world()).await;
    let _ids = crate::hospital::abac::ids(&[]);
    let _held = (
        &w.pool,
        &w.svc,
        &w.config,
        &w.other_ehr,
        &w.people["jansen"].role,
        crate::hospital::world::patch_op(json!([])),
    );
    println!("ACCESS {}", w.access);
    let a = call(
        &w.app,
        ROOT,
        "GET",
        &format!("{EXT}/ehr/{}/versioned_ehr_access/revision_history", w.ehr),
        Some("HOPERAT"),
        None,
    )
    .await;
    println!("HIST {}", a.body);
    let a = call(
        &w.app,
        ROOT,
        "GET",
        &format!("{EXT}/ehr/{}/ehr_access", w.ehr),
        Some("HOPERAT"),
        None,
    )
    .await;
    println!(
        "CUR {} {}",
        a.etag(),
        a.body.chars().take(300).collect::<String>()
    );
}
