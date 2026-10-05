// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Generation-twin templates: ONE hand-written source, stamped per generation.
//!
//! A hand-written spec-behaviour file that is byte-identical across a crate's
//! generations modulo generation-module paths keeps exactly one source —
//! `tools/openehr-codegen/templates/<crate>/<relative-path>`, written against
//! the crate's CURRENT generation — and `emit` stamps the per-generation
//! copies with the generation tokens substituted (the crate's own module and
//! every PAIRED dependency-generation path, from the composition table).
//! Stamped copies carry the [`TEMPLATE_MARKER`] header, so the ordinary
//! `@generated` machinery owns them: the emit purge removes stale copies,
//! `codegen-drift` re-derives them, and a hand edit is overwritten on the
//! next `emit`. Divergence between generations becomes impossible instead of
//! policed.
//!
//! A genuinely generation-specific behaviour difference is promoted OUT of
//! the template into a per-generation OVERRIDE —
//! `templates/<crate>/overrides/<generation-module>/<relative-path>`, taken
//! VERBATIM for that one generation (it is written for it) — an explicit,
//! reviewed file carrying its own adjudication, mirroring the decision-map
//! pattern.

use std::path::{Path, PathBuf};

use crate::plan::composition::{COMPOSITIONS, CrateComposition, GenerationSpec};
use crate::render::emit::GenFile;

/// The first-line marker of a stamped copy. Contains `@generated`, so every
/// generated-file scan (purge, drift, hand-edit protection) already covers
/// stamped copies.
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

/// Stamp every template of `comp`'s crate into per-generation [`GenFile`]s.
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
            // The source carries its own SPDX header (it is a tracked
            // first-party file); the stamped copy gets one at write time from
            // the crate it lands in, so exactly one authority states licensing.
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

/// Rewrite `body` from the CURRENT generation's spelling to `target`'s: the
/// crate's own generation module (`crate::v1_2::` → `crate::v1_1::`) and
/// every paired dependency-generation path (`openehr_base::v1_3::` →
/// `openehr_base::v1_2::`, per the composition table's `model_deps`).
///
/// Substitution is deliberately restricted to the two path forms — a bare
/// module token in prose stays untouched, and a wrong pairing surfaces as a
/// compile error in the stamped copy, never as silent divergence.
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

/// Recursively collect `.rs` template sources under `dir`, skipping the
/// `overrides/` subtree (overrides are consumed per generation, never
/// stamped for every generation).
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
