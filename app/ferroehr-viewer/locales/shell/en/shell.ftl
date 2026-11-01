# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1

# The English catalog of the words the shell and more than one group draw:
# the reference catalog and the one every lookup falls back to. Each message
# is one `Msg` of the crate's `locale` module; a catalog of another language
# holds the same messages in a directory named by its BCP 47 tag beside this
# one.

not-found-title = No such screen
not-found-body = The viewer has no page at { $path }. EHRs live under /ehrs.
committer-person = The person or their representative
adl-2 = ADL 2
adl-1-4 = ADL 1.4
input = Input
output = Output
complete = Complete
medium = Medium
required = Required
log-filter-needs-least-one-directive = a log filter needs at least one directive, e.g. `ferroehr=debug`
no-metric-name-given = no metric name given
db-connections-in-use = DB connections in use
aql-queries = AQL queries
compositions-committed = Compositions committed
no-current-version-known-reload-page = no current version is known: reload the page and retry
composition-body-empty = the composition body is empty
pick-date-time-travel = pick a date and time to travel to
party-id-required = a party id is required
version-uid-required-read-past-version = a version uid is required to read a past version
tag-key-required = a tag key is required
tag-key-required-item-tag-invariant = a tag key is required (ITEM_TAG invariant Inv_key_valid: the key is not empty)
cancel = Cancel
version-uid-required-read-past-ehr = a version uid is required to read a past EHR_STATUS version
tag-target-id-required = a tag target id is required
both-subject-id-subject-namespace-required = both a subject id and a subject namespace are required
provide-both-subject-id-subject-namespace = provide both a subject id and a subject namespace, or leave both empty
ehr-response-carried-no-ehr-id = EHR response carried no ehr_id
ehr-status = EHR Status
query-name = Name
no-adl2-artefact-id-delete = no ADL2 artefact id to delete
there-no-adl2-source-upload = there is no ADL2 source to upload
selected-file-was-empty = the selected file was empty
exact-version = Exact version
version-prefix-latest-match = Version prefix (latest match)
latest-version-no-version-sent = Latest version (no version sent)
no-subscription-id-delete = no subscription id to delete
no-subscription-id-update = no subscription id to update
subscription-name-required-may-hold-only = a subscription name is required and may hold only letters, digits, `_`, `.` and `-`
item-tag-list-json = ITEM_TAG list JSON: { $e }
z = { $with_seconds }Z
readiness-json = readiness JSON: { $e }
management-info-json = management info JSON: { $e }
metric-list-json = metric list JSON: { $e }
metric-detail-json = metric detail JSON: { $e }
loggers-json = loggers JSON: { $e }
loggers-json-2 = loggers JSON: { $e }
loggers-json-3 = loggers JSON: { $e }
composition-changed-since-page-loaded-reload = the composition changed since this page loaded: reload and retry. { $detail }
versioned-composition-json = versioned composition JSON: { $e }
version-json = version JSON: { $e }
revision-history-json = revision history JSON: { $e }
versioned-demographic-object-json = versioned demographic object JSON: { $e }
demographic-version-json = demographic version JSON: { $e }
ehr-json = EHR JSON: { $e }
ehr-status-json = ehr_status JSON: { $e }
versioned-ehr-status-json = versioned ehr_status JSON: { $e }
version-json-3 = version JSON: { $e }
ehr-json-2 = EHR JSON: { $e }
aql-result-json = AQL result JSON: { $e }
ehr-json-3 = EHR JSON: { $e }
opt-1-4-parse = OPT 1.4 parse: { $e }
webtemplate-build-2 = WebTemplate build: { $value0 }
template-list-json = template list JSON: { $e }
adl2-template-list-json = ADL2 template list JSON: { $e }
event-subscription-list-json = event subscription list JSON: { $e }
event-subscription-record-json = event subscription record JSON: { $e }
latest-version-s-full-object-version = the latest version's full OBJECT_VERSION_ID is required to delete a composition: reload this screen and retry
current-ehr-status-version-uid-required = the current EHR_STATUS version uid is required to update it: reload this tab and retry
no-value = (no value)
count-unavailable = Unavailable
count-no-row = The CDR answered the count with no row.
count-several-rows = The CDR answered the count with more than one row.
count-no-cell = The CDR answered the count with an empty row.
count-several-cells = The CDR answered the count with more than one value.
count-not-a-count = The CDR answered the count with a value that is no whole number.

audit-chip-agent = Principal: { $value }
audit-chip-outcome = Outcome: { $value }
audit-chip-action = Action: { $value }
audit-chip-patient = Patient: { $value }
chip-template = Template: { $value }
chip-composer = Composer: { $value }
sub-any-change = Any change
sub-any-kind = Any kind
sub-composition = Composition
sub-ehr-status = EHR status
sub-folder = Folder
## Numbers: the marks FerroUI's formatter groups a number with

number-group = ,
number-decimal = .

product-name = FerroEHR Viewer
brand-name = FerroEHR
brand-word = Viewer
nav-label = Viewer
nav-group-overview = Overview
nav-group-records = Records
nav-group-definitions = Definitions
nav-group-queries = Queries
nav-group-console = Console
nav-dashboard = Dashboard
nav-ehrs = EHRs
nav-parties = Parties
nav-templates = Templates
nav-terminology = Terminology
nav-stored-queries = Stored queries
nav-query = Query
nav-server = Server
nav-audit = Audit log
nav-subscriptions = Subscriptions
nav-configuration = Configuration
nav-about = About
pill-up = CDR up
pill-unreachable = CDR unreachable
pill-unknown = CDR unknown
pill-refused = CDR refused 401
pill-detail = FerroEHR { $version } at { $address }
pill-detail-profile = FerroEHR { $version } at { $address } · { $profile } deployment
pill-unreachable-detail = No answer from { $address }
unknown-version = an unknown version
sandbox-title = Sandbox
sandbox-gaps = { $count ->
    [one] 1 production safeguard is off
   *[other] { $count } production safeguards are off
}
sandbox-details = Details
sandbox-no-gaps = This deployment must not hold real patient data.
signed-in-basic = Signed in with a CDR account
signed-in-oidc = Signed in at the identity provider
your-menu = Your menu
your-access = Your access
search-label = Find in the CDR
search-placeholder = Id, name or subject id
search-submit = Find
search-waits = Search waits for the CDR
dock-label = Decision
discard = Discard
cdr-unreachable-code = CDR unreachable
invalid-code = Not sent
session-ended-code = Session ended
internal-code = Viewer error


page-refused = This page could not be read.

dashboard-no-stored-query = No stored query yet.
dashboard-write-one = Write one in Query

yes = Yes
no = No
column-created = Created

editor-save = Save
editor-utf-8 = UTF-8

column-version = Version
filter-template = Template
filter-from = From
filter-to = To
filter-apply = Apply
chip-from = From { $value }
chip-to = To { $value }

column-kind = Kind
change-creation = Creation
change-amendment = Amendment
change-modification = Modification
change-deleted = Deleted
staged-apply = Apply

tags-value = Value

composition-versions = Versions
composition-format = Format
composition-edit-title = Edit { $name }
editor-refused-heading = { $count ->
    [one] What the CDR refused · 1
   *[other] What the CDR refused · { $count }
}

parties-open = Open
party-edit = Edit
party-overview = Overview
party-facts = Facts

relationship-none = none

column-actions = Actions
template-actions-for = Actions for { $name }

# A day of an activity chart's axis, such as 22 Sep.
chart-day = { $day } { $month }
month-short-1 = Jan
month-short-2 = Feb
month-short-3 = Mar
month-short-4 = Apr
month-short-5 = May
month-short-6 = Jun
month-short-7 = Jul
month-short-8 = Aug
month-short-9 = Sep
month-short-10 = Oct
month-short-11 = Nov
month-short-12 = Dec
