// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The locale the viewer renders in, and the catalogs of its own words.
//!
//! The viewer's words are messages of Fluent catalogs under the crate's
//! `locales/` directory, one catalog for each part of the viewer that looks
//! them up (`locales/<part>/<tag>/<part>.ftl`), embedded at build time by
//! `fluent_templates::static_loader!`, the mechanism FerroUI catalogs its own
//! words with. [`Msg`] holds the words of the shell and of more than one
//! group; each route group (`crate::routes`) has its own message type and
//! catalog in a submodule, and so do the words only the server draws.
//! English is the one language that ships and the one every lookup falls
//! back to; a language joins as one catalog file in each part's directory
//! and no code change.
//!
//! The server chooses the locale once per request: the best language of the
//! request's `Accept-Language` the catalog holds, else the configured
//! `[ui] locale`, and holds it in the request's scope ([`crate::scope`]). The
//! document writes it as `<html lang>` (WCAG 2.2 SC 3.1.1), and every FerroUI
//! part is handed the same locale ([`current`]), so the viewer's words and
//! the components' words switch together. No openEHR spec governs the
//! viewer's localisation or the catalog layout: our own design.

use ferrohealth_ui::i18n::{Arg, Locale, Message};
use fluent_templates::Loader;

pub mod console;
pub mod definitions;
pub mod overview;
pub mod queries;
pub mod records;
pub mod server;
pub mod signin;

/// A message of one of the viewer's catalogs.
pub trait Catalogued: Copy {
    /// The message's id in its catalog.
    fn id(self) -> &'static str;

    /// The catalogs that hold the message, every language they ship.
    fn catalog() -> &'static (dyn Loader + Send + Sync);
}

/// Every language the viewer's catalogs ship, English first.
#[must_use]
pub fn shipped() -> Vec<Locale> {
    let mut tags: Vec<String> = catalog().locales().map(ToString::to_string).collect();
    tags.sort_by_key(|tag| (tag != "en", tag.clone()));
    tags.iter()
        .filter_map(|tag| Locale::parse(tag).ok())
        .collect()
}

/// The shell's catalogs, every language the viewer ships.
#[must_use]
pub fn catalog() -> &'static (dyn Loader + Send + Sync) {
    Msg::catalog()
}

/// Declares one catalog's messages, the type `$name`, each variant naming
/// one message of the catalogs under the directory `$dir` by its id.
macro_rules! messages {
    (
        $(#[$doc:meta])+
        $name:ident in $dir:tt {
            $($(#[$meta:meta])+ $variant:ident = $id:literal,)+
        }
    ) => {
        fluent_templates::static_loader! {
            static CATALOG = {
                locales: $dir,
                fallback_language: "en",
                // NOTE: Fluent's guide, "Bidirectionality": the isolation marks around a
                // placeable are for mixed-direction text; the English catalog draws none.
                customise: |bundle| {
                    bundle.set_use_isolating(false);
                    // FerroUI's formatter groups each number in the page's language,
                    // from each catalog's `number-group` (FerroUI #293).
                    bundle.set_formatter(Some(ferrohealth_ui::i18n::number::format_number));
                },
            };
        }

        $(#[$doc])+
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name {
            $($(#[$meta])+ $variant,)+
        }

        #[expect(
            clippy::same_name_method,
            reason = "the inherent id is const for the catalog tests; the trait's id serves the lookups"
        )]
        impl $name {
            /// Every message, in declaration order.
            pub const ALL: &[Self] = &[$(Self::$variant,)+];

            /// The message's id in its catalog.
            #[must_use]
            pub const fn id(self) -> &'static str {
                match self {
                    $(Self::$variant => $id,)+
                }
            }
        }

        impl $crate::locale::Catalogued for $name {
            fn id(self) -> &'static str {
                $name::id(self)
            }

            fn catalog() -> &'static (dyn fluent_templates::Loader + Send + Sync) {
                &*CATALOG
            }
        }
    };
}

pub(crate) use messages;

messages! {
    /// A string the viewer draws, by its message in the catalog of the
    /// shell and the words more than one group draws.
    Msg in "./locales/shell" {
        /// The title of the page for a path the viewer does not serve.
        Title404 = "not-found-title",
        /// That page's heading.
        Heading404 = "not-found-heading",
        /// That page's sentence.
        Body404 = "not-found-body",
        /// `Example`
        Example = "example",
        /// `Path catalog`
        PathCatalog = "path-catalog",
        /// `AOM2 JSON`
        Aom2Json = "aom2-json",
        /// `Source`
        Source = "source",
        /// `Choose an ADL2 file`
        ChooseAnAdl2File = "choose-an-adl2-file",
        /// `Choose an OPT/XML file`
        ChooseOptXmlFile = "choose-opt-xml-file",
        /// `operational_template (adl_version=2.0.6; rm_release=1.0.2; generated) …`
        OperationalTemplateAdlVersion20 = "operational-template-adl-version-2-0",
        /// `<template xmlns="http://schemas.openehr.org/v1"> …`
        TemplateXmlnsHttpSchemasOpenehrOrg = "template-xmlns-http-schemas-openehr-org",
        /// `Upload an ADL2 operational template`
        UploadAdl2OperationalTemplate = "upload-adl2-operational-template",
        /// `Upload an operational template`
        UploadAnOperationalTemplate = "upload-an-operational-template",
        /// `Upload ADL2`
        UploadAdl2 = "upload-adl2",
        /// `Upload OPT`
        UploadOpt = "upload-opt",
        /// `ADL 2`
        Adl2 = "adl-2",
        /// `ADL 1.4`
        Adl14 = "adl-1-4",
        /// `Input`
        Input = "input",
        /// `Output`
        Output = "output",
        /// `Complete`
        Complete = "complete",
        /// `Medium`
        Medium = "medium",
        /// `Required`
        Required = "required",
        /// a log filter needs at least one directive, e.g. `ferroehr=debug`
        LogFilterNeedsLeastOneDirective = "log-filter-needs-least-one-directive",
        /// `no metric name given`
        NoMetricNameGiven = "no-metric-name-given",
        /// `openEHR specification pins`
        OpenehrSpecificationPins = "openehr-specification-pins",
        /// `DB connections in use`
        DbConnectionsInUse = "db-connections-in-use",
        /// `AQL queries`
        AqlQueries = "aql-queries",
        /// `Compositions committed`
        CompositionsCommitted = "compositions-committed",
        /// `change type`
        ChangeType2 = "change-type-2",
        /// no current version is known — reload the page and retry
        NoCurrentVersionKnownReloadPage = "no-current-version-known-reload-page",
        /// `the composition body is empty`
        CompositionBodyEmpty = "composition-body-empty",
        /// `pick a date and time to travel to`
        PickDateTimeTravel = "pick-date-time-travel",
        /// `a party id is required`
        PartyIdRequired = "party-id-required",
        /// `a version uid is required to read a past version`
        VersionUidRequiredReadPastVersion = "version-uid-required-read-past-version",
        /// `Roles`
        Roles = "roles",
        /// `People`
        People = "people",
        /// `Organisations`
        Organisations = "organisations",
        /// `Groups`
        Groups = "groups",
        /// `Agents`
        Agents = "agents",
        /// `a tag key is required`
        TagKeyRequired = "tag-key-required",
        /// `a tag key is required (ITEM_TAG invariant Inv_key_valid: the key is not empty)`
        TagKeyRequiredItemTagInvariant = "tag-key-required-item-tag-invariant",
        /// `Cancel`
        Cancel = "cancel",
        /// `a version uid is required to read a past EHR_STATUS version`
        VersionUidRequiredReadPastEhr = "version-uid-required-read-past-ehr",
        /// `a tag target id is required`
        TagTargetIdRequired = "tag-target-id-required",
        /// `both a subject id and a subject namespace are required`
        BothSubjectIdSubjectNamespaceRequired = "both-subject-id-subject-namespace-required",
        /// `provide both a subject id and a subject namespace, or leave both empty`
        ProvideBothSubjectIdSubjectNamespace = "provide-both-subject-id-subject-namespace",
        /// `EHR response carried no ehr_id`
        EhrResponseCarriedNoEhrId = "ehr-response-carried-no-ehr-id",
        /// `EHR Status`
        EhrStatus = "ehr-status",
        /// `Query name`
        QueryName = "query-name",
        /// `no ADL2 artefact id to delete`
        NoAdl2ArtefactIdDelete = "no-adl2-artefact-id-delete",
        /// `there is no ADL2 source to upload`
        ThereNoAdl2SourceUpload = "there-no-adl2-source-upload",
        /// `the selected file was empty`
        SelectedFileWasEmpty = "selected-file-was-empty",
        /// `Exact version`
        ExactVersion = "exact-version",
        /// `Version prefix (latest match)`
        VersionPrefixLatestMatch = "version-prefix-latest-match",
        /// `Latest version (no version sent)`
        LatestVersionNoVersionSent = "latest-version-no-version-sent",
        /// `no subscription id to delete`
        NoSubscriptionIdDelete = "no-subscription-id-delete",
        /// `no subscription id to update`
        NoSubscriptionIdUpdate = "no-subscription-id-update",
        /// a subscription name is required and may hold only letters, digits, `_`, `.` and `-`
        SubscriptionNameRequiredMayHoldOnly = "subscription-name-required-may-hold-only",
        /// `Matches every committed version.`
        MatchesEveryCommittedVersion = "matches-every-committed-version",
        /// `ITEM_TAG list JSON: '$e'`
        ItemTagListJson = "item-tag-list-json",
        /// `$with_secondsZ`
        Z = "z",
        /// readiness JSON: `$e`
        ReadinessJson = "readiness-json",
        /// management info JSON: `$e`
        ManagementInfoJson = "management-info-json",
        /// metric list JSON: `$e`
        MetricListJson = "metric-list-json",
        /// metric detail JSON: `$e`
        MetricDetailJson = "metric-detail-json",
        /// loggers JSON: `$e`
        LoggersJson = "loggers-json",
        /// loggers JSON: `$e`
        LoggersJson2 = "loggers-json-2",
        /// loggers JSON: `$e`
        LoggersJson3 = "loggers-json-3",
        /// the composition changed since this page loaded — reload and retry. `$detail`
        CompositionChangedSincePageLoadedReload = "composition-changed-since-page-loaded-reload",
        /// versioned composition JSON: `$e`
        VersionedCompositionJson = "versioned-composition-json",
        /// version JSON: `$e`
        VersionJson = "version-json",
        /// revision history JSON: `$e`
        RevisionHistoryJson = "revision-history-json",
        /// versioned demographic object JSON: `$e`
        VersionedDemographicObjectJson = "versioned-demographic-object-json",
        /// demographic version JSON: `$e`
        DemographicVersionJson = "demographic-version-json",
        /// EHR JSON: `$e`
        EhrJson = "ehr-json",
        /// `ehr_status JSON: '$e'`
        EhrStatusJson = "ehr-status-json",
        /// `versioned ehr_status JSON: '$e'`
        VersionedEhrStatusJson = "versioned-ehr-status-json",
        /// version JSON: `$e`
        VersionJson3 = "version-json-3",
        /// EHR JSON: `$e`
        EhrJson2 = "ehr-json-2",
        /// AQL result JSON: `$e`
        AqlResultJson = "aql-result-json",
        /// EHR JSON: `$e`
        EhrJson3 = "ehr-json-3",
        /// OPT 1.4 parse: `$e`
        Opt14Parse = "opt-1-4-parse",
        /// WebTemplate build: `$value0`
        WebtemplateBuild2 = "webtemplate-build-2",
        /// template list JSON: `$e`
        TemplateListJson = "template-list-json",
        /// ADL2 template list JSON: `$e`
        Adl2TemplateListJson = "adl2-template-list-json",
        /// Matches `$parts`.
        Matches = "matches",
        /// event subscription list JSON: `$e`
        EventSubscriptionListJson = "event-subscription-list-json",
        /// event subscription record JSON: `$e`
        EventSubscriptionRecordJson = "event-subscription-record-json",
        /// `The CDR ingests an ADL 1.4 operational template as OPT/XML. Choose a file or paste the...`
        CdrIngestsAdl14Operational = "cdr-ingests-adl-1-4-operational",
        /// `The CDR ingests the ADL2 artefact SOURCE as text/plain. Choose a file or paste the...`
        CdrIngestsAdl2ArtefactSourceText = "cdr-ingests-adl2-artefact-source-text",
        /// `the latest version's full OBJECT_VERSION_ID is required to delete a composition:...`
        LatestVersionSFullObjectVersion = "latest-version-s-full-object-version",
        /// `the current EHR_STATUS version uid is required to update it: reload this tab and retry`
        CurrentEhrStatusVersionUidRequired = "current-ehr-status-version-uid-required",
        /// The CDR already holds a subscription with that name, so `$object` was not saved...
        CdrAlreadyHoldsSubscriptionNameSo = "cdr-already-holds-subscription-name-so",
        /// `$object is not on the CDR any more ($message): another operator may have deleted it....`
        NotCdrAnyMoreAnotherOperator = "not-cdr-any-more-another-operator",
        /// This session may not administer `$object` (`$message`). Sign in with an ADMIN-role...
        SessionMayNotAdministerSignAdmin = "session-may-not-administer-sign-admin",
        /// A value the server did not state.
        NoValue = "no-value",
        /// `FerroEHR`
        BrandName = "brand-name",
        /// `Viewer`
        BrandWord = "brand-word",
        /// `CDR unreachable`
        CdrUnreachableCode = "cdr-unreachable-code",
        /// `Discard`
        Discard = "discard",
        /// `Decision`
        DockLabel = "dock-label",
        /// `You are signed in as { $who }, and the CDR shows this only to { $needs }.`
        ForbiddenBody = "forbidden-body",
        /// `Viewer error`
        InternalCode = "internal-code",
        /// `Not sent`
        InvalidCode = "invalid-code",
        /// `About`
        NavAbout = "nav-about",
        /// `Audit log`
        NavAudit = "nav-audit",
        /// `Configuration`
        NavConfiguration = "nav-configuration",
        /// `Dashboard`
        NavDashboard = "nav-dashboard",
        /// `EHRs`
        NavEhrs = "nav-ehrs",
        /// `Console`
        NavGroupConsole = "nav-group-console",
        /// `Definitions`
        NavGroupDefinitions = "nav-group-definitions",
        /// `Overview`
        NavGroupOverview = "nav-group-overview",
        /// `Queries`
        NavGroupQueries = "nav-group-queries",
        /// `Records`
        NavGroupRecords = "nav-group-records",
        /// `Viewer`
        NavLabel = "nav-label",
        /// `Parties`
        NavParties = "nav-parties",
        /// `Query`
        NavQuery = "nav-query",
        /// `Server`
        NavServer = "nav-server",
        /// `Stored queries`
        NavStoredQueries = "nav-stored-queries",
        /// `Subscriptions`
        NavSubscriptions = "nav-subscriptions",
        /// `Templates`
        NavTemplates = "nav-templates",
        /// `Terminology`
        NavTerminology = "nav-terminology",
        /// `FerroEHR { $version } at { $address }`
        PillDetail = "pill-detail",
        /// `FerroEHR { $version } at { $address } · { $profile } deployment`
        PillDetailProfile = "pill-detail-profile",
        /// `CDR refused 401`
        PillRefused = "pill-refused",
        /// `CDR unknown`
        PillUnknown = "pill-unknown",
        /// `CDR unreachable`
        PillUnreachable = "pill-unreachable",
        /// `No answer from { $address }`
        PillUnreachableDetail = "pill-unreachable-detail",
        /// `CDR up`
        PillUp = "pill-up",
        /// `FerroEHR Viewer`
        ProductName = "product-name",
        /// `Details`
        SandboxDetails = "sandbox-details",
        /// `{ $count ->`
        SandboxGaps = "sandbox-gaps",
        /// `This deployment must not hold real patient data.`
        SandboxNoGaps = "sandbox-no-gaps",
        /// `Sandbox`
        SandboxTitle = "sandbox-title",
        /// `Find in the CDR`
        SearchLabel = "search-label",
        /// `Id, name or subject id`
        SearchPlaceholder = "search-placeholder",
        /// `Find`
        SearchSubmit = "search-submit",
        /// `Search waits for the CDR`
        SearchWaits = "search-waits",
        /// `Session ended`
        SessionEndedCode = "session-ended-code",
        /// `Signed in with a CDR account`
        SignedInBasic = "signed-in-basic",
        /// `Signed in at the identity provider`
        SignedInOidc = "signed-in-oidc",
        /// `an unknown version`
        UnknownVersion = "unknown-version",
        /// `Your access`
        YourAccess = "your-access",
        /// `Your menu`
        YourMenu = "your-menu",
        /// `No stored query yet.`
        DashboardNoStoredQuery = "dashboard-no-stored-query",
        /// `Write one in Query`
        DashboardWriteOne = "dashboard-write-one",
        /// `This page could not be read.`
        PageRefused = "page-refused",
        /// `Created`
        ColumnCreated = "column-created",
        /// `Save`
        EditorSave = "editor-save",
        /// `No`
        No = "no",
        /// `Yes`
        Yes = "yes",
        /// `From { $value }`
        ChipFrom = "chip-from",
        /// `To { $value }`
        ChipTo = "chip-to",
        /// `Version`
        ColumnVersion = "column-version",
        /// `The document is not valid UTF-8.`
        EditorUtf8 = "editor-utf-8",
        /// `Apply`
        FilterApply = "filter-apply",
        /// `From`
        FilterFrom = "filter-from",
        /// `Template`
        FilterTemplate = "filter-template",
        /// `To`
        FilterTo = "filter-to",
        /// `This EHR has no subject.`
        RevealNoSubject = "reveal-no-subject",
        /// `Amendment`
        ChangeAmendment = "change-amendment",
        /// `Creation`
        ChangeCreation = "change-creation",
        /// `Deleted`
        ChangeDeleted = "change-deleted",
        /// `Modification`
        ChangeModification = "change-modification",
        /// `Kind`
        ColumnKind = "column-kind",
        /// `Apply`
        StagedApply = "staged-apply",
        /// `Value`
        TagsValue = "tags-value",
        /// `Edit { $name }`
        CompositionEditTitle = "composition-edit-title",
        /// `Format`
        CompositionFormat = "composition-format",
        /// `Versions`
        CompositionVersions = "composition-versions",
        /// `{ $count ->`
        EditorRefusedHeading = "editor-refused-heading",
        /// `Open`
        PartiesOpen = "parties-open",
        /// `Edit`
        PartyEdit = "party-edit",
        /// `Facts`
        PartyFacts = "party-facts",
        /// `Overview`
        PartyOverview = "party-overview",
        /// `none`
        RelationshipNone = "relationship-none",
        /// `Actions`
        ColumnActions = "column-actions",
        /// `Actions for { $name }`
        TemplateActionsFor = "template-actions-for",
        /// `{ $day } { $month }`
        ChartDay = "chart-day",
        /// `Jan`
        MonthShort1 = "month-short-1",
        /// `Feb`
        MonthShort2 = "month-short-2",
        /// `Mar`
        MonthShort3 = "month-short-3",
        /// `Apr`
        MonthShort4 = "month-short-4",
        /// `May`
        MonthShort5 = "month-short-5",
        /// `Jun`
        MonthShort6 = "month-short-6",
        /// `Jul`
        MonthShort7 = "month-short-7",
        /// `Aug`
        MonthShort8 = "month-short-8",
        /// `Sep`
        MonthShort9 = "month-short-9",
        /// `Oct`
        MonthShort10 = "month-short-10",
        /// `Nov`
        MonthShort11 = "month-short-11",
        /// `Dec`
        MonthShort12 = "month-short-12",
        /// `Principal: { $value }`
        AuditChipAgent = "audit-chip-agent",
        /// `Outcome: { $value }`
        AuditChipOutcome = "audit-chip-outcome",
        /// `Action: { $value }`
        AuditChipAction = "audit-chip-action",
        /// `Patient: { $value }`
        AuditChipPatient = "audit-chip-patient",
        /// `Template: { $value }`
        ChipTemplate = "chip-template",
        /// `Composer: { $value }`
        ChipComposer = "chip-composer",
        /// `Any change`
        SubAnyChange = "sub-any-change",
        /// `Any kind`
        SubAnyKind = "sub-any-kind",
        /// `Composition`
        SubComposition = "sub-composition",
        /// `EHR status`
        SubEhrStatus = "sub-ehr-status",
        /// `Folder`
        SubFolder = "sub-folder",
    }
}

/// The viewer's message `msg` in the page's locale.
#[must_use]
pub fn tr<M: Catalogued>(msg: M) -> String {
    tr_in(M::catalog(), &current(), msg)
}

/// The viewer's message `msg` in the page's locale, with the placeables
/// `args` filled in.
#[must_use]
pub fn tr_with<M: Catalogued>(msg: M, args: &[(&'static str, Arg)]) -> String {
    current().lookup_with(M::catalog(), msg.id(), args)
}

/// The viewer's message `msg` in `locale`, with the placeables `args`
/// filled in: for a handler or a task, where no page tree is in reach.
#[must_use]
pub fn tr_in_with<M: Catalogued>(locale: &Locale, msg: M, args: &[(&'static str, Arg)]) -> String {
    locale.lookup_with(M::catalog(), msg.id(), args)
}

/// The viewer's message `msg` as a [`Message`] a component looks up as it
/// draws, such as a listing's noun or an editor's title.
#[must_use]
pub fn message<M: Catalogued>(msg: M) -> Message {
    Message::new(M::catalog(), msg.id())
}

/// The message `msg` of `catalog` in `locale`.
#[must_use]
pub fn tr_in<M: Catalogued>(catalog: &dyn Loader, locale: &Locale, msg: M) -> String {
    locale.lookup(catalog, msg.id())
}

/// The viewer's message `msg` in the locale of the page being rendered,
/// read from the request where no page tree is in reach: for a server
/// function.
#[must_use]
pub fn tr_served<M: Catalogued>(msg: M) -> String {
    current().lookup(M::catalog(), msg.id())
}

/// The viewer's message `msg` with the placeables `args` filled in, in the
/// locale [`tr_served`] reads.
#[must_use]
pub fn tr_served_with<M: Catalogued>(msg: M, args: &[(&'static str, Arg)]) -> String {
    current().lookup_with(M::catalog(), msg.id(), args)
}

/// The locale of the page being drawn, English outside a request.
#[must_use]
pub fn current() -> Locale {
    crate::scope::with(|scope| scope.locale.clone()).unwrap_or_default()
}

/// The locale of a request whose headers are `headers`.
///
/// The language of Your preferences when it names one the viewer ships
/// (family rule 9), else the best language of `Accept-Language` the catalog
/// holds, else the configured `[ui] locale`.
#[must_use]
pub fn negotiated(state: &crate::state::AppState, headers: &http::HeaderMap) -> Locale {
    let configured = Locale::parse(&state.config.ui.locale).unwrap_or_default();
    let header = headers
        .get(http::header::ACCEPT_LANGUAGE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let chosen = headers
        .get(http::header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .map(|cookies| {
            ferrohealth_ui::theme::Preferences::read(cookies, crate::prefs::COOKIE_PREFIX).language
        })
        .and_then(|language| language.tag().map(str::to_owned))
        .filter(|tag| shipped().iter().any(|locale| locale.tag() == *tag))
        .and_then(|tag| Locale::parse(&tag).ok());
    chosen.unwrap_or_else(|| Locale::negotiate_with(header, catalog(), configured))
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::collections::HashMap;
    use std::path::Path;

    use ferrohealth_ui::i18n::Locale;
    use fluent_templates::fluent_bundle::FluentValue;

    use super::console::ConsoleMsg;
    use super::definitions::DefinitionsMsg;
    use super::overview::OverviewMsg;
    use super::queries::QueriesMsg;
    use super::records::RecordsMsg;
    use super::server::ServerMsg;
    use super::signin::SignInMsg;
    use super::{Catalogued, Msg, catalog, tr_in};

    fluent_templates::static_loader! {
        static THROWAWAY = {
            locales: "./tests/locales",
            fallback_language: "qaa",
            customise: |bundle| bundle.set_use_isolating(false),
        };
    }

    /// Every placeable the catalog at `path` names, each set to a number, so
    /// a message that selects a form by one formats whole.
    fn placeables(path: &Path) -> HashMap<Cow<'static, str>, FluentValue<'static>> {
        let source = std::fs::read_to_string(path).expect("the catalog");
        source
            .split("{ $")
            .skip(1)
            .filter_map(|rest| rest.split([' ', '}']).next())
            .map(|name| (Cow::Owned(name.to_owned()), FluentValue::from(3)))
            .collect()
    }

    /// The languages under `locales/<part>/`, sorted.
    fn languages(part: &str) -> Vec<String> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("locales")
            .join(part);
        let mut found: Vec<String> = std::fs::read_dir(&root)
            .expect("the part's catalogs")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        found.sort();
        found
    }

    /// Holds the catalogs of `part` to every message of `M`, in every
    /// language the shell ships.
    fn holds_every<M: Catalogued>(part: &str, all: &[M]) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("locales")
            .join(part);
        let found = languages(part);
        let mut shipped: Vec<String> = M::catalog().locales().map(ToString::to_string).collect();
        shipped.sort();
        assert_eq!(
            found, shipped,
            "a catalog directory of {part} is a language"
        );
        assert_eq!(
            found,
            languages("shell"),
            "{part} ships the shell's languages"
        );
        assert!(
            found.contains(&String::from("en")),
            "English is the reference"
        );
        for tag in found {
            let language = tag.parse().expect("a language");
            let args = placeables(&root.join(&tag).join(format!("{part}.ftl")));
            for msg in all {
                let words = M::catalog().try_lookup_with_args(&language, msg.id(), &args);
                assert!(
                    words.is_some_and(|words| !words.trim().is_empty()),
                    "{part}/{tag} holds {}",
                    msg.id()
                );
            }
        }
    }

    #[test]
    fn every_catalog_directory_is_a_language_holding_every_message() {
        let mut parts = languages("");
        parts.sort();
        assert_eq!(
            parts,
            [
                "console",
                "definitions",
                "overview",
                "queries",
                "records",
                "server",
                "shell",
                "signin"
            ],
            "each directory under locales/ is the catalog of one part"
        );
        holds_every("shell", Msg::ALL);
        holds_every("overview", OverviewMsg::ALL);
        holds_every("records", RecordsMsg::ALL);
        holds_every("definitions", DefinitionsMsg::ALL);
        holds_every("queries", QueriesMsg::ALL);
        holds_every("console", ConsoleMsg::ALL);
        holds_every("signin", SignInMsg::ALL);
        holds_every("server", ServerMsg::ALL);
    }

    #[test]
    fn a_throwaway_catalog_changes_the_words_with_no_code_change() {
        let english = Locale::english();
        assert_eq!("No such screen", tr_in(catalog(), &english, Msg::Title404));
        let throwaway = Locale::parse("qaa").expect("a private-use language");
        assert_eq!("Qot qound", tr_in(&*THROWAWAY, &throwaway, Msg::Title404));
        assert_eq!(
            "Page not found",
            tr_in(catalog(), &throwaway, Msg::Heading404),
            "the shipped catalogs fall back to English"
        );
    }
}
