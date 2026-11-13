// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The segment planner of the dump.

use super::plan_segments;

#[test]
fn segments_group_greedily_under_the_limit() {
    // Three 40-byte records, limit 100 → [0,1] then [2].
    assert_eq!(plan_segments(&[40, 40, 40], 100), vec![0..2, 2..3]);
}

#[test]
fn one_record_per_segment_when_each_exceeds_the_limit() {
    assert_eq!(plan_segments(&[200, 200], 100), vec![0..1, 1..2]);
}

#[test]
fn a_single_oversized_record_is_its_own_segment() {
    // The 250-byte record cannot fit but still gets a segment of its own;
    // the two small ones share the next.
    assert_eq!(plan_segments(&[250, 10, 10], 100), vec![0..1, 1..3]);
}

#[test]
fn everything_fits_in_one_segment() {
    assert_eq!(plan_segments(&[10, 10, 10], 1000), vec![0..3]);
}

#[test]
fn no_records_no_segments() {
    assert_eq!(
        plan_segments(&[], 100),
        Vec::<std::ops::Range<usize>>::new()
    );
}
