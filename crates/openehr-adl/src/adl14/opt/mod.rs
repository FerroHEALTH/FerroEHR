// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! OPT 1.4 → ADL 2 conversion: the front end that feeds a parsed ADL 1.4
//! operational template to the archetype converter.
//!
//! A 1.4 operational template is a specialisation-flattened artefact whose
//! `definition` embeds each component archetype as a nested `C_ARCHETYPE_ROOT`
//! with its own at-code space, so the component code spaces collide if the
//! whole is fed to [`convert`], which takes one source archetype. This front
//! end decomposes the OPT into one 1.4-shaped `openehr_am::v2_4` source
//! archetype per embedded root, replacing each child by an `ARCHETYPE_SLOT`
//! whose `include` assertion names it and recording the parent-to-child fill
//! edge in [`OptConversion::structure`]. Anything a decomposed root cannot
//! express is reported in `RESOURCE_DESCRIPTION.conversion_details`.
//!
//! The input is the `openehr_its::opt14` model, so the caller parses the OPT
//! 1.4 XML with `openehr_its::opt14::from_xml` first.
//!
//! NOTE: no openEHR spec governs 1.4 → 2 conversion; the decomposition, slot
//! substitution and code allocation here are our own design, as is the whole
//! [`crate::adl14`] module.

mod decompose;
mod primitive;
mod slot;
mod tuple;
mod value;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::authored_archetype::{
    AuthoredArchetype, AuthoredArchetypeData,
};
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::terminology::archetype_term::ArchetypeTerm;
use openehr_am::v2_4::aom2::terminology::archetype_terminology::ArchetypeTerminology;
use openehr_am::v2_4::resource::resource_description::ResourceDescription;
use openehr_base::prelude::Uuid;
use openehr_its::opt14;

use crate::adl14::convert::{ConvertConfig, ConvertError, convert};
use crate::adl14::log::ConversionLog;
use crate::hrid::parse_hrid;
use crate::print::PrintError;

use self::decompose::Decomposer;
use self::value::term_code;

/// A failure converting a 1.4 operational template to ADL 2.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OptConvertError {
    /// An embedded archetype root carries an `ARCHETYPE_ID` that is no
    /// archetype identifier.
    #[error("OPT embedded root has an invalid archetype id {archetype_id:?}: {reason}")]
    Hrid {
        /// The archetype id as the OPT states it.
        archetype_id: String,
        /// Why the identifier parser refused it.
        reason: String,
    },
    /// The archetype converter refused a decomposed source.
    #[error("converting the decomposed source {archetype_id:?}")]
    Convert {
        /// The archetype id of the decomposed root.
        archetype_id: String,
        /// The converter's refusal.
        #[source]
        source: ConvertError,
    },
    /// The ADL 2 printer refused a converted source.
    #[error("printing the converted source {archetype_id:?}")]
    Print {
        /// The archetype id of the converted root.
        archetype_id: String,
        /// The printer's refusal.
        #[source]
        source: PrintError,
    },
}

/// A parent-to-child slot fill recovered from the flattened OPT: the
/// composition structure that flattening erased, which no single converted
/// archetype states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FillEdge {
    /// The archetype id of the root that holds the substituted slot.
    pub parent_archetype_id: String,
    /// The path within the parent archetype to the substituted slot.
    pub parent_path: String,
    /// The at-code of the substituted slot.
    pub slot_node_id: String,
    /// The archetype id of the root that filled the slot.
    pub child_archetype_id: String,
}

/// One converted archetype root of an operational template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvertedRoot {
    /// The archetype id of the embedded root, as the OPT states it.
    pub archetype_id: String,
    /// The ADL 2 source text of the converted archetype.
    pub adl2: String,
    /// The codes the converter synthesised for this root and its provenance
    /// notes.
    pub log: ConversionLog,
    /// The converted archetype's `RESOURCE_DESCRIPTION.conversion_details`:
    /// every construct of the root the conversion dropped or changed.
    pub conversion_details: BTreeMap<String, String>,
}

/// The result of converting a 1.4 operational template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptConversion {
    /// One converted source per embedded archetype root, the OPT's top root
    /// first, then the others in document order.
    pub roots: Vec<ConvertedRoot>,
    /// The slot fills that joined the roots in the OPT.
    pub structure: Vec<FillEdge>,
}

/// Converts a parsed 1.4 operational template into one ADL 2 source per
/// embedded archetype root.
///
/// Each embedded `C_ARCHETYPE_ROOT` is decomposed into a scoped 1.4-shaped
/// `v2_4` source archetype and run through [`convert`]; embedded children are
/// replaced by open slots in their parent and the fill edges recorded in
/// [`OptConversion::structure`].
///
/// # Errors
///
/// - [`OptConvertError::Hrid`] if an embedded root's archetype id does not
///   parse.
/// - [`OptConvertError::Convert`] if the converter refuses a decomposed source.
/// - [`OptConvertError::Print`] if a converted source carries a node the ADL 2
///   printer has no syntax for.
pub fn convert_opt_to_adl2(
    opt: &opt14::types::OperationalTemplate,
) -> Result<OptConversion, OptConvertError> {
    let (archetypes, structure) = convert_opt_to_archetypes(opt)?;
    let roots = archetypes
        .into_iter()
        .map(|root| {
            let adl2 =
                crate::print::print(&root.archetype).map_err(|source| OptConvertError::Print {
                    archetype_id: root.archetype_id.clone(),
                    source,
                })?;
            let conversion_details = crate::artefact::view(&root.archetype)
                .description
                .and_then(|d| d.conversion_details.clone())
                .unwrap_or_default();
            Ok(ConvertedRoot {
                archetype_id: root.archetype_id,
                adl2,
                log: root.log,
                conversion_details,
            })
        })
        .collect::<Result<Vec<ConvertedRoot>, OptConvertError>>()?;
    Ok(OptConversion { roots, structure })
}

/// One decomposed and converted OPT root, before printing.
#[derive(Debug, PartialEq)]
struct ConvertedArchetype {
    archetype_id: String,
    archetype: Archetype,
    log: ConversionLog,
}

/// The minimal valid `RESOURCE_DESCRIPTION` for a decomposed OPT root.
///
/// VARD (`master03` §Validity Rules) requires a description; the lifecycle
/// state uses the 1.4→2 converter's own mapping (`unmanaged`). The
/// conversion-report entries land in `conversion_details`, the AOM2
/// `RESOURCE_DESCRIPTION` field for conversion provenance.
fn minimal_description(conversion_details: BTreeMap<String, String>) -> ResourceDescription {
    ResourceDescription {
        title: None,
        original_author: BTreeMap::new(),
        original_namespace: None,
        original_publisher: None,
        other_contributors: openehr_base::containers::present(Vec::new()),
        lifecycle_state: "unmanaged".to_owned(),
        custodian_namespace: None,
        custodian_organisation: None,
        copyright: None,
        licence: None,
        ip_acknowledgements: None,
        references: None,
        resource_package_uri: None,
        conversion_details: (!conversion_details.is_empty()).then_some(conversion_details),
        details: None,
        other_details: None,
    }
}

/// Decomposes the OPT and converts each embedded root, returning the converted
/// `v2_4` archetypes with their logs and the recovered fill structure.
///
/// # Errors
///
/// As [`convert_opt_to_adl2`], except [`OptConvertError::Print`].
fn convert_opt_to_archetypes(
    opt: &opt14::types::OperationalTemplate,
) -> Result<(Vec<ConvertedArchetype>, Vec<FillEdge>), OptConvertError> {
    let mut dx = Decomposer::new(opt);
    // Unit 0 is the OPT's top root; nested roots are appended in document order
    // as they are discovered.
    dx.process_root(&opt.definition, "", true);
    let (language, units, edges) = dx.finish();

    let cfg = ConvertConfig {
        // NOTE: a flattened OPT inlines `-`-specialised roots standalone with
        // no resolvable lineage, so they emit unspecialised at depth 0 (no
        // openEHR spec governs 1.4→2 conversion — our own design).
        collapse_specialised_codes: true,
        ..ConvertConfig::default()
    };
    let mut archetypes = Vec::with_capacity(units.len());
    for unit in units {
        let mut log = ConversionLog::new();
        let hrid = parse_hrid(&unit.archetype_id).map_err(|reason| OptConvertError::Hrid {
            archetype_id: unit.archetype_id.clone(),
            reason,
        })?;
        let data = AuthoredArchetypeData {
            parent_archetype_id: None,
            archetype_id: hrid,
            is_differential: false,
            definition: unit.definition,
            terminology: Box::new(unit.terminology),
            rules: openehr_base::containers::present(Vec::new()),
            rm_overlay: None,
            uid: None,
            original_language: term_code("ISO_639-1", &language),
            description: Some(Box::new(minimal_description(unit.notes))),
            is_controlled: None,
            annotations: None,
            translations: None,
            adl_version: Some("1.4".to_owned()),
            build_uid: Uuid::new(uuid::Uuid::nil()),
            rm_release: String::new(),
            is_generated: true,
            other_meta_data: BTreeMap::new(),
        };
        // NOTE: `is_differential`, `adl_version`, `rm_release`, `is_generated`
        // above are the converter's starting point only — `convert` overrides
        // them from the absent parent and from `ConvertConfig`.
        let art =
            Archetype::AuthoredArchetype(Box::new(AuthoredArchetype::AuthoredArchetype(data)));
        let archetype =
            convert(&art, &cfg, &mut log).map_err(|source| OptConvertError::Convert {
                archetype_id: unit.archetype_id.clone(),
                source,
            })?;
        archetypes.push(ConvertedArchetype {
            archetype_id: unit.archetype_id,
            archetype,
            log,
        });
    }
    Ok((archetypes, edges))
}

/// One decomposed archetype root awaiting conversion.
struct RawUnit {
    archetype_id: String,
    definition: CComplexObject,
    terminology: ArchetypeTerminology,
    /// Conversion-report entries for this root (dropped bindings, carried-URI
    /// notes, fallbacks), landed in the converted archetype's
    /// `RESOURCE_DESCRIPTION.conversion_details`.
    notes: BTreeMap<String, String>,
}

/// Per-root working state threaded through the definition walk.
///
/// It holds the slot at-code allocator, the ac-code allocator with the ac codes
/// the root's flattened ontology defines (for `CONSTRAINT_REF` resolution), the
/// terminology entries minted en route (reference-set ac definitions and
/// bindings), and the conversion-report notes.
struct RootCx {
    slot_num: i64,
    next_ac: i64,
    defined_acs: std::collections::BTreeSet<String>,
    extra_terms: Vec<ArchetypeTerm>,
    /// `(terminology-key, code, uri)` term-binding entries.
    extra_bindings: Vec<(String, String, String)>,
    notes: BTreeMap<String, String>,
}

impl RootCx {
    /// Mint a fresh 1.4-space ac code (the converter core shifts it like every
    /// other code, keeping the constraint and its terminology entry aligned).
    fn alloc_ac(&mut self) -> String {
        let code = format!("ac{:04}", self.next_ac);
        self.next_ac += 1;
        code
    }

    fn note(&mut self, key: impl Into<String>, message: impl Into<String>) {
        self.notes.insert(key.into(), message.into());
    }
}
