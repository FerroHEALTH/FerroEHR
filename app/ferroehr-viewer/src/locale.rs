// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The locale the viewer renders in, and the catalog of its own words.
//!
//! The viewer's words are messages of the Fluent catalogs under the crate's
//! `locales/` directory, one directory per language
//! (`locales/en/ferroehr-viewer.ftl`), embedded at build time by
//! `fluent_templates::static_loader!`, the mechanism FerroUI catalogs its own
//! words with. English is the one catalog that ships and the one every lookup
//! falls back to; a language joins as one catalog file and no code change.
//!
//! The server chooses the locale once per request: the best language of the
//! request's `Accept-Language` the catalog holds, else the configured
//! `[ui] locale`. The document writes it as `<html lang>` (WCAG 2.2 SC 3.1.1),
//! the browser reads it back from there, and [`crate::app::App`] provides it
//! to FerroUI with `ferrohealth_ui::i18n::provide_locale`, so the viewer's
//! words and the components' words switch together. No openEHR spec governs
//! the viewer's localisation: our own design.

use ferrohealth_ui::i18n::{Arg, Locale, Message, locale};
use fluent_templates::Loader;

fluent_templates::static_loader! {
    static CATALOG = {
        locales: "./locales",
        fallback_language: "en",
        // NOTE: Fluent's guide, "Bidirectionality": the isolation marks around a
        // placeable are for mixed-direction text; the English catalog draws none.
        customise: |bundle| bundle.set_use_isolating(false),
    };
}

/// The viewer's catalogs, every language it ships.
#[must_use]
pub fn catalog() -> &'static (dyn Loader + Send + Sync) {
    &*CATALOG
}

/// Declares the viewer's messages: each variant names one message of the
/// catalog by its id.
macro_rules! messages {
    ($($(#[$meta:meta])+ $name:ident = $id:literal,)+) => {
        /// A string the viewer draws, by its message in the catalog.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Msg {
            $($(#[$meta])+ $name,)+
        }

        impl Msg {
            /// Every message, in declaration order.
            pub const ALL: &[Self] = &[$(Self::$name,)+];

            /// The message's id in the catalog.
            #[must_use]
            pub const fn id(self) -> &'static str {
                match self {
                    $(Self::$name => $id,)+
                }
            }
        }
    };
}

messages! {
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
    /// `no EHR id to delete`
    NoEhrIdDelete = "no-ehr-id-delete",
    /// `a stored-query delete needs both a qualified name and a version`
    StoredQueryDeleteNeedsBothQualified = "stored-query-delete-needs-both-qualified",
    /// `no template id to delete`
    NoTemplateIdDelete = "no-template-id-delete",
    /// `Return to the dashboard`
    ReturnToTheDashboard = "return-to-the-dashboard",
    ///  DESC
    Desc = "desc",
    ///  ASC
    Asc = "asc",
    /// `did not parse as a data path`
    DidNotParseDataPath = "did-not-parse-data-path",
    /// `Row order`
    RowOrder = "row-order",
    /// `Viewer`
    Viewer = "viewer",
    /// `Loading the rows…`
    LoadingTheRows = "loading-the-rows",
    /// `Example form`
    ExampleForm = "example-form",
    /// `Form`
    Form = "form",
    /// `Detail level`
    DetailLevel = "detail-level",
    /// `Format`
    Format = "format",
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
    /// `Document`
    Document = "document",
    /// `Filter`
    Filter = "filter",
    /// `Target path`
    TargetPath = "target-path",
    /// `Value`
    Value = "value",
    /// `Key`
    Key = "key",
    /// `A tag is identified by its key and target path together, so the same key on two...`
    TagIdentifiedKeyTargetPathTogether = "tag-identified-key-target-path-together",
    /// `Save tag`
    SaveTag = "save-tag",
    /// `Target path (optional)`
    TargetPathOptional = "target-path-optional",
    /// `Value (optional)`
    ValueOptional = "value-optional",
    /// `Set a tag`
    SetATag = "set-a-tag",
    /// `A tag needs a key (ITEM_TAG invariant Inv_key_valid).`
    TagNeedsKeyItemTagInvariant = "tag-needs-key-item-tag-invariant",
    /// `Delete`
    Delete = "delete",
    /// `Actions`
    Actions = "actions",
    /// `Target`
    Target = "target",
    /// `Item tags`
    ItemTags = "item-tags",
    /// `Collection: `
    Collection = "collection",
    /// `Delete failed`
    DeleteFailed = "delete-failed",
    /// `A deleted version was committed; earlier versions stay readable in History.`
    DeletedVersionWasCommittedEarlierVersions = "deleted-version-was-committed-earlier-versions",
    /// `X axis`
    XAxis = "x-axis",
    /// `The result set as a chart`
    ResultSetChart = "result-set-chart",
    /// `One scope or a whole space-separated claim. The form is...`
    OneScopeWholeSpaceSeparatedClaim = "one-scope-whole-space-separated-claim",
    /// `patient/composition-*.rs user/template-MyHospital::Template.v0.crud`
    PatientCompositionRsUserTemplateMyhospital = "patient-composition-rs-user-template-myhospital",
    /// `Preview a scope`
    PreviewAScope = "preview-a-scope",
    /// `not recognised`
    NotRecognised = "not-recognised",
    /// `Not a scope openEHR defines. It is carried alongside the openEHR scopes and grants...`
    NotScopeOpenehrDefinesCarriedAlongside = "not-scope-openehr-defines-carried-alongside",
    /// `broad access`
    BroadAccess = "broad-access",
    /// `Version document`
    VersionDocument = "version-document",
    /// `Open that version`
    OpenThatVersion = "open-that-version",
    /// `Date and time (interpreted as UTC)`
    DateTimeInterpretedUtc = "date-time-interpreted-utc",
    /// `Committer`
    Committer = "committer",
    /// `Change type`
    ChangeType = "change-type",
    /// `Committed`
    Committed = "committed",
    /// `Open`
    Open = "open",
    /// `Version`
    Version = "version",
    /// `Revision history`
    RevisionHistory = "revision-history",
    /// `Loading…`
    Loading = "loading",
    /// `multimedia files are stored over a connection without encryption`
    MultimediaFilesStoredOverConnectionWithout = "multimedia-files-stored-over-connection-without",
    /// `access records are sent to the audit repository without encryption`
    AccessRecordsSentAuditRepositoryWithout = "access-records-sent-audit-repository-without",
    /// `change events are published to a message broker without encryption`
    ChangeEventsPublishedMessageBrokerWithout = "change-events-published-message-broker-without",
    /// `a listener serves plain HTTP on an address reachable from the network`
    ListenerServesPlainHttpAddressReachable = "listener-serves-plain-http-address-reachable",
    /// `requests are served without authentication`
    RequestsServedWithoutAuthentication = "requests-served-without-authentication",
    /// `schema preparation runs on the clinical runtime credential`
    SchemaPreparationRunsClinicalRuntimeCredential = "schema-preparation-runs-clinical-runtime-credential",
    /// `access is not recorded to a durable audit trail`
    AccessNotRecordedDurableAuditTrail = "access-not-recorded-durable-audit-trail",
    /// `two domains reach the same PostgreSQL cluster`
    TwoDomainsReachSamePostgresqlCluster = "two-domains-reach-same-postgresql-cluster",
    /// `the clinical, demographic and linkage domains share one database credential`
    ClinicalDemographicLinkageDomainsShareOne = "clinical-demographic-linkage-domains-share-one",
    /// `text/csv; charset=utf-8`
    TextCsvCharsetUtf8 = "text-csv-charset-utf-8",
    /// `parameters must be a JSON object`
    ParametersMustJsonObject = "parameters-must-json-object",
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
    /// `In-flight requests`
    InFlightRequests = "in-flight-requests",
    /// `Older `
    Older = "older",
    ///  Newer
    Newer = "newer",
    /// `Record`
    Record = "record",
    /// `Resource`
    Resource = "resource",
    /// `Patient`
    Patient = "patient",
    /// `Principal`
    Principal = "principal",
    /// `Outcome`
    Outcome = "outcome",
    /// `Operation`
    Operation = "operation",
    /// `Event`
    Event2 = "event-2",
    /// `Recorded`
    Recorded = "recorded",
    /// `Audit records`
    AuditRecords = "audit-records",
    /// `major failure`
    MajorFailure = "major-failure",
    /// `serious failure`
    SeriousFailure = "serious-failure",
    /// `minor failure`
    MinorFailure = "minor-failure",
    /// `Action`
    Action2 = "action-2",
    /// `Patient id`
    PatientId = "patient-id",
    /// `To (instant)`
    ToInstant = "to-instant",
    /// `From (instant)`
    FromInstant = "from-instant",
    /// `Audit log`
    AuditLog = "audit-log",
    /// `Import`
    Import = "import",
    /// `Export`
    Export = "export",
    /// `Authentication`
    Authentication = "authentication",
    /// `change type`
    ChangeType2 = "change-type-2",
    /// `Audit`
    Audit = "audit",
    ///  · current
    Current = "current",
    /// `Version timeline`
    VersionTimeline = "version-timeline",
    /// `Latest`
    Latest = "latest",
    /// `At time`
    AtTime = "at-time",
    /// `Time travel (interpreted as UTC)`
    TimeTravelInterpretedUtc = "time-travel-interpreted-utc",
    /// `No version of this composition existed at that time.`
    NoVersionCompositionExistedTime = "no-version-composition-existed-time",
    /// `versions unavailable`
    VersionsUnavailable = "versions-unavailable",
    /// `Reading the versions`
    ReadingTheVersions = "reading-the-versions",
    /// `Committing…`
    Committing = "committing",
    /// `Commit new version`
    CommitNewVersion = "commit-new-version",
    /// `Composition (canonical JSON)`
    CompositionCanonicalJson = "composition-canonical-json",
    /// `Switch to canonical JSON to edit.`
    SwitchCanonicalJsonEdit = "switch-canonical-json-edit",
    /// `Commits on top of the latest version.`
    CommitsTopLatestVersion = "commits-top-latest-version",
    /// `Close editor`
    CloseEditor = "close-editor",
    /// `Edit as new version`
    EditAsNewVersion = "edit-as-new-version",
    /// `The commit needs the latest version id as its precondition — try again in a moment.`
    CommitNeedsLatestVersionIdPrecondition = "commit-needs-latest-version-id-precondition",
    /// `Version history still loading`
    VersionHistoryStillLoading = "version-history-still-loading",
    /// `preceding version`
    PrecedingVersion = "preceding-version",
    /// `owner EHR`
    OwnerEhr = "owner-ehr",
    /// `versioned object`
    VersionedObject = "versioned-object",
    /// `Versioned object`
    VersionedObject2 = "versioned-object-2",
    /// `none (deleted version)`
    NoneDeletedVersion = "none-deleted-version",
    /// `Delete composition`
    DeleteComposition = "delete-composition",
    /// `Deletes the latest version`
    DeletesTheLatestVersion = "deletes-the-latest-version",
    /// `Deletes the latest version.`
    DeletesTheLatestVersion2 = "deletes-the-latest-version-2",
    /// `EHRs`
    Ehrs = "ehrs",
    /// `this composition`
    ThisComposition = "this-composition",
    /// The composition was logically deleted — its version history stays readable.
    CompositionWasLogicallyDeletedVersionHistory = "composition-was-logically-deleted-version-history",
    /// `Composition deleted`
    CompositionDeleted = "composition-deleted",
    /// `the new composition version`
    TheNewCompositionVersion = "the-new-composition-version",
    /// `Commit failed`
    CommitFailed = "commit-failed",
    /// `New version committed`
    NewVersionCommitted = "new-version-committed",
    /// `A new version was committed.`
    NewVersionWasCommitted = "new-version-was-committed",
    /// no current version is known — reload the page and retry
    NoCurrentVersionKnownReloadPage = "no-current-version-known-reload-page",
    /// `the composition body is empty`
    CompositionBodyEmpty = "composition-body-empty",
    /// `pick a date and time to travel to`
    PickDateTimeTravel = "pick-date-time-travel",
    /// `Commit a composition and the trend appears here from the next day onward.`
    CommitCompositionTrendAppearsHereFrom = "commit-composition-trend-appears-here-from",
    /// `No commit activity yet`
    NoCommitActivityYet = "no-commit-activity-yet",
    /// `1 query`
    N1Query = "n-1-query",
    /// `Go to stored queries`
    GoToStoredQueries = "go-to-stored-queries",
    /// `Stored queries`
    StoredQueries = "stored-queries",
    /// `Templates`
    Templates = "templates",
    /// `Compositions`
    Compositions = "compositions",
    /// `Commit activity`
    CommitActivity = "commit-activity",
    /// `Query namespaces`
    QueryNamespaces = "query-namespaces",
    /// `Dashboard`
    Dashboard = "dashboard",
    /// `Open party`
    OpenParty = "open-party",
    /// `Demographic tags`
    DemographicTags = "demographic-tags",
    /// `Tag pages`
    TagPages = "tag-pages",
    /// `The demographic tag list spans every party kind, and a tag names its target without...`
    DemographicTagListSpansEveryParty = "demographic-tag-list-spans-every-party",
    /// `Tag index (all kinds)`
    TagIndexAllKinds = "tag-index-all-kinds",
    /// `Creating…`
    Creating = "creating",
    /// `The document below is the smallest party this kind accepts — every attribute in it is...`
    DocumentBelowSmallestPartyKindAccepts = "document-below-smallest-party-kind-accepts",
    /// `Create failed`
    CreateFailed = "create-failed",
    /// `Party created`
    PartyCreated = "party-created",
    /// `identity details`
    IdentityDetails = "identity-details",
    /// `legal identity`
    LegalIdentity = "legal-identity",
    /// `Either id form works: a versioned_object_uid opens the latest version, and a full...`
    EitherIdFormWorksVersionedObject = "either-id-form-works-versioned-object",
    /// `versioned_object_uid or version_uid`
    VersionedObjectUidVersionUid = "versioned-object-uid-version-uid",
    /// `Party kinds`
    PartyKinds = "party-kinds",
    /// . The five kinds are:
    TheFiveKindsAre = "the-five-kinds-are",
    /// `The demographic API has no party kind called `
    DemographicApiHasNoPartyKind = "demographic-api-has-no-party-kind",
    /// `Unknown party kind`
    UnknownPartyKind = "unknown-party-kind",
    /// `Contribution versions`
    ContributionVersions = "contribution-versions",
    /// `Type`
    Type2 = "type-2",
    /// `Versions in this contribution`
    VersionsInThisContribution = "versions-in-this-contribution",
    /// `Versions`
    Versions = "versions",
    /// `Commit`
    Commit = "commit",
    /// `Demographics`
    Demographics = "demographics",
    /// `The CDR holds no demographic contribution with this id. A contribution committed...`
    CdrHoldsNoDemographicContributionId = "cdr-holds-no-demographic-contribution-id",
    /// `a contribution uid is required`
    ContributionUidRequired = "contribution-uid-required",
    /// `No version existed at that time.`
    NoVersionExistedTime = "no-version-existed-time",
    /// `Version at a point in time`
    VersionPointTime = "version-point-time",
    /// `Newest first. Open a version to read the document exactly as it stood at that commit.`
    NewestFirstOpenVersionReadDocument = "newest-first-open-version-read-document",
    /// `Every demographic object is created with a first version; if none is listed, the CDR...`
    EveryDemographicObjectCreatedFirstVersion = "every-demographic-object-created-first-version",
    /// `No versions`
    NoVersions = "no-versions",
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
    /// `Party deleted`
    PartyDeleted = "party-deleted",
    /// `Delete party`
    DeleteParty = "delete-party",
    /// `Saving…`
    Saving = "saving",
    /// `Save party`
    SaveParty = "save-party",
    /// `details (canonical JSON ITEM_STRUCTURE — leave blank to remove)`
    DetailsCanonicalJsonItemStructureLeave = "details-canonical-json-item-structure-leave",
    /// `identities (canonical JSON array of PARTY_IDENTITY — at least one)`
    IdentitiesCanonicalJsonArrayPartyIdentity = "identities-canonical-json-array-party-identity",
    /// `Edit party`
    EditParty = "edit-party",
    /// `relationships (inline)`
    RelationshipsInline = "relationships-inline",
    /// `This party's current version is deleted. Its earlier versions are still readable —...`
    PartySCurrentVersionDeletedEarlier = "party-s-current-version-deleted-earlier",
    /// `The CDR holds no party of this kind with this id. Check the id and the kind — the same...`
    CdrHoldsNoPartyKindId = "cdr-holds-no-party-kind-id",
    /// `Save failed`
    SaveFailed = "save-failed",
    /// `Party changed on the server`
    PartyChangedServer = "party-changed-server",
    /// `Party updated`
    PartyUpdated = "party-updated",
    /// `Tags`
    Tags = "tags",
    /// `Relationships`
    Relationships = "relationships",
    /// `History`
    History2 = "history-2",
    /// `Party`
    Party = "party",
    /// `Party views`
    PartyViews = "party-views",
    /// `the loaded party document is not a JSON object — reload this tab and retry`
    LoadedPartyDocumentNotJsonObject = "loaded-party-document-not-json-object",
    /// `identities must be a JSON array of PARTY_IDENTITY objects`
    IdentitiesMustJsonArrayPartyIdentity = "identities-must-json-array-party-identity",
    /// `the party body must be a JSON object`
    PartyBodyMustJsonObject = "party-body-must-json-object",
    /// `party kind`
    PartyKind = "party-kind",
    /// `the current version uid is required to update this party — reload this tab and retry`
    CurrentVersionUidRequiredUpdateParty = "current-version-uid-required-update-party",
    /// `the CDR created the party but returned no uid to open it by`
    CdrCreatedPartyButReturnedNo = "cdr-created-party-but-returned-no",
    /// `this party`
    ThisParty = "this-party",
    /// `Archetype`
    Archetype = "archetype",
    /// `Own uid`
    OwnUid = "own-uid",
    /// `Create a relationship`
    CreateARelationship = "create-a-relationship",
    /// `Creates a PARTY_RELATIONSHIP resource with this party as its source. That resource is...`
    CreatesPartyRelationshipResourcePartySource = "creates-party-relationship-resource-party-source",
    /// `Relate this party`
    RelateThisParty = "relate-this-party",
    /// `These come from the party document itself, which carries the relationships this party...`
    TheseComeFromPartyDocumentItself = "these-come-from-party-document-itself",
    /// `Relationships (as source)`
    RelationshipsAsSource = "relationships-as-source",
    /// `This party's current version is deleted, so it has no relationships to show and cannot...`
    PartySCurrentVersionDeletedSo = "party-s-current-version-deleted-so",
    /// `Relationship deleted`
    RelationshipDeleted = "relationship-deleted",
    /// `Delete relationship`
    DeleteRelationship = "delete-relationship",
    /// `Save relationship`
    SaveRelationship = "save-relationship",
    /// `Relationship type`
    RelationshipType = "relationship-type",
    /// `Commits a new version on top of the one loaded above (If-Match). The two ends and the...`
    CommitsNewVersionTopOneLoaded = "commits-new-version-top-one-loaded",
    /// `Edit relationship`
    EditRelationship = "edit-relationship",
    /// A relationship needs a type — openEHR stores it as the relationship's name.
    RelationshipNeedsTypeOpenehrStoresRelationship = "relationship-needs-type-openehr-stores-relationship",
    /// `time validity`
    TimeValidity = "time-validity",
    /// `No relationship with this id. Relationship routes are an extension of this CDR, so a...`
    NoRelationshipIdRelationshipRoutesExtension = "no-relationship-id-relationship-routes-extension",
    /// `This relationship's current version is deleted. Its earlier versions are still...`
    RelationshipSCurrentVersionDeletedEarlier = "relationship-s-current-version-deleted-earlier",
    /// `Relationship changed on the server`
    RelationshipChangedServer = "relationship-changed-server",
    /// `Relationship updated`
    RelationshipUpdated = "relationship-updated",
    /// `Relationship`
    Relationship = "relationship",
    /// `Relationship views`
    RelationshipViews = "relationship-views",
    /// `Kind`
    Kind = "kind",
    /// `Create relationship`
    CreateRelationship = "create-relationship",
    /// `details (canonical JSON ITEM_STRUCTURE — optional)`
    DetailsCanonicalJsonItemStructureOptional = "details-canonical-json-item-structure-optional",
    /// `Target party id`
    TargetPartyId = "target-party-id",
    /// `Source party id`
    SourcePartyId = "source-party-id",
    /// `Archetype node id`
    ArchetypeNodeId = "archetype-node-id",
    /// `Both ends name a party by its versioned_object_uid — a relationship refers to the...`
    BothEndsNamePartyVersionedObject = "both-ends-name-party-versioned-object",
    /// `the new relationship`
    TheNewRelationship = "the-new-relationship",
    /// `Relationship created`
    RelationshipCreated = "relationship-created",
    /// `A relationship is reached by its own id: the demographic API publishes no way to ask...`
    RelationshipReachedOwnIdDemographicApi = "relationship-reached-own-id-demographic-api",
    /// `PARTY_RELATIONSHIP id`
    PartyRelationshipId = "party-relationship-id",
    /// `Open a relationship`
    OpenARelationship = "open-a-relationship",
    /// `Party relationships`
    PartyRelationships = "party-relationships",
    /// `(an abstract party type has no per-kind route)`
    AbstractPartyTypeHasNoPer = "abstract-party-type-has-no-per",
    /// `the loaded relationship document is not a JSON object — reload this screen and retry`
    LoadedRelationshipDocumentNotJsonObject = "loaded-relationship-document-not-json-object",
    /// `a relationship type is required (PARTY_RELATIONSHIP invariant Type_validity)`
    RelationshipTypeRequiredPartyRelationshipInvariant = "relationship-type-required-party-relationship-invariant",
    /// `an archetype node id is required (LOCATABLE.archetype_node_id)`
    ArchetypeNodeIdRequiredLocatableArchetype = "archetype-node-id-required-locatable-archetype",
    /// `the CDR created the relationship but returned no uid to open it by`
    CdrCreatedRelationshipButReturnedNo = "cdr-created-relationship-but-returned-no",
    /// `a relationship id is required`
    RelationshipIdRequired = "relationship-id-required",
    /// `this relationship`
    ThisRelationship = "this-relationship",
    /// `Tags are free key/value markers a client sets on a party; add one below.`
    TagsFreeKeyValueMarkersClient = "tags-free-key-value-markers-client",
    /// `No tags on this party`
    NoTagsParty = "no-tags-party",
    /// `this party's tag`
    ThisPartySTag = "this-party-s-tag",
    /// `Tag delete failed`
    TagDeleteFailed = "tag-delete-failed",
    /// `Every tag carrying that key was removed.`
    EveryTagCarryingKeyWasRemoved = "every-tag-carrying-key-was-removed",
    /// `Tag deleted`
    TagDeleted = "tag-deleted",
    /// `this party's tags`
    ThisPartySTags = "this-party-s-tags",
    /// `Tag save failed`
    TagSaveFailed = "tag-save-failed",
    /// `The party's tag collection was replaced with the merged list.`
    PartySTagCollectionWasReplaced = "party-s-tag-collection-was-replaced",
    /// `Tag saved`
    TagSaved = "tag-saved",
    /// `a tag key is required`
    TagKeyRequired = "tag-key-required",
    /// `a tag key is required (ITEM_TAG invariant Inv_key_valid: the key is not empty)`
    TagKeyRequiredItemTagInvariant = "tag-key-required-item-tag-invariant",
    /// `Commit 1 change as one contribution`
    Commit1ChangeOneContribution = "commit-1-change-one-contribution",
    /// `Commit as one contribution`
    CommitAsOneContribution = "commit-as-one-contribution",
    /// `Open it in the Contributions tab →`
    OpenContributionsTab = "open-contributions-tab",
    /// `Nothing was committed and your staged changes are untouched. The CDR refused the...`
    NothingWasCommittedYourStagedChanges = "nothing-was-committed-your-staged-changes",
    /// `why these changes belong together`
    WhyTheseChangesBelongTogether = "why-these-changes-belong-together",
    /// `Description`
    Description = "description",
    /// `defaults to your viewer identity`
    DefaultsYourViewerIdentity = "defaults-your-viewer-identity",
    /// `Committer (optional)`
    CommitterOptional = "committer-optional",
    /// `Contribution audit`
    ContributionAudit = "contribution-audit",
    /// `Remove`
    Remove = "remove",
    /// `Preview`
    Preview = "preview",
    /// `Staged changes`
    StagedChanges = "staged-changes",
    /// `Change`
    Change = "change",
    /// `Stage this change`
    StageThisChange = "stage-this-change",
    /// `Paste the canonical JSON document, or pick a target above to load it.`
    PasteCanonicalJsonDocumentPickTarget = "paste-canonical-json-document-pick-target",
    /// `Document (canonical JSON)`
    DocumentCanonicalJson = "document-canonical-json",
    /// `not loaded yet`
    NotLoadedYet = "not-loaded-yet",
    /// `Preceding version: `
    PrecedingVersion2 = "preceding-version-2",
    /// — pick a composition —
    PickAComposition = "pick-a-composition",
    /// `Loading compositions…`
    LoadingCompositions = "loading-compositions",
    /// `Composition`
    Composition2 = "composition-2",
    /// — from the document —
    FromTheDocument = "from-the-document",
    /// `Loading templates…`
    LoadingTemplates = "loading-templates",
    /// `Template`
    Template = "template",
    /// `The document is the EHR's current status, loaded for you — edit it and it commits as a...`
    DocumentEhrSCurrentStatusLoaded = "document-ehr-s-current-status-loaded",
    /// `Add a change`
    AddAChange = "add-a-change",
    /// `Staged changes live in this browser tab only. The viewer stores nothing of its own, so...`
    StagedChangesLiveBrowserTabOnly = "staged-changes-live-browser-tab-only",
    /// `A CONTRIBUTION is openEHR's atomic change set: every staged change is committed...`
    ContributionOpenehrSAtomicChangeSet = "contribution-openehr-s-atomic-change-set",
    /// `Commit several changes as one contribution`
    CommitSeveralChangesOneContribution = "commit-several-changes-one-contribution",
    /// `Contribution committed`
    ContributionCommitted = "contribution-committed",
    /// `this contribution`
    ThisContribution = "this-contribution",
    /// `a committer is required on a CONTRIBUTION audit`
    CommitterRequiredContributionAudit = "committer-required-contribution-audit",
    /// `stage at least one change before committing`
    StageLeastOneChangeBeforeCommitting = "stage-least-one-change-before-committing",
    /// `a creation commits a NEW versioned object, so it carries no preceding version`
    CreationCommitsNewVersionedObjectSo = "creation-commits-new-versioned-object-so",
    /// no preceding version is known yet — pick the target and wait for it to load
    NoPrecedingVersionKnownYetPick = "no-preceding-version-known-yet-pick",
    /// the document must be a JSON object — a canonical openEHR document
    DocumentMustJsonObjectCanonicalOpenehr = "document-must-json-object-canonical-openehr",
    /// the document is empty — paste or load the body to commit
    DocumentEmptyPasteLoadBodyCommit = "document-empty-paste-load-body-commit",
    /// `new COMPOSITION`
    NewComposition = "new-composition",
    /// EHR status — modify
    EhrStatusModify = "ehr-status-modify",
    /// Composition — amend
    CompositionAmend = "composition-amend",
    /// Composition — create
    CompositionCreate = "composition-create",
    /// `The “to” date`
    TheToDate = "the-to-date",
    /// `The “from” date`
    TheFromDate = "the-from-date",
    /// `Started`
    Started = "started",
    /// `Name`
    Name = "name",
    /// `Composer`
    Composer = "composer",
    /// `Widen the date window, shorten the template or composer text, or clear the filters.`
    WidenDateWindowShortenTemplateComposer = "widen-date-window-shorten-template-composer",
    /// `No compositions match these filters`
    NoCompositionsMatchTheseFilters = "no-compositions-match-these-filters",
    /// `Commit one with the form above, or through the CDR's REST API.`
    CommitOneFormAboveThroughCdr = "commit-one-form-above-through-cdr",
    /// `No compositions in this EHR`
    NoCompositionsEhr = "no-compositions-ehr",
    /// `Paste the composition document in the format chosen above.`
    PasteCompositionDocumentFormatChosenAbove = "paste-composition-document-format-chosen-above",
    /// `template id (required for FLAT)`
    TemplateIdRequiredFlat = "template-id-required-flat",
    /// `Template id`
    TemplateId = "template-id",
    /// `Canonical XML`
    CanonicalXml = "canonical-xml",
    /// `Canonical JSON`
    CanonicalJson = "canonical-json",
    /// `Commit composition`
    CommitComposition = "commit-composition",
    /// `Template and composer match anywhere in the value; the dates bound the composition's...`
    TemplateComposerMatchAnywhereValueDates = "template-composer-match-anywhere-value-dates",
    /// `Clear`
    Clear = "clear",
    /// `part of the composer's name`
    PartComposerSName = "part-composer-s-name",
    /// `To (UTC date)`
    ToUtcDate = "to-utc-date",
    /// `From (UTC date)`
    FromUtcDate = "from-utc-date",
    /// `part of the template id`
    PartTemplateId = "part-template-id",
    /// `the composition`
    TheComposition = "the-composition",
    /// `Composition committed`
    CompositionCommitted = "composition-committed",
    /// `The composition was committed.`
    TheCompositionWasCommitted = "the-composition-was-committed",
    /// `a template id is required to commit a FLAT composition`
    TemplateIdRequiredCommitFlatComposition = "template-id-required-commit-flat-composition",
    /// `Enter a contribution uid to look it up.`
    EnterContributionUidLookUp = "enter-contribution-uid-look-up",
    /// `Look up`
    LookUp = "look-up",
    /// `contribution uid (UUID)`
    ContributionUidUuid = "contribution-uid-uuid",
    /// `Contribution uid`
    ContributionUid = "contribution-uid",
    ///  contribution(s) total
    ContributionSTotal = "contribution-s-total",
    /// `Contribution`
    Contribution2 = "contribution-2",
    /// `Contributions`
    Contributions = "contributions",
    /// `Next →`
    Next = "next",
    /// `← Previous`
    Previous = "previous",
    /// `Contribution activity`
    ContributionActivity = "contribution-activity",
    /// `Every write to this EHR is a contribution; the timeline fills as they are committed.`
    EveryWriteEhrContributionTimelineFills = "every-write-ehr-contribution-timeline-fills",
    /// `No contribution activity yet`
    NoContributionActivityYet = "no-contribution-activity-yet",
    /// `Create directory`
    CreateDirectory = "create-directory",
    /// `Will be committed`
    WillBeCommitted = "will-be-committed",
    /// `This EHR has no directory yet. Create the empty root folder, then add sub-folders and...`
    EhrHasNoDirectoryYetCreate = "ehr-has-no-directory-yet-create",
    /// `Your unsaved changes are kept. Load the server version or save anyway from the banner.`
    YourUnsavedChangesKeptLoadServer = "your-unsaved-changes-kept-load-server",
    /// `Directory changed on the server`
    DirectoryChangedServer = "directory-changed-server",
    /// `The directory was deleted.`
    TheDirectoryWasDeleted = "the-directory-was-deleted",
    /// `Directory deleted`
    DirectoryDeleted = "directory-deleted",
    /// `the EHR's directory`
    TheEhrSDirectory = "the-ehr-s-directory",
    /// `Directory updated`
    DirectoryUpdated = "directory-updated",
    /// `Restore failed`
    RestoreFailed = "restore-failed",
    /// `Directory restored`
    DirectoryRestored = "directory-restored",
    /// `Directory created`
    DirectoryCreated = "directory-created",
    /// `the directory disappeared while resolving the conflict`
    DirectoryDisappearedWhileResolvingConflict = "directory-disappeared-while-resolving-conflict",
    /// `a path is required`
    APathIsRequired = "a-path-is-required",
    /// `a date and time is required`
    DateTimeRequired = "date-time-required",
    /// `the current directory version uid is required to delete`
    CurrentDirectoryVersionUidRequiredDelete = "current-directory-version-uid-required-delete",
    /// `the current directory version uid is required to update`
    CurrentDirectoryVersionUidRequiredUpdate = "current-directory-version-uid-required-update",
    /// `the directory body is empty`
    DirectoryBodyEmpty = "directory-body-empty",
    /// `The directory was committed.`
    TheDirectoryWasCommitted = "the-directory-was-committed",
    /// `Enter a path to view only that sub-folder of the directory.`
    EnterPathViewOnlySubFolder = "enter-path-view-only-sub-folder",
    /// `Path (e.g. folder-a/folder-b)`
    PathEGFolderFolderB = "path-e-g-folder-folder-b",
    /// `Sub-folder by path`
    SubFolderByPath = "sub-folder-by-path",
    /// `Pick a date and time to view the directory as it stood then.`
    PickDateTimeViewDirectoryStood = "pick-date-time-view-directory-stood",
    /// `Directory at a point in time`
    DirectoryPointTime = "directory-point-time",
    /// `Restore this version`
    RestoreThisVersion = "restore-this-version",
    /// `Load older versions`
    LoadOlderVersions = "load-older-versions",
    /// `Select a version to preview it.`
    SelectVersionPreview = "select-version-preview",
    /// `Loading versions…`
    LoadingVersions = "loading-versions",
    /// `Version history`
    VersionHistory = "version-history",
    /// `Cancel`
    Cancel = "cancel",
    /// `Delete directory`
    DeleteDirectory = "delete-directory",
    /// `Permanently delete this EHR's directory? This creates a deletion version.`
    PermanentlyDeleteEhrSDirectoryCreates = "permanently-delete-ehr-s-directory-creates",
    /// `Path query`
    PathQuery = "path-query",
    /// `Directory`
    Directory = "directory",
    /// `No compositions in this EHR yet.`
    NoCompositionsEhrYet = "no-compositions-ehr-yet",
    /// `Add reference`
    AddReference = "add-reference",
    /// `id value`
    IdValue = "id-value",
    /// `Id value`
    IdValue2 = "id-value-2",
    /// `id type (e.g. HIER_OBJECT_ID)`
    IdTypeEGHierObject = "id-type-e-g-hier-object",
    /// `Id type`
    IdType = "id-type",
    /// `type (e.g. COMPOSITION)`
    TypeEGComposition = "type-e-g-composition",
    /// `Reference type`
    ReferenceType = "reference-type",
    /// `namespace (e.g. local)`
    NamespaceEGLocal = "namespace-e-g-local",
    /// `Namespace`
    Namespace = "namespace",
    /// `Or enter a reference manually`
    EnterReferenceManually = "enter-reference-manually",
    /// `This EHR's compositions`
    ThisEhrSCompositions = "this-ehr-s-compositions",
    /// `Close`
    Close = "close",
    /// `Add item reference`
    AddItemReference = "add-item-reference",
    /// `Remove item`
    RemoveItem = "remove-item",
    /// `Rename folder`
    RenameFolder = "rename-folder",
    /// `Add subfolder`
    AddSubfolder = "add-subfolder",
    /// `Delete folder`
    DeleteFolder = "delete-folder",
    /// `New folder`
    NewFolder = "new-folder",
    /// `Cancel rename`
    CancelRename = "cancel-rename",
    /// `Confirm rename`
    ConfirmRename = "confirm-rename",
    /// `Folder name`
    FolderName = "folder-name",
    /// `Toggle folder`
    ToggleFolder = "toggle-folder",
    /// `Discard changes`
    DiscardChanges = "discard-changes",
    /// `Save directory`
    SaveDirectory = "save-directory",
    /// `Discard my edits and load the server version`
    DiscardMyEditsLoadServerVersion = "discard-my-edits-load-server-version",
    /// `Save anyway (overwrite the server change)`
    SaveAnywayOverwriteServerChange = "save-anyway-overwrite-server-change",
    /// `This directory changed on the server since it was loaded. Your unsaved edits are still...`
    DirectoryChangedServerSinceWasLoaded = "directory-changed-server-since-was-loaded",
    /// `Apply to tree`
    ApplyToTree = "apply-to-tree",
    /// `Hide JSON`
    HideJson = "hide-json",
    /// `Advanced: edit as JSON`
    AdvancedEditAsJson = "advanced-edit-as-json",
    /// `Status history`
    StatusHistory = "status-history",
    /// `Status`
    Status = "status",
    /// `EHR views`
    EhrViews = "ehr-views",
    /// `Delete EHR`
    DeleteEhr = "delete-ehr",
    /// `EHR deleted`
    EhrDeleted = "ehr-deleted",
    /// The CDR holds no EHR with this id — check the id, or create it on the EHRs screen.
    CdrHoldsNoEhrIdCheck = "cdr-holds-no-ehr-id-check",
    /// `Save status`
    SaveStatus = "save-status",
    /// `other_details (canonical JSON ITEM_STRUCTURE — leave blank to remove)`
    OtherDetailsCanonicalJsonItemStructure = "other-details-canonical-json-item-structure",
    /// `Allow new content to be committed to this EHR.`
    AllowNewContentCommittedEhr = "allow-new-content-committed-ehr",
    /// `Include this EHR in population queries (AQL).`
    IncludeEhrPopulationQueriesAql = "include-ehr-population-queries-aql",
    /// `Commits a new EHR_STATUS version on top of the one loaded above (If-Match), so a...`
    CommitsNewEhrStatusVersionTop = "commits-new-ehr-status-version-top",
    /// `Edit status`
    EditStatus = "edit-status",
    /// `the loaded EHR_STATUS document is not a JSON object — reload this tab and retry`
    LoadedEhrStatusDocumentNotJson = "loaded-ehr-status-document-not-json",
    /// `Versioned EHR_STATUS`
    VersionedEhrStatus = "versioned-ehr-status",
    /// `No EHR_STATUS version existed at that time.`
    NoEhrStatusVersionExistedTime = "no-ehr-status-version-existed-time",
    /// `EHR_STATUS at a point in time`
    EhrStatusPointTime = "ehr-status-point-time",
    /// `Newest first. Open a version to read the EHR_STATUS document exactly as it stood at...`
    NewestFirstOpenVersionReadEhr = "newest-first-open-version-read-ehr",
    /// `Every EHR is created with a first EHR_STATUS version; if none is listed, the CDR did...`
    EveryEhrCreatedFirstEhrStatus = "every-ehr-created-first-ehr-status",
    /// `No EHR_STATUS versions`
    NoEhrStatusVersions = "no-ehr-status-versions",
    /// `This EHR is not queryable — AQL over it returns nothing. Tick “is_queryable” below and...`
    EhrNotQueryableAqlOverReturns = "ehr-not-queryable-aql-over-returns",
    /// self — no external subject reference
    SelfNoExternalSubjectReference = "self-no-external-subject-reference",
    /// `EHR status changed on the server`
    EhrStatusChangedServer = "ehr-status-changed-server",
    /// `EHR status updated`
    EhrStatusUpdated = "ehr-status-updated",
    /// `A new EHR_STATUS version was committed.`
    NewEhrStatusVersionWasCommitted = "new-ehr-status-version-was-committed",
    /// `a version uid is required to read a past EHR_STATUS version`
    VersionUidRequiredReadPastEhr = "version-uid-required-read-past-ehr",
    /// `the EHR's status`
    TheEhrSStatus = "the-ehr-s-status",
    /// `Tagged objects`
    TaggedObjects = "tagged-objects",
    /// `tagged object`
    TaggedObject = "tagged-object",
    /// `tagged objects`
    TaggedObjects2 = "tagged-objects-2",
    /// `Tagged object pages`
    TaggedObjectPages = "tagged-object-pages",
    /// `Every tag on every object in this EHR, grouped by the object it is on. A tag names its...`
    EveryTagEveryObjectEhrGrouped = "every-tag-every-object-ehr-grouped",
    /// `Tags in this EHR`
    TagsInThisEhr = "tags-in-this-ehr",
    /// `The tag collection was replaced with the merged list.`
    TagCollectionWasReplacedMergedList = "tag-collection-was-replaced-merged-list",
    /// `No tags on this EHR status`
    NoTagsEhrStatus = "no-tags-ehr-status",
    /// `this EHR status's tags`
    EhrStatusSTags = "ehr-status-s-tags",
    /// `No tags on this collection`
    NoTagsCollection = "no-tags-collection",
    /// `this composition's tags`
    ThisCompositionSTags = "this-composition-s-tags",
    /// `a tag target id is required`
    TagTargetIdRequired = "tag-target-id-required",
    /// `Pages`
    Pages = "pages",
    /// `Created`
    Created = "created",
    /// `EHR ID`
    EhrId = "ehr-id",
    /// `No EHR found for that subject.`
    NoEhrFoundSubject = "no-ehr-found-subject",
    /// `Searching…`
    Searching = "searching",
    /// `Find by subject`
    FindBySubject = "find-by-subject",
    /// `Subject namespace`
    SubjectNamespace = "subject-namespace",
    /// `external subject id`
    ExternalSubjectId = "external-subject-id",
    /// `Subject id`
    SubjectId = "subject-id",
    /// `Find`
    Find = "find",
    /// `ehr_id (UUID)`
    EhrIdUuid = "ehr-id-uuid",
    /// `EHR id`
    EhrId2 = "ehr-id-2",
    /// `Find an EHR`
    FindAnEhr = "find-an-ehr",
    /// `Supply an EHR id to create that exact EHR — it must be a UUID, and an id already in...`
    SupplyEhrIdCreateExactEhr = "supply-ehr-id-create-exact-ehr",
    /// `Leave the subject fields blank for an anonymous EHR (the CDR mints the default...`
    LeaveSubjectFieldsBlankAnonymousEhr = "leave-subject-fields-blank-anonymous-ehr",
    /// `Create EHR`
    CreateEhr = "create-ehr",
    /// `Subject namespace (optional)`
    SubjectNamespaceOptional = "subject-namespace-optional",
    /// `Subject id (optional)`
    SubjectIdOptional = "subject-id-optional",
    /// `UUID, or blank`
    EhrIdUuidBlankLetsCdr = "ehr-id-uuid-blank-lets-cdr",
    /// `EHR id (optional)`
    EhrIdOptional = "ehr-id-optional",
    /// `Provide both a subject id and a namespace, or leave both empty for an anonymous EHR.`
    ProvideBothSubjectIdNamespaceLeave = "provide-both-subject-id-namespace-leave",
    /// `the new EHR`
    TheNewEhr = "the-new-ehr",
    /// `EHR created`
    EhrCreated = "ehr-created",
    /// `both a subject id and a subject namespace are required`
    BothSubjectIdSubjectNamespaceRequired = "both-subject-id-subject-namespace-required",
    /// `provide both a subject id and a subject namespace, or leave both empty`
    ProvideBothSubjectIdSubjectNamespace = "provide-both-subject-id-subject-namespace",
    /// `EHR response carried no ehr_id`
    EhrResponseCarriedNoEhrId = "ehr-response-carried-no-ehr-id",
    /// `EHR Status`
    EhrStatus = "ehr-status",
    /// `Sign in with OIDC`
    SignInWithOidc = "sign-in-with-oidc",
    /// `Wrong username or password.`
    WrongUsernameOrPassword = "wrong-username-or-password",
    /// `Your session ended. Sign in again to continue.`
    YourSessionEndedSignAgainContinue = "your-session-ended-sign-again-continue",
    /// `Loading the sign-in methods…`
    LoadingSignMethods = "loading-sign-methods",
    /// `server-side ([management] — each endpoint takes its own access level).`
    ServerSideManagementEachEndpointTakes = "server-side-management-each-endpoint-takes",
    ///  is not available: the CDR does not serve this management endpoint. Enable it
    NotAvailableCdrDoesNotServe = "not-available-cdr-does-not-serve",
    /// `Reset filter`
    ResetFilter = "reset-filter",
    /// `Restore the CDR's boot log filter? Any filter applied since startup is discarded.`
    RestoreCdrSBootLogFilter = "restore-cdr-s-boot-log-filter",
    /// `Reset log filter`
    ResetLogFilter = "reset-log-filter",
    /// `Apply filter`
    ApplyFilter = "apply-filter",
    /// `Apply log filter`
    ApplyLogFilter = "apply-log-filter",
    /// `Takes effect immediately, for this CDR instance only, until it restarts.`
    TakesEffectImmediatelyCdrInstanceOnly = "takes-effect-immediately-cdr-instance-only",
    ///  Reset to boot filter
    ResetToBootFilter = "reset-to-boot-filter",
    ///  Apply filter
    ApplyFilter2 = "apply-filter-2",
    /// `New filter directives`
    NewFilterDirectives = "new-filter-directives",
    /// `Log level`
    LogLevel = "log-level",
    /// `Log control`
    LogControl = "log-control",
    /// `Runtime configuration`
    RuntimeConfiguration = "runtime-configuration",
    ///  Open runtime configuration
    OpenRuntimeConfiguration = "open-runtime-configuration",
    /// `configured for.`
    ConfiguredFor = "configured-for",
    /// in exactly one place — the System screen, over the API base URL it is always
    ExactlyOnePlaceSystemScreenOver = "exactly-one-place-system-screen-over",
    /// `same snapshot on its management surface and its admin API, so the viewer reads it `
    SameSnapshotManagementSurfaceAdminApi = "same-snapshot-management-surface-admin-api",
    /// `The CDR's effective configuration, with every secret redacted. The CDR serves the `
    CdrSEffectiveConfigurationEverySecret = "cdr-s-effective-configuration-every-secret",
    /// `Metric samples`
    MetricSamples = "metric-samples",
    /// `Labels`
    Labels = "labels",
    /// `Inspect`
    Inspect = "inspect",
    /// `Metric`
    Metric = "metric",
    /// `Metrics`
    Metrics = "metrics",
    /// `Build & spec provenance`
    BuildSpecProvenance = "build-spec-provenance",
    /// `Build provenance`
    BuildProvenance = "build-provenance",
    /// This is the other question — the CDR's readiness probe, one row per dependency it checks.
    OtherQuestionCdrSReadinessProbe = "other-question-cdr-s-readiness-probe",
    /// `The topbar pill reports the API itself (status document: reachable, and at which...`
    TopbarPillReportsApiItselfStatus = "topbar-pill-reports-api-itself-status",
    /// `, which this deployment does not offer to this session.`
    WhichDeploymentDoesNotOfferSession = "which-deployment-does-not-offer-session",
    /// `Statuses only: the detail behind each one is served by the CDR's `
    StatusesOnlyDetailBehindEachOne = "statuses-only-detail-behind-each-one",
    /// `Health indicators`
    HealthIndicators = "health-indicators",
    /// `Indicator`
    Indicator = "indicator",
    /// `Detail`
    Detail = "detail",
    /// `Dependency health`
    DependencyHealth = "dependency-health",
    /// `Operations`
    Operations = "operations",
    /// `the log filter`
    TheLogFilter = "the-log-filter",
    /// `Log filter not reset`
    LogFilterNotReset = "log-filter-not-reset",
    /// `Log filter reset`
    LogFilterReset = "log-filter-reset",
    /// `Log filter not applied`
    LogFilterNotApplied = "log-filter-not-applied",
    /// `Log filter applied`
    LogFilterApplied = "log-filter-applied",
    /// `), chosen when you save it. The viewer stores no grouping of its own, so every openEHR...`
    ChosenWhenYouSaveViewerStores = "chosen-when-you-save-viewer-stores",
    /// `A query's group is the namespace of its qualified name (`
    QuerySGroupNamespaceQualifiedName = "query-s-group-namespace-qualified-name",
    /// `Namespaces`
    Namespaces = "namespaces",
    /// `Loading namespaces…`
    LoadingNamespaces = "loading-namespaces",
    /// `Run as ad-hoc AQL`
    RunAdHocAql = "run-ad-hoc-aql",
    /// `Loading query…`
    LoadingQuery = "loading-query",
    /// `Delete from CDR`
    DeleteFromCdr = "delete-from-cdr",
    /// `Open in builder`
    OpenInBuilder = "open-in-builder",
    /// `Open in editor`
    OpenInEditor = "open-in-editor",
    /// `Run`
    Run = "run",
    /// `Saved`
    Saved = "saved",
    /// `stored query`
    StoredQuery = "stored-query",
    /// `stored queries`
    StoredQueries2 = "stored-queries-2",
    /// `Stored query pages`
    StoredQueryPages = "stored-query-pages",
    /// `Delete stored query from the CDR`
    DeleteStoredQueryFromCdr = "delete-stored-query-from-cdr",
    /// `Raw AQL`
    RawAql = "raw-aql",
    /// `New query`
    NewQuery = "new-query",
    /// `Queries`
    Queries = "queries",
    /// `Stored query deleted`
    StoredQueryDeleted = "stored-query-deleted",
    /// `Results`
    Results = "results",
    /// `Save`
    Save = "save",
    /// `Parameters (JSON object)`
    ParametersJsonObject = "parameters-json-object",
    /// `AQL is valid.`
    AqlIsValid = "aql-is-valid",
    /// `Validating…`
    Validating = "validating",
    /// `Validate`
    Validate = "validate",
    /// `For example: SELECT c FROM EHR e CONTAINS COMPOSITION c`
    ExampleSelectCFromEhrE = "example-select-c-from-ehr-e",
    /// `Run with parameters`
    RunWithParameters = "run-with-parameters",
    /// `immutable, so the next one is proposed.`
    ImmutableSoNextOneProposed = "immutable-so-next-one-proposed",
    /// . Saving stores the version in the field below — that version is
    SavingStoresVersionFieldBelowVersion = "saving-stores-version-field-below-version",
    ///  at version
    AtVersion = "at-version",
    /// `Loaded stored query `
    LoadedStoredQuery = "loaded-stored-query",
    /// `Loading stored query…`
    LoadingStoredQuery = "loading-stored-query",
    /// `the stored query`
    TheStoredQuery = "the-stored-query",
    /// `Query saved`
    QuerySaved = "query-saved",
    /// `Next `
    Next2 = "next-2",
    ///  Previous
    Previous2 = "previous-2",
    /// `Table`
    Table = "table",
    /// `Chart`
    Chart = "chart",
    /// `Results view`
    ResultsView = "results-view",
    /// `Query results`
    QueryResults = "query-results",
    /// `matching rows`
    MatchingRows = "matching-rows",
    /// `Exports the query's own LIMIT window, or the server default.`
    ExportsQuerySOwnLimitWindow = "exports-query-s-own-limit-window",
    /// `Export JSON`
    ExportJson = "export-json",
    /// `Export CSV`
    ExportCsv = "export-csv",
    /// `Saves as `
    SavesAs = "saves-as",
    /// `Version (optional)`
    VersionOptional = "version-optional",
    /// `Query name`
    QueryName = "query-name",
    /// `Namespace (optional)`
    NamespaceOptional = "namespace-optional",
    /// `the server assigns the version, replacing the query stored at it`
    ServerAssignsVersionReplacingQueryStored = "server-assigns-version-replacing-query-stored",
    /// `Open in raw editor `
    OpenInRawEditor = "open-in-raw-editor",
    /// `AQL preview`
    AqlPreview = "aql-preview",
    /// `Limit`
    Limit = "limit",
    /// `+ add`
    Add = "add",
    /// `Order by`
    OrderBy = "order-by",
    /// `Columns`
    Columns = "columns",
    /// `Add projection columns with "+ column" in the path catalog.`
    AddProjectionColumnsColumnPathCatalog = "add-projection-columns-column-path-catalog",
    /// `EHRs (cohort)`
    EhrsCohort = "ehrs-cohort",
    /// `Count`
    Count = "count",
    /// `Data values`
    DataValues = "data-values",
    /// `No ordinal steps in the template for this node.`
    NoOrdinalStepsTemplateNode = "no-ordinal-steps-template-node",
    /// `value set (optional)`
    ValueSetOptional = "value-set-optional",
    ///  in this terminology.
    InThisTerminology = "in-this-terminology",
    /// `No value set `
    NoValueSet = "no-value-set",
    /// `look up`
    LookUp2 = "look-up-2",
    /// `No coded options in the template for this node.`
    NoCodedOptionsTemplateNode = "no-coded-options-template-node",
    /// `max numerator`
    MaxNumerator = "max-numerator",
    /// `min numerator`
    MinNumerator = "min-numerator",
    /// `max count`
    MaxCount = "max-count",
    /// `min count`
    MinCount = "min-count",
    /// `(any units)`
    AnyUnits = "any-units",
    /// `max magnitude`
    MaxMagnitude = "max-magnitude",
    /// `min magnitude`
    MinMagnitude = "min-magnitude",
    /// Presence check only for this type — matches when the node exists.
    PresenceCheckOnlyTypeMatchesWhen = "presence-check-only-type-matches-when",
    /// `add here`
    AddHere = "add-here",
    /// `+ group`
    Group2 = "group-2",
    /// `Remove this group`
    RemoveThisGroup = "remove-this-group",
    /// `Clear all conditions`
    ClearAllConditions = "clear-all-conditions",
    /// `Remove this condition`
    RemoveThisCondition = "remove-this-condition",
    /// `+ column`
    Column = "column",
    /// `+ condition`
    Condition = "condition",
    /// `Pick a template to browse its paths.`
    PickTemplateBrowsePaths = "pick-template-browse-paths",
    /// — any template —
    AnyTemplate = "any-template",
    ///  — the builder was left empty rather than loading a query it would change.
    BuilderWasLeftEmptyRatherThan = "builder-was-left-empty-rather-than",
    /// `Open it in the raw AQL editor instead`
    OpenRawAqlEditorInstead = "open-raw-aql-editor-instead",
    /// . Saving stores the version in the field below — that version is immutable, so the...
    SavingStoresVersionFieldBelowVersion2 = "saving-stores-version-field-below-version-2",
    /// `That link does not name a stored-query version, so the builder started empty.`
    LinkDoesNotNameStoredQuery = "link-does-not-name-stored-query",
    /// `Criteria`
    Criteria = "criteria",
    /// `Query builder`
    QueryBuilder = "query-builder",
    /// `This query sets its own row window (AQL LIMIT/TOP), so the run is not paged.`
    QuerySetsOwnRowWindowAql = "query-sets-own-row-window-aql",
    /// `). A field left blank is not sent at all.`
    FieldLeftBlankNotSentAll = "field-left-blank-not-sent-all",
    ///  as a boolean; anything else is sent as text. Quote a value to force text (
    BooleanAnythingElseSentTextQuote = "boolean-anything-else-sent-text-quote",
    ///  as a number,
    AsANumber = "as-a-number",
    /// `A value that reads as JSON is sent as that type — `
    ValueReadsJsonSentType = "value-reads-json-sent-type",
    /// `This query declares no parameters, so it runs as stored.`
    QueryDeclaresNoParametersSoRuns = "query-declares-no-parameters-so-runs",
    /// `Reading parameters…`
    ReadingParameters = "reading-parameters",
    /// `Parameters`
    Parameters = "parameters",
    /// `Sends `
    Sends = "sends",
    ///  — exactly that version.
    ExactlyThatVersion = "exactly-that-version",
    ///  — a partial pattern, so the CDR runs the latest version matching it.
    PartialPatternSoCdrRunsLatest = "partial-pattern-so-cdr-runs-latest",
    ///  — no version, so the CDR runs the latest one.
    NoVersionSoCdrRunsLatest = "no-version-so-cdr-runs-latest",
    /// `Resolution`
    Resolution = "resolution",
    ///  as stored at version
    AsStoredAtVersion = "as-stored-at-version",
    /// `That link does not name a stored-query version.`
    LinkDoesNotNameStoredQuery2 = "link-does-not-name-stored-query-2",
    /// `Definition`
    Definition = "definition",
    ///  and choose Run.
    AndChooseRun = "and-choose-run",
    /// `No stored query to run. Pick one from `
    NoStoredQueryRunPickOne = "no-stored-query-run-pick-one",
    /// `Run stored query`
    RunStoredQuery = "run-stored-query",
    /// `Main`
    Main = "main",
    /// `Access scopes`
    AccessScopes = "access-scopes",
    /// `Scopes on this session`
    ScopesOnThisSession = "scopes-on-this-session",
    /// `Signed in with`
    SignedInWith = "signed-in-with",
    /// `Identity`
    Identity = "identity",
    /// No scopes — Basic authentication grants full viewer access.
    NoScopesBasicAuthenticationGrantsFull = "no-scopes-basic-authentication-grants-full",
    /// `Signed in to this viewer`
    SignedViewer = "signed-viewer",
    /// `View scopes`
    ViewScopes = "view-scopes",
    /// `Sandbox deployment`
    SandboxDeployment = "sandbox-deployment",
    /// `Sandbox deployment: `
    SandboxDeployment2 = "sandbox-deployment-2",
    /// `CDR DOWN`
    CdrDown = "cdr-down",
    /// `Session ended`
    SessionEnded = "session-ended",
    /// `CDR DEGRADED`
    CdrDegraded = "cdr-degraded",
    /// `Checking the CDR`
    CheckingTheCdr = "checking-the-cdr",
    /// `Platform`
    Platform = "platform",
    /// `Clinical data`
    ClinicalData = "clinical-data",
    /// `System`
    System = "system",
    /// `Subscriptions`
    Subscriptions = "subscriptions",
    /// `Terminology`
    Terminology = "terminology",
    /// `Delete subscription`
    DeleteSubscription = "delete-subscription",
    /// `Edit`
    Edit = "edit",
    /// `State`
    State = "state",
    /// `Subscription`
    Subscription = "subscription",
    /// `event subscriptions`
    EventSubscriptions = "event-subscriptions",
    /// `Enabled`
    Enabled = "enabled",
    /// `blank = any`
    BlankAny = "blank-any",
    /// `Saving replaces every predicate: one cleared here becomes “any” on the CDR.`
    SavingReplacesEveryPredicateOneCleared = "saving-replaces-every-predicate-one-cleared",
    /// `Save subscription`
    SaveSubscription = "save-subscription",
    /// `The name is the CDR's queue key and cannot be changed.`
    NameCdrSQueueKeyCannot = "name-cdr-s-queue-key-cannot",
    /// `Subscription id: `
    SubscriptionId = "subscription-id",
    /// `Create subscription`
    CreateSubscription = "create-subscription",
    /// `Create a subscription`
    CreateASubscription = "create-a-subscription",
    /// `Subscription deleted`
    SubscriptionDeleted = "subscription-deleted",
    /// `Subscription updated`
    SubscriptionUpdated = "subscription-updated",
    /// `Update failed`
    UpdateFailed = "update-failed",
    /// `Subscription created`
    SubscriptionCreated = "subscription-created",
    ///  Open audit browser
    OpenAuditBrowser = "open-audit-browser",
    /// `filterable by time window, patient, principal, outcome and action.`
    FilterableTimeWindowPatientPrincipalOutcome = "filterable-time-window-patient-principal-outcome",
    /// `Who accessed what, with what outcome: the ATNA security audit trail, `
    WhoAccessedWhatWhatOutcomeAtna = "who-accessed-what-what-outcome-atna",
    /// `Show`
    Show = "show",
    /// `API family`
    ApiFamily = "api-family",
    /// `Served OpenAPI`
    ServedOpenapi = "served-openapi",
    /// `This CDR serves no document for that API family — pick another, or the complete surface.`
    CdrServesNoDocumentApiFamily = "cdr-serves-no-document-api-family",
    /// `Scopes`
    Scopes = "scopes",
    /// `PKCE methods`
    PkceMethods = "pkce-methods",
    /// `Grant types`
    GrantTypes = "grant-types",
    /// `Introspection`
    Introspection = "introspection",
    /// `Token`
    Token = "token",
    /// `Authorization`
    Authorization = "authorization",
    /// `SMART configuration returned, but it is not valid JSON.`
    SmartConfigurationReturnedButNotValid = "smart-configuration-returned-but-not-valid",
    /// `SMART is not enabled on this CDR: it serves no service-discovery document.`
    SmartNotEnabledCdrServesNo = "smart-not-enabled-cdr-serves-no",
    /// `Runtime configuration (redacted)`
    RuntimeConfigurationRedacted = "runtime-configuration-redacted",
    /// The CDR no longer accepts this session — sign in again to read the configuration.
    CdrNoLongerAcceptsSessionSign = "cdr-no-longer-accepts-session-sign",
    /// This session lacks the ADMIN role — the configuration view needs an admin sign-in.
    SessionLacksAdminRoleConfigurationView = "session-lacks-admin-role-configuration-view",
    /// `The CDR's admin API is disabled — enable [admin] to expose the configuration view.`
    CdrSAdminApiDisabledEnable = "cdr-s-admin-api-disabled-enable",
    /// `Repository usage`
    RepositoryUsage = "repository-usage",
    /// `Compositions per template`
    CompositionsPerTemplate = "compositions-per-template",
    /// `The product and openEHR REST versions are in the Status card — the manifest and the...`
    ProductOpenehrRestVersionsStatusCard = "product-openehr-rest-versions-status-card",
    /// `This CDR advertises no API groups in its manifest.`
    CdrAdvertisesNoApiGroupsManifest = "cdr-advertises-no-api-groups-manifest",
    /// `Mounted API groups`
    MountedApiGroups = "mounted-api-groups",
    /// `conformance profile`
    ConformanceProfile = "conformance-profile",
    /// `Conformance manifest`
    ConformanceManifest = "conformance-manifest",
    /// `Manufacturer`
    Manufacturer = "manufacturer",
    /// `postal address`
    PostalAddress = "postal-address",
    /// `Complete surface`
    CompleteSurface = "complete-surface",
    /// `example composition`
    ExampleComposition = "example-composition",
    /// `OperationalTemplateV2 JSON`
    Operationaltemplatev2Json = "operationaltemplatev2-json",
    /// `ADL2 source`
    Adl2Source = "adl2-source",
    /// `ADL2 templates`
    Adl2Templates = "adl2-templates",
    /// `Node inspector`
    NodeInspector = "node-inspector",
    /// `path catalog`
    PathCatalog2 = "path-catalog-2",
    /// `Path catalog (WT tree)`
    PathCatalogWtTree = "path-catalog-wt-tree",
    /// `Show version`
    ShowVersion = "show-version",
    /// `as stored`
    AsStored = "as-stored",
    /// `Release version (1.1.0, or a 1 / 1.0 prefix)`
    ReleaseVersion1101 = "release-version-1-1-0-1",
    /// `As stored`
    AsStored2 = "as-stored-2",
    /// `The listing holds no other version of this template family.`
    ListingHoldsNoOtherVersionTemplate = "listing-holds-no-other-version-template",
    /// `ADL2 template views`
    Adl2TemplateViews = "adl2-template-views",
    /// `node id`
    NodeId = "node-id",
    /// `Select a node to inspect its path and metadata.`
    SelectNodeInspectPathMetadata = "select-node-inspect-path-metadata",
    /// `back to templates`
    BackToTemplates = "back-to-templates",
    /// `Languages`
    Languages = "languages",
    /// `Default language`
    DefaultLanguage = "default-language",
    /// `Concept`
    Concept = "concept",
    /// `Delete template`
    DeleteTemplate = "delete-template",
    /// `Template deleted`
    TemplateDeleted = "template-deleted",
    /// `Template views`
    TemplateViews = "template-views",
    /// `Archetype ID`
    ArchetypeId = "archetype-id",
    /// `Upload your first ADL2 operational template with the Upload ADL2 button above.`
    UploadYourFirstAdl2OperationalTemplate = "upload-your-first-adl2-operational-template",
    /// `Upload your first operational template (OPT/XML) with the Upload OPT button above.`
    UploadYourFirstOperationalTemplateOpt = "upload-your-first-operational-template-opt",
    /// `ADL 2 templates`
    Adl2Templates2 = "adl-2-templates-2",
    /// `ADL 1.4 templates`
    Adl14Templates = "adl-1-4-templates",
    /// `Template pages`
    TemplatePages = "template-pages",
    /// `Template ID`
    TemplateId2 = "template-id-2",
    /// `Upload template`
    UploadTemplate = "upload-template",
    /// `Template source`
    TemplateSource = "template-source",
    /// `Template families`
    TemplateFamilies = "template-families",
    /// `the ADL2 operational template`
    TheAdl2OperationalTemplate = "the-adl2-operational-template",
    /// `Upload failed`
    UploadFailed = "upload-failed",
    /// `ADL2 template uploaded`
    Adl2TemplateUploaded = "adl2-template-uploaded",
    /// `the operational template`
    TheOperationalTemplate = "the-operational-template",
    /// `The operational template was accepted by the CDR.`
    OperationalTemplateWasAcceptedCdr = "operational-template-was-accepted-cdr",
    /// `Template uploaded`
    TemplateUploaded = "template-uploaded",
    /// `filter by id or concept…`
    FilterIdConcept = "filter-id-concept",
    /// `no ADL2 artefact id to delete`
    NoAdl2ArtefactIdDelete = "no-adl2-artefact-id-delete",
    /// `there is no ADL2 source to upload`
    ThereNoAdl2SourceUpload = "there-no-adl2-source-upload",
    /// `the selected file was empty`
    SelectedFileWasEmpty = "selected-file-was-empty",
    /// `Pick a terminology on the left first.`
    PickTerminologyLeftFirst = "pick-terminology-left-first",
    /// `The CDR serves no terminology called `
    CdrServesNoTerminologyCalled = "cdr-serves-no-terminology-called",
    /// `Origin`
    Origin = "origin",
    /// `Relation`
    Relation = "relation",
    /// `Targets`
    Targets = "targets",
    /// `Concept relations`
    ConceptRelations = "concept-relations",
    /// `Terms`
    Terms = "terms",
    /// `Term`
    Term = "term",
    /// `Language`
    Language = "language",
    /// `Preferred`
    Preferred = "preferred",
    /// `Terminology version `
    TerminologyVersion = "terminology-version",
    /// `Asking the CDR…`
    AskingTheCdr = "asking-the-cdr",
    /// `Candidate code`
    CandidateCode = "candidate-code",
    /// `Expand`
    Expand2 = "expand-2",
    /// `Value set id`
    ValueSetId = "value-set-id",
    /// `Expand a value set`
    ExpandAValueSet = "expand-a-value-set",
    ///  is not a member of
    NotMember = "not-member",
    ///  is a member of
    IsAMemberOf = "is-a-member-of",
    ///  in
    In = "in",
    /// `Unknown value set `
    UnknownValueSet = "unknown-value-set",
    /// `Both a value set id and a candidate code are needed.`
    BothValueSetIdCandidateCode = "both-value-set-id-candidate-code",
    /// `Type a value set id to expand.`
    TypeValueSetIdExpand = "type-value-set-id-expand",
    /// `Testing…`
    Testing = "testing",
    /// `Subsumption is strict: a code never subsumes itself.`
    SubsumptionStrictCodeNeverSubsumesItself = "subsumption-strict-code-never-subsumes-itself",
    /// `Test`
    Test = "test",
    /// `Reference code`
    ReferenceCode = "reference-code",
    /// `Test subsumption`
    TestSubsumption = "test-subsumption",
    ///  does not subsume
    DoesNotSubsume = "does-not-subsume",
    ///  subsumes
    Subsumes = "subsumes",
    /// `Both a reference code and a candidate code are needed.`
    BothReferenceCodeCandidateCodeNeeded = "both-reference-code-candidate-code-needed",
    /// `Looking up…`
    LookingUp = "looking-up",
    /// `Define`
    Define = "define",
    /// `Effective date (optional)`
    EffectiveDateOptional = "effective-date-optional",
    /// `Term code`
    TermCode = "term-code",
    /// `Define a term`
    DefineATerm = "define-a-term",
    /// `Unknown code `
    UnknownCode = "unknown-code",
    /// `Type a term code to define.`
    TypeTermCodeDefine = "type-term-code-define",
    /// `Attributes`
    Attributes = "attributes",
    /// `Available versions`
    AvailableVersions = "available-versions",
    /// `Publisher`
    Publisher = "publisher",
    /// `Pick a terminology on the left to describe it.`
    PickTerminologyLeftDescribe = "pick-terminology-left-describe",
    /// `Descriptor`
    Descriptor = "descriptor",
    /// `Terminologies`
    Terminologies = "terminologies",
    /// no stored query to run — open one from the stored-query list
    NoStoredQueryRunOpenOne = "no-stored-query-run-open-one",
    /// `the stored query needs a name`
    StoredQueryNeedsName = "stored-query-needs-name",
    /// `Exact version`
    ExactVersion = "exact-version",
    /// `Version prefix (latest match)`
    VersionPrefixLatestMatch = "version-prefix-latest-match",
    /// `Latest version (no version sent)`
    LatestVersionNoVersionSent = "latest-version-no-version-sent",
    /// `Exact match only.`
    ExactMatchOnly = "exact-match-only",
    /// `Wildcard pattern: * matches within one namespace segment, ** across segments.`
    WildcardPatternMatchesWithinOneNamespace = "wildcard-pattern-matches-within-one-namespace",
    /// All available templates or queries — including ad-hoc AQL.
    AllAvailableTemplatesQueriesIncludingAd = "all-available-templates-queries-including-ad",
    /// `Compositions of the matching template`
    CompositionsMatchingTemplate = "compositions-matching-template",
    /// `Operational templates`
    OperationalTemplates = "operational-templates",
    /// `A backend client acting without a user context: across all data.`
    BackendClientActingWithoutUserContext = "backend-client-acting-without-user-context",
    /// `Subject to the authenticated user's security profile — not limited to the patient in...`
    SubjectAuthenticatedUserSSecurityProfile = "subject-authenticated-user-s-security-profile",
    /// `Restricted to the EHR of the patient in context.`
    RestrictedEhrPatientContext = "restricted-ehr-patient-context",
    /// `Not a context openEHR defines. Carried alongside the openEHR scopes and inert here.`
    NotContextOpenehrDefinesCarriedAlongside = "not-context-openehr-defines-carried-alongside",
    /// `Asks for an episode of care in context. Experimental in the specification.`
    AsksEpisodeCareContextExperimentalSpecification = "asks-episode-care-context-experimental-specification",
    /// `Launch context · episode`
    LaunchContextEpisode = "launch-context-episode",
    /// `Asks the launching platform to put one patient/EHR in context; a patient-compartment...`
    AsksLaunchingPlatformPutOnePatient = "asks-launching-platform-put-one-patient",
    /// `Launch context · patient`
    LaunchContextPatient = "launch-context-patient",
    /// `The CDR decides every request from the credential this session holds.`
    CdrDecidesEveryRequestFromCredential = "cdr-decides-every-request-from-credential",
    /// `Viewer session`
    ViewerSession = "viewer-session",
    /// `The CDR resolves this session's roles and permissions from the same access token; the...`
    CdrResolvesSessionSRolesPermissions = "cdr-resolves-session-s-roles-permissions",
    /// `OIDC bearer token`
    OidcBearerToken = "oidc-bearer-token",
    /// `The viewer replays this account's CDR credentials on every call, so the CDR applies...`
    ViewerReplaysAccountSCdrCredentials = "viewer-replays-account-s-cdr-credentials",
    /// `Basic authentication`
    BasicAuthentication = "basic-authentication",
    /// `An OpenID Connect scope: it identifies the user to the application and reaches no...`
    OpenidConnectScopeIdentifiesUserApplication = "openid-connect-scope-identifies-user-application",
    /// Marks an embedded launch. Advisory only — the CDR performs no launch selection.
    MarksEmbeddedLaunchAdvisoryOnlyCdr = "marks-embedded-launch-advisory-only-cdr",
    /// `Launch marker`
    LaunchMarker = "launch-marker",
    /// `Scopes narrow access, they never grant it: they say what a token may ask for, and the...`
    ScopesNarrowAccessTheyNeverGrant = "scopes-narrow-access-they-never-grant",
    /// `no subscription id to delete`
    NoSubscriptionIdDelete = "no-subscription-id-delete",
    /// `no subscription id to update`
    NoSubscriptionIdUpdate = "no-subscription-id-update",
    /// a subscription name is required and may hold only letters, digits, `_`, `.` and `-`
    SubscriptionNameRequiredMayHoldOnly = "subscription-name-required-may-hold-only",
    /// `Matches every committed version.`
    MatchesEveryCommittedVersion = "matches-every-committed-version",
    /// `candidate code`
    CandidateCode2 = "candidate-code-2",
    /// `value set id`
    ValueSetId2 = "value-set-id-2",
    /// `terminology id`
    TerminologyId = "terminology-id",
    /// `reference code`
    ReferenceCode2 = "reference-code-2",
    /// `term code`
    TermCode2 = "term-code-2",
    /// `$object is not in the CDR any more: it was already deleted. Reload this screen.`
    NotCdrAnyMoreWasAlready = "not-cdr-any-more-was-already",
    /// The CDR rejected the delete of `$object`: `$message`.
    CdrRejectedDelete = "cdr-rejected-delete",
    /// Deleting `$object` failed: `$error`
    DeletingFailed = "deleting-failed",
    /// The viewer session expired before `$object` was deleted — sign in again and retry.
    ViewerSessionExpiredBeforeWasDeleted = "viewer-session-expired-before-was-deleted",
    /// Deleting `$object` failed: `$other`
    DeletingFailed2 = "deleting-failed-2",
    /// column `$value0`
    Column2 = "column-2",
    /// `$size bytes`
    Bytes = "bytes",
    /// `$total $noun over $points days, most on $busiest.`
    OverDaysMostOn = "over-days-most-on",
    /// `$series_name per day`
    PerDay = "per-day",
    /// `ITEM_TAG list JSON: '$e'`
    ItemTagListJson = "item-tag-list-json",
    /// `$model series over $axis_label.`
    SeriesOver = "series-over",
    /// Version `$version`
    Version2 = "version-2",
    /// Open: `$gaps`.
    Open2 = "open-2",
    /// export failed: `$e`
    ExportFailed = "export-failed",
    /// parameters must be a JSON object: `$e`
    ParametersMustJsonObject2 = "parameters-must-json-object-2",
    /// unknown export format ``$other`` (csv | json)
    UnknownExportFormatCsvJson = "unknown-export-format-csv-json",
    /// `unparseable RESULT_SET: '$e'`
    UnparseableResultSet = "unparseable-result-set",
    /// column `$value0`
    Column3 = "column-3",
    /// attachment; filename="`$filename`"
    AttachmentFilename = "attachment-filename",
    /// `$object was not sent: $message. Correct the input and retry.`
    WasNotSentCorrectInputRetry = "was-not-sent-correct-input-retry",
    /// The CDR rejected `$object`: `$message`. Fix the reported problem and retry.
    CdrRejectedFixReportedProblemRetry = "cdr-rejected-fix-reported-problem-retry",
    /// The CDR answered `$status` to `$object`: `$message`. Nothing was saved.
    CdrAnsweredNothingWasSaved = "cdr-answered-nothing-was-saved",
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
    /// `AuditEvent Bundle JSON: '$e'`
    AuditeventBundleJson = "auditevent-bundle-json",
    /// `$shown_from–$shown_to of $total records`
    OfRecords = "of-records",
    /// the composition changed since this page loaded — reload and retry. `$detail`
    CompositionChangedSincePageLoadedReload = "composition-changed-since-page-loaded-reload",
    /// versioned composition JSON: `$e`
    VersionedCompositionJson = "versioned-composition-json",
    /// version JSON: `$e`
    VersionJson = "version-json",
    /// revision history JSON: `$e`
    RevisionHistoryJson = "revision-history-json",
    /// New version `$uid`
    NewVersion = "new-version",
    /// Composition `$short`…
    Composition = "composition",
    /// EHR `$ehr_short`…
    Ehr = "ehr",
    /// `$members queries`
    Queries2 = "queries-2",
    /// `$kind id`
    Id = "id",
    /// Open a `$kind`
    OpenA = "open-a",
    /// openEHR-DEMOGRAPHIC-`$kind`.`$kind2`.v1
    OpenehrDemographicV1 = "openehr-demographic-v1",
    /// New `$kind` version `$uid`
    NewVersion2 = "new-version-2",
    /// the new `$kind`
    TheNew = "the-new",
    /// Create a `$kind`
    CreateA = "create-a",
    /// `$kind document (canonical JSON)`
    DocumentCanonicalJson2 = "document-canonical-json-2",
    /// Create `$kind`
    Create = "create",
    /// No party kind holds `$uid` — the tag may target an object that was since deleted.
    NoPartyKindHoldsTagMay = "no-party-kind-holds-tag-may",
    /// CONTRIBUTION JSON: `$e`
    ContributionJson = "contribution-json",
    /// Contribution `$short`…
    Contribution = "contribution",
    /// versioned demographic object JSON: `$e`
    VersionedDemographicObjectJson = "versioned-demographic-object-json",
    /// demographic version JSON: `$e`
    DemographicVersionJson = "demographic-version-json",
    /// the party body is not valid JSON: `$e`
    PartyBodyNotValidJson = "party-body-not-valid-json",
    /// party JSON: `$e`
    PartyJson = "party-json",
    /// identities is not valid JSON: `$e`
    IdentitiesNotValidJson = "identities-not-valid-json",
    /// details is not valid JSON: `$e`
    DetailsNotValidJson = "details-not-valid-json",
    /// the loaded party document is not valid JSON (`$e`) — reload this tab and retry
    LoadedPartyDocumentNotValidJson = "loaded-party-document-not-valid-json",
    /// the edited party could not be serialized: `$e`
    EditedPartyCouldNotSerialized = "edited-party-could-not-serialized",
    /// New version `$uid`
    NewVersion3 = "new-version-3",
    /// the `$end` party's id is required
    PartySIdRequired = "party-s-id-required",
    /// the `$end` party's kind must be one of the five party kinds
    PartySKindMustOneFive = "party-s-kind-must-one-five",
    /// details is not valid JSON: `$e`
    DetailsNotValidJson2 = "details-not-valid-json-2",
    /// the edited relationship could not be serialized: `$e`
    EditedRelationshipCouldNotSerialized = "edited-relationship-could-not-serialized",
    /// `PARTY_RELATIONSHIP JSON: '$e'`
    PartyRelationshipJson = "party-relationship-json",
    /// `New PARTY_RELATIONSHIP version '$uid'`
    NewPartyRelationshipVersion = "new-party-relationship-version",
    /// Relationship `$short`…
    Relationship2 = "relationship-2",
    /// New version `$uid`
    NewVersion4 = "new-version-4",
    /// Commit `$many` changes as one contribution
    CommitChangesOneContribution = "commit-changes-one-contribution",
    /// The change set was committed as one contribution (`$versions` `$noun`).
    ChangeSetWasCommittedOneContribution = "change-set-was-committed-one-contribution",
    /// Contribution `$contribution_uid` committed with `$versions` `$noun`.
    ContributionCommittedWith = "contribution-committed-with",
    /// not valid JSON: `$e`
    NotValidJson = "not-valid-json",
    /// change `$position` (`$change` — `$target`): `$complaint`
    Change2 = "change-2",
    /// the contribution could not be serialized: `$e`
    ContributionCouldNotSerialized = "contribution-could-not-serialized",
    /// the document is not valid JSON: `$e`
    DocumentNotValidJson = "document-not-valid-json",
    /// `$civilZ`
    Z2 = "z-2",
    /// New version `$uid`
    NewVersion5 = "new-version-5",
    /// contributions JSON: `$e`
    ContributionsJson = "contributions-json",
    /// New version `$uid`
    NewVersion6 = "new-version-6",
    /// `$folder_count folders · $item_count items`
    FoldersItems = "folders-items",
    /// version JSON: `$e`
    VersionJson2 = "version-json-2",
    /// directory JSON: `$e`
    DirectoryJson = "directory-json",
    /// subtree JSON: `$e`
    SubtreeJson = "subtree-json",
    /// directory JSON: `$e`
    DirectoryJson2 = "directory-json-2",
    /// Invalid JSON: `$e`
    InvalidJson = "invalid-json",
    /// `$folders folders · $items items`
    FoldersItems2 = "folders-items-2",
    /// EHR JSON: `$e`
    EhrJson = "ehr-json",
    /// `ehr_status ('$ehr_status_type')`
    EhrStatus2 = "ehr-status-2",
    /// EHR `$short`…
    Ehr2 = "ehr-2",
    /// EHR `$id` and all of its data were removed from the CDR.
    EhrAllDataWereRemovedFrom = "ehr-all-data-were-removed-from",
    /// EHR `$id`
    Ehr3 = "ehr-3",
    /// `other_details is not valid JSON: '$e'`
    OtherDetailsNotValidJson = "other-details-not-valid-json",
    /// `the loaded EHR_STATUS document is not valid JSON ('$e') — reload this tab and retry`
    LoadedEhrStatusDocumentNotValid = "loaded-ehr-status-document-not-valid",
    /// `the edited EHR_STATUS could not be serialized: '$e'`
    EditedEhrStatusCouldNotSerialized = "edited-ehr-status-could-not-serialized",
    /// `ehr_status JSON: '$e'`
    EhrStatusJson = "ehr-status-json",
    /// `versioned ehr_status JSON: '$e'`
    VersionedEhrStatusJson = "versioned-ehr-status-json",
    /// version JSON: `$e`
    VersionJson3 = "version-json-3",
    /// New version `$uid`
    NewVersion7 = "new-version-7",
    /// EHR JSON: `$e`
    EhrJson2 = "ehr-json-2",
    /// Nothing in this EHR holds `$uid` — the tagged object may have been deleted.
    NothingEhrHoldsTaggedObjectMay = "nothing-ehr-holds-tagged-object-may",
    /// AQL result JSON: `$e`
    AqlResultJson = "aql-result-json",
    /// EHR JSON: `$e`
    EhrJson3 = "ehr-json-3",
    /// New EHR `$new_id`
    NewEhr = "new-ehr",
    /// Could not read the CDR's login modes: `$error`
    CouldNotReadCdrSLogin = "could-not-read-cdr-s-login",
    /// The CDR is now logging with ``$filter`` (requested ``$filter2``).
    CdrNowLoggingRequested = "cdr-now-logging-requested",
    /// the log filter ``$filter``
    TheLogFilter2 = "the-log-filter-2",
    /// The CDR restored its boot filter ``$filter``.
    CdrRestoredBootFilter = "cdr-restored-boot-filter",
    /// `$name v$version was removed from the CDR.`
    VWasRemovedFromCdr = "v-was-removed-from-cdr",
    /// Stored query `$name` v`$version`
    StoredQueryV = "stored-query-v",
    /// `$count queries`
    Queries3 = "queries-3",
    /// Toggle `$node_label`
    Toggle = "toggle",
    /// Remove code `$remove_code`
    RemoveCode = "remove-code",
    /// Alias for column `$position`
    AliasForColumn = "alias-for-column",
    /// Remove column `$position`
    RemoveColumn = "remove-column",
    /// Sort path for rule `$position`
    SortPathForRule = "sort-path-for-rule",
    /// Sort direction for rule `$position`
    SortDirectionForRule = "sort-direction-for-rule",
    /// Remove sort rule `$position`
    RemoveSortRule = "remove-sort-rule",
    /// a new immutable version `$version`
    ANewImmutableVersion = "a-new-immutable-version",
    /// `$label is a coded value`
    IsACodedValue = "is-a-coded-value",
    /// `$label is one of $names`
    IsOneOf = "is-one-of",
    /// `$label is a graded value`
    IsAGradedValue = "is-a-graded-value",
    /// `$label is one of $names`
    IsOneOf2 = "is-one-of-2",
    /// `$label equals "$text"`
    Equals = "equals",
    /// `$label contains "$value1"`
    Contains = "contains",
    /// `$label from $from to $to`
    FromTo = "from-to",
    /// `$label on or after $from`
    OnOrAfter = "on-or-after",
    /// `$label on or before $to`
    OnOrBefore = "on-or-before",
    /// `$label (any date)`
    AnyDate = "any-date",
    /// `$label is $value`
    Is = "is",
    /// `$label is present`
    IsPresent = "is-present",
    /// NOT (`$body`)
    Not = "not",
    /// `$word between $a and $b`
    BetweenAnd = "between-and",
    /// `$word at least $a`
    AtLeast = "at-least",
    /// `$word at most $b`
    AtMost = "at-most",
    /// POST query/`$name`
    PostQuery = "post-query",
    /// POST query/`$name`/`$resolved`
    PostQuery2 = "post-query-2",
    /// CDR UP · v`$version``$profile`
    CdrUpV = "cdr-up-v",
    /// `$name was removed from the CDR.`
    WasRemovedFromCdr = "was-removed-from-cdr",
    /// subscription ``$name``
    Subscription2 = "subscription-2",
    /// Edit subscription `$name`
    EditSubscription = "edit-subscription",
    /// Showing the `$shown` busiest of `$total` templates.
    ShowingBusiestTemplates = "showing-busiest-templates",
    /// status JSON: `$e`
    StatusJson = "status-json",
    /// HTTP `$status`
    Http = "http",
    /// the CDR answered `$status` there
    TheCdrAnsweredThere = "the-cdr-answered-there",
    /// the CDR answered `$status` there: `$message`
    TheCdrAnsweredThere2 = "the-cdr-answered-there-2",
    /// openapi JSON: `$e`
    OpenapiJson = "openapi-json",
    /// `OperationalTemplateV2 parse: '$e'`
    Operationaltemplatev2Parse = "operationaltemplatev2-parse",
    /// WebTemplate build: `$value0`
    WebtemplateBuild = "webtemplate-build",
    /// ADL2 template · `$template_id`
    Adl2Template = "adl2-template",
    /// OPT 1.4 parse: `$e`
    Opt14Parse = "opt-1-4-parse",
    /// WebTemplate build: `$value0`
    WebtemplateBuild2 = "webtemplate-build-2",
    /// Template · `$template_id`
    Template2 = "template-2",
    /// `$id was removed from the CDR.`
    WasRemovedFromCdr2 = "was-removed-from-cdr-2",
    /// Template `$id`
    Template3 = "template-3",
    /// template list JSON: `$e`
    TemplateListJson = "template-list-json",
    /// ADL2 template list JSON: `$e`
    Adl2TemplateListJson = "adl2-template-list-json",
    /// `$template_id was accepted by the CDR.`
    WasAcceptedCdr = "was-accepted-cdr",
    /// `$template_id was removed from the CDR.`
    WasRemovedFromCdr3 = "was-removed-from-cdr-3",
    /// Template `$template_id`
    Template4 = "template-4",
    /// parameters must be a JSON object: `$e`
    ParametersMustJsonObject3 = "parameters-must-json-object-3",
    /// stored-query list JSON: `$e`
    StoredQueryListJson = "stored-query-list-json",
    /// stored query JSON: `$e`
    StoredQueryJson = "stored-query-json",
    /// parameters must be a JSON object: `$e`
    ParametersMustJsonObject4 = "parameters-must-json-object-4",
    /// stored-query result JSON: `$e`
    StoredQueryResultJson = "stored-query-result-json",
    /// Identity claim · `$name`
    IdentityClaim = "identity-claim",
    /// Launch context · `$other`
    LaunchContext = "launch-context",
    /// "`$compartment`" is not a compartment. A resource scope is `$resource_scope_form`, and...
    NotCompartmentResourceScopeMaster08Defines = "not-compartment-resource-scope-master08-defines",
    /// `"'$rest'" carries no .<permission> tail. A resource scope is '$resource_scope_form' —...`
    CarriesNoPermissionTailResourceScope = "carries-no-permission-tail-resource-scope",
    /// "`$tail`" is not a permission tail. master08 allows only c (create), r (read), u...
    NotPermissionTailMaster08AllowsOnly = "not-permission-tail-master08-allows-only",
    /// `"'$resource'" is not a resource. master08 defines template-<templateId>,...`
    NotResourceMaster08DefinesTemplateTemplateid = "not-resource-master08-defines-template-templateid",
    /// Not a resource scope. The master08 form is `$resource_scope_form` — for example...
    NotResourceScopeMaster08FormExample = "not-resource-scope-master08-form-example",
    /// Matches `$parts`.
    Matches = "matches",
    /// event subscription list JSON: `$e`
    EventSubscriptionListJson = "event-subscription-list-json",
    /// event subscription record JSON: `$e`
    EventSubscriptionRecordJson = "event-subscription-record-json",
    /// conformance manifest JSON: `$e`
    ConformanceManifestJson = "conformance-manifest-json",
    /// terminology JSON: `$e`
    TerminologyJson = "terminology-json",
    /// a `$what` is required
    AIsRequired = "a-is-required",
    /// `The CDR ingests an ADL 1.4 operational template as OPT/XML. Choose a file or paste the...`
    CdrIngestsAdl14Operational = "cdr-ingests-adl-1-4-operational",
    /// `The CDR ingests the ADL2 artefact SOURCE as text/plain. Choose a file or paste the...`
    CdrIngestsAdl2ArtefactSourceText = "cdr-ingests-adl2-artefact-source-text",
    /// `$object is still referenced by committed data, so the CDR refused the delete...`
    StillReferencedCommittedDataSoCdr = "still-referenced-committed-data-so-cdr",
    /// This session may not delete `$object` (`$message`). Sign in with an ADMIN-role account...
    SessionMayNotDeleteSignAdmin = "session-may-not-delete-sign-admin",
    /// The CDR no longer accepts this session, so `$object` was not deleted (`$message`)....
    CdrNoLongerAcceptsSessionSo = "cdr-no-longer-accepts-session-so",
    /// `Sandbox deployment: this deployment has not made the production separations and must...`
    SandboxDeploymentDeploymentHasNotMade = "sandbox-deployment-deployment-has-not-made",
    /// `no subject pseudonym namespace is declared, so an EHR subject may carry a real identifier`
    NoSubjectPseudonymNamespaceDeclaredSo = "no-subject-pseudonym-namespace-declared-so",
    /// `an access record the audit queue cannot take is dropped and the request still succeeds`
    AccessRecordAuditQueueCannotTake = "access-record-audit-queue-cannot-take",
    /// `an EHR without its own access settings admits every caller the role checks let through`
    EhrWithoutOwnAccessSettingsAdmits = "ehr-without-own-access-settings-admits",
    /// `role checks are off, so every signed-in caller may use every enabled surface, the...`
    RoleChecksOffSoEverySigned = "role-checks-off-so-every-signed",
    /// `$object changed in the CDR since this screen loaded ($message). Reload the latest...`
    ChangedCdrSinceScreenLoadedReload = "changed-cdr-since-screen-loaded-reload",
    /// The CDR already holds a conflicting object, so `$object` was not created (`$message`)....
    CdrAlreadyHoldsConflictingObjectSo = "cdr-already-holds-conflicting-object-so",
    /// `$object is not in the CDR ($message): it may have been deleted meanwhile. Reload this...`
    NotCdrMayHaveBeenDeleted = "not-cdr-may-have-been-deleted",
    /// The CDR no longer accepts this session, so `$object` was not saved (`$message`). Sign...
    CdrNoLongerAcceptsSessionSo2 = "cdr-no-longer-accepts-session-so-2",
    /// This session may not write `$object` (`$message`). Sign in with an account that...
    SessionMayNotWriteSignAccount = "session-may-not-write-sign-account",
    /// The viewer session expired before `$object` was saved — sign in again and retry; your...
    ViewerSessionExpiredBeforeWasSaved = "viewer-session-expired-before-was-saved",
    /// The CDR is unreachable (`$message`), so `$object` was not saved. Check the CDR, then...
    CdrUnreachableSoWasNotSaved = "cdr-unreachable-so-was-not-saved",
    /// `$object could not be saved: $message. Retry; if it persists, check the viewer logs.`
    CouldNotSavedRetryIfPersists = "could-not-saved-retry-if-persists",
    /// `$object moved on in the CDR since this screen loaded, so nothing was deleted...`
    MovedCdrSinceScreenLoadedSo = "moved-cdr-since-screen-loaded-so",
    /// The CDR refused to delete `$object`: `$message`. It may already be deleted — reload...
    CdrRefusedDeleteMayAlreadyDeleted = "cdr-refused-delete-may-already-deleted",
    /// `$object is not in the CDR ($message): it may have been deleted meanwhile. Reload this...`
    NotCdrMayHaveBeenDeleted2 = "not-cdr-may-have-been-deleted-2",
    /// This session may not delete `$object` (`$message`). Sign in with an account that...
    SessionMayNotDeleteSignAccount = "session-may-not-delete-sign-account",
    /// The CDR no longer accepts this session, so nothing was deleted (`$message`). Sign in...
    CdrNoLongerAcceptsSessionSo3 = "cdr-no-longer-accepts-session-so-3",
    /// `the latest version's full OBJECT_VERSION_ID is required to delete a composition:...`
    LatestVersionSFullObjectVersion = "latest-version-s-full-object-version",
    /// `Logically delete the latest version of this composition? The CDR commits a deleted...`
    LogicallyDeleteLatestVersionCompositionCdr = "logically-delete-latest-version-composition-cdr",
    /// The openEHR demographic API addresses a `$kind` by its id — it publishes no party...
    OpenehrDemographicApiAddressesIdPublishes = "openehr-demographic-api-addresses-id-publishes",
    /// `deleting a party needs the latest version's full OBJECT_VERSION_ID: reload this screen...`
    DeletingPartyNeedsLatestVersionS = "deleting-party-needs-latest-version-s",
    /// `identities must hold at least one PARTY_IDENTITY: a party with no identity is invalid...`
    IdentitiesMustHoldLeastOneParty = "identities-must-hold-least-one-party",
    /// Commits a new `$kind` version on top of the one loaded above (If-Match), so a...
    CommitsNewVersionTopOneLoaded2 = "commits-new-version-top-one-loaded-2",
    /// Delete `$kind` `$uid`? This commits a deleted version: the party stops resolving as...
    DeleteCommitsDeletedVersionPartyStops = "delete-commits-deleted-version-party-stops",
    /// `the current version uid is required to update this relationship: reload this screen...`
    CurrentVersionUidRequiredUpdateRelationship = "current-version-uid-required-update-relationship",
    /// `deleting a relationship needs the latest version's full OBJECT_VERSION_ID: reload this...`
    DeletingRelationshipNeedsLatestVersionS = "deleting-relationship-needs-latest-version-s",
    /// `a relationship type is required: it is stored as the relationship's name...`
    RelationshipTypeRequiredStoredRelationshipS = "relationship-type-required-stored-relationship-s",
    /// `the '$end' party must be named by its versioned_object_uid, not by one version's...`
    PartyMustNamedVersionedObjectUid = "party-must-named-versioned-object-uid",
    /// the loaded relationship document is not valid JSON (`$e`) — reload this screen and retry
    LoadedRelationshipDocumentNotValidJson = "loaded-relationship-document-not-valid-json",
    /// Delete relationship `$uid`? This commits a deleted version: the relationship stops...
    DeleteRelationshipCommitsDeletedVersionRelationship = "delete-relationship-commits-deleted-version-relationship",
    /// `These are the VERSIONED_PARTY container's tags. Saving one re-sends the whole...`
    TheseVersionedPartyContainerSTags = "these-versioned-party-container-s-tags",
    /// Permanently delete EHR `$ehr_id` — every composition, contribution and audit record in...
    PermanentlyDeleteEhrEveryCompositionContribution = "permanently-delete-ehr-every-composition-contribution",
    /// `the current EHR_STATUS version uid is required to update it: reload this tab and retry`
    CurrentEhrStatusVersionUidRequired = "current-ehr-status-version-uid-required",
    /// `Tags on the collection named below: the VERSIONED_COMPOSITION container while the...`
    TagsCollectionNamedBelowVersionedComposition = "tags-collection-named-below-versioned-composition",
    /// `Tags are free key/value markers a client sets on a composition; add one below.`
    TagsFreeKeyValueMarkersClient2 = "tags-free-key-value-markers-client-2",
    /// `These are the VERSIONED_EHR_STATUS container's tags, so they stay put when the status...`
    TheseVersionedEhrStatusContainerS = "these-versioned-ehr-status-container-s",
    /// `Tags are free key/value markers a client sets on the EHR's status; add one below.`
    TagsFreeKeyValueMarkersClient3 = "tags-free-key-value-markers-client-3",
    /// `An EHR id must be a UUID (8-4-4-4-12 hex, e.g. 7d44aa01-0f9e-4a2c-9a0f-2a6a5f9b1c3d)....`
    EhrIdMustUuid84 = "ehr-id-must-uuid-8-4",
    /// Switch the CDR's live log filter to “`$filter`”? Logging changes immediately for every...
    SwitchCdrSLiveLogFilter = "switch-cdr-s-live-log-filter",
    /// Permanently delete stored query “`$name`” version `$version` from the CDR? Every...
    PermanentlyDeleteStoredQueryVersionFrom = "permanently-delete-stored-query-version-from",
    /// `$version is not a version to store at: use major.minor.patch, for example 1.0.0`
    NotVersionStoreUseMajorMinor = "not-version-store-use-major-minor",
    /// `The name is unique on the CDR and cannot be changed afterwards: it may hold only...`
    NameUniqueCdrCannotChangedAfterwards = "name-unique-cdr-cannot-changed-afterwards",
    /// Delete the subscription “`$name`” from the CDR? `$value1` Consumers reading its stream...
    DeleteSubscriptionFromCdrConsumersReading = "delete-subscription-from-cdr-consumers-reading",
    /// SMART discovery could not be read: the CDR is unreachable (`$detail`). This says...
    SmartDiscoveryCouldNotReadCdr = "smart-discovery-could-not-read-cdr",
    /// SMART discovery could not be read: the CDR refused this session at `$endpoint`. Sign...
    SmartDiscoveryCouldNotReadCdr2 = "smart-discovery-could-not-read-cdr-2",
    /// SMART discovery could not be read: this session may not read `$endpoint`. It needs an...
    SmartDiscoveryCouldNotReadSession = "smart-discovery-could-not-read-session",
    /// SMART discovery could not be read from `$endpoint` — `$detail`. Whether SMART is...
    SmartDiscoveryCouldNotReadFrom = "smart-discovery-could-not-read-from",
    /// `Built by the viewer from the served OperationalTemplateV2: the ADL2 resource serves no...`
    BuiltViewerFromServedOperationaltemplatev2Adl2 = "built-viewer-from-served-operationaltemplatev2-adl2",
    /// The CDR holds no `$what` for this template at the selected version. Pick another...
    CdrHoldsNoTemplateSelectedVersion = "cdr-holds-no-template-selected-version",
    /// `The example is generated from the artefact this route names: the Definition API...`
    ExampleGeneratedFromArtefactRouteNames = "example-generated-from-artefact-route-names",
    /// Permanently delete the operational template “`$template_id`” from the CDR? This cannot...
    PermanentlyDeleteOperationalTemplateFromCdr = "permanently-delete-operational-template-from-cdr",
    /// Permanently delete the operational template “`$id`” from the CDR? This cannot be...
    PermanentlyDeleteOperationalTemplateFromCdr2 = "permanently-delete-operational-template-from-cdr-2",
    /// Permanently delete the ADL 2 artefact “`$id`” from the CDR's definition store? This...
    PermanentlyDeleteAdl2ArtefactFrom = "permanently-delete-adl-2-artefact-from",
    /// `` `$version` is not a version to store at: a stored-query version is `major.minor.patch`... ``
    NotVersionStoreStoredQueryVersion = "not-version-store-stored-query-version",
    /// The CDR already holds a subscription with that name, so `$object` was not saved...
    CdrAlreadyHoldsSubscriptionNameSo = "cdr-already-holds-subscription-name-so",
    /// `$object is not on the CDR any more ($message): another operator may have deleted it....`
    NotCdrAnyMoreAnotherOperator = "not-cdr-any-more-another-operator",
    /// This session may not administer `$object` (`$message`). Sign in with an ADMIN-role...
    SessionMayNotAdministerSignAdmin = "session-may-not-administer-sign-admin",
    /// `No version opened`
    NoVersionOpened = "no-version-opened",
    /// `Open a version from the revision history above, or resolve one by date and time.`
    OpenVersionFromRevisionHistoryAbove = "open-version-from-revision-history-above",
    /// `single-row`
    SingleRow = "single-row",
    /// `One row is not a trend`
    OneRowNotTrend = "one-row-not-trend",
    /// `A chart needs at least two rows. Widen the query's window, or relax a condition, and...`
    ChartNeedsLeastTwoRowsWiden = "chart-needs-least-two-rows-widen",
    /// `no-numeric-column`
    NoNumericColumn = "no-numeric-column",
    /// `Nothing numeric to chart`
    NothingNumericToChart = "nothing-numeric-to-chart",
    /// `Charting needs a column holding at least two numbers. Project a quantity magnitude, a...`
    ChartingNeedsColumnHoldingLeastTwo = "charting-needs-column-holding-least-two",
    /// `Create an EHR, find one by id or subject, or browse the most recent.`
    CreateEhrFindOneIdSubject = "create-ehr-find-one-id-subject",
    /// `No EHRs found`
    NoEhrsFound = "no-ehrs-found",
    /// `Create an EHR through the CDR, then browse it here.`
    CreateEhrThroughCdrThenBrowse = "create-ehr-through-cdr-then-browse",
    /// `The CDR's event subscriptions: the stored filters over its committed-version event stream.`
    CdrSEventSubscriptionsStoredFilters = "cdr-s-event-subscriptions-stored-filters",
    /// `The event-subscription API is disabled on this server`
    EventSubscriptionApiDisabledServer = "event-subscription-api-disabled-server",
    /// `The CDR answers its subscription routes as if unmounted. Set admin_api = true under...`
    CdrAnswersSubscriptionRoutesIfUnmounted = "cdr-answers-subscription-routes-if-unmounted",
    /// `No event subscriptions`
    NoEventSubscriptions = "no-event-subscriptions",
    /// `Create the first subscription with the card above.`
    CreateFirstSubscriptionCardAbove = "create-first-subscription-card-above",
    /// `The ATNA security audit trail: who accessed what, with what outcome (IHE ITI-81...`
    AtnaSecurityAuditTrailWhoAccessed = "atna-security-audit-trail-who-accessed",
    /// `The local audit repository is disabled`
    LocalAuditRepositoryDisabled = "local-audit-repository-disabled",
    /// `Enable [audit.store] on the CDR to record and browse the ATNA audit trail here.`
    EnableAuditStoreCdrRecordBrowse = "enable-audit-store-cdr-record-browse",
    /// `No audit records match`
    NoAuditRecordsMatch = "no-audit-records-match",
    /// `Widen the time window or clear a filter: every audited operation lands here.`
    WidenTimeWindowClearFilterEvery = "widen-time-window-clear-filter-every",
    /// `Pick a template, walk its paths, and turn data-value leaves into criteria and columns.`
    PickTemplateWalkPathsTurnData = "pick-template-walk-paths-turn-data",
    /// `No conditions yet`
    NoConditionsYet = "no-conditions-yet",
    /// `No rows`
    NoRows = "no-rows",
    /// `The query ran and matched nothing. Widen a condition or clear a filter.`
    QueryRanMatchedNothingWidenCondition = "query-ran-matched-nothing-widen-condition",
    /// `CDR status, the openEHR conformance manifest, SMART discovery, repository usage, the...`
    CdrStatusOpenehrConformanceManifestSmart = "cdr-status-openehr-conformance-manifest-smart",
    /// `No template usage yet`
    NoTemplateUsageYet = "no-template-usage-yet",
    /// `Counts appear once compositions are committed against a template.`
    CountsAppearOnceCompositionsCommittedAgainst = "counts-appear-once-compositions-committed-against",
    /// `No paths advertised`
    NoPathsAdvertised = "no-paths-advertised",
    /// `The CDR served an OpenAPI document with no operations in it.`
    CdrServedOpenapiDocumentNoOperations = "cdr-served-openapi-document-no-operations",
    /// `This version was deleted at the selected instant`
    VersionWasDeletedSelectedInstant = "version-was-deleted-selected-instant",
    /// `The delete committed a deleted-lifecycle version; pick an earlier version above to...`
    DeleteCommittedDeletedLifecycleVersionPick = "delete-committed-deleted-lifecycle-version-pick",
    /// `No audit for the selected version`
    NoAuditSelectedVersion = "no-audit-selected-version",
    /// `Pick another version above: every committed version carries its own audit.`
    PickAnotherVersionAboveEveryCommitted = "pick-another-version-above-every-committed",
    /// `No tags in this EHR`
    NoTagsEhr = "no-tags-ehr",
    /// `Tags are free key/value markers a client sets on a composition, the EHR status or the...`
    TagsFreeKeyValueMarkersClient4 = "tags-free-key-value-markers-client-4",
    /// `Browse the terminologies the CDR serves: describe one, define a code, expand a value...`
    BrowseTerminologiesCdrServesDescribeOne = "browse-terminologies-cdr-serves-describe-one",
    /// `The terminology extension is disabled on this server`
    TerminologyExtensionDisabledServer = "terminology-extension-disabled-server",
    /// `The CDR answers its terminology routes as if unmounted. Set api_enabled = true under...`
    CdrAnswersTerminologyRoutesIfUnmounted = "cdr-answers-terminology-routes-if-unmounted",
    /// `No terminologies`
    NoTerminologies = "no-terminologies",
    /// `The CDR serves the terminology API but reports no terminology in it.`
    CdrServesTerminologyApiButReports = "cdr-serves-terminology-api-but-reports",
    /// `Nothing in this extract`
    NothingInThisExtract = "nothing-in-this-extract",
    /// `The CDR answered with an extract that carries no term and no relationship.`
    CdrAnsweredExtractCarriesNoTerm = "cdr-answered-extract-carries-no-term",
    /// `The CDR's stored queries, grouped by the namespace in their qualified name.`
    CdrSStoredQueriesGroupedNamespace = "cdr-s-stored-queries-grouped-namespace",
    /// `No stored queries`
    NoStoredQueries = "no-stored-queries",
    /// `Build one in the query builder, or write raw AQL, then save it.`
    BuildOneQueryBuilderWriteRaw = "build-one-query-builder-write-raw",
    /// `No namespaces yet`
    NoNamespacesYet = "no-namespaces-yet",
    /// `Save a query as namespace::name: its namespace becomes a group here and a tile on the...`
    SaveQueryNamespaceNameNamespaceBecomes = "save-query-namespace-name-namespace-becomes",
    /// `Dependency health, build provenance, the metric registry, and live log control.`
    DependencyHealthBuildProvenanceMetricRegistry = "dependency-health-build-provenance-metric-registry",
    /// `Operational templates registered in the CDR, by archetype-model family.`
    OperationalTemplatesRegisteredCdrArchetypeModel = "operational-templates-registered-cdr-archetype-model",
    /// `No templates match the filter`
    NoTemplatesMatchFilter = "no-templates-match-filter",
    /// `Clear or shorten the filter to see the whole catalogue again.`
    ClearShortenFilterSeeWholeCatalogue = "clear-shorten-filter-see-whole-catalogue",
    /// `No templates yet`
    NoTemplatesYet = "no-templates-yet",
    /// `Repository overview, query cohorts, and recent commit activity.`
    RepositoryOverviewQueryCohortsRecentCommit = "repository-overview-query-cohorts-recent-commit",
    /// `No stored queries yet`
    NoStoredQueriesYet = "no-stored-queries-yet",
    /// `Save a query as namespace::name: each namespace becomes a cohort tile here.`
    SaveQueryNamespaceNameEachNamespace = "save-query-namespace-name-each-namespace",
    /// `Execute a stored query on the CDR: choose how its version resolves, bind its...`
    ExecuteStoredQueryCdrChooseHow = "execute-stored-query-cdr-choose-how",
    /// `Write and run AQL directly, bind parameters, and save it as a stored query.`
    WriteRunAqlDirectlyBindParameters = "write-run-aql-directly-bind-parameters",
    /// `A relationship joins two parties, source to target. It is its own versioned object...`
    RelationshipJoinsTwoPartiesSourceTarget = "relationship-joins-two-parties-source-target",
    /// `No relationships on this party`
    NoRelationshipsParty = "no-relationships-party",
    /// `A party document carries the relationships it is the SOURCE of. Create one below, or...`
    PartyDocumentCarriesRelationshipsSourceCreate = "party-document-carries-relationships-source-create",
    /// `No demographic tags`
    NoDemographicTags = "no-demographic-tags",
    /// `Tags are the one demographic collection the openEHR API lists. Set one on a party's...`
    TagsOneDemographicCollectionOpenehrApi = "tags-one-demographic-collection-openehr-api",
    /// `No versions listed`
    NoVersionsListed = "no-versions-listed",
    /// `A CONTRIBUTION always references at least one version; if none is listed, the CDR...`
    ContributionAlwaysReferencesLeastOneVersion = "contribution-always-references-least-one-version",
    /// `No contributions`
    NoContributions = "no-contributions",
    /// `This EHR has no contributions on this page.`
    EhrHasNoContributionsPage = "ehr-has-no-contributions-page",
    /// `Nothing staged yet`
    NothingStagedYet = "nothing-staged-yet",
    /// `Add a change above; they commit together as one contribution.`
    AddChangeAboveTheyCommitTogether = "add-change-above-they-commit-together",
    /// `No versions yet`
    NoVersionsYet = "no-versions-yet",
    /// `This EHR has no directory. Create one and its versions are listed here.`
    EhrHasNoDirectoryCreateOne = "ehr-has-no-directory-create-one",
    /// `Deleted at that time`
    DeletedAtThatTime = "deleted-at-that-time",
    /// `The directory had been deleted as of the selected time.`
    DirectoryHadBeenDeletedSelectedTime = "directory-had-been-deleted-selected-time",
    /// `No directory then`
    NoDirectoryThen = "no-directory-then",
    /// `No directory existed at the selected time.`
    NoDirectoryExistedSelectedTime = "no-directory-existed-selected-time",
    /// `No folder at that path`
    NoFolderPath = "no-folder-path",
    /// `The directory has no sub-folder matching the path.`
    DirectoryHasNoSubFolderMatching = "directory-has-no-sub-folder-matching",

    /// A value the server did not state.
    NoValue = "no-value",
    /// The web template tab.
    TabWt = "tab-wt",
    /// The operational template tab.
    TabOpt = "tab-opt",
    /// The filter choice that keeps every outcome.
    ChoiceAny = "choice-any",
    /// The audit outcome of a success.
    ChoiceSuccess = "choice-success",
    /// The audit action of a create.
    ActionCreate = "action-create",
    /// The audit action of a read.
    ActionRead = "action-read",
    /// The audit action of an update.
    ActionUpdate = "action-update",
    /// The audit action of a delete.
    ActionDelete = "action-delete",
    /// The audit action of an execution.
    ActionExecute = "action-execute",
    /// The noun of the EHRs, by count, with its empty and opening forms.
    NounEhrs = "noun-ehrs",
    /// The noun of the subscriptions, by count, with its empty and opening forms.
    NounSubscriptions = "noun-subscriptions",
    /// The noun of the tagged objects, by count, with its empty and opening forms.
    NounTaggedObjects = "noun-tagged-objects",
    /// The noun of the stored queries, by count, with its empty and opening forms.
    NounStoredQueries = "noun-stored-queries",
    /// The noun of the tags, by count, with its empty and opening forms.
    NounTags = "noun-tags",
    /// The noun of the compositions, by count, with its empty and opening forms.
    NounCompositions = "noun-compositions",
    /// The noun of the ADL 1.4 templates, by count, with its empty and opening forms.
    NounAdl14Templates = "noun-adl14-templates",
    /// The noun of the ADL 2 templates, by count, with its empty and opening forms.
    NounAdl2Templates = "noun-adl2-templates",
    /// The EHR status flag `is_queryable`, on a badge
    FlagQueryable = "flag-queryable",
    /// The EHR status flag `is_modifiable`, on a badge
    FlagModifiable = "flag-modifiable",
}

/// The viewer's message `msg` in the page's locale.
#[must_use]
pub fn tr(msg: Msg) -> String {
    tr_in(catalog(), &locale(), msg)
}

/// The console's message `msg` in the page's locale, with the placeables
/// `args` filled in.
#[must_use]
pub fn tr_with(msg: Msg, args: &[(&'static str, Arg)]) -> String {
    locale().lookup_with(catalog(), msg.id(), args)
}

/// The console's message `msg` in `locale`, with the placeables `args`
/// filled in: for a handler or a task, where no page tree is in reach.
#[must_use]
pub fn tr_in_with(locale: &Locale, msg: Msg, args: &[(&'static str, Arg)]) -> String {
    locale.lookup_with(catalog(), msg.id(), args)
}

/// The console's message `msg` as a [`Message`] a component looks up as it
/// draws, such as a listing's noun or an editor's title.
#[must_use]
pub fn message(msg: Msg) -> Message {
    Message::new(catalog(), msg.id())
}

/// The message `msg` of `catalog` in `locale`.
#[must_use]
pub fn tr_in(catalog: &dyn Loader, locale: &Locale, msg: Msg) -> String {
    locale.lookup(catalog, msg.id())
}

/// The console's message `msg` in the locale of the page being rendered,
/// read from the request where no page tree is in reach: for a server
/// function.
#[must_use]
pub fn tr_served(msg: Msg) -> String {
    current().lookup(catalog(), msg.id())
}

/// The console's message `msg` with the placeables `args` filled in, in the
/// locale [`tr_served`] reads.
#[must_use]
pub fn tr_served_with(msg: Msg, args: &[(&'static str, Arg)]) -> String {
    current().lookup_with(catalog(), msg.id(), args)
}

/// The locale of the page being rendered.
///
/// On the server it is the best language of the request's `Accept-Language`
/// the catalog holds, else the configured one; in the browser, the `lang` the
/// server wrote on `<html>`.
#[must_use]
pub fn current() -> Locale {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use leptos::prelude::use_context;
        let configured = use_context::<crate::state::AppState>()
            .and_then(|state| Locale::parse(&state.config.ui.locale).ok())
            .unwrap_or_default();
        let header = use_context::<http::request::Parts>()
            .and_then(|parts| {
                parts
                    .headers
                    .get(http::header::ACCEPT_LANGUAGE)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        Locale::negotiate_with(&header, catalog(), configured)
    }
    // NOTE: no openEHR spec governs this: our own design; the browser reads the
    // tag the server wrote, as the theme reads `data-theme`.
    #[cfg(target_arch = "wasm32")]
    {
        leptos::prelude::document()
            .document_element()
            .and_then(|root| root.get_attribute("lang"))
            .and_then(|tag| Locale::parse(&tag).ok())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::collections::HashMap;
    use std::path::Path;

    use ferrohealth_ui::i18n::Locale;
    use fluent_templates::fluent_bundle::FluentValue;

    use super::{Msg, catalog, tr_in};

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

    #[test]
    fn every_catalog_directory_is_a_language_holding_every_message() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("locales");
        let mut found: Vec<String> = std::fs::read_dir(&root)
            .expect("the locales directory")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        found.sort();
        let mut shipped: Vec<String> = catalog().locales().map(ToString::to_string).collect();
        shipped.sort();
        assert_eq!(found, shipped, "a catalog directory is a language");
        assert!(
            found.contains(&String::from("en")),
            "English is the reference"
        );
        for tag in found {
            let language = tag.parse().expect("a language");
            let args = placeables(&root.join(&tag).join("ferroehr-viewer.ftl"));
            for msg in Msg::ALL {
                let words = catalog().try_lookup_with_args(&language, msg.id(), &args);
                assert!(
                    words.is_some_and(|words| !words.trim().is_empty()),
                    "{tag} holds {}",
                    msg.id()
                );
            }
        }
    }

    #[test]
    fn a_throwaway_catalog_changes_the_words_with_no_code_change() {
        let english = Locale::english();
        assert_eq!("Not found", tr_in(catalog(), &english, Msg::Title404));
        let throwaway = Locale::parse("qaa").expect("a private-use language");
        assert_eq!("Qot qound", tr_in(&*THROWAWAY, &throwaway, Msg::Title404));
        assert_eq!(
            "Page not found",
            tr_in(catalog(), &throwaway, Msg::Heading404),
            "the shipped catalogs fall back to English"
        );
    }
}
