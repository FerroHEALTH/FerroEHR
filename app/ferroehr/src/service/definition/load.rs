// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Loading a definition from a file before the server starts, for
//! `ferroehr definitions load`.
//!
//! A source goes through the upload path of its format, so it is validated and
//! versioned exactly as `POST /definition/template/adl1.4` or
//! `POST /definition/template/adl2` would validate and version it. A source
//! whose id is already stored with the same text is left alone, so the load
//! can run at every start; one stored under the same id with other text is
//! refused, because a stored definition never changes (a new version takes a
//! new id). No openEHR spec governs loading definitions out of band: our own
//! design.

use openehr_adl::assemble::parse_artefact;
use openehr_adl::meta::summarize;
use openehr_adl::parse::Dialect;

use crate::error::ServiceError;
use crate::service::FerroEhrService;
use crate::storage::{adl2_repo, template_repo};

use super::adl2_engine::on_engine_stack;
use super::opt14::parse_opt;

/// The serialization a definition source is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    /// An OPT 1.4 operational template in its canonical XML.
    Opt14,
    /// An ADL 2 archetype, template or operational template.
    Adl2,
}

/// What loading one source did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Loaded {
    /// The definition was stored under this id.
    Stored(String),
    /// The id was already stored with the same text; nothing was written.
    Unchanged(String),
}

impl FerroEhrService {
    /// Loads one definition `source` in `format`: stores it through its
    /// upload path, or leaves it when its id is stored with the same text.
    ///
    /// # Errors
    ///
    /// - Every refusal of the upload path of `format`: a source that does not
    ///   parse, one the validation refuses, one that cannot be served as a Web
    ///   Template.
    /// - [`ServiceError::Conflict`] when the id is stored with other text.
    /// - A database failure.
    pub async fn definition_load(
        &self,
        format: SourceFormat,
        source: &str,
    ) -> Result<Loaded, ServiceError> {
        let (id, stored) = match format {
            SourceFormat::Opt14 => {
                let id = parse_opt(source)?.template_id.value;
                let stored = template_repo::template_xml(&self.pool, &id).await?;
                (id, stored)
            }
            SourceFormat::Adl2 => {
                let owned = source.to_owned();
                let id = on_engine_stack(move || {
                    parse_artefact(&owned, Dialect::Adl2)
                        .map(|archetype| summarize(&archetype).archetype_id)
                })
                .await?
                .map_err(|errs| super::adl2::syntax_bad_request(&errs))?;
                let stored = adl2_repo::adl(&self.pool, &id).await?;
                (id, stored)
            }
        };
        match stored {
            Some(stored) if stored == source => Ok(Loaded::Unchanged(id)),
            Some(_) => Err(ServiceError::conflict(format!(
                "a definition with the id '{id}' is stored with other content: a stored \
                 definition never changes, so a changed one takes a new version in its id"
            ))),
            None => {
                match format {
                    SourceFormat::Opt14 => {
                        self.store_template(source)
                            .await
                            .map_err(ServiceError::flattened)?;
                    }
                    SourceFormat::Adl2 => {
                        self.template_adl2_upload(source).await?;
                    }
                }
                Ok(Loaded::Stored(id))
            }
        }
    }
}
