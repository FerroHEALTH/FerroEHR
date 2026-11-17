// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The openEHR terminology group of each coded RM slot.

use super::openehr_group;

#[test]
fn openehr_group_maps_coded_rm_slots_to_their_group() {
    // The RM `has_code_for_group_id` slots (RM invariant tables under
    // docs/specs/openehr/RM/docs/UML/classes/): each maps to exactly one
    // openEHR terminology group so rubrics resolve unambiguously despite
    // SPECPR-51 cross-group code collisions.
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
