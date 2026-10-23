# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1

# The English catalog of the viewer's own words: the reference catalog and
# the one every lookup falls back to. Each message is one `Msg` of the
# crate's `locale` module; a catalog of another language holds the same
# messages in a directory named by its BCP 47 tag beside this one. The words
# the FerroUI components draw themselves are that library's catalog.

## The page for a path the viewer does not serve

not-found-title = No such screen
not-found-heading = Page not found
not-found-body = The viewer has no page at { $path }. EHRs live under /ehrs.

example = Example
path-catalog = Path catalog
aom2-json = AOM2 JSON
source = Source
choose-an-adl2-file = Choose an ADL2 file
choose-opt-xml-file = Choose an OPT/XML file
operational-template-adl-version-2-0 = operational_template (adl_version=2.0.6; rm_release=1.0.2; generated) …
template-xmlns-http-schemas-openehr-org = <template xmlns="http://schemas.openehr.org/v1"> …
upload-adl2-operational-template = Upload an ADL2 operational template
upload-an-operational-template = Upload an operational template
upload-adl2 = Upload ADL2
upload-opt = Upload OPT
adl-2 = ADL 2
adl-1-4 = ADL 1.4
no-ehr-id-delete = no EHR id to delete
stored-query-delete-needs-both-qualified = a stored-query delete needs both a qualified name and a version
no-template-id-delete = no template id to delete
return-to-the-dashboard = Return to the dashboard
desc = {" "}DESC
asc = {" "}ASC
did-not-parse-data-path = did not parse as a data path
row-order = Row order
viewer = Viewer
loading-the-rows = Loading the rows…
example-form = Example form
form = Form
detail-level = Detail level
format = Format
input = Input
output = Output
complete = Complete
medium = Medium
required = Required
document = Document
filter = Filter
target-path = Target path
value = Value
key = Key
tag-identified-key-target-path-together = A tag is identified by its key and target path together, so the same key on two different paths is two tags. Deleting addresses the key alone and removes both.
save-tag = Save tag
target-path-optional = Target path (optional)
value-optional = Value (optional)
set-a-tag = Set a tag
tag-needs-key-item-tag-invariant = A tag needs a key (ITEM_TAG invariant Inv_key_valid).
delete = Delete
actions = Actions
target = Target
item-tags = Item tags
collection = Collection:{" "}
delete-failed = Delete failed
deleted-version-was-committed-earlier-versions = A deleted version was committed; earlier versions stay readable in History.
x-axis = X axis
result-set-chart = The result set as a chart
one-scope-whole-space-separated-claim = One scope or a whole space-separated claim. The form is <compartment>/<resource>.<permission>: nothing is sent to the CDR, and nothing here changes what you may do.
patient-composition-rs-user-template-myhospital = patient/composition-*.rs user/template-MyHospital::Template.v0.crud
preview-a-scope = Preview a scope
not-recognised = not recognised
not-scope-openehr-defines-carried-alongside = Not a scope openEHR defines. It is carried alongside the openEHR scopes and grants nothing here.
broad-access = broad access
version-document = Version document
open-that-version = Open that version
date-time-interpreted-utc = Date and time (interpreted as UTC)
committer = Committer
change-type = Change type
committed = Committed
open = Open
version = Version
revision-history = Revision history
loading = Loading…
multimedia-files-stored-over-connection-without = multimedia files are stored over a connection without encryption
access-records-sent-audit-repository-without = access records are sent to the audit repository without encryption
change-events-published-message-broker-without = change events are published to a message broker without encryption
listener-serves-plain-http-address-reachable = a listener serves plain HTTP on an address reachable from the network
requests-served-without-authentication = requests are served without authentication
schema-preparation-runs-clinical-runtime-credential = schema preparation runs on the clinical runtime credential
access-not-recorded-durable-audit-trail = access is not recorded to a durable audit trail
two-domains-reach-same-postgresql-cluster = two domains reach the same PostgreSQL cluster
clinical-demographic-linkage-domains-share-one = the clinical, demographic and linkage domains share one database credential
text-csv-charset-utf-8 = text/csv; charset=utf-8
parameters-must-json-object = parameters must be a JSON object
log-filter-needs-least-one-directive = a log filter needs at least one directive, e.g. `ferroehr=debug`
no-metric-name-given = no metric name given
openehr-specification-pins = openEHR specification pins
db-connections-in-use = DB connections in use
aql-queries = AQL queries
compositions-committed = Compositions committed
in-flight-requests = In-flight requests
older = Older{" "}
newer = {" "}Newer
record = Record
resource = Resource
patient = Patient
principal = Principal
outcome = Outcome
operation = Operation
event-2 = Event
recorded = Recorded
major-failure = major failure
serious-failure = serious failure
minor-failure = minor failure
action-2 = Action
patient-id = Patient id
to-instant = To (instant)
from-instant = From (instant)
audit-log = Audit log
import = Import
export = Export
authentication = Authentication
change-type-2 = change type
audit = Audit
current = {" "}· current
version-timeline = Version timeline
latest = Latest
at-time = At time
time-travel-interpreted-utc = Time travel (interpreted as UTC)
no-version-composition-existed-time = No version of this composition existed at that time.
versions-unavailable = versions unavailable
reading-the-versions = Reading the versions
committing = Committing…
commit-new-version = Commit new version
composition-canonical-json = Composition (canonical JSON)
switch-canonical-json-edit = Switch to canonical JSON to edit.
commits-top-latest-version = Commits on top of the latest version.
close-editor = Close editor
edit-as-new-version = Edit as new version
commit-needs-latest-version-id-precondition = The commit needs the latest version id as its precondition: try again in a moment.
version-history-still-loading = Version history still loading
preceding-version = preceding version
owner-ehr = owner EHR
versioned-object = versioned object
versioned-object-2 = Versioned object
none-deleted-version = none (deleted version)
delete-composition = Delete composition
deletes-the-latest-version = Deletes the latest version
deletes-the-latest-version-2 = Deletes the latest version.
ehrs = EHRs
this-composition = this composition
composition-was-logically-deleted-version-history = The composition was logically deleted: its version history stays readable.
composition-deleted = Composition deleted
the-new-composition-version = the new composition version
commit-failed = Commit failed
new-version-committed = New version committed
new-version-was-committed = A new version was committed.
no-current-version-known-reload-page = no current version is known: reload the page and retry
composition-body-empty = the composition body is empty
pick-date-time-travel = pick a date and time to travel to
commit-composition-trend-appears-here-from = Commit a composition and the trend appears here from the next day onward.
no-commit-activity-yet = No commit activity yet
n-1-query = 1 query
go-to-stored-queries = Go to stored queries
stored-queries = Stored queries
templates = Templates
compositions = Compositions
commit-activity = Commit activity
query-namespaces = Query namespaces
dashboard = Dashboard
open-party = Open party
demographic-tags = Demographic tags
tag-pages = Tag pages
demographic-tag-list-spans-every-party = The demographic tag list spans every party kind, and a tag names its target without naming that target's kind: so opening a row asks the CDR which kind holds it.
tag-index-all-kinds = Tag index (all kinds)
creating = Creating…
document-below-smallest-party-kind-accepts = The document below is the smallest party this kind accepts: every attribute in it is mandatory. Replace the archetype ids and the identity details with your own, then create.
create-failed = Create failed
party-created = Party created
identity-details = identity details
legal-identity = legal identity
either-id-form-works-versioned-object = Either id form works: a versioned_object_uid opens the latest version, and a full version_uid is reduced to its container: every screen here addresses the container.
versioned-object-uid-version-uid = versioned_object_uid or version_uid
party-kinds = Party kinds
the-five-kinds-are = {"."} The five kinds are:
demographic-api-has-no-party-kind = The demographic API has no party kind called{" "}
unknown-party-kind = Unknown party kind
type-2 = Type
versions-in-this-contribution = Versions in this contribution
versions = Versions
commit = Commit
demographics = Demographics
cdr-holds-no-demographic-contribution-id = The CDR holds no demographic contribution with this id. A contribution committed against an EHR is read on that EHR's own screen instead.
contribution-uid-required = a contribution uid is required
no-version-existed-time = No version existed at that time.
version-point-time = Version at a point in time
newest-first-open-version-read-document = Newest first. Open a version to read the document exactly as it stood at that commit.
every-demographic-object-created-first-version = Every demographic object is created with a first version; if none is listed, the CDR reported no revision history for this id.
no-versions = No versions
party-id-required = a party id is required
version-uid-required-read-past-version = a version uid is required to read a past version
roles = Roles
people = People
organisations = Organisations
groups = Groups
agents = Agents
party-deleted = Party deleted
delete-party = Delete party
saving = Saving…
save-party = Save party
details-canonical-json-item-structure-leave = details (canonical JSON ITEM_STRUCTURE, leave blank to remove)
identities-canonical-json-array-party-identity = identities (canonical JSON array of PARTY_IDENTITY, at least one)
edit-party = Edit party
relationships-inline = relationships (inline)
party-s-current-version-deleted-earlier = This party's current version is deleted. Its earlier versions are still readable: open one from the History tab.
cdr-holds-no-party-kind-id = The CDR holds no party of this kind with this id. Check the id and the kind: the same id under a different kind is a different resource.
save-failed = Save failed
party-changed-server = Party changed on the server
party-updated = Party updated
tags = Tags
relationships = Relationships
history-2 = History
party = Party
loaded-party-document-not-json-object = the loaded party document is not a JSON object: reload this tab and retry
identities-must-json-array-party-identity = identities must be a JSON array of PARTY_IDENTITY objects
party-body-must-json-object = the party body must be a JSON object
party-kind = party kind
current-version-uid-required-update-party = the current version uid is required to update this party: reload this tab and retry
cdr-created-party-but-returned-no = the CDR created the party but returned no uid to open it by
this-party = this party
archetype = Archetype
own-uid = Own uid
create-a-relationship = Create a relationship
creates-party-relationship-resource-party-source = Creates a PARTY_RELATIONSHIP resource with this party as its source. That resource is versioned in its own right: an extension of this CDR, not part of the released openEHR demographic API.
relate-this-party = Relate this party
these-come-from-party-document-itself = These come from the party document itself, which carries the relationships this party is the source of. openEHR models the other direction as a derived attribute the CDR leaves unpopulated, so no request can list the relationships that point AT this party: open those by their own id.
relationships-as-source = Relationships (as source)
party-s-current-version-deleted-so = This party's current version is deleted, so it has no relationships to show and cannot be related to another party.
relationship-deleted = This relationship is deleted; its versions stay readable.
delete-relationship = Delete relationship
save-relationship = Save relationship
relationship-type = Type
commits-new-version-top-one-loaded = Commits a new version on top of the one loaded above (If-Match). The two ends and the time validity travel back exactly as the CDR served them: a relationship between different parties is a different relationship.
edit-relationship = Edit relationship
relationship-needs-type-openehr-stores-relationship = A relationship needs a type: openEHR stores it as the relationship's name.
time-validity = time validity
no-relationship-id-relationship-routes-extension = No relationship with this id. Relationship routes are an extension of this CDR, so a server built without them answers the same 404.
relationship-s-current-version-deleted-earlier = This relationship's current version is deleted. Its earlier versions are still readable: open one from the History tab.
relationship-changed-server = Relationship changed on the server
relationship-updated = Relationship updated
relationship = Relationship
relationship-views = Relationship views
kind = Kind
create-relationship = Create relationship
details-canonical-json-item-structure-optional = details (canonical JSON ITEM_STRUCTURE, optional)
target-party-id = Target party id
source-party-id = Source party id
archetype-node-id = Archetype node id
both-ends-name-party-versioned-object = Both ends name a party by its versioned_object_uid: a relationship refers to the party's version container, not to one of its versions. The type you give is stored as the relationship's name, which is what openEHR reads it as.
the-new-relationship = the new relationship
relationship-created = Relationship created
relationship-reached-own-id-demographic-api = A relationship is reached by its own id: the demographic API publishes no way to ask which relationships point at a given party.
party-relationship-id = PARTY_RELATIONSHIP id
open-a-relationship = Open a relationship
party-relationships = Relationships
abstract-party-type-has-no-per = (an abstract party type has no per-kind route)
loaded-relationship-document-not-json-object = the loaded relationship document is not a JSON object: reload this screen and retry
relationship-type-required-party-relationship-invariant = a relationship type is required (PARTY_RELATIONSHIP invariant Type_validity)
archetype-node-id-required-locatable-archetype = an archetype node id is required (LOCATABLE.archetype_node_id)
cdr-created-relationship-but-returned-no = the CDR created the relationship but returned no uid to open it by
relationship-id-required = a relationship id is required
this-relationship = this relationship
tags-free-key-value-markers-client = Tags are free key/value markers a client sets on a party; add one below.
no-tags-party = No tags on this party
this-party-s-tag = this party's tag
tag-delete-failed = Tag delete failed
every-tag-carrying-key-was-removed = Every tag carrying that key was removed.
tag-deleted = Tag deleted
this-party-s-tags = this party's tags
tag-save-failed = Tag save failed
party-s-tag-collection-was-replaced = The party's tag collection was replaced with the merged list.
tag-saved = Tag saved
tag-key-required = a tag key is required
tag-key-required-item-tag-invariant = a tag key is required (ITEM_TAG invariant Inv_key_valid: the key is not empty)
commit-1-change-one-contribution = Commit 1 change as one contribution
commit-as-one-contribution = Commit as one contribution
open-contributions-tab = Open it in the Contributions tab →
nothing-was-committed-your-staged-changes = Nothing was committed and your staged changes are untouched. The CDR refused the change set as a whole: its diagnostic follows verbatim. It always names the defect, and a shape defect also names the member index (counting from the top of the list above); a validation refusal does not, so read it against the staged documents.
why-these-changes-belong-together = why these changes belong together
description = Description
defaults-your-viewer-identity = defaults to your viewer identity
committer-optional = Committer (optional)
remove = Remove
preview = Preview
staged-changes = Staged changes
change = Change
stage-this-change = Stage this change
paste-canonical-json-document-pick-target = Paste the canonical JSON document, or pick a target above to load it.
document-canonical-json = Document (canonical JSON)
not-loaded-yet = not loaded yet
preceding-version-2 = Preceding version:{" "}
pick-a-composition = Pick a composition
loading-compositions = Loading compositions…
composition-2 = Composition
from-the-document = From the document
loading-templates = Loading templates…
template = Template
document-ehr-s-current-status-loaded = The document is the EHR's current status, loaded for you: edit it and it commits as a new EHR_STATUS version.
add-a-change = Add a change
staged-changes-live-browser-tab-only = Staged changes live in this browser tab only. The viewer stores nothing of its own, so leaving this screen discards them; nothing reaches the CDR until you commit.
contribution-openehr-s-atomic-change-set = A CONTRIBUTION is openEHR's atomic change set: every staged change is committed together, or none of them is. Stage the changes that belong together: a new composition and the EHR status it goes with: then commit once.
commit-several-changes-one-contribution = Commit several changes as one contribution
this-contribution = this contribution
committer-required-contribution-audit = a committer is required on a CONTRIBUTION audit
stage-least-one-change-before-committing = stage at least one change before committing
creation-commits-new-versioned-object-so = a creation commits a NEW versioned object, so it carries no preceding version
no-preceding-version-known-yet-pick = no preceding version is known yet: pick the target and wait for it to load
document-must-json-object-canonical-openehr = the document must be a JSON object: a canonical openEHR document
document-empty-paste-load-body-commit = the document is empty: paste or load the body to commit
new-composition = New composition
ehr-status-modify = EHR status: modify
composition-amend = Composition: amend
composition-create = Composition: create
the-to-date = The “to” date
the-from-date = The “from” date
started = Started
name = Name
composer = Composer
widen-date-window-shorten-template-composer = Widen the date window, shorten the template or composer text, or clear the filters.
no-compositions-match-these-filters = No compositions match these filters
commit-one-form-above-through-cdr = Commit one with the form above, or through the CDR's REST API.
no-compositions-ehr = No compositions in this EHR
paste-composition-document-format-chosen-above = Paste the composition document in the format chosen above.
template-id-required-flat = template id (required for FLAT)
template-id = Template id
canonical-xml = Canonical XML
canonical-json = Canonical JSON
commit-composition = Commit composition
template-composer-match-anywhere-value-dates = Template and composer match anywhere in the value; the dates bound the composition's context start time, each covering its whole UTC day.
clear = Clear
part-composer-s-name = part of the composer's name
to-utc-date = To (UTC date)
from-utc-date = From (UTC date)
part-template-id = part of the template id
the-composition = the composition
composition-committed = Composition committed
the-composition-was-committed = The composition was committed.
template-id-required-commit-flat-composition = a template id is required to commit a FLAT composition
enter-contribution-uid-look-up = Enter a contribution uid to look it up.
look-up = Look up
contribution-uid-uuid = contribution uid (UUID)
contribution-uid = Contribution uid
contribution-s-total = {" "}contribution(s) total
contribution-2 = Contribution
contributions = Contributions
next = Next →
previous = ← Previous
contribution-activity = Contribution activity
every-write-ehr-contribution-timeline-fills = Every write to this EHR is a contribution; the timeline fills as they are committed.
no-contribution-activity-yet = No contribution activity yet
create-directory = Create directory
will-be-committed = Will be committed
ehr-has-no-directory-yet-create = This EHR has no directory yet. Create the empty root folder, then add sub-folders and item references in the tree editor.
your-unsaved-changes-kept-load-server = Your unsaved changes are kept. Load the server version or save anyway from the banner.
directory-changed-server = Directory changed on the server
the-directory-was-deleted = The directory was deleted.
directory-deleted = Directory deleted
the-ehr-s-directory = the EHR's directory
directory-updated = Directory updated
restore-failed = Restore failed
directory-restored = Directory restored
directory-created = Directory created
directory-disappeared-while-resolving-conflict = the directory disappeared while resolving the conflict
a-path-is-required = a path is required
date-time-required = a date and time is required
current-directory-version-uid-required-delete = the current directory version uid is required to delete
current-directory-version-uid-required-update = the current directory version uid is required to update
directory-body-empty = the directory body is empty
the-directory-was-committed = The directory was committed.
enter-path-view-only-sub-folder = Enter a path to view only that sub-folder of the directory.
path-e-g-folder-folder-b = Path (e.g. folder-a/folder-b)
sub-folder-by-path = Sub-folder by path
pick-date-time-view-directory-stood = Pick a date and time to view the directory as it stood then.
directory-point-time = Directory at a point in time
restore-this-version = Restore this version
load-older-versions = Load older versions
select-version-preview = Select a version to preview it.
loading-versions = Loading versions…
version-history = Version history
cancel = Cancel
delete-directory = Delete directory
permanently-delete-ehr-s-directory-creates = Permanently delete this EHR's directory? This creates a deletion version.
path-query = Path query
directory = Directory
no-compositions-ehr-yet = No compositions in this EHR yet.
add-reference = Add reference
id-value = id value
id-value-2 = Id value
id-type-e-g-hier-object = id type (e.g. HIER_OBJECT_ID)
id-type = Id type
type-e-g-composition = type (e.g. COMPOSITION)
reference-type = Reference type
namespace-e-g-local = namespace (e.g. local)
namespace = Namespace
enter-reference-manually = Or enter a reference manually
this-ehr-s-compositions = This EHR's compositions
close = Close
add-item-reference = Add item reference
remove-item = Remove item
rename-folder = Rename folder
add-subfolder = Add subfolder
delete-folder = Delete folder
new-folder = New folder
cancel-rename = Cancel rename
confirm-rename = Confirm rename
folder-name = Folder name
toggle-folder = Toggle folder
discard-changes = Discard changes
save-directory = Save directory
discard-my-edits-load-server-version = Discard my edits and load the server version
save-anyway-overwrite-server-change = Save anyway (overwrite the server change)
directory-changed-server-since-was-loaded = This directory changed on the server since it was loaded. Your unsaved edits are still here.
apply-to-tree = Apply to tree
hide-json = Hide JSON
advanced-edit-as-json = Advanced: edit as JSON
status-history = Status history
status = Status
ehr-views = EHR views
delete-ehr = Delete EHR
ehr-deleted = EHR deleted
cdr-holds-no-ehr-id-check = The CDR holds no EHR with this id: check the id, or create it on the EHRs screen.
save-status = Save status
other-details-canonical-json-item-structure = other_details (canonical JSON ITEM_STRUCTURE, leave blank to remove)
allow-new-content-committed-ehr = Allow new content to be committed to this EHR.
include-ehr-population-queries-aql = Include this EHR in population queries (AQL).
commits-new-ehr-status-version-top = Commits a new EHR_STATUS version on top of the one loaded above (If-Match), so a concurrent change is refused rather than overwritten. Every other attribute: the subject included: is sent back exactly as the CDR served it.
edit-status = Edit status
loaded-ehr-status-document-not-json = the loaded EHR_STATUS document is not a JSON object: reload this tab and retry
versioned-ehr-status = Versioned EHR_STATUS
no-ehr-status-version-existed-time = No EHR_STATUS version existed at that time.
ehr-status-point-time = EHR_STATUS at a point in time
newest-first-open-version-read-ehr = Newest first. Open a version to read the EHR_STATUS document exactly as it stood at that commit.
every-ehr-created-first-ehr-status = Every EHR is created with a first EHR_STATUS version; if none is listed, the CDR did not report a revision history for this EHR.
no-ehr-status-versions = No EHR_STATUS versions
ehr-not-queryable-aql-over-returns = This EHR is not queryable: AQL over it returns nothing. Tick “is_queryable” below and save to bring it back into population queries.
self-no-external-subject-reference = self: no external subject reference
ehr-status-changed-server = EHR status changed on the server
ehr-status-updated = EHR status updated
new-ehr-status-version-was-committed = A new EHR_STATUS version was committed.
version-uid-required-read-past-ehr = a version uid is required to read a past EHR_STATUS version
the-ehr-s-status = the EHR's status
tagged-objects = Tagged objects
tagged-object = tagged object
tagged-objects-2 = tagged objects
tagged-object-pages = Tagged object pages
every-tag-every-object-ehr-grouped = Every tag on every object in this EHR, grouped by the object it is on. A tag names its target without naming that target's kind, so opening a group asks the CDR which object holds that id. The container form and one version of the same object are separate collections and appear as separate groups.
tags-in-this-ehr = Tags in this EHR
tag-collection-was-replaced-merged-list = The tag collection was replaced with the merged list.
no-tags-ehr-status = No tags on this EHR status
ehr-status-s-tags = this EHR status's tags
no-tags-collection = No tags on this collection
this-composition-s-tags = this composition's tags
tag-target-id-required = a tag target id is required
pages = Pages
created = Created
ehr-id = EHR ID
no-ehr-found-subject = No EHR found for that subject.
searching = Searching…
find-by-subject = Find by subject
subject-namespace = Subject namespace
external-subject-id = external subject id
subject-id = Subject id
find = Find
ehr-id-uuid = ehr_id (UUID)
ehr-id-2 = EHR id
find-an-ehr = Find an EHR
supply-ehr-id-create-exact-ehr = Supply an EHR id to create that exact EHR: it must be a UUID, and an id already in use is refused by the CDR.
leave-subject-fields-blank-anonymous-ehr = Leave the subject fields blank for an anonymous EHR (the CDR mints the default EHR_STATUS), or set both to bind a subject.{" "}
create-ehr = Create EHR
subject-namespace-optional = Subject namespace (optional)
subject-id-optional = Subject id (optional)
ehr-id-uuid-blank-lets-cdr = UUID, or blank
ehr-id-optional = EHR id (optional)
provide-both-subject-id-namespace-leave = Provide both a subject id and a namespace, or leave both empty for an anonymous EHR.
the-new-ehr = the new EHR
ehr-created = EHR created
both-subject-id-subject-namespace-required = both a subject id and a subject namespace are required
provide-both-subject-id-subject-namespace = provide both a subject id and a subject namespace, or leave both empty
ehr-response-carried-no-ehr-id = EHR response carried no ehr_id
ehr-status = EHR Status
sign-in-with-oidc = Sign in with OIDC
wrong-username-or-password = Wrong username or password.
your-session-ended-sign-again-continue = Your session ended. Sign in again to continue.
loading-sign-methods = Loading the sign-in methods…
server-side-management-each-endpoint-takes = server-side ([management], each endpoint takes its own access level).
not-available-cdr-does-not-serve = {" "}is not available: the CDR does not serve this management endpoint. Enable it{" "}
reset-filter = Reset filter
restore-cdr-s-boot-log-filter = Restore the CDR's boot log filter? Any filter applied since startup is discarded.
reset-log-filter = Reset log filter
apply-filter = Apply filter
apply-log-filter = Apply log filter
takes-effect-immediately-cdr-instance-only = Takes effect immediately, for this CDR instance only, until it restarts.
reset-to-boot-filter = {" "}Reset to boot filter
apply-filter-2 = {" "}Apply filter
new-filter-directives = New filter directives
log-level = Log level
log-control = Log control
runtime-configuration = Runtime configuration
open-runtime-configuration = {" "}Open runtime configuration
configured-for = configured for.
exactly-one-place-system-screen-over = in exactly one place: the System screen, over the API base URL it is always{" "}
same-snapshot-management-surface-admin-api = same snapshot on its management surface and its admin API, so the viewer reads it{" "}
cdr-s-effective-configuration-every-secret = The CDR's effective configuration, with every secret redacted. The CDR serves the{" "}
metric-samples = Metric samples
labels = Labels
inspect = Inspect
metric = Metric
metrics = Metrics
build-spec-provenance = Build & spec provenance
build-provenance = Build provenance
other-question-cdr-s-readiness-probe = This is the other question: the CDR's readiness probe, one row per dependency it checks.
topbar-pill-reports-api-itself-status = The topbar pill reports the API itself (status document: reachable, and at which version).{" "}
which-deployment-does-not-offer-session = , which this deployment does not offer to this session.
statuses-only-detail-behind-each-one = Statuses only: the detail behind each one is served by the CDR's{" "}
health-indicators = Health indicators
indicator = Indicator
detail = Detail
dependency-health = Dependency health
operations = Operations
the-log-filter = the log filter
log-filter-not-reset = Log filter not reset
log-filter-reset = Log filter reset
log-filter-not-applied = Log filter not applied
log-filter-applied = Log filter applied
chosen-when-you-save-viewer-stores = ), chosen when you save it. The viewer stores no grouping of its own, so every openEHR client sees the same one.
query-s-group-namespace-qualified-name = A query's group is the namespace of its qualified name (
namespaces = Namespaces
loading-namespaces = Loading namespaces…
run-ad-hoc-aql = Run as ad-hoc AQL
loading-query = Loading query…
delete-from-cdr = Delete from CDR
open-in-builder = Open in builder
open-in-editor = Open in editor
run = Run
saved = Saved
stored-query = stored query
stored-queries-2 = stored queries
stored-query-pages = Stored query pages
delete-stored-query-from-cdr = Delete stored query from the CDR
raw-aql = Raw AQL
new-query = New query
queries = Queries
stored-query-deleted = Stored query deleted
results = Results
save = Save
parameters-json-object = Parameters (JSON object)
aql-is-valid = AQL is valid.
validating = Validating…
validate = Validate
example-select-c-from-ehr-e = For example: SELECT c FROM EHR e CONTAINS COMPOSITION c
run-with-parameters = Run with parameters
immutable-so-next-one-proposed = immutable, so the next one is proposed.
saving-stores-version-field-below-version = {"."} Saving stores the version in the field below: that version is{" "}
at-version = {" "}at version{" "}
loaded-stored-query = Loaded stored query{" "}
loading-stored-query = Loading stored query…
the-stored-query = the stored query
query-saved = Query saved
next-2 = Next{" "}
previous-2 = {" "}Previous
table = Table
chart = Chart
results-view = Results view
query-results = Query results
matching-rows = matching rows
exports-query-s-own-limit-window = Exports the query's own LIMIT window, or the server default.
export-json = Export JSON
export-csv = Export CSV
saves-as = Saves as{" "}
version-optional = Version (optional)
query-name = Name
namespace-optional = Namespace (optional)
server-assigns-version-replacing-query-stored = the server assigns the version, replacing the query stored at it
open-in-raw-editor = Open in raw editor{" "}
aql-preview = AQL preview
limit = Limit
add = + add
order-by = Order by
columns = Columns
add-projection-columns-column-path-catalog = Add projection columns with "+ column" in the path catalog.
ehrs-cohort = EHRs (cohort)
count = Count
data-values = Data values
no-ordinal-steps-template-node = No ordinal steps in the template for this node.
value-set-optional = value set (optional)
in-this-terminology = {" "}in this terminology.
no-value-set = No value set{" "}
look-up-2 = look up
no-coded-options-template-node = No coded options in the template for this node.
max-numerator = max numerator
min-numerator = min numerator
max-count = max count
min-count = min count
any-units = (any units)
max-magnitude = max magnitude
min-magnitude = min magnitude
presence-check-only-type-matches-when = Presence check only for this type: matches when the node exists.
add-here = add here
group-2 = + group
remove-this-group = Remove this group
clear-all-conditions = Clear all conditions
remove-this-condition = Remove this condition
column = + column
condition = + condition
pick-template-browse-paths = Pick a template to browse its paths.
any-template = Any template
builder-was-left-empty-rather-than = , the builder was left empty rather than loading a query it would change.
open-raw-aql-editor-instead = Open it in the raw AQL editor instead
saving-stores-version-field-below-version-2 = {"."} Saving stores the version in the field below: that version is immutable, so the next one is proposed.
link-does-not-name-stored-query = That link does not name a stored-query version, so the builder started empty.
criteria = Criteria
query-builder = Query builder
query-sets-own-row-window-aql = This query sets its own row window (AQL LIMIT/TOP), so the run is not paged.
field-left-blank-not-sent-all = ). A field left blank is not sent at all.
boolean-anything-else-sent-text-quote = {" "}as a boolean; anything else is sent as text. Quote a value to force text (
as-a-number = {" "}as a number,{" "}
value-reads-json-sent-type = A value that reads as JSON is sent as that type:{" "}
query-declares-no-parameters-so-runs = This query declares no parameters, so it runs as stored.
reading-parameters = Reading parameters…
parameters = Parameters
sends = Sends{" "}
exactly-that-version = , exactly that version.
partial-pattern-so-cdr-runs-latest = , a partial pattern, so the CDR runs the latest version matching it.
no-version-so-cdr-runs-latest = , no version, so the CDR runs the latest one.
resolution = Resolution
as-stored-at-version = {" "}as stored at version{" "}
link-does-not-name-stored-query-2 = That link does not name a stored-query version.
definition = Definition
and-choose-run = {" "}and choose Run.
no-stored-query-run-pick-one = No stored query to run. Pick one from{" "}
run-stored-query = Run stored query
main = Main
scopes-on-this-session = Scopes on this session
signed-in-with = Signed in with
identity = Identity
no-scopes-basic-authentication-grants-full = No scopes: Basic authentication grants full viewer access.
signed-viewer = Signed in to this viewer
view-scopes = View scopes
sandbox-deployment = Sandbox deployment
sandbox-deployment-2 = Sandbox deployment:{" "}
cdr-down = CDR DOWN
session-ended = Session ended
cdr-degraded = CDR DEGRADED
checking-the-cdr = Checking the CDR
platform = Platform
clinical-data = Clinical data
system = System
subscriptions = Subscriptions
terminology = Terminology
delete-subscription = Delete subscription
edit = Edit
state = State
subscription = Subscription
event-subscriptions = event subscriptions
enabled = Enabled
blank-any = blank = any
saving-replaces-every-predicate-one-cleared = Saving replaces every predicate: one cleared here becomes “any” on the CDR.
save-subscription = Save subscription
name-cdr-s-queue-key-cannot = The name is the CDR's queue key and cannot be changed.
subscription-id = Subscription id:{" "}
create-subscription = Create subscription
create-a-subscription = Create a subscription
subscription-deleted = Subscription deleted
subscription-updated = Subscription updated
update-failed = Update failed
subscription-created = Subscription created
open-audit-browser = {" "}Open audit browser
filterable-time-window-patient-principal-outcome = filterable by time window, patient, principal, outcome and action.
who-accessed-what-what-outcome-atna = Who accessed what, with what outcome: the ATNA security audit trail,{" "}
show = Show
api-family = API family
served-openapi = Served OpenAPI
cdr-serves-no-document-api-family = This CDR serves no document for that API family: pick another, or the complete surface.
scopes = Scopes
pkce-methods = PKCE methods
grant-types = Grant types
introspection = Introspection
token = Token
authorization = Authorization
smart-configuration-returned-but-not-valid = SMART configuration returned, but it is not valid JSON.
smart-not-enabled-cdr-serves-no = SMART is not enabled on this CDR: it serves no service-discovery document.
runtime-configuration-redacted = Runtime configuration (redacted)
cdr-no-longer-accepts-session-sign = The CDR no longer accepts this session: sign in again to read the configuration.
session-lacks-admin-role-configuration-view = This session lacks the ADMIN role: the configuration view needs an admin sign-in.
cdr-s-admin-api-disabled-enable = The CDR's admin API is disabled: enable [admin] to expose the configuration view.
repository-usage = Repository usage
compositions-per-template = Compositions per template
product-openehr-rest-versions-status-card = The product and openEHR REST versions are in the Status card: the manifest and the status document report the same versions, so the viewer reads them in one place.
cdr-advertises-no-api-groups-manifest = This CDR advertises no API groups in its manifest.
mounted-api-groups = Mounted API groups
conformance-profile = conformance profile
conformance-manifest = Conformance manifest
manufacturer = Manufacturer
postal-address = postal address
complete-surface = Complete surface
example-composition = example composition
operationaltemplatev2-json = OperationalTemplateV2 JSON
adl2-source = ADL2 source
adl2-templates = ADL2 templates
node-inspector = Node inspector
path-catalog-2 = path catalog
path-catalog-wt-tree = Path catalog (WT tree)
show-version = Show version
as-stored = as stored
release-version-1-1-0-1 = Release version (1.1.0, or a 1 / 1.0 prefix)
as-stored-2 = As stored
listing-holds-no-other-version-template = The listing holds no other version of this template family.
adl2-template-views = ADL2 template views
node-id = node id
select-node-inspect-path-metadata = Select a node to inspect its path and metadata.
back-to-templates = back to templates
languages = Languages
default-language = Default language
concept = Concept
delete-template = Delete template
template-deleted = Template deleted
archetype-id = Archetype ID
upload-your-first-adl2-operational-template = Upload your first ADL2 operational template with the Upload ADL2 button above.
upload-your-first-operational-template-opt = Upload your first operational template (OPT/XML) with the Upload OPT button above.
adl-2-templates-2 = ADL 2 templates
adl-1-4-templates = ADL 1.4 templates
template-pages = Template pages
template-id-2 = Template ID
upload-template = Upload template
template-source = Source
template-families = Template families
the-adl2-operational-template = the ADL2 operational template
upload-failed = Upload failed
adl2-template-uploaded = ADL2 template uploaded
the-operational-template = the operational template
operational-template-was-accepted-cdr = The operational template was accepted by the CDR.
template-uploaded = Template uploaded
filter-id-concept = filter by id or concept…
no-adl2-artefact-id-delete = no ADL2 artefact id to delete
there-no-adl2-source-upload = there is no ADL2 source to upload
selected-file-was-empty = the selected file was empty
pick-terminology-left-first = Pick a terminology on the left first.
cdr-serves-no-terminology-called = The CDR serves no terminology called{" "}
origin = Origin
relation = Relation
targets = Targets
concept-relations = Concept relations
terms = Terms
term = Term
language = Language
preferred = Preferred
terminology-version = Terminology version{" "}
asking-the-cdr = Asking the CDR…
candidate-code = Candidate code
expand-2 = Expand
value-set-id = Value set id
expand-a-value-set = Expand a value set
not-member = {" "}is not a member of{" "}
is-a-member-of = {" "}is a member of{" "}
in = {" "}in{" "}
unknown-value-set = Unknown value set{" "}
both-value-set-id-candidate-code = Both a value set id and a candidate code are needed.
type-value-set-id-expand = Type a value set id to expand.
testing = Testing…
subsumption-strict-code-never-subsumes-itself = Subsumption is strict: a code never subsumes itself.
test = Test
reference-code = Reference code
test-subsumption = Test subsumption
does-not-subsume = {" "}does not subsume{" "}
subsumes = {" "}subsumes{" "}
both-reference-code-candidate-code-needed = Both a reference code and a candidate code are needed.
looking-up = Looking up…
define = Define
effective-date-optional = Effective date (optional)
define-a-term = Define a term
unknown-code = Unknown code{" "}
type-term-code-define = Type a term code to define.
attributes = Attributes
available-versions = Available versions
publisher = Publisher
pick-terminology-left-describe = Pick a terminology on the left to describe it.
descriptor = Descriptor
terminologies = Terminologies
no-stored-query-run-open-one = no stored query to run: open one from the stored-query list
stored-query-needs-name = the stored query needs a name
exact-version = Exact version
version-prefix-latest-match = Version prefix (latest match)
latest-version-no-version-sent = Latest version (no version sent)
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
scopes-narrow-access-they-never-grant = Scopes narrow access, they never grant it: they say what a token may ask for, and the CDR still decides every request against its own policies and your permissions. A grant shown here is an upper bound.
no-subscription-id-delete = no subscription id to delete
no-subscription-id-update = no subscription id to update
subscription-name-required-may-hold-only = a subscription name is required and may hold only letters, digits, `_`, `.` and `-`
matches-every-committed-version = Matches every committed version.
candidate-code-2 = candidate code
value-set-id-2 = value set id
terminology-id = terminology id
reference-code-2 = reference code
term-code-2 = term code
not-cdr-any-more-was-already = { $object } is not in the CDR any more: it was already deleted. Reload this screen.
cdr-rejected-delete = The CDR rejected the delete of { $object }: { $message }.
deleting-failed = Deleting { $object } failed: { $error }
viewer-session-expired-before-was-deleted = The viewer session expired before { $object } was deleted: sign in again and retry.
deleting-failed-2 = Deleting { $object } failed: { $other }
column-2 = column { $value0 }
bytes = { $size } bytes
over-days-most-on = { $total } { $noun } over { $points } days, most on { $busiest }.
per-day = { $series_name } per day
item-tag-list-json = ITEM_TAG list JSON: { $e }
series-over = { $model } series over { $axis_label }.
version-2 = Version { $version }
open-2 = Open: { $gaps }.
export-failed = export failed: { $e }
parameters-must-json-object-2 = parameters must be a JSON object: { $e }
unknown-export-format-csv-json = unknown export format `{ $other }` (csv | json)
unparseable-result-set = unparseable RESULT_SET: { $e }
column-3 = column { $value0 }
attachment-filename = attachment; filename="{ $filename }"
was-not-sent-correct-input-retry = { $object } was not sent: { $message }. Correct the input and retry.
cdr-rejected-fix-reported-problem-retry = The CDR rejected { $object }: { $message }. Fix the reported problem and retry.
cdr-answered-nothing-was-saved = The CDR answered { $status } to { $object }: { $message }. Nothing was saved.
z = { $with_seconds }Z
readiness-json = readiness JSON: { $e }
management-info-json = management info JSON: { $e }
metric-list-json = metric list JSON: { $e }
metric-detail-json = metric detail JSON: { $e }
loggers-json = loggers JSON: { $e }
loggers-json-2 = loggers JSON: { $e }
loggers-json-3 = loggers JSON: { $e }
auditevent-bundle-json = AuditEvent Bundle JSON: { $e }
of-records = { $shown_from }–{ $shown_to } of { $total } records
composition-changed-since-page-loaded-reload = the composition changed since this page loaded: reload and retry. { $detail }
versioned-composition-json = versioned composition JSON: { $e }
version-json = version JSON: { $e }
revision-history-json = revision history JSON: { $e }
new-version = New version { $uid }
composition = Composition { $short }…
ehr = EHR { $ehr_short }…
queries-2 = { $members } queries
id = { $kind } id
open-a = Open a { $kind }
openehr-demographic-v1 = openEHR-DEMOGRAPHIC-{ $kind }.{ $kind2 }.v1
new-version-2 = New { $kind } version { $uid }
the-new = the new { $kind }
create-a = Create a { $kind }
document-canonical-json-2 = { $kind } document (canonical JSON)
create = Create { $kind }
no-party-kind-holds-tag-may = No party kind holds { $uid }: the tag may target an object that was since deleted.
contribution-json = CONTRIBUTION JSON: { $e }
contribution = Contribution { $short }…
versioned-demographic-object-json = versioned demographic object JSON: { $e }
demographic-version-json = demographic version JSON: { $e }
party-body-not-valid-json = the party body is not valid JSON: { $e }
party-json = party JSON: { $e }
identities-not-valid-json = identities is not valid JSON: { $e }
details-not-valid-json = details is not valid JSON: { $e }
loaded-party-document-not-valid-json = the loaded party document is not valid JSON ({ $e }): reload this tab and retry
edited-party-could-not-serialized = the edited party could not be serialized: { $e }
new-version-3 = New version { $uid }
party-s-id-required = the { $end } party's id is required
party-s-kind-must-one-five = the { $end } party's kind must be one of the five party kinds
details-not-valid-json-2 = details is not valid JSON: { $e }
edited-relationship-could-not-serialized = the edited relationship could not be serialized: { $e }
party-relationship-json = PARTY_RELATIONSHIP JSON: { $e }
new-party-relationship-version = New PARTY_RELATIONSHIP version { $uid }
relationship-2 = Relationship { $short }…
new-version-4 = New version { $uid }
commit-changes-one-contribution = Commit { $many } changes as one contribution
change-set-was-committed-one-contribution = The change set was committed as one contribution ({ $versions } { $noun }).
contribution-committed-with = Contribution { $contribution_uid } committed with { $versions } { $noun }.
not-valid-json = not valid JSON: { $e }
change-2 = change { $position } ({ $change }, { $target }): { $complaint }
contribution-could-not-serialized = the contribution could not be serialized: { $e }
document-not-valid-json = the document is not valid JSON: { $e }
z-2 = { $civil }Z
new-version-5 = New version { $uid }
contributions-json = contributions JSON: { $e }
new-version-6 = New version { $uid }
folders-items = { $folder_count } folders · { $item_count } items
version-json-2 = version JSON: { $e }
directory-json = directory JSON: { $e }
subtree-json = subtree JSON: { $e }
directory-json-2 = directory JSON: { $e }
invalid-json = Invalid JSON: { $e }
folders-items-2 = { $folders } folders · { $items } items
ehr-json = EHR JSON: { $e }
ehr-status-2 = ehr_status ({ $ehr_status_type })
ehr-2 = EHR { $short }…
ehr-all-data-were-removed-from = EHR { $id } and all of its data were removed from the CDR.
ehr-3 = EHR { $id }
other-details-not-valid-json = other_details is not valid JSON: { $e }
loaded-ehr-status-document-not-valid = the loaded EHR_STATUS document is not valid JSON ({ $e }): reload this tab and retry
edited-ehr-status-could-not-serialized = the edited EHR_STATUS could not be serialized: { $e }
ehr-status-json = ehr_status JSON: { $e }
versioned-ehr-status-json = versioned ehr_status JSON: { $e }
version-json-3 = version JSON: { $e }
new-version-7 = New version { $uid }
ehr-json-2 = EHR JSON: { $e }
nothing-ehr-holds-tagged-object-may = Nothing in this EHR holds { $uid }: the tagged object may have been deleted.
aql-result-json = AQL result JSON: { $e }
ehr-json-3 = EHR JSON: { $e }
new-ehr = New EHR
could-not-read-cdr-s-login = Could not read the CDR's login modes: { $error }
cdr-now-logging-requested = The CDR is now logging with `{ $filter }` (requested `{ $filter2 }`).
the-log-filter-2 = the log filter `{ $filter }`
cdr-restored-boot-filter = The CDR restored its boot filter `{ $filter }`.
v-was-removed-from-cdr = { $name } v{ $version } was removed from the CDR.
stored-query-v = Stored query { $name } v{ $version }
queries-3 = { $count } queries
toggle = Toggle { $node_label }
remove-code = Remove code { $remove_code }
alias-for-column = Alias for column { $position }
remove-column = Remove column { $position }
sort-path-for-rule = Sort path for rule { $position }
sort-direction-for-rule = Sort direction for rule { $position }
remove-sort-rule = Remove sort rule { $position }
a-new-immutable-version = a new immutable version { $version }
is-a-coded-value = { $label } is a coded value
is-one-of = { $label } is one of { $names }
is-a-graded-value = { $label } is a graded value
is-one-of-2 = { $label } is one of { $names }
equals = { $label } equals "{ $text }"
contains = { $label } contains "{ $value1 }"
from-to = { $label } from { $from } to { $to }
on-or-after = { $label } on or after { $from }
on-or-before = { $label } on or before { $to }
any-date = { $label } (any date)
is = { $label } is { $value }
is-present = { $label } is present
not = NOT ({ $body })
between-and = { $word } between { $a } and { $b }
at-least = { $word } at least { $a }
at-most = { $word } at most { $b }
post-query = POST query/{ $name }
post-query-2 = POST query/{ $name }/{ $resolved }
cdr-up-v = CDR UP · v{ $version }{ $profile }
was-removed-from-cdr = { $name } was removed from the CDR.
subscription-2 = subscription `{ $name }`
edit-subscription = Edit subscription { $name }
showing-busiest-templates = Showing the { $shown } busiest of { $total } templates.
status-json = status JSON: { $e }
http = HTTP { $status }
the-cdr-answered-there = the CDR answered { $status } there
the-cdr-answered-there-2 = the CDR answered { $status } there: { $message }
openapi-json = openapi JSON: { $e }
operationaltemplatev2-parse = OperationalTemplateV2 parse: { $e }
webtemplate-build = WebTemplate build: { $value0 }
adl2-template = ADL2 template · { $template_id }
opt-1-4-parse = OPT 1.4 parse: { $e }
webtemplate-build-2 = WebTemplate build: { $value0 }
template-2 = Template · { $template_id }
was-removed-from-cdr-2 = { $id } was removed from the CDR.
template-3 = Template { $id }
template-list-json = template list JSON: { $e }
adl2-template-list-json = ADL2 template list JSON: { $e }
was-accepted-cdr = { $template_id } was accepted by the CDR.
was-removed-from-cdr-3 = { $template_id } was removed from the CDR.
template-4 = Template { $template_id }
parameters-must-json-object-3 = parameters must be a JSON object: { $e }
stored-query-list-json = stored-query list JSON: { $e }
stored-query-json = stored query JSON: { $e }
parameters-must-json-object-4 = parameters must be a JSON object: { $e }
stored-query-result-json = stored-query result JSON: { $e }
identity-claim = Identity claim · { $name }
launch-context = Launch context · { $other }
not-compartment-resource-scope-master08-defines = "{ $compartment }" is not a compartment. A resource scope is { $resource_scope_form }, and master08 defines exactly three compartments: patient, user, system.
carries-no-permission-tail-resource-scope = "{ $rest }" carries no .<permission> tail. A resource scope is { $resource_scope_form }: for example patient/composition-*.rs.
not-permission-tail-master08-allows-only = "{ $tail }" is not a permission tail. master08 allows only c (create), r (read), u (update), d (delete) and s (search), in any order: for example .rs or .crud.
not-resource-master08-defines-template-templateid = "{ $resource }" is not a resource. master08 defines template-<templateId>, composition-<templateId> and aql-<queryName>.
not-resource-scope-master08-form-example = Not a resource scope. The master08 form is { $resource_scope_form }: for example patient/composition-*.rs.
matches = Matches { $parts }.
event-subscription-list-json = event subscription list JSON: { $e }
event-subscription-record-json = event subscription record JSON: { $e }
conformance-manifest-json = conformance manifest JSON: { $e }
terminology-json = terminology JSON: { $e }
a-is-required = a { $what } is required
cdr-ingests-adl-1-4-operational = The CDR ingests an ADL 1.4 operational template as OPT/XML. Choose a file or paste the source below; the CDR's validation diagnostics are shown verbatim on refusal.
cdr-ingests-adl2-artefact-source-text = The CDR ingests the ADL2 artefact SOURCE as text/plain. Choose a file or paste the source below; the openEHR-ADL engine's diagnostics are shown verbatim on refusal.
still-referenced-committed-data-so-cdr = { $object } is still referenced by committed data, so the CDR refused the delete ({ $message }). Delete the versions that reference it first, then retry.
session-may-not-delete-sign-admin = This session may not delete { $object } ({ $message }). Sign in with an ADMIN-role account and retry.
cdr-no-longer-accepts-session-so = The CDR no longer accepts this session, so { $object } was not deleted ({ $message }). Sign in again and retry.
sandbox-deployment-deployment-has-not-made = Sandbox deployment: this deployment has not made the production separations and must not hold real patient data.
no-subject-pseudonym-namespace-declared-so = no subject pseudonym namespace is declared, so an EHR subject may carry a real identifier
access-record-audit-queue-cannot-take = an access record the audit queue cannot take is dropped and the request still succeeds
ehr-without-own-access-settings-admits = an EHR without its own access settings admits every caller the role checks let through
role-checks-off-so-every-signed = role checks are off, so every signed-in caller may use every enabled surface, the admin and management endpoints included
changed-cdr-since-screen-loaded-reload = { $object } changed in the CDR since this screen loaded ({ $message }). Reload the latest version, then reapply your change.
cdr-already-holds-conflicting-object-so = The CDR already holds a conflicting object, so { $object } was not created ({ $message }). Open the existing one, or change the identifier and retry.
not-cdr-may-have-been-deleted = { $object } is not in the CDR ({ $message }): it may have been deleted meanwhile. Reload this screen and retry.
cdr-no-longer-accepts-session-so-2 = The CDR no longer accepts this session, so { $object } was not saved ({ $message }). Sign in again and retry.
session-may-not-write-sign-account = This session may not write { $object } ({ $message }). Sign in with an account that carries the required role and retry.
viewer-session-expired-before-was-saved = The viewer session expired before { $object } was saved: sign in again and retry; your input is still on screen.
cdr-unreachable-so-was-not-saved = The CDR is unreachable ({ $message }), so { $object } was not saved. Check the CDR, then retry.
could-not-saved-retry-if-persists = { $object } could not be saved: { $message }. Retry; if it persists, check the viewer logs.
moved-cdr-since-screen-loaded-so = { $object } moved on in the CDR since this screen loaded, so nothing was deleted ({ $message }). Reload the version history and delete the version that is latest now.
cdr-refused-delete-may-already-deleted = The CDR refused to delete { $object }: { $message }. It may already be deleted: reload this screen to see the current history.
not-cdr-may-have-been-deleted-2 = { $object } is not in the CDR ({ $message }): it may have been deleted meanwhile. Reload this screen.
session-may-not-delete-sign-account = This session may not delete { $object } ({ $message }). Sign in with an account that carries the required role and retry.
cdr-no-longer-accepts-session-so-3 = The CDR no longer accepts this session, so nothing was deleted ({ $message }). Sign in again and retry.
latest-version-s-full-object-version = the latest version's full OBJECT_VERSION_ID is required to delete a composition: reload this screen and retry
logically-delete-latest-version-composition-cdr = Logically delete the latest version of this composition? The CDR commits a deleted version on top of it: the composition stops resolving as current, while every earlier version and its audit trail stay readable. It cannot be undone.
openehr-demographic-api-addresses-id-publishes = The openEHR demographic API addresses a { $kind } by its id: it publishes no party list, and AQL queries EHRs, not parties. Open one by id, create one, or find one through the tag index.
deleting-party-needs-latest-version-s = deleting a party needs the latest version's full OBJECT_VERSION_ID: reload this screen and retry
identities-must-hold-least-one-party = identities must hold at least one PARTY_IDENTITY: a party with no identity is invalid (PARTY invariant Identities_valid)
commits-new-version-top-one-loaded-2 = Commits a new { $kind } version on top of the one loaded above (If-Match), so a concurrent change is refused rather than overwritten. Every other attribute travels back exactly as the CDR served it.
delete-commits-deleted-version-party-stops = Delete { $kind } { $uid }? This commits a deleted version: the party stops resolving as current, and every earlier version stays readable by its own version uid.
current-version-uid-required-update-relationship = the current version uid is required to update this relationship: reload this screen and retry
deleting-relationship-needs-latest-version-s = deleting a relationship needs the latest version's full OBJECT_VERSION_ID: reload this screen and retry
relationship-type-required-stored-relationship-s = a relationship type is required: it is stored as the relationship's name (PARTY_RELATIONSHIP invariant Type_validity)
party-must-named-versioned-object-uid = the { $end } party must be named by its versioned_object_uid, not by one version's OBJECT_VERSION_ID: a relationship refers to the party's version container
loaded-relationship-document-not-valid-json = the loaded relationship document is not valid JSON ({ $e }): reload this screen and retry
delete-relationship-commits-deleted-version-relationship = Delete relationship { $uid }? This commits a deleted version: the relationship stops resolving as current, and every earlier version stays readable by its own version uid.
these-versioned-party-container-s-tags = These are the VERSIONED_PARTY container's tags. Saving one re-sends the whole collection, which is how the openEHR tag update works: a tag another client added in the meantime can be lost, so reload before editing a busy party.
permanently-delete-ehr-every-composition-contribution = Permanently delete EHR { $ehr_id }: every composition, contribution and audit record in it? This is the CDR's physical delete: nothing stays readable afterwards, and it cannot be undone.
current-ehr-status-version-uid-required = the current EHR_STATUS version uid is required to update it: reload this tab and retry
tags-collection-named-below-versioned-composition = Tags on the collection named below: the VERSIONED_COMPOSITION container while the selector shows Latest, or that one version once you pin it: openEHR keeps the two apart, so a tag set on one is invisible to the other. Saving re-sends the whole collection, and the openEHR tag operations take no version check, so a tag another client added meanwhile can be lost.
tags-free-key-value-markers-client-2 = Tags are free key/value markers a client sets on a composition; add one below.
these-versioned-ehr-status-container-s = These are the VERSIONED_EHR_STATUS container's tags, so they stay put when the status is edited into a new version. Saving re-sends the whole collection, and the openEHR tag operations take no version check, so a tag another client added meanwhile can be lost.
tags-free-key-value-markers-client-3 = Tags are free key/value markers a client sets on the EHR's status; add one below.
ehr-id-must-uuid-8-4 = An EHR id must be a UUID (8-4-4-4-12 hex, e.g. 7d44aa01-0f9e-4a2c-9a0f-2a6a5f9b1c3d). Leave it empty to let the CDR mint one.
switch-cdr-s-live-log-filter = Switch the CDR's live log filter to “{ $filter }”? Logging changes immediately for every request this instance serves; a verbose filter on a busy server produces a lot of output.
permanently-delete-stored-query-version-from = Permanently delete stored query “{ $name }” version { $version } from the CDR? Every client loses that version, and it cannot be undone.
not-version-store-use-major-minor = { $version } is not a version to store at: use major.minor.patch, for example 1.0.0
name-unique-cdr-cannot-changed-afterwards = The name is unique on the CDR and cannot be changed afterwards: it may hold only letters, digits, and “_”, “.” or “-”. Every predicate left empty matches anything, so a subscription with none of them set receives every committed version.
delete-subscription-from-cdr-consumers-reading = Delete the subscription “{ $name }” from the CDR? { $value1 } Consumers reading its stream stop receiving events, and the subscription cannot be restored: only created again under the same name.
smart-discovery-could-not-read-cdr = SMART discovery could not be read: the CDR is unreachable ({ $detail }). This says nothing about whether SMART is enabled: check the CDR is up, then reload.
smart-discovery-could-not-read-cdr-2 = SMART discovery could not be read: the CDR refused this session at { $endpoint }. Sign in again, then reload.
smart-discovery-could-not-read-session = SMART discovery could not be read: this session may not read { $endpoint }. It needs an account the CDR allows there.
smart-discovery-could-not-read-from = SMART discovery could not be read from { $endpoint }: { $detail }. Whether SMART is enabled is therefore unknown; check that the CDR serves that path.
built-viewer-from-served-operationaltemplatev2-adl2 = Built by the viewer from the served OperationalTemplateV2: the ADL2 resource serves no Web Template representation of its own.
cdr-holds-no-template-selected-version = The CDR holds no { $what } for this template at the selected version. Pick another version, or go back to the ADL2 listing.
example-generated-from-artefact-route-names = The example is generated from the artefact this route names: the Definition API declares no versioned example resource, so the version bar above does not change it.
permanently-delete-operational-template-from-cdr = Permanently delete the operational template “{ $template_id }” from the CDR? This cannot be undone. The CDR refuses the delete while a committed version still references the template.
permanently-delete-operational-template-from-cdr-2 = Permanently delete the operational template “{ $id }” from the CDR? This cannot be undone. The CDR refuses the delete while a committed version still references the template.
permanently-delete-adl-2-artefact-from = Permanently delete the ADL 2 artefact “{ $id }” from the CDR's definition store? This cannot be undone, and the store keeps no version history of it. The CDR refuses the delete while a committed version still references the template.
not-version-store-stored-query-version = `{ $version }` is not a version to store at: a stored-query version is `major.minor.patch` (for example `1.0.0`). A shorter pattern like `1` or `1.0` selects the latest matching version when READING a query; leave the field empty to let the server assign the version.
cdr-already-holds-subscription-name-so = The CDR already holds a subscription with that name, so { $object } was not saved ({ $message }). Choose another name, or edit the existing subscription.
not-cdr-any-more-another-operator = { $object } is not on the CDR any more ({ $message }): another operator may have deleted it. Reload this screen.
session-may-not-administer-sign-admin = This session may not administer { $object } ({ $message }). Sign in with an ADMIN-role account and retry.
no-version-opened = No version opened
open-version-from-revision-history-above = Open a version from the revision history above, or resolve one by date and time.
single-row = single-row
one-row-not-trend = One row is not a trend
chart-needs-least-two-rows-widen = A chart needs at least two rows. Widen the query's window, or relax a condition, and run it again.
no-numeric-column = no-numeric-column
nothing-numeric-to-chart = Nothing numeric to chart
charting-needs-column-holding-least-two = Charting needs a column holding at least two numbers. Project a quantity magnitude, a count, or an ordinal value. An ISO date/time column then becomes the X axis.
create-ehr-find-one-id-subject = Create an EHR, find one by id or subject, or browse the most recent.
no-ehrs-found = No EHRs found
create-ehr-through-cdr-then-browse = Create an EHR through the CDR, then browse it here.
cdr-s-event-subscriptions-stored-filters = The CDR's event subscriptions: the stored filters over its committed-version event stream.
event-subscription-api-disabled-server = The event-subscription API is disabled on this server
cdr-answers-subscription-routes-if-unmounted = The CDR answers its subscription routes as if unmounted. Set admin_api = true under [events] in the CDR's configuration to administer subscriptions here.
no-event-subscriptions = No event subscriptions
create-first-subscription-card-above = Create the first subscription with the card above.
atna-security-audit-trail-who-accessed = The ATNA security audit trail: who accessed what, with what outcome (IHE ITI-81 retrieval).
local-audit-repository-disabled = The local audit repository is disabled
enable-audit-store-cdr-record-browse = Enable [audit.store] on the CDR to record and browse the ATNA audit trail here.
no-audit-records-match = No audit records match
widen-time-window-clear-filter-every = Widen the time window or clear a filter: every audited operation lands here.
pick-template-walk-paths-turn-data = Pick a template, walk its paths, and turn data-value leaves into criteria and columns.
no-conditions-yet = No conditions yet
no-rows = No rows
query-ran-matched-nothing-widen-condition = The query ran and matched nothing. Widen a condition or clear a filter.
cdr-status-openehr-conformance-manifest-smart = CDR status, the openEHR conformance manifest, SMART discovery, repository usage, the served OpenAPI documents, the redacted runtime configuration, and the manufacturer.
no-template-usage-yet = No template usage yet
counts-appear-once-compositions-committed-against = Counts appear once compositions are committed against a template.
no-paths-advertised = No paths advertised
cdr-served-openapi-document-no-operations = The CDR served an OpenAPI document with no operations in it.
version-was-deleted-selected-instant = This version was deleted at the selected instant
delete-committed-deleted-lifecycle-version-pick = The delete committed a deleted-lifecycle version; pick an earlier version above to read it.
no-audit-selected-version = No audit for the selected version
pick-another-version-above-every-committed = Pick another version above: every committed version carries its own audit.
no-tags-ehr = No tags in this EHR
tags-free-key-value-markers-client-4 = Tags are free key/value markers a client sets on a composition, the EHR status or the directory; the composition viewer and the Status tab both set them.
browse-terminologies-cdr-serves-describe-one = Browse the terminologies the CDR serves: describe one, define a code, expand a value set, and test membership or subsumption.
terminology-extension-disabled-server = The terminology extension is disabled on this server
cdr-answers-terminology-routes-if-unmounted = The CDR answers its terminology routes as if unmounted. Set api_enabled = true under [terminology] in the CDR's configuration to browse terminologies, terms and value sets here.
no-terminologies = No terminologies
cdr-serves-terminology-api-but-reports = The CDR serves the terminology API but reports no terminology in it.
nothing-in-this-extract = Nothing in this extract
cdr-answered-extract-carries-no-term = The CDR answered with an extract that carries no term and no relationship.
cdr-s-stored-queries-grouped-namespace = The CDR's stored queries, grouped by the namespace in their qualified name.
no-stored-queries = No stored queries
build-one-query-builder-write-raw = Build one in the query builder, or write raw AQL, then save it.
no-namespaces-yet = No namespaces yet
save-query-namespace-name-namespace-becomes = Save a query as namespace::name: its namespace becomes a group here and a tile on the dashboard.
dependency-health-build-provenance-metric-registry = Dependency health, build provenance, the metric registry, and live log control.
operational-templates-registered-cdr-archetype-model = Operational templates registered in the CDR, by archetype-model family.
no-templates-match-filter = No templates match the filter
clear-shorten-filter-see-whole-catalogue = Clear or shorten the filter to see the whole catalogue again.
no-templates-yet = No templates yet
repository-overview-query-cohorts-recent-commit = Repository overview, query cohorts, and recent commit activity.
no-stored-queries-yet = No stored queries yet
save-query-namespace-name-each-namespace = Save a query as namespace::name: each namespace becomes a cohort tile here.
execute-stored-query-cdr-choose-how = Execute a stored query on the CDR: choose how its version resolves, bind its parameters, page the results.
write-run-aql-directly-bind-parameters = Write and run AQL directly, bind parameters, and save it as a stored query.
relationship-joins-two-parties-source-target = A relationship joins two parties, source to target. It is its own versioned object here: an extension of this CDR, not part of the released openEHR demographic API.
no-relationships-party = No relationships on this party
party-document-carries-relationships-source-create = A party document carries the relationships it is the SOURCE of. Create one below, or add it to the party's own relationships list on the Party tab.
no-demographic-tags = No demographic tags
tags-one-demographic-collection-openehr-api = Tags are the one demographic collection the openEHR API lists. Set one on a party's Tags tab and it appears here.
no-versions-listed = No versions listed
contribution-always-references-least-one-version = A CONTRIBUTION always references at least one version; if none is listed, the CDR reported an empty change set.
no-contributions = No contributions
ehr-has-no-contributions-page = This EHR has no contributions on this page.
nothing-staged-yet = Nothing staged yet
add-change-above-they-commit-together = Add a change above; they commit together as one contribution.
no-versions-yet = No versions yet
ehr-has-no-directory-create-one = This EHR has no directory. Create one and its versions are listed here.
deleted-at-that-time = Deleted at that time
directory-had-been-deleted-selected-time = The directory had been deleted as of the selected time.
no-directory-then = No directory then
no-directory-existed-selected-time = No directory existed at the selected time.
no-folder-path = No folder at that path
directory-has-no-sub-folder-matching = The directory has no sub-folder matching the path.

no-value = (no value)
tab-wt = WT
tab-opt = OPT
choice-any = any
choice-success = success
action-create = create
action-read = read
action-update = update
action-delete = delete
action-execute = execute
noun-ehrs =
    { $count ->
        [one] EHR
       *[other] EHRs
    }
    .empty = No EHRs
    .opening = EHRs
noun-subscriptions =
    { $count ->
        [one] subscription
       *[other] subscriptions
    }
    .empty = No subscriptions
    .opening = Subscriptions
noun-tagged-objects =
    { $count ->
        [one] tagged object
       *[other] tagged objects
    }
    .empty = No tagged objects
    .opening = Tagged objects
noun-stored-queries =
    { $count ->
        [one] stored query
       *[other] stored queries
    }
    .empty = No stored queries
    .opening = Stored queries
noun-tags =
    { $count ->
        [one] tag
       *[other] tags
    }
    .empty = No tags
    .opening = Tags
noun-compositions =
    { $count ->
        [one] composition
       *[other] compositions
    }
    .empty = No compositions
    .opening = Compositions
noun-adl14-templates =
    { $count ->
        [one] ADL 1.4 template
       *[other] ADL 1.4 templates
    }
    .empty = No ADL 1.4 templates
    .opening = ADL 1.4 templates
noun-adl2-templates =
    { $count ->
        [one] ADL 2 template
       *[other] ADL 2 templates
    }
    .empty = No ADL 2 templates
    .opening = ADL 2 templates
flag-queryable = queryable
flag-modifiable = modifiable

## Numbers: the marks FerroUI's formatter groups a number with

number-group = ,
number-decimal = .

## The product and the shell

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

## The error pages

forbidden-body = You are signed in as { $who }, and the CDR shows this only to { $needs }.

## Sign in

sign-in-title = Sign in
sign-in-at = Sign in at { $host }
sign-in-redirecting = Taking you to { $host }…
sign-in-identity-provider = the identity provider
sign-in-basic-submit = Sign in with a CDR account
sign-in-caption = The viewer of the FerroEHR clinical data repository.
sign-in-expired = Your session ended. Sign in again to return to the page you were on.
sign-in-signed-out = You are signed out.
sign-in-refused-provider = The identity provider did not sign you in.
sign-in-refused = Not signed in: the CDR refused that username or password.
sign-in-failed = Not signed in: { $error }
sign-in-cdr-unreachable = The CDR did not answer. You can still try to sign in.

## Common words

try-again = Try again
unknown-count = —
upload-a-template = Upload a template
your-preferences = Your preferences
brand-accent = Teal
column-namespace = Namespace
column-matches = Matches

## The dashboard

stat-ehrs = EHRs
stat-compositions = Compositions, found in Query
stat-templates = Templates
stat-stored-queries = Stored queries
dashboard-subtitle = What { $host } holds, as it answered just now.
dashboard-no-answer = The CDR did not answer.
dashboard-refused = The CDR did not answer, so the dashboard cannot be shown. Your session is kept.
dashboard-per-day = Compositions per day
dashboard-per-day-summary = { $total } compositions from { $first } to { $last }, at most { $most } on one day.
dashboard-per-day-note = By commit time, over the newest compositions; read with one AQL query.
dashboard-per-day-refused = The compositions per day could not be read.
dashboard-no-compositions = No composition is committed yet.
dashboard-create-first-ehr = Create the first EHR
dashboard-by-namespace = Matches by namespace
dashboard-queries = { $count ->
    [one] 1 query
   *[other] { $count } queries
}
dashboard-matches = { $count ->
    [one] 1 match
   *[other] { $count } matches
}
dashboard-no-total = No total
dashboard-namespaces-refused = The stored queries could not be read.
dashboard-no-stored-query = No stored query yet.
dashboard-write-one = Write one in Query

## About

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

## Your preferences

reveal-label = Show a revealed identifier for
reveal-description = How long a subject id stays shown after you reveal it; the viewer caps it at 2 minutes.
reveal-seconds = { $count } seconds
reveal-minutes = { $count ->
    [one] 1 minute
   *[other] { $count } minutes
}

## Your access

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

## Common words

yes = Yes
no = No
none-shown = Not recorded
column-created = Created
column-ehr = EHR

## Find

find-title = Find
find-subtitle = One answer for an id, a subject, a template or a stored query.
find-empty = Type an id, a subject, a template or a query name in the search field above.
find-no-id-holds = No EHR, composition or party has the id { $id }.
find-nothing-for = Nothing matches “{ $typed }”.
find-found-some = { $count ->
    [one] 1 record matches.
   *[other] { $count } records match.
}
find-matches = { $count ->
    [one] 1 match for “{ $typed }”
   *[other] { $count } matches for “{ $typed }”
}
find-other-ways = Other ways to find a record
find-compositions = Compositions
find-ehr = EHR { $id }
find-ehr-line = Created { $created }
find-composition-line = In EHR { $ehr }
find-party-line = { $kind }
find-query-line = Stored query, version { $version }
find-template-line = Template: { $concept }
find-nothing-title = Nothing found
find-nothing-line = Check the spelling, or open the list of the kind of record you look for.
find-unfinished-title = Part of the search did not finish
find-unfinished-line = The CDR refused the search over { $kinds }; the answer may be incomplete.
find-way-ehr = EHRs
find-way-ehr-line = Every EHR, with the subject search.
find-way-subject = By subject
find-way-subject-line = An EHR by its subject id and namespace.
find-way-party = Parties
find-way-party-line = A party by its id or its tags.

## EHRs

ehrs-subtitle = Every EHR the CDR holds, newest first.
ehrs-find-label = Subject id
ehrs-namespace = Namespace
ehrs-subject-chip = Subject: { $subject }
ehrs-subject-needs-namespace = A subject search needs the namespace the subject id belongs to.
ehrs-find-refused = The subject search could not be finished.
ehrs-refused = The EHRs could not be read.
ehrs-empty = The CDR holds no EHR yet.
ehrs-no-match = No EHR belongs to this subject.
ehrs-create-first = Create the first EHR
ehrs-pages = EHR pages
ehr-open = Open EHR { $id }
ehr-new-subtitle = An EHR with its status; the subject is optional.
ehr-new-ehr-id = EHR id
ehr-new-ehr-id-help = Leave it empty and the CDR mints a UUID.
ehr-new-ehr-id-placeholder = UUID, or empty
ehr-new-subject = Subject
ehr-new-subject-intro = The person this EHR is about, as an id the CDR keeps without reading it.
ehr-new-subject-id = Subject id
ehr-new-namespace = Namespace
ehr-new-create = Create EHR
ehr-new-not-yet = Not created yet
ehr-new-not-created = EHR not created
ehr-new-not-uuid = An EHR id is a UUID, such as 7d44aa01-0f9e-4a2c-9a0f-2a6a5f9b1c3d.
ehr-new-in-use = An EHR with this id exists already.
ehr-new-refused = The CDR refused to create the EHR.

## One EHR

ehr-title = EHR { $id }
ehr-subtitle = Created { $created } in { $system }.
ehr-not-found-title = No EHR { $id }
ehr-refused = The EHR could not be read.
ehr-back = Back to EHRs
ehr-more = More actions
ehr-copy-id = Copy EHR id
ehr-commit-several = Commit several changes
ehr-create-directory = Create the directory
ehr-delete = Delete EHR
ehr-delete-title = Delete EHR { $id }?
ehr-delete-message = The EHR and every composition, directory and status version in it are removed from the CDR. This cannot be undone.
ehr-delete-confirm-label = Type the first 8 characters of the EHR id to confirm
ehr-delete-confirm = Delete EHR
ehr-delete-mismatch = The characters you typed are not the start of this EHR id.
ehr-delete-refused = The CDR refused to delete the EHR.
ehr-tab-compositions = Compositions
ehr-tab-directory = Directory
ehr-tab-status = Status
ehr-tab-contributions = Contributions
ehr-tab-tags = Tags
directory-create-refused = The CDR refused to create the directory.
fact-version = Version
fact-contribution = Contribution
fact-lifecycle = Lifecycle
fact-signed = Committed

## Editors

editor-title = Edit
editor-opened = Opened at the latest version
editor-not-saved = Not saved
editor-save = Save
editor-save-as-version = Save as version { $version }
editor-reload-version = Reload version { $version }
editor-reload-first = Reload the latest version before you save.
editor-reload-replaces = Reloading replaces your edits with the version on the server.
editor-json = JSON
editor-utf-8 = UTF-8

## EHR status

status-title = Status of EHR { $id }
status-this-version = This version
status-capabilities = Capabilities
status-queryable-help = AQL over the population includes this EHR.
status-not-queryable = Not queryable: AQL over the population leaves this EHR out.
status-modifiable-help = Clients may commit to this EHR.
status-other-details = Other details
status-other-details-intro = The ITEM_STRUCTURE the status carries beside its flags.
status-document = Document
status-document-title = EHR_STATUS, version { $version }
status-document-intro = The status as the CDR holds it.
status-versions = Versions
status-at-time-opens = Opens the version that was current at that moment.
status-edit = Edit status
status-edit-title = Edit the EHR status
status-edit-subtitle = Saved as one new version.
status-saved = Status saved
status-saved-line = Saved as a new version.
status-saved-version = Saved as version { $version }.
status-save-refused = The CDR refused the status.
status-refused = The EHR status could not be read.
status-history-refused = The status history could not be read.
status-version-refused = This version could not be read.

## Directory

directory-folders-title = Folders
directory-empty = Empty
directory-folders = { $count ->
    [one] 1 folder
   *[other] { $count } folders
}
directory-items = { $count ->
    [one] 1 reference
   *[other] { $count } references
}
directory-folders-items = { $folders ->
    [one] 1 folder
   *[other] { $folders } folders
}, { $items ->
    [one] 1 reference
   *[other] { $items } references
}
directory-saved-version = Saved as version { $version }.
directory-edit-title = Edit the directory
directory-edit-subtitle = Saved as one new version.
directory-edit-intro = Add, rename and remove folders, and add references to the EHR's compositions. Nothing reaches the CDR until you save.
directory-add-reference = Add the reference
directory-reference-label = Composition
directory-keep-name = Keep this name
directory-save-refused = The CDR refused the directory.
directory-reload-or-overwrite = Reload the latest version, or overwrite it.
directory-overwrite = Overwrite
directory-overwrite-note = Overwrite replaces the other change with yours.
directory-title = Directory of EHR { $id }
directory-tree-label = The directory's folders
directory-unnamed = (no name)
directory-new-folder = New folder
directory-none = This EHR has no directory. Create it from the EHR's actions.
directory-refused = The directory could not be read.
directory-none-at-time = The EHR had no directory at that moment.
directory-changed = Changed, not saved
directory-at-time-opens = Opens the version that was current at that moment.
directory-versions = Versions
directory-versions-intro = Every version of the directory; restoring one writes it back as a new version.
directory-version-holds = { $folders ->
    [one] 1 folder
   *[other] { $folders } folders
}, { $items ->
    [one] 1 reference
   *[other] { $items } references
}
directory-history-refused = The directory's versions could not be read.
directory-show-path = Show one folder by path
directory-path-placeholder = episodes/2026
directory-show = Show
directory-showing-path = Showing the folder at { $path }.
directory-no-path = The directory has no folder at { $path }; the whole tree is shown.
directory-path-refused = The folder at this path could not be read.
directory-edit = Edit
directory-more = More directory actions
directory-copy-id = Copy the directory id
directory-delete = Delete the directory
directory-delete-title = Delete the directory?
directory-delete-message = The directory is marked deleted; its versions stay in the history and the compositions it references are kept.
directory-delete-confirm = Delete the directory
directory-delete-refused = The CDR refused to delete the directory.
directory-restore = Restore…
directory-restore-version = Restore version { $version }
directory-restore-title = Restore version { $version }?
directory-restore-message = The tree of version { $version } is written back as version { $next }. The versions in between stay in the history.
directory-restore-as = Restore as version { $next }
directory-restore-refused = The CDR refused to restore this version.
directory-reload = Reload the directory
directory-saved = Directory saved

## Compositions

column-composition = Composition
column-start-time = Start time
column-composer = Composer
column-version = Version
compositions-empty = This EHR holds no composition yet.
compositions-no-match = No composition matches these filters.
compositions-pages = Composition pages
compositions-refused = The compositions could not be read.
filter-template = Template
filter-start-time = Start time
filter-from = From
filter-to = To
filter-composer = Composer
filter-apply = Apply
chip-template = Template: { $value }
chip-from = From { $value }
chip-to = To { $value }
chip-composer = Composer: { $value }
reveal-no-subject = This EHR has no subject.

## Contributions

contributions-title = Contributions to EHR { $id }
contributions-per-day = Contributions per day
contributions-per-day-title = Contributions per day, last 30 days
contributions-per-day-summary = Contributions per day, { $first } to { $last }: { $total } contributions, at most { $most } on one day.
contributions-activity-refused = The contributions per day could not be read.
contributions-empty = This EHR has no contribution yet.
contributions-pages = Contribution pages
contributions-refused = The contributions could not be read.
column-contribution = Contribution
column-committed = Committed
column-committer = Committer
column-change = Change
column-object = Object
column-kind = Kind
change-creation = Creation
change-amendment = Amendment
change-modification = Modification
change-deleted = Deleted
change-attestation = Attestation
change-synthesis = Synthesis
contribution-title = Contribution { $id }
contribution-subtitle = One change set, committed together.
contribution-refused = The contribution could not be read.
contribution-audit = Audit
contribution-versions = Versions
contribution-record = Record
fact-committed = Committed
fact-committer = Committer
fact-change = Change
fact-description = Description
fact-system = System
contribution-new-title = { $mode ->
    [new] New contribution
   *[edit] Edit
}
contribution-new-subtitle = Several changes, committed together: all of them or none.
contribution-committed = Contribution committed
contribution-made-versions = { $count ->
    [one] It made 1 version.
   *[other] It made { $count } versions.
}
contribution-not-committed = Not committed
contribution-commit-refused = The CDR refused the contribution, so nothing was committed.
contribution-choices-refused = The templates or compositions to pick from could not be read.
contribution-composition-refused = The composition to amend could not be read.
contribution-committer-placeholder = Leave empty to commit as yourself
contribution-commit = { $count ->
    [one] Commit 1 change
   *[other] Commit { $count } changes
}
staged-count = { $count ->
    [one] 1 change staged.
   *[other] { $count } changes staged.
}
staged-title = { $number }. { $what }
staged-what = What changes
staged-recorded-as = Recorded as the audit change type { $change }.
staged-apply = Apply
staged-create = A new composition
staged-amend = An amended composition
staged-status = The EHR status
staged-from-document = The one the document names
staged-composition = Composition
staged-composition-help = Its latest version is the one the change supersedes.
staged-pick = Pick a composition
staged-status-from = Loaded from version { $version }, which it supersedes.
staged-document = The document of change { $number }
staged-remove = Remove change { $number }
staged-stage = Stage a change

## EHR tags

tags-title = Tags in EHR { $id }
tags-intro = A tag is set on its object, on that object's Tags tab; this list reads them all.
tags-key = Key
tags-value = Value
tags-target-path = Target path
tags-whole-object = the whole object
tags-caption = Tags in this EHR
tags-empty = No object in this EHR carries a tag.
tags-no-match = No tag matches these filters.
tags-refused = The tags could not be read.
tags-composition = Composition { $id }
tags-ehr-status = EHR status
tags-versioned-composition = versioned composition
tags-versioned-status = versioned EHR status
tags-versioned-directory = versioned directory
tags-unknown-object = an object the CDR did not name
tags-one-version = version { $version }
chip-key = Key: { $value }
chip-value = Value: { $value }

## Compositions: one composition

composition-title = Composition { $id }
composition-refused = The composition could not be read.
composition-views = Views of this composition
composition-reading = Reading
composition-document = Document
composition-versions = Versions
composition-more = More actions for the composition
composition-copy-id = Copy the id
composition-edit = Edit as a new version
composition-edit-latest = Edit the latest
composition-line = { $template } · version { $version } · started { $started } by { $composer }.
composition-line-older = { $template } · started { $started } by { $composer }.
composition-deleted-line = { $template } · deleted { $when } by { $who } as version { $version }.
composition-deleted-mark = Deleted
composition-read-version = Read version { $version }
composition-context = Context
composition-no-reading = This document is not a canonical composition the reading understands; the Document tab shows it as sent.
composition-show-document = Show the full document
composition-format = Format
composition-converts = The CDR converts; the viewer shows its answer as sent.
composition-document-title = COMPOSITION · { $format }
composition-document-refused = The CDR could not send the document in this format.
composition-deleted-version = This version is the deletion; it holds no document.
composition-at-time-opens = Opens the version that was current at that moment.
composition-history-refused = The versions could not be read.
composition-version-refused = This version could not be read.
composition-version-n = Version { $version }
composition-preceding = Preceding version
composition-first-version = None: this is the first version
composition-holds-content = Holds content
composition-versioned = The versioned composition
composition-versioned-object = Versioned object
composition-first-committed = First committed
composition-last-before-deletion = You are reading version { $version }, the last before the deletion; it is read-only.
composition-delete = Delete composition
composition-delete-title = Delete the composition { $name }?
composition-delete-message = { $count ->
    [one] Marks the composition deleted with a new version { $next }; its version and its audit stay readable, and it leaves the EHR's composition list.
   *[other] Marks the composition deleted with a new version { $next }; its { $count } versions and their audit stay readable, and it leaves the EHR's composition list.
}
composition-delete-confirm = Delete the composition
composition-delete-refused = The CDR refused to delete the composition.
composition-deleted-no-edit = This composition is deleted; there is no version to edit.
composition-edit-title = Edit { $name }
composition-edit-subtitle = Version { $version }, opened now.
composition-editor-label = The composition, canonical JSON
composition-saved = Composition saved
composition-saved-version = Saved as version { $version }.
composition-save-refused = The CDR refused the document, so no version was saved.
composition-new-title = { $mode ->
    [new] New composition
   *[edit] Edit
}
composition-new-page-title = New composition
composition-new-subtitle = A document committed as version 1 under a template.
composition-new-editor-label = The new composition
composition-template-help = The template the document is committed against; FLAT needs it.
composition-templates-refused = The templates could not be read.
composition-not-yet = Not committed yet.
composition-commit = Commit this composition
composition-commit-refused = The CDR refused the document, so nothing was committed.
editor-refused-heading = { $count ->
    [one] What the CDR refused · 1
   *[other] What the CDR refused · { $count }
}
editor-xml = XML

## Tags of one object

tag-collection-composition = composition
tags-of = Tags of the { $object }
tag-intro = Tags stay on the { $object } across its versions; setting one commits no version.
tag-none = The { $object } carries no tag.
tag-whole = the whole { $object }
tag-remove-column = Remove
tag-remove = Remove the tag { $key }
tag-value-optional = Value (optional)
tag-path-optional = Target path (optional)
tag-set-title = Set a tag
tag-set-action = Set the tag
tag-refused = The CDR refused the tag.
tag-set = Tag set
tag-set-line = { $key } is on { $object }.
tag-removed = Tag removed
tag-removed-line = { $key } is gone from { $object }.
tag-count = { $count ->
    [one] 1 tag
   *[other] { $count } tags
}
composition-not-committed = Not committed

## Parties

kind-person = Person
kind-organisation = Organisation
kind-group = Group
kind-agent = Agent
kind-role = Role
parties-subtitle = People, organisations, groups, agents and roles, opened by their id.
parties-new = New party
parties-note = Parties follow the released RM 1.1.0 demographic model; the REST API for them is still in development, and FerroEHR adds the relationship routes it does not define yet.
parties-uid = Party uid
parties-open = Open
parties-open-refused = The party could not be opened.
parties-none-holds = No party has this uid.
parties-no-kind-holds = No party of any kind has this uid.
parties-tags-title = Tags on parties
parties-tag-index = The demographic tag index
parties-tag-on = On
parties-no-tags = No party carries a tag yet.
parties-tags-refused = The tags on parties could not be read.
party-title = Party { $id }
party-refused = The party could not be read.
party-views = Views of this party
party-line = { $kind } · version { $version } · { $id }
party-deleted-line = { $kind } · deleted { $when } by { $who } as version { $version }
party-more = More actions for the party
party-copy-id = Copy the id
party-edit = Edit
party-relate = Relate this party
party-delete = Delete party
party-delete-title = Delete { $name }?
party-delete-message = The party is marked deleted. { $count ->
    [one] Its version stays readable,
   *[other] Its { $count } versions stay readable,
} { $relationships ->
    [0] and nothing else changes.
    [one] and its relationship stays.
   *[other] and its { $relationships } relationships stay.
}
party-delete-confirm = Delete the party
party-delete-refused = The CDR refused to delete the party.
party-deleted-intro = The party is deleted; its versions stay readable, and its relationships stay.
party-deleted-no-edit = This party is deleted; there is no version to edit.
party-overview = Overview
party-facts = Facts
party-archetype = Archetype
party-current-version = Current version
party-identities = Identities
party-no-identities = This party carries no identity the reading understands.
party-relationships-from = Relationships from this party
party-relationships-intro = The relationships this party is the source of; the CDR cannot list the ones that point at it.
party-no-relationships = This party is the source of no relationship.
party-document-title = { $type } · version { $version }
party-version-document = The party at version { $version }
party-at-time-opens = Opens the party as it stood then.
party-new-title = { $mode ->
    [new] New party
   *[edit] Edit
}
party-new-subtitle = A starting document for a { $kind }.
party-kind-help = Start over puts this kind's starting document in the editor.
party-start-over = Start over
party-new-editor-label = The new party
party-not-yet = Not created yet.
party-not-created = Not created.
party-create = Create this party
party-create-refused = The CDR refused the party, so nothing was created.
party-edit-intro = The whole party as JSON, as New party takes it; it must keep at least one identity.
party-editor-label = The party
party-saved = Party saved
party-save-refused = The CDR refused the party, so no version was saved.
tag-collection-party = party

## Relationships

relationship-title = Relationship
relationship-line = Relationship · version { $version } · { $id }
relationship-refused = The relationship could not be read.
relationship-more = More actions for the relationship
relationship-copy-id = Copy the id
relationship-delete = Delete relationship
relationship-delete-title = Delete the relationship { $name }?
relationship-delete-message = The relationship is marked deleted; both parties stay, and its versions stay readable.
relationship-delete-confirm = Delete the relationship
relationship-delete-refused = The CDR refused to delete the relationship.
relationship-deleted-toast = Relationship deleted
relationship-source = Source
relationship-target = Target
relationship-target-kind = Target kind
relationship-target-uid = Target party uid
relationship-details = Details
relationship-details-intro = Optional extra details, as JSON.
relationship-none = none
relationship-validity = Time validity
relationship-new-title = { $mode ->
    [new] New relationship
   *[edit] Edit
}
relationship-new-page-title = New relationship
relationship-new-subtitle = From this party to another.
relationship-from = From
relationship-type-help = The type as a key, for example employment.
relationship-create = Create this relationship
relationship-create-refused = The CDR refused the relationship, so nothing was created.
relationship-ends = Ends
relationship-ends-intro = The two ends are fixed; other parties make another relationship.
relationship-saved = Relationship saved
relationship-save-refused = The CDR refused the relationship, so no version was saved.

## Templates

templates-subtitle = The operational templates the CDR validates compositions against.
templates-upload = Upload a template
templates-find = Id, concept or archetype
templates-families = Template families
templates-family = Family
templates-empty = No template is uploaded yet.
templates-no-match = No template matches.
templates-refused = The templates could not be read.
chip-contains = Contains: { $value }
column-template = Template
column-root-archetype = Root archetype
column-uploaded = Uploaded
column-actions = Actions
template-actions-for = Actions for { $name }
template-build-query = Build a query
template-delete = Delete
template-delete-title = Delete the template { $name }?
template-delete-message = The CDR refuses while compositions use the template; delete or migrate them first.
template-delete-confirm = Delete the template
template-delete-refused = The CDR refused to delete the template.
template-more = More actions for the template
template-download = Download
template-copy-id = Copy id
template-views = Views of this template
template-refused = The template could not be read.
template-adl-14-line = ADL 1.4
template-adl-14-subtitle = { $concept } · ADL 1.4
template-adl-2-subtitle = { $hrid } · ADL 2
template-paths = Paths
template-json = JSON
template-example = Example
template-paths-of = The paths of { $name }
template-node = Node
template-pick-node = Pick a node in the tree to read its path, type and codes.
template-adl-2-paths = The viewer builds these paths itself: the CDR has no web template for ADL 2.
template-no-catalog = The CDR sent no operational template to build paths from.
template-no-source = The CDR holds no source for this release.
template-source-title = Source
template-json-title = JSON · release { $release }
template-example-type = Type
template-example-detail = Detail level
template-show-example = Show the example
template-example-title = Example · { $format }
template-no-example = The CDR built no example for these options.
template-example-refused = The CDR could not build an example for these options.
template-identity = Identity
template-use = Use
template-use-line = Query the compositions committed under it
template-concept = Concept
template-version = Template version
template-default-language = Default language
template-languages = Languages
template-uid = UID
template-release = Release
template-releases = Releases
template-release-count = { $count ->
    [one] 1 release
   *[other] { $count } releases
}
template-reading-release = You are reading release { $release }.
template-read-latest = Read { $release }, the latest
template-upload-title = { $mode ->
    [new] Upload a template
   *[edit] Edit
}
template-upload-subtitle = Choose a file or paste the source; the CDR validates it on upload.
template-opt-source = The operational template
template-adl-2-source = The ADL 2 source
template-choose-file = Choose a file
template-opt-placeholder = <?xml version="1.0" encoding="UTF-8"?>
template-adl-2-placeholder = template (adl_version=2.4.0; rm_release=1.1.0)
template-not-yet = Not uploaded yet.
template-not-uploaded = Not uploaded.
template-upload = Upload this template
template-upload-refused = The CDR refused the template, so nothing was uploaded.

## Terminology

term-subtitle = What the CDR can answer about the code sets it serves.
term-operations = Terminology operations
term-terminologies = Terminologies
term-lookup = Look up
term-value-sets = Value sets
term-hierarchy = Hierarchy
term-off = The CDR's terminology API is off.
term-off-line = Set in ferroehr.toml and read at start; the Configuration page shows the running value.
term-refused = The terminologies could not be read.
term-terminology = Terminology
term-publisher = Publisher
term-served = Terminologies the CDR serves
term-id = Id
term-uri = Identifying URI
term-fields = Fields you can ask for
term-descriptor = Descriptor
term-lookup-in = Look up a code in it
term-expand-group = Expand one of its groups
term-code = Code
term-effective-date = Effective date (optional)
term-look-up = Look up
term-defined = Defined:
term-undefined = The terminology { $terminology } does not define { $code }.
term-language = Language
term-preferred = Preferred term
term-lookup-refused = The CDR could not look the code up.
term-value-set = Value set
term-expand = Expand
term-members = Members
term-no-value-set = The terminology has no value set { $set }.
term-value-set-refused = The CDR could not expand the value set.
term-validate-title = Validate a code
term-validate = Validate
term-member = { $code } is a member of { $set }.
term-not-member = { $code } is not a member of { $set }.
term-validate-refused = The CDR could not validate the code.
term-broader = Broader code
term-narrower = Narrower code
term-test = Test
term-subsumes = { $broader } is broader than { $narrower }.
term-not-subsumes = { $broader } is not broader than { $narrower }.
term-flat = The openEHR vocabulary has no broader or narrower codes, so it always answers no; a terminology server bound to the CDR can answer otherwise.
term-subsumption-refused = The CDR could not test the codes.

## Stored queries

queries-subtitle = The CDR's stored queries, grouped by the namespace of their name.
queries-unqualified = Unqualified
queries-group-count = { $names ->
    [one] 1 query
   *[other] { $names } queries
}{ $versions ->
    [one] {""}
   *[other] , { $versions } versions
}
queries-older-version = older version
queries-open-in-query = Open in Query
queries-delete-version = Delete this version…
queries-namespace-chip = Namespace { $value }
queries-refused = The stored queries could not be read.
queries-views = Views of this stored query
queries-more = More actions for the stored query
queries-copy-name = Copy name
queries-line = { $name } · version { $version }
queries-delete-named = Delete version { $version }
queries-delete-title = Delete { $name } version { $version }?
queries-delete-message = Removes this version from the CDR's stored queries for good, for every client; its other versions stay.
queries-delete-confirm = Delete version { $version }
queries-deleted = Stored query version deleted
queries-delete-refused = The CDR refused to delete the version.
queries-definition = Definition
queries-stored-refused = The stored query could not be read.
queries-reading-older = You are reading version { $version }; Latest runs { $latest }.
queries-read-version = Read { $version }
queries-type = Type
queries-saved = Saved
queries-aql-title = { $name } { $version }
queries-which-version = Which version runs
queries-param-help = Numbers and true or false are sent as they are; quote a number to send it as text.
queries-fix-version = Nothing was sent: fix the version first.
queries-versions-intro = Each version is its own immutable definition; Latest runs the highest.

## Query

query-subtitle = Build AQL from a template, or write it; run it against the CDR.
query-run = Run
query-save = Save to the CDR
query-save-action = Save
query-namespace = Namespace
query-version-help = Empty writes the version the CDR assigns.
query-version-taken = This version of this name already exists, and a saved version cannot change.
query-save-refused = The CDR refused to store the query.
query-open-stored = Open it
query-build = Build
query-aql = AQL
query-aql-title = Query · AQL
query-ways = Ways to write the query
query-returns = Returns
query-shape-data = Data points
query-shape-compositions = Whole compositions
query-shape-count = A count
query-shape-ehrs = EHRs (cohort)
query-data-points = Data points
query-data-point = Data point
query-column-name = Column name
query-data-point-n = Data point { $number }
query-column-name-n = Column name of data point { $number }
query-remove-data-point = Remove the data point { $name }
query-add-data-point = Add a data point
relationship-needs-type-and-target = Give the relationship a type and a target party.
relationship-needs-type = Give the relationship a type.
queries-version-help = Latest sends no version; Prefix and Exact send the one typed here.
queries-sends = Sends
directory-older-versions = Older versions
query-save-name-missing = Name the query to save it.
query-no-data-points = No data point yet; the query returns nothing until you add one.
query-conditions = Conditions
query-order-limit = Order and limit
query-order-by = Order by
query-no-order = No order
query-direction = Direction
query-newest-first = Newest first
query-oldest-first = Oldest first
query-limit = Row limit
query-bad-limit = The row limit is not a whole number.
query-it-builds = The AQL it builds
query-built-aql = Built AQL
query-open-in-aql = Open in AQL
query-open-in-build = Open in Build
query-not-built = The builder cannot write this query yet: { $reason }
query-cannot-build = This AQL cannot be opened in Build: { $reason }
query-not-number = The value of { $path }, { $value }, is not a number.
query-unknown-path = The template has no data point at { $path }.
query-catalog-refused = The template's data points could not be read.
query-run-refused = The CDR refused the query.
query-the-aql = The AQL
query-opened-stored = Opened from the stored query { $name } { $version }.
query-stored-refused = The stored query could not be read.
query-build-made = The AQL Build made, as it runs; edits here leave Build behind.
query-nothing-to-run = Nothing to run: write a query first.
query-grammar-stops = Nothing was sent: the grammar check found an error.
query-grammar-heading = { $count ->
    [one] Grammar check · 1
   *[other] Grammar check · { $count }
}
query-unexpected = Unexpected { $token } here.
query-ended-early = The query ends before it is complete.
query-parameters = Parameters
query-param-name-n = Name of parameter { $number }
query-param-value-n = Value of parameter { $number }
query-remove-param = Remove the parameter { $name }
query-add-param = Add a parameter
query-no-params = No parameter.

## Results

results-title = Results
results-chart-label = The numeric columns over the results
results-chart-summary = { $series } over { $axis }.
results-axis = The x axis is { $axis }, at its true spacing.
results-not-run = Run the query to see its rows here.
results-window = { $count ->
    [one] 1 row. No LIMIT in the query, so the CDR returns at most its own page.
   *[other] { $count } rows. No LIMIT in the query, so the CDR returns at most its own page.
}
results-window-own = { $count ->
    [one] 1 row, inside the query's own LIMIT.
   *[other] { $count } rows, inside the query's own LIMIT.
}

## Server

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
api-all = The complete surface
api-ehr = openEHR: EHR
api-query = openEHR: Query
api-definition = openEHR: Definition
api-demographic = openEHR: Demographic (in development)
api-admin = openEHR: Admin (in development)
api-management = FerroEHR: Status and Management
api-terminology = FerroEHR: Terminology
api-relationships = FerroEHR: Party Relationships
api-events = FerroEHR: Event Subscriptions
api-audit = FerroEHR: Audit
api-smart = FerroEHR: SMART Discovery

## Audit log

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
audit-chip-agent = Principal: { $value }
audit-chip-outcome = Outcome: { $value }
audit-chip-action = Action: { $value }
audit-chip-patient = Patient: { $value }
audit-record-gone = The audit log holds no record at this address any more; open it from the audit log again.
audit-what = What
audit-stored-as = Stored as a FHIR AuditEvent (IHE ITI-81).
audit-the-event = The event
audit-as-stored = As stored

## Subscriptions

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
sub-any-kind = Any kind
sub-composition = Composition
sub-ehr-status = EHR status
sub-folder = Folder
sub-any-change = Any change
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

## Configuration

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

template-delete-row = Delete…
sub-delete-row = Delete…
