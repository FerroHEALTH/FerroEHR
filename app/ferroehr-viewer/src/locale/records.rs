// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The words of the records group, in the catalogs under `locales/records/`.
//!
//! Only code of the records group looks them up, so their catalog travels
//! with that code: the browser loads it with the group's chunks, never with
//! the entry WebAssembly (`crate::locale`).

crate::locale::messages! {
    /// A string the records group draws, by its message in its catalog.
    RecordsMsg in "./locales/records" {
        /// `Composition deleted`
        CompositionDeleted = "composition-deleted",
        /// `Party created`
        PartyCreated = "party-created",
        /// `Contribution versions`
        ContributionVersions = "contribution-versions",
        /// `a contribution uid is required`
        ContributionUidRequired = "contribution-uid-required",
        /// `Party deleted`
        PartyDeleted = "party-deleted",
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
        /// `Relationship deleted`
        RelationshipDeleted = "relationship-deleted",
        /// `Relationship type`
        RelationshipType = "relationship-type",
        /// `Relationship created`
        RelationshipCreated = "relationship-created",
        /// `Party relationships`
        PartyRelationships = "party-relationships",
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
        /// `Audit`
        ContributionAudit = "contribution-audit",
        /// `Contribution audit`
        ContributionNewAudit = "contribution-new-audit",
        /// `Contribution committed`
        ContributionCommitted = "contribution-committed",
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
        /// `Composition committed`
        CompositionCommitted = "composition-committed",
        /// `a template id is required to commit a FLAT composition`
        TemplateIdRequiredCommitFlatComposition = "template-id-required-commit-flat-composition",
        /// `Directory deleted`
        DirectoryDeleted = "directory-deleted",
        /// `Directory restored`
        DirectoryRestored = "directory-restored",
        /// `Directory created`
        DirectoryCreated = "directory-created",
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
        /// `EHR views`
        EhrViews = "ehr-views",
        /// `EHR deleted`
        EhrDeleted = "ehr-deleted",
        /// `the loaded EHR_STATUS document is not a JSON object — reload this tab and retry`
        LoadedEhrStatusDocumentNotJson = "loaded-ehr-status-document-not-json",
        /// `Subject id`
        SubjectId = "subject-id",
        /// `EHR created`
        EhrCreated = "ehr-created",
        /// `$size bytes`
        Bytes = "bytes",
        /// CONTRIBUTION JSON: `$e`
        ContributionJson = "contribution-json",
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
        /// contributions JSON: `$e`
        ContributionsJson = "contributions-json",
        /// `other_details is not valid JSON: '$e'`
        OtherDetailsNotValidJson = "other-details-not-valid-json",
        /// `the loaded EHR_STATUS document is not valid JSON ('$e') — reload this tab and retry`
        LoadedEhrStatusDocumentNotValid = "loaded-ehr-status-document-not-valid",
        /// `the edited EHR_STATUS could not be serialized: '$e'`
        EditedEhrStatusCouldNotSerialized = "edited-ehr-status-could-not-serialized",
        /// New EHR `$new_id`
        NewEhr = "new-ehr",
        /// `deleting a party needs the latest version's full OBJECT_VERSION_ID: reload this screen...`
        DeletingPartyNeedsLatestVersionS = "deleting-party-needs-latest-version-s",
        /// `identities must hold at least one PARTY_IDENTITY: a party with no identity is invalid...`
        IdentitiesMustHoldLeastOneParty = "identities-must-hold-least-one-party",
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
        /// The EHR status flag `is_queryable`, on a badge
        FlagQueryable = "flag-queryable",
        /// The EHR status flag `is_modifiable`, on a badge
        FlagModifiable = "flag-modifiable",
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
        /// `Copy EHR id`
        EhrCopyId = "ehr-copy-id",
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
        /// `This version's envelope could not be read.`
        StatusFactsRefused = "status-facts-refused",
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
        /// `Composer`
        ColumnComposer = "column-composer",
        /// `Composition`
        ColumnComposition = "column-composition",
        /// `Start time`
        ColumnStartTime = "column-start-time",
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
        /// `The EHR's compositions could not be listed.`
        DirectoryCompositionsRefused = "directory-compositions-refused",
        /// `More directory actions`
        DirectoryMore = "directory-more",
        /// `Copy the directory id`
        DirectoryCopyId = "directory-copy-id",
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
        /// `Composer`
        FilterComposer = "filter-composer",
        /// `Start time`
        FilterStartTime = "filter-start-time",
        /// `Attestation`
        ChangeAttestation = "change-attestation",
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
        /// `Object`
        ColumnObject = "column-object",
        /// `The templates or compositions to pick from could not be read.`
        ContributionChoicesRefused = "contribution-choices-refused",
        /// `{ $count ->`
        ContributionCommit = "contribution-commit",
        /// `The CDR refused the contribution, so nothing was committed.`
        ContributionCommitRefused = "contribution-commit-refused",
        /// `Leave empty to commit as { $name }`
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
        /// `The composition, canonical JSON`
        CompositionEditorLabel = "composition-editor-label",
        /// `First committed`
        CompositionFirstCommitted = "composition-first-committed",
        /// `None: this is the first version`
        CompositionFirstVersion = "composition-first-version",
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
        /// `Copy the id`
        CompositionCopyId = "composition-copy-id",
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
        /// `Views of this composition`
        CompositionViews = "composition-views",
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
        /// `The whole party as JSON, as New party takes it; it must keep at least one identity.`
        PartyEditIntro = "party-edit-intro",
        /// `The party`
        PartyEditorLabel = "party-editor-label",
        /// `Identities`
        PartyIdentities = "party-identities",
        /// `Start over puts this kind's starting document in the editor.`
        PartyKindHelp = "party-kind-help",
        /// `{ $kind } · version { $version } · { $id }`
        PartyLine = "party-line",
        /// `More actions for the party`
        PartyMore = "party-more",
        /// `Copy the id`
        PartyCopyId = "party-copy-id",
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
        /// `Copy the id`
        RelationshipCopyId = "relationship-copy-id",
        /// `New relationship`
        RelationshipNewPageTitle = "relationship-new-page-title",
        /// `From this party to another.`
        RelationshipNewSubtitle = "relationship-new-subtitle",
        /// `{ $mode ->`
        RelationshipNewTitle = "relationship-new-title",
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
        /// `Give the relationship a type.`
        RelationshipNeedsType = "relationship-needs-type",
        /// `Give the relationship a type and a target party.`
        RelationshipNeedsTypeAndTarget = "relationship-needs-type-and-target",
        /// `Older versions`
        DirectoryOlderVersions = "directory-older-versions",
    }
}
