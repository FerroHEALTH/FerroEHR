// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The storage domains: one schema, one `search_path`, one pool and one DSN
//! each.
//!
//! A pseudonymisation domain is only as separate as the weakest thing that
//! spans it. Schemas and grants separate it inside one database, but a
//! superuser, a base backup, WAL archiving and physical replication carry every
//! schema of a database together
//! (<https://www.postgresql.org/docs/18/backup-dump.html>), and the law asks for
//! a separation that can reach another machine: EPDV Art. 10 Abs. 1 lit. b
//! requires the data to be stored "von anderen Datenbeständen getrennt", DSV
//! Art. 4 Abs. 5 requires the log to be kept "getrennt vom System, in welchem
//! die Personendaten bearbeitet werden", and the RM reads the same way — the
//! identity cross-reference "could be located on different machines" (BASE
//! `architecture_overview/master07-security.adoc` §Anonymity).
//!
//! So each domain carries its own `[database.<domain>]` DSN, defaulting to the
//! shared `[database] url`. The default is the co-located deployment, unchanged: one
//! database, three schemas, the reciprocal grants and the boot gate
//! ([`crate::db::verify_domain_isolation`]). A regulated deployment moves a
//! domain to its own database or cluster by editing one URL.
//!
//! No openEHR spec governs schemas, pools or database roles — our own
//! design/extension.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::db::DbConfig;
use ferrohealth_secret::secret_url::SecretUrl;

/// One storage domain: the schema it owns, the pool that serves it, and the
/// runtime roles that hold it.
///
/// Two of the three are pseudonymisation domains ([`Self::Clinical`],
/// [`Self::Party`]) and are mutually barred; the third is the audit trail, which openEHR rules out of the EHR content altogether
/// (BASE `architecture_overview/master07-security.adoc` §Access logging) and
/// which the clinical runtime role writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Domain {
    /// The clinical record: EHRs, their versioned objects, definitions, tags.
    Clinical,
    /// The identities: PARTY versioned objects and their change control.
    Party,
    /// The local IHE ATNA Audit Record Repository.
    Audit,
}

impl Domain {
    /// Every domain, in the order their migration sets apply.
    pub const ALL: [Self; 3] = [Self::Clinical, Self::Party, Self::Audit];

    /// The configuration token, the `[database.<domain>]` table name, and the
    /// `PostgreSQL` schema this domain owns, which the migration set and the
    /// `search_path` are built from.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Clinical => "clinical",
            Self::Party => "party",
            Self::Audit => "audit",
        }
    }

    /// The `search_path` every pooled connection serving this domain carries.
    ///
    /// This one constant is the whole routing mechanism: a pool opened with it
    /// reuses every storage function unchanged, because the SQL those functions
    /// emit names its relations unqualified and `search_path` decides which
    /// schema they resolve in. No other domain's schema is ever on it — a
    /// query this pool issues against another domain's relation must fail to
    /// resolve rather than quietly cross the boundary.
    #[must_use]
    pub const fn search_path(self) -> &'static str {
        match self {
            Self::Clinical => "SET search_path TO clinical, ext, public",
            Self::Party => "SET search_path TO party, ext, public",
            Self::Audit => "SET search_path TO audit, ext, public",
        }
    }

    /// The runtime roles that serve this domain — the writer first, then the
    /// read-only twin where one exists.
    ///
    /// The audit trail has none of its own: it is not a pseudonymisation
    /// domain, the clinical runtime role writes it, and the grants say so
    /// (`migrations/audit/0006_grants.sql`).
    #[must_use]
    pub const fn roles(self) -> &'static [&'static str] {
        match self {
            Self::Clinical => &["ferroehr_clinical", "ferroehr_clinical_reader"],
            Self::Party => &["ferroehr_party", "ferroehr_party_reader"],
            Self::Audit => &[],
        }
    }

    /// The other domains' schemas this domain's roles must not be able to read.
    ///
    /// Empty for [`Self::Audit`], which every domain's writer may reach.
    #[must_use]
    pub const fn barred_from(self) -> &'static [&'static str] {
        match self {
            Self::Clinical => &["party"],
            Self::Party => &["clinical"],
            Self::Audit => &[],
        }
    }

    /// Whether this domain is a pseudonymisation domain — one of the two the
    /// separation is about.
    ///
    /// The audit trail is not: openEHR rules the access log out of the EHR
    /// content altogether (BASE `architecture_overview/master07-security.adoc`
    /// §Access logging) and the clinical runtime role writes it, so it shares
    /// that credential by design rather than by omission.
    #[must_use]
    pub const fn is_pseudonymisation_domain(self) -> bool {
        !self.barred_from().is_empty()
    }

    /// Returns the other domains whose SCHEMA this domain's migration set
    /// names, so that schema must exist while the set applies.
    ///
    /// The shipped grant files revoke the runtime roles from the other
    /// domains' schemas by name (`migrations/party/0008_grants.sql` from
    /// `clinical`), and a `REVOKE` on a schema that does not exist fails. A
    /// database that does not hold such a domain gets its schema only for the
    /// duration of the run, empty, and loses it again afterwards.
    #[must_use]
    pub const fn names_schemas_of(self) -> &'static [Self] {
        match self {
            Self::Party => &[Self::Clinical],
            Self::Clinical | Self::Audit => &[],
        }
    }
}

impl std::fmt::Display for Domain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One domain's connection settings: the `[database.<domain>]` table.
///
/// Every key is unset by default, which is what makes the co-located
/// deployment the zero-configuration one: the domain then connects on the
/// shared `[database] url`, and its database is prepared on `[database]
/// migrate_url`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DomainDsn {
    /// This domain's `PostgreSQL` DSN. Unset reuses `[database] url`.
    ///
    /// Credentials are redacted from every rendering ([`SecretUrl`]).
    pub url: Option<SecretUrl>,
    /// Path to a file holding [`Self::url`], read at boot in place of it — the
    /// route for a deployment that mounts its database credential as a file
    /// rather than passing it as an environment value, which is readable
    /// through `/proc/<pid>/environ` and inherited by every child process.
    /// Setting both is a boot error.
    pub url_file: Option<PathBuf>,
    /// The credential that prepares the database this domain lives in when
    /// that is not the database `[database] migrate_url` reaches.
    ///
    /// `[database] migrate_url` names ONE database. A domain relocated to
    /// another one is prepared there, and its runtime DSN is the wrong
    /// credential for that: under the separated posture it is a DML-only role
    /// that can neither issue the DDL `apply` runs nor read the `ext`
    /// bookkeeping `verify` compares. Unset, the relocated domain's database
    /// is prepared on the domain's runtime DSN, which is what a single-role
    /// sandbox runs. No openEHR spec governs migration mechanics or database
    /// roles — our own design/extension.
    pub migrate_url: Option<SecretUrl>,
    /// Path to a file holding [`Self::migrate_url`], read at boot in place of
    /// it. Setting both is a boot error.
    pub migrate_url_file: Option<PathBuf>,
}

impl DbConfig {
    /// The migration credential configured for the database `domain` lives
    /// in, when `[database.<domain>] migrate_url` names one.
    #[must_use]
    pub fn domain_migrate_dsn(&self, domain: Domain) -> Option<&str> {
        self.domain(domain)
            .migrate_url
            .as_ref()
            .map(SecretUrl::expose)
    }

    /// One domain's settings.
    #[must_use]
    pub const fn domain(&self, domain: Domain) -> &DomainDsn {
        match domain {
            Domain::Clinical => &self.clinical,
            Domain::Party => &self.party,
            Domain::Audit => &self.audit,
        }
    }

    /// One domain's settings, mutably — the loader's seam for resolving
    /// `url_file` into `url`.
    pub const fn domain_mut(&mut self, domain: Domain) -> &mut DomainDsn {
        match domain {
            Domain::Clinical => &mut self.clinical,
            Domain::Party => &mut self.party,
            Domain::Audit => &mut self.audit,
        }
    }

    /// The DSN a domain connects on: its own when the deployment named one,
    /// the shared `[database] url` otherwise.
    #[must_use]
    pub fn dsn(&self, domain: Domain) -> &str {
        self.domain(domain)
            .url
            .as_ref()
            .map_or_else(|| self.url.expose(), SecretUrl::expose)
    }

    /// Whether this domain connects on a DSN of its own rather than the shared
    /// one.
    #[must_use]
    pub const fn is_separated(&self, domain: Domain) -> bool {
        self.domain(domain).url.is_some()
    }

    /// Whether the two pseudonymisation domains connect on two different
    /// DSNs.
    ///
    /// Distinctness rather than "each one is set": both domains relocated to
    /// the SAME second database still share a credential, and a posture check
    /// that counted them as separated would report a separation that does not
    /// exist. The audit trail is deliberately not counted — it is
    /// not a pseudonymisation domain, and requiring a fourth credential for it
    /// would make the check say something other than what it means.
    #[must_use]
    pub fn pseudonymisation_domains_are_separated(&self) -> bool {
        let layout = self.layout();
        !layout
            .placement(Domain::Clinical)
            .shares_dsn_with(layout.placement(Domain::Party))
    }

    /// The resolved layout: which DSN each domain connects on, and which
    /// domains share one.
    #[must_use]
    pub fn layout(&self) -> DomainLayout {
        let place = |domain: Domain| DomainPlacement {
            domain,
            dsn: self.dsn(domain).to_owned(),
            separated: self.is_separated(domain),
        };
        DomainLayout {
            clinical: place(Domain::Clinical),
            party: place(Domain::Party),
            audit: place(Domain::Audit),
        }
    }
}

/// Where one domain is placed: the DSN it connects on, and whether that DSN is
/// its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainPlacement {
    /// The domain.
    pub domain: Domain,
    /// The DSN it connects on, shared or its own. Never rendered: it carries a
    /// credential.
    dsn: String,
    /// Whether `[database.<domain>] url` named this DSN, rather than `[database] url`.
    pub separated: bool,
}

impl DomainPlacement {
    /// Whether this domain and `other` connect on the same DSN text.
    ///
    /// DSN text is what the CONFIGURATION says, which is the question this
    /// answers: whether the operator asked for two connections or one. Whether
    /// two textually different DSNs reach one cluster is a different question,
    /// answered from `pg_control_system()` rather than from a string
    /// ([`crate::db::cluster_identity`], #3226).
    #[must_use]
    pub fn shares_dsn_with(&self, other: &Self) -> bool {
        self.dsn == other.dsn
    }
}

/// The resolved placement of all three domains — what `config check` prints and
/// what the boot gate reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainLayout {
    /// Where the clinical domain is placed.
    clinical: DomainPlacement,
    /// Where the party domain is placed.
    party: DomainPlacement,
    /// Where the audit domain is placed.
    audit: DomainPlacement,
}

impl DomainLayout {
    /// One domain's placement.
    #[must_use]
    pub const fn placement(&self, domain: Domain) -> &DomainPlacement {
        match domain {
            Domain::Clinical => &self.clinical,
            Domain::Party => &self.party,
            Domain::Audit => &self.audit,
        }
    }

    /// Every placement, in [`Domain::ALL`] order.
    #[must_use]
    pub const fn placements(&self) -> [&DomainPlacement; 3] {
        [&self.clinical, &self.party, &self.audit]
    }

    /// Returns the distinct DSNs, each with the domains placed on it, in domain
    /// order.
    ///
    /// The grouping by configured DSN TEXT, which is what `config check`
    /// describes. Schema preparation groups by the database a connection
    /// reaches instead (`pg_control_system()` and `current_database()`), since
    /// two DSNs that differ only in host name or credential reach one database.
    #[must_use]
    pub fn groups(&self) -> Vec<DomainGroup<'_>> {
        let mut groups: Vec<DomainGroup<'_>> = Vec::new();
        for placement in self.placements() {
            if let Some(group) = groups.iter_mut().find(|group| group.dsn == placement.dsn) {
                group.domains.push(placement.domain);
            } else {
                groups.push(DomainGroup {
                    dsn: placement.dsn.as_str(),
                    domains: vec![placement.domain],
                });
            }
        }
        groups
    }

    /// One line per group, naming where its domains are placed and never its
    /// DSN.
    ///
    /// What `ferroehr config check` prints: an operator reading it can see at a
    /// glance whether the deployment they believe they configured is the one
    /// the server resolved.
    #[must_use]
    pub fn describe(&self) -> String {
        let mut out = String::from("storage domain layout:");
        for group in self.groups() {
            let names: Vec<&str> = group.domains.iter().map(|d| d.as_str()).collect();
            let separated = group
                .domains
                .first()
                .is_some_and(|domain| self.placement(*domain).separated);
            out.push_str("\n  - ");
            out.push_str(&names.join(", "));
            match group.domains.first() {
                Some(domain) if separated => {
                    out.push_str(": its own [database.");
                    out.push_str(domain.as_str());
                    out.push_str("] DSN");
                }
                _ => out.push_str(": [database] url"),
            }
            if group.domains.len() > 1 {
                out.push_str(" (one connection, shared)");
            }
        }
        out
    }
}

/// The domains placed on one DSN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainGroup<'a> {
    /// The DSN they share. Never rendered: it carries a credential.
    dsn: &'a str,
    /// The domains on it, in [`Domain::ALL`] order.
    pub domains: Vec<Domain>,
}

impl DomainGroup<'_> {
    /// The DSN this group connects on — for the caller that opens it, never
    /// for a rendering.
    #[must_use]
    pub const fn dsn(&self) -> &str {
        self.dsn
    }
}

/// The three connection pools the server runs on, one per storage domain.
///
/// They differ in `search_path` always, and in credential when the deployment
/// gives a domain its own `[database.<domain>] url`. Holding them in one value
/// is what makes "no statement joins two domains" structural: a caller reaches
/// for a domain, never for a connection.
#[derive(Debug, Clone)]
pub struct DomainPools {
    /// Serves the `clinical` schema.
    pub clinical: PgPool,
    /// Serves the `party` schema.
    pub party: PgPool,
    /// Serves the `audit` schema.
    pub audit: PgPool,
}

impl DomainPools {
    /// The pool serving one domain.
    #[must_use]
    pub const fn get(&self, domain: Domain) -> &PgPool {
        match domain {
            Domain::Clinical => &self.clinical,
            Domain::Party => &self.party,
            Domain::Audit => &self.audit,
        }
    }

    /// Three lazily-opened pools over the DSN one existing pool already holds —
    /// the co-located deployment's three domains, for a caller that has a
    /// [`PgPool`] and no configuration.
    ///
    /// [`crate::db::domain_pool_from`] per domain: pool defaults rather than a
    /// deployment's tuning, and no connection opened until one is asked for.
    #[must_use]
    pub fn from_shared(pool: &PgPool) -> Self {
        Self {
            clinical: crate::db::domain_pool_from(pool, Domain::Clinical),
            party: crate::db::domain_pool_from(pool, Domain::Party),
            audit: crate::db::domain_pool_from(pool, Domain::Audit),
        }
    }

    /// Close every pool, in domain order.
    pub async fn close(&self) {
        for domain in Domain::ALL {
            self.get(domain).close().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_domain_has_its_own_search_path_and_names_no_other_schema() {
        for domain in Domain::ALL {
            let path = domain.search_path();
            assert!(path.contains(domain.as_str()), "{domain}: {path}");
            for other in Domain::ALL {
                if other != domain {
                    assert!(
                        !path.contains(&format!(" {}", other.as_str())),
                        "{domain}'s search path names {other}: {path}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_default_layout_places_every_domain_on_the_shared_dsn() {
        let layout = DbConfig::default().layout();
        let groups = layout.groups();
        assert_eq!(groups.len(), 1, "{groups:?}");
        assert_eq!(groups[0].domains, Domain::ALL);
        for domain in Domain::ALL {
            assert!(!layout.placement(domain).separated);
        }
    }

    #[test]
    fn a_relocated_domain_is_its_own_group() {
        let storage = DbConfig {
            party: DomainDsn {
                url: Some(SecretUrl::new("postgres://p@party-host/party")),
                ..DomainDsn::default()
            },
            ..DbConfig::default()
        };
        let layout = storage.layout();
        let groups = layout.groups();
        assert_eq!(groups.len(), 2, "{groups:?}");
        assert_eq!(groups[0].domains, vec![Domain::Clinical, Domain::Audit]);
        assert_eq!(groups[1].domains, vec![Domain::Party]);
        assert!(layout.placement(Domain::Party).separated);
        assert!(
            layout.describe().contains("its own [database.party] DSN"),
            "{}",
            layout.describe()
        );
    }

    /// The linkage domain is gone (#4684): a configuration that names it is
    /// refused like any unknown key, never run without the separation it
    /// asked for.
    #[test]
    fn a_linkage_domain_is_refused() {
        let err = toml::from_str::<crate::config::FerroEhrConfig>(
            "[database.linkage]\nurl = \"postgres://u:p@h:5432/d\"\n",
        )
        .expect_err("an unknown key");
        assert!(err.to_string().contains("linkage"), "{err}");
    }

    #[test]
    fn the_layout_description_never_carries_a_dsn() {
        let storage = DbConfig {
            party: DomainDsn {
                url: Some(SecretUrl::new("postgres://secret:pw@host/party")),
                ..DomainDsn::default()
            },
            ..DbConfig::default()
        };
        let described = storage.layout().describe();
        assert!(!described.contains("pw"), "{described}");
        assert!(!described.contains("postgres://"), "{described}");
    }
}
