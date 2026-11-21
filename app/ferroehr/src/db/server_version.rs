// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The version gate of the boot path: every domain's server must run the
//! `PostgreSQL` release FerroEHR is built and tested on, or newer.
//!
//! Each domain pool reads `server_version_num` once (`PostgreSQL` 19 docs,
//! "Preset Options"), since a deployment may place a domain in a database,
//! or on a server, of its own. No specification governs the minimum: our
//! own design, the release every product of the family pins.

use super::DbError;
use super::domain::{Domain, DomainPools};

/// The oldest `PostgreSQL` FerroEHR runs on, as a `server_version_num`.
pub const MINIMUM: i32 = 190_000;

/// Refuses the first domain whose server is older than [`MINIMUM`].
///
/// # Errors
///
/// [`DbError::UnsupportedServer`] naming the domain and both versions, and
/// [`DbError::Sqlx`] when the setting cannot be read.
pub async fn verify(pools: &DomainPools) -> Result<(), DbError> {
    for domain in [Domain::Clinical, Domain::Party, Domain::Audit] {
        let found: i32 = sqlx::query_scalar("SELECT current_setting('server_version_num')::int")
            .fetch_one(pools.get(domain))
            .await?;
        supported(domain, found)?;
    }
    Ok(())
}

/// Refuses a `server_version_num` older than [`MINIMUM`] for `domain`.
fn supported(domain: Domain, found: i32) -> Result<(), DbError> {
    if found < MINIMUM {
        return Err(DbError::UnsupportedServer {
            domain,
            found,
            required: MINIMUM,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::supported;
    use crate::db::DbError;
    use crate::db::domain::Domain;

    #[test]
    fn a_server_older_than_postgresql_19_is_refused_naming_the_domain_and_both_versions() {
        let refused = supported(Domain::Audit, 180_007).expect_err("PostgreSQL 18.7 is refused");
        assert!(matches!(
            refused,
            DbError::UnsupportedServer {
                domain: Domain::Audit,
                found: 180_007,
                required: 190_000
            }
        ));
        assert!(
            refused
                .to_string()
                .ends_with("database runs PostgreSQL server_version_num 180007, and FerroEHR needs 190000 or newer"),
            "{refused}"
        );
    }

    #[test]
    fn postgresql_19_and_newer_are_accepted() {
        for found in [190_000, 190_003, 200_000] {
            assert!(supported(Domain::Clinical, found).is_ok(), "{found}");
        }
    }
}
