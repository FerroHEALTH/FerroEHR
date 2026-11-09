// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The earliest RM release a canonical-JSON composition decodes as.
//!
//! Hand-written; preserved across `openehr-codegen` regeneration (it carries no
//! generated-file marker, so `lib.rs` auto-declares `pub mod release;`).
//!
//! RM `docs/UML/classes/org.openehr.rm.common.archetyped.adoc` defines
//! `ARCHETYPED.rm_version` as the "Version of the openEHR reference model used
//! to create this object". A builder that names it needs the oldest release
//! whose model admits everything the composition holds, which is the first
//! generation, in release order, whose canonical-JSON decode accepts it.

#![expect(
    clippy::disallowed_types,
    reason = "the release probe reads the canonical JSON node a consumer assembled before any typed \
              decode (#1694 boundary class)"
)]

use serde::Deserialize as _;
use serde_json::Value;

use crate::Generation;

impl Generation {
    /// Returns the earliest generation whose canonical-JSON decode accepts
    /// `composition` as a `COMPOSITION`, trying the generations oldest first.
    ///
    /// A composition holding no element outside RM 1.1.0 answers
    /// [`Generation::V1_1`]; one holding an element only RM 1.2.0 admits (a
    /// `GENERIC_ENTRY` whose `data` is a `CLUSTER` or an `ELEMENT`: the RM 1.1.0
    /// BMM types it `ITEM_TREE`, RM 1.2.0 types it `ITEM`) answers
    /// [`Generation::V1_2`].
    ///
    /// # Errors
    ///
    /// Returns [`NoGenerationDecodes`], carrying every generation's refusal,
    /// when no generation decodes `composition`.
    pub fn earliest_for_composition(composition: &Value) -> Result<Self, NoGenerationDecodes> {
        let mut rejections = Vec::new();
        let mut next = Some(Self::V1_1);
        while let Some(generation) = next {
            match generation.decode_composition(composition) {
                Ok(()) => return Ok(generation),
                Err(error) => rejections.push((generation, error)),
            }
            next = generation.successor();
        }
        Err(NoGenerationDecodes { rejections })
    }

    /// The next generation in release order, or `None` for the newest.
    const fn successor(self) -> Option<Self> {
        match self {
            Self::V1_1 => Some(Self::V1_2),
            Self::V1_2 => None,
        }
    }

    /// Decodes `composition` as this generation's `COMPOSITION`.
    fn decode_composition(self, composition: &Value) -> Result<(), serde_json::Error> {
        match self {
            Self::V1_1 => {
                crate::v1_1::composition::composition::Composition::deserialize(composition)
                    .map(drop)
            }
            Self::V1_2 => {
                crate::v1_2::composition::composition::Composition::deserialize(composition)
                    .map(drop)
            }
        }
    }
}

/// No generation of this crate decodes a value as a `COMPOSITION`.
#[derive(Debug)]
pub struct NoGenerationDecodes {
    rejections: Vec<(Generation, serde_json::Error)>,
}

impl NoGenerationDecodes {
    /// Each generation's refusal, oldest generation first.
    #[must_use]
    pub fn rejections(&self) -> &[(Generation, serde_json::Error)] {
        &self.rejections
    }
}

impl std::fmt::Display for NoGenerationDecodes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("no RM generation decodes the value as a COMPOSITION")?;
        for (generation, error) in &self.rejections {
            write!(f, "; {generation}: {error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for NoGenerationDecodes {
    // NOTE: the newest generation's refusal is the source, since its model is the widest.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        let (_, error) = self.rejections.last()?;
        Some(error)
    }
}
