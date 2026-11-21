// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The record's confidentiality marking withheld from a query by a reader
//! of the `EHR_ACCESS` settings who is neither the subject nor the controller.
//!
//! Draft access control, The Decision Procedure, Reading and Writing the
//! Settings, item 3: a cell that serves a whole `EHR_ACCESS` or its
//! `settings` is served without `marking`. No path below `settings`
//! resolves, since RM `ACCESS_CONTROL_SETTINGS` declares no attribute, so no
//! predicate or projection reads the marking itself. No openEHR spec governs
//! this: our own design (#6165).

use super::cell::Cell;
use super::ir::{LeafPath, PathTarget, QueryIr, SelectValue, Source};
use crate::service::ehr::access_view;

/// The RM type whose settings carry the marking.
const EHR_ACCESS: &str = "EHR_ACCESS";

/// Serves every cell of `rows` that holds a whole `EHR_ACCESS` or its
/// `settings`, by the SELECT columns of `ir`, without `marking`.
pub fn strip_cells(ir: &QueryIr, rows: &mut Vec<Vec<Cell>>) {
    let carriers: Vec<(usize, bool)> = ir
        .select
        .iter()
        .enumerate()
        .filter_map(|(i, column)| match &column.value {
            SelectValue::Path(PathTarget::Data(leaf)) => {
                carrier(&ir.sources, leaf).map(|whole| (i, whole))
            }
            _ => None,
        })
        .collect();
    if carriers.is_empty() {
        return;
    }
    for row in rows.iter_mut() {
        for (i, whole) in &carriers {
            let Some(cell) = row.get_mut(*i) else {
                continue;
            };
            let mut value = cell.to_value();
            if value.is_null() {
                continue;
            }
            if *whole {
                access_view::redact_object(&mut value);
            } else {
                access_view::redact_settings(&mut value);
            }
            *cell = Cell::of_jsonb(&value);
        }
    }
    if ir.distinct {
        // NOTE: our own design (#6165), rows that differ only by a withheld marking read as one.
        let mut seen: Vec<Vec<Cell>> = Vec::with_capacity(rows.len());
        rows.retain(|row| {
            let first = !seen.contains(row);
            if first {
                seen.push(row.clone());
            }
            first
        });
    }
}

/// Whether the source of `leaf` can be an `EHR_ACCESS`.
fn of_ehr_access(sources: &[Source], leaf: &LeafPath) -> bool {
    matches!(sources.get(leaf.source.0), Some(Source::Rm(source)) if source.rm_type.contains(EHR_ACCESS))
}

/// Whether `leaf` serves a whole `EHR_ACCESS` (`Some(true)`) or its
/// `settings` (`Some(false)`); `None` for any other path.
fn carrier(sources: &[Source], leaf: &LeafPath) -> Option<bool> {
    if !leaf.anchor.is_empty() || !of_ehr_access(sources, leaf) {
        return None;
    }
    match leaf.fragment.as_slice() {
        [] => Some(true),
        [step] if step.name == "settings" => Some(false),
        _ => None,
    }
}
