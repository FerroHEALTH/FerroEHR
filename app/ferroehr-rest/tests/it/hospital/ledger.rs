// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What the access log must hold of each access a test makes: exactly one
//! entry per request, granted or refused, with the refusal's code, the step
//! that decided it and what decided it, the grounds of a granted access, and
//! the emergency mark (EHDS Annex II 3.2, Art. 9, Art. 11(5),
//! `docs/law/eu/ehds/text.html`). The entry's shape is the draft
//! `org.openehr.rm.access_log` 0.1.0; which grounds are named is our own
//! design (#5514).

use std::time::{Duration, Instant};

use ferroehr::system_log::store::AuditStore;
use ferroehr::system_log::store::chains::VerifyScope;
use serde_json::Value;

use crate::common::BASE;
use crate::hospital::world::{Answer, ROOT, World, call};

/// What one access must have written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Expect {
    /// The `refused-*` code and the step number, `None` for a granted access.
    pub(crate) refused: Option<(&'static str, u8)>,
    /// The kinds of the grounds a granted access names, beside its roles.
    pub(crate) grounds: &'static [&'static str],
    /// Whether the entry is marked an emergency.
    pub(crate) emergency: bool,
}

impl Expect {
    /// A granted access resting on `grounds`.
    pub(crate) const fn granted(grounds: &'static [&'static str]) -> Self {
        Self {
            refused: None,
            grounds,
            emergency: false,
        }
    }

    /// An access refused with `code` at `step`.
    pub(crate) const fn refused(code: &'static str, step: u8) -> Self {
        Self {
            refused: Some((code, step)),
            grounds: &[],
            emergency: false,
        }
    }

    /// The same, marked an emergency.
    pub(crate) const fn emergency(self) -> Self {
        Self {
            emergency: true,
            ..self
        }
    }
}

/// The accesses of one test, each with what it must have written.
#[derive(Debug, Default)]
pub(crate) struct Ledger {
    noted: Vec<Noted>,
}

/// One noted access: its label, its request id, what it must have written,
/// and the `EHR_ACCESS` and `ACCESS_POLICY` versions in force when it ran.
#[derive(Debug)]
struct Noted {
    label: String,
    request_id: String,
    expect: Expect,
    access: String,
    policy: String,
}

impl Ledger {
    /// Notes the access to `world` that answered `answer`, named `label`.
    pub(crate) fn note(
        &mut self,
        world: &World,
        label: impl Into<String>,
        answer: &Answer,
        expect: Expect,
    ) {
        self.noted.push(Noted {
            label: label.into(),
            request_id: answer.request_id(),
            expect,
            access: world.access.clone(),
            policy: world.policy.clone(),
        });
    }

    /// Checks every noted access against the access log, once a marker access
    /// made after them has landed (the drain writes in order), and that both
    /// audit chains verify.
    pub(crate) async fn verify(self, world: &World) {
        let marker = call(
            &world.app,
            ROOT,
            "GET",
            &format!("{BASE}/ehr/{}", world.other_ehr),
            Some("HOPERAT"),
            None,
        )
        .await;
        let marker_id = marker.request_id();
        let entries = landed(world, &marker_id).await;
        for noted in &self.noted {
            let mine: Vec<&Value> = entries
                .iter()
                .filter(|(id, _)| id.as_deref() == Some(noted.request_id.as_str()))
                .map(|(_, entry)| entry)
                .filter(|entry| entry.get("ehr_id").is_some())
                .collect();
            assert_eq!(
                mine.len(),
                1,
                "{}: one entry per access, got {mine:#?}",
                noted.label
            );
            check(noted, mine[0]);
        }
        verify_chains(world).await;
    }
}

/// Every stored access-log entry with its request id, once the entry of
/// `marker` is among them.
async fn landed(world: &World, marker: &str) -> Vec<(Option<String>, Value)> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let rows: Vec<(Option<String>, String)> = sqlx::query_as(
            "SELECT s.request_id, audit.access_log_document(r.content, sh.content, r.record_type, r.uid, \
         r.time_recorded, r.ehr_id, r.request_id, r.resource_id)::text FROM audit.access_log_record r \
             JOIN audit.access_log_search((SELECT x.last_id FROM audit.access_log_index_state x \
             WHERE x.singleton)) s ON s.chain_seq = r.chain_seq \
             LEFT JOIN audit.access_log_shared sh ON sh.digest = r.shared \
             WHERE r.record_type = 'ACCESS_LOG_ENTRY' ORDER BY r.chain_seq",
        )
        .fetch_all(&world.pool)
        .await
        .expect("read the access log");
        if rows.iter().any(|(id, _)| id.as_deref() == Some(marker)) {
            return rows
                .into_iter()
                .map(|(id, document)| {
                    (
                        id,
                        serde_json::from_str::<Value>(&document).expect("an entry"),
                    )
                })
                .collect();
        }
        assert!(Instant::now() < deadline, "the marker access never landed");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Checks `entry` against what `noted` expects of it.
fn check(noted: &Noted, entry: &Value) {
    let (label, expect) = (noted.label.as_str(), noted.expect);
    let outcome = &entry["outcome"];
    assert_eq!(
        entry["is_emergency"],
        Value::Bool(expect.emergency),
        "{label}: the emergency mark: {entry}"
    );
    let Some((code, step)) = expect.refused else {
        assert_eq!(outcome["result"]["value"], "granted", "{label}: {entry}");
        let grounds = entry["authorisations"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for kind in expect.grounds {
            let ground = grounds
                .iter()
                .find(|ground| ground["kind"]["value"] == *kind)
                .unwrap_or_else(|| panic!("{label}: a {kind} ground: {entry}"));
            match ground["name"].as_str() {
                Some("ACCESS_POLICY") => {
                    assert_eq!(ground["version"], noted.policy.as_str(), "{label}: {entry}");
                }
                Some("EHR_ACCESS") => {
                    assert_eq!(ground["version"], noted.access.as_str(), "{label}: {entry}");
                }
                _ => {}
            }
        }
        return;
    };
    assert_eq!(outcome["result"]["value"], "refused", "{label}: {entry}");
    assert_eq!(
        outcome["reason"]["defining_code"]["code_string"], code,
        "{label}: the refusal code: {entry}"
    );
    let described = outcome["description"]["value"].as_str().unwrap_or_default();
    assert!(
        described.starts_with(&format!("step {step} (")),
        "{label}: the step: {described}"
    );
    // NOTE: the procedure of #5737, steps 4 to 7 read the access policy (purpose, rules, classifications, care), 8 the default.
    let decided = match step {
        1..=3 => format!("of EHR_ACCESS {}", noted.access),
        4..=7 => format!("ACCESS_POLICY {}", noted.policy),
        8 => format!("EHR_ACCESS {}", noted.access),
        _ if code == "refused-not-represented" => "the subject the caller represents".to_owned(),
        _ => "patient context".to_owned(),
    };
    assert!(
        described.contains(&decided),
        "{label}: what decided it, {decided}: {described}"
    );
}

/// Both audit chains, the access trail and the access log, verify from
/// their start.
pub(crate) async fn verify_chains(world: &World) {
    let findings = AuditStore::new(world.pool.clone())
        .verify_chains_advancing(VerifyScope::Full)
        .await
        .expect("the chains verify");
    assert!(findings.is_empty(), "the chains: {findings:?}");
}
