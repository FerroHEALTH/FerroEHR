// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The in-memory size of the RM types a request reads, held under the budget
//! the emitter's by-size boxing decision aims for (#6002).
//!
//! The emitter boxes an RM field or enum variant whose estimated size passes
//! its threshold (`openehr-codegen` `plan::layout`). These are the real sizes
//! on a 64-bit target; a regression here means a large subtree is held by
//! value again, which costs `memmove` on every read and stack on every
//! nesting level. No openEHR specification governs layout: our own design.

use openehr_rm::prelude::*;

#[test]
fn the_polymorphic_slots_of_the_content_tree_are_two_words() {
    assert_eq!(size_of::<ContentItem>(), 16);
    assert_eq!(size_of::<Item>(), 16);
    assert_eq!(size_of::<ItemStructure>(), 16);
    assert_eq!(size_of::<Event<ItemStructure>>(), 16);
}

#[test]
fn the_nodes_of_a_composition_stay_under_their_budget() {
    let budget = [
        ("Composition", size_of::<Composition>(), 768),
        ("Section", size_of::<Section>(), 384),
        ("Observation", size_of::<Observation>(), 640),
        ("Evaluation", size_of::<Evaluation>(), 640),
        ("Element", size_of::<Element>(), 384),
        ("Cluster", size_of::<Cluster>(), 384),
        ("History", size_of::<History<ItemStructure>>(), 512),
        ("DataValue", size_of::<DataValue>(), 256),
        ("DvText", size_of::<DvText>(), 256),
        ("PartyProxy", size_of::<PartyProxy>(), 256),
    ];
    for (name, size, max) in budget {
        assert!(
            size <= max,
            "{name} is {size} bytes, over its {max}-byte budget"
        );
    }
}
