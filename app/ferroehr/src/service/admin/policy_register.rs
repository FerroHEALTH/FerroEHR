// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The operational-policy admin registers behind the admin API: read one,
//! read its versions, write a new version, and read the newest versions on
//! every request.
//!
//! A write replaces the whole register and names the version it replaces, so
//! a writer that read an older version is refused (`412`) rather than
//! overwriting a change it never saw. A document that does not hold is
//! refused whole (`400`) and never reaches the request path. No openEHR spec
//! governs operational policy: our own design/extension.

use sqlx::Connection as _;

use crate::db::acquire::acquire;
use crate::error::ServiceError;
use crate::policy_register::access_delay::AccessDelayRegister;
use crate::policy_register::{LiveRegisters, Register};
use crate::service::FerroEhrService;
use crate::storage::policy_register as store;
use crate::system_log::categories::{CategoryMapConfig, MapSource};

/// The committer a version names when the request carried no identity.
const UNAUTHENTICATED: &str = "unauthenticated";

/// One version of a register as the admin API serves it.
#[derive(Debug, Clone)]
pub struct RegisterVersion {
    /// The register.
    pub register: Register,
    /// The version; `0` is the default, which no one committed.
    pub version: i64,
    /// The register document, as JSON text.
    pub document: String,
    /// Who committed the version; `None` for the default.
    pub committer: Option<String>,
    /// When it was committed; `None` for the default.
    pub committed_at: Option<jiff::Timestamp>,
}

impl RegisterVersion {
    /// The default version of `register`.
    fn default_of(register: Register) -> Result<Self, ServiceError> {
        let document = match register {
            Register::AuditCategories => serde_json::to_string(&CategoryMapConfig::default()),
            Register::AccessDelay => serde_json::to_string(&AccessDelayRegister::default()),
        }?;
        Ok(Self {
            register,
            version: 0,
            document,
            committer: None,
            committed_at: None,
        })
    }

    /// A stored version of `register`.
    fn stored(register: Register, row: store::RegisterRow) -> Self {
        Self {
            register,
            version: row.version,
            document: row.document,
            committer: Some(row.committer),
            committed_at: Some(row.committed_at),
        }
    }
}

impl FerroEhrService {
    /// The operational-policy registers in force on this replica.
    #[must_use]
    pub fn policy_registers(&self) -> &std::sync::Arc<LiveRegisters> {
        &self.registers
    }

    /// Refuse a category register that classifies nothing, the
    /// `no_category_map` floor of a production deployment that audits and has
    /// not accepted the gap (`deployment.profile`, `deployment.accepts`).
    #[must_use]
    pub fn with_required_category_map(mut self, required: bool) -> Self {
        self.require_category_map = required;
        self
    }

    /// Bring the registers in force up to date before the request path
    /// decides anything: a read when a register write was notified or the
    /// listener is down ([`crate::policy_register::listen::Listened`]), and no database
    /// work otherwise.
    ///
    /// # Errors
    /// `service_overloaded` (`503`) when the register store does not answer.
    pub async fn refresh_policy_registers(&self) -> Result<(), ServiceError> {
        self.registers.refresh_if_stale().await.map_err(|error| {
            ServiceError::overloaded("the policy registers could not be read", error)
        })
    }

    /// Read the newest version of every register and install each one this
    /// replica does not hold, whatever the listener says.
    ///
    /// # Errors
    /// `service_overloaded` (`503`) when the register store does not answer.
    pub async fn read_policy_registers(&self) -> Result<(), ServiceError> {
        self.registers.refresh().await.map_err(|error| {
            ServiceError::overloaded("the policy registers could not be read", error)
        })
    }

    /// The newest version of `register`, or its default when it has none.
    ///
    /// # Errors
    /// `exception` — a database fault.
    pub async fn policy_register(
        &self,
        register: Register,
    ) -> Result<RegisterVersion, ServiceError> {
        match store::newest(self.registers.pool(), register.as_str())
            .await
            .map_err(ServiceError::Storage)?
        {
            Some(row) => Ok(RegisterVersion::stored(register, row)),
            None => RegisterVersion::default_of(register),
        }
    }

    /// Every version of `register`, newest first, ending with the default.
    ///
    /// # Errors
    /// `exception` — a database fault.
    pub async fn policy_register_history(
        &self,
        register: Register,
    ) -> Result<Vec<RegisterVersion>, ServiceError> {
        let mut versions: Vec<RegisterVersion> =
            store::history(self.registers.pool(), register.as_str())
                .await
                .map_err(ServiceError::Storage)?
                .into_iter()
                .map(|row| RegisterVersion::stored(register, row))
                .collect();
        versions.push(RegisterVersion::default_of(register)?);
        Ok(versions)
    }

    /// Replace `register` whole with `document`, when `replaces` is its
    /// newest version, and install the new version on this replica; the
    /// version written.
    ///
    /// # Errors
    /// - `precondition_violation` (`400`) — the document is not the
    ///   register's, or does not hold: a category map that does not compile,
    ///   classifies nothing under the `no_category_map` floor, or drops a
    ///   category the access delay names; an access delay with an unusable
    ///   limit or a category the map does not name.
    /// - `version_mismatch` (`412`) — `replaces` is not the newest version.
    /// - `service_overloaded` (`503`) — the register store does not answer.
    /// - `exception` — a database fault (rolled back).
    pub async fn put_policy_register(
        &self,
        register: Register,
        replaces: i64,
        document: &str,
    ) -> Result<i64, ServiceError> {
        self.read_policy_registers().await?;
        let canonical = match register {
            Register::AuditCategories => self.checked_categories(document)?.1,
            Register::AccessDelay => self.checked_delay(document)?,
        };
        let committer = crate::versioning::committer::current_committer()
            .map_or_else(|| UNAUTHENTICATED.to_owned(), |identity| identity.subject);
        let mut conn = acquire(self.registers.pool()).await?;
        let mut tx = conn.begin().await.map_err(ServiceError::Database)?;
        let written = store::append(&mut tx, register.as_str(), replaces, &canonical, &committer)
            .await
            .map_err(ServiceError::Storage)?;
        let Some(version) = written else {
            return Err(ServiceError::version_conflict(format!(
                "the {register} register is no longer at version {replaces}: read it again and \
                 write the change over the newest version"
            )));
        };
        if register == Register::AuditCategories {
            let (classifier, _) = self.checked_categories(document)?;
            crate::storage::marks::replace_category_map_on(&mut tx, classifier.rows())
                .await
                .map_err(ServiceError::Storage)?;
        }
        // NOTE: PostgreSQL 18 NOTIFY (sql-notify) delivers at commit, to every listening replica.
        sqlx::query("SELECT pg_notify($1, $2)")
            .bind(crate::policy_register::listen::CHANNEL)
            .bind(format!("{register}:{version}"))
            .execute(&mut *tx)
            .await
            .map_err(ServiceError::Database)?;
        tx.commit().await.map_err(ServiceError::Database)?;
        self.read_policy_registers().await?;
        Ok(version)
    }

    /// The category map `document` compiled and written canonically, when it
    /// holds.
    fn checked_categories(
        &self,
        document: &str,
    ) -> Result<(crate::system_log::categories::Classifier, String), ServiceError> {
        let map: CategoryMapConfig = parse(Register::AuditCategories, document)?;
        let classifier = map.compile().map_err(|errors| {
            ServiceError::precondition(format!(
                "the category map does not compile: {}",
                errors
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ")
            ))
        })?;
        if self.require_category_map && map.source() == MapSource::None {
            return Err(ServiceError::precondition(
                "the category map classifies nothing (no template, no archetype and shipped_map \
                 false), which deployment.profile = \"production\" refuses as the \
                 no_category_map gap: declare this deployment's templates and archetypes, set \
                 shipped_map = true, or accept no_category_map by name in deployment.accepts",
            ));
        }
        let delay = self.registers.access_delay();
        let dropped = delay.register.errors(&classifier);
        if !dropped.is_empty() {
            return Err(ServiceError::precondition(format!(
                "the access-delay register would no longer hold under this map: {}",
                dropped.join("; ")
            )));
        }
        Ok((classifier, serde_json::to_string(&map)?))
    }

    /// The access-delay `document` written canonically, when it holds under
    /// the category map in force.
    fn checked_delay(&self, document: &str) -> Result<String, ServiceError> {
        let register: AccessDelayRegister = parse(Register::AccessDelay, document)?;
        let errors = register.errors(&self.registers.categories().classifier);
        if !errors.is_empty() {
            return Err(ServiceError::precondition(errors.join("; ")));
        }
        Ok(serde_json::to_string(&register)?)
    }
}

/// `document` read as the document of `register`.
fn parse<T: serde::de::DeserializeOwned>(
    register: Register,
    document: &str,
) -> Result<T, ServiceError> {
    serde_json::from_str(document).map_err(|error| {
        ServiceError::precondition(format!(
            "the body is not a {register} register document: {error}"
        ))
    })
}
