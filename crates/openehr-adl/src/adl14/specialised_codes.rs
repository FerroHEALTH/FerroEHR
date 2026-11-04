// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The AOM 1.4 rule for the term codes a specialised archetype introduces.
//!
//! AOM 1.4 `AM/docs/AOM1.4/master07-ontology_package.adoc` §Specialisation
//! Depth: term codes "introduced in the terminology of specialised archetypes
//! (i.e. which did not exist in the terminology of the parent archetype) are
//! defined in a strict way, using '.' (period) markers", while constraint
//! codes "exist in a flat code space". A 1.4 specialisation is authored flat,
//! so an undotted new code reads like an inherited one, and only the parent's
//! terminology tells the two apart.

use std::collections::BTreeSet;

use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::authored_archetype::{
    AuthoredArchetype, AuthoredArchetypeData,
};
use openehr_am::v2_4::aom2::definitions::adl_code_definitions::AdlCodeDefinitionsData;
use openehr_am::v2_4::aom2::terminology::archetype_terminology::ArchetypeTerminology;

use crate::adl14::convert::ConvertError;
use crate::codes::specialisation_depth;
use crate::hrid::raw_id_specialisation_depth;

/// Refuses a 1.4 specialised `child` that introduces a term code its 1.4
/// `parent` does not define with a specialisation depth other than the
/// child's own.
///
/// The child's depth is the one its identifier states
/// ([`raw_id_specialisation_depth`]). A child that specialises nothing, or an artefact that is not an
/// authored archetype, passes.
///
/// # Errors
///
/// [`ConvertError::UndottedNewCodes`] naming every such code, in code order.
pub fn check_introduced_codes(child: &Archetype, parent: &Archetype) -> Result<(), ConvertError> {
    let (Some(child), Some(parent)) = (authored(child), authored(parent)) else {
        return Ok(());
    };
    if child.parent_archetype_id.is_none() {
        return Ok(());
    }
    let depth = raw_id_specialisation_depth(&child.archetype_id.physical_id());
    let inherited = term_codes(&parent.terminology);
    let codes: Vec<String> = term_codes(&child.terminology)
        .into_iter()
        .filter(|code| AdlCodeDefinitionsData::is_at_code(code))
        .filter(|code| !inherited.contains(code))
        .filter(|code| specialisation_depth(code) != Some(depth))
        .collect();
    if codes.is_empty() {
        Ok(())
    } else {
        Err(ConvertError::UndottedNewCodes { depth, codes })
    }
}

fn authored(a: &Archetype) -> Option<&AuthoredArchetypeData> {
    match a {
        Archetype::AuthoredArchetype(b) => match b.as_ref() {
            AuthoredArchetype::AuthoredArchetype(d) => Some(d),
            _ => None,
        },
        Archetype::TemplateOverlay(_) => None,
    }
}

/// Every code with a term definition in any language.
fn term_codes(terminology: &ArchetypeTerminology) -> BTreeSet<String> {
    terminology
        .term_definitions
        .values()
        .flat_map(|terms| terms.keys().cloned())
        .collect()
}
