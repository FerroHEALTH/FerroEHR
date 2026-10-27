// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The words of the console group, in the catalogs under `locales/console/`.
//!
//! Only code of the console group looks them up, so their catalog travels
//! with that code: the browser loads it with the group's chunks, never with
//! the entry WebAssembly (`crate::locale`).

crate::locale::messages! {
    /// A string the console group draws, by its message in its catalog.
    ConsoleMsg in "./locales/console" {
        /// `Audit records`
        AuditRecords = "audit-records",
        /// `major failure`
        MajorFailure = "major-failure",
        /// `serious failure`
        SeriousFailure = "serious-failure",
        /// `minor failure`
        MinorFailure = "minor-failure",
        /// `Import`
        Import = "import",
        /// `Export`
        Export = "export",
        /// `Authentication`
        Authentication = "authentication",
        /// `Access scopes`
        AccessScopes = "access-scopes",
        /// `Complete surface`
        CompleteSurface = "complete-surface",
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
        /// `AuditEvent Bundle JSON: '$e'`
        AuditeventBundleJson = "auditevent-bundle-json",
        /// HTTP `$status`
        Http = "http",
        /// the CDR answered `$status` there
        TheCdrAnsweredThere = "the-cdr-answered-there",
        /// the CDR answered `$status` there: `$message`
        TheCdrAnsweredThere2 = "the-cdr-answered-there-2",
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
        /// SMART discovery could not be read: the CDR is unreachable (`$detail`). This says...
        SmartDiscoveryCouldNotReadCdr = "smart-discovery-could-not-read-cdr",
        /// SMART discovery could not be read: the CDR refused this session at `$endpoint`. Sign...
        SmartDiscoveryCouldNotReadCdr2 = "smart-discovery-could-not-read-cdr-2",
        /// SMART discovery could not be read: this session may not read `$endpoint`. It needs an...
        SmartDiscoveryCouldNotReadSession = "smart-discovery-could-not-read-session",
        /// SMART discovery could not be read from `$endpoint` — `$detail`. Whether SMART is...
        SmartDiscoveryCouldNotReadFrom = "smart-discovery-could-not-read-from",
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
        /// `Your preferences`
        YourPreferences = "your-preferences",
        /// `The viewer's state is not in reach.`
        AboutNoState = "about-no-state",
        /// `Action`
        AuditAction = "audit-action",
        /// `As stored`
        AuditAsStored = "audit-as-stored",
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
        /// `How configuration works`
        ConfigHowTitle = "config-how-title",
        /// `The CDR reads ferroehr.toml once at start, ...`
        ConfigHowReads = "config-how-reads",
        /// `Where you see it`
        ConfigHowWhereTitle = "config-how-where-title",
        /// `Here, read-only: the CDR's settings as it serves them, redacted, and the viewer's own.`
        ConfigHowWhere = "config-how-where",
        /// `How you change it`
        ConfigHowChangeTitle = "config-how-change-title",
        /// `Change the file, review it, and restart the CDR or the viewer.`
        ConfigHowChange = "config-how-change",
        /// `Nothing here can be changed from the viewer: the database holds clinical data, never settings.`
        ConfigHowNothing = "config-how-nothing",
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
        /// `Any template`
        SubAnyTemplate = "sub-any-template",
        /// `Change type`
        SubChangeType = "sub-change-type",
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
    }
}
