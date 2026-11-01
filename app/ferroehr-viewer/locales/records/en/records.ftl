# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1

# The English catalog of the records group: the reference catalog and the
# one every lookup falls back to. Each message is one `RecordsMsg` of the
# crate's `locale::records` module; a catalog of another language holds the
# same messages in a directory named by its BCP 47 tag beside this one.

composition-deleted = Composition deleted
party-created = Party created
contribution-uid-required = a contribution uid is required
party-deleted = Party deleted
loaded-party-document-not-json-object = the loaded party document is not a JSON object: reload this tab and retry
identities-must-json-array-party-identity = identities must be a JSON array of PARTY_IDENTITY objects
party-body-must-json-object = the party body must be a JSON object
party-kind = party kind
current-version-uid-required-update-party = the current version uid is required to update this party: reload this tab and retry
cdr-created-party-but-returned-no = the CDR created the party but returned no uid to open it by
relationship-deleted = This relationship is deleted; its versions stay readable.
relationship-type = Type
relationship-created = Relationship created
party-relationships = Relationships
loaded-relationship-document-not-json-object = the loaded relationship document is not a JSON object: reload this screen and retry
relationship-type-required-party-relationship-invariant = a relationship type is required (PARTY_RELATIONSHIP invariant Type_validity)
archetype-node-id-required-locatable-archetype = an archetype node id is required (LOCATABLE.archetype_node_id)
cdr-created-relationship-but-returned-no = the CDR created the relationship but returned no uid to open it by
relationship-id-required = a relationship id is required
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
composition-committed = Composition committed
template-id-required-commit-flat-composition = a template id is required to commit a FLAT composition
directory-deleted = Directory deleted
directory-restored = Directory restored
directory-created = Directory created
a-path-is-required = a path is required
date-time-required = a date and time is required
current-directory-version-uid-required-delete = the current directory version uid is required to delete
current-directory-version-uid-required-update = the current directory version uid is required to update
directory-body-empty = the directory body is empty
ehr-views = EHR views
ehr-deleted = EHR deleted
loaded-ehr-status-document-not-json = the loaded EHR_STATUS document is not a JSON object: reload this tab and retry
subject-id = Subject id
ehr-created = EHR created
bytes = { $size } bytes
contribution-json = CONTRIBUTION JSON: { $e }
party-body-not-valid-json = the party body is not valid JSON: { $e }
party-json = party JSON: { $e }
identities-not-valid-json = identities is not valid JSON: { $e }
details-not-valid-json = details is not valid JSON: { $e }
loaded-party-document-not-valid-json = the loaded party document is not valid JSON ({ $e }): reload this tab and retry
edited-party-could-not-serialized = the edited party could not be serialized: { $e }
party-s-id-required = the { $end } party's id is required
party-s-kind-must-one-five = the { $end } party's kind must be one of the five party kinds
details-not-valid-json-2 = details is not valid JSON: { $e }
edited-relationship-could-not-serialized = the edited relationship could not be serialized: { $e }
party-relationship-json = PARTY_RELATIONSHIP JSON: { $e }
not-valid-json = not valid JSON: { $e }
change-2 = change { $position } ({ $change }, { $target }): { $complaint }
contribution-could-not-serialized = the contribution could not be serialized: { $e }
document-not-valid-json = the document is not valid JSON: { $e }
z-2 = { $civil }Z
contributions-json = contributions JSON: { $e }
other-details-not-valid-json = other_details is not valid JSON: { $e }
loaded-ehr-status-document-not-valid = the loaded EHR_STATUS document is not valid JSON ({ $e }): reload this tab and retry
edited-ehr-status-could-not-serialized = the edited EHR_STATUS could not be serialized: { $e }
new-ehr = New EHR
deleting-party-needs-latest-version-s = deleting a party needs the latest version's full OBJECT_VERSION_ID: reload this screen and retry
identities-must-hold-least-one-party = identities must hold at least one PARTY_IDENTITY: a party with no identity is invalid (PARTY invariant Identities_valid)
current-version-uid-required-update-relationship = the current version uid is required to update this relationship: reload this screen and retry
deleting-relationship-needs-latest-version-s = deleting a relationship needs the latest version's full OBJECT_VERSION_ID: reload this screen and retry
relationship-type-required-stored-relationship-s = a relationship type is required: it is stored as the relationship's name (PARTY_RELATIONSHIP invariant Type_validity)
party-must-named-versioned-object-uid = the { $end } party must be named by its versioned_object_uid, not by one version's OBJECT_VERSION_ID: a relationship refers to the party's version container
loaded-relationship-document-not-valid-json = the loaded relationship document is not valid JSON ({ $e }): reload this screen and retry
flag-queryable = queryable
flag-modifiable = modifiable

## Numbers: the marks FerroUI's formatter groups a number with

number-group = ,
number-decimal = .

none-shown = Not recorded
column-ehr = EHR

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

editor-title = Edit
editor-opened = Opened at the latest version
editor-not-saved = Not saved
editor-save-as-version = Save as version { $version }
editor-reload-version = Reload version { $version }
editor-reload-first = Reload the latest version before you save.
editor-reload-replaces = Reloading replaces your edits with the version on the server.
editor-json = JSON

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
status-facts-refused = This version's envelope could not be read.
status-version-refused = This version could not be read.

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
directory-compositions-refused = The EHR's compositions could not be listed.
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

column-composition = Composition
column-start-time = Start time
column-composer = Composer
compositions-empty = This EHR holds no composition yet.
compositions-no-match = No composition matches these filters.
compositions-pages = Composition pages
compositions-refused = The compositions could not be read.
filter-start-time = Start time
filter-composer = Composer

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
change-attestation = Attestation
change-synthesis = Synthesis
contribution-title = Contribution { $id }
contribution-subtitle = One change set, committed together.
contribution-refused = The contribution could not be read.
contribution-audit = Audit
contribution-new-audit = Contribution audit
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
contribution-committer-placeholder = Leave empty to commit as { $name }
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

tags-title = Tags in EHR { $id }
tags-intro = A tag is set on its object, on that object's Tags tab; this list reads them all.
tags-key = Key
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

composition-title = Composition { $id }
composition-refused = The composition could not be read.
composition-views = Views of this composition
composition-reading = Reading
composition-document = Document
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
editor-xml = XML

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
composition-not-committed = Not committed

kind-person = Person
kind-organisation = Organisation
kind-group = Group
kind-agent = Agent
kind-role = Role
parties-subtitle = People, organisations, groups, agents and roles, opened by their id.
parties-new = New party
parties-note = Parties follow the released RM 1.1.0 demographic model; the REST API for them is still in development, and FerroEHR adds the relationship routes it does not define yet.
parties-uid = Party uid
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

relationship-needs-type-and-target = Give the relationship a type and a target party.
relationship-needs-type = Give the relationship a type.
directory-older-versions = Older versions
