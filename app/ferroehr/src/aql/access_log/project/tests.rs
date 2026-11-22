// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The typed cells are the cells the record's view read through a JSON value
//! gives, over entries built from generated audit events, for each reader
//! and a set of paths that crosses every container and every member the
//! subject view leaves out (#6005).

#![expect(
    clippy::disallowed_types,
    reason = "the reference the typed cells are held to reads the record's ITS-JSON view \
              (#1694 family 5: AQL result cells)"
)]

use openehr_query::ast::{ColumnExpr, Primitive};
use openehr_query::parser::parse_str;
use openehr_rm::draft::access_log::access_log::access_log_record::AccessLogRecord;
use proptest::prelude::*;
use serde_json::Value;

use super::{Shown, cell, literal};
use crate::access_log::entry::entry_of;
use crate::access_log::retention::stated;
use crate::access_log::stored_form::parts::tests::event;
use crate::access_log::view::{LogReader, Restrictions, view_of};
use crate::aql::access_log::path::{LogPath, resolve};
use crate::aql::cell::Cell;
use crate::system_log::retention::LogRetention;

/// The paths the cells are compared on.
const PATHS: [&str; 28] = [
    "l",
    "l/uid",
    "l/uid/value",
    "l/ehr_id/value",
    "l/time_recorded/value",
    "l/accessor",
    "l/accessor/kind",
    "l/accessor/kind/defining_code/code_string",
    "l/accessor/party",
    "l/accessor/roles",
    "l/accessor/roles/defining_code/code_string",
    "l/accessor/credential",
    "l/accessor/credential/subject",
    "l/action",
    "l/purposes",
    "l/purposes/defining_code/code_string",
    "l/is_emergency",
    "l/items",
    "l/items/version_id/value",
    "l/items/origins",
    "l/items/origins/system_id",
    "l/items/categories/defining_code/code_string",
    "l/channel",
    "l/channel/request_id",
    "l/query",
    "l/query/text_digest",
    "l/query/rows",
    "l/other_details/items/value",
];

/// `text` typed against the entry's classes.
fn path(text: &str) -> LogPath {
    let query = parse_str(&["SELECT ", text, " FROM ACCESS_LOG_ENTRY l"].concat()).expect("parses");
    let Some(ColumnExpr::Path(path)) = query.select.columns.first().map(|c| c.column.clone())
    else {
        panic!("a path column");
    };
    resolve(&path, "l").expect("resolves")
}

/// The cell the view `reader` reads of `record` gives for `path`, through
/// the record's JSON value.
fn through_value(
    record: &AccessLogRecord,
    path: &LogPath,
    reader: LogReader,
    restrictions: &Restrictions,
) -> Cell {
    let view = view_of(record, reader, restrictions)
        .expect("the view")
        .unwrap_or(Value::Null);
    Cell::of_value(&path.read(&view))
}

/// `entry` as a stored record.
fn record_of(
    entry: openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry,
) -> AccessLogRecord {
    AccessLogRecord::AccessLogEntry(entry)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1024))]

    #[test]
    fn a_typed_cell_is_the_cell_of_the_view(event in event(), withhold in any::<bool>()) {
        let retention = stated(Some(jiff::civil::date(2031, 11, 8)), &LogRetention::default());
        if let Some(entry) = entry_of(&event, Some("patient-1"), "cdr.example.org", retention) {
            let restrictions = if withhold {
                Restrictions::withholding(&entry.accessor.kind, &["party", "roles"])
            } else {
                Restrictions::default()
            };
            let record = record_of(entry);
            for text in PATHS {
                let path = path(text);
                for reader in [LogReader::Reviewer, LogReader::Person] {
                    let shown = Shown::of(&record, reader, &restrictions);
                    let typed = cell(&record, &path, &shown).expect("the cell");
                    let reference = through_value(&record, &path, reader, &restrictions);
                    prop_assert_eq!(
                        typed.json(),
                        reference.json(),
                        "{} as {:?}", text, reader
                    );
                }
            }
        }
    }
}

/// A tombstone shows the person nothing: every path reads null, or an empty
/// list for one that crosses a container.
#[test]
fn a_tombstone_shows_the_person_nothing() {
    let entry = crate::access_log::stored_form::parts::tests::read_entry();
    let grounds = entry.retention.grounds.clone();
    let tombstone = crate::access_log::store::tombstone_of(
        entry,
        crate::versioning::audit::dv_date_time(
            &"2031-11-09T00:00:00Z".parse().expect("an instant"),
        ),
        grounds,
    );
    let record = AccessLogRecord::AccessLogTombstone(tombstone);
    let restrictions = Restrictions::default();
    for text in PATHS {
        let path = path(text);
        for reader in [LogReader::Reviewer, LogReader::Person] {
            let shown = Shown::of(&record, reader, &restrictions);
            assert_eq!(
                cell(&record, &path, &shown).expect("the cell").json(),
                through_value(&record, &path, reader, &restrictions).json(),
                "{text} as {reader:?}"
            );
        }
    }
}

/// A literal's cell is the cell of its JSON value.
#[test]
fn a_literal_is_the_cell_of_its_value() {
    let literals = [
        (
            Primitive::String("a \"quoted\" é".to_owned()),
            Value::from("a \"quoted\" é"),
        ),
        (Primitive::Integer(-42), Value::from(-42)),
        (Primitive::Real(0.1), Value::from(0.1)),
        (Primitive::Real(1e300), Value::from(1e300)),
        (Primitive::Real(f64::NAN), Value::from(f64::NAN)),
        (Primitive::Boolean(true), Value::from(true)),
        (Primitive::Null, Value::Null),
    ];
    for (primitive, value) in literals {
        assert_eq!(literal(&primitive), Cell::of_value(&value), "{primitive:?}");
    }
}
