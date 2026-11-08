// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `ferroehr definitions load`: definitions from files, stored before `serve`.
//!
//! A deployment that maps its professionals to demographic parties needs the
//! party templates in the definition store at boot, before any client can
//! upload them, so it loads them with this step first. Each file goes through
//! the upload path of its format and a file already stored with the same text
//! is left alone, so the step runs at every start. No openEHR spec governs
//! loading definitions out of band: our own design.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use clap::Subcommand;
use ferroehr::service::FerroEhrService;
use ferroehr::service::definition::load::{Loaded, SourceFormat};

/// `ferroehr definitions …` subcommands.
#[derive(Debug, Subcommand)]
pub enum DefinitionsCmd {
    /// Store each file in the definition store, in the order given, with the
    /// validation and versioning of the REST upload, then exit. An `.opt`
    /// file is an OPT 1.4 template and an `.adls` file an ADL 2 archetype or
    /// template, whose parents and slot fillers come before it. A file whose
    /// id is stored with the same text is left alone; one stored with other
    /// text is refused.
    Load {
        /// The definition files.
        #[arg(required = true, value_name = "PATH")]
        paths: Vec<PathBuf>,
    },
}

/// The format a definition file's extension names: `.opt` for OPT 1.4 and
/// `.adls` for ADL 2 (AM ADL2 `master07.02-adl_conventions.adoc`: the source
/// file of an archetype has the extension `.adls`).
///
/// # Errors
/// Any other extension.
pub fn format_of(path: &Path) -> anyhow::Result<SourceFormat> {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension) if extension.eq_ignore_ascii_case("opt") => Ok(SourceFormat::Opt14),
        Some(extension) if extension.eq_ignore_ascii_case("adls") => Ok(SourceFormat::Adl2),
        _ => anyhow::bail!(
            "{} is neither an OPT 1.4 template (.opt) nor an ADL 2 source (.adls)",
            path.display()
        ),
    }
}

/// Loads each of `paths` into `service`, in order, and names what each did.
///
/// # Errors
/// A file that cannot be read or has no known extension, or a refusal of the
/// store; the files before it stay stored.
#[expect(
    clippy::print_stdout,
    reason = "the definitions subcommand's PURPOSE is console output \
              (.claude/rules/reliability.md §tools)"
)]
pub async fn load(service: &FerroEhrService, paths: &[PathBuf]) -> anyhow::Result<()> {
    for path in paths {
        let format = format_of(path)?;
        let source = tokio::fs::read_to_string(path)
            .await
            .with_context(|| format!("reading {}", path.display()))?;
        let loaded = service
            .definition_load(format, &source)
            .await
            .with_context(|| format!("loading {}", path.display()))?;
        match loaded {
            Loaded::Stored(id) => println!("stored {id} from {}", path.display()),
            Loaded::Unchanged(id) => println!("unchanged {id} from {}", path.display()),
        }
    }
    Ok(())
}

/// `ferroehr definitions …`.
///
/// # Errors
/// The configuration does not load or validate, the database cannot be
/// reached or prepared, the terminology server cannot be built, or
/// [`load`] fails.
pub async fn run(
    cmd: &DefinitionsCmd,
    file: Option<&Path>,
    sets: &[(String, String)],
) -> anyhow::Result<()> {
    match cmd {
        DefinitionsCmd::Load { paths } => {
            let config = ferroehr::config::load(file, sets)?;
            config.validate()?;
            let pools = crate::connect_pool(&config).await?;
            let service = crate::attach_terminology(
                FerroEhrService::new(&pools).with_spec_profile(config.spec_profile),
                &config,
            )?;
            load(&service, paths).await
        }
    }
}
