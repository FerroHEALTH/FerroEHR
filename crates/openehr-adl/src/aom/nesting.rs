// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The nesting bound over a constraint-model tree.
//!
//! The parser bounds what it reads, but a flat form composes structure: a
//! specialisation's differential paths and an OPT's inlined fillers each
//! place nodes below what any single parsed artefact carries, so every
//! producer of a composed tree re-checks the result against the engine's one
//! bound, [`openehr_lang::nesting::MAX_NESTING_DEPTH`], before handing it to
//! the recursive walkers downstream.

use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_lang::nesting::{Nesting, NestingExceeded};

use crate::aom::access::complex_attributes;

/// Check that no object in the tree under `root` sits deeper than the engine
/// bound; the root itself is level zero.
///
/// The walk descends with the same budget it checks, so it cannot itself
/// exceed the bound.
///
/// # Errors
/// [`NestingExceeded`] at the first object below the bound.
pub fn check_definition_nesting(root: &CComplexObject) -> Result<(), NestingExceeded> {
    check_complex(root, Nesting::ROOT)
}

fn check_complex(cco: &CComplexObject, level: Nesting) -> Result<(), NestingExceeded> {
    for attr in complex_attributes(cco) {
        for child in attr.children.iter().flatten() {
            if let CObject::CComplexObject(inner) = child {
                check_complex(inner, level.descend()?)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use openehr_lang::nesting::MAX_NESTING_DEPTH;

    use super::*;
    use crate::aom::build::complex_object;
    use openehr_am::v2_4::aom2::constraint_model::c_attribute::CAttribute;

    /// A single-attribute chain `levels` objects deep below the root.
    fn chain(levels: usize) -> CComplexObject {
        let mut node = complex_object(
            "CLUSTER".to_owned(),
            "id2".to_owned(),
            Vec::new(),
            Vec::new(),
            None,
        );
        for _ in 0..levels {
            let attr = CAttribute {
                parent: None,
                soc_parent: None,
                rm_attribute_name: "items".to_owned(),
                existence: None,
                children: openehr_base::containers::present(vec![node]),
                differential_path: None,
                cardinality: None,
                is_multiple: true,
            };
            node = complex_object(
                "CLUSTER".to_owned(),
                "id1".to_owned(),
                vec![attr],
                Vec::new(),
                None,
            );
        }
        match node {
            CObject::CComplexObject(cco) => cco,
            other => panic!("complex_object builds a complex object, got {other:?}"),
        }
    }

    #[test]
    fn a_tree_at_the_bound_passes_and_one_level_more_is_refused() {
        assert_eq!(check_definition_nesting(&chain(MAX_NESTING_DEPTH)), Ok(()));
        assert_eq!(
            check_definition_nesting(&chain(MAX_NESTING_DEPTH + 1)),
            Err(NestingExceeded {
                limit: MAX_NESTING_DEPTH
            })
        );
    }
}
