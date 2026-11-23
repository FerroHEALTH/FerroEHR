// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Generation-twin templates: one hand-written source per file under
//! `tools/openehr-codegen/templates/<crate>/`, written against the current
//! generation and stamped per generation with its module paths substituted.
//! A generation-specific file lives under
//! `templates/<crate>/overrides/<generation-module>/`, taken verbatim.

use std::path::{Path, PathBuf};

use crate::plan::composition::{COMPOSITIONS, CrateComposition, GenerationSpec};
use crate::render::emit::GenFile;

/// The first-line marker of a stamped copy, which contains `@generated`.
pub(crate) const TEMPLATE_MARKER: &str = "// @generated-from-template";

/// Whether a written file is a template-stamped copy (first line carries the
/// marker).
pub(crate) fn is_template_stamped(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|s| {
        s.lines()
            .next()
            .is_some_and(|l| l.starts_with(TEMPLATE_MARKER))
    })
}

/// Stamps every template of `comp`'s crate into per-generation [`GenFile`]s.
///
/// # Errors
/// Returns an error when the templates tree cannot be read, or when a
/// template's path escapes its crate directory.
pub(crate) fn stamp_templates(
    templates_root: &Path,
    comp: &CrateComposition,
) -> Result<Vec<GenFile>, Box<dyn std::error::Error>> {
    let dir = templates_root.join(comp.crate_name);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let overrides = dir.join("overrides");
    let mut sources = Vec::new();
    collect_templates(&dir, &overrides, &mut sources)?;
    sources.sort();
    let Some(current) = current_generation(comp) else {
        return Err(format!("composition `{}` has no current generation", comp.key).into());
    };
    let mut out = Vec::new();
    for source in &sources {
        let rel = source.strip_prefix(&dir)?.to_path_buf();
        let rel_str = path_to_slash(&rel);
        let body = std::fs::read_to_string(source)?;
        for generation in comp.generations {
            let override_path = overrides.join(generation.module).join(&rel);
            let (text, provenance) = if override_path.exists() {
                (
                    std::fs::read_to_string(&override_path)?,
                    format!(
                        "templates/{}/overrides/{}/{rel_str} (per-generation override)",
                        comp.crate_name, generation.module
                    ),
                )
            } else {
                (
                    substitute(&body, current, generation),
                    format!("templates/{}/{rel_str}", comp.crate_name),
                )
            };
            // The stamped copy gets its SPDX header from the crate it lands in.
            let text = crate::render::spdx::strip_leading_header(&text);
            out.push(GenFile {
                path: format!("{}/{rel_str}", generation.module),
                body: format!(
                    "{TEMPLATE_MARKER} {provenance} — DO NOT EDIT; edit the source and re-run \
                     `openehr-codegen -- emit`.\n{text}"
                ),
            });
        }
    }
    Ok(out)
}

/// Rewrites `body` from the current generation's paths to `target`'s: the
/// crate's own `crate::vX_Y::` and each paired dependency path; a bare token
/// in prose is left alone.
pub(crate) fn substitute(body: &str, current: &GenerationSpec, target: &GenerationSpec) -> String {
    let mut out = body.replace(
        &format!("crate::{}::", current.module),
        &format!("crate::{}::", target.module),
    );
    for dep in current.model_deps {
        let Some(paired) = target.model_deps.iter().find(|d| d.key == dep.key) else {
            continue;
        };
        if paired.generation == dep.generation {
            continue;
        }
        let dep_crate = dep_crate_ident(dep.key);
        out = out.replace(
            &format!("{dep_crate}::{}::", dep.generation),
            &format!("{dep_crate}::{}::", paired.generation),
        );
    }
    out
}

/// The crate's CURRENT generation ([`crate::plan::composition::compose`]
/// asserts exactly one exists; `None` only on a defective table).
pub(crate) fn current_generation(comp: &CrateComposition) -> Option<&'static GenerationSpec> {
    comp.generations.iter().find(|g| g.current)
}

/// The Rust crate ident of a composition dependency key
/// (`base` → `openehr_base`).
fn dep_crate_ident(key: &str) -> String {
    COMPOSITIONS
        .iter()
        .find(|c| c.key == key)
        .map_or_else(|| format!("openehr_{key}"), |c| c.crate_name.to_owned())
        .replace('-', "_")
}

/// Collects `.rs` template sources under `dir`, skipping `overrides/`.
fn collect_templates(
    dir: &Path,
    overrides: &Path,
    out: &mut Vec<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path == overrides {
            continue;
        }
        if path.is_dir() {
            collect_templates(&path, overrides, out)?;
        } else if path.extension().and_then(std::ffi::OsStr::to_str) == Some("rs") {
            out.push(path);
        }
    }
    Ok(())
}

/// A relative path as forward-slash text (the [`GenFile`] path convention).
fn path_to_slash(path: &Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
