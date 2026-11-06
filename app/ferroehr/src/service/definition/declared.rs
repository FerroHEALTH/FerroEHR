// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The templates the configuration declares writable, held to the store at
//! boot.
//!
//! Each id a federation agreement's or a system client's `writes` names must
//! be stored, so a table cannot grant a template the server does not hold.
//! No openEHR spec governs configuration: our own design.

use std::collections::BTreeSet;

use crate::config::agreement::missing_templates;
use crate::config::authz::AuthzConfig;
use crate::db::acquire::acquire;
use crate::error::ServiceError;
use crate::service::FerroEhrService;
use crate::storage::{adl2_repo, template_repo};

/// The template ids the configuration grants that the store does not hold,
/// each named with its table.
#[derive(Debug, thiserror::Error)]
#[error("{}", .0.join("; "))]
pub struct UnheldTemplates(pub Vec<String>);

impl FerroEhrService {
    /// Checks that every template a `[[authz.federation.agreement]]` or an
    /// `[[authz.system_client]]` lists in `writes` is stored, as an OPT 1.4
    /// or an ADL 2 template.
    ///
    /// # Errors
    /// [`ServiceError`] on a database failure, wrapped in the outer `Ok`; the
    /// inner [`UnheldTemplates`] names every template the store lacks.
    pub async fn unheld_written_templates(
        &self,
        authz: &AuthzConfig,
    ) -> Result<Result<(), UnheldTemplates>, ServiceError> {
        let named: BTreeSet<&str> = authz
            .federation
            .agreement
            .iter()
            .flat_map(|a| a.writes.iter())
            .chain(authz.system_client.iter().flat_map(|c| c.writes.iter()))
            .map(String::as_str)
            .collect();
        let mut held = BTreeSet::new();
        for template in named {
            let stored = template_repo::template_exists(&self.pool, template).await?
                || adl2_repo::find(&mut *acquire(&self.pool).await?, template)
                    .await?
                    .is_some();
            if stored {
                held.insert(template.to_owned());
            }
        }
        let missing = missing_templates(&authz.federation.agreement, &authz.system_client, &held);
        Ok(if missing.is_empty() {
            Ok(())
        } else {
            Err(UnheldTemplates(missing))
        })
    }
}
