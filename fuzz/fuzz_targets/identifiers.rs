// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

#![no_main]
//! The identifier readers, which parse bytes taken straight from the URL.
//!
//! Every versioned read addresses its object with a `{version_uid}` PATH
//! parameter, and `ferroehr_rest::overview::version_id::parse_version_uid`
//! hands that string to `ObjectVersionId::from_str` unmodified. So this is the
//! one parser family an unauthenticated caller reaches before any body is read,
//! any content type is negotiated, or any authorization runs — and a panic here
//! is not a `400`, it is a thread unwind on the request path.
//!
//! `OBJECT_VERSION_ID` is a COMPOSITE (BASE `base_types` master05 §Composite
//! Identifiers): `object_id::creating_system_id::version_tree_id`, where the
//! third part is itself dotted (`trunk.branch.branch_version`). Two separators
//! with different meanings and a nested grammar is exactly the shape where a
//! reader mis-slices, so the harness walks the parsed value's accessors rather
//! than stopping at "it parsed".

use libfuzzer_sys::fuzz_target;
use std::str::FromStr;

use openehr_base::prelude::{ArchetypeId, HierObjectId, ObjectVersionId, VersionTreeId};
use openehr_base::v1_3::base_types::identification::lexical::composite_ids_equal;

fuzz_target!(|data: &[u8]| {
    // Identifiers arrive as text; non-UTF-8 is rejected by the HTTP layer long
    // before this, so feeding it here would only waste executions.
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };

    // Each reader is expected to REFUSE most of what arrives — malformed input
    // is the point. What it may never do is panic, and what it may never do on
    // success is hand back a value whose own accessors panic.
    if let Ok(ovid) = ObjectVersionId::from_str(s) {
        let object_id = ovid.object_id();
        let system = ovid.creating_system_id();
        let tree = ovid.version_tree_id();
        let _ = ovid.creating_system_id_str();
        let _ = ovid.is_branch();

        // A value reassembled from the parts this reader produced must
        // IDENTIFY the same thing — a mis-slice moves a `::` boundary, which
        // no case fold hides. BASE master05 §Composite Identifiers and Case
        // makes the comparison case-insensitive; the typed `Uid` door renders
        // a UUID part lowercase, so byte equality refused a legal spelling of
        // the same identifier (#2746).
        let recomposed = ObjectVersionId::compose(&object_id, &system, &tree);
        assert!(
            composite_ids_equal(recomposed.value(), ovid.value()),
            "an OBJECT_VERSION_ID recomposed from its own parts must identify \
             itself (master05 case-insensitive comparison): {} vs {}",
            recomposed.value(),
            ovid.value()
        );
    }

    // The sibling readers reached from bodies and query text. Grouped here
    // rather than split into targets because they share a grammar family and a
    // corpus that exercises one exercises the others.
    let _ = HierObjectId::from_str(s);
    if let Ok(archetype) = ArchetypeId::from_str(s) {
        let _ = archetype.qualified_rm_entity();
        let _ = archetype.domain_concept();
        let _ = archetype.version_id();
    }
    if let Ok(tree) = VersionTreeId::from_str(s) {
        let _ = tree.trunk_version();
        let _ = tree.is_branch();
    }
});
