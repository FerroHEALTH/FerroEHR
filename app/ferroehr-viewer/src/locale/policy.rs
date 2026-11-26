// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The words of the access control pages, in the catalogs under
//! `locales/policy/`.

crate::locale::messages! {
    /// A string the access control pages draw, by its message in its catalog.
    PolicyMsg in "./locales/policy" {
        /// `The access policy's history is not a JSON list`
        HistoryJson = "history-json",
        /// `The access policy is not a JSON object`
        PolicyJson = "policy-json",
        /// `Policy`
        TabPolicy = "tab-policy",
        /// `Versions`
        TabVersions = "tab-versions",
        /// `The controller's rules for every EHR this CDR keeps, as the CDR decides each request by them.`
        PolicySubtitle = "policy-subtitle",
        /// `Access policy sections`
        PolicySections = "policy-sections",
        /// `Your account may not read the access policy.`
        PolicyForbidden = "policy-forbidden",
        /// `The CDR's admin API is off, so this cannot be read here.`
        AdminOff = "admin-off",
        /// `The access policy could not be read.`
        PolicyRefused = "policy-refused",
        /// `No such version of the access policy.`
        VersionMissing = "version-missing",
        /// The access policy has no version `$version`.
        VersionMissingLine = "version-missing-line",
        /// `Classifications`
        SectionClassifications = "section-classifications",
        /// `Which data carry which sensitivities and confidentiality.`
        SectionClassificationsIntro = "section-classifications-intro",
        /// `Controller rules`
        SectionRules = "section-rules",
        /// `Who may do what with which data, for which purposes (EHDS Art. 11(3)). A deny refuses; a permit clears the sensitivities it names.`
        SectionRulesIntro = "section-rules-intro",
        /// `Catalogue`
        SectionCatalogue = "section-catalogue",
        /// `The named directives a patient accepts by reference.`
        SectionCatalogueIntro = "section-catalogue-intro",
        /// `Care relationship facts`
        SectionCare = "section-care",
        /// `Whether a professional's clinical access needs a care relationship, and how one is read. An attribute left out reads as its strictest default.`
        SectionCareIntro = "section-care-intro",
        /// `Purpose of use`
        SectionPurpose = "section-purpose",
        /// `Whether a request must declare its purpose before any rule is read.`
        SectionPurposeIntro = "section-purpose-intro",
        /// `Conflict rules`
        SectionConflicts = "section-conflicts",
        /// `The conflicts of interest the professional path admits, and for whom.`
        SectionConflictsIntro = "section-conflicts-intro",
        /// `Concealed confidentialities`
        SectionConcealed = "section-concealed",
        /// `The confidentiality levels whose refused records answer as absent.`
        SectionConcealedIntro = "section-concealed-intro",
        /// `Log restrictions`
        SectionLogRestrictions = "section-log-restrictions",
        /// `What the access log's subject view leaves out, as the Member State's law lays down (EHDS Art. 9(3)).`
        SectionLogRestrictionsIntro = "section-log-restrictions-intro",
        /// `Value sets`
        SectionValueSets = "section-value-sets",
        /// `The value set on the terminology server each coded attribute is bound to.`
        SectionValueSetsIntro = "section-value-sets-intro",
        /// `Other attributes`
        SectionOther = "section-other",
        /// `The rest of what the policy states.`
        SectionOtherIntro = "section-other-intro",
        /// `None: the policy leaves this out.`
        LeftOut = "left-out",
        /// `Left out`
        LeftOutShort = "left-out-short",
        /// `Name`
        FactName = "fact-name",
        /// `Controller`
        FactController = "fact-controller",
        /// `This policy`
        ThisPolicy = "this-policy",
        /// `Write the first version`
        WriteFirst = "write-first",
        /// `Edit the policy`
        EditPolicy = "edit-policy",
        /// `No access policy is committed`
        NoneCommitted = "none-committed",
        /// `Nothing is tightened: the CDR decides every request by its configuration alone.`
        NoneCommittedLine = "none-committed-line",
        /// `ACCESS_POLICY`, version `$version`
        DocumentTitle = "document-title",
        /// `Its own`
        OwnContribution = "own-contribution",
        /// `Versions`
        VersionsCaption = "versions-caption",
        /// `Every version of the access policy, newest first. Open one to read it.`
        VersionsIntro = "versions-intro",
        /// `the version in force then`
        AtTimeOpens = "at-time-opens",
        /// `Version`
        FactVersion = "fact-version",
        /// `Committer`
        FactCommitter = "fact-committer",
        /// `Committed`
        FactCommitted = "fact-committed",
        /// `In force`
        InForce = "in-force",
        /// `Access policy`
        StarterName = "starter-name",
        /// `Access policy saved`
        Saved = "saved",
        /// Version `$version` is in force on every replica from the next request.
        SavedVersion = "saved-version",
        /// `The CDR did not save the access policy.`
        SaveRefused = "save-refused",
        /// `Save as version 1`
        SaveFirst = "save-first",
        /// `Reload`
        ReloadNone = "reload-none",
        /// `The access policy, canonical JSON`
        EditorLabel = "editor-label",
        /// `The whole ACCESS_POLICY. The save replaces the version you opened; if another administrator saved first, the CDR refuses it.`
        EditIntro = "edit-intro",
        /// `The first ACCESS_POLICY. Fill in the controller, the ORGANISATION whose policy this is, then save.`
        EditFirstIntro = "edit-first-intro",
        /// `Document`
        EditDocument = "edit-document",
        /// `The first version`
        EditFirstSubtitle = "edit-first-subtitle",
        /// Opened at version `$version`
        EditSubtitle = "edit-subtitle",
        /// `Edit the access policy`
        EditTitle = "edit-title",
        /// `The CDR's answer about the grants is not a result set`
        GrantsJson = "grants-json",
        /// `always`
        GrantsAlways = "grants-always",
        /// `$from` to `$to`
        GrantsBetween = "grants-between",
        /// `In force`
        GrantsInForce = "grants-in-force",
        /// `Not in force`
        GrantsNotInForce = "grants-not-in-force",
        /// `Role`
        GrantsRole = "grants-role",
        /// `Grants`
        GrantsCodes = "grants-codes",
        /// `Terms`
        GrantsTerms = "grants-terms",
        /// `Held by`
        GrantsHeldBy = "grants-held-by",
        /// `Validity`
        GrantsValidity = "grants-validity",
        /// Role: `$role`; capability: `$capability`
        GrantsValidityWords = "grants-validity-words",
        /// `State`
        GrantsState = "grants-state",
        /// `Open`
        GrantsOpen = "grants-open",
        /// `Holder`
        GrantsOpenPerformer = "grants-open-performer",
        /// `Provider`
        GrantsOpenProvider = "grants-open-provider",
        /// `Roles holding a grant`
        GrantsRoles = "grants-roles",
        /// `Each role whose capability holds an openehr_access grant, who performs it, its terms and whether it holds today.`
        GrantsRolesIntro = "grants-roles-intro",
        /// `System agent`
        GrantsAgent = "grants-agent",
        /// `Client id`
        GrantsClient = "grants-client",
        /// `Issuer`
        GrantsIssuer = "grants-issuer",
        /// `Acts for`
        GrantsActsFor = "grants-acts-for",
        /// `System agents`
        GrantsAgents = "grants-agents",
        /// `Each party bound to an OAuth 2.0 client, and the provider it acts for.`
        GrantsAgentsIntro = "grants-agents-intro",
        /// `No role holds a grant`
        GrantsNoRoles = "grants-no-roles",
        /// `Either none is written, or your account may not read the policy parties.`
        GrantsNoRolesLine = "grants-no-roles-line",
        /// `No system agent`
        GrantsNoAgents = "grants-no-agents",
        /// `No party binds an OAuth 2.0 client that your account may read.`
        GrantsNoAgentsLine = "grants-no-agents-line",
        /// Only the first `$count` of a list are shown.
        GrantsTruncated = "grants-truncated",
        /// `A grant is given by a new version of its ROLE that adds the grant to the role's capability, and ended by a version whose validity ends before today. Open the role to change it in the party editor.`
        GrantsHow = "grants-how",
        /// `Your account may not read grants.`
        GrantsForbidden = "grants-forbidden",
        /// `The grants could not be read.`
        GrantsRefused = "grants-refused",
        /// `Who may do what beyond the scopes of a login: the roles that hold an openehr_access grant and the system clients.`
        GrantsSubtitle = "grants-subtitle",
        /// This party binds the OAuth 2.0 client `$client` of `$issuer`.
        MarkClient = "mark-client",
        /// `Policy party`
        MarkTitle = "mark-title",
        /// `Only an administrator reads and writes this party, as it holds a grant or binds a client.`
        MarkIntro = "mark-intro",
        /// `Add`
        CareAdd = "care-add",
        /// `A care relationship of this person: the carer, the part they play, the provider in whose care it is, and the period. The CDR checks the role against the value set the access policy binds and the writer's grant.`
        CareAddIntro = "care-add-intro",
        /// `The CDR did not add the care relationship.`
        CareAddRefused = "care-add-refused",
        /// `Add a care relationship`
        CareAddTitle = "care-add-title",
        /// `Care relationship added`
        CareAdded = "care-added",
        /// `Ended ones too`
        CareAll = "care-all",
        /// `Live on`
        CareAt = "care-at",
        /// `Care relationships`
        CareCaption = "care-caption",
        /// `Carer`
        CareCarer = "care-carer",
        /// `Carer id`
        CareCarerId = "care-carer-id",
        /// `The carer party's id: a professional's PERSON or ROLE, a care team's GROUP or an ORGANISATION unit.`
        CareCarerIdHelp = "care-carer-id-help",
        /// `Carer kind`
        CareCarerKind = "care-carer-kind",
        /// `Change`
        CareChange = "care-change",
        /// `Change the role or the period. The save replaces the version you read; End closes the period today.`
        CareChangeIntro = "care-change-intro",
        /// `The CDR did not change the care relationship.`
        CareChangeRefused = "care-change-refused",
        /// `Care relationship changed`
        CareChanged = "care-changed",
        /// `Delete`
        CareDelete = "care-delete",
        /// `Delete this care relationship as written in error? Its earlier versions keep it.`
        CareDeleteMessage = "care-delete-message",
        /// `The CDR did not delete the care relationship.`
        CareDeleteRefused = "care-delete-refused",
        /// `Care relationship deleted`
        CareDeleted = "care-deleted",
        /// `Direct`
        CareDirect = "care-direct",
        /// `PARTY_RELATIONSHIP`, version `$version`
        CareDocument = "care-document",
        /// `The care relationships of this EHR's subject, as the CDR lets you read them: live ones today unless you ask for a day or for ended ones.`
        CareEhrIntro = "care-ehr-intro",
        /// Care · EHR `$id`
        CareEhrTitle = "care-ehr-title",
        /// `End today`
        CareEnd = "care-end",
        /// `End this care relationship today? Its period closes, and from tomorrow it gives no ground for access.`
        CareEndMessage = "care-end-message",
        /// `The CDR did not end the care relationship.`
        CareEndRefused = "care-end-refused",
        /// `Care relationship ended`
        CareEnded = "care-ended",
        /// `Your account may not read these care relationships.`
        CareForbidden = "care-forbidden",
        /// `From`
        CareFrom = "care-from",
        /// `This care relationship is deleted.`
        CareGone = "care-gone",
        /// `In their care`
        CareInCare = "care-in-care",
        /// `The care relationships that reach this party as the carer, directly or through a team.`
        CareInCareIntro = "care-in-care-intro",
        /// `The CDR's care relationship is not JSON`
        CareJson = "care-json",
        /// `Manage on the person's page`
        CareManage = "care-manage",
        /// `Add, change or end a care relationship on the subject's PERSON.`
        CareManageLine = "care-manage-line",
        /// `This EHR's subject carries no external reference, so no care relationship names it.`
        CareNoSubject = "care-no-subject",
        /// `No care relationship`
        CareNone = "care-none",
        /// `None that you may read.`
        CareNoneLine = "care-none-line",
        /// `Care relationships of this person`
        CareOfPerson = "care-of-person",
        /// `Who cares for this person, as the CDR lets you read it.`
        CareOfPersonIntro = "care-of-person-intro",
        /// `An older version, read-only.`
        CareOlder = "care-older",
        /// `open`
        CareOpen = "care-open",
        /// `Care relationship pages`
        CarePages = "care-pages",
        /// `Who cares for this party, and whom it cares for.`
        CarePartySubtitle = "care-party-subtitle",
        /// Care · `$id`
        CarePartyTitle = "care-party-title",
        /// `Path`
        CarePath = "care-path",
        /// `Period`
        CarePeriod = "care-period",
        /// `Provider`
        CareProvider = "care-provider",
        /// `An identifier the provider's ORGANISATION carries: its issuer and its id.`
        CareProviderHelp = "care-provider-help",
        /// `Provider id`
        CareProviderId = "care-provider-id",
        /// `Provider identifier issuer`
        CareProviderIssuer = "care-provider-issuer",
        /// `The care relationships could not be read.`
        CareRefused = "care-refused",
        /// `Care role`
        CareRole = "care-role",
        /// `The part the carer plays. A role of another terminology is posted as terminology|code.`
        CareRoleHelp = "care-role-help",
        /// `Save the change`
        CareSave = "care-save",
        /// `Show`
        CareShow = "care-show",
        /// `Subject`
        CareSubject = "care-subject",
        /// `Care`
        CareTab = "care-tab",
        /// `Team`
        CareTeam = "care-team",
        /// `This care relationship`
        CareThis = "care-this",
        /// `Care relationship`
        CareTitle = "care-title",
        /// `To`
        CareTo = "care-to",
        /// `Every version of this care relationship, newest first, with who committed it and when.`
        CareVersionsIntro = "care-versions-intro",
        /// `The versions could not be read.`
        CareVersionsRefused = "care-versions-refused",
        /// `attender`
        RoleAtnd = "role-atnd",
        /// `consultant`
        RoleCon = "role-con",
        /// `participation`
        RolePart = "role-part",
        /// `primary performer`
        RolePprf = "role-pprf",
        /// `responsible party`
        RoleResp = "role-resp",
        /// `secondary performer`
        RoleSprf = "role-sprf",
        /// `The CDR's register is not JSON`
        RegisterJson = "register-json",
        /// `Audit categories`
        RegisterAuditCategories = "register-audit-categories",
        /// `Access delay`
        RegisterAccessDelay = "register-access-delay",
        /// `The default, which no one committed`
        RegisterDefault = "register-default",
        /// `$version` (shown)
        RegisterVersionShown = "register-version-shown",
        /// `An older version of the register, read-only.`
        RegisterOlder = "register-older",
        /// `The EHDS category map every access is classified through (Annex II 3.2(c)): template and archetype ids, each to its categories or none.`
        RegisterAuditCategoriesIntro = "register-audit-categories-intro",
        /// `The delay of the person's own access to newly registered data (EHDS Art. 3(3)), where the Member State provides for it.`
        RegisterAccessDelayIntro = "register-access-delay-intro",
        /// `Edit the register`
        RegisterEdit = "register-edit",
        /// `Document`
        RegisterDocument = "register-document",
        /// `Every version, newest first, ending with version 0, the default. Open one to read it.`
        RegisterHistoryIntro = "register-history-intro",
        /// Register document, version `$version`
        RegisterDocumentTitle = "register-document-title",
        /// `Register saved`
        RegisterSaved = "register-saved",
        /// Version `$version` is in force on every replica from the next request.
        RegisterSavedVersion = "register-saved-version",
        /// `The CDR did not save the register.`
        RegisterSaveRefused = "register-save-refused",
        /// `The whole register document. The save replaces the version you opened; if another administrator saved first, the CDR refuses it.`
        RegisterEditIntro = "register-edit-intro",
        /// `Replaces the register whole`
        RegisterEditSubtitle = "register-edit-subtitle",
        /// `Your account may not read this register.`
        RegisterForbidden = "register-forbidden",
        /// `Audit categories`
        TabAuditCategories = "tab-audit-categories",
        /// `Access delay`
        TabAccessDelay = "tab-access-delay",
        /// `The CDR's legal mark is not JSON`
        MarksJson = "marks-json",
        /// `Your account may not read the legal marks.`
        MarksForbidden = "marks-forbidden",
        /// `The legal mark could not be read.`
        MarksRefused = "marks-refused",
        /// `Legal mark recorded`
        MarksSaved = "marks-saved",
        /// `The CDR did not record the legal mark.`
        MarksSaveRefused = "marks-save-refused",
        /// `The whole EHR`
        MarksWholeEhr = "marks-whole-ehr",
        /// `No restriction of processing was ever requested.`
        MarksNoRestriction = "marks-no-restriction",
        /// `Object`
        MarksObject = "marks-object",
        /// `Ground`
        MarksGround = "marks-ground",
        /// `Placed`
        MarksPlaced = "marks-placed",
        /// `Lifted`
        MarksLifted = "marks-lifted",
        /// `Note`
        MarksNote = "marks-note",
        /// `In force`
        MarksInForce = "marks-in-force",
        /// `Restriction of processing`
        MarksRestrictions = "marks-restrictions",
        /// `Requests to restrict processing (GDPR Art. 18), newest first; a lift keeps the request. While in force, the restricted data answers 403 to every released read and write.`
        MarksRestrictionsIntro = "marks-restrictions-intro",
        /// `Versioned object id`
        MarksObjectId = "marks-object-id",
        /// `Leave empty for the whole EHR.`
        MarksObjectIdHelp = "marks-object-id-help",
        /// `Restrict processing`
        MarksRestrict = "marks-restrict",
        /// `Restrict processing? Every released read and write of the data it covers is refused until it is lifted.`
        MarksRestrictMessage = "marks-restrict-message",
        /// `Lift the restriction`
        MarksLift = "marks-lift",
        /// `Lift every restriction in force at this grain? The data is processed again.`
        MarksLiftMessage = "marks-lift-message",
        /// `The objection stands`
        MarksObjected = "marks-objected",
        /// `Objected, overridden on a public-interest ground`
        MarksOverridden = "marks-overridden",
        /// `No objection`
        MarksNoObjection = "marks-no-objection",
        /// `Standing`
        MarksStanding = "marks-standing",
        /// `Record the objection`
        MarksRecordObjection = "marks-record-objection",
        /// `Record the person's objection to research processing? Population queries, exports and the change feed leave this EHR out while it stands.`
        MarksObjectMessage = "marks-object-message",
        /// `Override ground`
        MarksOverrideGround = "marks-override-ground",
        /// `Override on a public-interest ground`
        MarksOverride = "marks-override",
        /// `Override the objection on the ground given (GDPR Art. 21(6))? The EHR is back in the research population.`
        MarksOverrideMessage = "marks-override-message",
        /// `Withdraw the objection`
        MarksWithdraw = "marks-withdraw",
        /// `Withdraw the objection entirely?`
        MarksWithdrawMessage = "marks-withdraw-message",
        /// `Objection to research processing`
        MarksObjection = "marks-objection",
        /// `The person's objection to processing for research (GDPR Art. 21(6)) and the controller's override of it.`
        MarksObjectionIntro = "marks-objection-intro",
        /// `No composition was held from the person.`
        MarksNoDelay = "marks-no-delay",
        /// `Basis`
        MarksBasis = "marks-basis",
        /// `Until`
        MarksUntil = "marks-until",
        /// `Released`
        MarksReleased = "marks-released",
        /// `Held`
        MarksHeld = "marks-held",
        /// `Access delay`
        MarksDelays = "marks-delays",
        /// `Compositions held from the person's own access (EHDS Art. 3(3)), newest first, each until its end or a professional's release.`
        MarksDelaysIntro = "marks-delays-intro",
        /// `Composition id`
        MarksCompositionId = "marks-composition-id",
        /// `An RFC 3339 instant within the access-delay register's limit; empty for the limit.`
        MarksUntilHelp = "marks-until-help",
        /// `Hold from the person`
        MarksDelay = "marks-delay",
        /// `Hold this composition from the person until its end or a release?`
        MarksDelayMessage = "marks-delay-message",
        /// `Release to the person`
        MarksRelease = "marks-release",
        /// `Release this composition to the person? They read it from now on.`
        MarksReleaseMessage = "marks-release-message",
        /// `No retention anchor is recorded.`
        MarksNoAnchor = "marks-no-anchor",
        /// `Jurisdiction`
        MarksJurisdiction = "marks-jurisdiction",
        /// `Anchored at`
        MarksAnchoredAt = "marks-anchored-at",
        /// `EHR-wide hold placed`
        MarksHoldAt = "marks-hold-at",
        /// `Hold ground`
        MarksHoldGround = "marks-hold-ground",
        /// `No object is held from disposal.`
        MarksNoHold = "marks-no-hold",
        /// `Held objects`
        MarksHolds = "marks-holds",
        /// `An RFC 3339 instant, or empty.`
        MarksInstantHelp = "marks-instant-help",
        /// `Record the anchor`
        MarksSetAnchor = "marks-set-anchor",
        /// `Record this retention anchor? A hold keeps the EHR off the due list.`
        MarksAnchorMessage = "marks-anchor-message",
        /// `Hold from disposal`
        MarksHold = "marks-hold",
        /// `Hold this object from disposal (EPDV Art. 10 Abs. 2 lit. b)?`
        MarksHoldMessage = "marks-hold-message",
        /// `Release the hold`
        MarksUnhold = "marks-unhold",
        /// `Release the hold on this object? It counts as due again when its period runs.`
        MarksUnholdMessage = "marks-unhold-message",
        /// `Retention`
        MarksRetention = "marks-retention",
        /// `The EHR's retention jurisdiction and anchor, an EHR-wide hold, and the objects held from disposal.`
        MarksRetentionIntro = "marks-retention-intro",
        /// Legal marks · EHR `$id`
        MarksTitle = "marks-title",
        /// `Legal marks`
        MarksTab = "marks-tab",
        /// `This composition may be held from the person (EHDS Art. 3(3)). As the treating professional you release it to them here.`
        MarksHoldNote = "marks-hold-note",
        /// Held from the person until `$until`.
        MarksHoldShown = "marks-hold-shown",
        /// `Released to the person`
        MarksReleaseDone = "marks-release-done",
        /// `The CDR did not release the composition.`
        MarksReleaseRefused = "marks-release-refused",
        /// `The access settings are not JSON`
        AccessSettingsJson = "access-settings-json",
        /// `Your account may not read these access settings.`
        AccessForbidden = "access-forbidden",
        /// `The access settings could not be read.`
        AccessRefused = "access-refused",
        /// `Directive`
        DirectiveName = "directive-name",
        /// `Kind`
        DirectiveKind = "directive-kind",
        /// `Effect`
        DirectiveEffect = "directive-effect",
        /// `Covers`
        DirectiveCovers = "directive-covers",
        /// `Names`
        DirectiveNames = "directive-names",
        /// `Emergency override`
        DirectiveEmergency = "directive-emergency",
        /// `Directives`
        AccessDirectives = "access-directives",
        /// `The person's consents, opt-outs and restrictions of access (EHDS Art. 8 and 10), and the directives they accepted: what each covers, whom it names, its period and whether an emergency overrides it.`
        AccessDirectivesIntro = "access-directives-intro",
        /// `No directive.`
        AccessNoDirectives = "access-no-directives",
        /// `Commit new settings`
        AccessCommit = "access-commit",
        /// `An older version of the settings, read-only.`
        AccessOlder = "access-older",
        /// `No settings: the policy's defaults apply.`
        AccessNone = "access-none",
        /// `These settings`
        AccessThis = "access-this",
        /// `EHR_ACCESS` settings, version `$version`
        AccessDocument = "access-document",
        /// Access settings · EHR `$id`
        AccessTitle = "access-title",
        /// `Access settings committed`
        AccessSaved = "access-saved",
        /// `The CDR did not commit the access settings.`
        AccessSaveRefused = "access-save-refused",
        /// The `EHR_ACCESS` settings, canonical JSON
        AccessEditorLabel = "access-editor-label",
        /// `Repair: the settings in force do not parse, so replace them without naming their version`
        AccessRepair = "access-repair",
        /// The whole `ACCESS_DIRECTIVE_SETTINGS` on the person's instruction, or null for none. The commit names the version you opened; if the settings changed meanwhile, the CDR refuses it.
        AccessEditIntro = "access-edit-intro",
        /// `Access settings`
        AccessTab = "access-tab",
        /// `On the person's instruction`
        AccessEditSubtitle = "access-edit-subtitle",
        /// `Commit access settings`
        AccessEditTitle = "access-edit-title",
    }
}
