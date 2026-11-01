# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1

# The English catalog of the console group: the reference catalog and the
# one every lookup falls back to. Each message is one `ConsoleMsg` of the
# crate's `locale::console` module; a catalog of another language holds the
# same messages in a directory named by its BCP 47 tag beside this one.

major-failure = major failure
serious-failure = serious failure
minor-failure = minor failure
import = Import
export = Export
authentication = Authentication
complete-surface = Complete surface
exact-match-only = Exact match only.
wildcard-pattern-matches-within-one-namespace = Wildcard pattern: * matches within one namespace segment, ** across segments.
all-available-templates-queries-including-ad = All available templates or queries: including ad-hoc AQL.
compositions-matching-template = Compositions of the matching template
operational-templates = Operational templates
backend-client-acting-without-user-context = A backend client acting without a user context: across all data.
subject-authenticated-user-s-security-profile = Subject to the authenticated user's security profile: not limited to the patient in context.
restricted-ehr-patient-context = Restricted to the EHR of the patient in context.
not-context-openehr-defines-carried-alongside = Not a context openEHR defines. Carried alongside the openEHR scopes and inert here.
asks-episode-care-context-experimental-specification = Asks for an episode of care in context. Experimental in the specification.
launch-context-episode = Launch context · episode
asks-launching-platform-put-one-patient = Asks the launching platform to put one patient/EHR in context; a patient-compartment scope is then read against that EHR.
launch-context-patient = Launch context · patient
cdr-decides-every-request-from-credential = The CDR decides every request from the credential this session holds.
viewer-session = Viewer session
cdr-resolves-session-s-roles-permissions = The CDR resolves this session's roles and permissions from the same access token; the scopes below are the ones the token was issued with.
oidc-bearer-token = OIDC bearer token
viewer-replays-account-s-cdr-credentials = The viewer replays this account's CDR credentials on every call, so the CDR applies that account's own privileges. A Basic session carries no SMART scopes.
basic-authentication = Basic authentication
openid-connect-scope-identifies-user-application = An OpenID Connect scope: it identifies the user to the application and reaches no clinical data.
marks-embedded-launch-advisory-only-cdr = Marks an embedded launch. Advisory only: the CDR performs no launch selection.
launch-marker = Launch marker
auditevent-bundle-json = AuditEvent Bundle JSON: { $e }
identity-claim = Identity claim · { $name }
launch-context = Launch context · { $other }
not-compartment-resource-scope-master08-defines = "{ $compartment }" is not a compartment. A resource scope is { $resource_scope_form }, and master08 defines exactly three compartments: patient, user, system.
carries-no-permission-tail-resource-scope = "{ $rest }" carries no .<permission> tail. A resource scope is { $resource_scope_form }: for example patient/composition-*.rs.
not-permission-tail-master08-allows-only = "{ $tail }" is not a permission tail. master08 allows only c (create), r (read), u (update), d (delete) and s (search), in any order: for example .rs or .crud.
not-resource-master08-defines-template-templateid = "{ $resource }" is not a resource. master08 defines template-<templateId>, composition-<templateId> and aql-<queryName>.
not-resource-scope-master08-form-example = Not a resource scope. The master08 form is { $resource_scope_form }: for example patient/composition-*.rs.
choice-any = any
choice-success = success
action-create = create
action-read = read
action-update = update
action-delete = delete
action-execute = execute

## Numbers: the marks FerroUI's formatter groups a number with

number-group = ,
number-decimal = .

your-preferences = Your preferences
brand-accent = Teal

about-subtitle = The viewer, the CDR it reads, and the manufacturer.
about-cdr = CDR
about-cdr-version = FerroEHR { $version }
about-open-ehr-rest = openEHR REST
about-rest-version = ITS-REST { $version }
about-ferro-ui = FerroUI
about-ferro-ui-version = ferrohealth-ui { $version }
about-licence = Business Source License 1.1
about-book = The FerroEHR book
about-manifest = The conformance manifest
about-manifest-line = What the CDR answers at OPTIONS on its base path.
about-manifest-refused = The conformance manifest could not be read.
about-no-state = The viewer's state is not in reach.

reveal-label = Show a revealed identifier for
reveal-description = How long a subject id stays shown after you reveal it; the viewer caps it at 2 minutes.
reveal-seconds = { $count } seconds
reveal-minutes = { $count ->
    [one] 1 minute
   *[other] { $count } minutes
}

access-subtitle = What this session may do.
access-who = Who you are
access-signed-in-as = Signed in as
access-how = How
access-how-basic = Basic sign-in with a CDR account
access-how-oidc = OIDC at the identity provider
access-decides = What decides
access-session = Session
access-session-ends = Ends after { $minutes } minutes without a request
access-scopes = Scopes
access-no-scopes = A Basic session carries no SMART scopes; the CDR checks the account's role.
access-preview = Read a scope
access-preview-intro = Paste a scope to see what it allows.
access-scope-field = Scope or claim
access-scope-placeholder = patient/composition-*.rs
access-read-it = Read it
access-preview-empty = Paste a scope to read it here.
access-previewed = Previewed scopes
scope-not-recognised = Not a SMART on openEHR scope.
scope-resource = { $family } { $pattern }

server-subtitle = The CDR this viewer reads: what it is, how it runs, and what it serves.
server-sections = Server sections
server-overview = Overview
server-overview-line = How the CDR was built and deployed.
server-health = Health
server-health-line = What the CDR depends on, and whether each part is healthy.
server-metrics = Metrics
server-metrics-line = Counters since the last start.
server-logging = Logging
server-logging-line = The live log filter of the instance that answered.
server-api = API
server-api-line = The OpenAPI documents the CDR serves for itself, by API family.
server-smart = SMART
server-smart-line = The scopes the CDR supports; { $access } shows the ones you hold.
server-no-management = The CDR has no management endpoints turned on, so Health, Metrics and Logging are not offered.
server-status = Status
server-state = State
server-up = Up
server-unknown = Unknown
server-unreachable = Unreachable
server-deployment = Deployment
server-production = Production
server-sandbox = Sandbox
server-version = Version
server-conformance = Versions and conformance
server-safeguard = Safeguard
server-safeguards-off = Production safeguards that are off
server-sandbox-intro = A sandbox must not hold real patient data.
server-build = Build
server-no-build-info = The CDR serves no build information.
server-build-refused = The build information could not be read.
server-dependencies = Dependencies
server-dependency = Dependency
server-detail = Detail
server-public-readiness = The CDR serves only each dependency's state, not its detail.
server-health-refused = The dependencies could not be read.
server-now = Now
server-metrics-refused = The metrics could not be read.
server-metric = Metric
server-show = Show
server-labels = Labels
server-samples-of = Samples of { $metric }
server-no-metric = The CDR has no such metric.
server-registry = The metric registry
server-metric-count = { $count ->
    [one] 1 metric
   *[other] { $count } metrics
}
server-filter = Filter
server-in-effect = In effect
server-at-boot = At boot
server-change-filter = Change the log filter
server-change-filter-intro = Applies to this instance until it restarts.
server-new-filter = New filter
server-apply = Apply…
server-reset = Reset to the boot filter
server-at-boot-already = The filter in effect is the boot filter.
server-apply-title = Apply the new log filter?
server-apply-message = It applies to this instance until it restarts.
server-apply-confirm = Apply the filter
server-filter-applied = Log filter applied
server-filter-reset = Log filter reset to the boot filter
server-filter-refused = The CDR refused the log filter.
server-no-logging = The CDR serves no log filter control.
server-logging-refused = The log filter could not be read.
server-api-family = API family
server-method-path = Method and path
server-summary = Summary
server-endpoint-count = { $count ->
    [one] 1 endpoint
   *[other] { $count } endpoints
}
server-endpoints-of = Endpoints of { $family }
server-open-api = The OpenAPI document
server-api-refused = The OpenAPI document could not be read.
server-scopes = Scopes it supports
server-sign-in = Sign-in
server-also = Also
server-grant-types = Grant types
server-pkce = PKCE
server-no-smart = The CDR serves no SMART discovery document.
server-smart-refused = The SMART discovery document could not be read.

audit-subtitle = The CDR's security audit log: who accessed what, and with what outcome.
audit-find = Principal
audit-outcome = Outcome
audit-action = Action
audit-when = When
audit-patient = Patient
audit-event = Event
audit-principal = Principal
audit-records = Audit records
audit-pages = Audit pages
audit-success = Success
audit-unknown = Unknown
audit-no-patient = No patient
audit-empty = The CDR has recorded no audit event yet.
audit-no-match = No audit record matches these filters.
audit-refused = The audit log could not be read.
audit-store-off = The CDR keeps no local audit store; its records go only to the forwarding target, so there is nothing to read here.
audit-store-off-line = Set in ferroehr.toml; the Configuration page shows the running value.
audit-record-gone = The audit log holds no record at this address any more; open it from the audit log again.
audit-what = What
audit-stored-as = Stored as a FHIR AuditEvent (IHE ITI-81).
audit-the-event = The event
audit-as-stored = As stored

sub-subtitle = Get a message on the event queue for each change that matches.
sub-new = New subscription
sub-off = The CDR's subscription API is off, so there is nothing to list.
sub-off-line = Set in ferroehr.toml; the Configuration page shows the running value.
sub-refused = The subscriptions could not be read.
sub-empty = No subscription yet.
sub-state-enabled = Enabled
sub-state-disabled = Disabled
sub-enable = Enable
sub-disable = Disable
sub-enabled = Subscription enabled
sub-disabled = Subscription disabled
sub-save-refused = The CDR refused the subscription.
sub-any-template = Any template
sub-every-version = Every committed version of any kind.
sub-every-of-kind = Every committed { $kind }.
sub-every-change = Every { $change } of any kind.
sub-every-change-of-kind = Every { $change } of a { $kind }.
sub-under-template = { $line } Under { $template } only.
sub-queue = Queue
sub-name-fixed = The name cannot change: a new name would be a different queue.
sub-name-help = Letters, digits, dots, dashes and underscores. The name cannot change later.
sub-name-invalid = Use letters, digits, dots, dashes and underscores only.
sub-name-taken = The CDR already has a subscription called { $name }. Choose another name.
sub-selects = What it selects
sub-change-type = Change type
sub-delivery = Delivery
sub-deliver = Deliver messages
sub-deliver-help = When off, the subscription is kept but sends nothing.
sub-create = Create subscription
sub-created = Subscription created
sub-created-line = { $name } · queue ferroehr.events.{ $name }
sub-editor-title = { $mode ->
    [new] New subscription
   *[edit] Edit { $name }
}
sub-edit-subtitle = Saving replaces what this subscription selects.
sub-none = The CDR has no subscription called { $name }.
sub-saved = Subscription saved
sub-more = More actions for the subscription
sub-delete = Delete the subscription
sub-delete-title = Delete the subscription { $name }?
sub-delete-message = Removes the subscription for good. The queue and its messages stay on the broker; remove them there.
sub-delete-confirm = Delete the subscription
sub-deleted = Subscription deleted
sub-delete-refused = The CDR refused to delete the subscription.

config-subtitle = The CDR's settings as it serves them, redacted, and the viewer's own.
config-sections = Configuration sections
config-how-title = How configuration works
config-how-reads = The CDR reads ferroehr.toml once at start, and the viewer reads ferroehr-viewer.toml; an environment variable overrides a key in either.
config-how-where-title = Where you see it
config-how-where = Here, read-only: the CDR's settings as it serves them, redacted, and the viewer's own.
config-how-change-title = How you change it
config-how-change = Change the file, review it, and restart the CDR or the viewer.
config-how-nothing = Nothing here can be changed from the viewer: the database holds clinical data, never settings.
config-deployment = Deployment
config-server = Server and database
config-access = Access
config-records = Records
config-integrations = Integrations
config-operations = Operations
config-viewer = Viewer
config-refused = The CDR's configuration could not be read.
config-on = On
config-off = Off
config-minutes = { $count } min
config-cdr-address = CDR address
config-cdr-address-line = The address the viewer calls for every CDR request.
config-cdr-base-path = CDR base path
config-cdr-base-path-line = Must match the base path the CDR serves its API under.
config-management-address = Management address
config-management-address-line = Where the viewer reads health, metrics and logging.
config-basic-sign-in = Basic sign-in
config-basic-sign-in-line = Offers the username and password form; the CDR checks the account.
config-oidc-sign-in = OIDC sign-in
config-oidc-sign-in-line = The identity provider that signs people in.
config-oidc-secret = OIDC client secret
config-oidc-secret-line = Proves the viewer to the identity provider; read from a file and never shown.
config-idle = Session idle time
config-idle-line = A session ends after this long without a request.
config-session-key = Session key
config-session-key-line = Seals the session cookie; every replica holds the same key.
config-language = Language
config-language-line = Used when the browser asks for a language the viewer does not have.
sub-delete-row = Delete…
