// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The typed RM-instance walk files of a generation and the released
//! generation they pair with.

use crate::load::bmm::BmmSchema;
use crate::plan::composition::{ComposedGeneration, ComposedUnit};
use crate::render::emit::GenFile;

/// The typed RM-instance walk of one RM generation
/// ([`crate::render::emit_walk`]): its own classes named under the generation
/// module, and the BASE classes its types reach named through the paired BASE
/// generation.
pub(super) fn walk_files(
    g: &ComposedGeneration,
    unit: &ComposedUnit,
    aug: &BmmSchema,
    released: Option<(&ComposedGeneration, &ComposedUnit)>,
) -> Vec<GenFile> {
    let module = g.spec.module;
    let base_external = crate::analyze::External::default();
    let mut schemas = vec![crate::render::emit_walk::WalkSchema {
        schema: aug,
        path: Box::new(move |class: &str| {
            Some(crate::render::emit_walk::local_path(module, aug, class))
        }),
        external: &g.external,
    }];
    for dep in &g.dep_schemas {
        schemas.push(crate::render::emit_walk::WalkSchema {
            schema: dep,
            path: Box::new(|class: &str| g.external.module_of(class).map(str::to_owned)),
            external: &base_external,
        });
    }
    let mut files = crate::render::emit_walk::emit_files(&unit.model, &schemas);
    // The reflective accessor serves the Simplified Formats, which read and
    // build the release model only.
    if g.spec.current {
        files.extend(crate::render::emit_access::emit_files(
            &unit.model,
            &schemas,
        ));
        if let Some((rg, ru)) = released {
            files.extend(crate::render::emit_released::emit_files(
                &unit.model,
                &ru.model,
                rg.spec.module,
                &schemas,
            ));
        }
    }
    files
}

/// The released generation of the RM: the newest generation older than the
/// current one (the table lists generations oldest first).
pub(super) fn released_generation(
    generations: &[ComposedGeneration],
) -> Result<Option<(&ComposedGeneration, &ComposedUnit)>, Box<dyn std::error::Error>> {
    let Some(current) = generations.iter().position(|g| g.spec.current) else {
        return Ok(None);
    };
    match current.checked_sub(1).and_then(|i| generations.get(i)) {
        Some(g) => Ok(Some((g, g.unit()?))),
        None => Ok(None),
    }
}
