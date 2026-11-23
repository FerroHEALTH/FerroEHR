// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The documents a reader gets from a stored entry, against the goldens
//! written before the entry was stored as compact facts (#6322): the BALP
//! `AuditEvent` the ITI-81 search and the feed serve, the DICOM message the
//! syslog sender sends, and the search tokens of the rendering. Each entry is
//! stored through the store's own parts and read back as every reader reads
//! it. `FERROEHR_BLESS_GOLDEN=1` writes the goldens again.

use secrecy::ExposeSecret as _;

use super::super::Projection;
use super::{EHR, classified_read, events, policy};
use crate::access_log::entry::entry_of;
use crate::access_log::retention::stated;
use crate::system_log::event::AuditEvent;
use crate::system_log::render::CompiledPolicy;
use crate::system_log::retention::LogRetention;
use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;

/// The uid every golden entry carries, so its documents are deterministic.
const UID: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f00";

/// The events of the golden set: the parity set, a classified read whose
/// origins are all on the record, and a read the map did not classify.
fn golden_events(policy: &CompiledPolicy) -> Vec<(&'static str, AuditEvent, Option<&'static str>)> {
    let mut all = events(policy);
    let mut uncapped = classified_read(policy);
    uncapped.origin_count = Some(2);
    all.push(("uncapped read", uncapped, Some("subject-1")));
    let mut unclassified = classified_read(policy);
    unclassified.category = None;
    unclassified.category_map_digest = None;
    unclassified.origins = vec!["cdr.example.org".to_owned()];
    unclassified.origin_count = Some(1);
    unclassified.ehr_id = Some(EHR.to_owned());
    all.push(("unclassified read", unclassified, None));
    for (_, event, _) in &mut all {
        event.timestamp = "2026-11-13T10:15:30.123456Z".parse().expect("an instant");
    }
    all
}

/// The entry `event` writes, with the golden uid.
fn written(event: &AuditEvent, subject: Option<&str>) -> AccessLogEntry {
    let mut entry = entry_of(
        event,
        subject,
        "cdr.example.org",
        stated(
            Some(jiff::civil::date(2031, 11, 13)),
            &LogRetention::default(),
        ),
    )
    .expect("an entry");
    entry.uid = HierObjectId::new(UID).expect("a uid");
    entry
}

/// `entry` stored as the store stores it, indexed with its request and
/// resource ids, and read back as every reader reads it.
fn stored(entry: &AccessLogEntry, event: &AuditEvent) -> AccessLogEntry {
    let index = crate::access_log::intake::IndexRow {
        action: event.action.as_code(),
        outcome: 0,
        principal: None,
        patient_id: None,
        resource_id: event.object_id.clone(),
        request_id: event.request_id.clone(),
        client_ip: None,
        ehr_id: None,
        search_tokens: String::new(),
        render_policy: Vec::new(),
    };
    let document = crate::access_log::intake::IntakeEntry::read_back(entry, index)
        .expect("the entry is stored and read back");
    serde_json::from_str(&document).expect("the entry reads back")
}

/// The golden of one event: its BALP `AuditEvent`, its DICOM message and the
/// search tokens of its rendering.
fn golden(
    projection: &Projection<'_>,
    event: &AuditEvent,
    subject: Option<&str>,
) -> serde_json::Value {
    let entry = written(event, subject);
    let (recorded, recorded_subject) = projection.record(&entry, 0).expect("a record");
    let resource = crate::system_log::fhir::to_resource(
        &recorded,
        projection.context,
        recorded_subject.as_deref(),
    )
    .expect("a resource");
    let tokens: Vec<serde_json::Value> =
        crate::system_log::search::resource_tokens(resource.as_resource(), &recorded)
            .into_iter()
            .map(|token| serde_json::json!([token.parameter, token.system, token.code]))
            .collect();
    let read = stored(&entry, event);
    let balp = projection.audit_event(&read, 7).expect("a BALP AuditEvent");
    let dicom = projection.audit_message(&read, 7).expect("a DICOM message");
    serde_json::json!({
        "balp": balp,
        "dicom": String::from_utf8(dicom.expose_secret().to_vec()).expect("UTF-8"),
        "tokens": tokens,
    })
}

/// Every document a reader gets from a stored entry is the golden written
/// before the entry was stored as compact facts.
#[test]
fn a_stored_entry_renders_its_goldens() {
    let policy = policy();
    let projection = Projection {
        context: &policy.context,
        categories: &policy.categories,
    };
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/access_log/projection/tests/golden");
    let bless = std::env::var_os("FERROEHR_BLESS_GOLDEN").is_some();
    for (name, event, subject) in golden_events(&policy) {
        let path = dir.join(format!("{}.json", name.replace(' ', "-")));
        let mut text =
            serde_json::to_string_pretty(&golden(&projection, &event, subject)).expect("a golden");
        text.push('\n');
        if bless {
            std::fs::create_dir_all(&dir).expect("the golden directory");
            std::fs::write(&path, &text).expect("the golden is written");
            continue;
        }
        let want = std::fs::read_to_string(&path).expect("the golden is committed");
        assert_eq!(text, want, "{name}: the documents differ from the golden");
    }
}
