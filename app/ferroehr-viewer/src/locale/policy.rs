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
        /// `The CDR's admin API is off, so the access policy cannot be read here.`
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
    }
}
