// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The words of the definitions group, in the catalogs under `locales/definitions/`.
//!
//! Only code of the definitions group looks them up, so their catalog travels
//! with that code: the browser loads it with the group's chunks, never with
//! the entry WebAssembly (`crate::locale`).

crate::locale::messages! {
    /// A string the definitions group draws, by its message in its catalog.
    DefinitionsMsg in "./locales/definitions" {
        /// `Template deleted`
        TemplateDeleted = "template-deleted",
        /// `Template views`
        TemplateViews = "template-views",
        /// `Template source`
        TemplateSource = "template-source",
        /// `Template uploaded`
        TemplateUploaded = "template-uploaded",
        /// `Term code`
        TermCode = "term-code",
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
        /// `OperationalTemplateV2 parse: '$e'`
        Operationaltemplatev2Parse = "operationaltemplatev2-parse",
        /// WebTemplate build: `$value0`
        WebtemplateBuild = "webtemplate-build",
        /// terminology JSON: `$e`
        TerminologyJson = "terminology-json",
        /// a `$what` is required
        AIsRequired = "a-is-required",
        /// `Contains: { $value }`
        ChipContains = "chip-contains",
        /// `Root archetype`
        ColumnRootArchetype = "column-root-archetype",
        /// `Template`
        ColumnTemplate = "column-template",
        /// `Uploaded`
        ColumnUploaded = "column-uploaded",
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
        /// `Download`
        TemplateDownload = "template-download",
        /// `Copy id`
        TemplateCopyId = "template-copy-id",
        /// `The CDR sent no operational template to build paths from.`
        TemplateNoCatalog = "template-no-catalog",
        /// `The CDR built no example for these options.`
        TemplateNoExample = "template-no-example",
        /// `A template rooted at { $root } has no example: the CDR builds examples for COMPOSITION and party templates.`
        TemplateNoExampleForRoot = "template-no-example-for-root",
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
        /// `Delete…`
        TemplateDeleteRow = "template-delete-row",
    }
}
