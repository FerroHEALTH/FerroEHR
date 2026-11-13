// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The application service layer.
//!
//! Our own design: concrete [`FerroEhrService`] methods, one folder per domain
//! area. No openEHR specification governs how a CDR is built inside; the
//! oracles are the wire and content specifications each method serves.
//!
//! Each chapter folder owns its domain logic as inherent `FerroEhrService`
//! methods; the protocol adapter (`ferroehr-rest`) calls them directly.
//! Change-control semantics live in [`crate::versioning`]; SQL row I/O lives
//! in [`crate::storage`] (no openEHR spec governs the SQL — our own design);
//! this layer orchestrates.
//!
//! [`linkage`] is our own extension: the crossings between the demographic
//! and the clinical domain, and the access record each one owes.
//!
//! The root modules beside the chapters carry the cross-chapter service
//! vocabulary: the CONTRIBUTION commit both chapters share
//! (`contribution`), the version-commit envelope ([`version_update`]), the list cursor ([`list`]), the
//! platform-service enumeration ([`platform_service`]), the
//! protocol-adapter response envelope ([`response`]), and the crate-internal
//! ITS-REST datetime-request-parameter decoder (`datetime`).
//!
//! NOTE: no openEHR spec governs the internal layering: our own design; every
//! service call returns [`crate::error::ServiceError`], which the protocol adapter maps onto the wire.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): service-surface signatures over stored canonical \
              fragments and dynamic shapes"
)]

pub mod admin;
pub mod definition;
pub mod demographic;
pub mod ehr;
pub mod ehr_index;
pub mod linkage;
pub mod message;
pub mod query;
pub mod terminology;

pub(crate) mod contribution;
pub(crate) mod datetime;
pub mod list;
pub mod platform_service;
pub mod response;
pub mod version_update;
mod write;

use crate::service::definition::lineage::{ArchetypeLineageCache, archetype_lineage_cache};
use crate::service::ehr::access::EhrAccessCache;
use crate::service::query::config::QueryConfig;
use crate::service::query::plan_cache::PlanCache;
use crate::service::terminology::fhir::FhirTerminologyProvider;

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use sqlx::PgPool;

use crate::db::domain::DomainPools;
use crate::ids::EhrId;
use crate::system_log::sender::AuditSender;
use crate::versioning::SigningCtx;
use crate::versioning::signature::signer::Signer;
use openehr_sdt::flat::cache::WebTemplateCache;

/// The default openEHR system identifier stamped into `EHR.system_id`,
/// `AUDIT_DETAILS.system_id`, and every `OBJECT_VERSION_ID.creating_system_id`.
///
/// This is the fallback only: a deployment sets its own identifier with the
/// `[server] system_id` config key (`FERROEHR__SERVER__SYSTEM_ID`), which the
/// binary wires in via [`FerroEhrService::with_system_id`]. An unset key leaves
/// this value in force.
pub const DEFAULT_SYSTEM_ID: &str = "ferroehr.local";

/// The DB-backed application service — the concrete platform behind the
/// chapter methods.
#[derive(Debug, Clone)]
pub struct FerroEhrService {
    pub(crate) pool: PgPool,
    /// The pool serving the **demographic** pseudonymisation domain — the same
    /// database (or, when a deployment separates the runtime roles, a second
    /// DSN), with `search_path` pointing at the `demographic` schema.
    ///
    /// Every storage function the demographic chapter calls is the clinical
    /// one, unchanged: the SQL names its relations unqualified, so the pool's
    /// search path alone decides which domain it reads and writes. Parties are
    /// physically separated from clinical content (GDPR Art. 4(5) and
    /// Art. 32(1)(a)); no openEHR spec governs storage layout — our own
    /// design/extension.
    pub(crate) demographic_pool: PgPool,
    system_id: String,
    /// The ACTIVE openEHR specification generation set (`spec_profile`).
    /// Boot-fixed; the AQL planner's profile gate and the ingress acceptance
    /// boundary read it.
    pub(crate) spec_profile: crate::config::profile::SpecProfile,
    /// Cache of `WebTemplate`s built from stored OPTs, used by composition
    /// validation on create/update. Cheaply cloneable (moka-backed).
    ///
    /// NOTE: cache health is telemetry only — no wire introspection endpoint
    /// exists or is planned (adjudicated 2026-07-25 on the tracker: an
    /// internal optimisation's state is a metric, not a REST surface; no
    /// openEHR spec governs the cache — our own design).
    pub(crate) web_templates: WebTemplateCache,
    /// Version signer (`VERSION.signature`, RM common master06 §Digital
    /// Signature). Defaults to server-side `digest` signing; the binary wires
    /// the configured [`Signer`].
    signer: Arc<Signer>,
    /// The boot outcome of the `[deployment] licence_file` section
    /// ([`crate::licence::state::LicenceState`]); no licence by default. Read
    /// by `/management/status`; it changes nothing else.
    licence: crate::licence::state::LicenceState,
    /// The declared deployment posture and the separations it found open
    /// ([`crate::config::deployment::DeploymentPosture`]): the profile on
    /// `/rest/status`, the whole posture on `/management/status`; the sandbox
    /// default with nothing evaluated until the binary installs the boot
    /// evaluation.
    deployment: crate::config::deployment::DeploymentPosture,
    /// The stamp key every server-minted identifier carries
    /// ([`crate::licence::stamp`]), derived from `licence` once at boot.
    stamp: crate::licence::stamp::StampKey,
    /// The optional IHE ATNA audit sender of the system log
    /// (`crate::system_log`). `None` = auditing off; the binary
    /// wires the configured [`AuditSender`] via [`Self::with_audit`].
    pub(crate) audit: Option<AuditSender>,
    /// The optional local Audit Record Repository (read side — the ITI-81
    /// retrieval; `crate::system_log::store`). `None` = the local store is
    /// off; the binary wires it via [`Self::with_audit_store`].
    pub(crate) audit_store: Option<crate::system_log::store::AuditStore>,
    /// The optional FHIR R4B terminology server, built when a deployment opts
    /// in ([`TerminologyServerConfig`](crate::service::terminology::config::TerminologyServerConfig)).
    /// Consulted by the terminology calls the bundle does not
    /// answer, composition constraint-binding validation, ADL 2 VETDF, and AQL
    /// `TERMINOLOGY(…)` resolution; `None` keeps terminology on the in-process
    /// `openehr-term` bundle only (BASE
    /// `docs/architecture_overview/master12-terminology.adoc` §Binding
    /// Terminology Value-sets to Archetypes).
    pub(crate) terminology: Option<Arc<FhirTerminologyProvider>>,
    /// The optional `DV_MULTIMEDIA` externalization engine (no openEHR spec
    /// governs media externalization — our own extension,
    /// [`crate::extensions::multimedia`]). `None` (default) = inline
    /// behaviour byte-identical.
    #[cfg(feature = "multimedia")]
    pub(crate) multimedia: Option<Arc<ferroehr_ext::multimedia::MultimediaEngine>>,
    /// Per-EHR cache of the current `EHR_ACCESS` scheme settings ("All access
    /// decisions to data in the EHR must be made in accordance with the
    /// policies and rules in this object" — RM ehr `ehr_access.adoc`).
    /// Invalidated on every `EHR_ACCESS` commit.
    pub(in crate::service) ehr_access: EhrAccessCache,
    /// Bounded cache of lowered AQL plans keyed on the query text. Shared
    /// across service clones (moka-backed). No openEHR spec governs it — our
    /// own performance design.
    pub(crate) plan_cache: PlanCache,
    /// Memo of the stored archetype specialisation graph
    /// (`adl2_artefact.parent_hrid` edges) the AQL archetype predicate widens a
    /// parent query through (BASE Artefact Identification
    /// §Supporting Archetype-based Querying). Invalidated on every local ADL2 artefact
    /// write; the memo itself is spec-silent — our own performance design.
    pub(crate) archetype_lineage: ArchetypeLineageCache,
    /// Per-query DB execution budget (`[record.query] timeout`); `None` disables it
    /// (the global request timeout is then the only guard). On overrun the
    /// query is reported as `408`. No openEHR spec governs a query timeout —
    /// our own extension.
    query_timeout: Option<Duration>,
    /// The row ceiling applied to a query that neither the AQL nor the request
    /// bounds; `None` leaves such a query unbounded. See
    /// [`QueryConfig::max_result_rows`](crate::service::query::config::QueryConfig::max_result_rows).
    query_result_ceiling: Option<i64>,
    /// When the transactional event outbox is written on a commit. The outbox
    /// feeds the readers the change-feed grants name — no openEHR spec governs
    /// eventing (our own extension). With no reader the per-commit
    /// `event_outbox` INSERT (and its envelope serialization) is pure overhead,
    /// so the binary gates it on those grants. [`crate::extensions::outbox::OutboxGate::Always`] in
    /// [`Self::new`] so a bare service (tests, embeddings) never silently drops
    /// an event; the binary sets the real gate via [`Self::with_outbox_gate`].
    pub(crate) outbox_gate: crate::extensions::outbox::OutboxGate,
    /// `[record.versioning] idempotent_resend` (#4415); the binary sets it via
    /// [`Self::with_skip_identical_resend`].
    skip_identical_resend: bool,
    /// Short-lived stash of a just-created EHR's RM `EHR` wire body, built from
    /// the commit results at creation and popped by the `ehr_created_object`
    /// adapter seam so a `Prefer: return=representation` create response is
    /// served without re-reading the EHR (the header + status/access version +
    /// folder reads `ehr_read` would repeat). Bounded + short TTL; an evicted
    /// or absent entry falls back to a full read, so it is invalidation-free by
    /// construction (an EHR's identity at creation is immutable). No openEHR
    /// spec governs it — our own performance design.
    pub(in crate::service) created_ehr_repr: moka::future::Cache<EhrId, Value>,
    /// The national-identifier protection engine
    /// ([`crate::identifier_protection::engine`]), when the
    /// deployment configured one.
    ///
    /// `None` leaves identifiers stored exactly as written, which is the
    /// default posture; the binary installs the compiled
    /// `[privacy.identifier_protection]` section.
    pub(in crate::service) identifiers:
        Option<Arc<crate::identifier_protection::engine::IdentifierProtection>>,
    /// The clinical-side data-minimisation policy
    /// ([`crate::privacy`]) every clinical commit body is checked against.
    ///
    /// [`Self::new`] leaves it unenforced, as it leaves every other configured
    /// collaborator unwired; the binary installs the compiled
    /// `[privacy]` section via [`Self::with_privacy`], and that section's own
    /// default refuses identified parties and scans in `strict`.
    pub(in crate::service) privacy: Arc<crate::privacy::PrivacyPolicy>,
    /// The `[privacy.cohort]` allow-list and limits
    /// ([`crate::service::linkage::cohort::config::CohortConfig`]).
    ///
    /// Empty by default, which leaves the cross-domain cohort surface off: a
    /// deployment that has not stated which demographic leaves may be selected
    /// on has not decided. No openEHR spec governs it — our own extension.
    pub(in crate::service) cohort: Arc<linkage::cohort::config::CohortConfig>,
    /// Who holds the subject-to-EHR cross-reference
    /// ([`ehr_index::provider::EhrIndexProvider`]).
    ///
    /// [`Self::new`] installs none, so every subject-scoped call refuses until
    /// the binary installs the `[pixm]` provider or a test opts in to the
    /// local map.
    pub(in crate::service) ehr_index: ehr_index::provider::EhrIndexProvider,
    /// The operational-policy admin registers in force: the audit category
    /// map, the EHDS Art. 3(3) access delay and the Cedar policy set, each at
    /// its default in [`Self::new`] until [`Self::refresh_policy_registers`]
    /// reads the stored versions.
    pub(in crate::service) registers: Arc<crate::policy_register::LiveRegisters>,
    /// The controller's `ACCESS_POLICY` in force, at no version in
    /// [`Self::new`] until [`Self::read_access_policy`] reads the stored one.
    pub(in crate::service) access_policy: Arc<crate::access_policy::LivePolicy>,
    /// Whether a category register that classifies nothing is refused: the
    /// `no_category_map` floor of a production deployment that audits and has
    /// not accepted the gap by name. `false` in [`Self::new`].
    pub(in crate::service) require_category_map: bool,
    /// The demographic definitions of the shared party mapping; empty in
    /// [`Self::new`], which validates no party against a template.
    pub(in crate::service) party_templates: Arc<demographic::template::MappedPartyTemplates>,
    /// The part of the shared party mapping the binding side table is derived
    /// under; empty in [`Self::new`], which derives no row, so every login is
    /// unbound until the mapping is installed.
    pub(in crate::service) binding: Arc<crate::party_binding::BindingMapping>,
    /// The policy grants in force on the party pool, under the default
    /// `[authz.grants] allowed` until the binary sets the configured one.
    pub(in crate::service) grants: Arc<crate::party_binding::grants::live::LiveGrants>,
    /// Who may import and which wrapped committers an import may carry;
    /// [`Self::new`] lets every caller import with no wrapped-committer rule
    /// beyond its own issuer.
    pub(in crate::service) import_policy: Arc<message::import_policy::ImportPolicy>,
}

impl FerroEhrService {
    /// Construct the service over a domain's already-open pools, with the
    /// default system id and the default (server-side `digest`) version signer.
    ///
    /// It takes [`DomainPools`] rather than one pool because building a pool
    /// spawns that pool's sqlx maintenance task, which panics outside a tokio
    /// runtime: this constructor opens nothing and spawns nothing, so it cannot
    /// panic, and the caller builds the pools where a runtime is in scope
    /// ([`DomainPools::from_shared`] derives all three from one existing pool).
    /// The clinical pool serves this service and the party pool the demographic
    /// chapter, so the SCHEMA separation is always on. A deployment that separates the runtime ROLES, tunes a pool,
    /// or enables tenancy passes pools of its own, or replaces one afterwards
    /// with [`Self::with_demographic_pool`].
    #[must_use]
    pub fn new(pools: &DomainPools) -> Self {
        Self {
            demographic_pool: pools.party.clone(),
            pool: pools.clinical.clone(),
            system_id: DEFAULT_SYSTEM_ID.to_owned(),
            spec_profile: crate::config::profile::SpecProfile::default(),
            web_templates: WebTemplateCache::default(),
            signer: Arc::new(Signer::digest_default()),
            licence: crate::licence::state::LicenceState::NoLicence(
                crate::licence::state::Reason::NoneEmbedded,
            ),
            deployment: crate::config::deployment::DeploymentPosture {
                profile: crate::config::deployment::DeploymentProfile::Sandbox,
                gaps: Vec::new(),
                accepted: Vec::new(),
            },
            stamp: crate::licence::stamp::StampKey::fail_safe(),
            audit: None,
            audit_store: None,
            terminology: None,
            #[cfg(feature = "multimedia")]
            multimedia: None,
            ehr_access: EhrAccessCache::default(),
            plan_cache: PlanCache::default(),
            archetype_lineage: archetype_lineage_cache(),
            query_timeout: None,
            query_result_ceiling: None,
            outbox_gate: crate::extensions::outbox::OutboxGate::Always,
            skip_identical_resend: false,
            created_ehr_repr: moka::future::Cache::builder()
                .max_capacity(4096)
                .time_to_live(Duration::from_secs(30))
                .build(),
            identifiers: None,
            privacy: Arc::new(crate::privacy::PrivacyPolicy::default()),
            cohort: Arc::new(linkage::cohort::config::CohortConfig::default()),
            ehr_index: ehr_index::provider::EhrIndexProvider::Unconfigured,
            registers: Arc::new(crate::policy_register::LiveRegisters::new(
                pools.clinical.clone(),
            )),
            access_policy: Arc::new(crate::access_policy::LivePolicy::new(
                pools.clinical.clone(),
            )),
            require_category_map: false,
            party_templates: Arc::default(),
            binding: Arc::default(),
            grants: Arc::new(crate::party_binding::grants::live::LiveGrants::new(
                pools.party.clone(),
            )),
            import_policy: Arc::default(),
        }
    }

    // ── Builders (the binary wires each configured subsystem) ────────────────

    /// Install the national-identifier protection engine.
    ///
    /// Without it the demographic write path stores identifiers exactly as
    /// written, which is the default posture; the binary installs the compiled
    /// `[privacy.identifier_protection]` section when it is enabled.
    #[must_use]
    pub fn with_identifier_protection(
        mut self,
        engine: Arc<crate::identifier_protection::engine::IdentifierProtection>,
    ) -> Self {
        self.identifiers = Some(engine);
        self
    }

    /// The national-identifier protection engine, when one is configured.
    #[must_use]
    pub fn identifier_protection(
        &self,
    ) -> Option<&crate::identifier_protection::engine::IdentifierProtection> {
        self.identifiers.as_deref()
    }

    /// Install the protection engine when the deployment built one.
    ///
    /// The optional twin of [`Self::with_identifier_protection`], so the
    /// binary's builder chain stays one expression whether or not the section
    /// is enabled.
    #[must_use]
    pub fn with_identifier_protection_opt(
        mut self,
        engine: Option<Arc<crate::identifier_protection::engine::IdentifierProtection>>,
    ) -> Self {
        self.identifiers = engine;
        self
    }

    /// Install the compiled clinical-side data-minimisation policy
    /// ([`crate::privacy::PrivacyPolicy`]).
    ///
    /// Without it the service enforces none of the three rules, which is the
    /// bare-service posture; the binary always calls this with the resolved
    /// `[privacy]` section.
    #[must_use]
    pub fn with_privacy(mut self, policy: Arc<crate::privacy::PrivacyPolicy>) -> Self {
        self.privacy = policy;
        self
    }

    /// Install the `[privacy.cohort]` allow-list and limits.
    ///
    /// Without it the service binds no predicate, so
    /// [`Self::cohort_enabled`] is false and every cohort execution is refused
    /// [`NotConfigured`](crate::service::linkage::cohort::CohortError::NotConfigured);
    /// the binary always calls this with the resolved section.
    #[must_use]
    pub fn with_cohort(mut self, config: linkage::cohort::config::CohortConfig) -> Self {
        self.cohort = Arc::new(config);
        self
    }

    /// Install the pool serving the demographic pseudonymisation domain.
    ///
    /// This is what makes the ROLE separation a deployment choice: the pool may
    /// authenticate as `ferroehr_party` on its own DSN
    /// (`[database].demographic_url`), so neither runtime credential can read the
    /// other domain's relations. Without it, [`Self::new`]'s derived pool runs
    /// the same schema separation over the clinical credential.
    #[must_use]
    pub fn with_demographic_pool(mut self, pool: PgPool) -> Self {
        let grants = crate::party_binding::grants::live::LiveGrants::new(pool.clone());
        grants.set_ceiling((*self.grants.ceiling()).clone());
        grants.set_federation((*self.grants.federation()).clone());
        self.grants = Arc::new(grants);
        self.demographic_pool = pool;
        self
    }

    /// Selects the openEHR specification generation set this service runs
    /// (`spec_profile`; default `development`).
    #[must_use]
    pub fn with_spec_profile(mut self, profile: crate::config::profile::SpecProfile) -> Self {
        self.spec_profile = profile;
        self
    }

    /// Set the openEHR system id (identifies this CDR in `EHR.system_id`,
    /// audit rows, and every minted `OBJECT_VERSION_ID`). The binary wires
    /// `[server] system_id`; without a call, [`DEFAULT_SYSTEM_ID`] stands.
    #[must_use]
    pub fn with_system_id(mut self, system_id: impl Into<String>) -> Self {
        self.system_id = system_id.into();
        self
    }

    /// Install the boot outcome of the `[deployment] licence_file` section. The stamp key
    /// every server-minted identifier carries follows from it; nothing else
    /// does.
    #[must_use]
    pub fn with_licence(mut self, licence: crate::licence::state::LicenceState) -> Self {
        self.stamp = licence.stamp_key();
        self.licence = licence;
        self
    }

    /// Install the boot evaluation of the deployment posture (#3226).
    #[must_use]
    pub fn with_deployment(
        mut self,
        deployment: crate::config::deployment::DeploymentPosture,
    ) -> Self {
        self.deployment = deployment;
        self
    }

    /// The declared deployment posture and its open separations.
    #[must_use]
    pub fn deployment(&self) -> &crate::config::deployment::DeploymentPosture {
        &self.deployment
    }

    /// The boot outcome of the `[deployment] licence_file` section.
    #[must_use]
    pub fn licence(&self) -> &crate::licence::state::LicenceState {
        &self.licence
    }

    /// The stamp key server-minted identifiers carry.
    #[must_use]
    pub fn stamp(&self) -> &crate::licence::stamp::StampKey {
        &self.stamp
    }

    /// Install the configured version [`Signer`] (RM common master06 §Digital
    /// Signature).
    #[must_use]
    pub fn with_signer(mut self, signer: Arc<Signer>) -> Self {
        self.signer = signer;
        self
    }

    /// Install the IHE ATNA audit sender of the system log; the binary boots
    /// it and wires it here.
    #[must_use]
    pub fn with_audit(mut self, sender: AuditSender) -> Self {
        self.registers.attach_audit(sender.clone());
        self.audit = Some(sender);
        self
    }

    /// Install the local Audit Record Repository (the read side serving the
    /// RESTful-ATNA ITI-81 retrieval); the binary wires it when
    /// `[audit.store]` is enabled.
    #[must_use]
    pub fn with_audit_store(mut self, store: crate::system_log::store::AuditStore) -> Self {
        self.audit_store = Some(store);
        self
    }

    /// Install the FHIR R4B terminology server (opt-in). Without it,
    /// terminology is answered by the in-process `openehr-term` bundle alone.
    #[must_use]
    pub fn with_terminology_server(mut self, server: Arc<FhirTerminologyProvider>) -> Self {
        self.terminology = Some(server);
        self
    }

    /// The terminology server, or `None` when the deployment configures none.
    pub(crate) fn terminology_server(&self) -> Option<&FhirTerminologyProvider> {
        self.terminology.as_deref()
    }

    /// Install the `DV_MULTIMEDIA` externalization engine (opt-in extension).
    /// Without it, inline media is stored verbatim (byte-identical).
    #[must_use]
    #[cfg(feature = "multimedia")]
    pub fn with_multimedia(
        mut self,
        engine: Arc<ferroehr_ext::multimedia::MultimediaEngine>,
    ) -> Self {
        self.multimedia = Some(engine);
        self
    }

    /// The origins a retention period may be keyed on: those the grants of
    /// `data_origin` in force name (EHDS Annex II 3.4, [`crate::origins`]).
    ///
    /// # Errors
    /// [`crate::error::ServiceError::Storage`] when the grants must be read and the party
    /// store cannot answer.
    pub async fn registered_origins(
        &self,
    ) -> Result<std::collections::BTreeSet<String>, crate::error::ServiceError> {
        Ok(crate::origins::registered(
            self.fresh_grants().await?,
            jiff::Timestamp::now(),
        ))
    }

    /// The policy grants, read again first when this replica's copy is
    /// stale.
    ///
    /// # Errors
    /// [`crate::error::ServiceError::Storage`] when the grants must be read
    /// and the party store cannot answer.
    pub(crate) async fn fresh_grants(
        &self,
    ) -> Result<&crate::party_binding::grants::live::LiveGrants, crate::error::ServiceError> {
        self.grants
            .refresh_if_stale()
            .await
            .map_err(|error| crate::error::ServiceError::from(error.into_inner()))?;
        Ok(&self.grants)
    }

    /// Sets `[record.versioning] idempotent_resend` (#4415): an update whose
    /// content equals the current version's answers with that version and
    /// commits nothing.
    #[must_use]
    pub fn with_skip_identical_resend(mut self, skip: bool) -> Self {
        self.skip_identical_resend = skip;
        self
    }

    /// Set when the transactional event outbox is written on commit. The
    /// binary sets [`crate::extensions::outbox::OutboxGate::WhileGranted`], so the `event_outbox` INSERT
    /// is skipped while no change-feed grant names a reader. No openEHR spec
    /// governs eventing — our own extension.
    #[must_use]
    pub fn with_outbox_gate(mut self, gate: crate::extensions::outbox::OutboxGate) -> Self {
        self.outbox_gate = gate;
        self
    }

    /// Apply the `[record.query]` tuning knobs — the plan-cache capacity and the
    /// per-query execution budget ([`QueryConfig`]). No openEHR spec governs
    /// these — our own operational extension.
    #[must_use]
    pub fn with_query_config(mut self, query: &QueryConfig) -> Self {
        self.plan_cache = PlanCache::new(query.plan_cache_capacity);
        self.query_timeout = query.budget();
        self.query_result_ceiling = query.result_ceiling();
        self
    }

    // ── Accessors ─────────────────────────────────────────────────────────────

    /// The AQL plan cache, for observability. No openEHR spec governs it.
    #[must_use]
    pub fn plan_cache(&self) -> &PlanCache {
        &self.plan_cache
    }

    /// The openEHR `system_id` in effect for this write.
    ///
    /// One instance is one logical EHR system, so this is the configured
    /// identifier — BASE `architecture_overview/master06-design_of_the_ehr.adoc`
    /// §System Identity: the `system_id` names the system "legally responsible"
    /// for the data, and it "becomes embedded in the version identifiers of
    /// committed -- and possibly signed -- content".
    pub(crate) fn effective_system_id(&self) -> String {
        self.system_id.clone()
    }

    /// The configured version [`Signer`] (used for read-time verification).
    pub(crate) fn signer(&self) -> &Signer {
        &self.signer
    }

    /// The write-time signing context (RM common master06 §Digital Signature)
    /// threaded into every versioned-object commit.
    pub(crate) fn signing_ctx(&self) -> SigningCtx<'_> {
        SigningCtx {
            system_id: self.effective_system_id(),
            signer: &self.signer,
            stamp: &self.stamp,
            spec_profile: self.spec_profile,
            #[cfg(feature = "multimedia")]
            multimedia: self.multimedia.as_deref(),
            identifiers: self.identifiers.as_deref(),
            outbox_enabled: self.outbox_written(),
            skip_identical_resend: self.skip_identical_resend,
            binding: &self.binding,
            grants: &self.grants,
            access_delay: self.registers.access_delay().policy.clone(),
            direct_origin: crate::origins::DirectOrigin::current(&self.system_id),
        }
    }
}
