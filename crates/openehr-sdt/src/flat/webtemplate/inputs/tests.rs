// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The openEHR terminology group of each coded RM slot.

use super::openehr_group;

#[test]
fn openehr_group_maps_coded_rm_slots_to_their_group() {
    // Each RM `has_code_for_group_id` slot maps to exactly one terminology group.
    assert_eq!(
        openehr_group("ISM_TRANSITION", "current_state"),
        Some("instruction_states")
    );
    assert_eq!(
        openehr_group("ISM_TRANSITION", "transition"),
        Some("instruction_transitions")
    );
    assert_eq!(
        openehr_group("COMPOSITION", "category"),
        Some("composition_category")
    );
    assert_eq!(openehr_group("EVENT_CONTEXT", "setting"), Some("setting"));
    assert_eq!(
        openehr_group("POINT_EVENT", "math_function"),
        Some("event_math_function")
    );
    // Unmapped: a plain attribute, or an attribute on a type that fixes no
    // group, has no group hint (rubric falls back to the global search).
    assert_eq!(openehr_group("ELEMENT", "value"), None);
    assert_eq!(openehr_group("ISM_TRANSITION", "careflow_step"), None);
}
