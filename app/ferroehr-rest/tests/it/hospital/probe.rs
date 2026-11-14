//! probe
use crate::common::EXT;
use crate::hospital::build::world;
use crate::hospital::world::call;

#[tokio::test]
async fn probe() {
    let w = Box::pin(world()).await;
    let _held = (
        &w.pool,
        &w.svc,
        &w.config,
        &w.other_ehr,
        &w.people["jansen"].role,
        crate::hospital::world::patch_op(serde_json::json!([])),
    );
    let a = call(
        &w.app,
        "jansen",
        "GET",
        &format!("{EXT}/message/export/{}", w.ehr),
        Some("TREAT"),
        None,
    )
    .await;
    println!("EXP {} {}", a.status, a.request_id());
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let rows: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as("SELECT chain || ' ' || operation || ' ' || action, resource_id, request_id FROM audit.access_trail ORDER BY chain_seq DESC LIMIT 5").fetch_all(&w.pool).await.unwrap_or_default();
    for r in rows {
        println!("TRAIL {r:?}");
    }
    let rows: Vec<(Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT ehr_id::text, content FROM audit.access_log_record ORDER BY chain_seq DESC LIMIT 3",
    )
    .fetch_all(&w.pool)
    .await
    .unwrap_or_default();
    for r in rows {
        println!("LOG {r:?}");
    }
}
