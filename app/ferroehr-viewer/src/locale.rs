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
        // FerroUI's formatter groups every number a message takes in the
        // page's language, from each catalog's `number-group` (FerroUI #293).
        customise: |bundle| {
            bundle.set_use_isolating(false);
            bundle.set_formatter(Some(ferrohealth_ui::i18n::number::format_number));
        },
    };
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
    /// `FerroEHR`
    BrandName = "brand-name",
    /// `Viewer`
    BrandWord = "brand-word",
    /// `CDR unreachable`
    CdrUnreachableCode = "cdr-unreachable-code",
    /// `{ $count ->`
    DaysAgo = "days-ago",
    /// `Discard`
    Discard = "discard",
    /// `Decision`
    DockLabel = "dock-label",
    /// `You are signed in as { $who }, and the CDR shows this only to { $needs }.`
    ForbiddenBody = "forbidden-body",
    /// `{ $count } h ago`
    HoursAgo = "hours-ago",
    /// `Viewer error`
    InternalCode = "internal-code",
    /// `Not sent`
    InvalidCode = "invalid-code",
    /// `just now`
    JustNow = "just-now",
    /// `{ $count } min ago`
    MinutesAgo = "minutes-ago",
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
    /// `{ $count } s ago`
    SecondsAgo = "seconds-ago",
    /// `Session ended`
    SessionEndedCode = "session-ended-code",
    /// `Sign in at { $host }`
    SignInAt = "sign-in-at",
    /// `Sign in with a CDR account`
    SignInBasicSubmit = "sign-in-basic-submit",
    /// `The viewer of the FerroEHR clinical data repository.`
    SignInCaption = "sign-in-caption",
    /// `The CDR did not answer. You can still try to sign in.`
    SignInCdrUnreachable = "sign-in-cdr-unreachable",
    /// `Your session ended. Sign in again to return to the page you were on.`
    SignInExpired = "sign-in-expired",
    /// `Not signed in: { $error }`
    SignInFailed = "sign-in-failed",
    /// `the identity provider`
    SignInIdentityProvider = "sign-in-identity-provider",
    /// `Taking you to { $host }…`
    SignInRedirecting = "sign-in-redirecting",
    /// `Not signed in: the CDR refused that username or password.`
    SignInRefused = "sign-in-refused",
    /// `The identity provider did not sign you in.`
    SignInRefusedProvider = "sign-in-refused-provider",
    /// `You are signed out.`
    SignInSignedOut = "sign-in-signed-out",
    /// `Sign in`
    SignInTitle = "sign-in-title",
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
    /// `The FerroEHR book`
    AboutBook = "about-book",
    /// `CDR`
    AboutCdr = "about-cdr",
    /// `FerroEHR { $version }`
    AboutCdrVersion = "about-cdr-version",
    /// `FerroUI`
    AboutFerroUi = "about-ferro-ui",
    /// `ferrohealth-ui { $version }`
    AboutFerroUiVersion = "about-ferro-ui-version",
    /// `Business Source License 1.1`
    AboutLicence = "about-licence",
    /// `The conformance manifest`
    AboutManifest = "about-manifest",
    /// `What the CDR answers at OPTIONS on its base path.`
    AboutManifestLine = "about-manifest-line",
    /// `The conformance manifest could not be read.`
    AboutManifestRefused = "about-manifest-refused",
    /// `openEHR REST`
    AboutOpenEhrRest = "about-open-ehr-rest",
    /// `ITS-REST { $version }`
    AboutRestVersion = "about-rest-version",
    /// `The viewer, the CDR it reads, and the manufacturer.`
    AboutSubtitle = "about-subtitle",
    /// `What decides`
    AccessDecides = "access-decides",
    /// `How`
    AccessHow = "access-how",
    /// `Basic sign-in with a CDR account`
    AccessHowBasic = "access-how-basic",
    /// `OIDC at the identity provider`
    AccessHowOidc = "access-how-oidc",
    /// `A Basic session carries no SMART scopes; the CDR checks the account's role.`
    AccessNoScopes = "access-no-scopes",
    /// `Read a scope`
    AccessPreview = "access-preview",
    /// `Paste a scope to read it here.`
    AccessPreviewEmpty = "access-preview-empty",
    /// `Paste a scope to see what it allows.`
    AccessPreviewIntro = "access-preview-intro",
    /// `Previewed scopes`
    AccessPreviewed = "access-previewed",
    /// `Read it`
    AccessReadIt = "access-read-it",
    /// `Scope or claim`
    AccessScopeField = "access-scope-field",
    /// `patient/composition-*.rs`
    AccessScopePlaceholder = "access-scope-placeholder",
    /// `Session`
    AccessSession = "access-session",
    /// `Ends after { $minutes } minutes without a request`
    AccessSessionEnds = "access-session-ends",
    /// `Signed in as`
    AccessSignedInAs = "access-signed-in-as",
    /// `What this session may do.`
    AccessSubtitle = "access-subtitle",
    /// `Who you are`
    AccessWho = "access-who",
    /// `Teal`
    BrandAccent = "brand-accent",
    /// `Matches`
    ColumnMatches = "column-matches",
    /// `Namespace`
    ColumnNamespace = "column-namespace",
    /// `Matches by namespace`
    DashboardByNamespace = "dashboard-by-namespace",
    /// `Create the first EHR`
    DashboardCreateFirstEhr = "dashboard-create-first-ehr",
    /// `{ $count ->`
    DashboardMatches = "dashboard-matches",
    /// `The stored queries could not be read.`
    DashboardNamespacesRefused = "dashboard-namespaces-refused",
    /// `The CDR did not answer.`
    DashboardNoAnswer = "dashboard-no-answer",
    /// `No composition is committed yet.`
    DashboardNoCompositions = "dashboard-no-compositions",
    /// `No stored query yet.`
    DashboardNoStoredQuery = "dashboard-no-stored-query",
    /// `No total`
    DashboardNoTotal = "dashboard-no-total",
    /// `Compositions per day`
    DashboardPerDay = "dashboard-per-day",
    /// `By commit time, over the newest compositions; read with one AQL query.`
    DashboardPerDayNote = "dashboard-per-day-note",
    /// `The compositions per day could not be read.`
    DashboardPerDayRefused = "dashboard-per-day-refused",
    /// `{ $total } compositions from { $first } to { $last }, at most { $most } on one day.`
    DashboardPerDaySummary = "dashboard-per-day-summary",
    /// `{ $count ->`
    DashboardQueries = "dashboard-queries",
    /// `The CDR did not answer, so the dashboard cannot be shown. Your session is kept.`
    DashboardRefused = "dashboard-refused",
    /// `What { $host } holds, as it answered just now.`
    DashboardSubtitle = "dashboard-subtitle",
    /// `Write one in Query`
    DashboardWriteOne = "dashboard-write-one",
    /// `How long a subject id stays shown after you reveal it; the viewer caps it at 2 minutes.`
    RevealDescription = "reveal-description",
    /// `Show a revealed identifier for`
    RevealLabel = "reveal-label",
    /// `{ $count ->`
    RevealMinutes = "reveal-minutes",
    /// `{ $count } seconds`
    RevealSeconds = "reveal-seconds",
    /// `Not a SMART on openEHR scope.`
    ScopeNotRecognised = "scope-not-recognised",
    /// `{ $family } { $pattern }`
    ScopeResource = "scope-resource",
    /// `Compositions, found in Query`
    StatCompositions = "stat-compositions",
    /// `EHRs`
    StatEhrs = "stat-ehrs",
    /// `Stored queries`
    StatStoredQueries = "stat-stored-queries",
    /// `Templates`
    StatTemplates = "stat-templates",
    /// `Try again`
    TryAgain = "try-again",
    /// `—`
    UnknownCount = "unknown-count",
    /// `Upload a template`
    UploadATemplate = "upload-a-template",
    /// `Your preferences`
    YourPreferences = "your-preferences",
    /// `The viewer's state is not in reach.`
    AboutNoState = "about-no-state",
    /// `Created`
    ColumnCreated = "column-created",
    /// `EHR`
    ColumnEhr = "column-ehr",
    /// `Add the reference`
    DirectoryAddReference = "directory-add-reference",
    /// `The CDR refused to create the directory.`
    DirectoryCreateRefused = "directory-create-refused",
    /// `Add, rename and remove folders, and add references to the EHR's compositions. Nothing reac`
    DirectoryEditIntro = "directory-edit-intro",
    /// `Saved as one new version.`
    DirectoryEditSubtitle = "directory-edit-subtitle",
    /// `Edit the directory`
    DirectoryEditTitle = "directory-edit-title",
    /// `Empty`
    DirectoryEmpty = "directory-empty",
    /// `{ $count ->`
    DirectoryFolders = "directory-folders",
    /// `{ $folders ->`
    DirectoryFoldersItems = "directory-folders-items",
    /// `Folders`
    DirectoryFoldersTitle = "directory-folders-title",
    /// `{ $count ->`
    DirectoryItems = "directory-items",
    /// `Keep this name`
    DirectoryKeepName = "directory-keep-name",
    /// `Overwrite`
    DirectoryOverwrite = "directory-overwrite",
    /// `Overwrite replaces the other change with yours.`
    DirectoryOverwriteNote = "directory-overwrite-note",
    /// `Composition`
    DirectoryReferenceLabel = "directory-reference-label",
    /// `Reload the latest version, or overwrite it.`
    DirectoryReloadOrOverwrite = "directory-reload-or-overwrite",
    /// `The CDR refused the directory.`
    DirectorySaveRefused = "directory-save-refused",
    /// `Saved as version { $version }.`
    DirectorySavedVersion = "directory-saved-version",
    /// `The document is not valid JSON.`
    EditorJson = "editor-json",
    /// `Not saved`
    EditorNotSaved = "editor-not-saved",
    /// `Opened at the latest version`
    EditorOpened = "editor-opened",
    /// `Reload the latest version before you save.`
    EditorReloadFirst = "editor-reload-first",
    /// `Reloading replaces your edits with the version on the server.`
    EditorReloadReplaces = "editor-reload-replaces",
    /// `Reload version { $version }`
    EditorReloadVersion = "editor-reload-version",
    /// `Save`
    EditorSave = "editor-save",
    /// `Save as version { $version }`
    EditorSaveAsVersion = "editor-save-as-version",
    /// `Edit`
    EditorTitle = "editor-title",
    /// `Back to EHRs`
    EhrBack = "ehr-back",
    /// `Commit several changes`
    EhrCommitSeveral = "ehr-commit-several",
    /// `Create the directory`
    EhrCreateDirectory = "ehr-create-directory",
    /// `Delete EHR…`
    EhrDelete = "ehr-delete",
    /// `Delete EHR`
    EhrDeleteConfirm = "ehr-delete-confirm",
    /// `Type the first 8 characters of the EHR id to confirm`
    EhrDeleteConfirmLabel = "ehr-delete-confirm-label",
    /// `The EHR and every composition, directory and status version in it are removed from the CDR`
    EhrDeleteMessage = "ehr-delete-message",
    /// `The characters you typed are not the start of this EHR id.`
    EhrDeleteMismatch = "ehr-delete-mismatch",
    /// `The CDR refused to delete the EHR.`
    EhrDeleteRefused = "ehr-delete-refused",
    /// `Delete EHR { $id }?`
    EhrDeleteTitle = "ehr-delete-title",
    /// `More actions`
    EhrMore = "ehr-more",
    /// `Create EHR`
    EhrNewCreate = "ehr-new-create",
    /// `EHR id`
    EhrNewEhrId = "ehr-new-ehr-id",
    /// `Leave it empty and the CDR mints a UUID.`
    EhrNewEhrIdHelp = "ehr-new-ehr-id-help",
    /// `UUID, or empty`
    EhrNewEhrIdPlaceholder = "ehr-new-ehr-id-placeholder",
    /// `An EHR with this id exists already.`
    EhrNewInUse = "ehr-new-in-use",
    /// `Namespace`
    EhrNewNamespace = "ehr-new-namespace",
    /// `EHR not created`
    EhrNewNotCreated = "ehr-new-not-created",
    /// `An EHR id is a UUID, such as 7d44aa01-0f9e-4a2c-9a0f-2a6a5f9b1c3d.`
    EhrNewNotUuid = "ehr-new-not-uuid",
    /// `Not created yet`
    EhrNewNotYet = "ehr-new-not-yet",
    /// `The CDR refused to create the EHR.`
    EhrNewRefused = "ehr-new-refused",
    /// `Subject`
    EhrNewSubject = "ehr-new-subject",
    /// `Subject id`
    EhrNewSubjectId = "ehr-new-subject-id",
    /// `The person this EHR is about, as an id the CDR keeps without reading it.`
    EhrNewSubjectIntro = "ehr-new-subject-intro",
    /// `An EHR with its status; the subject is optional.`
    EhrNewSubtitle = "ehr-new-subtitle",
    /// `No EHR { $id }`
    EhrNotFoundTitle = "ehr-not-found-title",
    /// `Open EHR { $id }`
    EhrOpen = "ehr-open",
    /// `The EHR could not be read.`
    EhrRefused = "ehr-refused",
    /// `In { $system }`
    EhrSubtitle = "ehr-subtitle",
    /// `Compositions`
    EhrTabCompositions = "ehr-tab-compositions",
    /// `Contributions`
    EhrTabContributions = "ehr-tab-contributions",
    /// `Directory`
    EhrTabDirectory = "ehr-tab-directory",
    /// `Status`
    EhrTabStatus = "ehr-tab-status",
    /// `Tags`
    EhrTabTags = "ehr-tab-tags",
    /// `EHR { $id }`
    EhrTitle = "ehr-title",
    /// `Create the first EHR`
    EhrsCreateFirst = "ehrs-create-first",
    /// `The CDR holds no EHR yet.`
    EhrsEmpty = "ehrs-empty",
    /// `Subject id`
    EhrsFindLabel = "ehrs-find-label",
    /// `The subject search could not be finished.`
    EhrsFindRefused = "ehrs-find-refused",
    /// `Namespace`
    EhrsNamespace = "ehrs-namespace",
    /// `No EHR belongs to this subject.`
    EhrsNoMatch = "ehrs-no-match",
    /// `EHR pages`
    EhrsPages = "ehrs-pages",
    /// `The EHRs could not be read.`
    EhrsRefused = "ehrs-refused",
    /// `Subject: { $subject }`
    EhrsSubjectChip = "ehrs-subject-chip",
    /// `A subject search needs the namespace the subject id belongs to.`
    EhrsSubjectNeedsNamespace = "ehrs-subject-needs-namespace",
    /// `Every EHR the CDR holds, newest first.`
    EhrsSubtitle = "ehrs-subtitle",
    /// `Contribution`
    FactContribution = "fact-contribution",
    /// `Lifecycle`
    FactLifecycle = "fact-lifecycle",
    /// `Committed`
    FactSigned = "fact-signed",
    /// `Version`
    FactVersion = "fact-version",
    /// `In EHR { $ehr }`
    FindCompositionLine = "find-composition-line",
    /// `Compositions`
    FindCompositions = "find-compositions",
    /// `EHR { $id }`
    FindEhr = "find-ehr",
    /// `Created { $created }`
    FindEhrLine = "find-ehr-line",
    /// `Type an id, a subject, a template or a query name in the search field above.`
    FindEmpty = "find-empty",
    /// `{ $count ->`
    FindFoundSome = "find-found-some",
    /// `{ $count ->`
    FindMatches = "find-matches",
    /// `No EHR, composition or party has the id { $id }.`
    FindNoIdHolds = "find-no-id-holds",
    /// `Nothing matches “{ $typed }”.`
    FindNothingFor = "find-nothing-for",
    /// `Check the spelling, or open the list of the kind of record you look for.`
    FindNothingLine = "find-nothing-line",
    /// `Nothing found`
    FindNothingTitle = "find-nothing-title",
    /// `Other ways to find a record`
    FindOtherWays = "find-other-ways",
    /// `{ $kind }`
    FindPartyLine = "find-party-line",
    /// `Stored query, version { $version }`
    FindQueryLine = "find-query-line",
    /// `One answer for an id, a subject, a template or a stored query.`
    FindSubtitle = "find-subtitle",
    /// `Template: { $concept }`
    FindTemplateLine = "find-template-line",
    /// `Find`
    FindTitle = "find-title",
    /// `The CDR refused the search over { $kinds }; the answer may be incomplete.`
    FindUnfinishedLine = "find-unfinished-line",
    /// `Part of the search did not finish`
    FindUnfinishedTitle = "find-unfinished-title",
    /// `EHRs`
    FindWayEhr = "find-way-ehr",
    /// `Every EHR, with the subject search.`
    FindWayEhrLine = "find-way-ehr-line",
    /// `Parties`
    FindWayParty = "find-way-party",
    /// `A party by its id or its tags.`
    FindWayPartyLine = "find-way-party-line",
    /// `By subject`
    FindWaySubject = "find-way-subject",
    /// `An EHR by its subject id and namespace.`
    FindWaySubjectLine = "find-way-subject-line",
    /// `No`
    No = "no",
    /// `Not recorded`
    NoneShown = "none-shown",
    /// `Opens the version that was current at that moment.`
    StatusAtTimeOpens = "status-at-time-opens",
    /// `Capabilities`
    StatusCapabilities = "status-capabilities",
    /// `Document`
    StatusDocument = "status-document",
    /// `The status as the CDR holds it.`
    StatusDocumentIntro = "status-document-intro",
    /// `EHR_STATUS, version { $version }`
    StatusDocumentTitle = "status-document-title",
    /// `Edit status`
    StatusEdit = "status-edit",
    /// `Saved as one new version.`
    StatusEditSubtitle = "status-edit-subtitle",
    /// `Edit the EHR status`
    StatusEditTitle = "status-edit-title",
    /// `The status history could not be read.`
    StatusHistoryRefused = "status-history-refused",
    /// `Clients may commit to this EHR.`
    StatusModifiableHelp = "status-modifiable-help",
    /// `Not queryable: AQL over the population leaves this EHR out.`
    StatusNotQueryable = "status-not-queryable",
    /// `Other details`
    StatusOtherDetails = "status-other-details",
    /// `The ITEM_STRUCTURE the status carries beside its flags.`
    StatusOtherDetailsIntro = "status-other-details-intro",
    /// `AQL over the population includes this EHR.`
    StatusQueryableHelp = "status-queryable-help",
    /// `The EHR status could not be read.`
    StatusRefused = "status-refused",
    /// `The CDR refused the status.`
    StatusSaveRefused = "status-save-refused",
    /// `Status saved`
    StatusSaved = "status-saved",
    /// `Saved as a new version.`
    StatusSavedLine = "status-saved-line",
    /// `Saved as version { $version }.`
    StatusSavedVersion = "status-saved-version",
    /// `This version`
    StatusThisVersion = "status-this-version",
    /// `Status of EHR { $id }`
    StatusTitle = "status-title",
    /// `This version could not be read.`
    StatusVersionRefused = "status-version-refused",
    /// `Versions`
    StatusVersions = "status-versions",
    /// `Yes`
    Yes = "yes",
    /// `Composer: { $value }`
    ChipComposer = "chip-composer",
    /// `From { $value }`
    ChipFrom = "chip-from",
    /// `Template: { $value }`
    ChipTemplate = "chip-template",
    /// `To { $value }`
    ChipTo = "chip-to",
    /// `Composer`
    ColumnComposer = "column-composer",
    /// `Composition`
    ColumnComposition = "column-composition",
    /// `Start time`
    ColumnStartTime = "column-start-time",
    /// `Version`
    ColumnVersion = "column-version",
    /// `This EHR holds no composition yet.`
    CompositionsEmpty = "compositions-empty",
    /// `No composition matches these filters.`
    CompositionsNoMatch = "compositions-no-match",
    /// `Composition pages`
    CompositionsPages = "compositions-pages",
    /// `The compositions could not be read.`
    CompositionsRefused = "compositions-refused",
    /// `Opens the version that was current at that moment.`
    DirectoryAtTimeOpens = "directory-at-time-opens",
    /// `Changed, not saved`
    DirectoryChanged = "directory-changed",
    /// `Delete the directory…`
    DirectoryDelete = "directory-delete",
    /// `Delete the directory`
    DirectoryDeleteConfirm = "directory-delete-confirm",
    /// `The directory is marked deleted; its versions stay in the history and the compositions it `
    DirectoryDeleteMessage = "directory-delete-message",
    /// `The CDR refused to delete the directory.`
    DirectoryDeleteRefused = "directory-delete-refused",
    /// `Delete the directory?`
    DirectoryDeleteTitle = "directory-delete-title",
    /// `Edit`
    DirectoryEdit = "directory-edit",
    /// `The directory's versions could not be read.`
    DirectoryHistoryRefused = "directory-history-refused",
    /// `More directory actions`
    DirectoryMore = "directory-more",
    /// `New folder`
    DirectoryNewFolder = "directory-new-folder",
    /// `The directory has no folder at { $path }; the whole tree is shown.`
    DirectoryNoPath = "directory-no-path",
    /// `This EHR has no directory. Create it from the EHR's actions.`
    DirectoryNone = "directory-none",
    /// `The EHR had no directory at that moment.`
    DirectoryNoneAtTime = "directory-none-at-time",
    /// `episodes/2026`
    DirectoryPathPlaceholder = "directory-path-placeholder",
    /// `The folder at this path could not be read.`
    DirectoryPathRefused = "directory-path-refused",
    /// `The directory could not be read.`
    DirectoryRefused = "directory-refused",
    /// `Reload the directory`
    DirectoryReload = "directory-reload",
    /// `Restore…`
    DirectoryRestore = "directory-restore",
    /// `Restore as version { $next }`
    DirectoryRestoreAs = "directory-restore-as",
    /// `The tree of version { $version } is written back as version { $next }. The versions in bet`
    DirectoryRestoreMessage = "directory-restore-message",
    /// `The CDR refused to restore this version.`
    DirectoryRestoreRefused = "directory-restore-refused",
    /// `Restore version { $version }?`
    DirectoryRestoreTitle = "directory-restore-title",
    /// `Restore version { $version }`
    DirectoryRestoreVersion = "directory-restore-version",
    /// `Directory saved`
    DirectorySaved = "directory-saved",
    /// `Show`
    DirectoryShow = "directory-show",
    /// `Show one folder by path`
    DirectoryShowPath = "directory-show-path",
    /// `Showing the folder at { $path }.`
    DirectoryShowingPath = "directory-showing-path",
    /// `Directory of EHR { $id }`
    DirectoryTitle = "directory-title",
    /// `The directory's folders`
    DirectoryTreeLabel = "directory-tree-label",
    /// `(no name)`
    DirectoryUnnamed = "directory-unnamed",
    /// `{ $folders ->`
    DirectoryVersionHolds = "directory-version-holds",
    /// `Versions`
    DirectoryVersions = "directory-versions",
    /// `Every version of the directory; restoring one writes it back as a new version.`
    DirectoryVersionsIntro = "directory-versions-intro",
    /// `The document is not valid UTF-8.`
    EditorUtf8 = "editor-utf-8",
    /// `Apply`
    FilterApply = "filter-apply",
    /// `Composer`
    FilterComposer = "filter-composer",
    /// `From`
    FilterFrom = "filter-from",
    /// `Start time`
    FilterStartTime = "filter-start-time",
    /// `Template`
    FilterTemplate = "filter-template",
    /// `To`
    FilterTo = "filter-to",
    /// `This EHR has no subject.`
    RevealNoSubject = "reveal-no-subject",
    /// `Amendment`
    ChangeAmendment = "change-amendment",
    /// `Attestation`
    ChangeAttestation = "change-attestation",
    /// `Creation`
    ChangeCreation = "change-creation",
    /// `Deleted`
    ChangeDeleted = "change-deleted",
    /// `Modification`
    ChangeModification = "change-modification",
    /// `Synthesis`
    ChangeSynthesis = "change-synthesis",
    /// `Key: { $value }`
    ChipKey = "chip-key",
    /// `Value: { $value }`
    ChipValue = "chip-value",
    /// `Change`
    ColumnChange = "column-change",
    /// `Committed`
    ColumnCommitted = "column-committed",
    /// `Committer`
    ColumnCommitter = "column-committer",
    /// `Contribution`
    ColumnContribution = "column-contribution",
    /// `Kind`
    ColumnKind = "column-kind",
    /// `Object`
    ColumnObject = "column-object",
    /// `The templates or compositions to pick from could not be read.`
    ContributionChoicesRefused = "contribution-choices-refused",
    /// `{ $count ->`
    ContributionCommit = "contribution-commit",
    /// `The CDR refused the contribution, so nothing was committed.`
    ContributionCommitRefused = "contribution-commit-refused",
    /// `Leave empty to commit as yourself`
    ContributionCommitterPlaceholder = "contribution-committer-placeholder",
    /// `The composition to amend could not be read.`
    ContributionCompositionRefused = "contribution-composition-refused",
    /// `{ $count ->`
    ContributionMadeVersions = "contribution-made-versions",
    /// `Several changes, committed together: all of them or none.`
    ContributionNewSubtitle = "contribution-new-subtitle",
    /// `{ $mode ->`
    ContributionNewTitle = "contribution-new-title",
    /// `Not committed`
    ContributionNotCommitted = "contribution-not-committed",
    /// `Record`
    ContributionRecord = "contribution-record",
    /// `The contribution could not be read.`
    ContributionRefused = "contribution-refused",
    /// `One change set, committed together.`
    ContributionSubtitle = "contribution-subtitle",
    /// `Contribution { $id }`
    ContributionTitle = "contribution-title",
    /// `The contributions per day could not be read.`
    ContributionsActivityRefused = "contributions-activity-refused",
    /// `This EHR has no contribution yet.`
    ContributionsEmpty = "contributions-empty",
    /// `Contribution pages`
    ContributionsPages = "contributions-pages",
    /// `Contributions per day`
    ContributionsPerDay = "contributions-per-day",
    /// `Contributions per day, { $first } to { $last }: { $total } contributions, at most { $most `
    ContributionsPerDaySummary = "contributions-per-day-summary",
    /// `Contributions per day, last 30 days`
    ContributionsPerDayTitle = "contributions-per-day-title",
    /// `The contributions could not be read.`
    ContributionsRefused = "contributions-refused",
    /// `Contributions to EHR { $id }`
    ContributionsTitle = "contributions-title",
    /// `Change`
    FactChange = "fact-change",
    /// `Committed`
    FactCommitted = "fact-committed",
    /// `Committer`
    FactCommitter = "fact-committer",
    /// `Description`
    FactDescription = "fact-description",
    /// `System`
    FactSystem = "fact-system",
    /// `An amended composition`
    StagedAmend = "staged-amend",
    /// `Apply`
    StagedApply = "staged-apply",
    /// `Composition`
    StagedComposition = "staged-composition",
    /// `Its latest version is the one the change supersedes.`
    StagedCompositionHelp = "staged-composition-help",
    /// `{ $count ->`
    StagedCount = "staged-count",
    /// `A new composition`
    StagedCreate = "staged-create",
    /// `The document of change { $number }`
    StagedDocument = "staged-document",
    /// `The one the document names`
    StagedFromDocument = "staged-from-document",
    /// `Pick a composition`
    StagedPick = "staged-pick",
    /// `Recorded as the audit change type { $change }.`
    StagedRecordedAs = "staged-recorded-as",
    /// `Remove change { $number }`
    StagedRemove = "staged-remove",
    /// `Stage a change`
    StagedStage = "staged-stage",
    /// `The EHR status`
    StagedStatus = "staged-status",
    /// `Loaded from version { $version }, which it supersedes.`
    StagedStatusFrom = "staged-status-from",
    /// `{ $number }. { $what }`
    StagedTitle = "staged-title",
    /// `What changes`
    StagedWhat = "staged-what",
    /// `Tags in this EHR`
    TagsCaption = "tags-caption",
    /// `Composition { $id }`
    TagsComposition = "tags-composition",
    /// `EHR status`
    TagsEhrStatus = "tags-ehr-status",
    /// `No object in this EHR carries a tag.`
    TagsEmpty = "tags-empty",
    /// `A tag is set on its object, on that object's Tags tab; this list reads them all.`
    TagsIntro = "tags-intro",
    /// `Key`
    TagsKey = "tags-key",
    /// `No tag matches these filters.`
    TagsNoMatch = "tags-no-match",
    /// `version { $version }`
    TagsOneVersion = "tags-one-version",
    /// `The tags could not be read.`
    TagsRefused = "tags-refused",
    /// `Target path`
    TagsTargetPath = "tags-target-path",
    /// `Tags in EHR { $id }`
    TagsTitle = "tags-title",
    /// `an object the CDR did not name`
    TagsUnknownObject = "tags-unknown-object",
    /// `Value`
    TagsValue = "tags-value",
    /// `versioned composition`
    TagsVersionedComposition = "tags-versioned-composition",
    /// `versioned directory`
    TagsVersionedDirectory = "tags-versioned-directory",
    /// `versioned EHR status`
    TagsVersionedStatus = "tags-versioned-status",
    /// `the whole object`
    TagsWholeObject = "tags-whole-object",
    /// `Opens the version that was current at that moment.`
    CompositionAtTimeOpens = "composition-at-time-opens",
    /// `Commit this composition`
    CompositionCommit = "composition-commit",
    /// `The CDR refused the document, so nothing was committed.`
    CompositionCommitRefused = "composition-commit-refused",
    /// `Context`
    CompositionContext = "composition-context",
    /// `The CDR converts; the viewer shows its answer as sent.`
    CompositionConverts = "composition-converts",
    /// `Delete composition…`
    CompositionDelete = "composition-delete",
    /// `Delete the composition`
    CompositionDeleteConfirm = "composition-delete-confirm",
    /// `{ $count ->`
    CompositionDeleteMessage = "composition-delete-message",
    /// `The CDR refused to delete the composition.`
    CompositionDeleteRefused = "composition-delete-refused",
    /// `Delete the composition { $name }?`
    CompositionDeleteTitle = "composition-delete-title",
    /// `{ $template } · deleted { $when } by { $who } as version { $version }.`
    CompositionDeletedLine = "composition-deleted-line",
    /// `Deleted`
    CompositionDeletedMark = "composition-deleted-mark",
    /// `This composition is deleted; there is no version to edit.`
    CompositionDeletedNoEdit = "composition-deleted-no-edit",
    /// `This version is the deletion; it holds no document.`
    CompositionDeletedVersion = "composition-deleted-version",
    /// `Document`
    CompositionDocument = "composition-document",
    /// `The CDR could not send the document in this format.`
    CompositionDocumentRefused = "composition-document-refused",
    /// `COMPOSITION · { $format }`
    CompositionDocumentTitle = "composition-document-title",
    /// `Edit as a new version`
    CompositionEdit = "composition-edit",
    /// `Edit the latest`
    CompositionEditLatest = "composition-edit-latest",
    /// `Version { $version }, opened now.`
    CompositionEditSubtitle = "composition-edit-subtitle",
    /// `Edit { $name }`
    CompositionEditTitle = "composition-edit-title",
    /// `The composition, canonical JSON`
    CompositionEditorLabel = "composition-editor-label",
    /// `First committed`
    CompositionFirstCommitted = "composition-first-committed",
    /// `None: this is the first version`
    CompositionFirstVersion = "composition-first-version",
    /// `Format`
    CompositionFormat = "composition-format",
    /// `The versions could not be read.`
    CompositionHistoryRefused = "composition-history-refused",
    /// `Holds content`
    CompositionHoldsContent = "composition-holds-content",
    /// `You are reading version { $version }, the last before the deletion; it is read-only.`
    CompositionLastBeforeDeletion = "composition-last-before-deletion",
    /// `{ $template } · version { $version } · started { $started } by { $composer }.`
    CompositionLine = "composition-line",
    /// `{ $template } · started { $started } by { $composer }.`
    CompositionLineOlder = "composition-line-older",
    /// `More actions for the composition`
    CompositionMore = "composition-more",
    /// `The new composition`
    CompositionNewEditorLabel = "composition-new-editor-label",
    /// `New composition`
    CompositionNewPageTitle = "composition-new-page-title",
    /// `A document committed as version 1 under a template.`
    CompositionNewSubtitle = "composition-new-subtitle",
    /// `{ $mode ->`
    CompositionNewTitle = "composition-new-title",
    /// `This document is not a canonical composition the reading understands; the Document tab sho`
    CompositionNoReading = "composition-no-reading",
    /// `Not committed yet.`
    CompositionNotYet = "composition-not-yet",
    /// `Preceding version`
    CompositionPreceding = "composition-preceding",
    /// `Read version { $version }`
    CompositionReadVersion = "composition-read-version",
    /// `Reading`
    CompositionReading = "composition-reading",
    /// `The composition could not be read.`
    CompositionRefused = "composition-refused",
    /// `The CDR refused the document, so no version was saved.`
    CompositionSaveRefused = "composition-save-refused",
    /// `Composition saved`
    CompositionSaved = "composition-saved",
    /// `Saved as version { $version }.`
    CompositionSavedVersion = "composition-saved-version",
    /// `Show the full document`
    CompositionShowDocument = "composition-show-document",
    /// `The template the document is committed against; FLAT needs it.`
    CompositionTemplateHelp = "composition-template-help",
    /// `The templates could not be read.`
    CompositionTemplatesRefused = "composition-templates-refused",
    /// `Composition { $id }`
    CompositionTitle = "composition-title",
    /// `Version { $version }`
    CompositionVersionN = "composition-version-n",
    /// `This version could not be read.`
    CompositionVersionRefused = "composition-version-refused",
    /// `The versioned composition`
    CompositionVersioned = "composition-versioned",
    /// `Versioned object`
    CompositionVersionedObject = "composition-versioned-object",
    /// `Versions`
    CompositionVersions = "composition-versions",
    /// `Views of this composition`
    CompositionViews = "composition-views",
    /// `{ $count ->`
    EditorRefusedHeading = "editor-refused-heading",
    /// `XML`
    EditorXml = "editor-xml",
    /// `composition`
    TagCollectionComposition = "tag-collection-composition",
    /// `{ $count ->`
    TagCount = "tag-count",
    /// `Tags stay on the { $object } across its versions; setting one commits no version.`
    TagIntro = "tag-intro",
    /// `The { $object } carries no tag.`
    TagNone = "tag-none",
    /// `Target path (optional)`
    TagPathOptional = "tag-path-optional",
    /// `The CDR refused the tag.`
    TagRefused = "tag-refused",
    /// `Remove the tag { $key }`
    TagRemove = "tag-remove",
    /// `Remove`
    TagRemoveColumn = "tag-remove-column",
    /// `Tag removed`
    TagRemoved = "tag-removed",
    /// `{ $key } is gone from { $object }.`
    TagRemovedLine = "tag-removed-line",
    /// `Tag set`
    TagSet = "tag-set",
    /// `Set the tag`
    TagSetAction = "tag-set-action",
    /// `{ $key } is on { $object }.`
    TagSetLine = "tag-set-line",
    /// `Set a tag`
    TagSetTitle = "tag-set-title",
    /// `Value (optional)`
    TagValueOptional = "tag-value-optional",
    /// `the whole { $object }`
    TagWhole = "tag-whole",
    /// `Tags of the { $object }`
    TagsOf = "tags-of",
    /// `Not committed`
    CompositionNotCommitted = "composition-not-committed",
    /// `Agent`
    KindAgent = "kind-agent",
    /// `Group`
    KindGroup = "kind-group",
    /// `Organisation`
    KindOrganisation = "kind-organisation",
    /// `Person`
    KindPerson = "kind-person",
    /// `Role`
    KindRole = "kind-role",
    /// `New party`
    PartiesNew = "parties-new",
    /// `No { $kind } and no other kind of party has this uid.`
    PartiesNoKindHolds = "parties-no-kind-holds",
    /// `No party carries a tag yet.`
    PartiesNoTags = "parties-no-tags",
    /// `No party has this uid.`
    PartiesNoneHolds = "parties-none-holds",
    /// `Parties follow the released RM 1.1.0 demographic model; the REST API for them is still in `
    PartiesNote = "parties-note",
    /// `Open`
    PartiesOpen = "parties-open",
    /// `The party could not be opened.`
    PartiesOpenRefused = "parties-open-refused",
    /// `People, organisations, groups, agents and roles, opened by their id.`
    PartiesSubtitle = "parties-subtitle",
    /// `The demographic tag index`
    PartiesTagIndex = "parties-tag-index",
    /// `On`
    PartiesTagOn = "parties-tag-on",
    /// `The tags on parties could not be read.`
    PartiesTagsRefused = "parties-tags-refused",
    /// `Tags on parties`
    PartiesTagsTitle = "parties-tags-title",
    /// `Party uid`
    PartiesUid = "parties-uid",
    /// `Archetype`
    PartyArchetype = "party-archetype",
    /// `Opens the party as it stood then.`
    PartyAtTimeOpens = "party-at-time-opens",
    /// `Create this party`
    PartyCreate = "party-create",
    /// `The CDR refused the party, so nothing was created.`
    PartyCreateRefused = "party-create-refused",
    /// `Current version`
    PartyCurrentVersion = "party-current-version",
    /// `Delete party…`
    PartyDelete = "party-delete",
    /// `Delete the party`
    PartyDeleteConfirm = "party-delete-confirm",
    /// `The party is marked deleted. { $count ->`
    PartyDeleteMessage = "party-delete-message",
    /// `The CDR refused to delete the party.`
    PartyDeleteRefused = "party-delete-refused",
    /// `Delete { $name }?`
    PartyDeleteTitle = "party-delete-title",
    /// `The party is deleted; its versions stay readable, and its relationships stay.`
    PartyDeletedIntro = "party-deleted-intro",
    /// `{ $kind } · deleted { $when } by { $who } as version { $version }`
    PartyDeletedLine = "party-deleted-line",
    /// `This party is deleted; there is no version to edit.`
    PartyDeletedNoEdit = "party-deleted-no-edit",
    /// `{ $type } · version { $version }`
    PartyDocumentTitle = "party-document-title",
    /// `Edit`
    PartyEdit = "party-edit",
    /// `The whole party as JSON, as New party takes it; it must keep at least one identity.`
    PartyEditIntro = "party-edit-intro",
    /// `The party`
    PartyEditorLabel = "party-editor-label",
    /// `Facts`
    PartyFacts = "party-facts",
    /// `Identities`
    PartyIdentities = "party-identities",
    /// `Start over puts this kind's starting document in the editor.`
    PartyKindHelp = "party-kind-help",
    /// `{ $kind } · version { $version } · { $id }`
    PartyLine = "party-line",
    /// `More actions for the party`
    PartyMore = "party-more",
    /// `The new party`
    PartyNewEditorLabel = "party-new-editor-label",
    /// `A starting document for a { $kind }.`
    PartyNewSubtitle = "party-new-subtitle",
    /// `{ $mode ->`
    PartyNewTitle = "party-new-title",
    /// `This party carries no identity the reading understands.`
    PartyNoIdentities = "party-no-identities",
    /// `This party is the source of no relationship.`
    PartyNoRelationships = "party-no-relationships",
    /// `Not created.`
    PartyNotCreated = "party-not-created",
    /// `Not created yet.`
    PartyNotYet = "party-not-yet",
    /// `Overview`
    PartyOverview = "party-overview",
    /// `The party could not be read.`
    PartyRefused = "party-refused",
    /// `Relate this party`
    PartyRelate = "party-relate",
    /// `Relationships from this party`
    PartyRelationshipsFrom = "party-relationships-from",
    /// `The relationships this party is the source of; the CDR cannot list the ones that point at `
    PartyRelationshipsIntro = "party-relationships-intro",
    /// `The CDR refused the party, so no version was saved.`
    PartySaveRefused = "party-save-refused",
    /// `Party saved`
    PartySaved = "party-saved",
    /// `Start over`
    PartyStartOver = "party-start-over",
    /// `Party { $id }`
    PartyTitle = "party-title",
    /// `The party at version { $version }`
    PartyVersionDocument = "party-version-document",
    /// `Create this relationship`
    RelationshipCreate = "relationship-create",
    /// `The CDR refused the relationship, so nothing was created.`
    RelationshipCreateRefused = "relationship-create-refused",
    /// `Delete relationship…`
    RelationshipDelete = "relationship-delete",
    /// `Delete the relationship`
    RelationshipDeleteConfirm = "relationship-delete-confirm",
    /// `The relationship is marked deleted; both parties stay, and its versions stay readable.`
    RelationshipDeleteMessage = "relationship-delete-message",
    /// `The CDR refused to delete the relationship.`
    RelationshipDeleteRefused = "relationship-delete-refused",
    /// `Delete the relationship { $name }?`
    RelationshipDeleteTitle = "relationship-delete-title",
    /// `Relationship deleted`
    RelationshipDeletedToast = "relationship-deleted-toast",
    /// `Details`
    RelationshipDetails = "relationship-details",
    /// `Optional extra details, as JSON.`
    RelationshipDetailsIntro = "relationship-details-intro",
    /// `Ends`
    RelationshipEnds = "relationship-ends",
    /// `The two ends are fixed; other parties make another relationship.`
    RelationshipEndsIntro = "relationship-ends-intro",
    /// `From`
    RelationshipFrom = "relationship-from",
    /// `Relationship · version { $version } · { $id }`
    RelationshipLine = "relationship-line",
    /// `More actions for the relationship`
    RelationshipMore = "relationship-more",
    /// `New relationship`
    RelationshipNewPageTitle = "relationship-new-page-title",
    /// `From this party to another.`
    RelationshipNewSubtitle = "relationship-new-subtitle",
    /// `{ $mode ->`
    RelationshipNewTitle = "relationship-new-title",
    /// `none`
    RelationshipNone = "relationship-none",
    /// `The relationship could not be read.`
    RelationshipRefused = "relationship-refused",
    /// `The CDR refused the relationship, so no version was saved.`
    RelationshipSaveRefused = "relationship-save-refused",
    /// `Relationship saved`
    RelationshipSaved = "relationship-saved",
    /// `Source`
    RelationshipSource = "relationship-source",
    /// `Target`
    RelationshipTarget = "relationship-target",
    /// `Target kind`
    RelationshipTargetKind = "relationship-target-kind",
    /// `Target party uid`
    RelationshipTargetUid = "relationship-target-uid",
    /// `Relationship`
    RelationshipTitle = "relationship-title",
    /// `The type as a key, for example employment.`
    RelationshipTypeHelp = "relationship-type-help",
    /// `Time validity`
    RelationshipValidity = "relationship-validity",
    /// `party`
    TagCollectionParty = "tag-collection-party",
    /// `Contains: { $value }`
    ChipContains = "chip-contains",
    /// `Actions`
    ColumnActions = "column-actions",
    /// `Root archetype`
    ColumnRootArchetype = "column-root-archetype",
    /// `Template`
    ColumnTemplate = "column-template",
    /// `Uploaded`
    ColumnUploaded = "column-uploaded",
    /// `Actions for { $name }`
    TemplateActionsFor = "template-actions-for",
    /// `ADL 1.4`
    TemplateAdl14Line = "template-adl-14-line",
    /// `{ $concept } · ADL 1.4`
    TemplateAdl14Subtitle = "template-adl-14-subtitle",
    /// `The viewer builds these paths itself: the CDR has no web template for ADL 2.`
    TemplateAdl2Paths = "template-adl-2-paths",
    /// `template (adl_version=2.4.0; rm_release=1.1.0)`
    TemplateAdl2Placeholder = "template-adl-2-placeholder",
    /// `The ADL 2 source`
    TemplateAdl2Source = "template-adl-2-source",
    /// `{ $hrid } · ADL 2`
    TemplateAdl2Subtitle = "template-adl-2-subtitle",
    /// `Build a query`
    TemplateBuildQuery = "template-build-query",
    /// `Choose a file`
    TemplateChooseFile = "template-choose-file",
    /// `Concept`
    TemplateConcept = "template-concept",
    /// `Default language`
    TemplateDefaultLanguage = "template-default-language",
    /// `Delete…`
    TemplateDelete = "template-delete",
    /// `Delete the template`
    TemplateDeleteConfirm = "template-delete-confirm",
    /// `The CDR refuses while compositions use the template; delete or migrate them first.`
    TemplateDeleteMessage = "template-delete-message",
    /// `The CDR refused to delete the template.`
    TemplateDeleteRefused = "template-delete-refused",
    /// `Delete the template { $name }?`
    TemplateDeleteTitle = "template-delete-title",
    /// `Example`
    TemplateExample = "template-example",
    /// `Detail level`
    TemplateExampleDetail = "template-example-detail",
    /// `The CDR could not build an example for these options.`
    TemplateExampleRefused = "template-example-refused",
    /// `Example · { $format }`
    TemplateExampleTitle = "template-example-title",
    /// `Type`
    TemplateExampleType = "template-example-type",
    /// `Identity`
    TemplateIdentity = "template-identity",
    /// `JSON`
    TemplateJson = "template-json",
    /// `JSON · release { $release }`
    TemplateJsonTitle = "template-json-title",
    /// `Languages`
    TemplateLanguages = "template-languages",
    /// `More actions for the template`
    TemplateMore = "template-more",
    /// `The CDR sent no operational template to build paths from.`
    TemplateNoCatalog = "template-no-catalog",
    /// `The CDR built no example for these options.`
    TemplateNoExample = "template-no-example",
    /// `The CDR holds no source for this release.`
    TemplateNoSource = "template-no-source",
    /// `Node`
    TemplateNode = "template-node",
    /// `Not uploaded.`
    TemplateNotUploaded = "template-not-uploaded",
    /// `Not uploaded yet.`
    TemplateNotYet = "template-not-yet",
    /// `<?xml version="1.0" encoding="UTF-8"?>`
    TemplateOptPlaceholder = "template-opt-placeholder",
    /// `The operational template`
    TemplateOptSource = "template-opt-source",
    /// `Paths`
    TemplatePaths = "template-paths",
    /// `The paths of { $name }`
    TemplatePathsOf = "template-paths-of",
    /// `Pick a node in the tree to read its path, type and codes.`
    TemplatePickNode = "template-pick-node",
    /// `Read { $release }, the latest`
    TemplateReadLatest = "template-read-latest",
    /// `You are reading release { $release }.`
    TemplateReadingRelease = "template-reading-release",
    /// `The template could not be read.`
    TemplateRefused = "template-refused",
    /// `Release`
    TemplateRelease = "template-release",
    /// `{ $count ->`
    TemplateReleaseCount = "template-release-count",
    /// `Releases`
    TemplateReleases = "template-releases",
    /// `Show the example`
    TemplateShowExample = "template-show-example",
    /// `Source`
    TemplateSourceTitle = "template-source-title",
    /// `UID`
    TemplateUid = "template-uid",
    /// `Upload this template`
    TemplateUpload = "template-upload",
    /// `The CDR refused the template, so nothing was uploaded.`
    TemplateUploadRefused = "template-upload-refused",
    /// `Choose a file or paste the source; the CDR validates it on upload.`
    TemplateUploadSubtitle = "template-upload-subtitle",
    /// `{ $mode ->`
    TemplateUploadTitle = "template-upload-title",
    /// `Use`
    TemplateUse = "template-use",
    /// `Query the compositions committed under it`
    TemplateUseLine = "template-use-line",
    /// `Template version`
    TemplateVersion = "template-version",
    /// `No template is uploaded yet.`
    TemplatesEmpty = "templates-empty",
    /// `Template families`
    TemplatesFamilies = "templates-families",
    /// `Family`
    TemplatesFamily = "templates-family",
    /// `Id, concept or archetype`
    TemplatesFind = "templates-find",
    /// `No template matches.`
    TemplatesNoMatch = "templates-no-match",
    /// `The templates could not be read.`
    TemplatesRefused = "templates-refused",
    /// `The operational templates the CDR validates compositions against.`
    TemplatesSubtitle = "templates-subtitle",
    /// `Upload a template`
    TemplatesUpload = "templates-upload",
    /// `Broader code`
    TermBroader = "term-broader",
    /// `Defined:`
    TermDefined = "term-defined",
    /// `Descriptor`
    TermDescriptor = "term-descriptor",
    /// `Effective date (optional)`
    TermEffectiveDate = "term-effective-date",
    /// `Expand`
    TermExpand = "term-expand",
    /// `Expand one of its groups`
    TermExpandGroup = "term-expand-group",
    /// `Fields you can ask for`
    TermFields = "term-fields",
    /// `The openEHR vocabulary has no broader or narrower codes, so it always answers no; a termin`
    TermFlat = "term-flat",
    /// `Hierarchy`
    TermHierarchy = "term-hierarchy",
    /// `Id`
    TermId = "term-id",
    /// `Language`
    TermLanguage = "term-language",
    /// `Look up`
    TermLookUp = "term-look-up",
    /// `Look up`
    TermLookup = "term-lookup",
    /// `Look up a code in it`
    TermLookupIn = "term-lookup-in",
    /// `The CDR could not look the code up.`
    TermLookupRefused = "term-lookup-refused",
    /// `{ $code } is a member of { $set }.`
    TermMember = "term-member",
    /// `Members`
    TermMembers = "term-members",
    /// `Narrower code`
    TermNarrower = "term-narrower",
    /// `The terminology has no value set { $set }.`
    TermNoValueSet = "term-no-value-set",
    /// `{ $code } is not a member of { $set }.`
    TermNotMember = "term-not-member",
    /// `{ $broader } is not broader than { $narrower }.`
    TermNotSubsumes = "term-not-subsumes",
    /// `The CDR's terminology API is off.`
    TermOff = "term-off",
    /// `Set in ferroehr.toml and read at start; the Configuration page shows the running value.`
    TermOffLine = "term-off-line",
    /// `Terminology operations`
    TermOperations = "term-operations",
    /// `Preferred term`
    TermPreferred = "term-preferred",
    /// `Publisher`
    TermPublisher = "term-publisher",
    /// `The terminologies could not be read.`
    TermRefused = "term-refused",
    /// `Terminologies the CDR serves`
    TermServed = "term-served",
    /// `{ $broader } is broader than { $narrower }.`
    TermSubsumes = "term-subsumes",
    /// `The CDR could not test the codes.`
    TermSubsumptionRefused = "term-subsumption-refused",
    /// `What the CDR can answer about the code sets it serves.`
    TermSubtitle = "term-subtitle",
    /// `Terminologies`
    TermTerminologies = "term-terminologies",
    /// `Terminology`
    TermTerminology = "term-terminology",
    /// `Test`
    TermTest = "term-test",
    /// `The terminology { $terminology } does not define { $code }.`
    TermUndefined = "term-undefined",
    /// `Identifying URI`
    TermUri = "term-uri",
    /// `Validate`
    TermValidate = "term-validate",
    /// `The CDR could not validate the code.`
    TermValidateRefused = "term-validate-refused",
    /// `Validate a code`
    TermValidateTitle = "term-validate-title",
    /// `Value set`
    TermValueSet = "term-value-set",
    /// `The CDR could not expand the value set.`
    TermValueSetRefused = "term-value-set-refused",
    /// `Value sets`
    TermValueSets = "term-value-sets",
    /// `{ $name } { $version }`
    QueriesAqlTitle = "queries-aql-title",
    /// `Definition`
    QueriesDefinition = "queries-definition",
    /// `Delete version { $version }`
    QueriesDeleteConfirm = "queries-delete-confirm",
    /// `Removes this version from the CDR's stored queries for good, for every client; its other v`
    QueriesDeleteMessage = "queries-delete-message",
    /// `Delete version { $version }…`
    QueriesDeleteNamed = "queries-delete-named",
    /// `The CDR refused to delete the version.`
    QueriesDeleteRefused = "queries-delete-refused",
    /// `Delete { $name } version { $version }?`
    QueriesDeleteTitle = "queries-delete-title",
    /// `Delete this version…`
    QueriesDeleteVersion = "queries-delete-version",
    /// `Stored query version deleted`
    QueriesDeleted = "queries-deleted",
    /// `Nothing was sent: fix the version first.`
    QueriesFixVersion = "queries-fix-version",
    /// `{ $names ->`
    QueriesGroupCount = "queries-group-count",
    /// `{ $name } · version { $version }`
    QueriesLine = "queries-line",
    /// `More actions for the stored query`
    QueriesMore = "queries-more",
    /// `Namespace { $value }`
    QueriesNamespaceChip = "queries-namespace-chip",
    /// `older version`
    QueriesOlderVersion = "queries-older-version",
    /// `Open in Query`
    QueriesOpenInQuery = "queries-open-in-query",
    /// `Numbers and true or false are sent as they are; quote a number to send it as text.`
    QueriesParamHelp = "queries-param-help",
    /// `Read { $version }`
    QueriesReadVersion = "queries-read-version",
    /// `You are reading version { $version }; Latest runs { $latest }.`
    QueriesReadingOlder = "queries-reading-older",
    /// `The stored queries could not be read.`
    QueriesRefused = "queries-refused",
    /// `Saved`
    QueriesSaved = "queries-saved",
    /// `The stored query could not be read.`
    QueriesStoredRefused = "queries-stored-refused",
    /// `The CDR's stored queries, grouped by the namespace of their name.`
    QueriesSubtitle = "queries-subtitle",
    /// `Type`
    QueriesType = "queries-type",
    /// `Unqualified`
    QueriesUnqualified = "queries-unqualified",
    /// `Each version is its own immutable definition; Latest runs the highest.`
    QueriesVersionsIntro = "queries-versions-intro",
    /// `Views of this stored query`
    QueriesViews = "queries-views",
    /// `Which version runs`
    QueriesWhichVersion = "queries-which-version",
    /// `Add a data point`
    QueryAddDataPoint = "query-add-data-point",
    /// `Add a parameter`
    QueryAddParam = "query-add-param",
    /// `AQL`
    QueryAql = "query-aql",
    /// `Query · AQL`
    QueryAqlTitle = "query-aql-title",
    /// `The row limit is not a whole number.`
    QueryBadLimit = "query-bad-limit",
    /// `Build`
    QueryBuild = "query-build",
    /// `The AQL Build made, as it runs; edits here leave Build behind.`
    QueryBuildMade = "query-build-made",
    /// `Built AQL`
    QueryBuiltAql = "query-built-aql",
    /// `This AQL cannot be opened in Build: { $reason }`
    QueryCannotBuild = "query-cannot-build",
    /// `The template's data points could not be read.`
    QueryCatalogRefused = "query-catalog-refused",
    /// `Column name`
    QueryColumnName = "query-column-name",
    /// `Column name of data point { $number }`
    QueryColumnNameN = "query-column-name-n",
    /// `Conditions`
    QueryConditions = "query-conditions",
    /// `Data point`
    QueryDataPoint = "query-data-point",
    /// `Data point { $number }`
    QueryDataPointN = "query-data-point-n",
    /// `Data points`
    QueryDataPoints = "query-data-points",
    /// `Direction`
    QueryDirection = "query-direction",
    /// `The query ends before it is complete.`
    QueryEndedEarly = "query-ended-early",
    /// `{ $count ->`
    QueryGrammarHeading = "query-grammar-heading",
    /// `Nothing was sent: the grammar check found an error.`
    QueryGrammarStops = "query-grammar-stops",
    /// `The AQL it builds`
    QueryItBuilds = "query-it-builds",
    /// `Row limit`
    QueryLimit = "query-limit",
    /// `Namespace`
    QueryNamespace = "query-namespace",
    /// `Newest first`
    QueryNewestFirst = "query-newest-first",
    /// `No data point yet; the query returns nothing until you add one.`
    QueryNoDataPoints = "query-no-data-points",
    /// `No order`
    QueryNoOrder = "query-no-order",
    /// `No parameter.`
    QueryNoParams = "query-no-params",
    /// `The builder cannot write this query yet: { $reason }`
    QueryNotBuilt = "query-not-built",
    /// `The value of { $path }, { $value }, is not a number.`
    QueryNotNumber = "query-not-number",
    /// `Nothing to run: write a query first.`
    QueryNothingToRun = "query-nothing-to-run",
    /// `Oldest first`
    QueryOldestFirst = "query-oldest-first",
    /// `Open in AQL`
    QueryOpenInAql = "query-open-in-aql",
    /// `Open in Build`
    QueryOpenInBuild = "query-open-in-build",
    /// `Open it`
    QueryOpenStored = "query-open-stored",
    /// `Opened from the stored query { $name } { $version }.`
    QueryOpenedStored = "query-opened-stored",
    /// `Order by`
    QueryOrderBy = "query-order-by",
    /// `Order and limit`
    QueryOrderLimit = "query-order-limit",
    /// `Name of parameter { $number }`
    QueryParamNameN = "query-param-name-n",
    /// `Value of parameter { $number }`
    QueryParamValueN = "query-param-value-n",
    /// `Parameters`
    QueryParameters = "query-parameters",
    /// `Remove the data point { $name }`
    QueryRemoveDataPoint = "query-remove-data-point",
    /// `Remove the parameter { $name }`
    QueryRemoveParam = "query-remove-param",
    /// `Returns`
    QueryReturns = "query-returns",
    /// `Run`
    QueryRun = "query-run",
    /// `The CDR refused the query.`
    QueryRunRefused = "query-run-refused",
    /// `Save to the CDR`
    QuerySave = "query-save",
    /// `Save`
    QuerySaveAction = "query-save-action",
    /// `The CDR refused to store the query.`
    QuerySaveRefused = "query-save-refused",
    /// `Whole compositions`
    QueryShapeCompositions = "query-shape-compositions",
    /// `A count`
    QueryShapeCount = "query-shape-count",
    /// `Data points`
    QueryShapeData = "query-shape-data",
    /// `EHRs (cohort)`
    QueryShapeEhrs = "query-shape-ehrs",
    /// `The stored query could not be read.`
    QueryStoredRefused = "query-stored-refused",
    /// `Build AQL from a template, or write it; run it against the CDR.`
    QuerySubtitle = "query-subtitle",
    /// `The AQL`
    QueryTheAql = "query-the-aql",
    /// `Unexpected { $token } here.`
    QueryUnexpected = "query-unexpected",
    /// `The template has no data point at { $path }.`
    QueryUnknownPath = "query-unknown-path",
    /// `Empty writes the version the CDR assigns.`
    QueryVersionHelp = "query-version-help",
    /// `This version of this name already exists, and a saved version cannot change.`
    QueryVersionTaken = "query-version-taken",
    /// `Ways to write the query`
    QueryWays = "query-ways",
    /// `The x axis is { $axis }, at its true spacing.`
    ResultsAxis = "results-axis",
    /// `Chart`
    ResultsChart = "results-chart",
    /// `The numeric columns over the results`
    ResultsChartLabel = "results-chart-label",
    /// `{ $series } over { $axis }.`
    ResultsChartSummary = "results-chart-summary",
    /// `CSV`
    ResultsCsv = "results-csv",
    /// `JSON`
    ResultsJson = "results-json",
    /// `These results have no numbers to draw, so they are shown as a table.`
    ResultsNoChart = "results-no-chart",
    /// `The query matched no row.`
    ResultsNone = "results-none",
    /// `Run the query to see its rows here.`
    ResultsNotRun = "results-not-run",
    /// `Show as`
    ResultsShowAs = "results-show-as",
    /// `Table`
    ResultsTable = "results-table",
    /// `Results`
    ResultsTitle = "results-title",
    /// `{ $count ->`
    ResultsWindow = "results-window",
    /// `{ $count ->`
    ResultsWindowOwn = "results-window-own",
    /// `openEHR: Admin (in development)`
    ApiAdmin = "api-admin",
    /// `The complete surface`
    ApiAll = "api-all",
    /// `FerroEHR: Audit`
    ApiAudit = "api-audit",
    /// `openEHR: Definition`
    ApiDefinition = "api-definition",
    /// `openEHR: Demographic (in development)`
    ApiDemographic = "api-demographic",
    /// `openEHR: EHR`
    ApiEhr = "api-ehr",
    /// `FerroEHR: Event Subscriptions`
    ApiEvents = "api-events",
    /// `FerroEHR: Status and Management`
    ApiManagement = "api-management",
    /// `openEHR: Query`
    ApiQuery = "api-query",
    /// `FerroEHR: Party Relationships`
    ApiRelationships = "api-relationships",
    /// `FerroEHR: SMART Discovery`
    ApiSmart = "api-smart",
    /// `FerroEHR: Terminology`
    ApiTerminology = "api-terminology",
    /// `Action`
    AuditAction = "audit-action",
    /// `As stored`
    AuditAsStored = "audit-as-stored",
    /// `Action: { $value }`
    AuditChipAction = "audit-chip-action",
    /// `Principal: { $value }`
    AuditChipAgent = "audit-chip-agent",
    /// `Outcome: { $value }`
    AuditChipOutcome = "audit-chip-outcome",
    /// `Patient: { $value }`
    AuditChipPatient = "audit-chip-patient",
    /// `The CDR has recorded no audit event yet.`
    AuditEmpty = "audit-empty",
    /// `Event`
    AuditEvent = "audit-event",
    /// `Principal`
    AuditFind = "audit-find",
    /// `No audit record matches these filters.`
    AuditNoMatch = "audit-no-match",
    /// `No patient`
    AuditNoPatient = "audit-no-patient",
    /// `Outcome`
    AuditOutcome = "audit-outcome",
    /// `Audit pages`
    AuditPages = "audit-pages",
    /// `Patient`
    AuditPatient = "audit-patient",
    /// `Principal`
    AuditPrincipal = "audit-principal",
    /// `This record is no longer on the page it was listed on; open it from the audit log again.`
    AuditRecordGone = "audit-record-gone",
    /// `The audit log could not be read.`
    AuditRefused = "audit-refused",
    /// `The CDR keeps no local audit store; its records go only to the forwarding target, so there`
    AuditStoreOff = "audit-store-off",
    /// `Set in ferroehr.toml; the Configuration page shows the running value.`
    AuditStoreOffLine = "audit-store-off-line",
    /// `Stored as a FHIR AuditEvent (IHE ITI-81).`
    AuditStoredAs = "audit-stored-as",
    /// `The CDR's security audit log: who accessed what, and with what outcome.`
    AuditSubtitle = "audit-subtitle",
    /// `Success`
    AuditSuccess = "audit-success",
    /// `The event`
    AuditTheEvent = "audit-the-event",
    /// `Unknown`
    AuditUnknown = "audit-unknown",
    /// `What`
    AuditWhat = "audit-what",
    /// `When`
    AuditWhen = "audit-when",
    /// `Access`
    ConfigAccess = "config-access",
    /// `Basic sign-in`
    ConfigBasicSignIn = "config-basic-sign-in",
    /// `Offers the username and password form; the CDR checks the account.`
    ConfigBasicSignInLine = "config-basic-sign-in-line",
    /// `CDR address`
    ConfigCdrAddress = "config-cdr-address",
    /// `The address the viewer calls for every CDR request.`
    ConfigCdrAddressLine = "config-cdr-address-line",
    /// `CDR base path`
    ConfigCdrBasePath = "config-cdr-base-path",
    /// `Must match the base path the CDR serves its API under.`
    ConfigCdrBasePathLine = "config-cdr-base-path-line",
    /// `Deployment`
    ConfigDeployment = "config-deployment",
    /// `read at start; change the file and restart to change a setting`
    ConfigHow = "config-how",
    /// `Session idle time`
    ConfigIdle = "config-idle",
    /// `A session ends after this long without a request.`
    ConfigIdleLine = "config-idle-line",
    /// `Integrations`
    ConfigIntegrations = "config-integrations",
    /// `Language`
    ConfigLanguage = "config-language",
    /// `Used when the browser asks for a language the viewer does not have.`
    ConfigLanguageLine = "config-language-line",
    /// `Management address`
    ConfigManagementAddress = "config-management-address",
    /// `Where the viewer reads health, metrics and logging.`
    ConfigManagementAddressLine = "config-management-address-line",
    /// `{ $count } min`
    ConfigMinutes = "config-minutes",
    /// `Off`
    ConfigOff = "config-off",
    /// `OIDC client secret`
    ConfigOidcSecret = "config-oidc-secret",
    /// `Proves the viewer to the identity provider; read from a file and never shown.`
    ConfigOidcSecretLine = "config-oidc-secret-line",
    /// `OIDC sign-in`
    ConfigOidcSignIn = "config-oidc-sign-in",
    /// `The identity provider that signs people in.`
    ConfigOidcSignInLine = "config-oidc-sign-in-line",
    /// `On`
    ConfigOn = "config-on",
    /// `Operations`
    ConfigOperations = "config-operations",
    /// `Records`
    ConfigRecords = "config-records",
    /// `The CDR's configuration could not be read.`
    ConfigRefused = "config-refused",
    /// `Configuration sections`
    ConfigSections = "config-sections",
    /// `Server and database`
    ConfigServer = "config-server",
    /// `Session key`
    ConfigSessionKey = "config-session-key",
    /// `Seals the session cookie; every replica holds the same key.`
    ConfigSessionKeyLine = "config-session-key-line",
    /// `The CDR's settings as it serves them, redacted, and the viewer's own.`
    ConfigSubtitle = "config-subtitle",
    /// `Viewer`
    ConfigViewer = "config-viewer",
    /// `Also`
    ServerAlso = "server-also",
    /// `API`
    ServerApi = "server-api",
    /// `API family`
    ServerApiFamily = "server-api-family",
    /// `The OpenAPI documents the CDR serves for itself, by API family.`
    ServerApiLine = "server-api-line",
    /// `The OpenAPI document could not be read.`
    ServerApiRefused = "server-api-refused",
    /// `Apply…`
    ServerApply = "server-apply",
    /// `Apply the filter`
    ServerApplyConfirm = "server-apply-confirm",
    /// `It applies to this instance until it restarts.`
    ServerApplyMessage = "server-apply-message",
    /// `Apply the new log filter?`
    ServerApplyTitle = "server-apply-title",
    /// `At boot`
    ServerAtBoot = "server-at-boot",
    /// `The filter in effect is the boot filter.`
    ServerAtBootAlready = "server-at-boot-already",
    /// `Build`
    ServerBuild = "server-build",
    /// `The build information could not be read.`
    ServerBuildRefused = "server-build-refused",
    /// `Change the log filter`
    ServerChangeFilter = "server-change-filter",
    /// `Applies to this instance until it restarts.`
    ServerChangeFilterIntro = "server-change-filter-intro",
    /// `Versions and conformance`
    ServerConformance = "server-conformance",
    /// `Dependencies`
    ServerDependencies = "server-dependencies",
    /// `Dependency`
    ServerDependency = "server-dependency",
    /// `Deployment`
    ServerDeployment = "server-deployment",
    /// `Detail`
    ServerDetail = "server-detail",
    /// `{ $count ->`
    ServerEndpointCount = "server-endpoint-count",
    /// `Endpoints of { $family }`
    ServerEndpointsOf = "server-endpoints-of",
    /// `Filter`
    ServerFilter = "server-filter",
    /// `Log filter applied`
    ServerFilterApplied = "server-filter-applied",
    /// `The CDR refused the log filter.`
    ServerFilterRefused = "server-filter-refused",
    /// `Log filter reset to the boot filter`
    ServerFilterReset = "server-filter-reset",
    /// `Grant types`
    ServerGrantTypes = "server-grant-types",
    /// `Health`
    ServerHealth = "server-health",
    /// `What the CDR depends on, and whether each part is healthy.`
    ServerHealthLine = "server-health-line",
    /// `The dependencies could not be read.`
    ServerHealthRefused = "server-health-refused",
    /// `In effect`
    ServerInEffect = "server-in-effect",
    /// `Labels`
    ServerLabels = "server-labels",
    /// `Logging`
    ServerLogging = "server-logging",
    /// `The live log filter of the instance that answered.`
    ServerLoggingLine = "server-logging-line",
    /// `The log filter could not be read.`
    ServerLoggingRefused = "server-logging-refused",
    /// `Method and path`
    ServerMethodPath = "server-method-path",
    /// `Metric`
    ServerMetric = "server-metric",
    /// `{ $count ->`
    ServerMetricCount = "server-metric-count",
    /// `Metrics`
    ServerMetrics = "server-metrics",
    /// `Counters since the last start.`
    ServerMetricsLine = "server-metrics-line",
    /// `The metrics could not be read.`
    ServerMetricsRefused = "server-metrics-refused",
    /// `New filter`
    ServerNewFilter = "server-new-filter",
    /// `The CDR serves no build information.`
    ServerNoBuildInfo = "server-no-build-info",
    /// `The CDR serves no log filter control.`
    ServerNoLogging = "server-no-logging",
    /// `The CDR has no management endpoints turned on, so Health, Metrics and Logging are not offe`
    ServerNoManagement = "server-no-management",
    /// `The CDR has no such metric.`
    ServerNoMetric = "server-no-metric",
    /// `The CDR serves no SMART discovery document.`
    ServerNoSmart = "server-no-smart",
    /// `Now`
    ServerNow = "server-now",
    /// `The OpenAPI document`
    ServerOpenApi = "server-open-api",
    /// `Overview`
    ServerOverview = "server-overview",
    /// `How the CDR was built and deployed.`
    ServerOverviewLine = "server-overview-line",
    /// `PKCE`
    ServerPkce = "server-pkce",
    /// `Production`
    ServerProduction = "server-production",
    /// `The CDR serves only each dependency's state, not its detail.`
    ServerPublicReadiness = "server-public-readiness",
    /// `The metric registry`
    ServerRegistry = "server-registry",
    /// `Reset to the boot filter`
    ServerReset = "server-reset",
    /// `Safeguard`
    ServerSafeguard = "server-safeguard",
    /// `Production safeguards that are off`
    ServerSafeguardsOff = "server-safeguards-off",
    /// `Samples of { $metric }`
    ServerSamplesOf = "server-samples-of",
    /// `Sandbox`
    ServerSandbox = "server-sandbox",
    /// `A sandbox must not hold real patient data.`
    ServerSandboxIntro = "server-sandbox-intro",
    /// `Scopes it supports`
    ServerScopes = "server-scopes",
    /// `Server sections`
    ServerSections = "server-sections",
    /// `Show`
    ServerShow = "server-show",
    /// `Sign-in`
    ServerSignIn = "server-sign-in",
    /// `SMART`
    ServerSmart = "server-smart",
    /// `The scopes the CDR supports; Your access shows the ones you hold.`
    ServerSmartLine = "server-smart-line",
    /// `The SMART discovery document could not be read.`
    ServerSmartRefused = "server-smart-refused",
    /// `State`
    ServerState = "server-state",
    /// `Status`
    ServerStatus = "server-status",
    /// `The CDR this viewer reads: what it is, how it runs, and what it serves.`
    ServerSubtitle = "server-subtitle",
    /// `Summary`
    ServerSummary = "server-summary",
    /// `Unknown`
    ServerUnknown = "server-unknown",
    /// `Unreachable`
    ServerUnreachable = "server-unreachable",
    /// `Up`
    ServerUp = "server-up",
    /// `Version`
    ServerVersion = "server-version",
    /// `Any change`
    SubAnyChange = "sub-any-change",
    /// `Any kind`
    SubAnyKind = "sub-any-kind",
    /// `Any template`
    SubAnyTemplate = "sub-any-template",
    /// `Change the name first.`
    SubChangeName = "sub-change-name",
    /// `Change type`
    SubChangeType = "sub-change-type",
    /// `Composition`
    SubComposition = "sub-composition",
    /// `Create subscription`
    SubCreate = "sub-create",
    /// `Subscription created`
    SubCreated = "sub-created",
    /// `{ $name } · queue ferroehr.events.{ $name }`
    SubCreatedLine = "sub-created-line",
    /// `Delete the subscription…`
    SubDelete = "sub-delete",
    /// `Delete the subscription`
    SubDeleteConfirm = "sub-delete-confirm",
    /// `Removes the subscription for good. The queue and its messages stay on the broker; remove t`
    SubDeleteMessage = "sub-delete-message",
    /// `The CDR refused to delete the subscription.`
    SubDeleteRefused = "sub-delete-refused",
    /// `Delete the subscription { $name }?`
    SubDeleteTitle = "sub-delete-title",
    /// `Subscription deleted`
    SubDeleted = "sub-deleted",
    /// `Deliver messages`
    SubDeliver = "sub-deliver",
    /// `When off, the subscription is kept but sends nothing.`
    SubDeliverHelp = "sub-deliver-help",
    /// `Delivery`
    SubDelivery = "sub-delivery",
    /// `Disable`
    SubDisable = "sub-disable",
    /// `Subscription disabled`
    SubDisabled = "sub-disabled",
    /// `Saving replaces what this subscription selects.`
    SubEditSubtitle = "sub-edit-subtitle",
    /// `{ $mode ->`
    SubEditorTitle = "sub-editor-title",
    /// `EHR status`
    SubEhrStatus = "sub-ehr-status",
    /// `No subscription yet.`
    SubEmpty = "sub-empty",
    /// `Enable`
    SubEnable = "sub-enable",
    /// `Subscription enabled`
    SubEnabled = "sub-enabled",
    /// `Every { $change } of any kind.`
    SubEveryChange = "sub-every-change",
    /// `Every { $change } of a { $kind }.`
    SubEveryChangeOfKind = "sub-every-change-of-kind",
    /// `Every committed { $kind }.`
    SubEveryOfKind = "sub-every-of-kind",
    /// `Every committed version of any kind.`
    SubEveryVersion = "sub-every-version",
    /// `Fix the name first.`
    SubFixName = "sub-fix-name",
    /// `Folder`
    SubFolder = "sub-folder",
    /// `More actions for the subscription`
    SubMore = "sub-more",
    /// `The name cannot change: a new name would be a different queue.`
    SubNameFixed = "sub-name-fixed",
    /// `Letters, digits, dots, dashes and underscores. The name cannot change later.`
    SubNameHelp = "sub-name-help",
    /// `Use letters, digits, dots, dashes and underscores only.`
    SubNameInvalid = "sub-name-invalid",
    /// `The CDR already has a subscription called { $name }. Choose another name.`
    SubNameTaken = "sub-name-taken",
    /// `New subscription`
    SubNew = "sub-new",
    /// `The CDR has no subscription called { $name }.`
    SubNone = "sub-none",
    /// `The CDR's subscription API is off, so there is nothing to list.`
    SubOff = "sub-off",
    /// `Set in ferroehr.toml; the Configuration page shows the running value.`
    SubOffLine = "sub-off-line",
    /// `Queue`
    SubQueue = "sub-queue",
    /// `The subscriptions could not be read.`
    SubRefused = "sub-refused",
    /// `The CDR refused the subscription.`
    SubSaveRefused = "sub-save-refused",
    /// `Subscription saved`
    SubSaved = "sub-saved",
    /// `What it selects`
    SubSelects = "sub-selects",
    /// `Disabled`
    SubStateDisabled = "sub-state-disabled",
    /// `Enabled`
    SubStateEnabled = "sub-state-enabled",
    /// `Get a message on the event queue for each change that matches.`
    SubSubtitle = "sub-subtitle",
    /// `{ $line } Under { $template } only.`
    SubUnderTemplate = "sub-under-template",
    /// `Delete…`
    SubDeleteRow = "sub-delete-row",
    /// `Delete…`
    TemplateDeleteRow = "template-delete-row",
    /// `Name the query to save it.`
    QuerySaveNameMissing = "query-save-name-missing",
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
        // The language preference of Your preferences wins over the browser's
        // when it names a language the viewer ships (family rule 9).
        let chosen = use_context::<http::request::Parts>()
            .and_then(|parts| {
                parts
                    .headers
                    .get(http::header::COOKIE)
                    .and_then(|value| value.to_str().ok())
                    .map(|cookies| {
                        ferrohealth_ui::theme::Preferences::read(cookies, crate::prefs::COOKIE_PREFIX)
                            .language
                    })
            })
            .and_then(|language| language.tag().map(str::to_owned))
            .filter(|tag| shipped().iter().any(|locale| locale.tag() == *tag))
            .and_then(|tag| Locale::parse(&tag).ok());
        chosen.unwrap_or_else(|| Locale::negotiate_with(&header, catalog(), configured))
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
